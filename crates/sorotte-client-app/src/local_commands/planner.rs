use sorotte_client_core::ClientSession;

use super::controlled_rooms::{controlled_room_base_name, generate_room_password};
use super::display::{local_input_error_output_line, localized_current_offset_message};
use super::playlist::playlist_index_in_bounds;
use super::types::{
    LocalInputCommand, LocalInputCommandErrorKind, LocalInputCommandPlanningContext,
    LocalOffsetCommand, PlannedLocalInputDispatch, PlannedLocalRuntimeAction,
    PlannedLocalRuntimeDispatch,
};

impl LocalInputCommand {
    fn uses_shared_playlists(&self) -> bool {
        matches!(
            self,
            Self::ShowPlaylist
                | Self::SelectPlaylistIndex(_)
                | Self::NextPlaylistItem
                | Self::QueuePlaylistItem { .. }
                | Self::DeletePlaylistIndex(_)
                | Self::UndoPlaylistChange
                | Self::ShuffleRemainingPlaylist
                | Self::ShuffleEntirePlaylist
        )
    }
}

pub fn plan_local_input_dispatch(
    command: LocalInputCommand,
    context: &LocalInputCommandPlanningContext<'_>,
    shared_playlists_enabled: bool,
) -> PlannedLocalInputDispatch {
    if !shared_playlists_enabled && command.uses_shared_playlists() {
        return PlannedLocalInputDispatch::Suppressed;
    }

    match command {
        LocalInputCommand::Chat(chat_message) => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::SendChat(chat_message))
        }
        LocalInputCommand::RequestUserList => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::RequestUserList)
        }
        LocalInputCommand::ShowUnknownCommandHelp => {
            PlannedLocalInputDispatch::EmitUnknownCommandHelp
        }
        LocalInputCommand::ShowHelp => PlannedLocalInputDispatch::EmitHelp,
        LocalInputCommand::ShowPlaylistInvalidIndexError => {
            PlannedLocalInputDispatch::EmitError(LocalInputCommandErrorKind::PlaylistInvalidIndex)
        }
        LocalInputCommand::ShowQueueMissingFileError => {
            PlannedLocalInputDispatch::EmitError(LocalInputCommandErrorKind::QueueMissingFile)
        }
        LocalInputCommand::ShowPlaylist => PlannedLocalInputDispatch::EmitPlaylist,
        LocalInputCommand::SelectPlaylistIndex(index) => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::SetPlaylistIndex(index))
        }
        LocalInputCommand::NextPlaylistItem => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::AdvancePlaylistIndex)
        }
        LocalInputCommand::QueuePlaylistItem {
            file_name,
            select_after_queue,
        } => PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::QueuePlaylistItem {
            file_name,
            select_after_queue,
        }),
        LocalInputCommand::DeletePlaylistIndex(index) => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::DeletePlaylistIndex(index))
        }
        LocalInputCommand::UndoPlaylistChange => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::UndoPlaylistChange)
        }
        LocalInputCommand::ShuffleRemainingPlaylist => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::ShuffleRemainingPlaylist)
        }
        LocalInputCommand::ShuffleEntirePlaylist => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::ShuffleEntirePlaylist)
        }
        LocalInputCommand::UndoSeek => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::UndoSeek)
        }
        LocalInputCommand::KeepWaitingForSeekPreparation => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::KeepWaitingForSeekPreparation)
        }
        LocalInputCommand::JoinNearestBufferedSeekPreparation => PlannedLocalInputDispatch::Run(
            PlannedLocalRuntimeAction::JoinNearestBufferedSeekPreparation,
        ),
        LocalInputCommand::CancelSeekPreparation => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::CancelSeekPreparation)
        }
        LocalInputCommand::SetUserOffset(command) => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::SetUserOffset(command))
        }
        LocalInputCommand::SeekAbsolute(position_seconds) => PlannedLocalInputDispatch::Run(
            PlannedLocalRuntimeAction::SeekToPosition(position_seconds),
        ),
        LocalInputCommand::SeekRelative(offset_seconds) => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::SeekByOffset(offset_seconds))
        }
        LocalInputCommand::Play => PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::Play),
        LocalInputCommand::Pause => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::Pause)
        }
        LocalInputCommand::TogglePause => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::TogglePause)
        }
        LocalInputCommand::ToggleReady => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::ToggleReady)
        }
        LocalInputCommand::SetUserReady { username, ready } => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::SetUserReady {
                username,
                ready,
            })
        }
        LocalInputCommand::CreateControlledRoom(room_name) => {
            let room = room_name.unwrap_or_else(|| {
                context
                    .current_room
                    .unwrap_or(context.configured_room)
                    .to_owned()
            });
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::RequestControllerAuth {
                room: controlled_room_base_name(&room),
                password: generate_room_password().into(),
            })
        }
        LocalInputCommand::AuthController(password) => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::RequestControllerAuth {
                room: context
                    .current_room
                    .unwrap_or(context.configured_room)
                    .to_owned(),
                password,
            })
        }
        LocalInputCommand::SetRoomWithDefaultFallback => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::SetRoomWithDefaultFallback(
                context.configured_room.to_owned(),
            ))
        }
        LocalInputCommand::SetRoom(room) => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::SetRoom(room))
        }
    }
}

