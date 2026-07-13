use sorotte_client_core::ClientSession;

use super::controlled_rooms::{
    controlled_room_base_name_legacy_compatible, generate_room_password_legacy_compatible,
};
use super::display::{
    local_input_error_output_line_legacy_compatible,
    localized_current_offset_message_legacy_compatible,
};
use super::playlist::playlist_index_in_bounds_legacy_compatible;
use super::types::{
    LocalInputCommand, LocalInputCommandErrorKind, LocalInputCommandPlanningContext,
    LocalOffsetCommand, PlannedLocalInputCommand, PlannedLocalInputDispatch,
    PlannedLocalRuntimeAction, PlannedLocalRuntimeDispatch,
};

impl PlannedLocalInputCommand {
    pub fn uses_shared_playlists(&self) -> bool {
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

pub fn plan_local_input_dispatch_legacy_compatible(
    command: PlannedLocalInputCommand,
    shared_playlists_enabled: bool,
) -> PlannedLocalInputDispatch {
    if !shared_playlists_enabled && command.uses_shared_playlists() {
        return PlannedLocalInputDispatch::Suppressed;
    }

    match command {
        PlannedLocalInputCommand::SendChat(chat_message) => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::SendChat(chat_message))
        }
        PlannedLocalInputCommand::RequestUserList => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::RequestUserList)
        }
        PlannedLocalInputCommand::ShowUnknownCommandHelp => {
            PlannedLocalInputDispatch::EmitUnknownCommandHelp
        }
        PlannedLocalInputCommand::ShowHelp => PlannedLocalInputDispatch::EmitHelp,
        PlannedLocalInputCommand::ShowError(error_kind) => {
            PlannedLocalInputDispatch::EmitError(error_kind)
        }
        PlannedLocalInputCommand::ShowPlaylist => PlannedLocalInputDispatch::EmitPlaylist,
        PlannedLocalInputCommand::SelectPlaylistIndex(index) => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::SetPlaylistIndex(index))
        }
        PlannedLocalInputCommand::NextPlaylistItem => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::AdvancePlaylistIndex)
        }
        PlannedLocalInputCommand::QueuePlaylistItem {
            file_name,
            select_after_queue,
        } => PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::QueuePlaylistItem {
            file_name,
            select_after_queue,
        }),
        PlannedLocalInputCommand::DeletePlaylistIndex(index) => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::DeletePlaylistIndex(index))
        }
        PlannedLocalInputCommand::UndoPlaylistChange => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::UndoPlaylistChange)
        }
        PlannedLocalInputCommand::ShuffleRemainingPlaylist => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::ShuffleRemainingPlaylist)
        }
        PlannedLocalInputCommand::ShuffleEntirePlaylist => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::ShuffleEntirePlaylist)
        }
        PlannedLocalInputCommand::UndoSeek => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::UndoSeek)
        }
        PlannedLocalInputCommand::KeepWaitingForSeekPreparation => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::KeepWaitingForSeekPreparation)
        }
        PlannedLocalInputCommand::JoinNearestBufferedSeekPreparation => {
            PlannedLocalInputDispatch::Run(
                PlannedLocalRuntimeAction::JoinNearestBufferedSeekPreparation,
            )
        }
        PlannedLocalInputCommand::CancelSeekPreparation => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::CancelSeekPreparation)
        }
        PlannedLocalInputCommand::SetUserOffset(command) => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::SetUserOffset(command))
        }
        PlannedLocalInputCommand::SeekAbsolute(position_seconds) => PlannedLocalInputDispatch::Run(
            PlannedLocalRuntimeAction::SeekToPosition(position_seconds),
        ),
        PlannedLocalInputCommand::SeekRelative(offset_seconds) => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::SeekByOffset(offset_seconds))
        }
        PlannedLocalInputCommand::TogglePause => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::TogglePause)
        }
        PlannedLocalInputCommand::ToggleReady => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::ToggleReady)
        }
        PlannedLocalInputCommand::SetUserReady { username, ready } => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::SetUserReady {
                username,
                ready,
            })
        }
        PlannedLocalInputCommand::RequestControllerAuth { room, password } => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::RequestControllerAuth {
                room,
                password,
            })
        }
        PlannedLocalInputCommand::SetRoomWithLegacyFallback(room) => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::SetRoomWithLegacyFallback(
                room,
            ))
        }
        PlannedLocalInputCommand::SetRoom(room) => {
            PlannedLocalInputDispatch::Run(PlannedLocalRuntimeAction::SetRoom(room))
        }
    }
}

