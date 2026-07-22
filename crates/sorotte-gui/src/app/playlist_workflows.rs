use std::collections::BTreeSet;

use sha2::{Digest, Sha256};

use super::shell_state::{
    GuiMediaMatchToolHealth, GuiMediaSourceProviderId, GuiPlaylistDefaultSourceId,
    GuiPlaylistDefaultSourceOption, GuiPlaylistDefaultSourceState, GuiPlaylistResolutionStep,
    GuiPlaylistSourceOption, GuiPlaylistSourcePolicy, GuiPlaylistSourceState,
    GuiPlaylistSourceStatus, GuiPlaylistTextEditSessionState, GuiPlexPlaylistSearchResult,
    GuiPlexPlaylistSearchState, GuiPluginSelection, GuiShellView, GuiTransientNotificationLevel,
    GuiUrlEditSessionState, MainWindowPlaylistRow, SorotteGuiShellAppState,
    playlist_entries_multiline_text, shuffle_playlist_entries_in_place,
};
use super::support::normalized_editable_text;

impl SorotteGuiShellAppState {
    pub(super) fn playlist_backed_media_opens_preferred(&self) -> bool {
        true
    }

    pub(super) fn shared_playlist_events_enabled(&self) -> bool {
        self.main_window.shared_playlist_enabled
    }

    pub(super) fn media_open_runtime_available(&self) -> bool {
        self.main_window.playback.can_toggle_pause
            || self.main_window.playback.can_seek
            || self.main_window.playback.can_manage_playlist
    }

    pub(super) fn ensure_shared_playlist_event_allowed(&mut self) -> bool {
        if self.shared_playlist_events_enabled() {
            true
        } else {
            self.record_action_error(
                "Shared playlist events are unavailable when shared playlists are disabled.",
            )
        }
    }

    pub(super) fn normalize_shared_playlist_entries(entries: Vec<String>) -> Vec<String> {
        entries
            .into_iter()
            .filter_map(|entry| normalized_editable_text(&entry))
            .collect()
    }

    pub(super) fn current_shared_playlist_entries(&self) -> Vec<String> {
        self.main_window
            .playlist
            .iter()
            .map(|row| row.label.clone())
            .collect()
    }

    pub(super) fn playlist_source_state_for_entry(&self, entry: &str) -> GuiPlaylistSourceState {
        let source_state = self
            .playlist_default_provider_for_new_entry(entry)
            .map(GuiPlaylistSourceState::for_playlist_default)
            .unwrap_or_else(|| GuiPlaylistSourceState::inferred_for_entry(entry));
        self.refreshed_playlist_source_state_for_entry(entry, source_state)
    }

