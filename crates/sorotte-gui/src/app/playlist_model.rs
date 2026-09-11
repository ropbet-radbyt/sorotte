//! Playlist identity and source policy shared by the shell and runtime worker.
use super::shell_state::*;
use super::support::normalized_editable_text;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub(super) struct GuiPlaylistSources<'a> {
    pub(super) default_source: GuiPlaylistDefaultSourceId,
    pub(super) media_match: &'a GuiMediaMatchState,
    pub(super) plex: &'a GuiPlexState,
    pub(super) plugin_enablement: GuiPluginEnablementState,
}

impl GuiPlaylistSources<'_> {
    pub(super) fn playlist_source_state_for_entry(&self, entry: &str) -> GuiPlaylistSourceState {
        let source_state = self
            .playlist_default_provider_for_new_entry(entry)
            .map(GuiPlaylistSourceState::for_playlist_default)
            .unwrap_or_else(|| GuiPlaylistSourceState::inferred_for_entry(entry));
        self.refreshed_playlist_source_state_for_entry(entry, source_state)
    }

    pub(super) fn playlist_default_provider_for_new_entry(
        &self,
        entry: &str,
    ) -> Option<GuiMediaSourceProviderId> {
        let default_provider = self.default_source.provider_id().cloned()?;
        let default_available = self
            .playlist_source_options_for_entry(entry, &default_provider)
            .into_iter()
            .find(|option| option.provider_id == default_provider)
            .is_some_and(|option| option.enabled);
        if default_available {
            Some(default_provider)
        } else {
            None
        }
    }

    pub(super) fn refreshed_playlist_source_state_for_entry(
        &self,
        entry: &str,
        mut state: GuiPlaylistSourceState,
    ) -> GuiPlaylistSourceState {
        let unresolved_automatic = state.policy == GuiPlaylistSourcePolicy::Automatic
            && state.resolved_provider_id.is_none()
            && matches!(
                state.status,
                GuiPlaylistSourceStatus::Resolving | GuiPlaylistSourceStatus::Missing
            );
        let selected_provider_id = state
            .preferred_provider_id()
            .cloned()
            .unwrap_or_else(|| state.current_provider_id.clone());
        state.options = self.playlist_source_options_for_entry(entry, &selected_provider_id);
        if unresolved_automatic {
            state.current_label = "Automatic".to_owned();
            for option in &mut state.options {
                option.selected = false;
            }
            return state;
        }
        if let Some(actual_provider) = state
            .options
            .iter()
            .find(|option| option.provider_id == state.current_provider_id)
        {
            state.current_label = actual_provider.label.clone();
        }
        if let Some(selected_option) = state
            .options
            .iter()
            .find(|option| option.provider_id == selected_provider_id)
        {
            if !selected_option.enabled && state.resolved_provider_id.is_none() {
                state.status = GuiPlaylistSourceStatus::Disabled;
                state.detail = selected_option.detail.clone();
            } else if state.status == GuiPlaylistSourceStatus::Disabled {
                state.status = GuiPlaylistSourceStatus::Available;
                state.detail = Some("Waiting for playlist activation.".to_owned());
                state.resolution_steps.clear();
            }
        }
        state
    }

    pub(super) fn playlist_source_options_for_entry(
        &self,
        entry: &str,
        selected_provider_id: &GuiMediaSourceProviderId,
    ) -> Vec<GuiPlaylistSourceOption> {
        vec![
            self.playlist_source_option(
                GuiMediaSourceProviderId::local(),
                "Local",
                selected_provider_id,
                true,
                Some("Resolve only a direct path, the current player file, or configured local media-search directories."),
            ),
            self.playlist_media_match_source_option(selected_provider_id),
            self.playlist_plex_stream_source_option(entry, selected_provider_id),
        ]
    }

    pub(super) fn refreshed_playlist_source_default_state(
        &self,
        mut state: GuiPlaylistDefaultSourceState,
    ) -> GuiPlaylistDefaultSourceState {
        state.options = self.playlist_source_default_options(&state.current_source_id);
        if let Some(selected_option) = state
            .options
            .iter()
            .find(|option| option.source_id == state.current_source_id)
        {
            state.current_label = selected_option.label.clone();
        } else {
            state.current_source_id = GuiPlaylistDefaultSourceId::automatic();
            state.current_label = "Automatic".to_owned();
            state.options = self.playlist_source_default_options(&state.current_source_id);
        }
        state
    }

    pub(super) fn playlist_source_default_options(
        &self,
        selected_source_id: &GuiPlaylistDefaultSourceId,
    ) -> Vec<GuiPlaylistDefaultSourceOption> {
        let mut options = vec![self.playlist_source_default_option(
            GuiPlaylistDefaultSourceId::automatic(),
            "Automatic",
            selected_source_id,
            true,
            Some("Use the built-in source priority for new playlist items."),
        )];
        options.extend(
            self.playlist_source_options_for_entry("", &GuiMediaSourceProviderId::local())
                .into_iter()
                .map(|option| {
                    self.playlist_source_default_option(
                        GuiPlaylistDefaultSourceId::provider(option.provider_id),
                        &option.label,
                        selected_source_id,
                        option.enabled,
                        option.detail.as_deref(),
                    )
                }),
        );
        options
    }

    pub(super) fn playlist_source_default_option(
        &self,
        source_id: GuiPlaylistDefaultSourceId,
        label: &str,
        selected_source_id: &GuiPlaylistDefaultSourceId,
        enabled: bool,
        detail: Option<&str>,
    ) -> GuiPlaylistDefaultSourceOption {
        let selected = &source_id == selected_source_id;
        GuiPlaylistDefaultSourceOption {
            source_id,
            label: label.to_owned(),
            status: if !enabled {
                GuiPlaylistSourceStatus::Disabled
            } else if selected {
                GuiPlaylistSourceStatus::Active
            } else {
                GuiPlaylistSourceStatus::Available
            },
            detail: detail.map(str::to_owned),
            enabled,
            selected,
        }
    }

    pub(super) fn playlist_media_match_source_option(
        &self,
        selected_provider_id: &GuiMediaSourceProviderId,
    ) -> GuiPlaylistSourceOption {
        let detail = if !self
            .plugin_enablement
            .enabled_for(GuiPluginSelection::MediaMatching)
        {
            Some("Media Matching plugin is disabled.")
        } else if !self.media_match.settings.fingerprinting_enabled {
            Some("Media Matching fingerprinting is disabled.")
        } else if self.media_match.health != GuiMediaMatchToolHealth::Healthy {
            Some("Media Matching will run when its tools and cache can provide a match.")
        } else {
            Some("Resolve through cached or background Media Matching lookup.")
        };
        let enabled = self
            .plugin_enablement
            .enabled_for(GuiPluginSelection::MediaMatching)
            && self.media_match.settings.fingerprinting_enabled;
        self.playlist_source_option(
            GuiMediaSourceProviderId::media_matching(),
            "Media Matching",
            selected_provider_id,
            enabled,
            detail,
        )
    }

    pub(super) fn playlist_plex_stream_source_option(
        &self,
        entry: &str,
        selected_provider_id: &GuiMediaSourceProviderId,
    ) -> GuiPlaylistSourceOption {
        let selected_server_available = self
            .plex
            .selected_server_url
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
        let entry_is_plex_uri = sorotte_plex::is_plex_playlist_uri(entry);
        let detail = if !self.plugin_enablement.enabled_for(GuiPluginSelection::Plex) {
            Some("Plex plugin is disabled.")
        } else if !self.plex.authenticated {
            Some("Plex is not authenticated.")
        } else if !self.plex.streaming_enabled {
            Some("Plex streaming is disabled.")
        } else if !entry_is_plex_uri && !selected_server_available {
            Some("Select a Plex server before resolving non-Plex playlist entries.")
        } else {
            Some("Resolve through the Plex stream provider.")
        };
        let enabled = self.plugin_enablement.enabled_for(GuiPluginSelection::Plex)
            && self.plex.authenticated
            && self.plex.streaming_enabled
            && (entry_is_plex_uri || selected_server_available);
        self.playlist_source_option(
            GuiMediaSourceProviderId::plex_stream(),
            "Plex Stream",
            selected_provider_id,
            enabled,
            detail,
        )
    }

    pub(super) fn playlist_source_option(
        &self,
        provider_id: GuiMediaSourceProviderId,
        label: &str,
        selected_provider_id: &GuiMediaSourceProviderId,
        enabled: bool,
        detail: Option<&str>,
    ) -> GuiPlaylistSourceOption {
        let selected = &provider_id == selected_provider_id;
        GuiPlaylistSourceOption {
            provider_id,
            label: label.to_owned(),
            status: if !enabled {
                GuiPlaylistSourceStatus::Disabled
            } else if selected {
                GuiPlaylistSourceStatus::Active
            } else {
                GuiPlaylistSourceStatus::Available
            },
            detail: detail.map(str::to_owned),
            enabled,
            selected,
        }
    }
}
pub(super) fn normalize_shared_playlist_entries(entries: Vec<String>) -> Vec<String> {
    entries
        .into_iter()
        .filter_map(|entry| normalized_editable_text(&entry))
        .collect()
}

