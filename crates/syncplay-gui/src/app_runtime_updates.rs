use syncplay_client_app::app_boundary::state::StoredClientSettingsMvp;

use super::shell_state::{
    GuiCommandAvailabilityRuntimeOverride, GuiCommandRuntimeSnapshot,
    GuiConfigurationDraftRuntimeSnapshot, GuiConfigurationRuntimeSnapshot, GuiDialogControlKind,
    GuiDraftRuntimeSnapshot, GuiErrorRuntimeSnapshot, GuiFeedbackRuntimeSnapshot,
    GuiFocusedConfigurationControlState, GuiInteractionRuntimeSnapshot,
    GuiMainWindowUserEditSessionState, GuiMediaIndexRuntimeSnapshot, GuiPendingOperationKind,
    GuiPendingOperationState, GuiPlaylistTextEditSessionState, GuiPublicServerEditSessionState,
    GuiRoomHistoryEditSessionState, GuiSavedConfigurationRuntimeSnapshot, GuiShellView,
    GuiTextEditSessionState, GuiTransientNotification, GuiTransientNotificationLevel,
    GuiUrlEditSessionState, GuiValidationIssue, MenuDialogRuntimeSnapshot, MenuDialogShellState,
    SyncplayGuiShellAppState,
};
use super::support::normalized_editable_text;

