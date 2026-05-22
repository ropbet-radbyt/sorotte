use std::path::Path;

use sorotte_player_api::LocalFileUpdate;

use super::super::runtime_queue::GuiQueuedRuntimeBridgeHandle;
use super::super::runtime_stack::GuiPlayerLaunchRuntimeState;
use super::super::shell_state::{
    GuiCommandAvailabilityState, GuiCommandRuntimeSnapshot, GuiMediaIndexRuntimeSnapshot,
    GuiPendingOperationKind, GuiPlayerSetupIssue, GuiPlayerSetupIssueKind,
    GuiPlayerSetupRuntimeSnapshot, GuiShellAction, MainWindowRuntimeSnapshot,
    MenuActionRuntimeOverride, MenuDialogRuntimeSnapshot, MenuSectionShellState,
    SorotteGuiShellAppState,
};
use super::GuiPersistedConfigRuntimeOwner;

impl GuiPersistedConfigRuntimeOwner {
    fn projected_media_index_runtime_snapshot(&self) -> GuiMediaIndexRuntimeSnapshot {
        if let Some(progress) = self.attached_media_search_progress.as_ref() {
            return GuiMediaIndexRuntimeSnapshot {
                active: true,
                message: Some(Self::media_index_progress_message(progress)),
            };
        }
        if let Some(progress) =
            self.pending_attached_media_resolution
                .as_ref()
                .and_then(|pending| {
                    pending
                        .latest_progress
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .clone()
                })
        {
            return GuiMediaIndexRuntimeSnapshot {
                active: true,
                message: Some(Self::media_index_progress_message(&progress)),
            };
        }
        self.media_index_runtime_snapshot_impl()
    }

    fn menu_action_enabled(
        section: Option<&MenuSectionShellState>,
        action_label: &str,
    ) -> Option<bool> {
        section.and_then(|section| {
            section
                .actions
                .iter()
                .find(|action| action.label == action_label)
                .map(|action| action.enabled)
        })
    }

    fn format_local_file_playlist_entry(local_file: &LocalFileUpdate) -> String {
        let mut details = Vec::new();
        if let Some(duration_seconds) = local_file.duration_seconds {
            details.push(format!("{duration_seconds:.3}s"));
        }
        if let Some(size_bytes) = local_file.size_bytes {
            details.push(format!("{size_bytes} bytes"));
        }
        if details.is_empty() {
            local_file.name.clone()
        } else {
            format!("{} [{}]", local_file.name, details.join(", "))
        }
    }

    pub(super) fn player_local_file_playlist_entries_impl(&self) -> Vec<String> {
        self.player_local_file
            .as_ref()
            .map(Self::format_local_file_playlist_entry)
            .into_iter()
            .collect()
    }

    fn player_setup_issue_impl(&self) -> Option<GuiPlayerSetupIssue> {
        if self.player.is_some() {
            return None;
        }

        let message = self.player_unavailability_reason.as_ref()?.clone();
        let kind = match &self.player_launch_state {
            GuiPlayerLaunchRuntimeState::None => GuiPlayerSetupIssueKind::NotConfigured,
            GuiPlayerLaunchRuntimeState::UnsupportedConfiguredPlayer { .. } => {
                GuiPlayerSetupIssueKind::UnsupportedConfiguredPlayer
            }
            GuiPlayerLaunchRuntimeState::ExplicitMpvIpc { .. } => {
                GuiPlayerSetupIssueKind::IpcAttachFailed
            }
            GuiPlayerLaunchRuntimeState::ManagedMpv(_) => {
                if message.contains("binary does not exist") {
                    GuiPlayerSetupIssueKind::MissingBinary
                } else if message.contains("JSON IPC attach failed") {
                    GuiPlayerSetupIssueKind::IpcAttachFailed
                } else if message.contains("exited") {
                    GuiPlayerSetupIssueKind::ExitedAfterLaunch
                } else {
                    GuiPlayerSetupIssueKind::LaunchFailed
                }
            }
            GuiPlayerLaunchRuntimeState::TestPlayer => return None,
        };

        Some(GuiPlayerSetupIssue { kind, message })
    }

    pub(super) fn player_setup_runtime_snapshot_impl(&self) -> GuiPlayerSetupRuntimeSnapshot {
        GuiPlayerSetupRuntimeSnapshot {
            issue: self.player_setup_issue_impl(),
        }
    }

