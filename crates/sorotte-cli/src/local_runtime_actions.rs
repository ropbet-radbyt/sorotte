use sorotte_client_app::app_boundary::application::{
    ClientApplication, ClientCommand, ClientEvent,
};
use sorotte_client_app::app_boundary::commands::{
    PlannedLocalRuntimeAction, plan_local_runtime_dispatch_legacy_compatible,
};
use sorotte_player_api::{PlayerError, PlayerPlaybackTelemetryUpdate};
use sorotte_player_mpv::{MpvAdapter, MpvNetworkMediaOptionsTransitionOutcome};
use sorotte_protocol::DirectReadinessSurface;

use crate::client_config::ClientLoopConfig;
use crate::language_support::current_legacy_runtime_language_tag_legacy_compatible;

pub(super) const PLAYER_CHAT_INPUT_POLL_INTERVAL_MS: u64 = 100;

#[derive(Default)]
struct NetworkMediaOptionsWarningState {
    hook_failure: Option<PlayerError>,
    policy_failure: Option<PlayerError>,
}

impl NetworkMediaOptionsWarningState {
    fn current_failure(&self) -> Option<&PlayerError> {
        self.hook_failure.as_ref().or(self.policy_failure.as_ref())
    }
}

pub(super) fn publish_pending_local_file_updates(
    application: &mut ClientApplication<MpvAdapter>,
    config: &ClientLoopConfig,
) -> anyhow::Result<()> {
    loop {
        let published = application.publish_pending_local_file_update_legacy_compatible(
            config.filename_privacy_mode,
            config.filesize_privacy_mode,
        );
        surface_network_media_options_transition_outcomes(application)?;
        let published = published?;
        if !published {
            break;
        }
    }
    Ok(())
}

fn surface_network_media_options_transition_outcomes(
    application: &mut ClientApplication<MpvAdapter>,
) -> anyhow::Result<()> {
    let mut warning_state = NetworkMediaOptionsWarningState::default();
    loop {
        let (outcome, player_connected) = application.with_player_io(|player| {
            (
                player.take_network_media_options_transition_outcome(),
                player.is_connected(),
            )
        });
        let Some(outcome) = outcome else {
            if let Some(error) = warning_state.current_failure() {
                eprintln!("{}", network_media_options_warning_message(error));
            }
            return Ok(());
        };
        if let Err(error) = fold_network_media_options_transition_outcome(
            &mut warning_state,
            outcome,
            player_connected,
        ) {
            return Err(anyhow::anyhow!(
                "mpv JSON IPC became unavailable while applying streaming options to externally activated network media: {error}"
            ));
        }
    }
}

fn network_media_options_warning_message(error: &PlayerError) -> String {
    format!(
        "warning: mpv playback remains available, but streaming options need attention: {error}; desired options remain configured for later network transitions"
    )
}

fn fold_network_media_options_transition_outcome(
    warning_state: &mut NetworkMediaOptionsWarningState,
    outcome: MpvNetworkMediaOptionsTransitionOutcome,
    player_connected: bool,
) -> Result<(), PlayerError> {
    match outcome {
        MpvNetworkMediaOptionsTransitionOutcome::HookRecovered => {
            warning_state.hook_failure = None;
            Ok(())
        }
        MpvNetworkMediaOptionsTransitionOutcome::NoActiveMedia
        | MpvNetworkMediaOptionsTransitionOutcome::LocalMediaUnchanged
        | MpvNetworkMediaOptionsTransitionOutcome::NetworkMediaUpdated => {
            warning_state.policy_failure = None;
            Ok(())
        }
        MpvNetworkMediaOptionsTransitionOutcome::HookDegraded(error) if !player_connected => {
            Err(error)
        }
        MpvNetworkMediaOptionsTransitionOutcome::HookDegraded(error) => {
            warning_state.hook_failure = Some(error);
            Ok(())
        }
        MpvNetworkMediaOptionsTransitionOutcome::Failed(error) if !player_connected => Err(error),
        MpvNetworkMediaOptionsTransitionOutcome::Failed(error) => {
            warning_state.policy_failure = Some(error);
            Ok(())
        }
    }
}

pub(super) fn drain_player_chat_input_legacy_compatible(
    application: &mut ClientApplication<MpvAdapter>,
) -> anyhow::Result<bool> {
    Ok(application.run_player_chat_input_if_needed()? > 0)
}

fn command_result(events: Vec<ClientEvent>) -> anyhow::Result<bool> {
    if let Some(ClientEvent::OperationFailed { message, .. }) = events
        .iter()
        .find(|event| matches!(event, ClientEvent::OperationFailed { .. }))
    {
        return Err(anyhow::anyhow!(message.clone()));
    }
    Ok(events
        .iter()
        .find_map(ClientEvent::command_changed)
        .unwrap_or(false))
}

