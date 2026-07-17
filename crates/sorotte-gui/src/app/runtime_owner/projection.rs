use std::path::Path;

use sorotte_client_core::{
    PlaybackCoordinationSnapshot, SeekPreparationDegradedReason, SeekPreparationPhase,
    SeekPreparationTerminalOutcome,
};
use sorotte_player_api::LocalFileUpdate;

use super::super::runtime_queue::GuiQueuedRuntimeBridgeHandle;
use super::super::runtime_stack::GuiPlayerLaunchRuntimeState;
use super::super::shell_state::{
    GuiCommandAvailabilityState, GuiCommandRuntimeSnapshot, GuiMediaIndexRuntimeSnapshot,
    GuiPendingOperationKind, GuiPlayerSetupIssue, GuiPlayerSetupIssueKind,
    GuiPlayerSetupRuntimeSnapshot, GuiSeekPreparationDegradedReason, GuiSeekPreparationPhase,
    GuiSeekPreparationRuntimeSnapshot, GuiSeekPreparationState, GuiShellAction,
    MainWindowRuntimeSnapshot, MenuActionId, MenuActionRuntimeOverride, MenuDialogRuntimeSnapshot,
    SorotteGuiShellAppState,
};
use super::GuiPersistedConfigRuntimeOwner;

fn gui_seek_preparation_degraded_reason(
    reason: SeekPreparationDegradedReason,
) -> GuiSeekPreparationDegradedReason {
    match reason {
        SeekPreparationDegradedReason::NonSeekable => GuiSeekPreparationDegradedReason::NonSeekable,
        SeekPreparationDegradedReason::OutsideLiveWindow => {
            GuiSeekPreparationDegradedReason::OutsideLiveWindow
        }
        SeekPreparationDegradedReason::TimedOut => GuiSeekPreparationDegradedReason::TimedOut,
        SeekPreparationDegradedReason::TimelineWindowUnavailable => {
            GuiSeekPreparationDegradedReason::TimelineWindowUnavailable
        }
        SeekPreparationDegradedReason::TransportFailed => {
            GuiSeekPreparationDegradedReason::TransportFailed
        }
    }
}

fn current_seek_handoff_recovery_is_degraded(snapshot: &PlaybackCoordinationSnapshot) -> bool {
    let Some(media_generation) = snapshot.media_generation else {
        return false;
    };
    let handoff_terminal = snapshot
        .last_seek_preparation_terminal
        .as_ref()
        .is_some_and(|terminal| {
            terminal.media_generation == media_generation
                && matches!(
                    terminal.terminal_outcome,
                    Some(
                        SeekPreparationTerminalOutcome::Ready
                            | SeekPreparationTerminalOutcome::Superseded
                    )
                )
        });
    let degraded_recovery = snapshot
        .recovery_episode
        .as_ref()
        .is_some_and(|recovery| recovery.media_generation == media_generation && recovery.degraded);
    handoff_terminal && degraded_recovery
}

