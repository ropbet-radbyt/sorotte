use super::super::GuiShellModal;
use super::super::shell_state::{
    GuiCommandAvailabilityRuntimeOverride, GuiCommandRuntimeSnapshot,
    GuiConfigStorageRuntimeSnapshot, GuiConfigurationDraftRuntimeSnapshot,
    GuiConfigurationRuntimeSnapshot, GuiDialogControlKind, GuiDraftRuntimeSnapshot,
    GuiErrorRuntimeSnapshot, GuiFeedbackRuntimeSnapshot, GuiFocusedConfigurationControlState,
    GuiInteractionRuntimeSnapshot, GuiMainWindowUserEditSessionState, GuiMediaIndexRuntimeSnapshot,
    GuiMediaMatchRemediationRuntimeSnapshot, GuiMediaMatchRuntimeSnapshot, GuiPendingOperationKind,
    GuiPersistedSettingsPatch, GuiPlayerSetupIssue, GuiPlayerSetupRuntimeSnapshot,
    GuiPlaylistTextEditSessionState, GuiPlexRuntimeSnapshot, GuiPlexServerRow,
    GuiPublicServerEditSessionState, GuiSavedConfigurationRuntimeSnapshot,
    GuiSavedServerConnectIntent, GuiSeekPreparationRuntimeSnapshot, GuiStreamHelperHealth,
    GuiStreamHelperRemediationRuntimeSnapshot, GuiStreamHelperRuntimeSnapshot,
    GuiTextEditSessionState, GuiTransientNotification, GuiUrlEditSessionState, GuiValidationIssue,
    MenuDialogRuntimeSnapshot, MenuDialogShellState, SorotteGuiShellAppState,
};
use super::super::support::normalized_editable_text;

