use super::*;

impl SorotteGuiShellAppState {
    pub(super) fn apply_main_window_action(&mut self, action: GuiShellAction) -> bool {
        match action {
            GuiShellAction::UpdateNewMainWindowUserDraft(buffer) => {
                self.new_main_window_user_draft = buffer;
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::CommitNewMainWindowUser => {
                if !self.announce_main_window_user_joined(self.new_main_window_user_draft.clone()) {
                    return false;
                }
                self.new_main_window_user_draft.clear();
                true
            }
            GuiShellAction::AppendSharedPlaylistEntries(entries) => {
                self.append_shared_playlist_entries_locally(entries)
            }
            GuiShellAction::ReplaceSharedPlaylistEntries(entries) => {
                self.replace_shared_playlist_entries_locally(entries)
            }
            GuiShellAction::LoadSharedPlaylistFromFile {
                path,
                entries,
                shuffled,
            } => self.load_shared_playlist_from_file(path, entries, shuffled),
            GuiShellAction::SaveSharedPlaylistToFile(path) => {
                self.save_shared_playlist_to_file(path)
            }
            GuiShellAction::PushTransientNotification { level, message } => {
                let trimmed = message.trim();
                if trimmed.is_empty() {
                    return self
                        .record_action_error("Transient notification messages must be non-empty.");
                }
                self.push_transient_notification(level, trimmed.to_owned());
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::DismissTransientNotification(index) => {
                if index >= self.notifications.len() {
                    return self.record_action_error(
                        "No transient notification exists at the requested index.",
                    );
                }
                self.notifications.remove(index);
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::DismissSetupAlert => {
                let had_action_error = self.validation.last_action_error.take().is_some();
                let notification_index = self.notifications.iter().rposition(|notification| {
                    !matches!(notification.level, GuiTransientNotificationLevel::Info)
                });
                let had_notification = notification_index
                    .map(|index| self.notifications.remove(index))
                    .is_some();
                self.refresh_validation();
                had_action_error || had_notification
            }
            GuiShellAction::ClearTransientNotifications => {
                let had_notifications = !self.notifications.is_empty();
                self.notifications.clear();
                if had_notifications {
                    self.clear_action_error_and_refresh();
                }
                had_notifications
            }
            GuiShellAction::BeginConfigurationTextEdit { section, label } => {
                let Some(control) = self.configuration.control(section, label) else {
                    return self.record_action_error(
                        "No editable configuration control exists at the requested location.",
                    );
                };
                if !control.kind.is_editable() || control.kind == GuiDialogControlKind::Checkbox {
                    return self.record_action_error(
                        "The requested configuration control does not support text-edit sessions.",
                    );
                }
                self.text_edit_session = Some(GuiTextEditSessionState {
                    section,
                    label,
                    buffer: control.value.clone(),
                    is_dirty: false,
                });
                if let Some(tab) = Self::configuration_tab_for_section(section) {
                    self.select_configuration_tab(tab);
                }
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::UpdateConfigurationTextEdit(buffer) => {
                let Some(session) = self.text_edit_session.as_mut() else {
                    return self.record_action_error(
                        "No configuration text-edit session is currently active.",
                    );
                };
                let current_value = self
                    .configuration
                    .control_value(session.section, session.label)
                    .unwrap_or("(missing)")
                    .to_owned();
                session.is_dirty = buffer != current_value;
                session.buffer = buffer;
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::CommitConfigurationTextEdit => {
                let Some(session) = self.text_edit_session.clone() else {
                    return self.record_action_error(
                        "No configuration text-edit session is currently active.",
                    );
                };
                let previous_settings = self.configuration.to_stored_settings();
                let applied = self.configuration.apply_text_value(
                    session.section,
                    session.label,
                    &session.buffer,
                );
                if !applied {
                    return self.record_action_error(
                        "Configuration text-edit session could not be committed.",
                    );
                }
                self.text_edit_session = None;
                self.sync_derived_surfaces_from_configuration_settings(&previous_settings);
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::CancelConfigurationTextEdit => {
                if self.text_edit_session.is_none() {
                    return self.record_action_error(
                        "No configuration text-edit session is currently active.",
                    );
                }
                self.text_edit_session = None;
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::BeginRoomHistoryEdit => self.begin_room_history_edit(),
            GuiShellAction::UpdateRoomHistoryEdit(buffer) => self.update_room_history_edit(buffer),
            GuiShellAction::CommitRoomHistoryEdit => self.commit_room_history_edit(),
            GuiShellAction::CancelRoomHistoryEdit => self.cancel_room_history_edit(),
            GuiShellAction::BeginSharedPlaylistTextEdit => self.begin_shared_playlist_text_edit(),
            GuiShellAction::UpdateSharedPlaylistTextEdit(buffer) => {
                self.update_shared_playlist_text_edit(buffer)
            }
            GuiShellAction::CancelSharedPlaylistTextEdit => self.cancel_shared_playlist_text_edit(),
            GuiShellAction::BeginSharedPlaylistUrlEdit => self.begin_shared_playlist_url_edit(),
            GuiShellAction::UpdateSharedPlaylistUrlEdit(buffer) => {
                self.update_shared_playlist_url_edit(buffer)
            }
            GuiShellAction::CancelSharedPlaylistUrlEdit => self.cancel_shared_playlist_url_edit(),
            GuiShellAction::BeginPlexPlaylistSearch => self.begin_plex_playlist_search(),
            GuiShellAction::UpdatePlexPlaylistSearchQuery(query) => {
                self.update_plex_playlist_search_query(query)
            }
            GuiShellAction::SubmitPlexPlaylistSearch { query } => {
                self.submit_plex_playlist_search(query)
            }
            GuiShellAction::CompletePlexPlaylistSearch {
                query,
                results,
                error,
            } => self.complete_plex_playlist_search(query, results, error),
            GuiShellAction::SelectPlexPlaylistSearchResult(index) => {
                self.select_plex_playlist_search_result(index)
            }
            GuiShellAction::AddSelectedPlexPlaylistSearchResult => {
                self.add_selected_plex_playlist_search_result()
            }
            GuiShellAction::CompletePlexPlaylistItemResolve { rating_key, error } => {
                self.complete_plex_playlist_item_resolve(rating_key, error)
            }
            GuiShellAction::CancelPlexPlaylistSearch => self.cancel_plex_playlist_search(),
            GuiShellAction::BeginMediaUrlEdit => self.begin_media_url_edit(),
            GuiShellAction::UpdateMediaUrlEdit(buffer) => self.update_media_url_edit(buffer),
            GuiShellAction::CancelMediaUrlEdit => self.cancel_media_url_edit(),
            GuiShellAction::BeginCreateControlledRoomEdit => {
                self.begin_create_controlled_room_edit()
            }
            GuiShellAction::UpdateCreateControlledRoomEdit(buffer) => {
                self.update_create_controlled_room_edit(buffer)
            }
            GuiShellAction::CancelCreateControlledRoomEdit => {
                self.cancel_create_controlled_room_edit()
            }
            GuiShellAction::BeginControllerAuthEdit => self.begin_controller_auth_edit(),
            GuiShellAction::UpdateControllerAuthPasswordEdit(buffer) => {
                self.update_controller_auth_password_edit(buffer)
            }
            GuiShellAction::CancelControllerAuthEdit => self.cancel_controller_auth_edit(),
            GuiShellAction::SelectMainWindowUser(index) => {
                if index >= self.main_window.users.len() {
                    return self
                        .record_action_error("No main-window user exists at the requested index.");
                }
                self.selection.selected_main_window_user = Some(index);
                self.apply_selection_to_surfaces();
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::AddMainWindowUser(username) => self.add_main_window_user(username),
            GuiShellAction::AnnounceMainWindowUserJoined(username) => {
                self.announce_main_window_user_joined(username)
            }
            GuiShellAction::AnnounceSelectedMainWindowUserRenamed(username) => {
                self.announce_selected_main_window_user_renamed(username)
            }
            GuiShellAction::AnnounceSelectedMainWindowUserLeft => {
                self.announce_selected_main_window_user_left()
            }
            GuiShellAction::BeginPlaybackPause => self.begin_playback_pause_state(true),
            GuiShellAction::BeginPlaybackResume => self.begin_playback_pause_state(false),
            GuiShellAction::BeginPlaybackPauseToggle => self.begin_playback_pause_toggle(),
            GuiShellAction::CompletePlaybackPauseState(paused) => {
                self.complete_playback_pause_state(paused)
            }
            GuiShellAction::CancelPlaybackPauseState => self.cancel_playback_pause_state(),
            GuiShellAction::CompletePlaybackPauseToggle => self.complete_playback_pause_toggle(),
            GuiShellAction::CancelPlaybackPauseToggle => self.cancel_playback_pause_toggle(),
            GuiShellAction::AnnouncePlaybackPaused => self.announce_playback_pause_state(true),
            GuiShellAction::AnnouncePlaybackResumed => self.announce_playback_pause_state(false),
            GuiShellAction::RequestSeekPrompt
            | GuiShellAction::RequestOffsetPrompt
            | GuiShellAction::RequestPlaybackUndoSeek => {
                self.request_main_window_playback_control()
            }
            GuiShellAction::AnnounceLocalUserReady => self.announce_local_user_ready_state(true),
            GuiShellAction::AnnounceLocalUserNotReady => {
                self.announce_local_user_ready_state(false)
            }
            GuiShellAction::AnnounceAutoplayState(active) => self.announce_autoplay_state(active),
            GuiShellAction::AnnounceAutoplayThreshold(threshold) => {
                self.announce_autoplay_threshold(threshold)
            }
            GuiShellAction::AnnounceSharedPlaylistLoaded(entries) => {
                self.announce_shared_playlist_loaded(entries)
            }
            GuiShellAction::AnnounceSharedPlaylistEntryAdded(entry) => {
                self.announce_shared_playlist_entry_added(entry)
            }
            GuiShellAction::AnnounceSharedPlaylistSelectionChanged(index) => {
                self.announce_shared_playlist_selection_changed(index)
            }
            GuiShellAction::AnnounceSelectedSharedPlaylistEntryRemoved => {
                self.announce_selected_shared_playlist_entry_removed()
            }
            GuiShellAction::UndoSharedPlaylistChange => self.undo_shared_playlist_change(),
            GuiShellAction::ShuffleRemainingSharedPlaylist => {
                self.shuffle_remaining_shared_playlist()
            }
            GuiShellAction::ShuffleEntireSharedPlaylist => self.shuffle_entire_shared_playlist(),
            GuiShellAction::BeginLocalChatSend(message) => self.begin_local_chat_send(message),
            GuiShellAction::CompleteLocalChatSend => self.complete_local_chat_send(),
            GuiShellAction::CancelLocalChatSend => self.cancel_local_chat_send(),
            GuiShellAction::AnnounceRemoteChatMessage { sender, message } => {
                self.announce_remote_chat_message(sender, message)
            }
            GuiShellAction::AnnounceSystemChatEvent(message) => {
                self.announce_system_chat_event(message)
            }
            GuiShellAction::ToggleSelectedMainWindowUserReady => {
                self.toggle_selected_main_window_user_ready()
            }
            GuiShellAction::ToggleSelectedMainWindowUserController => {
                self.toggle_selected_main_window_user_controller()
            }
            GuiShellAction::RemoveSelectedMainWindowUser => self.remove_selected_main_window_user(),
            GuiShellAction::SelectMainWindowPlaylist(index)
            | GuiShellAction::ActivateMainWindowPlaylist(index) => {
                if index >= self.main_window.playlist.len() {
                    return self
                        .record_action_error("No playlist row exists at the requested index.");
                }
                self.set_main_window_playlist_selection(Some(index), true);
                self.apply_selection_to_surfaces();
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::MoveMainWindowPlaylistRow {
                from_index,
                to_index,
            } => self.move_main_window_playlist_row(from_index, to_index),
            GuiShellAction::MoveSelectedMainWindowPlaylistUp => {
                self.move_selected_main_window_playlist(-1)
            }
            GuiShellAction::MoveSelectedMainWindowPlaylistDown => {
                self.move_selected_main_window_playlist(1)
            }
            GuiShellAction::RemoveSelectedMainWindowPlaylist => {
                self.remove_selected_main_window_playlist()
            }
            _ => unreachable!("action routed to wrong reducer domain"),
        }
    }
}