pub(super) fn run_planned_local_runtime_action_legacy_compatible(
    application: &mut ClientApplication<MpvAdapter>,
    user_offset_seconds: &mut f64,
    now_seconds: f64,
    action: PlannedLocalRuntimeAction,
) -> anyhow::Result<bool> {
    let language = current_legacy_runtime_language_tag_legacy_compatible();
    let dispatch = plan_local_runtime_dispatch_legacy_compatible(
        application.session(),
        *user_offset_seconds,
        action,
        language.as_deref(),
    );
    if let Some(updated_user_offset_seconds) = dispatch.updated_user_offset_seconds {
        *user_offset_seconds = updated_user_offset_seconds;
    }
    if let Some(line_to_emit) = dispatch.line_to_emit {
        println!("{line_to_emit}");
    }

    let command = match dispatch.action {
        Some(PlannedLocalRuntimeAction::SendChat(message)) => {
            Some(ClientCommand::SendChat(message))
        }
        Some(PlannedLocalRuntimeAction::RequestUserList) => Some(ClientCommand::RequestUserList),
        Some(PlannedLocalRuntimeAction::SetPlaylistIndex(index)) => {
            Some(ClientCommand::SetPlaylistIndex(index))
        }
        Some(PlannedLocalRuntimeAction::AdvancePlaylistIndex) => {
            Some(ClientCommand::AdvancePlaylistIndex)
        }
        Some(PlannedLocalRuntimeAction::QueuePlaylistItem {
            file_name,
            select_after_queue,
        }) => Some(ClientCommand::QueuePlaylistItem {
            file_name,
            select_after_queue,
        }),
        Some(PlannedLocalRuntimeAction::DeletePlaylistIndex(index)) => {
            Some(ClientCommand::DeletePlaylistIndex(index))
        }
        Some(PlannedLocalRuntimeAction::UndoPlaylistChange) => {
            Some(ClientCommand::UndoPlaylistChange)
        }
        Some(PlannedLocalRuntimeAction::ShuffleRemainingPlaylist) => {
            Some(ClientCommand::ShuffleRemainingPlaylist)
        }
        Some(PlannedLocalRuntimeAction::ShuffleEntirePlaylist) => {
            Some(ClientCommand::ShuffleEntirePlaylist)
        }
        Some(PlannedLocalRuntimeAction::UndoSeek) => Some(ClientCommand::UndoSeek),
        Some(PlannedLocalRuntimeAction::KeepWaitingForSeekPreparation) => {
            return Ok(application.run_keep_waiting_for_seek_preparation(now_seconds)?);
        }
        Some(PlannedLocalRuntimeAction::JoinNearestBufferedSeekPreparation) => {
            return Ok(application.run_join_nearest_buffered_seek_preparation(now_seconds)?);
        }
        Some(PlannedLocalRuntimeAction::CancelSeekPreparation) => {
            return Ok(application.run_cancel_seek_preparation(now_seconds)?);
        }
        Some(PlannedLocalRuntimeAction::SetUserOffset(_)) => None,
        Some(PlannedLocalRuntimeAction::SeekToPosition(position_seconds)) => {
            Some(ClientCommand::SeekToPosition(position_seconds))
        }
        Some(PlannedLocalRuntimeAction::SeekByOffset(offset_seconds)) => {
            Some(ClientCommand::SeekByOffset(offset_seconds))
        }
        Some(PlannedLocalRuntimeAction::Play) => Some(ClientCommand::SetPaused(false)),
        Some(PlannedLocalRuntimeAction::Pause) => Some(ClientCommand::SetPaused(true)),
        Some(PlannedLocalRuntimeAction::TogglePause) => {
            let paused = application.player().paused();
            let position_seconds = application.player().position_seconds();
            let _ = application.dispatch(ClientCommand::PlayerPlaybackObserved(
                PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(paused)
                    .with_position_seconds(position_seconds),
            ));
            Some(ClientCommand::TogglePause)
        }
        Some(PlannedLocalRuntimeAction::ToggleReady) => Some(ClientCommand::SetReadyFrom {
            username: None,
            ready: None,
            manually_initiated: true,
            surface: DirectReadinessSurface::CliCommand,
        }),
        Some(PlannedLocalRuntimeAction::SetUserReady { username, ready }) => {
            Some(ClientCommand::SetReadyFrom {
                username: Some(username),
                ready: Some(ready),
                manually_initiated: true,
                surface: DirectReadinessSurface::CliCommand,
            })
        }
        Some(PlannedLocalRuntimeAction::RequestControllerAuth { room, password }) => {
            Some(ClientCommand::RequestControllerAuth { room, password })
        }
        Some(PlannedLocalRuntimeAction::SetRoomWithLegacyFallback(room)) => {
            Some(ClientCommand::SetRoom {
                room,
                legacy_fallback: true,
            })
        }
        Some(PlannedLocalRuntimeAction::SetRoom(room)) => Some(ClientCommand::SetRoom {
            room,
            legacy_fallback: false,
        }),
        None => None,
    };

    command.map_or(Ok(false), |command| {
        command_result(application.dispatch(command))
    })
}

