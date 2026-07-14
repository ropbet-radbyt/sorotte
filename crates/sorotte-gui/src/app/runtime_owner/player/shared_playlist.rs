use super::*;
use crate::app::shell_state::MainWindowPlaylistRow;
use sorotte_player_api::LocalFileUpdate;
use sorotte_plex::{
    PlexClientConfig, PlexMatchedItem, PlexPlaylistUri, format_plex_playlist_uri,
    server_scoped_cache_key_for_file,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum SharedPlaylistImportFormat {
    Text,
    M3u,
    M3u8,
}

impl GuiPersistedConfigRuntimeOwner {
    fn shared_playlist_import_format(path: &str) -> Option<SharedPlaylistImportFormat> {
        let lower_path = path.to_ascii_lowercase();
        if lower_path.ends_with(".txt") {
            Some(SharedPlaylistImportFormat::Text)
        } else if lower_path.ends_with(".m3u") {
            Some(SharedPlaylistImportFormat::M3u)
        } else if lower_path.ends_with(".m3u8") {
            Some(SharedPlaylistImportFormat::M3u8)
        } else {
            None
        }
    }

    pub(super) fn shared_playlist_mutation_current_index(
        &self,
        state: &SorotteGuiShellAppState,
        allow_local_selection: bool,
    ) -> Option<usize> {
        let playlist_len = state.main_window.playlist.len();
        self.session
            .as_ref()
            .and_then(|session| session.current_room_playlist_index())
            .or(state.main_window.active_playlist_index)
            .or(self.active_shared_playlist_index)
            .or_else(|| {
                allow_local_selection
                    .then_some(state.selection.selected_main_window_playlist)
                    .flatten()
            })
            .filter(|index| *index < playlist_len)
    }

    pub(in crate::app::runtime_owner) fn open_media_unavailable_message_impl(
        &self,
        selected_paths: &[String],
    ) -> String {
        let base = if selected_paths.len() == 1 {
            "Opening media requires a playback runtime connection; the selected file was not opened."
                .to_owned()
        } else {
            format!(
                "Opening media requires a playback runtime connection; {} selected files were not opened.",
                selected_paths.len()
            )
        };
        if let Some(reason) = self.player_unavailability_reason.as_deref() {
            format!("{base} {reason}")
        } else {
            base
        }
    }

    pub(super) fn shared_playlist_open_unavailable_message_impl(
        &self,
        selected_paths: &[String],
    ) -> String {
        let base = if selected_paths.len() == 1 {
            "Opening media into the shared playlist requires a session or playback runtime connection; the selected file was not opened or queued."
                .to_owned()
        } else {
            format!(
                "Opening media into the shared playlist requires a session or playback runtime connection; {} selected files were not opened or queued.",
                selected_paths.len()
            )
        };
        if let Some(reason) = self.player_unavailability_reason.as_deref() {
            format!("{base} {reason}")
        } else {
            base
        }
    }

    pub(super) fn shared_playlist_session_unavailable_message_impl(&self) -> String {
        "Shared playlist updates require a session runtime connection; the selected media was not added to the room playlist."
            .to_owned()
    }

    pub(super) fn shared_playlist_control_unavailable_message_impl(&self) -> String {
        "Shared playlist control is unavailable for the active room; the selected media was not added to the room playlist or opened in the attached player."
            .to_owned()
    }

    fn shared_playlist_import_entries_from_path(path: &str) -> Result<Option<Vec<String>>, String> {
        if path.contains("://") {
            return Ok(None);
        }
        let Some(format) = Self::shared_playlist_import_format(path) else {
            return Ok(None);
        };
        let contents = std::fs::read_to_string(path)
            .map_err(|error| format!("Shared playlist import failed reading '{path}': {error}"))?;
        let playlist_entries = if format == SharedPlaylistImportFormat::Text {
            contents
                .lines()
                .filter_map(normalized_editable_text)
                .collect::<Vec<_>>()
        } else {
            let lines = contents
                .lines()
                .filter_map(|line| {
                    let line = line.trim();
                    normalized_editable_text(line.trim_start_matches('\u{feff}'))
                })
                .collect::<Vec<_>>();
            if format == SharedPlaylistImportFormat::M3u8
                && lines
                    .iter()
                    .any(|line| line.to_ascii_uppercase().starts_with("#EXT-X-"))
            {
                return Ok(None);
            }
            let playlist_parent = Path::new(path).parent().unwrap_or_else(|| Path::new(""));
            lines
                .into_iter()
                .filter(|line| !line.starts_with('#'))
                .map(|entry| {
                    if entry.contains("://") || Path::new(&entry).is_absolute() {
                        entry
                    } else {
                        playlist_parent
                            .join(entry)
                            .components()
                            .collect::<PathBuf>()
                            .to_string_lossy()
                            .into_owned()
                    }
                })
                .collect::<Vec<_>>()
        };
        if playlist_entries.is_empty() {
            return Err(format!(
                "Shared playlist import file '{path}' did not contain any playlist entries."
            ));
        }
        Ok(Some(playlist_entries))
    }

    fn shared_playlist_local_file_update_for_path(path: &str) -> Option<LocalFileUpdate> {
        let path = normalized_editable_text(path)?;
        if path.contains("://")
            || matches!(
                Self::shared_playlist_import_format(&path),
                Some(SharedPlaylistImportFormat::Text | SharedPlaylistImportFormat::M3u)
            )
        {
            return None;
        }
        let metadata = std::fs::metadata(&path).ok()?;
        if !metadata.is_file() {
            return None;
        }
        let file_name = Path::new(&path)
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())?
            .to_owned();
        Some(
            LocalFileUpdate::new(file_name)
                .with_path(path)
                .with_size_bytes(metadata.len()),
        )
    }

    fn cached_plex_playlist_uri_for_local_file(
        &mut self,
        config: &PlexClientConfig,
        file: &LocalFileUpdate,
    ) -> Option<PlexPlaylistUri> {
        let machine_identifier = config
            .selected_server_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())?
            .to_owned();
        let cache_key = server_scoped_cache_key_for_file(config, file)?;
        let engine = self.take_plex_sync_engine(config.clone()).ok()?;
        let item = engine
            .cache()
            .entries
            .get(&cache_key)
            .cloned()
            .map(PlexMatchedItem::from);
        self.plex_sync_engine = Some(engine);
        let item = item?;
        let file_name = file
            .path
            .as_deref()
            .and_then(|path| Path::new(path).file_name())
            .and_then(|name| name.to_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                let name = file.name.trim();
                (!name.is_empty()).then(|| name.to_owned())
            });
        Some(PlexPlaylistUri {
            machine_identifier,
            rating_key: item.rating_key,
            title: Some(item.title),
            file_name,
            duration_millis: item.duration_millis,
            size_bytes: file.size_bytes,
            media_type: Some(item.media_type),
        })
    }

    fn shared_playlist_plex_publish_target_for_path(
        &mut self,
        state: &SorotteGuiShellAppState,
        path: &str,
    ) -> Option<String> {
        if !state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::Plex)
        {
            return None;
        }
        let settings = state.configuration.to_stored_settings();
        let config = super::super::plex::plex_config_from_settings(&settings);
        if !config.streaming_enabled || !config.has_selected_server() {
            return None;
        }
        let local_file = Self::shared_playlist_local_file_update_for_path(path)?;
        if let Some(uri) = self.cached_plex_playlist_uri_for_local_file(&config, &local_file) {
            return Some(format_plex_playlist_uri(&uri));
        }
        // Cache misses may require blocking Plex HTTP search/metadata calls. Keep playlist
        // projection cheap; explicit Plex search/open paths resolve uncached streams on workers.
        None
    }

    pub(in crate::app::runtime_owner) fn shared_playlist_open_dispatch_for_selected_paths_impl(
        &mut self,
        state: &SorotteGuiShellAppState,
        paths: Vec<String>,
    ) -> Result<GuiSharedPlaylistOpenDispatch, String> {
        if paths.len() == 1
            && let Some(playlist_entries) =
                Self::shared_playlist_import_entries_from_path(&paths[0])?
        {
            return Ok(GuiSharedPlaylistOpenDispatch {
                items: playlist_entries
                    .into_iter()
                    .map(|published_entry| GuiSharedPlaylistOpenItem {
                        published_entry,
                        local_origin: None,
                    })
                    .collect(),
                imported_from_file: true,
            });
        }

        let items = paths
            .iter()
            .filter_map(|path| {
                let local_file = if path.contains("://") {
                    None
                } else {
                    Some(Self::shared_playlist_local_file_update_for_path(path)?)
                };
                let published_entry = self
                    .shared_playlist_plex_publish_target_for_path(state, path)
                    .or_else(|| shared_playlist_entry_for_media_path(path))?;
                let local_origin = local_file.and_then(|file| file.path);
                Some(GuiSharedPlaylistOpenItem {
                    published_entry,
                    local_origin,
                })
            })
            .collect::<Vec<_>>();
        if items.is_empty() {
            return Err(
                "Shared playlist open could not derive any playlist entries from the selected files."
                    .to_owned(),
            );
        }
        Ok(GuiSharedPlaylistOpenDispatch {
            items,
            imported_from_file: false,
        })
    }

    pub(in crate::app::runtime_owner) fn shared_playlist_open_dispatch_for_paths_impl(
        paths: Vec<String>,
    ) -> Result<GuiSharedPlaylistOpenDispatch, String> {
        if paths.len() == 1
            && let Some(playlist_entries) =
                Self::shared_playlist_import_entries_from_path(&paths[0])?
        {
            return Ok(GuiSharedPlaylistOpenDispatch {
                items: playlist_entries
                    .into_iter()
                    .map(|published_entry| GuiSharedPlaylistOpenItem {
                        published_entry,
                        local_origin: None,
                    })
                    .collect(),
                imported_from_file: true,
            });
        }

        let items = paths
            .iter()
            .filter_map(|path| {
                let local_file = if path.contains("://") {
                    None
                } else {
                    Some(Self::shared_playlist_local_file_update_for_path(path)?)
                };
                let published_entry = shared_playlist_entry_for_media_path(path)?;
                let local_origin = local_file.and_then(|file| file.path);
                Some(GuiSharedPlaylistOpenItem {
                    published_entry,
                    local_origin,
                })
            })
            .collect::<Vec<_>>();
        if items.is_empty() {
            return Err(
                "Shared playlist open could not derive any playlist entries from the selected files."
                    .to_owned(),
            );
        }
        Ok(GuiSharedPlaylistOpenDispatch {
            items,
            imported_from_file: false,
        })
    }

    pub(in crate::app::runtime_owner) fn reconcile_local_shared_playlist_media_paths(
        &mut self,
        state: &SorotteGuiShellAppState,
    ) {
        let room_changed = self.playlist_resolution.room_name.as_deref()
            != Some(state.main_window.room_name.as_str());
        let scope_initialized = self.playlist_resolution.room_name.is_some();
        let session_active = self.session.is_some();
        let session_changed = self.playlist_resolution.session_generation
            != self.session_generation
            || self.playlist_resolution.session_active != session_active;
        let playlist_revision = self
            .session
            .as_ref()
            .and_then(|session| session.current_room_playlist_revision());
        let remote_playlist_revision = self
            .session
            .as_ref()
            .map_or(0, |session| session.current_room_playlist_remote_revision());
        let current_row_ids = state
            .main_window
            .playlist
            .iter()
            .map(|row| row.entry_id)
            .collect::<Vec<_>>();
        let remote_playlist_replacement = !room_changed
            && !session_changed
            && self.playlist_resolution.remote_playlist_revision != remote_playlist_revision;
        let established_scope_transition = scope_initialized && session_changed;
        if room_changed || session_changed || remote_playlist_replacement {
            self.playlist_resolution.generation =
                self.playlist_resolution.generation.wrapping_add(1);
            self.playlist_resolution.local_origins_by_row.clear();
            self.pending_playlist_source_resolution = None;
            self.last_attached_media_resolution_trigger = None;
        }
        self.playlist_resolution.row_scope_reset_pending |=
            remote_playlist_replacement || established_scope_transition;
        self.playlist_resolution.room_name = Some(state.main_window.room_name.clone());
        self.playlist_resolution.session_generation = self.session_generation;
        self.playlist_resolution.session_active = session_active;
        self.playlist_resolution.playlist_revision = playlist_revision;
        self.playlist_resolution.remote_playlist_revision = remote_playlist_revision;

        if self.playlist_resolution.row_ids != current_row_ids {
            self.playlist_resolution.row_ids = current_row_ids.clone();
        }

        let mut retained_row_ids = current_row_ids.into_iter().collect::<BTreeSet<_>>();
        retained_row_ids.extend(
            state
                .playlist_entry_id_undo_snapshot
                .iter()
                .flatten()
                .copied(),
        );
        let previous_count = self.playlist_resolution.local_origins_by_row.len();
        self.playlist_resolution
            .local_origins_by_row
            .retain(|entry_id, path| retained_row_ids.contains(entry_id) && path.is_file());
        if previous_count != self.playlist_resolution.local_origins_by_row.len() {
            self.last_attached_media_resolution_trigger = None;
        }

        if let Some(pending) = self.pending_playlist_source_resolution.as_mut() {
            if pending.generation != self.playlist_resolution.generation {
                self.pending_playlist_source_resolution = None;
            } else if let Some(index) = state
                .main_window
                .playlist
                .iter()
                .position(|row| row.entry_id == pending.entry_id)
            {
                pending.index = index;
            } else {
                self.pending_playlist_source_resolution = None;
            }
        }
    }

    pub(in crate::app::runtime_owner) fn apply_pending_playlist_row_scope_reset(
        &mut self,
        state: &mut SorotteGuiShellAppState,
    ) -> bool {
        if !self.playlist_resolution.row_scope_reset_pending {
            return false;
        }

        let fresh_rows = state
            .main_window
            .playlist
            .iter()
            .map(|row| {
                let source_state = state.playlist_source_state_for_entry(&row.label);
                MainWindowPlaylistRow {
                    entry_id: source_state.entry_id,
                    label: row.label.clone(),
                    is_selected: row.is_selected,
                    source_state,
                }
            })
            .collect::<Vec<_>>();
        state.main_window.playlist = fresh_rows;
        state.playlist_undo_snapshot = None;
        state.playlist_source_undo_snapshot = None;
        state.playlist_entry_id_undo_snapshot = None;
        self.playlist_resolution.row_ids = state
            .main_window
            .playlist
            .iter()
            .map(|row| row.entry_id)
            .collect();
        self.playlist_resolution.row_scope_reset_pending = false;
        true
    }

    pub(super) fn begin_shared_playlist_replacement_scope(&mut self) {
        self.playlist_resolution.generation = self.playlist_resolution.generation.wrapping_add(1);
        self.playlist_resolution.row_ids.clear();
        self.playlist_resolution.row_scope_reset_pending = false;
        self.pending_playlist_source_resolution = None;
        self.last_attached_media_resolution_trigger = None;
    }

    pub(super) fn remember_local_shared_playlist_media_paths(
        &mut self,
        state: &SorotteGuiShellAppState,
        dispatch: &GuiSharedPlaylistOpenDispatch,
        opened_rows: &[(GuiPlaylistEntryId, String)],
    ) -> GuiPlaylistLocalOriginBindingOutcome {
        self.reconcile_local_shared_playlist_media_paths(state);
        let mut used_items = vec![false; dispatch.items.len()];
        let mut outcome = GuiPlaylistLocalOriginBindingOutcome::default();
        let mut changed = false;
        for (entry_id, label) in opened_rows {
            let Some((item_index, item)) = dispatch
                .items
                .iter()
                .enumerate()
                .find(|(index, item)| !used_items[*index] && item.published_entry == *label)
            else {
                continue;
            };
            used_items[item_index] = true;
            let Some(local_origin) = item.local_origin.as_deref() else {
                continue;
            };
            let path = PathBuf::from(local_origin);
            if !path.is_file() {
                outcome.unavailable_row_ids.push(*entry_id);
                continue;
            }
            outcome.bound_row_ids.push(*entry_id);
            changed |= self
                .playlist_resolution
                .local_origins_by_row
                .insert(*entry_id, path.clone())
                .as_ref()
                != Some(&path);
        }
        if changed {
            self.last_attached_media_resolution_trigger = None;
        }
        outcome
    }

    pub(super) fn local_shared_playlist_media_path_for_row(
        &mut self,
        state: &SorotteGuiShellAppState,
        entry_id: GuiPlaylistEntryId,
    ) -> Option<String> {
        self.reconcile_local_shared_playlist_media_paths(state);
        let path = self
            .playlist_resolution
            .local_origins_by_row
            .get(&entry_id)?
            .clone();
        if path.is_file() {
            return Some(path.to_string_lossy().into_owned());
        }
        self.playlist_resolution
            .local_origins_by_row
            .remove(&entry_id);
        self.last_attached_media_resolution_trigger = None;
        None
    }

    pub(super) fn shared_playlist_open_success_message(
        dispatch: &GuiSharedPlaylistOpenDispatch,
        entry_count: usize,
    ) -> String {
        if dispatch.imported_from_file {
            if entry_count == 1 {
                "Imported 1 entry into the shared playlist.".to_owned()
            } else {
                format!("Imported {entry_count} entries into the shared playlist.")
            }
        } else if entry_count == 1 {
            "Loaded 1 selected media entry into the shared playlist.".to_owned()
        } else {
            format!("Loaded {entry_count} selected media entries into the shared playlist.")
        }
    }
}
