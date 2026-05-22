use sorotte_client_app::app_boundary::commands::{
    PlannedLocalRuntimeAction, plan_local_runtime_dispatch_legacy_compatible,
};
use sorotte_client_core::{ClientRuntime, QueuedRuntimeControl};
use sorotte_player_api::PlayerPlaybackTelemetryUpdate;
use sorotte_player_mpv::MpvAdapter;

use crate::client_config::ClientLoopConfig;
use crate::language_support::current_legacy_runtime_language_tag_legacy_compatible;
pub(super) const PLAYER_CHAT_INPUT_POLL_INTERVAL_MS: u64 = 100;

pub(super) fn publish_pending_local_file_updates(
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    config: &ClientLoopConfig,
) -> anyhow::Result<()> {
    loop {
        let published = runtime.publish_pending_local_file_update_legacy_compatible(
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
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
) -> anyhow::Result<bool> {
    Ok(runtime.run_player_chat_input_if_needed()? > 0)
}

pub(super) fn run_planned_local_runtime_action_legacy_compatible(
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    user_offset_seconds: &mut f64,
    action: PlannedLocalRuntimeAction,
) -> anyhow::Result<bool> {
    let language = current_legacy_runtime_language_tag_legacy_compatible();
    let dispatch = plan_local_runtime_dispatch_legacy_compatible(
        runtime.session(),
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
    match dispatch.action {
        Some(PlannedLocalRuntimeAction::SendChat(chat_message)) => {
            Ok(runtime.run_send_chat_message(chat_message)?)
        }
        Some(PlannedLocalRuntimeAction::RequestUserList) => Ok(runtime.run_request_user_list()?),
        Some(PlannedLocalRuntimeAction::SetPlaylistIndex(index)) => {
            Ok(runtime.run_set_playlist_index(index)?)
        }
        Some(PlannedLocalRuntimeAction::AdvancePlaylistIndex) => {
            Ok(runtime.run_advance_playlist_index()?)
        }
        Some(PlannedLocalRuntimeAction::QueuePlaylistItem {
            file_name,
            select_after_queue,
        }) => Ok(runtime.run_queue_playlist_item(file_name, select_after_queue)?),
        Some(PlannedLocalRuntimeAction::DeletePlaylistIndex(index)) => {
            Ok(runtime.run_delete_playlist_index(index)?)
        }
        Some(PlannedLocalRuntimeAction::UndoPlaylistChange) => {
            Ok(runtime.run_undo_playlist_change()?)
        }
        Some(PlannedLocalRuntimeAction::ShuffleRemainingPlaylist) => {
            Ok(runtime.run_shuffle_remaining_playlist()?)
        }
        Some(PlannedLocalRuntimeAction::ShuffleEntirePlaylist) => {
            Ok(runtime.run_shuffle_entire_playlist()?)
        }
        Some(PlannedLocalRuntimeAction::UndoSeek) => Ok(runtime.run_undo_seek()?),
        Some(PlannedLocalRuntimeAction::SetUserOffset(_)) => Ok(false),
        Some(PlannedLocalRuntimeAction::SeekToPosition(position_seconds)) => {
            Ok(runtime.run_seek_to_position(position_seconds)?)
        }
        Some(PlannedLocalRuntimeAction::SeekByOffset(offset_seconds)) => {
            Ok(runtime.run_seek_by_offset(offset_seconds)?)
        }
        Some(PlannedLocalRuntimeAction::TogglePause) => {
            let player_paused = runtime.player().paused();
            let player_position_seconds = runtime.player().position_seconds();
            runtime
                .session_mut()
                .apply_player_playback_telemetry_update(
                    &PlayerPlaybackTelemetryUpdate::default()
                        .with_paused(player_paused)
                        .with_position_seconds(player_position_seconds),
                );
            Ok(runtime.run_set_paused(!player_paused)?)
        }
        Some(PlannedLocalRuntimeAction::ToggleReady) => Ok(runtime.run_toggle_ready(true)?),
        Some(PlannedLocalRuntimeAction::SetUserReady { username, ready }) => {
            Ok(runtime.run_set_ready_for_user(username, ready, true)?)
        }
        Some(PlannedLocalRuntimeAction::RequestControllerAuth { room, password }) => {
            Ok(runtime.run_request_controller_auth(room, password)?)
        }
        Some(PlannedLocalRuntimeAction::SetRoomWithLegacyFallback(room)) => {
            Ok(runtime.run_set_room_with_legacy_fallback(room)?)
        }
        Some(PlannedLocalRuntimeAction::SetRoom(room)) => Ok(runtime.run_set_room(room)?),
        None => Ok(false),
    }
}