impl SyncplayGuiShellAppState {
    pub(super) fn apply_menu_dialog_runtime_snapshot(
        &mut self,
        snapshot: MenuDialogRuntimeSnapshot,
    ) -> bool {
        let previous_tls_prompt_expected = self.menus.tls_prompt_expected;
        let previous_update_notice_expected = self.menus.update_notice_expected;
        let settings = self.configuration.to_stored_settings();
        let baseline_menus = MenuDialogShellState::from_stored_settings(&settings);
        for action_override in snapshot.action_overrides {
            self.remember_runtime_menu_action_override(&baseline_menus, &action_override);
            let mut applied = false;
            for section in &mut self.menus.sections {
                if section.title != action_override.section_title {
                    continue;
                }
                if let Some(action) = section
                    .actions
                    .iter_mut()
                    .find(|action| action.label == action_override.action_label)
                {
                    action.enabled = action_override.enabled;
                    applied = true;
                    break;
                }
            }
            if !applied {
                return self.record_action_error(format!(
                    "No menu action exists for '{} / {}' in the runtime snapshot.",
                    action_override.section_title, action_override.action_label
                ));
            }
        }

        self.menus.tls_prompt_expected = snapshot.tls_prompt_expected;
        self.menus.update_notice_expected = snapshot.update_notice_expected;
        self.menus.about_dialog_available = snapshot.about_dialog_available;
        self.sync_dialog_menu_actions_from_runtime_state();
        self.normalize_selected_menu_action_after_runtime_update();
        self.apply_selection_to_surfaces();
        self.open_newly_expected_modal_if_needed(
            previous_tls_prompt_expected,
            previous_update_notice_expected,
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn apply_gui_feedback_runtime_snapshot(
        &mut self,
        snapshot: GuiFeedbackRuntimeSnapshot,
    ) -> bool {
        let mut normalized_validation_issues = Vec::with_capacity(snapshot.validation_issues.len());
        for issue in snapshot.validation_issues {
            let Some(scope) = normalized_editable_text(&issue.scope) else {
                return self.record_action_error(
                    "GUI feedback runtime snapshots cannot contain empty validation scopes.",
                );
            };
            let Some(label) = normalized_editable_text(&issue.label) else {
                return self.record_action_error(
                    "GUI feedback runtime snapshots cannot contain empty validation labels.",
                );
            };
            let Some(message) = normalized_editable_text(&issue.message) else {
                return self.record_action_error(
                    "GUI feedback runtime snapshots cannot contain empty validation messages.",
                );
            };
            normalized_validation_issues.push(GuiValidationIssue {
                scope,
                label,
                message,
            });
        }

        let mut normalized_notifications = Vec::with_capacity(snapshot.notifications.len());
        for notification in snapshot.notifications {
            let Some(message) = normalized_editable_text(&notification.message) else {
                return self.record_action_error(
                    "GUI feedback runtime snapshots cannot contain empty notification messages.",
                );
            };
            normalized_notifications.push(GuiTransientNotification {
                level: notification.level,
                message,
            });
        }

        self.runtime_validation_issues = normalized_validation_issues;
        self.notifications = normalized_notifications;
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn apply_gui_error_runtime_snapshot(
        &mut self,
        snapshot: GuiErrorRuntimeSnapshot,
    ) -> bool {
        let last_action_error = match snapshot.last_action_error {
            Some(message) => {
                let Some(message) = normalized_editable_text(&message) else {
                    return self.record_action_error(
                        "GUI error runtime snapshots cannot contain an empty action error message.",
                    );
                };
                Some(message)
            }
            None => None,
        };

        self.validation.last_action_error = last_action_error;
        self.refresh_validation();
        true
    }

    pub(super) fn apply_gui_command_runtime_snapshot(
        &mut self,
        snapshot: GuiCommandRuntimeSnapshot,
    ) -> bool {
        if snapshot.pending_operation.is_some() && snapshot.command_availability.any_enabled() {
            return self.record_action_error(
                "GUI command runtime snapshots cannot leave command actions enabled while a pending operation is active.",
            );
        }

        let can_toggle_pause = snapshot.command_availability.can_toggle_pause;
        let command_availability = snapshot.command_availability;
        self.pending_operation = snapshot
            .pending_operation
            .map(|kind| GuiPendingOperationState { kind });
        if snapshot.pending_operation != Some(GuiPendingOperationKind::ConnectSavedServer) {
            self.pending_saved_server_connect_saves_configuration = false;
        }
        let baseline_command_availability = self.command_availability_without_runtime_override();
        self.runtime_command_availability_override =
            GuiCommandAvailabilityRuntimeOverride::from_baseline_and_snapshot(
                &baseline_command_availability,
                &command_availability,
            );
        self.sync_playback_menu_actions_from_runtime_state(can_toggle_pause);
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn apply_gui_media_index_runtime_snapshot(
        &mut self,
        snapshot: GuiMediaIndexRuntimeSnapshot,
    ) -> bool {
        let message = if snapshot.active {
            let Some(message) = snapshot
                .message
                .as_deref()
                .and_then(normalized_editable_text)
            else {
                return self.record_action_error(
                    "GUI media-index runtime snapshots must include a non-empty message while indexing is active.",
                );
            };
            Some(message)
        } else {
            None
        };

        self.media_index_status.active = snapshot.active;
        self.media_index_status.message = message;
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn apply_gui_interaction_runtime_snapshot(
        &mut self,
        snapshot: GuiInteractionRuntimeSnapshot,
    ) -> bool {
        if snapshot
            .selection
            .selected_main_window_user
            .is_some_and(|index| index >= self.main_window.users.len())
        {
            return self.record_action_error(
                "GUI interaction runtime snapshots cannot select a missing main-window user.",
            );
        }
        if snapshot
            .selection
            .selected_main_window_playlist
            .is_some_and(|index| index >= self.main_window.playlist.len())
        {
            return self.record_action_error(
                "GUI interaction runtime snapshots cannot select a missing playlist row.",
            );
        }
        if snapshot
            .selection
            .selected_menu_action
            .is_some_and(|(section_index, action_index)| {
                self.menus
                    .sections
                    .get(section_index)
                    .is_none_or(|section| action_index >= section.actions.len())
            })
        {
            return self.record_action_error(
                "GUI interaction runtime snapshots cannot select a missing menu action.",
            );
        }
        if snapshot
            .selection
            .selected_media_search_directory
            .is_some_and(|index| index >= self.media_search.directories.len())
        {
            return self.record_action_error(
                "GUI interaction runtime snapshots cannot select a missing media-search directory.",
            );
        }
        if snapshot
            .selected_public_server_index
            .is_some_and(|index| index >= self.public_servers.servers.len())
        {
            return self.record_action_error(
                "GUI interaction runtime snapshots cannot select a missing public server row.",
            );
        }

        let focused_configuration_control = match snapshot.focused_configuration_control {
            Some(focused) => {
                let Some(section) = normalized_editable_text(&focused.section) else {
                    return self.record_action_error(
                        "GUI interaction runtime snapshots cannot contain an empty focused control section.",
                    );
                };
                let Some(label) = normalized_editable_text(&focused.label) else {
                    return self.record_action_error(
                        "GUI interaction runtime snapshots cannot contain an empty focused control label.",
                    );
                };
                let Some((section_title, control_label, kind)) =
                    self.configuration.control_identity(&section, &label)
                else {
                    return self.record_action_error(
                        "GUI interaction runtime snapshots cannot focus an unknown configuration control.",
                    );
                };
                if !kind.is_editable() {
                    return self.record_action_error(
                        "GUI interaction runtime snapshots cannot focus a non-editable configuration control.",
                    );
                }
                Some(GuiFocusedConfigurationControlState {
                    section: section_title,
                    label: control_label,
                    kind,
                    activation_count: focused.activation_count,
                })
            }
            None => None,
        };

        let text_edit_session = match snapshot.text_edit_session {
            Some(session) => {
                let Some(section) = normalized_editable_text(&session.section) else {
                    return self.record_action_error(
                        "GUI interaction runtime snapshots cannot contain an empty text-edit section.",
                    );
                };
                let Some(label) = normalized_editable_text(&session.label) else {
                    return self.record_action_error(
                        "GUI interaction runtime snapshots cannot contain an empty text-edit label.",
                    );
                };
                let Some((section_title, control_label, kind)) =
                    self.configuration.control_identity(&section, &label)
                else {
                    return self.record_action_error(
                        "GUI interaction runtime snapshots cannot target an unknown text-edit control.",
                    );
                };
                if !kind.is_editable() || kind == GuiDialogControlKind::Checkbox {
                    return self.record_action_error(
                        "GUI interaction runtime snapshots cannot target a non-text-editable configuration control.",
                    );
                }
                Some(GuiTextEditSessionState {
                    section: section_title,
                    label: control_label,
                    buffer: session.buffer,
                    is_dirty: session.is_dirty,
                })
            }
            None => None,
        };

        let playlist_text_edit_session = match snapshot.playlist_text_edit_session {
            Some(session) => {
                if !self.shared_playlist_events_enabled() {
                    return self.record_action_error(
                        "GUI interaction runtime snapshots cannot edit the shared playlist when shared playlists are disabled.",
                    );
                }
                Some(GuiPlaylistTextEditSessionState {
                    buffer: session.buffer,
                    is_dirty: session.is_dirty,
                })
            }
            None => None,
        };

        let playlist_url_edit_session = match snapshot.playlist_url_edit_session {
            Some(session) => {
                if !self.shared_playlist_events_enabled() {
                    return self.record_action_error(
                        "GUI interaction runtime snapshots cannot edit shared playlist URLs when shared playlists are disabled.",
                    );
                }
                Some(GuiUrlEditSessionState {
                    buffer: session.buffer,
                    is_dirty: session.is_dirty,
                })
            }
            None => None,
        };

        let media_url_edit_session =
            snapshot
                .media_url_edit_session
                .map(|session| GuiUrlEditSessionState {
                    buffer: session.buffer,
                    is_dirty: session.is_dirty,
                });

        let public_server_edit_session = match snapshot.public_server_edit_session {
            Some(session) => {
                if session
                    .editing_index
                    .is_some_and(|index| index >= self.public_servers.servers.len())
                {
                    return self.record_action_error(
                        "GUI interaction runtime snapshots cannot edit a missing public server row.",
                    );
                }
                let (original_label, original_address) = session
                    .editing_index
                    .and_then(|index| self.public_servers.servers.get(index))
                    .map(|row| (Some(row.label.clone()), Some(row.address.clone())))
                    .unwrap_or((None, None));
                Some(GuiPublicServerEditSessionState {
                    editing_index: session.editing_index,
                    label_buffer: session.label_buffer,
                    address_buffer: session.address_buffer,
                    is_dirty: session.is_dirty,
                    original_label,
                    original_address,
                })
            }
            None => None,
        };

        let main_window_user_edit_session = match snapshot.main_window_user_edit_session {
            Some(session) => {
                if session.editing_index >= self.main_window.users.len() {
                    return self.record_action_error(
                        "GUI interaction runtime snapshots cannot edit a missing main-window user.",
                    );
                }
                Some(GuiMainWindowUserEditSessionState {
                    editing_index: session.editing_index,
                    username_buffer: session.username_buffer,
                    is_dirty: session.is_dirty,
                    original_username: self.main_window.users[session.editing_index]
                        .username
                        .clone(),
                })
            }
            None => None,
        };

        self.selection = snapshot.selection;
        self.set_selected_public_server_index(snapshot.selected_public_server_index);
        self.focused_configuration_control = focused_configuration_control;
        self.public_server_edit_session = public_server_edit_session;
        self.main_window_user_edit_session = main_window_user_edit_session;
        self.text_edit_session = text_edit_session;
        self.playlist_text_edit_session = playlist_text_edit_session;
        self.playlist_url_edit_session = playlist_url_edit_session;
        self.media_url_edit_session = media_url_edit_session;
        self.normalize_selection();
        self.normalize_selected_menu_action_after_runtime_update();
        self.apply_selection_to_surfaces();
        self.normalize_focused_configuration_control();
        self.normalize_public_server_edit_session();
        self.normalize_main_window_user_edit_session();
        self.normalize_text_edit_session();
        self.normalize_playlist_text_edit_session();
        self.normalize_playlist_url_edit_session();
        self.normalize_media_url_edit_session();
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn apply_gui_draft_runtime_snapshot(
        &mut self,
        snapshot: GuiDraftRuntimeSnapshot,
    ) -> bool {
        let outgoing_chat_message = match snapshot.outgoing_chat_message {
            Some(message) => {
                let Some(message) = normalized_editable_text(&message) else {
                    return self.record_action_error(
                        "GUI draft runtime snapshots cannot contain an empty outgoing chat message.",
                    );
                };
                if self
                    .pending_operation
                    .as_ref()
                    .is_some_and(|pending| pending.kind != GuiPendingOperationKind::SendChatMessage)
                {
                    return self.record_action_error(
                        "GUI draft runtime snapshots cannot stage an outgoing chat message while a different pending operation is active.",
                    );
                }
                Some(message)
            }
            None => {
                if self
                    .pending_operation
                    .as_ref()
                    .is_some_and(|pending| pending.kind == GuiPendingOperationKind::SendChatMessage)
                {
                    return self.record_action_error(
                        "GUI draft runtime snapshots cannot clear the outgoing chat message while chat send is still pending.",
                    );
                }
                None
            }
        };

        self.outgoing_chat_message = outgoing_chat_message;
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn apply_gui_configuration_draft_runtime_snapshot(
        &mut self,
        snapshot: GuiConfigurationDraftRuntimeSnapshot,
    ) -> bool {
        if self.pending_operation.as_ref().is_some_and(|pending| {
            matches!(
                pending.kind,
                GuiPendingOperationKind::SaveConfiguration
                    | GuiPendingOperationKind::ResetConfiguration
                    | GuiPendingOperationKind::ReloadConfiguration
            )
        }) {
            return self.record_action_error(
                "GUI configuration draft runtime snapshots cannot apply while a configuration command is already in progress.",
            );
        }

        self.resync_from_settings(snapshot.settings);
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn apply_gui_saved_configuration_runtime_snapshot(
        &mut self,
        snapshot: GuiSavedConfigurationRuntimeSnapshot,
    ) -> bool {
        if self.pending_operation.as_ref().is_some_and(|pending| {
            matches!(
                pending.kind,
                GuiPendingOperationKind::SaveConfiguration
                    | GuiPendingOperationKind::ResetConfiguration
                    | GuiPendingOperationKind::ReloadConfiguration
            )
        }) {
            return self.record_action_error(
                "GUI saved-configuration runtime snapshots cannot apply while a configuration command is already in progress.",
            );
        }

        self.saved_configuration = snapshot.settings;
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn apply_gui_configuration_runtime_snapshot(
        &mut self,
        snapshot: GuiConfigurationRuntimeSnapshot,
    ) -> bool {
        if self.pending_operation.as_ref().is_some_and(|pending| {
            matches!(
                pending.kind,
                GuiPendingOperationKind::SaveConfiguration
                    | GuiPendingOperationKind::ResetConfiguration
                    | GuiPendingOperationKind::ReloadConfiguration
            )
        }) {
            return self.record_action_error(
                "GUI configuration runtime snapshots cannot apply while a configuration command is already in progress.",
            );
        }

        self.resync_from_settings(snapshot.draft_settings);
        self.saved_configuration = snapshot.saved_settings;
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn begin_room_history_edit(&mut self) -> bool {
        if self.room_history_edit_session.is_some() {
            return self.record_action_error("A room-history edit session is already active.");
        }
        self.room_history_edit_session = Some(GuiRoomHistoryEditSessionState {
            buffer: self.configuration.room_history_multiline_text(),
            is_dirty: false,
        });
        self.active_view = GuiShellView::Configuration;
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn update_room_history_edit(&mut self, buffer: String) -> bool {
        let Some(session) = self.room_history_edit_session.as_mut() else {
            return self.record_action_error("No room-history edit session is currently active.");
        };
        let current_value = self.configuration.room_history_multiline_text();
        session.is_dirty = buffer != current_value;
        session.buffer = buffer;
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn commit_room_history_edit(&mut self) -> bool {
        let Some(session) = self.room_history_edit_session.clone() else {
            return self.record_action_error("No room-history edit session is currently active.");
        };
        self.configuration
            .apply_room_history_multiline_text(&session.buffer);
        self.room_history_edit_session = None;
        let room_history_count = self
            .configuration
            .settings
            .room_list
            .as_ref()
            .map_or(0, Vec::len);
        self.push_transient_notification(
            GuiTransientNotificationLevel::Success,
            format!("Room history updated: {room_history_count} entries."),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn cancel_room_history_edit(&mut self) -> bool {
        if self.room_history_edit_session.is_none() {
            return self.record_action_error("No room-history edit session is currently active.");
        }
        self.room_history_edit_session = None;
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn begin_configuration_save(&mut self) -> bool {
        if self.pending_operation.is_some() {
            return self.record_action_error("Another GUI operation is already in progress.");
        }
        if !self.validation.issues.is_empty() {
            return self.record_action_error(
                "Configuration cannot be saved while validation issues remain.",
            );
        }

        self.pending_operation = Some(GuiPendingOperationState {
            kind: GuiPendingOperationKind::SaveConfiguration,
        });
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            "Configuration save started.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn complete_configuration_save(
        &mut self,
        settings: StoredClientSettingsMvp,
    ) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self.record_action_error("No configuration save is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::SaveConfiguration {
            return self
                .record_action_error("The active GUI operation is not a configuration save.");
        }

        self.saved_configuration = settings;
        self.pending_operation = None;
        self.pending_saved_server_connect_saves_configuration = false;
        self.push_system_chat_message("Configuration saved.".to_owned());
        self.push_transient_notification(
            GuiTransientNotificationLevel::Success,
            "Configuration saved.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn begin_configuration_reset(&mut self) -> bool {
        if self.pending_operation.is_some() {
            return self.record_action_error("Another GUI operation is already in progress.");
        }
        if !self.has_unsaved_configuration_changes() {
            return self.record_action_error(
                "Configuration reset is unavailable with no unsaved changes.",
            );
        }

        self.pending_operation = Some(GuiPendingOperationState {
            kind: GuiPendingOperationKind::ResetConfiguration,
        });
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            "Configuration reset started.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn complete_configuration_reset(
        &mut self,
        settings: StoredClientSettingsMvp,
    ) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self.record_action_error("No configuration reset is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::ResetConfiguration {
            return self
                .record_action_error("The active GUI operation is not a configuration reset.");
        }

        self.pending_operation = None;
        self.pending_saved_server_connect_saves_configuration = false;
        self.resync_from_settings(settings.clone());
        self.saved_configuration = settings;
        self.push_system_chat_message("Configuration reset to the last saved state.".to_owned());
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            "Configuration reset completed.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn cancel_configuration_reset(&mut self) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self.record_action_error("No configuration reset is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::ResetConfiguration {
            return self
                .record_action_error("The active GUI operation is not a configuration reset.");
        }

        self.pending_operation = None;
        self.push_transient_notification(
            GuiTransientNotificationLevel::Warning,
            "Configuration reset canceled.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn begin_configuration_reload(&mut self) -> bool {
        if self.pending_operation.is_some() {
            return self.record_action_error("Another GUI operation is already in progress.");
        }

        self.pending_operation = Some(GuiPendingOperationState {
            kind: GuiPendingOperationKind::ReloadConfiguration,
        });
        self.push_transient_notification(
            GuiTransientNotificationLevel::Info,
            "Configuration reload started.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn complete_configuration_reload(
        &mut self,
        settings: StoredClientSettingsMvp,
    ) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self.record_action_error("No configuration reload is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::ReloadConfiguration {
            return self
                .record_action_error("The active GUI operation is not a configuration reload.");
        }

        self.pending_operation = None;
        self.pending_saved_server_connect_saves_configuration = false;
        self.resync_from_settings(settings.clone());
        self.saved_configuration = settings;
        self.push_system_chat_message("Configuration snapshot loaded.".to_owned());
        self.push_transient_notification(
            GuiTransientNotificationLevel::Success,
            "Configuration reload completed.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn begin_clear_gui_data(&mut self) -> bool {
        if self.pending_operation.is_some() {
            return self.record_action_error("Another GUI operation is already in progress.");
        }

        self.pending_operation = Some(GuiPendingOperationState {
            kind: GuiPendingOperationKind::ClearGuiData,
        });
        self.push_transient_notification(
            GuiTransientNotificationLevel::Warning,
            "Clear GUI data started.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn complete_clear_gui_data(&mut self) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self
                .record_action_error("No clear-GUI-data operation is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::ClearGuiData {
            return self
                .record_action_error("The active GUI operation is not a clear-GUI-data request.");
        }

        self.reset_to_first_run_state(StoredClientSettingsMvp::default());
        self.pending_saved_server_connect_saves_configuration = false;
        self.push_transient_notification(
            GuiTransientNotificationLevel::Success,
            "GUI data cleared. First-run configuration restored.".to_owned(),
        );
        self.push_system_chat_message(
            "GUI data cleared. First-run configuration restored.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn cancel_clear_gui_data(&mut self) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self
                .record_action_error("No clear-GUI-data operation is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::ClearGuiData {
            return self
                .record_action_error("The active GUI operation is not a clear-GUI-data request.");
        }

        self.pending_operation = None;
        self.pending_saved_server_connect_saves_configuration = false;
        self.push_transient_notification(
            GuiTransientNotificationLevel::Warning,
            "Clear GUI data canceled.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn cancel_configuration_reload(&mut self) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self.record_action_error("No configuration reload is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::ReloadConfiguration {
            return self
                .record_action_error("The active GUI operation is not a configuration reload.");
        }

        self.pending_operation = None;
        self.pending_saved_server_connect_saves_configuration = false;
        self.push_transient_notification(
            GuiTransientNotificationLevel::Warning,
            "Configuration reload canceled.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }

    pub(super) fn cancel_configuration_save(&mut self) -> bool {
        let Some(pending) = self.pending_operation.as_ref() else {
            return self.record_action_error("No configuration save is currently in progress.");
        };
        if pending.kind != GuiPendingOperationKind::SaveConfiguration {
            return self
                .record_action_error("The active GUI operation is not a configuration save.");
        }

        self.pending_operation = None;
        self.pending_saved_server_connect_saves_configuration = false;
        self.push_transient_notification(
            GuiTransientNotificationLevel::Warning,
            "Configuration save canceled.".to_owned(),
        );
        self.clear_action_error_and_refresh();
        true
    }
}