pub(super) fn reconciled_playlist_row(
    previous_rows: &[MainWindowPlaylistRow],
    used_previous_rows: &mut [bool],
    index: usize,
    label: &str,
    preferred_entry_id: Option<super::shell_state::GuiPlaylistEntryId>,
) -> Option<MainWindowPlaylistRow> {
    if let Some(preferred_entry_id) = preferred_entry_id
        && let Some((candidate_index, row)) =
            previous_rows
                .iter()
                .enumerate()
                .find(|(candidate_index, row)| {
                    !used_previous_rows
                        .get(*candidate_index)
                        .copied()
                        .unwrap_or(false)
                        && row.entry_id == preferred_entry_id
                        && row.label == label
                })
    {
        if let Some(used) = used_previous_rows.get_mut(candidate_index) {
            *used = true;
        }
        let mut row = row.clone();
        row.source_state.entry_id = row.entry_id;
        return Some(row);
    }
    if let Some(row) = previous_rows.get(index)
        && !used_previous_rows.get(index).copied().unwrap_or(false)
        && row.label == label
    {
        if let Some(used) = used_previous_rows.get_mut(index) {
            *used = true;
        }
        let mut row = row.clone();
        row.source_state.entry_id = row.entry_id;
        return Some(row);
    }

    previous_rows
        .iter()
        .enumerate()
        .find(|(candidate_index, row)| {
            !used_previous_rows
                .get(*candidate_index)
                .copied()
                .unwrap_or(false)
                && row.label == label
        })
        .map(|(candidate_index, row)| {
            if let Some(used) = used_previous_rows.get_mut(candidate_index) {
                *used = true;
            }
            let mut row = row.clone();
            row.source_state.entry_id = row.entry_id;
            row
        })
}

