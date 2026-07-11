use sorotte_client_app::app_boundary::application::{
    ClientApplication, ClientCommand, ClientEvent,
};
use sorotte_client_app::app_boundary::commands::{
    PlannedLocalRuntimeAction, plan_local_runtime_dispatch_legacy_compatible,
};
use sorotte_player_api::PlayerPlaybackTelemetryUpdate;
use sorotte_player_mpv::MpvAdapter;

use crate::client_config::ClientLoopConfig;
use crate::language_support::current_legacy_runtime_language_tag_legacy_compatible;

pub(super) const PLAYER_CHAT_INPUT_POLL_INTERVAL_MS: u64 = 100;

pub(super) fn publish_pending_local_file_updates(
    application: &mut ClientApplication<MpvAdapter>,
    config: &ClientLoopConfig,
) -> anyhow::Result<()> {
    loop {
        let published = application.publish_pending_local_file_update_legacy_compatible(
            config.filename_privacy_mode,
            config.filesize_privacy_mode,
        )?;
        if !published {
            break;
        }
    }
    Ok(())
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
        Some(PlannedLocalRuntimeAction::SetUserOffset(_)) => None,
        Some(PlannedLocalRuntimeAction::SeekToPosition(position_seconds)) => {
            Some(ClientCommand::SeekToPosition(position_seconds))
        }
        Some(PlannedLocalRuntimeAction::SeekByOffset(offset_seconds)) => {
            Some(ClientCommand::SeekByOffset(offset_seconds))
        }
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
        Some(PlannedLocalRuntimeAction::ToggleReady) => Some(ClientCommand::SetReady {
            username: None,
            ready: None,
            manually_initiated: true,
        }),
        Some(PlannedLocalRuntimeAction::SetUserReady { username, ready }) => {
            Some(ClientCommand::SetReady {
                username: Some(username),
                ready: Some(ready),
                manually_initiated: true,
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