pub fn resolved_local_user_offset_seconds(
    current_user_offset_seconds: f64,
    global_position_seconds: f64,
    command: &LocalOffsetCommand,
) -> f64 {
    let current_local_position = global_position_seconds + current_user_offset_seconds;
    match command {
        LocalOffsetCommand::Absolute(offset_seconds) => *offset_seconds,
        LocalOffsetCommand::Relative(offset_delta_seconds) => {
            current_user_offset_seconds + offset_delta_seconds
        }
        LocalOffsetCommand::RelativeFromCurrentPositionMinus(offset_seconds) => {
            current_local_position - offset_seconds
        }
    }
}

pub fn plan_local_offset_runtime_dispatch(
    current_user_offset_seconds: f64,
    global_position_seconds: f64,
    command: &LocalOffsetCommand,
    language: Option<&str>,
) -> PlannedLocalRuntimeDispatch {
    let updated_user_offset_seconds = resolved_local_user_offset_seconds(
        current_user_offset_seconds,
        global_position_seconds,
        command,
    );
    PlannedLocalRuntimeDispatch {
        line_to_emit: Some(localized_current_offset_message(
            updated_user_offset_seconds,
            language,
        )),
        action: Some(PlannedLocalRuntimeAction::SeekToPosition(
            global_position_seconds + updated_user_offset_seconds,
        )),
        updated_user_offset_seconds: Some(updated_user_offset_seconds),
    }
}

fn plan_local_playlist_runtime_dispatch(
    session: &ClientSession,
    index: i64,
    language: Option<&str>,
    action: PlannedLocalRuntimeAction,
) -> PlannedLocalRuntimeDispatch {
    if !playlist_index_in_bounds(session, index) {
        return PlannedLocalRuntimeDispatch {
            line_to_emit: Some(local_input_error_output_line(
                LocalInputCommandErrorKind::PlaylistInvalidIndex,
                language,
            )),
            action: None,
            updated_user_offset_seconds: None,
        };
    }

    PlannedLocalRuntimeDispatch {
        line_to_emit: None,
        action: Some(action),
        updated_user_offset_seconds: None,
    }
}

pub fn plan_local_playlist_select_runtime_dispatch(
    session: &ClientSession,
    index: i64,
    language: Option<&str>,
) -> PlannedLocalRuntimeDispatch {
    plan_local_playlist_runtime_dispatch(
        session,
        index,
        language,
        PlannedLocalRuntimeAction::SetPlaylistIndex(index),
    )
}

pub fn plan_local_playlist_delete_runtime_dispatch(
    session: &ClientSession,
    index: i64,
    language: Option<&str>,
) -> PlannedLocalRuntimeDispatch {
    plan_local_playlist_runtime_dispatch(
        session,
        index,
        language,
        PlannedLocalRuntimeAction::DeletePlaylistIndex(index),
    )
}

pub fn plan_local_runtime_dispatch(
    session: &ClientSession,
    current_user_offset_seconds: f64,
    action: PlannedLocalRuntimeAction,
    language: Option<&str>,
) -> PlannedLocalRuntimeDispatch {
    match action {
        PlannedLocalRuntimeAction::SetUserOffset(command) => {
            let global_position_seconds = session
                .current_room_playstate()
                .and_then(|playstate| playstate.position)
                .unwrap_or(0.0);
            plan_local_offset_runtime_dispatch(
                current_user_offset_seconds,
                global_position_seconds,
                &command,
                language,
            )
        }
        PlannedLocalRuntimeAction::SetPlaylistIndex(index) => {
            plan_local_playlist_select_runtime_dispatch(session, index, language)
        }
        PlannedLocalRuntimeAction::DeletePlaylistIndex(index) => {
            plan_local_playlist_delete_runtime_dispatch(session, index, language)
        }
        action => PlannedLocalRuntimeDispatch {
            line_to_emit: None,
            action: Some(action),
            updated_user_offset_seconds: None,
        },
    }
}