#[cfg(test)]
mod network_media_options_transition_outcome_tests {
    use super::*;
    use sorotte_player_api::PlayerError;

    #[test]
    fn network_policy_success_supersedes_only_a_pending_policy_failure() {
        let mut warning_state = NetworkMediaOptionsWarningState::default();
        fold_network_media_options_transition_outcome(
            &mut warning_state,
            MpvNetworkMediaOptionsTransitionOutcome::Failed(PlayerError::OperationFailed(
                "test option rejection".to_owned(),
            )),
            true,
        )
        .expect("a healthy rejection should be buffered as a warning");
        assert!(warning_state.policy_failure.is_some());

        fold_network_media_options_transition_outcome(
            &mut warning_state,
            MpvNetworkMediaOptionsTransitionOutcome::NetworkMediaUpdated,
            true,
        )
        .expect("a later successful transition should remain healthy");

        assert!(
            warning_state.policy_failure.is_none(),
            "a successful outcome in the same drain batch must suppress the stale warning"
        );
    }

    #[test]
    fn idle_and_local_policy_state_do_not_clear_a_hook_warning() {
        let mut warning_state = NetworkMediaOptionsWarningState::default();
        fold_network_media_options_transition_outcome(
            &mut warning_state,
            MpvNetworkMediaOptionsTransitionOutcome::HookDegraded(PlayerError::OperationFailed(
                "test hook loss".to_owned(),
            )),
            true,
        )
        .expect("a healthy hook failure should remain a scoped warning");

        for outcome in [
            MpvNetworkMediaOptionsTransitionOutcome::NoActiveMedia,
            MpvNetworkMediaOptionsTransitionOutcome::LocalMediaUnchanged,
        ] {
            fold_network_media_options_transition_outcome(&mut warning_state, outcome, true)
                .expect("media policy completion should remain healthy");
            assert!(
                warning_state.hook_failure.is_some(),
                "media policy state must not clear independent hook health"
            );
        }

        fold_network_media_options_transition_outcome(
            &mut warning_state,
            MpvNetworkMediaOptionsTransitionOutcome::HookRecovered,
            true,
        )
        .expect("positive hook recovery should remain healthy");
        assert!(warning_state.hook_failure.is_none());
    }

    #[test]
    fn credential_bearing_targets_never_reach_cli_warning_or_fatal_text() {
        const CANARY: &str = "SOROTTE-CLI-RAW-TARGET-CANARY";
        let cases = [
            (
                format!("https://alice:{CANARY}@example.test/media"),
                "https://cdn.example.test/media".to_owned(),
            ),
            (
                format!("https://example.test/media?sig={CANARY}"),
                format!("https://example.test/media?sig={CANARY}"),
            ),
            (
                format!("https://example.test/media?auth={CANARY}"),
                format!("https://example.test/media?auth={CANARY}"),
            ),
            (
                format!("https://example.test/media?X-Amz-Signature={CANARY}"),
                format!("https://example.test/media?X-Amz-Signature={CANARY}"),
            ),
            (
                "https://example.test/watch/1".to_owned(),
                format!("edl://nested.example.test/video?token={CANARY}"),
            ),
            (
                format!("C:/Users/{CANARY}/private/movie.mkv"),
                format!("https://cdn.example.test/video?opaque={CANARY}"),
            ),
        ];

        for (source, resolved) in cases {
            let mut adapter = MpvAdapter::default();
            adapter.inject_test_network_media_options_policy_failure(42, &source, &resolved);
            let outcome = adapter
                .take_network_media_options_transition_outcome()
                .expect("the fixture should queue a sanitized policy failure");
            let mut warning_state = NetworkMediaOptionsWarningState::default();
            fold_network_media_options_transition_outcome(&mut warning_state, outcome, true)
                .expect("an attached-player failure should remain scoped");
            let warning = network_media_options_warning_message(
                warning_state
                    .current_failure()
                    .expect("the scoped warning should remain active"),
            );
            assert!(!warning.contains(CANARY));
            assert!(!warning.contains(&source));
            assert!(!warning.contains(&resolved));
            assert!(warning.contains("hook load 42"));

            let mut adapter = MpvAdapter::default();
            adapter.inject_test_network_media_options_policy_failure(42, &source, &resolved);
            let fatal = fold_network_media_options_transition_outcome(
                &mut NetworkMediaOptionsWarningState::default(),
                adapter
                    .take_network_media_options_transition_outcome()
                    .expect("the second fixture should queue a failure"),
                false,
            )
            .expect_err("a disconnected player should surface a fatal error")
            .to_string();
            assert!(!fatal.contains(CANARY));
            assert!(!fatal.contains(&source));
            assert!(!fatal.contains(&resolved));
        }
    }
}
