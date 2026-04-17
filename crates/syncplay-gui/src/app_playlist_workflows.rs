use std::collections::BTreeSet;

use sha2::{Digest, Sha256};

use super::shell_state::{
    GuiMainWindowTab, GuiPlaylistTextEditSessionState, GuiTransientNotificationLevel,
    GuiUrlEditSessionState, MainWindowPlaylistRow, SyncplayGuiShellAppState,
    playlist_entries_multiline_text, shuffle_playlist_entries_in_place,
};
use super::support::normalized_editable_text;

impl SyncplayGuiShellAppState {
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

    pub(super) fn shared_playlist_drop_target_available(&self) -> bool {
        self.playlist_backed_media_opens_preferred()
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

    pub(super) fn shared_playlist_entries_after_media_open_from_state(
        &self,
        opened_entries: Vec<String>,
        insert_slot: Option<usize>,
    ) -> (Vec<String>, Option<usize>) {
        Self::shared_playlist_entries_after_media_open(
            &self.current_shared_playlist_entries(),
            self.selection.selected_main_window_playlist,
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
            self.playlist_undo_snapshot = Some(current_entries);
        }
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
        self.main_window.playlist = entries
            .iter()
            .map(|label| MainWindowPlaylistRow {
                label: label.clone(),
                is_selected: false,
            })
            .collect();
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
        let Some(previous_entries) = self.playlist_undo_snapshot.clone() else {
            return self.record_action_error("No shared playlist change is available to undo.");
        };
        if previous_entries == current_entries {
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
        self.apply_shared_playlist_entries(previous_entries, target_index, true);
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
        let mut shuffled_entries = current_entries.clone();
        let seed = self.next_shared_playlist_shuffle_seed(&current_entries, current_index, true);
        shuffle_playlist_entries_in_place(&mut shuffled_entries[shuffle_start..], seed);
        if shuffled_entries == current_entries {
            return self
                .record_action_error("No remaining shared playlist entries can be shuffled.");
        }
        self.remember_shared_playlist_undo_snapshot_if_changed(&shuffled_entries);
        self.apply_shared_playlist_entries(shuffled_entries, Some(current_index), true);
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
        let mut shuffled_entries = current_entries.clone();
        let seed = self.next_shared_playlist_shuffle_seed(&current_entries, current_index, false);
        shuffle_playlist_entries_in_place(&mut shuffled_entries, seed);
        self.remember_shared_playlist_undo_snapshot_if_changed(&shuffled_entries);
        self.apply_shared_playlist_entries(shuffled_entries, Some(0), true);
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
        self.select_main_window_tab(GuiMainWindowTab::Playlist);
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
        self.select_main_window_tab(GuiMainWindowTab::Playlist);
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

    pub(super) fn begin_media_url_edit(&mut self) -> bool {
        self.select_main_window_tab(GuiMainWindowTab::Playback);
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