impl SorotteGuiShellAppState {
    pub(in crate::app) fn apply_menu_dialog_runtime_snapshot(
        &mut self,
        snapshot: MenuDialogRuntimeSnapshot,
    ) -> bool {
        let previous_tls_prompt_expected = self.menus.tls_prompt_expected;
        let previous_update_notice_expected = self.menus.update_notice_expected;
        let settings = self.configuration.to_stored_settings();
        let baseline_menus = MenuDialogShellState::from_stored_settings(&settings);
        for action_override in snapshot.action_overrides {
            self.remember_runtime_menu_action_override(&baseline_menus, &action_override);
            let Some(action) = self.menus.action_mut(action_override.id) else {
                return self.record_action_error(format!(
                    "No menu action exists for '{}' in the runtime snapshot.",
                    action_override.id.automation_id()
                ));
            };
            action.enabled = action_override.enabled;
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

    pub(in crate::app) fn apply_gui_player_setup_runtime_snapshot(
        &mut self,
        snapshot: GuiPlayerSetupRuntimeSnapshot,
    ) -> bool {
        let previous_issue_kind = self.player_setup_issue.as_ref().map(|issue| issue.kind);
        let issue = match snapshot.issue {
            Some(issue) => {
                let Some(message) = normalized_editable_text(&issue.message) else {
                    return self.record_action_error(
                        "GUI player-setup runtime snapshots cannot contain an empty issue message.",
                    );
                };
                Some(GuiPlayerSetupIssue {
                    kind: issue.kind,
                    message,
                })
            }
            None => None,
        };

        let next_issue_kind = issue.as_ref().map(|next| next.kind);
        self.player_setup_issue = issue;
        if self.player_setup_issue.is_none() && self.open_modal == Some(GuiShellModal::PlayerSetup)
        {
            self.open_modal = None;
        } else if next_issue_kind.is_some()
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
        if snapshot.preparation.is_some() && snapshot.degraded_reason.is_some() {
            return self.record_action_error(
                "GUI seek-preparation snapshots cannot be active and terminally degraded at the same time.",
            );
        }
        let preparation = match snapshot.preparation {
            Some(preparation) => {
                if !preparation.frozen_target_seconds.is_finite()
                    || preparation.frozen_target_seconds < 0.0
                {
                    return self.record_action_error(
                        "GUI seek-preparation snapshots require a finite, non-negative target.",
                    );
                }
                if preparation.cache_refill_percent.is_some_and(|percent| {
                    !percent.is_finite() || !(0.0..=100.0).contains(&percent)
                }) {
                    return self.record_action_error(
                        "GUI seek-preparation snapshots require cache refill between 0 and 100 percent.",
                    );
                }
                if preparation
                    .buffered_ahead_seconds
                    .is_some_and(|seconds| !seconds.is_finite() || seconds < 0.0)
                {
                    return self.record_action_error(
                        "GUI seek-preparation snapshots require finite, non-negative buffered-ahead time.",
                    );
                }
                if preparation
                    .nearest_safe_buffered_position_seconds
                    .is_some_and(|seconds| !seconds.is_finite() || seconds < 0.0)
                {
                    return self.record_action_error(
                        "GUI seek-preparation snapshots require a finite, non-negative nearest buffered position.",
                    );
                }
                if preparation.can_join_nearest_buffered
                    && preparation.nearest_safe_buffered_position_seconds.is_none()
                {
                    return self.record_action_error(
                        "GUI seek-preparation snapshots cannot enable nearest-buffered joining without a safe position.",
                    );
                }
                Some(preparation)
            }
            None => None,
        };

        self.seek_preparation = preparation;
        self.seek_preparation_degraded_reason = snapshot.degraded_reason;
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn apply_gui_stream_helper_runtime_snapshot(
        &mut self,
        snapshot: GuiStreamHelperRuntimeSnapshot,
    ) -> bool {
        let mut normalize_optional_value =
            |value: Option<String>, error_message: &'static str| -> Result<Option<String>, bool> {
                match value {
                    Some(value) => {
                        let Some(value) = normalized_editable_text(&value) else {
                            return Err(self.record_action_error(error_message));
                        };
                        Ok(Some(value.to_owned()))
                    }
                    None => Ok(None),
                }
            };
        let message = match snapshot.message {
            Some(message) => {
                let Some(message) = normalized_editable_text(&message) else {
                    return self.record_action_error(
                    "GUI stream-helper runtime snapshots cannot contain an empty issue message.",
                );
                };
                Some(message)
            }
            None => None,
        };
        if snapshot.health != GuiStreamHelperHealth::Healthy && message.is_none() {
            return self.record_action_error(
            "GUI stream-helper runtime snapshots must include a non-empty message while unhealthy.",
        );
        }
        let install_location = match normalize_optional_value(
            snapshot.install_location,
            "GUI stream-helper runtime snapshots cannot contain an empty install location.",
        ) {
            Ok(value) => value,
            Err(result) => return result,
        };
        let downloader_status = match normalize_optional_value(
            snapshot.downloader_status,
            "GUI stream-helper runtime snapshots cannot contain an empty yt-dlp status.",
        ) {
            Ok(value) => value,
            Err(result) => return result,
        };
        let js_runtime_status = match normalize_optional_value(
            snapshot.js_runtime_status,
            "GUI stream-helper runtime snapshots cannot contain an empty Deno status.",
        ) {
            Ok(value) => value,
            Err(result) => return result,
        };

        self.stream_helper.health = snapshot.health;
        self.stream_helper.message = message;
        self.stream_helper.target = snapshot
            .target
            .and_then(|target| normalized_editable_text(&target));
        self.stream_helper.install_supported = snapshot.install_supported;
        self.stream_helper.integration_supported = snapshot.integration_supported;
        self.stream_helper.retry_available = snapshot.retry_available;
        self.stream_helper.install_location = install_location;
        self.stream_helper.downloader_status = downloader_status;
        self.stream_helper.js_runtime_status = js_runtime_status;
        self.stream_helper.open_install_location_available =
            snapshot.open_install_location_available;
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
        let label = match snapshot.label {
            Some(label) => {
                let Some(label) = normalized_editable_text(&label) else {
                    return self.record_action_error(
                        "GUI stream-helper remediation snapshots cannot contain an empty label.",
                    );
                };
                Some(label)
            }
            None => None,
        };
        let detail = match snapshot.detail {
            Some(detail) => {
                let Some(detail) = normalized_editable_text(&detail) else {
                    return self.record_action_error(
                        "GUI stream-helper remediation snapshots cannot contain an empty detail.",
                    );
                };
                Some(detail)
            }
            None => None,
        };
        if snapshot.active && label.is_none() {
            return self.record_action_error(
            "GUI stream-helper remediation snapshots must include a non-empty label while active.",
        );
        }
        if !snapshot.progress_fraction.is_finite()
            || !(0.0..=1.0).contains(&snapshot.progress_fraction)
        {
            return self.record_action_error(
            "GUI stream-helper remediation snapshots must use a progress value between 0.0 and 1.0.",
        );
        }

        self.stream_helper_remediation.active = snapshot.active;
        self.stream_helper_remediation.label = label.filter(|_| snapshot.active);
        self.stream_helper_remediation.detail = detail.filter(|_| snapshot.active);
        self.stream_helper_remediation.progress_fraction = if snapshot.active {
            snapshot.progress_fraction
        } else {
            0.0
        };
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn apply_gui_media_match_runtime_snapshot(
        &mut self,
        snapshot: GuiMediaMatchRuntimeSnapshot,
    ) -> bool {
        let mut normalize_optional_value =
            |value: Option<String>, error_message: &'static str| -> Result<Option<String>, bool> {
                match value {
                    Some(value) => {
                        let Some(value) = normalized_editable_text(&value) else {
                            return Err(self.record_action_error(error_message));
                        };
                        Ok(Some(value.to_owned()))
                    }
                    None => Ok(None),
                }
            };
        let message = match snapshot.message {
            Some(message) => {
                let Some(message) = normalized_editable_text(&message) else {
                    return self.record_action_error(
                        "GUI media-match runtime snapshots cannot contain an empty issue message.",
                    );
                };
                Some(message)
            }
            None => None,
        };
        if snapshot.health != super::super::shell_state::GuiMediaMatchToolHealth::Healthy
            && message.is_none()
        {
            return self.record_action_error(
            "GUI media-match runtime snapshots must include a non-empty message while unhealthy.",
        );
        }
        let install_location = match normalize_optional_value(
            snapshot.install_location,
            "GUI media-match runtime snapshots cannot contain an empty install location.",
        ) {
            Ok(value) => value,
            Err(result) => return result,
        };
        let ffmpeg_status = match normalize_optional_value(
            snapshot.ffmpeg_status,
            "GUI media-match runtime snapshots cannot contain an empty ffmpeg status.",
        ) {
            Ok(value) => value,
            Err(result) => return result,
        };
        let ffprobe_status = match normalize_optional_value(
            snapshot.ffprobe_status,
            "GUI media-match runtime snapshots cannot contain an empty ffprobe status.",
        ) {
            Ok(value) => value,
            Err(result) => return result,
        };
        self.media_match.settings = snapshot.settings;
        self.media_match.health = snapshot.health;
        self.media_match.message = message;
        self.media_match.install_supported = snapshot.install_supported;
        self.media_match.integration_supported = snapshot.integration_supported;
        self.media_match.install_location = install_location;
        self.media_match.ffmpeg_status = ffmpeg_status;
        self.media_match.ffprobe_status = ffprobe_status;
        self.media_match.cache_status = snapshot
            .cache_status
            .and_then(|value| normalized_editable_text(&value));
        self.media_match.current_decision = snapshot
            .current_decision
            .and_then(|value| normalized_editable_text(&value));
        self.media_match.nearest_match = snapshot
            .nearest_match
            .and_then(|value| normalized_editable_text(&value));
        self.media_match.last_evidence = snapshot
            .last_evidence
            .and_then(|value| normalized_editable_text(&value));
        self.media_match.remote_status = snapshot
            .remote_status
            .and_then(|value| normalized_editable_text(&value));
        self.media_match.background_status = snapshot
            .background_status
            .and_then(|value| normalized_editable_text(&value));
        self.media_match.open_install_location_available = snapshot.open_install_location_available;
        self.refresh_playlist_source_states();
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn apply_gui_media_match_remediation_runtime_snapshot(
        &mut self,
        snapshot: GuiMediaMatchRemediationRuntimeSnapshot,
    ) -> bool {
        let label = match snapshot.label {
            Some(label) => {
                let Some(label) = normalized_editable_text(&label) else {
                    return self.record_action_error(
                        "GUI media-match remediation snapshots cannot contain an empty label.",
                    );
                };
                Some(label)
            }
            None => None,
        };
        let detail = match snapshot.detail {
            Some(detail) => {
                let Some(detail) = normalized_editable_text(&detail) else {
                    return self.record_action_error(
                        "GUI media-match remediation snapshots cannot contain an empty detail.",
                    );
                };
                Some(detail)
            }
            None => None,
        };
        if snapshot.active && label.is_none() {
            return self.record_action_error(
            "GUI media-match remediation snapshots must include a non-empty label while active.",
        );
        }
        if !snapshot.progress_fraction.is_finite()
            || !(0.0..=1.0).contains(&snapshot.progress_fraction)
        {
            return self.record_action_error(
            "GUI media-match remediation snapshots must use a progress value between 0.0 and 1.0.",
        );
        }

        self.media_match_remediation.active = snapshot.active;
        self.media_match_remediation.label = label.filter(|_| snapshot.active);
        self.media_match_remediation.detail = detail.filter(|_| snapshot.active);
        self.media_match_remediation.progress_fraction = if snapshot.active {
            snapshot.progress_fraction
        } else {
            0.0
        };
        self.clear_action_error_and_refresh();
        true
    }

    pub(in crate::app) fn apply_gui_plex_runtime_snapshot(
        &mut self,
        snapshot: GuiPlexRuntimeSnapshot,
    ) -> bool {
        if snapshot.status.trim().is_empty() {
            return self
                .record_action_error("GUI Plex runtime snapshots cannot contain empty status.");
        }
        self.plex.enabled = snapshot.enabled;
        self.plex.streaming_enabled = snapshot.streaming_enabled;
        self.plex.authenticated = snapshot.authenticated;
        self.plex.authenticating = snapshot.authenticating;
        self.plex.auth_code = snapshot
            .auth_code
            .and_then(|value| normalized_editable_text(&value));
        self.plex.auth_url = snapshot
            .auth_url
            .and_then(|value| normalized_editable_text(&value));
        self.plex.selected_server_id = snapshot
            .selected_server_id
            .and_then(|value| normalized_editable_text(&value));
        self.plex.selected_server_url = snapshot
            .selected_server_url
            .and_then(|value| normalized_editable_text(&value));
        self.plex.servers = snapshot
            .servers
            .into_iter()
            .filter_map(|server| {
                Some(GuiPlexServerRow {
                    name: normalized_editable_text(&server.name)?,
                    machine_identifier: normalized_editable_text(&server.machine_identifier)?,
                    uri: normalized_editable_text(&server.uri)?,
                    reachability: server.reachability,
                    connection_kind: server.connection_kind,
                    has_local_connection: server.has_local_connection,
                    owned: server.owned,
                    selected: server.selected,
                })
            })
            .collect();
        self.plex.status = snapshot.status;
        self.plex.current_item = snapshot
            .current_item
            .and_then(|value| normalized_editable_text(&value));
        self.plex.last_report = snapshot
            .last_report
            .and_then(|value| normalized_editable_text(&value));
        self.plex.last_error = snapshot
            .last_error
            .and_then(|value| normalized_editable_text(&value));
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
        patch.apply_to(&mut self.saved_configuration);
        patch.apply_to(&mut self.configuration.settings);

        match patch {
            GuiPersistedSettingsPatch::PluginEnabled { plugin, enabled } => {
                self.plugin_enablement.set_enabled_for(plugin, enabled);
                self.refresh_playlist_source_states();
            }
            GuiPersistedSettingsPatch::MediaMatchFingerprintingEnabled(enabled) => {
                self.media_match.settings.fingerprinting_enabled = enabled;
                self.refresh_playlist_source_states();
            }
            GuiPersistedSettingsPatch::MediaMatchBackgroundWarmupEnabled(enabled) => {
                self.media_match.settings.background_warmup_enabled = enabled;
            }
            GuiPersistedSettingsPatch::MediaMatchWireSharingEnabled(enabled) => {
                self.media_match.settings.wire_sharing_enabled = enabled;
            }
            GuiPersistedSettingsPatch::MediaMatchRuntimeToleranceEnabled(enabled) => {
                self.media_match.settings.runtime_tolerance_enabled = enabled;
            }
            GuiPersistedSettingsPatch::MediaMatchAutoplayPolicy(policy) => {
                self.media_match.settings.autoplay_policy = policy;
            }
            GuiPersistedSettingsPatch::PlexAuthenticated {
                clear_selected_server,
                ..
            } => {
                self.plex.authenticated = true;
                if clear_selected_server {
                    self.plex.selected_server_id = None;
                    self.plex.selected_server_url = None;
                }
            }
            GuiPersistedSettingsPatch::PlexServerSelected {
                machine_identifier,
                uri,
                ..
            } => {
                self.plex.selected_server_id = Some(machine_identifier);
                self.plex.selected_server_url = Some(uri);
            }
            GuiPersistedSettingsPatch::PlexSyncEnabled(enabled) => {
                self.plex.enabled = enabled;
            }
            GuiPersistedSettingsPatch::PlexStreamingEnabled(enabled) => {
                self.plex.streaming_enabled = enabled;
            }
            GuiPersistedSettingsPatch::PlexDisconnected => {
                self.plex.enabled = false;
                self.plex.streaming_enabled = false;
                self.plex.authenticated = false;
                self.plex.selected_server_id = None;
                self.plex.selected_server_url = None;
            }
        }

        self.clear_action_error_and_refresh();
        true
    }
}
