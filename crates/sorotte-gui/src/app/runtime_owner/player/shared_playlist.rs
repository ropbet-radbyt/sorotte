use super::*;
use sorotte_player_api::LocalFileUpdate;
use sorotte_plex::{
    PlexClientConfig, PlexMatchedItem, PlexPlaylistUri, format_plex_playlist_uri,
    server_scoped_cache_key_for_file,
};

impl GuiPersistedConfigRuntimeOwner {
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

    pub(super) fn selected_opened_entry_offset(
        selected_playlist_index: Option<usize>,
        opened_entry_count: usize,
        playlist_insert_slot: Option<usize>,
    ) -> Option<usize> {
        let selected_index = selected_playlist_index?;
        if opened_entry_count == 0 {
            return None;
        }
        match playlist_insert_slot {
            Some(insert_slot) => selected_index
                .checked_sub(insert_slot)
                .filter(|offset| *offset < opened_entry_count),
            None => Some(selected_index).filter(|offset| *offset < opened_entry_count),
        }
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

    fn shared_playlist_path_is_import_playlist(path: &str) -> bool {
        let lower_path = path.to_ascii_lowercase();
        lower_path.ends_with(".txt")
            || lower_path.ends_with(".m3u")
            || lower_path.ends_with(".m3u8")
    }

    fn shared_playlist_import_entries_from_path(path: &str) -> Result<Option<Vec<String>>, String> {
        if path.contains("://") {
            return Ok(None);
        }
        if !Self::shared_playlist_path_is_import_playlist(path) {
            return Ok(None);
        }
        let contents = std::fs::read_to_string(path)
            .map_err(|error| format!("Shared playlist import failed reading '{path}': {error}"))?;
        let playlist_entries = contents
            .lines()
            .filter_map(normalized_editable_text)
            .collect::<Vec<_>>();
        if playlist_entries.is_empty() {
            return Err(format!(
                "Shared playlist import file '{path}' did not contain any playlist entries."
            ));
        }
        Ok(Some(playlist_entries))
    }

    fn shared_playlist_local_file_update_for_path(path: &str) -> Option<LocalFileUpdate> {
        let path = normalized_editable_text(path)?;
        if path.contains("://") || Self::shared_playlist_path_is_import_playlist(&path) {
            return None;
        }
        let file_name = Path::new(&path)
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())?
            .to_owned();
        let mut file = LocalFileUpdate::new(file_name).with_path(path.clone());
        if let Ok(metadata) = std::fs::metadata(&path)
            && metadata.is_file()
        {
            file = file.with_size_bytes(metadata.len());
        }
        Some(file)
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
                playlist_entries,
                player_paths: None,
                imported_from_file: true,
            });
        }

        let playlist_entries = paths
            .iter()
            .filter_map(|path| {
                self.shared_playlist_plex_publish_target_for_path(state, path)
                    .or_else(|| shared_playlist_entry_for_media_path(path))
            })
            .collect::<Vec<_>>();
        if playlist_entries.is_empty() {
            return Err(
                "Shared playlist open could not derive any playlist entries from the selected files."
                    .to_owned(),
            );
        }
        Ok(GuiSharedPlaylistOpenDispatch {
            playlist_entries,
            player_paths: Some(paths),
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
                playlist_entries,
                player_paths: None,
                imported_from_file: true,
            });
        }

        let playlist_entries = paths
            .iter()
            .filter_map(|path| shared_playlist_entry_for_media_path(path))
            .collect::<Vec<_>>();
        if playlist_entries.is_empty() {
            return Err(
                "Shared playlist open could not derive any playlist entries from the selected files."
                    .to_owned(),
            );
        }
        Ok(GuiSharedPlaylistOpenDispatch {
            playlist_entries,
            player_paths: Some(paths),
            imported_from_file: false,
        })
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
