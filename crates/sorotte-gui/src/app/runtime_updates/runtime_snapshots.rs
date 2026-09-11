use super::super::GuiShellModal;
use super::super::shell_state::{
    GuiCommandAvailabilityRuntimeOverride, GuiCommandRuntimeSnapshot,
    GuiConfigStorageRuntimeSnapshot, GuiConfigurationDraftRuntimeSnapshot,
    GuiConfigurationRuntimeSnapshot, GuiDialogControlKind, GuiDraftRuntimeSnapshot,
    GuiErrorRuntimeSnapshot, GuiFeedbackRuntimeSnapshot, GuiFocusedConfigurationControlState,
    GuiInteractionRuntimeSnapshot, GuiMainWindowUserEditSessionState, GuiMediaIndexRuntimeSnapshot,
    GuiMediaMatchRemediationRuntimeSnapshot, GuiMediaMatchRuntimeSnapshot, GuiPendingOperationKind,
    GuiPersistedSettingsPatch, GuiPlayerSetupIssueKind, GuiPlayerSetupRuntimeSnapshot,
    GuiPlaylistTextEditSessionState, GuiPlexRuntimeSnapshot, GuiPublicServerEditSessionState,
    GuiSavedConfigurationRuntimeSnapshot, GuiSavedServerConnectIntent,
    GuiSeekPreparationRuntimeSnapshot, GuiStreamHelperHealth,
    GuiStreamHelperRemediationRuntimeSnapshot, GuiStreamHelperRuntimeSnapshot,
    GuiTextEditSessionState, GuiTransientNotification, GuiUrlEditSessionState, GuiValidationIssue,
    MenuDialogRuntimeSnapshot, SorotteGuiShellAppState,
};
use super::super::support::normalized_editable_text;

