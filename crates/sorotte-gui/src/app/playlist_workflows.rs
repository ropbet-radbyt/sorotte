use super::shell_state::{
    GuiMediaSourceProviderId, GuiPlaylistDefaultSourceId, GuiPlaylistDefaultSourceOption,
    GuiPlaylistDefaultSourceState, GuiPlaylistResolutionStep, GuiPlaylistSourceOption,
    GuiPlaylistSourceState, GuiPlaylistSourceStatus, GuiPlaylistTextEditSessionState,
    GuiPlexPlaylistSearchResult, GuiPlexPlaylistSearchState, GuiShellView,
    GuiTransientNotificationLevel, GuiUrlEditSessionState, MainWindowPlaylistRow,
    SorotteGuiShellAppState, playlist_entries_multiline_text, shuffle_playlist_entries_in_place,
};
use super::support::normalized_editable_text;

impl SorotteGuiShellAppState {
    fn playlist_source_default_options(
        &self,
        selected_source_id: &GuiPlaylistDefaultSourceId,
    ) -> Vec<GuiPlaylistDefaultSourceOption> {
        self.playlist_source_context()
            .playlist_source_default_options(selected_source_id)
    }

    fn playlist_source_context(&self) -> super::playlist_model::GuiPlaylistSources<'_> {
        super::playlist_model::GuiPlaylistSources {
            default_source: self
                .main_window
                .playlist_default_source
                .current_source_id
                .clone(),
            media_match: &self.media_match,
            plex: &self.plex,
            plugin_enablement: self.plugin_enablement,
        }
    }

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
        super::playlist_model::normalize_shared_playlist_entries(entries)
    }

    pub(super) fn current_shared_playlist_entries(&self) -> Vec<String> {
        self.main_window
            .playlist
            .iter()
            .map(|row| row.label.clone())
            .collect()
    }

    #[cfg(test)]
    pub(super) fn playlist_source_state_for_entry(&self, entry: &str) -> GuiPlaylistSourceState {
        self.playlist_source_context()
            .playlist_source_state_for_entry(entry)
    }

    pub(super) fn refreshed_playlist_source_state_for_entry(
        &self,
        entry: &str,
        state: GuiPlaylistSourceState,
    ) -> GuiPlaylistSourceState {
        self.playlist_source_context()
            .refreshed_playlist_source_state_for_entry(entry, state)
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

    fn playlist_source_options_for_entry(
        &self,
        entry: &str,
        selected_provider_id: &GuiMediaSourceProviderId,
    ) -> Vec<GuiPlaylistSourceOption> {
        self.playlist_source_context()
            .playlist_source_options_for_entry(entry, selected_provider_id)
    }

    pub(super) fn refreshed_playlist_source_default_state(
        &self,
        state: GuiPlaylistDefaultSourceState,
    ) -> GuiPlaylistDefaultSourceState {
        self.playlist_source_context()
            .refreshed_playlist_source_default_state(state)
    }

    pub(super) fn unique_shared_playlist_additions(
        current_entries: &[String],
        entries: Vec<String>,
    ) -> Vec<String> {
        super::playlist_model::unique_shared_playlist_additions(current_entries, entries)
    }

    #[cfg(test)]
    pub(super) fn shared_playlist_entries_after_media_open(
        current_entries: &[String],
        current_index: Option<usize>,
        opened_entries: Vec<String>,
        insert_slot: Option<usize>,
    ) -> (Vec<String>, Option<usize>) {
        super::playlist_model::shared_playlist_entries_after_media_open(
            current_entries,
            current_index,
            opened_entries,
            insert_slot,
        )
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

    #[cfg(test)]
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
        super::playlist_model::shared_playlist_target_index_from_changed_entries(
            current_entries,
            current_index,
            next_entries,
        )
    }

    pub(super) fn apply_shared_playlist_entries(
        &mut self,
        entries: Vec<String>,
        selected_index: Option<usize>,
        selection_is_local: bool,
    ) {
        let sources = super::playlist_model::GuiPlaylistSources {
            default_source: self
                .main_window
                .playlist_default_source
                .current_source_id
                .clone(),
            media_match: &self.media_match,
            plex: &self.plex,
            plugin_enablement: self.plugin_enablement,
        };
        super::playlist_model::apply_shared_playlist_entries(
            &mut self.main_window,
            &mut self.selection,
            &mut self.main_window_playlist_selection_is_local,
            &sources,
            entries,
            selected_index,
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
        super::playlist_model::next_shared_playlist_shuffle_seed(
            &mut self.playlist_shuffle_nonce,
            entries,
            current_index,
            shuffle_scope_remaining,
        )
    }

    pub(super) fn selected_shared_playlist_entry(&self) -> Option<&str> {
        self.selection
            .selected_main_window_playlist
            .and_then(|index| self.main_window.playlist.get(index))
            .map(|row| row.label.as_str())
    }

    pub(super) fn replace_shared_playlist_entries_locally(&mut self, entries: Vec<String>) -> bool {
        match self
            .playlist_edit_model()
            .replace_shared_playlist_entries_locally(entries)
        {
            Ok(applied) => {
                if applied {
                    let message = if self.main_window.playlist.is_empty() {
                        "Shared playlist cleared.".to_owned()
                    } else {
                        format!(
                            "Shared playlist updated ({} entries).",
                            self.main_window.playlist.len()
                        )
                    };
                    self.push_system_chat_message(message.clone());
                    self.push_transient_notification(
                        GuiTransientNotificationLevel::Success,
                        message,
                    );
                }
                self.apply_selection_to_surfaces();
                self.clear_action_error_and_refresh();
                applied
            }
            Err(message) => self.record_action_error(message),
        }
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
        match self.playlist_edit_model().undo_shared_playlist_change() {
            Ok(applied) => {
                if applied {
                    self.push_system_chat_message("Shared playlist undo requested.".to_owned());
                    self.push_transient_notification(
                        GuiTransientNotificationLevel::Info,
                        "Shared playlist undo requested.".to_owned(),
                    );
                }
                self.apply_selection_to_surfaces();
                self.clear_action_error_and_refresh();
                applied
            }
            Err(message) => self.record_action_error(message),
        }
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
        match self.playlist_edit_model().shuffle_entire_shared_playlist() {
            Ok(applied) => {
                if applied {
                    self.push_system_chat_message("Shared playlist shuffled.".to_owned());
                    self.push_transient_notification(
                        GuiTransientNotificationLevel::Info,
                        "Shared playlist shuffled.".to_owned(),
                    );
                }
                self.apply_selection_to_surfaces();
                self.clear_action_error_and_refresh();
                applied
            }
            Err(message) => self.record_action_error(message),
        }
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
        let applied = super::feature_snapshots::complete_plex_playlist_search(
            &mut self.plex_playlist_search,
            query,
            results,
            error,
        );
        if applied {
            self.clear_action_error_and_refresh();
        }
        applied
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
        let applied = super::feature_snapshots::complete_plex_playlist_item_resolve(
            &mut self.plex_playlist_search,
            rating_key,
            error,
        );
        if applied {
            self.clear_action_error_and_refresh();
        }
        applied
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

impl SorotteGuiShellAppState {
    pub(in crate::app) fn playlist_edit_model(
        &mut self,
    ) -> crate::app::playlist_model::GuiPlaylistEditing<'_> {
        let sources = crate::app::playlist_model::GuiPlaylistSources {
            default_source: self
                .main_window
                .playlist_default_source
                .current_source_id
                .clone(),
            media_match: &self.media_match,
            plex: &self.plex,
            plugin_enablement: self.plugin_enablement,
        };
        crate::app::playlist_model::GuiPlaylistEditing {
            main_window: &mut self.main_window,
            selection: &mut self.selection,
            playlist_undo_snapshot: &mut self.playlist_undo_snapshot,
            playlist_source_undo_snapshot: &mut self.playlist_source_undo_snapshot,
            playlist_entry_id_undo_snapshot: &mut self.playlist_entry_id_undo_snapshot,
            selection_is_local: &mut self.main_window_playlist_selection_is_local,
            shuffle_nonce: &mut self.playlist_shuffle_nonce,
            sources,
        }
    }
}