pub(super) fn unique_shared_playlist_additions(
    current_entries: &[String],
    entries: Vec<String>,
) -> Vec<String> {
    let mut seen_entries = current_entries.iter().cloned().collect::<BTreeSet<_>>();
    normalize_shared_playlist_entries(entries)
        .into_iter()
        .filter(|entry| seen_entries.insert(entry.clone()))
        .collect()
}

pub(super) fn shared_playlist_entries_after_media_open(
    current_entries: &[String],
    current_index: Option<usize>,
    opened_entries: Vec<String>,
    insert_slot: Option<usize>,
) -> (Vec<String>, Option<usize>) {
    let opened_entries = if insert_slot.is_some() {
        unique_shared_playlist_additions(current_entries, opened_entries)
    } else {
        normalize_shared_playlist_entries(opened_entries)
    };
    if opened_entries.is_empty() {
        return (
            current_entries.to_vec(),
            insert_slot.and(current_index.filter(|index| *index < current_entries.len())),
        );
    }
    if let Some(insert_slot) = insert_slot {
        let mut playlist_entries = current_entries.to_vec();
        let insert_slot = insert_slot.min(playlist_entries.len());
        playlist_entries.splice(insert_slot..insert_slot, opened_entries);
        return (
            playlist_entries.clone(),
            Some(
                shared_playlist_target_index_from_changed_entries(
                    current_entries,
                    current_index,
                    &playlist_entries,
                )
                .min(playlist_entries.len().saturating_sub(1)),
            ),
        );
    }
    (opened_entries, Some(0))
}