impl SorotteGuiShellAppState {
    pub(in crate::app) fn apply_menu_dialog_runtime_snapshot(
        &mut self,
        snapshot: MenuDialogRuntimeSnapshot,
    ) -> bool {
        let previous_tls_prompt_expected = self.menus.tls_prompt_expected;
        let previous_update_notice_expected = self.menus.update_notice_expected;
        if let Err(message) = super::super::feature_snapshots::apply_menu_dialog_snapshot(
            &mut self.menus,
            &mut self.runtime_menu_action_overrides,
            &self.configuration.to_stored_settings(),
            snapshot,
        ) {
            return self.record_action_error(message);
        }
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

    pub(in crate::app) fn apply_gui_feedback_runtime_snapshot(
        &mut self,
        snapshot: GuiFeedbackRuntimeSnapshot,
    ) -> bool {
        let mut normalized_validation_issues = Vec::with_capacity(snapshot.validation_issues.len());
        for issue in snapshot.validation_issues {
            let setting_id = issue.setting_id;
            let (scope, label) = if let Some(id) = setting_id {
                (id.section().to_owned(), id.label().to_owned())
            } else {
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
                (scope, label)
            };
            let Some(message) = normalized_editable_text(&issue.message) else {
                return self.record_action_error(
                    "GUI feedback runtime snapshots cannot contain empty validation messages.",
                );
            };
            normalized_validation_issues.push(GuiValidationIssue {
                setting_id,
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

    pub(in crate::app) fn apply_gui_error_runtime_snapshot(
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

    pub(in crate::app) fn apply_gui_command_runtime_snapshot(
        &mut self,
        snapshot: GuiCommandRuntimeSnapshot,
    ) -> bool {
        if snapshot.pending_operation.is_some() && snapshot.command_availability.any_enabled() {
            return self.record_action_error(
            "GUI command runtime snapshots cannot leave command actions enabled while a pending operation is active.",
        );
        }

        let current_pending_operation = self.pending_operation.as_ref().map(|pending| pending.kind);
        if snapshot.pending_operation != current_pending_operation {
            // Command projections are asynchronous and can arrive after the shell has begun or
            // completed an operation. Pending lifecycle transitions belong to the explicit
            // Begin/Complete/Cancel actions; a stale availability snapshot must never resurrect
            // or retire an unrelated operation.
            return false;
        }

        let can_toggle_pause = snapshot.command_availability.can_toggle_pause;
        let command_availability = snapshot.command_availability;
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

    pub(in crate::app) fn apply_gui_config_storage_runtime_snapshot(
        &mut self,
        snapshot: GuiConfigStorageRuntimeSnapshot,
    ) -> bool {
        self.config_storage = snapshot;
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn apply_gui_media_index_runtime_snapshot(
        &mut self,
        snapshot: GuiMediaIndexRuntimeSnapshot,
    ) -> bool {
        if let Err(message) =
            super::super::feature_snapshots::apply_gui_media_index_runtime_snapshot(
                &mut self.media_index_status,
                snapshot,
            )
        {
            return self.record_action_error(message);
        }

        self.clear_action_error_and_refresh();
        true
    }
    pub(in crate::app) fn apply_gui_player_setup_runtime_snapshot(
        &mut self,
        snapshot: GuiPlayerSetupRuntimeSnapshot,
    ) -> bool {
        let previous_issue_kind = self.player_setup_issue.as_ref().map(|issue| issue.kind);
        if let Err(message) =
            super::super::feature_snapshots::apply_gui_player_setup_runtime_snapshot(
                &mut self.player_setup_issue,
                snapshot,
            )
        {
            return self.record_action_error(message);
        }
        let next_issue_kind = self.player_setup_issue.as_ref().map(|next| next.kind);
        if self.player_setup_issue.is_none() && self.open_modal == Some(GuiShellModal::PlayerSetup)
        {
            self.open_modal = None;
        } else if next_issue_kind.is_some()
            && next_issue_kind != Some(GuiPlayerSetupIssueKind::PlayerSettingsDegraded)
            && next_issue_kind != previous_issue_kind
            && self.open_modal.is_none()
        {
            self.open_modal = Some(GuiShellModal::PlayerSetup);
        }

        self.clear_action_error_and_refresh();
        true
    }
    pub(in crate::app) fn apply_gui_seek_preparation_runtime_snapshot(
        &mut self,
        snapshot: GuiSeekPreparationRuntimeSnapshot,
    ) -> bool {
        if let Err(message) =
            super::super::feature_snapshots::apply_gui_seek_preparation_runtime_snapshot(
                &mut self.seek_preparation,
                &mut self.seek_preparation_degraded_reason,
                snapshot,
            )
        {
            return self.record_action_error(message);
        }

        self.clear_action_error_and_refresh();
        true
    }
    pub(in crate::app) fn apply_gui_stream_helper_runtime_snapshot(
        &mut self,
        snapshot: GuiStreamHelperRuntimeSnapshot,
    ) -> bool {
        if let Err(message) =
            super::super::feature_snapshots::apply_gui_stream_helper_runtime_snapshot(
                &mut self.stream_helper,
                snapshot,
            )
        {
            return self.record_action_error(message);
        }
        if self.stream_helper.health == GuiStreamHelperHealth::Healthy
            && self.open_modal == Some(GuiShellModal::StreamSupport)
        {
            self.open_modal = None;
        }

        self.clear_action_error_and_refresh();
        true
    }
    pub(in crate::app) fn apply_gui_stream_helper_remediation_runtime_snapshot(
        &mut self,
        snapshot: GuiStreamHelperRemediationRuntimeSnapshot,
    ) -> bool {
        if let Err(message) =
            super::super::feature_snapshots::apply_gui_stream_helper_remediation_runtime_snapshot(
                &mut self.stream_helper_remediation,
                snapshot,
            )
        {
            return self.record_action_error(message);
        }

        self.clear_action_error_and_refresh();
        true
    }
    pub(in crate::app) fn apply_gui_media_match_runtime_snapshot(
        &mut self,
        snapshot: GuiMediaMatchRuntimeSnapshot,
    ) -> bool {
        if let Err(message) =
            super::super::feature_snapshots::apply_gui_media_match_runtime_snapshot(
                &mut self.media_match,
                snapshot,
            )
        {
            return self.record_action_error(message);
        }

        self.refresh_playlist_source_states();

        self.clear_action_error_and_refresh();
        true
    }
    pub(in crate::app) fn apply_gui_media_match_remediation_runtime_snapshot(
        &mut self,
        snapshot: GuiMediaMatchRemediationRuntimeSnapshot,
    ) -> bool {
        if let Err(message) =
            super::super::feature_snapshots::apply_gui_media_match_remediation_runtime_snapshot(
                &mut self.media_match_remediation,
                snapshot,
            )
        {
            return self.record_action_error(message);
        }

        self.clear_action_error_and_refresh();
        true
    }
    pub(in crate::app) fn apply_gui_plex_runtime_snapshot(
        &mut self,
        snapshot: GuiPlexRuntimeSnapshot,
    ) -> bool {
        if let Err(message) = super::super::feature_snapshots::apply_gui_plex_runtime_snapshot(
            &mut self.plex,
            snapshot,
        ) {
            return self.record_action_error(message);
        }

        self.refresh_playlist_source_states();

        self.clear_action_error_and_refresh();
        true
    }
    pub(in crate::app) fn apply_gui_interaction_runtime_snapshot(
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
                let Some(setting_id) = normalized_editable_text(&focused.setting_id) else {
                    return self.record_action_error(
                    "GUI interaction runtime snapshots cannot contain an empty focused setting ID.",
                );
                };
                let Some((id, kind)) = self.configuration.control_identity(&setting_id) else {
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
                    id,
                    kind,
                    activation_count: focused.activation_count,
                })
            }
            None => None,
        };

        let text_edit_session = match snapshot.text_edit_session {
            Some(session) => {
                let Some(setting_id) = normalized_editable_text(&session.setting_id) else {
                    return self.record_action_error(
                    "GUI interaction runtime snapshots cannot contain an empty text-edit setting ID.",
                );
                };
                let Some((id, kind)) = self.configuration.control_identity(&setting_id) else {
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
                    id,
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

        let preserved_local_playlist_selection = self
            .main_window_playlist_selection_is_local
            .then_some(self.selection.selected_main_window_playlist)
            .flatten()
            .filter(|&index| index < self.main_window.playlist.len());

        self.selection = snapshot.selection;
        self.main_window_playlist_selection_is_local = false;
        if let Some(index) = preserved_local_playlist_selection {
            self.selection.selected_main_window_playlist = Some(index);
            self.main_window_playlist_selection_is_local = true;
        }
        self.set_selected_public_server_index(snapshot.selected_public_server_index);
        self.focused_configuration_control = focused_configuration_control;
        self.public_server_edit_session = public_server_edit_session;
        self.main_window_user_edit_session = main_window_user_edit_session;
        self.text_edit_session = text_edit_session;
        self.playlist_text_edit_session = playlist_text_edit_session;
        self.playlist_url_edit_session = playlist_url_edit_session;
        self.plex_playlist_search = snapshot.plex_playlist_search;
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

    pub(in crate::app) fn apply_gui_draft_runtime_snapshot(
        &mut self,
        snapshot: GuiDraftRuntimeSnapshot,
    ) -> bool {
        let outgoing_chat_message = match snapshot.outgoing_chat_message {
            Some(message) => {
                if message.is_empty() {
                    return self.record_action_error(
                        "GUI draft runtime snapshots cannot contain an empty outgoing chat message.",
                    );
                }
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

    pub(in crate::app) fn apply_gui_configuration_draft_runtime_snapshot(
        &mut self,
        snapshot: GuiConfigurationDraftRuntimeSnapshot,
    ) -> bool {
        if self.pending_operation.as_ref().is_some_and(|pending| {
            matches!(
                pending.kind,
                GuiPendingOperationKind::SaveConfiguration
                    | GuiPendingOperationKind::DiscardConfigurationChanges
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

    pub(in crate::app) fn apply_gui_saved_configuration_runtime_snapshot(
        &mut self,
        snapshot: GuiSavedConfigurationRuntimeSnapshot,
    ) -> bool {
        if self.pending_operation.as_ref().is_some_and(|pending| {
            matches!(
                pending.kind,
                GuiPendingOperationKind::SaveConfiguration
                    | GuiPendingOperationKind::DiscardConfigurationChanges
                    | GuiPendingOperationKind::ReloadConfiguration
            )
        }) {
            return self.record_action_error(
            "GUI saved-configuration runtime snapshots cannot apply while a configuration command is already in progress.",
        );
        }

        if self.pending_saved_server_connect_intent
            == Some(GuiSavedServerConnectIntent::SaveAndConnect)
            && self
                .pending_operation
                .as_ref()
                .is_some_and(|pending| pending.kind == GuiPendingOperationKind::ConnectSavedServer)
        {
            self.settle_persisted_configuration(snapshot.settings, true);
        } else {
            self.saved_configuration = snapshot.settings;
        }
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn apply_gui_configuration_runtime_snapshot(
        &mut self,
        snapshot: GuiConfigurationRuntimeSnapshot,
    ) -> bool {
        if self.pending_operation.as_ref().is_some_and(|pending| {
            matches!(
                pending.kind,
                GuiPendingOperationKind::SaveConfiguration
                    | GuiPendingOperationKind::DiscardConfigurationChanges
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

    pub(in crate::app) fn apply_gui_persisted_settings_patch(
        &mut self,
        patch: GuiPersistedSettingsPatch,
    ) -> bool {
        if super::super::configuration_model::apply_persisted_settings_patch(
            &mut self.saved_configuration,
            &mut self.configuration,
            &mut self.plugin_enablement,
            &mut self.media_match,
            &mut self.plex,
            patch,
        ) {
            self.refresh_playlist_source_states();
        }
        self.clear_action_error_and_refresh();
        true
    }
}