    pub(super) fn command_availability_for_runtime_state_impl(
        &self,
        state: &SorotteGuiShellAppState,
        player_attached: bool,
    ) -> GuiCommandAvailabilityState {
        let player_runtime_available =
            player_attached || self.player_runtime_available_for_actions();
        let settings = state.configuration.to_stored_settings();
        let busy = state.pending_operation.is_some();
        let chat_unavailable_reason =
            state.chat_send_unavailable_reason_from_settings(&settings, self.session.is_some());
        let command_availability = GuiCommandAvailabilityState {
            can_save_configuration: !busy && state.validation.issues.is_empty(),
            can_reset_configuration: !busy && state.has_unsaved_configuration_changes(),
            can_reload_configuration: !busy,
            can_connect_saved_server: !busy
                && state.saved_session_connect_target().is_some()
                && !state.connect_blocked_by_player_setup_issue(),
            can_disconnect_session: !busy && self.session_active(),
            can_connect_public_server: !busy && state.public_servers.can_connect,
            can_refresh_public_servers: !busy && state.public_servers.can_refresh,
            can_search_missing_media: !busy && state.media_search.can_search_missing_media,
            can_toggle_pause: !busy && player_runtime_available,
            can_send_chat_message: chat_unavailable_reason.is_none(),
            chat_unavailable_reason,
        };
        if let Some(session) = self.session.as_ref() {
            session.adjust_command_availability(state, command_availability)
        } else {
            command_availability
        }
    }

    pub(super) fn placeholder_local_file_for_path(path: &str) -> LocalFileUpdate {
        let name = if path.contains("://") {
            path.to_owned()
        } else {
            Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(path)
                .to_owned()
        };
        LocalFileUpdate::new(name).with_path(path.to_owned())
    }

    pub(super) fn sync_player_runtime_state(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        state: &SorotteGuiShellAppState,
    ) {
        let pre_poll_media_index_status = self.projected_media_index_runtime_snapshot();
        let _ = self.poll_attached_media_search_index_build(
            Self::automatic_media_search_retry_interval(state),
        );
        let player_attached = self.player.is_some();
        let player_runtime_available = self.player_runtime_available_for_actions();
        let can_manage_playlist = self
            .session
            .as_ref()
            .map(|session| {
                state.main_window.shared_playlist_enabled && session.playlist_control_available()
            })
            .unwrap_or(player_runtime_available && state.main_window.shared_playlist_enabled);
        let desired_playlist = if state.main_window.shared_playlist_enabled {
            None
        } else {
            let playlist = self.player_local_file_playlist_entries_impl();
            let playlist_matches = state.main_window.playlist.len() == playlist.len()
                && state
                    .main_window
                    .playlist
                    .iter()
                    .zip(playlist.iter())
                    .all(|(current, desired)| current.label == *desired);
            (!playlist_matches).then_some(playlist)
        };
        let desired_paused = player_attached.then_some(self.player_paused).flatten();
        let main_window_changed = state.main_window.playback.can_toggle_pause
            != player_runtime_available
            || state.main_window.playback.can_seek != player_runtime_available
            || !state.main_window.playback.can_set_offset
            || state.main_window.playback.can_manage_playlist != can_manage_playlist
            || desired_playlist.is_some()
            || desired_paused.is_some_and(|paused| state.main_window.playback_paused != paused)
            || (state.main_window.user_offset_seconds - self.user_offset_seconds).abs()
                > f64::EPSILON;

        if main_window_changed {
            let mut desired_main_window =
                MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
            desired_main_window.can_toggle_pause = player_runtime_available;
            desired_main_window.can_seek = player_runtime_available;
            desired_main_window.can_set_offset = true;
            desired_main_window.can_manage_playlist = can_manage_playlist;
            if let Some(desired_playlist) = desired_playlist {
                desired_main_window.playlist = desired_playlist;
            }
            if let Some(paused) = desired_paused {
                desired_main_window.playback_paused = paused;
            }
            desired_main_window.user_offset_seconds = self.user_offset_seconds;
            handle.push_action(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
                desired_main_window,
            ));
        }

        let mut desired_media_index_status = self.projected_media_index_runtime_snapshot();
        if !desired_media_index_status.active
            && pre_poll_media_index_status.active
            && state
                .pending_operation
                .as_ref()
                .is_some_and(|pending| pending.kind == GuiPendingOperationKind::SearchMissingMedia)
        {
            desired_media_index_status = pre_poll_media_index_status;
        }
        if state.media_index_status.active != desired_media_index_status.active
            || state.media_index_status.message != desired_media_index_status.message
        {
            handle.push_action(GuiShellAction::ApplyGuiMediaIndexRuntimeSnapshot(
                desired_media_index_status,
            ));
        }

        let desired_player_setup = self.player_setup_runtime_snapshot_impl();
        if state.player_setup_issue != desired_player_setup.issue {
            handle.push_action(GuiShellAction::ApplyGuiPlayerSetupRuntimeSnapshot(
                desired_player_setup,
            ));
        }

        let desired_stream_helper = self.stream_helper_runtime_snapshot.clone();
        if state.stream_helper.health != desired_stream_helper.health
            || state.stream_helper.message != desired_stream_helper.message
            || state.stream_helper.target != desired_stream_helper.target
            || state.stream_helper.install_supported != desired_stream_helper.install_supported
            || state.stream_helper.integration_supported
                != desired_stream_helper.integration_supported
            || state.stream_helper.retry_available != desired_stream_helper.retry_available
            || state.stream_helper.install_location != desired_stream_helper.install_location
            || state.stream_helper.downloader_status != desired_stream_helper.downloader_status
            || state.stream_helper.js_runtime_status != desired_stream_helper.js_runtime_status
            || state.stream_helper.open_install_location_available
                != desired_stream_helper.open_install_location_available
        {
            handle.push_action(GuiShellAction::ApplyGuiStreamHelperRuntimeSnapshot(
                desired_stream_helper,
            ));
        }

