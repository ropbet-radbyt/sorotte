use syncplay_client_app::app_boundary::state::parse_host_and_optional_port_from_host_arg_legacy_compatible;

use super::shell_state::{
    GuiDialogControlKind, GuiFocusedConfigurationControlState, GuiMainWindowUserEditSessionState,
    GuiPendingOperationKind, GuiPendingOperationState, GuiPublicServerEditSessionState,
    GuiShellAction, GuiTextEditSessionState, GuiTransientNotificationLevel,
    SyncplayGuiShellAppState,
};
use super::support::normalized_editable_text;

impl SyncplayGuiShellAppState {
    pub(super) fn apply(&mut self, action: GuiShellAction) -> bool {
        match action {
            GuiShellAction::SwitchView(view) => {
                self.active_view = view;
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::OpenModal(modal) => {
                self.open_modal = Some(modal);
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::CloseModal => self.close_modal_window(),
            GuiShellAction::DismissUpdateNotice => self.dismiss_update_notice(),
            GuiShellAction::BeginUpdateCheck { user_initiated } => {
                self.begin_update_check(user_initiated)
            }
            GuiShellAction::ApplyUpdateCheckResult(result) => {
                self.apply_update_check_result(result)
            }
            GuiShellAction::ApplyStartupPublicServerCache(servers) => {
                self.apply_startup_public_server_cache(servers)
            }
            GuiShellAction::TrustTlsCertificatePrompt => self.complete_tls_certificate_prompt(true),
            GuiShellAction::RejectTlsCertificatePrompt => {
                self.complete_tls_certificate_prompt(false)
            }
            GuiShellAction::TriggerSelectedMenuAction => self.trigger_selected_menu_action(),
            GuiShellAction::AnnounceTlsCertificatePromptRequired => {
                self.announce_tls_certificate_prompt_required()
            }
            GuiShellAction::AnnounceUpdateNoticeAvailable => {
                self.announce_update_notice_available()
            }
            GuiShellAction::AnnounceAboutDialogRequested => self.announce_about_dialog_requested(),
            GuiShellAction::AnnounceHelpRequested => self.announce_help_requested(),
            GuiShellAction::ApplyMenuDialogRuntimeSnapshot(snapshot) => {
                self.apply_menu_dialog_runtime_snapshot(snapshot)
            }
            GuiShellAction::ApplyGuiFeedbackRuntimeSnapshot(snapshot) => {
                self.apply_gui_feedback_runtime_snapshot(snapshot)
            }
            GuiShellAction::ApplyGuiErrorRuntimeSnapshot(snapshot) => {
                self.apply_gui_error_runtime_snapshot(snapshot)
            }
            GuiShellAction::ApplyGuiCommandRuntimeSnapshot(snapshot) => {
                self.apply_gui_command_runtime_snapshot(snapshot)
            }
            GuiShellAction::ApplyGuiInteractionRuntimeSnapshot(snapshot) => {
                self.apply_gui_interaction_runtime_snapshot(snapshot)
            }
            GuiShellAction::ApplyGuiDraftRuntimeSnapshot(snapshot) => {
                self.apply_gui_draft_runtime_snapshot(snapshot)
            }
            GuiShellAction::ApplyGuiConfigurationDraftRuntimeSnapshot(snapshot) => {
                self.apply_gui_configuration_draft_runtime_snapshot(snapshot)
            }
            GuiShellAction::ApplyGuiSavedConfigurationRuntimeSnapshot(snapshot) => {
                self.apply_gui_saved_configuration_runtime_snapshot(snapshot)
            }
            GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(snapshot) => {
                self.apply_gui_configuration_runtime_snapshot(snapshot)
            }
            GuiShellAction::BeginConfigurationSave => self.begin_configuration_save(),
            GuiShellAction::CompleteConfigurationSave(settings) => {
                self.complete_configuration_save(settings)
            }
            GuiShellAction::CancelConfigurationSave => self.cancel_configuration_save(),
            GuiShellAction::BeginConfigurationReset => self.begin_configuration_reset(),
            GuiShellAction::CompleteConfigurationReset(settings) => {
                self.complete_configuration_reset(settings)
            }
            GuiShellAction::CancelConfigurationReset => self.cancel_configuration_reset(),
            GuiShellAction::BeginConfigurationReload => self.begin_configuration_reload(),
            GuiShellAction::CompleteConfigurationReload(settings) => {
                self.complete_configuration_reload(settings)
            }
            GuiShellAction::CancelConfigurationReload => self.cancel_configuration_reload(),
            GuiShellAction::BeginClearGuiData => self.begin_clear_gui_data(),
            GuiShellAction::CompleteClearGuiData => self.complete_clear_gui_data(),
            GuiShellAction::CancelClearGuiData => self.cancel_clear_gui_data(),
            GuiShellAction::BeginPendingOperation(kind) => {
                if self.pending_operation.is_some() {
                    return self
                        .record_action_error("Another GUI operation is already in progress.");
                }
                self.pending_operation = Some(GuiPendingOperationState { kind });
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::CompletePendingOperation => {
                let Some(pending) = self.pending_operation.as_ref() else {
                    return self.record_action_error("No GUI operation is currently in progress.");
                };
                if pending.kind == GuiPendingOperationKind::SendChatMessage {
                    self.outgoing_chat_message = None;
                }
                self.pending_operation = None;
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::CancelPendingOperation => self.cancel_pending_operation(),
            GuiShellAction::FocusConfigurationControl { section, label } => {
                let Some(control) = self.configuration.control(section, label) else {
                    return self.record_action_error(
                        "No editable configuration control exists at the requested location.",
                    );
                };
                if !control.kind.is_editable() {
                    return self.record_action_error(
                        "The requested configuration control is not focusable.",
                    );
                }
                let activation_count = self
                    .focused_configuration_control
                    .as_ref()
                    .filter(|focused| focused.section == section && focused.label == label)
                    .map_or(0, |focused| focused.activation_count);
                self.focused_configuration_control = Some(GuiFocusedConfigurationControlState {
                    section,
                    label,
                    kind: control.kind,
                    activation_count,
                });
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::ActivateFocusedConfigurationControl => {
                let Some(focused) = self.focused_configuration_control.clone() else {
                    return self
                        .record_action_error("No configuration control is currently focused.");
                };

                if focused.kind == GuiDialogControlKind::Checkbox {
                    let Some(current_value) = self
                        .configuration
                        .control_value(focused.section, focused.label)
                    else {
                        return self.record_action_error(
                            "The focused configuration control no longer exists.",
                        );
                    };
                    let next_value = current_value != "yes";
                    let previous_settings = self.configuration.to_stored_settings();
                    let applied = self.configuration.apply_bool_value(
                        focused.section,
                        focused.label,
                        next_value,
                    );
                    if !applied {
                        return self.record_action_error(
                            "The focused checkbox control could not be toggled.",
                        );
                    }
                    if let Some(focused_state) = self.focused_configuration_control.as_mut() {
                        focused_state.activation_count += 1;
                    }
                    self.sync_derived_surfaces_from_configuration_settings(&previous_settings);
                    self.clear_action_error_and_refresh();
                    return true;
                }

                let Some(control) = self.configuration.control(focused.section, focused.label)
                else {
                    return self.record_action_error(
                        "The focused configuration control no longer exists.",
                    );
                };
                self.text_edit_session = Some(GuiTextEditSessionState {
                    section: focused.section,
                    label: focused.label,
                    buffer: control.value.clone(),
                    is_dirty: false,
                });
                if let Some(focused_state) = self.focused_configuration_control.as_mut() {
                    focused_state.activation_count += 1;
                }
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::ClearConfigurationControlFocus => {
                let had_focus = self.focused_configuration_control.is_some();
                self.focused_configuration_control = None;
                if had_focus {
                    self.clear_action_error_and_refresh();
                }
                had_focus
            }
            GuiShellAction::BeginAddPublicServer => {
                self.public_server_edit_session = Some(GuiPublicServerEditSessionState {
                    editing_index: None,
                    label_buffer: String::new(),
                    address_buffer: String::new(),
                    is_dirty: false,
                    original_label: None,
                    original_address: None,
                });
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::BeginEditSelectedPublicServer => {
                let Some(index) = self.selected_public_server_index() else {
                    return self.record_action_error("No public server is currently selected.");
                };
                let Some(row) = self.public_servers.servers.get(index) else {
                    return self
                        .record_action_error("No public server exists at the requested index.");
                };
                self.public_server_edit_session = Some(GuiPublicServerEditSessionState {
                    editing_index: Some(index),
                    label_buffer: row.label.clone(),
                    address_buffer: row.address.clone(),
                    is_dirty: false,
                    original_label: Some(row.label.clone()),
                    original_address: Some(row.address.clone()),
                });
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::UpdatePublicServerEditLabel(buffer) => {
                let Some(session) = self.public_server_edit_session.as_mut() else {
                    return self
                        .record_action_error("No public-server edit session is currently active.");
                };
                session.label_buffer = buffer;
                session.is_dirty = true;
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::UpdatePublicServerEditAddress(buffer) => {
                let Some(session) = self.public_server_edit_session.as_mut() else {
                    return self
                        .record_action_error("No public-server edit session is currently active.");
                };
                session.address_buffer = buffer;
                session.is_dirty = true;
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::CommitPublicServerEdit => {
                let Some(session) = self.public_server_edit_session.clone() else {
                    return self
                        .record_action_error("No public-server edit session is currently active.");
                };
                let label = session.label_buffer.trim();
                let address = session.address_buffer.trim();
                if label.is_empty() || address.is_empty() {
                    return self.record_action_error(
                        "Public-server label and address must both be non-empty.",
                    );
                }
                let (host, _) =
                    parse_host_and_optional_port_from_host_arg_legacy_compatible(address);
                if host.trim().is_empty() {
                    return self.record_action_error("Public-server address is not valid.");
                }

                let mut settings = self.configuration.to_stored_settings();
                let mut servers = settings.public_servers.take().unwrap_or_default();
                if let Some(index) = session.editing_index {
                    if index >= servers.len() {
                        return self.record_action_error(
                            "The public server being edited no longer exists.",
                        );
                    }
                    servers[index] = (label.to_owned(), address.to_owned());
                } else {
                    servers.push((label.to_owned(), address.to_owned()));
                }
                let selected_index = session.editing_index.unwrap_or(servers.len() - 1);
                settings.public_servers = Some(servers);
                self.resync_from_settings(settings);
                self.public_server_edit_session = None;
                if !self.apply_public_server_selection(selected_index) {
                    return false;
                }
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::CancelPublicServerEdit => {
                if self.public_server_edit_session.is_none() {
                    return self
                        .record_action_error("No public-server edit session is currently active.");
                }
                self.public_server_edit_session = None;
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::RemoveSelectedPublicServer => {
                let Some(index) = self.selected_public_server_index() else {
                    return self.record_action_error("No public server is currently selected.");
                };
                let mut settings = self.configuration.to_stored_settings();
                let mut servers = settings.public_servers.take().unwrap_or_default();
                if index >= servers.len() {
                    return self
                        .record_action_error("No public server exists at the requested index.");
                }
                servers.remove(index);
                settings.public_servers = if servers.is_empty() {
                    None
                } else {
                    Some(servers)
                };
                self.resync_from_settings(settings);
                if self.public_servers.servers.is_empty() {
                    self.set_selected_public_server_index(None);
                } else if index >= self.public_servers.servers.len() {
                    self.set_selected_public_server_index(Some(
                        self.public_servers.servers.len() - 1,
                    ));
                } else {
                    self.set_selected_public_server_index(Some(index));
                }
                self.public_server_edit_session = None;
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::BeginEditSelectedMainWindowUser => {
                let Some(index) = self.selection.selected_main_window_user else {
                    return self.record_action_error("No main-window user is currently selected.");
                };
                let Some(user) = self.main_window.users.get(index) else {
                    return self
                        .record_action_error("No main-window user exists at the requested index.");
                };
                self.main_window_user_edit_session = Some(GuiMainWindowUserEditSessionState {
                    editing_index: index,
                    username_buffer: user.username.clone(),
                    is_dirty: false,
                    original_username: user.username.clone(),
                });
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::UpdateMainWindowUserEdit(buffer) => {
                let Some(session) = self.main_window_user_edit_session.as_mut() else {
                    return self.record_action_error(
                        "No main-window user edit session is currently active.",
                    );
                };
                let Some(user) = self.main_window.users.get(session.editing_index) else {
                    self.main_window_user_edit_session = None;
                    return self.record_action_error(
                        "The main-window user being edited no longer exists.",
                    );
                };
                session.is_dirty = buffer != user.username;
                session.username_buffer = buffer;
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::CommitMainWindowUserEdit => {
                let Some(session) = self.main_window_user_edit_session.clone() else {
                    return self.record_action_error(
                        "No main-window user edit session is currently active.",
                    );
                };
                let Some((previous_username, username)) = self.rename_main_window_user_at_index(
                    session.editing_index,
                    session.username_buffer,
                    "Renamed main-window user names must be non-empty.",
                    "The main-window user being edited no longer exists.",
                ) else {
                    return false;
                };
                self.main_window_user_edit_session = None;
                self.push_transient_notification(
                    GuiTransientNotificationLevel::Success,
                    format!("User renamed: {previous_username} -> {username}."),
                );
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::CancelMainWindowUserEdit => {
                if self.main_window_user_edit_session.is_none() {
                    return self.record_action_error(
                        "No main-window user edit session is currently active.",
                    );
                }
                self.main_window_user_edit_session = None;
                self.clear_action_error_and_refresh();
                true
            }
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
            GuiShellAction::UpdateNewPlaylistEntryDraft(buffer) => {
                self.new_playlist_entry_draft = buffer;
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::CommitNewPlaylistEntry => {
                if !self.announce_shared_playlist_entry_added(self.new_playlist_entry_draft.clone())
                {
                    return false;
                }
                self.new_playlist_entry_draft.clear();
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
            GuiShellAction::CompletePlaybackPauseToggle => self.complete_playback_pause_toggle(),
            GuiShellAction::CancelPlaybackPauseToggle => self.cancel_playback_pause_toggle(),
            GuiShellAction::AnnouncePlaybackPaused => self.announce_playback_pause_state(true),
            GuiShellAction::AnnouncePlaybackResumed => self.announce_playback_pause_state(false),
            GuiShellAction::RequestSeekPrompt
            | GuiShellAction::RequestOffsetPrompt
            | GuiShellAction::RequestPlaybackUndoSeek => {
                self.clear_action_error_and_refresh();
                true
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
            GuiShellAction::SelectMainWindowPlaylist(index) => {
                if index >= self.main_window.playlist.len() {
                    return self
                        .record_action_error("No playlist row exists at the requested index.");
                }
                self.selection.selected_main_window_playlist = Some(index);
                self.apply_selection_to_surfaces();
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::MoveSelectedMainWindowPlaylistUp => {
                self.move_selected_main_window_playlist(-1)
            }
            GuiShellAction::MoveSelectedMainWindowPlaylistDown => {
                self.move_selected_main_window_playlist(1)
            }
            GuiShellAction::RemoveSelectedMainWindowPlaylist => {
                self.remove_selected_main_window_playlist()
            }
            GuiShellAction::SelectMenuAction {
                section_index,
                action_index,
            } => {
                let Some(section) = self.menus.sections.get(section_index) else {
                    return self
                        .record_action_error("No menu section exists at the requested index.");
                };
                if action_index >= section.actions.len() {
                    return self
                        .record_action_error("No menu action exists at the requested index.");
                }
                self.selection.selected_menu_action = Some((section_index, action_index));
                self.apply_selection_to_surfaces();
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::SelectMediaSearchDirectory(index) => {
                if index >= self.media_search.directories.len() {
                    return self.record_action_error(
                        "No media-search directory exists at the requested index.",
                    );
                }
                self.selection.selected_media_search_directory = Some(index);
                self.apply_selection_to_surfaces();
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::MoveSelectedMediaSearchDirectoryUp => {
                self.move_selected_media_search_directory(-1)
            }
            GuiShellAction::MoveSelectedMediaSearchDirectoryDown => {
                self.move_selected_media_search_directory(1)
            }
            GuiShellAction::RemoveSelectedMediaSearchDirectory => {
                self.remove_selected_media_search_directory()
            }
            GuiShellAction::EditConfigurationText {
                section,
                label,
                value,
            } => {
                let previous_settings = self.configuration.to_stored_settings();
                let applied = self.configuration.apply_text_value(section, label, &value);
                if applied {
                    self.sync_derived_surfaces_from_configuration_settings(&previous_settings);
                    self.clear_action_error_and_refresh();
                } else {
                    return self
                        .record_action_error("Configuration text control could not be updated.");
                }
                applied
            }
            GuiShellAction::EditConfigurationBool {
                section,
                label,
                value,
            } => {
                let previous_settings = self.configuration.to_stored_settings();
                let applied = self.configuration.apply_bool_value(section, label, value);
                if applied {
                    self.sync_derived_surfaces_from_configuration_settings(&previous_settings);
                    self.clear_action_error_and_refresh();
                } else {
                    return self.record_action_error(
                        "Configuration checkbox control could not be updated.",
                    );
                }
                applied
            }
            GuiShellAction::AnnouncePublicServerSelectionChanged(index) => {
                self.announce_public_server_selection_changed(index)
            }
            GuiShellAction::BeginSavedServerConnect => self.begin_saved_server_connect(),
            GuiShellAction::CompleteSavedServerConnect => self.complete_saved_server_connect(),
            GuiShellAction::CancelSavedServerConnect => self.cancel_saved_server_connect(),
            GuiShellAction::BeginSessionDisconnect => self.begin_session_disconnect(),
            GuiShellAction::CompleteSessionDisconnect => self.complete_session_disconnect(),
            GuiShellAction::CancelSessionDisconnect => self.cancel_session_disconnect(),
            GuiShellAction::BeginSelectedPublicServerConnect => {
                self.begin_selected_public_server_connect()
            }
            GuiShellAction::CompleteSelectedPublicServerConnect => {
                self.complete_selected_public_server_connect()
            }
            GuiShellAction::BeginPublicServerRefresh => self.begin_public_server_refresh(),
            GuiShellAction::CompletePublicServerRefresh(servers) => {
                self.complete_public_server_refresh(servers)
            }
            GuiShellAction::AnnounceCustomPublicServerAdded { label, address } => {
                self.announce_custom_public_server_added(label, address)
            }
            GuiShellAction::SelectPublicServer(index) => {
                if !self.apply_public_server_selection(index) {
                    return false;
                }
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::AddMediaSearchDirectory(path) => {
                if !self.add_media_search_directory_path(path) {
                    return false;
                }
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::AnnounceMediaSearchDirectorySelected(index) => {
                self.announce_media_search_directory_selected(index)
            }
            GuiShellAction::AnnounceMediaSearchDirectoryBrowsed(path) => {
                self.announce_media_search_directory_browsed(path)
            }
            GuiShellAction::BeginMissingMediaSearch => self.begin_missing_media_search(),
            GuiShellAction::CompleteMissingMediaSearch(found_path) => {
                self.complete_missing_media_search(found_path)
            }
            GuiShellAction::ToggleMainWindowPlaybackButtons => {
                self.toggle_main_window_playback_buttons()
            }
            GuiShellAction::ToggleMainWindowAutoplayControls => {
                self.toggle_main_window_autoplay_controls()
            }
            GuiShellAction::ToggleMainWindowHideEmptyRooms => {
                self.toggle_main_window_hide_empty_rooms()
            }
            GuiShellAction::RequestMainWindowUserMediaOpen(_) => {
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::RequestMainWindowUserContainingFolderOpen(_) => {
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::RequestMainWindowUserReady { .. }
            | GuiShellAction::RequestControllerAuth { .. } => {
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::AddTrustedDomain(domain) => self.add_trusted_domain(domain),
            GuiShellAction::JoinMainWindowRoom(room) => self.join_main_window_room(room),
            GuiShellAction::LeaveMainWindowRoom => self.leave_main_window_room(),
            GuiShellAction::SetMainWindowRoom(room) => {
                let normalized = normalized_editable_text(&room);
                let Some(room) = normalized else {
                    return self.record_action_error("Room name cannot be empty.");
                };
                self.set_main_window_room_state(Some(room));
                self.clear_action_error_and_refresh();
                true
            }
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot) => {
                self.apply_main_window_runtime_snapshot(snapshot)
            }
            GuiShellAction::ApplyGuiRuntimeSnapshot(snapshot) => {
                self.apply_gui_runtime_snapshot(snapshot)
            }
            GuiShellAction::PushChatMessage { sender, message } => {
                if sender.trim().is_empty() || message.trim().is_empty() {
                    return self
                        .record_action_error("Chat sender and message must both be non-empty.");
                }
                self.append_chat_row(sender, message);
                self.clear_action_error_and_refresh();
                true
            }
        }
    }
}