pub fn plan_local_input_command_legacy_compatible(
    command: LocalInputCommand,
    context: &LocalInputCommandPlanningContext<'_>,
) -> PlannedLocalInputCommand {
    match command {
        LocalInputCommand::Chat(chat_message) => PlannedLocalInputCommand::SendChat(chat_message),
        LocalInputCommand::RequestUserList => PlannedLocalInputCommand::RequestUserList,
        LocalInputCommand::ShowUnknownCommandHelp => {
            PlannedLocalInputCommand::ShowUnknownCommandHelp
        }
        LocalInputCommand::ShowHelp => PlannedLocalInputCommand::ShowHelp,
        LocalInputCommand::ShowPlaylistInvalidIndexError => {
            PlannedLocalInputCommand::ShowError(LocalInputCommandErrorKind::PlaylistInvalidIndex)
        }
        LocalInputCommand::ShowQueueMissingFileError => {
            PlannedLocalInputCommand::ShowError(LocalInputCommandErrorKind::QueueMissingFile)
        }
        LocalInputCommand::ShowPlaylist => PlannedLocalInputCommand::ShowPlaylist,
        LocalInputCommand::SelectPlaylistIndex(index) => {
            PlannedLocalInputCommand::SelectPlaylistIndex(index)
        }
        LocalInputCommand::NextPlaylistItem => PlannedLocalInputCommand::NextPlaylistItem,
        LocalInputCommand::QueuePlaylistItem {
            file_name,
            select_after_queue,
        } => PlannedLocalInputCommand::QueuePlaylistItem {
            file_name,
            select_after_queue,
        },
        LocalInputCommand::DeletePlaylistIndex(index) => {
            PlannedLocalInputCommand::DeletePlaylistIndex(index)
        }
        LocalInputCommand::UndoPlaylistChange => PlannedLocalInputCommand::UndoPlaylistChange,
        LocalInputCommand::ShuffleRemainingPlaylist => {
            PlannedLocalInputCommand::ShuffleRemainingPlaylist
        }
        LocalInputCommand::ShuffleEntirePlaylist => PlannedLocalInputCommand::ShuffleEntirePlaylist,
        LocalInputCommand::UndoSeek => PlannedLocalInputCommand::UndoSeek,
        LocalInputCommand::KeepWaitingForSeekPreparation => {
            PlannedLocalInputCommand::KeepWaitingForSeekPreparation
        }
        LocalInputCommand::JoinNearestBufferedSeekPreparation => {
            PlannedLocalInputCommand::JoinNearestBufferedSeekPreparation
        }
        LocalInputCommand::CancelSeekPreparation => PlannedLocalInputCommand::CancelSeekPreparation,
        LocalInputCommand::SetUserOffset(command) => {
            PlannedLocalInputCommand::SetUserOffset(command)
        }
        LocalInputCommand::SeekAbsolute(position_seconds) => {
            PlannedLocalInputCommand::SeekAbsolute(position_seconds)
        }
        LocalInputCommand::SeekRelative(offset_seconds) => {
            PlannedLocalInputCommand::SeekRelative(offset_seconds)
        }
        LocalInputCommand::TogglePause => PlannedLocalInputCommand::TogglePause,
        LocalInputCommand::ToggleReady => PlannedLocalInputCommand::ToggleReady,
        LocalInputCommand::SetUserReady { username, ready } => {
            PlannedLocalInputCommand::SetUserReady { username, ready }
        }
        LocalInputCommand::CreateControlledRoom(room_name) => {
            let room = room_name.unwrap_or_else(|| {
                context
                    .current_room
                    .unwrap_or(context.configured_room)
                    .to_owned()
            });
            PlannedLocalInputCommand::RequestControllerAuth {
                room: controlled_room_base_name_legacy_compatible(&room),
                password: generate_room_password_legacy_compatible().into(),
            }
        }
        LocalInputCommand::AuthController(password) => {
            PlannedLocalInputCommand::RequestControllerAuth {
                room: context
                    .current_room
                    .unwrap_or(context.configured_room)
                    .to_owned(),
                password,
            }
        }
        LocalInputCommand::SetRoomWithLegacyFallback => {
            PlannedLocalInputCommand::SetRoomWithLegacyFallback(context.configured_room.to_owned())
        }
        LocalInputCommand::SetRoom(room) => PlannedLocalInputCommand::SetRoom(room),
    }
}