        let desired_stream_helper_remediation =
            self.stream_helper_remediation_runtime_snapshot.clone();
        if state.stream_helper_remediation.active != desired_stream_helper_remediation.active
            || state.stream_helper_remediation.label != desired_stream_helper_remediation.label
            || state.stream_helper_remediation.detail != desired_stream_helper_remediation.detail
            || (state.stream_helper_remediation.progress_fraction
                - desired_stream_helper_remediation.progress_fraction)
                .abs()
                > f32::EPSILON
        {
            handle.push_action(
                GuiShellAction::ApplyGuiStreamHelperRemediationRuntimeSnapshot(
                    desired_stream_helper_remediation,
                ),
            );
        }

        let desired_media_match = self.media_match_runtime_snapshot.clone();
        if state.media_match.settings != desired_media_match.settings
            || state.media_match.health != desired_media_match.health
            || state.media_match.message != desired_media_match.message
            || state.media_match.install_supported != desired_media_match.install_supported
            || state.media_match.integration_supported != desired_media_match.integration_supported
            || state.media_match.install_location != desired_media_match.install_location
            || state.media_match.ffmpeg_status != desired_media_match.ffmpeg_status
            || state.media_match.ffprobe_status != desired_media_match.ffprobe_status
            || state.media_match.fpcalc_status != desired_media_match.fpcalc_status
            || state.media_match.cache_status != desired_media_match.cache_status
            || state.media_match.current_decision != desired_media_match.current_decision
            || state.media_match.last_evidence != desired_media_match.last_evidence
            || state.media_match.open_install_location_available
                != desired_media_match.open_install_location_available
        {
            handle.push_action(GuiShellAction::ApplyGuiMediaMatchRuntimeSnapshot(
                desired_media_match,
            ));
        }

        let desired_media_match_remediation = self.media_match_remediation_runtime_snapshot.clone();
        if state.media_match_remediation.active != desired_media_match_remediation.active
            || state.media_match_remediation.label != desired_media_match_remediation.label
            || state.media_match_remediation.detail != desired_media_match_remediation.detail
            || (state.media_match_remediation.progress_fraction
                - desired_media_match_remediation.progress_fraction)
                .abs()
                > f32::EPSILON
        {
            handle.push_action(
                GuiShellAction::ApplyGuiMediaMatchRemediationRuntimeSnapshot(
                    desired_media_match_remediation,
                ),
            );
        }

        let playback_section = state
            .menus
            .sections
            .iter()
            .find(|section| section.title == "Playback");
        let advanced_section = state
            .menus
            .sections
            .iter()
            .find(|section| section.title == "Advanced");
        let mut action_overrides = Vec::new();
        for (action_label, enabled) in [
            ("Play", player_attached),
            ("Pause", player_attached),
            ("Toggle Pause", player_attached),
            ("Seek", player_attached),
            (
                "Undo Seek",
                state.pending_operation.is_none() && state.main_window.playback.can_undo_seek,
            ),
            (
                "Shared Playlist",
                self.session
                    .as_ref()
                    .map(|session| {
                        state.main_window.shared_playlist_enabled
                            && session.playlist_control_available()
                    })
                    .unwrap_or(player_attached && state.main_window.shared_playlist_enabled),
            ),
        ] {
            let current_enabled = Self::menu_action_enabled(playback_section, action_label);
            if current_enabled.is_some_and(|current_enabled| current_enabled != enabled) {
                action_overrides.push(MenuActionRuntimeOverride {
                    section_title: "Playback",
                    action_label,
                    enabled,
                });
            }
        }
        let current_offset_enabled = Self::menu_action_enabled(advanced_section, "Set Offset");
        let desired_offset_enabled = state.pending_operation.is_none();
        if current_offset_enabled
            .is_some_and(|current_enabled| current_enabled != desired_offset_enabled)
        {
            action_overrides.push(MenuActionRuntimeOverride {
                section_title: "Advanced",
                action_label: "Set Offset",
                enabled: desired_offset_enabled,
            });
        }
        if !action_overrides.is_empty() {
            handle.push_action(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
                MenuDialogRuntimeSnapshot {
                    action_overrides,
                    tls_prompt_expected: state.menus.tls_prompt_expected,
                    update_notice_expected: state.menus.update_notice_expected,
                    about_dialog_available: state.menus.about_dialog_available,
                },
            ));
        }

        let desired_command_availability =
            self.command_availability_for_runtime_state_impl(state, player_attached);
        if state.commands != desired_command_availability {
            handle.push_action(GuiShellAction::ApplyGuiCommandRuntimeSnapshot(
                GuiCommandRuntimeSnapshot {
                    command_availability: desired_command_availability,
                    pending_operation: state.pending_operation.as_ref().map(|pending| pending.kind),
                },
            ));
        }
    }
}