fn projected_seek_preparation_runtime_snapshot(
    coordination: Option<&PlaybackCoordinationSnapshot>,
) -> GuiSeekPreparationRuntimeSnapshot {
    if coordination.is_some_and(current_seek_handoff_recovery_is_degraded) {
        return GuiSeekPreparationRuntimeSnapshot {
            preparation: None,
            degraded_reason: Some(GuiSeekPreparationDegradedReason::ConvergenceDegraded),
        };
    }

    let preparation = coordination
        .and_then(|snapshot| snapshot.seek_preparation.as_ref())
        .map(|preparation| GuiSeekPreparationState {
            phase: match preparation.phase {
                SeekPreparationPhase::Seeking => GuiSeekPreparationPhase::Seeking,
                SeekPreparationPhase::Fetching => GuiSeekPreparationPhase::Fetching,
                SeekPreparationPhase::Refilling => GuiSeekPreparationPhase::Refilling,
                SeekPreparationPhase::ReadyToJoin => GuiSeekPreparationPhase::ReadyToJoin,
                SeekPreparationPhase::CatchingUp => GuiSeekPreparationPhase::CatchingUp,
            },
            frozen_target_seconds: preparation.frozen_target_seconds,
            cache_refill_percent: preparation.cache_buffering_percent,
            buffered_ahead_seconds: preparation.buffered_ahead_seconds,
            nearest_safe_buffered_position_seconds: preparation
                .nearest_safe_buffered_position_seconds,
            can_keep_waiting: preparation.can_keep_waiting,
            can_cancel_and_remain: preparation.can_cancel_and_remain,
            can_join_nearest_buffered: preparation.can_join_nearest_buffered,
        });
    let degraded_reason = if preparation.is_none() {
        coordination
            .and_then(|snapshot| {
                snapshot
                    .last_seek_preparation_terminal
                    .as_ref()
                    .filter(|terminal| snapshot.media_generation == Some(terminal.media_generation))
            })
            .and_then(|terminal| terminal.terminal_outcome)
            .and_then(|outcome| match outcome {
                SeekPreparationTerminalOutcome::Degraded(reason) => {
                    Some(gui_seek_preparation_degraded_reason(reason))
                }
                _ => None,
            })
    } else {
        None
    };
    GuiSeekPreparationRuntimeSnapshot {
        preparation,
        degraded_reason,
    }
}

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
            if let Some(reason) = self
                .core_player_configuration_health
                .streaming_degraded_reason()
            {
                return Some(GuiPlayerSetupIssue {
                    kind: GuiPlayerSetupIssueKind::PlayerSettingsDegraded,
                    message: reason.to_owned(),
                    retry_available: self
                        .core_player_configuration_health
                        .streaming_retryable_in_place(),
                });
            }
            if let Some(reason) = self.player_integration_health.bridge_degraded_reason() {
                return Some(GuiPlayerSetupIssue {
                    kind: GuiPlayerSetupIssueKind::BridgeDegraded,
                    message: reason.to_owned(),
                    retry_available: self.player_integration_health.bridge_retryable_in_place(),
                });
            }
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

        Some(GuiPlayerSetupIssue {
            retry_available: kind != GuiPlayerSetupIssueKind::NotConfigured,
            kind,
            message,
        })
    }

    pub(super) fn player_setup_runtime_snapshot_impl(&self) -> GuiPlayerSetupRuntimeSnapshot {
        GuiPlayerSetupRuntimeSnapshot {
            issue: self.player_setup_issue_impl(),
        }
    }

    pub(super) fn seek_preparation_runtime_snapshot_impl(
        &self,
    ) -> GuiSeekPreparationRuntimeSnapshot {
        let coordination = self
            .session
            .as_ref()
            .and_then(|session| session.playback_coordination_snapshot());
        projected_seek_preparation_runtime_snapshot(coordination.as_ref())
    }

    pub(super) fn command_availability_for_runtime_state_impl(
        &self,
        state: &SorotteGuiShellAppState,
        player_attached: bool,
    ) -> GuiCommandAvailabilityState {
        let player_runtime_available =
            player_attached || self.player_runtime_available_for_actions();
        let settings = self
            .active_session_settings
            .as_ref()
            .filter(|_| self.session_projects_to_shell)
            .map(|runtime_settings| runtime_settings.settings.clone())
            .unwrap_or_else(|| state.configuration.to_stored_settings());
        let busy = state.pending_operation.is_some();
        let chat_unavailable_reason =
            state.chat_send_unavailable_reason_from_settings(&settings, self.session.is_some());
        let command_availability = GuiCommandAvailabilityState {
            can_save_configuration: !busy
                && state.validation.issues.is_empty()
                && state.has_unsaved_configuration_changes(),
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
        let size_bytes = if path.contains("://") {
            0
        } else {
            std::fs::metadata(path)
                .map(|metadata| metadata.len())
                .unwrap_or_default()
        };
        LocalFileUpdate::new(name)
            .with_size_bytes(size_bytes)
            .with_path(path.to_owned())
    }

    pub(super) fn sync_player_runtime_state(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        state: &SorotteGuiShellAppState,
    ) {
        let pre_poll_media_index_status = self.projected_media_index_runtime_snapshot();
        let pre_poll_media_index_revision = self.attached_media_search_index_revision;
        let media_index_was_pending = self.pending_attached_media_resolution.is_some();
        let media_index_still_pending = self.poll_attached_media_search_index_build(
            self.automatic_media_search_retry_interval(state),
        );
        if media_index_was_pending
            && !media_index_still_pending
            && self.attached_media_search_index_revision != pre_poll_media_index_revision
        {
            let mut projected_state = state.clone();
            let _ = self.retry_pending_playlist_source_resolution(handle, &mut projected_state);
            self.sync_active_shared_playlist_media_and_playstate_impl(&projected_state);
        }
        let player_attached = self.player.is_some();
        let player_runtime_available = self.player_runtime_available_for_actions();
        let shared_playlist_enabled = self.runtime_shared_playlist_enabled(state);
        let can_manage_playlist = self
            .session
            .as_ref()
            .map(|session| shared_playlist_enabled && session.playlist_control_available())
            .unwrap_or(player_runtime_available && shared_playlist_enabled);
        let desired_playlist = if shared_playlist_enabled {
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
        let main_window_changed = state.main_window.shared_playlist_enabled
            != shared_playlist_enabled
            || state.main_window.playback.can_toggle_pause != player_runtime_available
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
            desired_main_window.shared_playlist_enabled = shared_playlist_enabled;
            desired_main_window.can_toggle_pause = player_runtime_available;
            desired_main_window.can_seek = player_runtime_available;
            desired_main_window.can_set_offset = true;
            desired_main_window.can_manage_playlist = can_manage_playlist;
            if let Some(desired_playlist) = desired_playlist {
                desired_main_window.playlist = desired_playlist;
                desired_main_window.playlist_entry_ids.clear();
                desired_main_window.playlist_source_states.clear();
                desired_main_window.active_playlist_index = None;
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

        let desired_seek_preparation = self.seek_preparation_runtime_snapshot_impl();
        if state.seek_preparation != desired_seek_preparation.preparation
            || state.seek_preparation_degraded_reason != desired_seek_preparation.degraded_reason
        {
            handle.push_action(GuiShellAction::ApplyGuiSeekPreparationRuntimeSnapshot(
                desired_seek_preparation,
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
            || state.media_match.cache_status != desired_media_match.cache_status
            || state.media_match.current_decision != desired_media_match.current_decision
            || state.media_match.nearest_match != desired_media_match.nearest_match
            || state.media_match.last_evidence != desired_media_match.last_evidence
            || state.media_match.remote_status != desired_media_match.remote_status
            || state.media_match.background_status != desired_media_match.background_status
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

        let playback_controls_available =
            state.pending_operation.is_none() && !state.main_window.playlist.is_empty();
        let mut action_overrides = Vec::new();
        for (id, enabled) in [
            (
                MenuActionId::Play,
                player_runtime_available && playback_controls_available,
            ),
            (
                MenuActionId::Pause,
                player_runtime_available && playback_controls_available,
            ),
            (
                MenuActionId::TogglePause,
                player_runtime_available && playback_controls_available,
            ),
            (
                MenuActionId::Seek,
                player_runtime_available && playback_controls_available,
            ),
            (
                MenuActionId::UndoSeek,
                playback_controls_available && state.main_window.playback.can_undo_seek,
            ),
            (MenuActionId::SharedPlaylist, can_manage_playlist),
        ] {
            let current_enabled = state.menus.action(id).map(|action| action.enabled);
            if current_enabled.is_some_and(|current_enabled| current_enabled != enabled) {
                action_overrides.push(MenuActionRuntimeOverride { id, enabled });
            }
        }
        let current_offset_enabled = state
            .menus
            .action(MenuActionId::SetOffset)
            .map(|action| action.enabled);
        let desired_offset_enabled =
            playback_controls_available && state.main_window.playback.can_set_offset;
        if current_offset_enabled
            .is_some_and(|current_enabled| current_enabled != desired_offset_enabled)
        {
            action_overrides.push(MenuActionRuntimeOverride {
                id: MenuActionId::SetOffset,
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

#[cfg(test)]
mod seek_preparation_projection_tests {
    use sorotte_client_core::{
        PlaybackDiagnostic, RecoveryEpisodeSnapshot, SeekPreparationSnapshot,
        SeekTargetAvailability,
    };

    use super::*;

    fn ready_terminal(media_generation: u64) -> SeekPreparationSnapshot {
        SeekPreparationSnapshot {
            id: 7,
            media_generation,
            load_attempt: 1,
            room_revision: 12,
            latest_room_revision: 12,
            requested_target_seconds: 135.0,
            frozen_target_seconds: 135.0,
            frozen_room_anchor_position_seconds: 135.0,
            frozen_room_anchor_observed_at_seconds: 10.0,
            latest_room_position_seconds: 138.0,
            availability: SeekTargetAvailability::Cached,
            phase: SeekPreparationPhase::CatchingUp,
            cache_buffering_percent: Some(100.0),
            buffered_ahead_seconds: Some(30.0),
            nearest_safe_buffered_position_seconds: None,
            started_at_seconds: 10.0,
            terminal_outcome: Some(SeekPreparationTerminalOutcome::Ready),
            can_keep_waiting: false,
            can_cancel_and_remain: false,
            can_join_nearest_buffered: false,
        }
    }

    fn snapshot_with_ready_terminal(media_generation: u64) -> PlaybackCoordinationSnapshot {
        let terminal = ready_terminal(media_generation);
        PlaybackCoordinationSnapshot {
            media_generation: Some(media_generation),
            pending_local_pause_intent: None,
            pending_local_pause_intent_dormant: false,
            last_local_pause_intent_stage_accepted: None,
            diagnostic: PlaybackDiagnostic::ReadyWaitingForRoom,
            recovery_episode: Some(RecoveryEpisodeSnapshot {
                id: 9,
                media_generation,
                entered_at_seconds: 11.0,
                hard_seek_attempts: 1,
                stable_since_seconds: None,
                catchup_deadline_seconds: Some(20.0),
                degraded: false,
            }),
            seek_preparation: Some(terminal.clone()),
            last_seek_preparation_terminal_outcome: terminal.terminal_outcome,
            last_seek_preparation_terminal: Some(terminal),
            metrics: Default::default(),
            transport_telemetry_observed: true,
            ordinary_correction_blocked: false,
            last_applied_revision: Some(12),
            last_started_revision: None,
            last_degraded_reason: None,
        }
    }

    fn snapshot_with_superseded_terminal(media_generation: u64) -> PlaybackCoordinationSnapshot {
        let mut snapshot = snapshot_with_ready_terminal(media_generation);
        snapshot.seek_preparation = None;
        snapshot.last_seek_preparation_terminal_outcome =
            Some(SeekPreparationTerminalOutcome::Superseded);
        snapshot
            .last_seek_preparation_terminal
            .as_mut()
            .unwrap()
            .terminal_outcome = Some(SeekPreparationTerminalOutcome::Superseded);
        snapshot
    }

    #[test]
    fn current_ready_seek_with_degraded_recovery_projects_terminal_convergence_status() {
        let mut snapshot = snapshot_with_ready_terminal(3);

        let active = projected_seek_preparation_runtime_snapshot(Some(&snapshot));
        assert_eq!(
            active.preparation.as_ref().map(|state| state.phase),
            Some(GuiSeekPreparationPhase::CatchingUp)
        );
        assert_eq!(active.degraded_reason, None);

        snapshot.recovery_episode.as_mut().unwrap().degraded = true;
        let degraded = projected_seek_preparation_runtime_snapshot(Some(&snapshot));
        assert_eq!(degraded.preparation, None);
        assert_eq!(
            degraded.degraded_reason,
            Some(GuiSeekPreparationDegradedReason::ConvergenceDegraded)
        );
    }

    #[test]
    fn current_join_nearest_handoff_with_degraded_recovery_projects_convergence_status() {
        let mut snapshot = snapshot_with_superseded_terminal(3);

        let active = projected_seek_preparation_runtime_snapshot(Some(&snapshot));
        assert_eq!(active, GuiSeekPreparationRuntimeSnapshot::default());

        snapshot.recovery_episode.as_mut().unwrap().degraded = true;
        let degraded = projected_seek_preparation_runtime_snapshot(Some(&snapshot));
        assert_eq!(degraded.preparation, None);
        assert_eq!(
            degraded.degraded_reason,
            Some(GuiSeekPreparationDegradedReason::ConvergenceDegraded)
        );
    }

    #[test]
    fn convergence_degradation_requires_current_media_terminal_and_recovery() {
        let mut stale_terminal = snapshot_with_ready_terminal(3);
        stale_terminal.recovery_episode.as_mut().unwrap().degraded = true;
        stale_terminal
            .last_seek_preparation_terminal
            .as_mut()
            .unwrap()
            .media_generation = 2;
        assert_ne!(
            projected_seek_preparation_runtime_snapshot(Some(&stale_terminal)).degraded_reason,
            Some(GuiSeekPreparationDegradedReason::ConvergenceDegraded)
        );

        let mut stale_recovery = snapshot_with_ready_terminal(3);
        let recovery = stale_recovery.recovery_episode.as_mut().unwrap();
        recovery.media_generation = 2;
        recovery.degraded = true;
        assert_ne!(
            projected_seek_preparation_runtime_snapshot(Some(&stale_recovery)).degraded_reason,
            Some(GuiSeekPreparationDegradedReason::ConvergenceDegraded)
        );

        let mut stale_join_handoff = snapshot_with_superseded_terminal(3);
        stale_join_handoff
            .recovery_episode
            .as_mut()
            .unwrap()
            .degraded = true;
        stale_join_handoff
            .last_seek_preparation_terminal
            .as_mut()
            .unwrap()
            .media_generation = 2;
        assert_ne!(
            projected_seek_preparation_runtime_snapshot(Some(&stale_join_handoff)).degraded_reason,
            Some(GuiSeekPreparationDegradedReason::ConvergenceDegraded)
        );
    }
}