pub fn resolved_local_user_offset_seconds_legacy_compatible(
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

pub fn plan_local_offset_runtime_dispatch_legacy_compatible(
    current_user_offset_seconds: f64,
    global_position_seconds: f64,
    command: &LocalOffsetCommand,
    language: Option<&str>,
) -> PlannedLocalRuntimeDispatch {
    let updated_user_offset_seconds = resolved_local_user_offset_seconds_legacy_compatible(
        current_user_offset_seconds,
        global_position_seconds,
        command,
    );
    PlannedLocalRuntimeDispatch {
        line_to_emit: Some(localized_current_offset_message_legacy_compatible(
            updated_user_offset_seconds,
            language,
        )),
        action: Some(PlannedLocalRuntimeAction::SeekToPosition(
            global_position_seconds + updated_user_offset_seconds,
        )),
        updated_user_offset_seconds: Some(updated_user_offset_seconds),
    }
}

fn plan_local_playlist_runtime_dispatch_legacy_compatible(
    session: &ClientSession,
    index: i64,
    language: Option<&str>,
    action: PlannedLocalRuntimeAction,
) -> PlannedLocalRuntimeDispatch {
    if !playlist_index_in_bounds_legacy_compatible(session, index) {
        return PlannedLocalRuntimeDispatch {
            line_to_emit: Some(local_input_error_output_line_legacy_compatible(
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

pub fn plan_local_playlist_select_runtime_dispatch_legacy_compatible(
    session: &ClientSession,
    index: i64,
    language: Option<&str>,
) -> PlannedLocalRuntimeDispatch {
    plan_local_playlist_runtime_dispatch_legacy_compatible(
        session,
        index,
        language,
        PlannedLocalRuntimeAction::SetPlaylistIndex(index),
    )
}

pub fn plan_local_playlist_delete_runtime_dispatch_legacy_compatible(
    session: &ClientSession,
    index: i64,
    language: Option<&str>,
) -> PlannedLocalRuntimeDispatch {
    plan_local_playlist_runtime_dispatch_legacy_compatible(
        session,
        index,
        language,
        PlannedLocalRuntimeAction::DeletePlaylistIndex(index),
    )
}

pub fn plan_local_runtime_dispatch_legacy_compatible(
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
            plan_local_offset_runtime_dispatch_legacy_compatible(
                current_user_offset_seconds,
                global_position_seconds,
                &command,
                language,
            )
        }
        PlannedLocalRuntimeAction::SetPlaylistIndex(index) => {
            plan_local_playlist_select_runtime_dispatch_legacy_compatible(session, index, language)
        }
        PlannedLocalRuntimeAction::DeletePlaylistIndex(index) => {
            plan_local_playlist_delete_runtime_dispatch_legacy_compatible(session, index, language)
        }
        action => PlannedLocalRuntimeDispatch {
            line_to_emit: None,
            action: Some(action),
            updated_user_offset_seconds: None,
        },
    }
}