pub(super) fn shared_playlist_target_index_from_changed_entries(
    current_entries: &[String],
    current_index: Option<usize>,
    next_entries: &[String],
) -> usize {
    let Some(current_index) = current_index else {
        return 0;
    };
    if next_entries.len() <= 1 {
        return 0;
    }

    let mut index = current_index;
    while index <= current_entries.len() {
        if let Some(entry) = current_entries.get(index)
            && let Some(valid_index) = next_entries.iter().position(|candidate| candidate == entry)
        {
            return valid_index;
        }
        index = index.saturating_add(1);
    }

    let mut index = current_index;
    while index > 0 {
        if let Some(entry) = current_entries.get(index)
            && let Some(valid_index) = next_entries.iter().position(|candidate| candidate == entry)
        {
            return if valid_index < next_entries.len().saturating_sub(1) {
                valid_index.saturating_add(1)
            } else {
                valid_index
            };
        }
        index = index.saturating_sub(1);
    }
    0
}

pub(super) fn apply_shared_playlist_entries(
    main_window: &mut MainWindowShellState,
    selection: &mut GuiSelectionState,
    local_selection: &mut bool,
    sources: &GuiPlaylistSources<'_>,
    entries: Vec<String>,
    selected_index: Option<usize>,
    selection_is_local: bool,
) {
    let current_entries = current_shared_playlist_entries(&main_window.playlist);
    let active_entry_id = main_window
        .active_playlist_index
        .filter(|index| *index < current_entries.len())
        .and_then(|index| main_window.playlist.get(index))
        .map(|row| row.entry_id);
    let fallback_active_playlist_index = main_window
        .active_playlist_index
        .filter(|index| *index < current_entries.len())
        .map(|current_index| {
            shared_playlist_target_index_from_changed_entries(
                &current_entries,
                Some(current_index),
                &entries,
            )
            .min(entries.len().saturating_sub(1))
        });
    let previous_rows = main_window.playlist.clone();
    let mut used_previous_rows = vec![false; previous_rows.len()];
    main_window.playlist = entries
        .iter()
        .enumerate()
        .map(|(index, label)| {
            let previous_row = reconciled_playlist_row(
                &previous_rows,
                &mut used_previous_rows,
                index,
                label,
                None,
            );
            let source_state = previous_row
                .as_ref()
                .map(|row| {
                    sources
                        .refreshed_playlist_source_state_for_entry(label, row.source_state.clone())
                })
                .unwrap_or_else(|| sources.playlist_source_state_for_entry(label));
            MainWindowPlaylistRow {
                entry_id: source_state.entry_id,
                label: label.clone(),
                is_selected: false,
                source_state,
            }
        })
        .collect();
    main_window.active_playlist_index = active_entry_id
        .and_then(|entry_id| {
            main_window
                .playlist
                .iter()
                .position(|row| row.entry_id == entry_id)
        })
        .or(fallback_active_playlist_index);
    selection.selected_main_window_playlist =
        selected_index.filter(|index| *index < main_window.playlist.len());
    *local_selection = selection_is_local && selection.selected_main_window_playlist.is_some();
    for (index, row) in main_window.playlist.iter_mut().enumerate() {
        row.is_selected = selection.selected_main_window_playlist == Some(index);
    }
}

pub(super) fn next_shared_playlist_shuffle_seed(
    shuffle_nonce: &mut u64,
    entries: &[String],
    current_index: usize,
    shuffle_scope_remaining: bool,
) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(if shuffle_scope_remaining {
        &b"remaining"[..]
    } else {
        &b"entire"[..]
    });
    hasher.update((current_index as u64).to_le_bytes());
    hasher.update(shuffle_nonce.to_le_bytes());
    for entry in entries {
        hasher.update(entry.as_bytes());
        hasher.update([0]);
    }
    *shuffle_nonce = shuffle_nonce.wrapping_add(1);

    let digest = hasher.finalize();
    let mut seed_bytes = [0u8; 8];
    seed_bytes.copy_from_slice(&digest[..8]);
    let seed = u64::from_le_bytes(seed_bytes);
    if seed == 0 {
        0x9E37_79B9_7F4A_7C15
    } else {
        seed
    }
}