    fn playlist_default_provider_for_new_entry(
        &self,
        entry: &str,
    ) -> Option<GuiMediaSourceProviderId> {
        let default_provider = self
            .main_window
            .playlist_default_source
            .current_source_id
            .provider_id()
            .cloned()?;
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

    pub(super) fn set_playlist_source_state(
        &mut self,
        index: usize,
        source_state: GuiPlaylistSourceState,
    ) -> bool {
        let Some(label) = self
            .main_window
            .playlist
            .get(index)
            .map(|row| row.label.clone())
        else {
            return false;
        };
        let mut source_state = self.refreshed_playlist_source_state_for_entry(&label, source_state);
        if let Some(row) = self.main_window.playlist.get_mut(index) {
            source_state.entry_id = row.entry_id;
            row.source_state = source_state;
            true
        } else {
            false
        }
    }

    pub(super) fn refresh_playlist_source_states(&mut self) {
        let refreshed_states = self
            .main_window
            .playlist
            .iter()
            .map(|row| {
                self.refreshed_playlist_source_state_for_entry(&row.label, row.source_state.clone())
            })
            .collect::<Vec<_>>();
        for (row, source_state) in self.main_window.playlist.iter_mut().zip(refreshed_states) {
            row.source_state = source_state;
        }
        self.main_window.playlist_default_source = self.refreshed_playlist_source_default_state(
            self.main_window.playlist_default_source.clone(),
        );
    }

    pub(super) fn select_main_window_playlist_source(
        &mut self,
        index: usize,
        provider_id: GuiMediaSourceProviderId,
    ) -> bool {
        let Some(label) = self
            .main_window
            .playlist
            .get(index)
            .map(|row| row.label.clone())
        else {
            return self.record_action_error("No playlist row exists at the requested index.");
        };
        let Some(option) = self
            .playlist_source_options_for_entry(&label, &provider_id)
            .into_iter()
            .find(|option| option.provider_id == provider_id)
        else {
            return self.record_action_error("The requested playlist source is not registered.");
        };
        if !option.enabled {
            return self.record_action_error(
                option
                    .detail
                    .unwrap_or_else(|| "The requested playlist source is disabled.".to_owned()),
            );
        }
        let mut source_state = GuiPlaylistSourceState::for_provider(option.provider_id.clone());
        source_state.current_label = option.label.clone();
        source_state.status = GuiPlaylistSourceStatus::Resolving;
        source_state.detail = Some(format!("Resolving with {}.", option.label));
        source_state.options.clear();
        source_state.resolution_steps = vec![GuiPlaylistResolutionStep {
            provider_id: option.provider_id,
            label: option.label,
            status: GuiPlaylistSourceStatus::Resolving,
            detail: Some("Explicitly requested for this client.".to_owned()),
        }];
        if !self.set_playlist_source_state(index, source_state) {
            return self.record_action_error("No playlist row exists at the requested index.");
        }
        self.set_main_window_playlist_selection(Some(index), true);
        self.apply_selection_to_surfaces();
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn select_main_window_playlist_default_source(
        &mut self,
        source_id: GuiPlaylistDefaultSourceId,
    ) -> bool {
        let Some(option) = self
            .playlist_source_default_options(&source_id)
            .into_iter()
            .find(|option| option.source_id == source_id)
        else {
            return self
                .record_action_error("The requested playlist default source is not registered.");
        };
        if !option.enabled {
            return self.record_action_error(option.detail.unwrap_or_else(|| {
                "The requested playlist default source is disabled.".to_owned()
            }));
        }
        self.main_window.playlist_default_source =
            self.refreshed_playlist_source_default_state(GuiPlaylistDefaultSourceState {
                current_source_id: option.source_id,
                current_label: option.label,
                options: Vec::new(),
            });
        self.clear_action_error_and_refresh();
        true
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

    fn playlist_source_options_for_entry(
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

    fn playlist_source_default_options(
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

    fn playlist_source_default_option(
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

    fn playlist_media_match_source_option(
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

    fn playlist_plex_stream_source_option(
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

    fn playlist_source_option(
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

    pub(super) fn unique_shared_playlist_additions(
        current_entries: &[String],
        entries: Vec<String>,
    ) -> Vec<String> {
        let mut seen_entries = current_entries.iter().cloned().collect::<BTreeSet<_>>();
        Self::normalize_shared_playlist_entries(entries)
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
            Self::unique_shared_playlist_additions(current_entries, opened_entries)
        } else {
            Self::normalize_shared_playlist_entries(opened_entries)
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
                    Self::shared_playlist_target_index_from_changed_entries(
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

    #[cfg(test)]
    pub(super) fn shared_playlist_entries_after_media_open_from_state(
        &self,
        opened_entries: Vec<String>,
        insert_slot: Option<usize>,
    ) -> (Vec<String>, Option<usize>) {
        self.shared_playlist_entries_after_media_open_from_state_with_current_index(
            opened_entries,
            insert_slot,
            self.selection.selected_main_window_playlist,
        )
    }

    pub(super) fn shared_playlist_entries_after_media_open_from_state_with_current_index(
        &self,
        opened_entries: Vec<String>,
        insert_slot: Option<usize>,
        current_index: Option<usize>,
    ) -> (Vec<String>, Option<usize>) {
        Self::shared_playlist_entries_after_media_open(
            &self.current_shared_playlist_entries(),
            current_index,
            opened_entries,
            insert_slot,
        )
    }

    pub(super) fn remember_shared_playlist_undo_snapshot_if_changed(
        &mut self,
        next_entries: &[String],
    ) {
        let current_entries = self.current_shared_playlist_entries();
        if current_entries != next_entries {
            self.remember_shared_playlist_undo_snapshot();
        }
    }

    pub(super) fn remember_shared_playlist_undo_snapshot_if_rows_changed(
        &mut self,
        next_rows: &[MainWindowPlaylistRow],
    ) {
        let labels_changed = self
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .ne(next_rows.iter().map(|row| row.label.as_str()));
        let entry_ids_changed = self
            .main_window
            .playlist
            .iter()
            .map(|row| row.entry_id)
            .ne(next_rows.iter().map(|row| row.entry_id));
        if labels_changed || entry_ids_changed {
            self.remember_shared_playlist_undo_snapshot();
        }
    }

    pub(super) fn remember_shared_playlist_undo_snapshot(&mut self) {
        self.playlist_source_undo_snapshot = Some(
            self.main_window
                .playlist
                .iter()
                .map(|row| row.source_state.clone())
                .collect(),
        );
        self.playlist_entry_id_undo_snapshot = Some(
            self.main_window
                .playlist
                .iter()
                .map(|row| row.entry_id)
                .collect(),
        );
        self.playlist_undo_snapshot = Some(self.current_shared_playlist_entries());
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
                && let Some(valid_index) =
                    next_entries.iter().position(|candidate| candidate == entry)
            {
                return valid_index;
            }
            index = index.saturating_add(1);
        }

        let mut index = current_index;
        while index > 0 {
            if let Some(entry) = current_entries.get(index)
                && let Some(valid_index) =
                    next_entries.iter().position(|candidate| candidate == entry)
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
        &mut self,
        entries: Vec<String>,
        selected_index: Option<usize>,
        selection_is_local: bool,
    ) {
        let current_entries = self.current_shared_playlist_entries();
        let active_entry_id = self
            .main_window
            .active_playlist_index
            .filter(|index| *index < current_entries.len())
            .and_then(|index| self.main_window.playlist.get(index))
            .map(|row| row.entry_id);
        let fallback_active_playlist_index = self
            .main_window
            .active_playlist_index
            .filter(|index| *index < current_entries.len())
            .map(|current_index| {
                Self::shared_playlist_target_index_from_changed_entries(
                    &current_entries,
                    Some(current_index),
                    &entries,
                )
                .min(entries.len().saturating_sub(1))
            });
        let previous_rows = self.main_window.playlist.clone();
        let mut used_previous_rows = vec![false; previous_rows.len()];
        self.main_window.playlist = entries
            .iter()
            .enumerate()
            .map(|(index, label)| {
                let previous_row = Self::reconciled_playlist_row(
                    &previous_rows,
                    &mut used_previous_rows,
                    index,
                    label,
                    None,
                );
                let source_state = previous_row
                    .as_ref()
                    .map(|row| {
                        self.refreshed_playlist_source_state_for_entry(
                            label,
                            row.source_state.clone(),
                        )
                    })
                    .unwrap_or_else(|| self.playlist_source_state_for_entry(label));
                MainWindowPlaylistRow {
                    entry_id: source_state.entry_id,
                    label: label.clone(),
                    is_selected: false,
                    source_state,
                }
            })
            .collect();
        self.main_window.active_playlist_index = active_entry_id
            .and_then(|entry_id| {
                self.main_window
                    .playlist
                    .iter()
                    .position(|row| row.entry_id == entry_id)
            })
            .or(fallback_active_playlist_index);
        self.set_main_window_playlist_selection(
            selected_index.filter(|index| *index < self.main_window.playlist.len()),
            selection_is_local,
        );
        self.apply_selection_to_surfaces();
    }

    pub(super) fn next_shared_playlist_shuffle_seed(
        &mut self,
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
        hasher.update(self.playlist_shuffle_nonce.to_le_bytes());
        for entry in entries {
            hasher.update(entry.as_bytes());
            hasher.update([0]);
        }
        self.playlist_shuffle_nonce = self.playlist_shuffle_nonce.wrapping_add(1);

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

    pub(super) fn selected_shared_playlist_entry(&self) -> Option<&str> {
        self.selection
            .selected_main_window_playlist
            .and_then(|index| self.main_window.playlist.get(index))
            .map(|row| row.label.as_str())
    }

    pub(super) fn replace_shared_playlist_entries_locally(&mut self, entries: Vec<String>) -> bool {
        if !self.ensure_shared_playlist_event_allowed() {
            return false;
        }
        let entries = Self::normalize_shared_playlist_entries(entries);
        let current_entries = self.current_shared_playlist_entries();
        let current_index = self.selection.selected_main_window_playlist;
        let target_index = if entries.is_empty() {
            None
        } else {
            Some(
                Self::shared_playlist_target_index_from_changed_entries(
                    &current_entries,
                    current_index,
                    &entries,
                )
                .min(entries.len().saturating_sub(1)),
            )
        };
        self.remember_shared_playlist_undo_snapshot_if_changed(&entries);
        self.apply_shared_playlist_entries(entries.clone(), target_index, true);
        let message = if entries.is_empty() {
            "Shared playlist cleared.".to_owned()
        } else {
            format!("Shared playlist updated ({} entries).", entries.len())
        };
        self.push_system_chat_message(message.clone());
        self.push_transient_notification(GuiTransientNotificationLevel::Success, message);
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn append_shared_playlist_entries_locally(&mut self, entries: Vec<String>) -> bool {
        if !self.ensure_shared_playlist_event_allowed() {
            return false;
        }
        let current_entries = self.current_shared_playlist_entries();
        let entries = Self::unique_shared_playlist_additions(&current_entries, entries);
        if entries.is_empty() {
            self.clear_action_error_and_refresh();
            return true;
        }
        let current_index = self.selection.selected_main_window_playlist;
        let mut playlist_entries = current_entries.clone();
        self.remember_shared_playlist_undo_snapshot_if_changed(
            &[playlist_entries.clone(), entries.clone()].concat(),
        );
        playlist_entries.extend(entries.iter().cloned());
        let selected_index = Some(
            Self::shared_playlist_target_index_from_changed_entries(
                &current_entries,
                current_index,
                &playlist_entries,
            )
            .min(playlist_entries.len().saturating_sub(1)),
        );
        self.apply_shared_playlist_entries(playlist_entries, selected_index, true);
        let message = if entries.len() == 1 {
            format!("Shared playlist entry added: {}.", entries[0])
        } else {
            format!("Shared playlist entries added: {} items.", entries.len())
        };
        self.push_system_chat_message(message.clone());
        self.push_transient_notification(GuiTransientNotificationLevel::Info, message);
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn undo_shared_playlist_change(&mut self) -> bool {
        if !self.ensure_shared_playlist_event_allowed() {
            return false;
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
        let Some(previous_entries) = self.playlist_undo_snapshot.clone() else {
            return self.record_action_error("No shared playlist change is available to undo.");
        };
        let previous_sources = self
            .playlist_source_undo_snapshot
            .clone()
            .filter(|sources| sources.len() == previous_entries.len());
        let previous_entry_ids = self
            .playlist_entry_id_undo_snapshot
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
            return self.record_action_error("No shared playlist change is available to undo.");
        }
        let current_index = self.selection.selected_main_window_playlist;
        let target_index = if previous_entries.is_empty() {
            None
        } else {
            Some(
                Self::shared_playlist_target_index_from_changed_entries(
                    &current_entries,
                    current_index,
                    &previous_entries,
                )
                .min(previous_entries.len().saturating_sub(1)),
            )
        };
        self.playlist_undo_snapshot = Some(current_entries);
        self.playlist_source_undo_snapshot = Some(current_sources);
        self.playlist_entry_id_undo_snapshot = Some(current_entry_ids);
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
        self.apply_selection_to_surfaces();
        self.push_system_chat_message("Shared playlist undo requested.".to_owned());
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            "Shared playlist undo requested.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn shuffle_remaining_shared_playlist(&mut self) -> bool {
        if !self.ensure_shared_playlist_event_allowed() {
            return false;
        }
        let Some(current_index) = self.selection.selected_main_window_playlist else {
            return self.record_action_error("No shared playlist entry is currently selected.");
        };
        let current_entries = self.current_shared_playlist_entries();
        if current_index >= current_entries.len() {
            return self.record_action_error("No shared playlist entry is currently selected.");
        }
        let shuffle_start = current_index.saturating_add(1);
        if shuffle_start >= current_entries.len() {
            return self
                .record_action_error("No remaining shared playlist entries can be shuffled.");
        }
        let active_entry_id = self
            .main_window
            .active_playlist_index
            .and_then(|index| self.main_window.playlist.get(index))
            .map(|row| row.entry_id);
        let mut shuffled_rows = self.main_window.playlist.clone();
        let seed = self.next_shared_playlist_shuffle_seed(&current_entries, current_index, true);
        shuffle_playlist_entries_in_place(&mut shuffled_rows[shuffle_start..], seed);
        if shuffled_rows.iter().map(|row| row.entry_id).eq(self
            .main_window
            .playlist
            .iter()
            .map(|row| row.entry_id))
        {
            return self
                .record_action_error("No remaining shared playlist entries can be shuffled.");
        }
        self.remember_shared_playlist_undo_snapshot_if_rows_changed(&shuffled_rows);
        self.main_window.playlist = shuffled_rows;
        self.main_window.active_playlist_index = active_entry_id.and_then(|entry_id| {
            self.main_window
                .playlist
                .iter()
                .position(|row| row.entry_id == entry_id)
        });
        self.set_main_window_playlist_selection(Some(current_index), true);
        self.apply_selection_to_surfaces();
        self.push_system_chat_message("Remaining shared playlist entries shuffled.".to_owned());
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            "Remaining shared playlist entries shuffled.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn shuffle_entire_shared_playlist(&mut self) -> bool {
        if !self.ensure_shared_playlist_event_allowed() {
            return false;
        }
        let current_entries = self.current_shared_playlist_entries();
        if current_entries.is_empty() {
            return self.record_action_error("The shared playlist is currently empty.");
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
        self.apply_selection_to_surfaces();
        self.push_system_chat_message("Shared playlist shuffled.".to_owned());
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            "Shared playlist shuffled.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn begin_shared_playlist_text_edit(&mut self) -> bool {
        if !self.ensure_shared_playlist_event_allowed() {
            return false;
        }
        self.active_view = GuiShellView::Room;
        self.playlist_text_edit_session = Some(GuiPlaylistTextEditSessionState {
            buffer: playlist_entries_multiline_text(&self.current_shared_playlist_entries()),
            is_dirty: false,
        });
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn update_shared_playlist_text_edit(&mut self, buffer: String) -> bool {
        let Some(session) = self.playlist_text_edit_session.as_mut() else {
            return self.record_action_error("No shared playlist text editor is currently active.");
        };
        session.buffer = buffer;
        session.is_dirty = true;
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn cancel_shared_playlist_text_edit(&mut self) -> bool {
        if self.playlist_text_edit_session.is_none() {
            return self.record_action_error("No shared playlist text editor is currently active.");
        }
        self.playlist_text_edit_session = None;
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn begin_shared_playlist_url_edit(&mut self) -> bool {
        if !self.ensure_shared_playlist_event_allowed() {
            return false;
        }
        self.active_view = GuiShellView::Room;
        self.playlist_url_edit_session = Some(GuiUrlEditSessionState {
            buffer: String::new(),
            is_dirty: false,
        });
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn update_shared_playlist_url_edit(&mut self, buffer: String) -> bool {
        let Some(session) = self.playlist_url_edit_session.as_mut() else {
            return self.record_action_error("No shared playlist URL editor is currently active.");
        };
        session.buffer = buffer;
        session.is_dirty = normalized_editable_text(&session.buffer).is_some();
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn cancel_shared_playlist_url_edit(&mut self) -> bool {
        if self.playlist_url_edit_session.is_none() {
            return self.record_action_error("No shared playlist URL editor is currently active.");
        }
        self.playlist_url_edit_session = None;
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn begin_plex_playlist_search(&mut self) -> bool {
        if !self.ensure_shared_playlist_event_allowed() {
            return false;
        }
        if !self.plex.authenticated
            || self
                .plex
                .selected_server_url
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
        {
            return self.record_action_error(
                "Select a Plex server before adding Plex media to the shared playlist.",
            );
        }
        self.active_view = GuiShellView::Room;
        self.plex_playlist_search = Some(GuiPlexPlaylistSearchState::default());
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn update_plex_playlist_search_query(&mut self, query: String) -> bool {
        let Some(search) = self.plex_playlist_search.as_mut() else {
            return self.record_action_error("No Plex playlist picker is currently active.");
        };
        search.query = query;
        search.error = None;
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn submit_plex_playlist_search(&mut self, query: String) -> bool {
        let Some(search) = self.plex_playlist_search.as_mut() else {
            return self.record_action_error("No Plex playlist picker is currently active.");
        };
        search.query = query;
        search.searching = true;
        search.adding_rating_key = None;
        search.error = None;
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn complete_plex_playlist_search(
        &mut self,
        query: String,
        results: Vec<GuiPlexPlaylistSearchResult>,
        error: Option<String>,
    ) -> bool {
        let Some(search) = self.plex_playlist_search.as_mut() else {
            return false;
        };
        if !search.searching || search.query.as_str() != query.as_str() {
            return false;
        }
        search.query = query;
        search.searching = false;
        search.adding_rating_key = None;
        search.error = error.and_then(|message| normalized_editable_text(&message));
        if search.error.is_some() {
            search.results.clear();
            search.selected_index = None;
        } else {
            search.results = results;
            search.selected_index = if search.results.is_empty() {
                None
            } else {
                Some(
                    search
                        .selected_index
                        .unwrap_or(0)
                        .min(search.results.len().saturating_sub(1)),
                )
            };
        }
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn select_plex_playlist_search_result(&mut self, index: usize) -> bool {
        let Some(search) = self.plex_playlist_search.as_mut() else {
            return self.record_action_error("No Plex playlist picker is currently active.");
        };
        if index >= search.results.len() {
            return self
                .record_action_error("No Plex playlist search result exists at that index.");
        }
        search.selected_index = Some(index);
        search.error = None;
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn add_selected_plex_playlist_search_result(&mut self) -> bool {
        let Some(search) = self.plex_playlist_search.as_mut() else {
            return self.record_action_error("No Plex playlist picker is currently active.");
        };
        let Some(index) = search.selected_index else {
            return self.record_action_error("No Plex playlist search result is selected.");
        };
        let Some(result) = search.results.get(index) else {
            return self
                .record_action_error("No Plex playlist search result exists at that index.");
        };
        search.adding_rating_key = Some(result.rating_key.clone());
        search.error = None;
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn complete_plex_playlist_item_resolve(
        &mut self,
        rating_key: String,
        error: Option<String>,
    ) -> bool {
        let Some(search) = self.plex_playlist_search.as_mut() else {
            return false;
        };
        if search.adding_rating_key.as_deref() != Some(rating_key.as_str()) {
            return false;
        }
        search.adding_rating_key = None;
        search.error = error.and_then(|message| normalized_editable_text(&message));
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn cancel_plex_playlist_search(&mut self) -> bool {
        if self.plex_playlist_search.is_none() {
            return self.record_action_error("No Plex playlist picker is currently active.");
        }
        self.plex_playlist_search = None;
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn begin_media_url_edit(&mut self) -> bool {
        self.active_view = GuiShellView::Room;
        self.media_url_edit_session = Some(GuiUrlEditSessionState {
            buffer: String::new(),
            is_dirty: false,
        });
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn update_media_url_edit(&mut self, buffer: String) -> bool {
        let Some(session) = self.media_url_edit_session.as_mut() else {
            return self.record_action_error("No open-URL editor is currently active.");
        };
        session.buffer = buffer;
        session.is_dirty = normalized_editable_text(&session.buffer).is_some();
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn cancel_media_url_edit(&mut self) -> bool {
        if self.media_url_edit_session.is_none() {
            return self.record_action_error("No open-URL editor is currently active.");
        }
        self.media_url_edit_session = None;
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn load_shared_playlist_from_file(
        &mut self,
        path: String,
        entries: Vec<String>,
        shuffled: bool,
    ) -> bool {
        self.remember_media_dialog_directory(&path);
        if !self.ensure_shared_playlist_event_allowed() {
            return false;
        }
        let mut entries = Self::normalize_shared_playlist_entries(entries);
        if shuffled && !entries.is_empty() {
            let seed = self.next_shared_playlist_shuffle_seed(&entries, 0, false);
            shuffle_playlist_entries_in_place(&mut entries, seed);
        }
        let target_index = (!entries.is_empty()).then_some(0);
        self.remember_shared_playlist_undo_snapshot_if_changed(&entries);
        self.apply_shared_playlist_entries(entries, target_index, true);
        let message = if shuffled {
            format!("Shared playlist loaded and shuffled from file: {path}.")
        } else {
            format!("Shared playlist loaded from file: {path}.")
        };
        self.push_system_chat_message(message.clone());
        self.push_transient_notification(GuiTransientNotificationLevel::Success, message);
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn save_shared_playlist_to_file(&mut self, path: String) -> bool {
        self.remember_media_dialog_directory(&path);
        self.push_system_chat_message(format!("Shared playlist saved to file: {path}."));
        self.push_transient_notification(
            GuiTransientNotificationLevel::Success,
            format!("Shared playlist saved to file: {path}."),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn announce_shared_playlist_loaded(&mut self, entries: Vec<String>) -> bool {
        if !self.ensure_shared_playlist_event_allowed() {
            return false;
        }
        let entries = Self::normalize_shared_playlist_entries(entries);
        self.remember_shared_playlist_undo_snapshot_if_changed(&entries);
        if entries.is_empty() {
            self.apply_shared_playlist_entries(Vec::new(), None, false);
            self.push_system_chat_message("Shared playlist cleared.".to_owned());
            self.push_transient_notification(
                GuiTransientNotificationLevel::Info,
                "Shared playlist cleared.".to_owned(),
            );
            self.clear_action_error_and_refresh();
            return true;
        }

        self.apply_shared_playlist_entries(entries, Some(0), false);
        self.push_system_chat_message(format!(
            "Shared playlist loaded ({} entries).",
            self.main_window.playlist.len()
        ));
        self.push_transient_notification(
            GuiTransientNotificationLevel::Success,
            format!(
                "Shared playlist loaded: {} entries.",
                self.main_window.playlist.len()
            ),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn announce_shared_playlist_entry_added(&mut self, entry: String) -> bool {
        if !self.ensure_shared_playlist_event_allowed() {
            return false;
        }
        let Some(entry) = normalized_editable_text(&entry) else {
            return self.record_action_error("Shared playlist entries must be non-empty.");
        };
        let current_entries = self.current_shared_playlist_entries();
        if current_entries.iter().any(|candidate| candidate == &entry) {
            self.clear_action_error_and_refresh();
            return true;
        }
        let current_index = self.selection.selected_main_window_playlist;
        let mut playlist_entries = current_entries.clone();
        playlist_entries.push(entry.clone());
        self.remember_shared_playlist_undo_snapshot_if_changed(&playlist_entries);
        let selected_index = Some(
            Self::shared_playlist_target_index_from_changed_entries(
                &current_entries,
                current_index,
                &playlist_entries,
            )
            .min(playlist_entries.len().saturating_sub(1)),
        );
        self.apply_shared_playlist_entries(playlist_entries, selected_index, false);
        self.push_system_chat_message(format!("Shared playlist entry added: {entry}."));
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            format!("Shared playlist entry added: {entry}."),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn announce_shared_playlist_selection_changed(&mut self, index: usize) -> bool {
        if !self.ensure_shared_playlist_event_allowed() {
            return false;
        }
        if index >= self.main_window.playlist.len() {
            return self
                .record_action_error("No shared playlist entry exists at the requested index.");
        }
        self.set_main_window_playlist_selection(Some(index), false);
        self.apply_selection_to_surfaces();
        let label = self.main_window.playlist[index].label.clone();
        self.push_system_chat_message(format!("Shared playlist selection changed: {label}."));
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            format!("Shared playlist selected: {label}."),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn announce_selected_shared_playlist_entry_removed(&mut self) -> bool {
        if !self.ensure_shared_playlist_event_allowed() {
            return false;
        }
        let Some(index) = self.selection.selected_main_window_playlist else {
            return self.record_action_error("No shared playlist entry is currently selected.");
        };
        let Some(entry) = self.main_window.playlist.get(index) else {
            return self
                .record_action_error("No shared playlist entry exists at the requested index.");
        };
        let label = entry.label.clone();
        let mut playlist_entries = self.current_shared_playlist_entries();
        playlist_entries.remove(index);
        self.remember_shared_playlist_undo_snapshot_if_changed(&playlist_entries);
        let next_selection = if playlist_entries.is_empty() {
            None
        } else if index >= playlist_entries.len() {
            Some(playlist_entries.len() - 1)
        } else {
            Some(index)
        };
        self.apply_shared_playlist_entries(playlist_entries, next_selection, false);
        self.push_system_chat_message(format!("Shared playlist entry removed: {label}."));
        self.push_transient_notification(
            GuiTransientNotificationLevel::Warning,
            format!("Shared playlist entry removed: {label}."),
        );
        self.clear_action_error_and_refresh();
        true
    }
}