pub(super) fn current_shared_playlist_entries(rows: &[MainWindowPlaylistRow]) -> Vec<String> {
    rows.iter().map(|row| row.label.clone()).collect()
}

/// Local playlist edits preserve row identity, source choices and undo together.
/// Edits operate on the feature model independently of presentation.
pub(super) struct GuiPlaylistEditing<'a> {
    pub(super) main_window: &'a mut MainWindowShellState,
    pub(super) selection: &'a mut GuiSelectionState,
    pub(super) selection_is_local: &'a mut bool,
    pub(super) playlist_undo_snapshot: &'a mut Option<Vec<String>>,
    pub(super) playlist_source_undo_snapshot: &'a mut Option<Vec<GuiPlaylistSourceState>>,
    pub(super) playlist_entry_id_undo_snapshot: &'a mut Option<Vec<GuiPlaylistEntryId>>,
    pub(super) shuffle_nonce: &'a mut u64,
    pub(super) sources: GuiPlaylistSources<'a>,
}
impl GuiPlaylistEditing<'_> {
    fn apply_playlist_selection(&mut self) {
        for (index, row) in self.main_window.playlist.iter_mut().enumerate() {
            row.is_selected = self.selection.selected_main_window_playlist == Some(index);
        }
    }
    fn current_shared_playlist_entries(&self) -> Vec<String> {
        current_shared_playlist_entries(&self.main_window.playlist)
    }
    fn apply_shared_playlist_entries(
        &mut self,
        entries: Vec<String>,
        selected: Option<usize>,
        local: bool,
    ) {
        apply_shared_playlist_entries(
            self.main_window,
            self.selection,
            self.selection_is_local,
            &self.sources,
            entries,
            selected,
            local,
        );
    }
    fn set_main_window_playlist_selection(&mut self, selected: Option<usize>, local: bool) {
        self.selection.selected_main_window_playlist = selected;
        *self.selection_is_local = local && selected.is_some();
    }
    fn next_shared_playlist_shuffle_seed(
        &mut self,
        entries: &[String],
        index: usize,
        remaining: bool,
    ) -> u64 {
        next_shared_playlist_shuffle_seed(self.shuffle_nonce, entries, index, remaining)
    }
    fn remember_shared_playlist_undo_snapshot_if_changed(&mut self, next: &[String]) {
        if self.current_shared_playlist_entries() == next {
            return;
        }
        self.remember_shared_playlist_undo_snapshot();
    }
    fn remember_shared_playlist_undo_snapshot(&mut self) {
        *self.playlist_undo_snapshot = Some(self.current_shared_playlist_entries());
        *self.playlist_source_undo_snapshot = Some(
            self.main_window
                .playlist
                .iter()
                .map(|row| row.source_state.clone())
                .collect(),
        );
        *self.playlist_entry_id_undo_snapshot = Some(
            self.main_window
                .playlist
                .iter()
                .map(|row| row.entry_id)
                .collect(),
        );
    }
    fn remember_shared_playlist_undo_snapshot_if_rows_changed(
        &mut self,
        next: &[MainWindowPlaylistRow],
    ) {
        if self
            .main_window
            .playlist
            .iter()
            .map(|row| (&row.label, row.entry_id))
            .eq(next.iter().map(|row| (&row.label, row.entry_id)))
        {
            return;
        }
        *self.playlist_undo_snapshot = Some(self.current_shared_playlist_entries());
        *self.playlist_source_undo_snapshot = Some(
            self.main_window
                .playlist
                .iter()
                .map(|row| row.source_state.clone())
                .collect(),
        );
        *self.playlist_entry_id_undo_snapshot = Some(
            self.main_window
                .playlist
                .iter()
                .map(|row| row.entry_id)
                .collect(),
        );
    }
    fn refresh_playlist_source_states(&mut self) {
        for row in &mut self.main_window.playlist {
            row.source_state = self
                .sources
                .refreshed_playlist_source_state_for_entry(&row.label, row.source_state.clone());
        }
        self.main_window.playlist_default_source =
            self.sources.refreshed_playlist_source_default_state(
                self.main_window.playlist_default_source.clone(),
            );
    }
    pub(super) fn undo_shared_playlist_change(&mut self) -> Result<bool, String> {
        if !self.main_window.shared_playlist_enabled {
            return Err(
                "Shared playlist events are unavailable when shared playlists are disabled."
                    .to_owned(),
            );
        }
        let current_entries = self.current_shared_playlist_entries();
        let current_sources = self
            .main_window
            .playlist
            .iter()
            .map(|row| row.source_state.clone())
            .collect::<Vec<_>>();
        let current_entry_ids = self
            .main_window
            .playlist
            .iter()
            .map(|row| row.entry_id)
            .collect::<Vec<_>>();
        let active_entry_id = self
            .main_window
            .active_playlist_index
            .and_then(|index| self.main_window.playlist.get(index))
            .map(|row| row.entry_id);
        let selected_entry_id = self
            .selection
            .selected_main_window_playlist
            .and_then(|index| self.main_window.playlist.get(index))
            .map(|row| row.entry_id);
        let Some(previous_entries) = (*self.playlist_undo_snapshot).clone() else {
            return Err(String::from(
                "No shared playlist change is available to undo.",
            ));
        };
        let previous_sources = (*self.playlist_source_undo_snapshot)
            .clone()
            .filter(|sources| sources.len() == previous_entries.len());
        let previous_entry_ids = (*self.playlist_entry_id_undo_snapshot)
            .clone()
            .filter(|entry_ids| entry_ids.len() == previous_entries.len());
        let entries_unchanged = previous_entries == current_entries;
        let entry_ids_unchanged = previous_entry_ids
            .as_ref()
            .is_none_or(|entry_ids| entry_ids == &current_entry_ids);
        let sources_unchanged = previous_sources
            .as_ref()
            .is_none_or(|sources| sources == &current_sources);
        if entries_unchanged && entry_ids_unchanged && sources_unchanged {
            return Err(String::from(
                "No shared playlist change is available to undo.",
            ));
        }
        let current_index = self.selection.selected_main_window_playlist;
        let target_index = if previous_entries.is_empty() {
            None
        } else {
            Some(
                shared_playlist_target_index_from_changed_entries(
                    &current_entries,
                    current_index,
                    &previous_entries,
                )
                .min(previous_entries.len().saturating_sub(1)),
            )
        };
        (*self.playlist_undo_snapshot) = Some(current_entries);
        (*self.playlist_source_undo_snapshot) = Some(current_sources);
        (*self.playlist_entry_id_undo_snapshot) = Some(current_entry_ids);
        self.apply_shared_playlist_entries(previous_entries, target_index, true);
        let fallback_active_playlist_index = self.main_window.active_playlist_index;
        let fallback_selected_playlist_index = self.selection.selected_main_window_playlist;
        if let Some(previous_entry_ids) = previous_entry_ids {
            for (row, entry_id) in self.main_window.playlist.iter_mut().zip(previous_entry_ids) {
                row.entry_id = entry_id;
                row.source_state.entry_id = entry_id;
            }
        }
        if let Some(previous_sources) = previous_sources {
            for (row, source_state) in self.main_window.playlist.iter_mut().zip(previous_sources) {
                row.source_state = source_state;
            }
            self.refresh_playlist_source_states();
        }
        self.main_window.active_playlist_index = active_entry_id
            .and_then(|entry_id| {
                self.main_window
                    .playlist
                    .iter()
                    .position(|row| row.entry_id == entry_id)
            })
            .or_else(|| {
                fallback_active_playlist_index
                    .filter(|index| *index < self.main_window.playlist.len())
            });
        let selected_playlist_index = selected_entry_id
            .and_then(|entry_id| {
                self.main_window
                    .playlist
                    .iter()
                    .position(|row| row.entry_id == entry_id)
            })
            .or_else(|| {
                fallback_selected_playlist_index
                    .filter(|index| *index < self.main_window.playlist.len())
            });
        self.set_main_window_playlist_selection(selected_playlist_index, true);

        self.apply_playlist_selection();
        Ok(true)
    }

    pub(super) fn move_main_window_playlist_row(
        &mut self,
        from_index: usize,
        to_index: usize,
    ) -> Result<bool, String> {
        if !self.main_window.playback.can_manage_playlist {
            return Err(String::from(
                "Playlist row movement is unavailable when shared playlist controls are disabled.",
            ));
        }
        if from_index >= self.main_window.playlist.len()
            || to_index >= self.main_window.playlist.len()
        {
            return Err(String::from(
                "No playlist row exists at the requested index.",
            ));
        }
        if from_index == to_index {
            return Ok(false);
        }

        let active_entry_id = self
            .main_window
            .active_playlist_index
            .and_then(|index| self.main_window.playlist.get(index))
            .map(|row| row.entry_id);
        let current_index = self.selection.selected_main_window_playlist;
        let mut next_rows = self.main_window.playlist.clone();
        let moved_row = next_rows.remove(from_index);
        next_rows.insert(to_index, moved_row);
        let next_selection = current_index.map(|selected_index| {
            if selected_index == from_index {
                to_index
            } else if from_index < selected_index && selected_index <= to_index {
                selected_index - 1
            } else if to_index <= selected_index && selected_index < from_index {
                selected_index + 1
            } else {
                selected_index
            }
        });
        self.remember_shared_playlist_undo_snapshot_if_rows_changed(&next_rows);
        self.main_window.playlist = next_rows;
        self.main_window.active_playlist_index = active_entry_id.and_then(|entry_id| {
            self.main_window
                .playlist
                .iter()
                .position(|row| row.entry_id == entry_id)
        });
        self.set_main_window_playlist_selection(next_selection, true);

        self.apply_playlist_selection();
        Ok(true)
    }

    pub(super) fn shuffle_entire_shared_playlist(&mut self) -> Result<bool, String> {
        if !self.main_window.shared_playlist_enabled {
            return Err(
                "Shared playlist events are unavailable when shared playlists are disabled."
                    .to_owned(),
            );
        }
        let current_entries = self.current_shared_playlist_entries();
        if current_entries.is_empty() {
            return Err(String::from("The shared playlist is currently empty."));
        }
        let current_index = self.selection.selected_main_window_playlist.unwrap_or(0);
        let active_entry_id = self
            .main_window
            .active_playlist_index
            .and_then(|index| self.main_window.playlist.get(index))
            .map(|row| row.entry_id);
        let mut shuffled_rows = self.main_window.playlist.clone();
        let seed = self.next_shared_playlist_shuffle_seed(&current_entries, current_index, false);
        shuffle_playlist_entries_in_place(&mut shuffled_rows, seed);
        self.remember_shared_playlist_undo_snapshot_if_rows_changed(&shuffled_rows);
        self.main_window.playlist = shuffled_rows;
        self.main_window.active_playlist_index = active_entry_id.and_then(|entry_id| {
            self.main_window
                .playlist
                .iter()
                .position(|row| row.entry_id == entry_id)
        });
        self.set_main_window_playlist_selection(Some(0), true);

        self.apply_playlist_selection();
        Ok(true)
    }

    pub(super) fn replace_shared_playlist_entries_locally(
        &mut self,
        entries: Vec<String>,
    ) -> Result<bool, String> {
        if !self.main_window.shared_playlist_enabled {
            return Err(
                "Shared playlist events are unavailable when shared playlists are disabled."
                    .to_owned(),
            );
        }
        let entries = normalize_shared_playlist_entries(entries);
        let current_entries = self.current_shared_playlist_entries();
        let current_index = self.selection.selected_main_window_playlist;
        let target_index = if entries.is_empty() {
            None
        } else {
            Some(
                shared_playlist_target_index_from_changed_entries(
                    &current_entries,
                    current_index,
                    &entries,
                )
                .min(entries.len().saturating_sub(1)),
            )
        };
        self.remember_shared_playlist_undo_snapshot_if_changed(&entries);
        self.apply_shared_playlist_entries(entries.clone(), target_index, true);

        self.apply_playlist_selection();
        Ok(true)
    }
}
