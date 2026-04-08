use syncplay_client_app::app_boundary::commands::{
    LocalInputCommandPlanningContext, PlannedLocalInputDispatch, PlannedLocalRuntimeAction,
    local_input_error_output_line_legacy_compatible, parse_local_input_command,
    plan_local_input_command_legacy_compatible, plan_local_input_dispatch_legacy_compatible,
    render_local_input_display_lines_legacy_compatible,
};
use syncplay_client_core::ClientSession;

use super::runtime_bridge::GuiRuntimeRequest;
use super::shell_state::{GuiDraftRuntimeSnapshot, GuiShellAction, SyncplayGuiShellAppState};
use super::support::{configured_room_name_text, joined_room_name_text, normalized_editable_text};

const LEGACY_SYNCPLAY_VERSION: &str = "1.7.5";
const PLAYLIST_EMPTY_MESSAGE_LEGACY: &str = "Playlist is currently empty.";

#[cfg(test)]
#[path = "app_local_command_dispatch/tests.rs"]
mod tests;

#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct GuiShellDispatchPlan {
    pub(super) shell_actions: Vec<GuiShellAction>,
    pub(super) runtime_requests: Vec<GuiRuntimeRequest>,
}

impl GuiShellDispatchPlan {
    pub(super) fn from_shell_actions(
        state: &SyncplayGuiShellAppState,
        actions: Vec<GuiShellAction>,
    ) -> Self {
        let mut plan = Self::default();
        for action in actions {
            match action {
                GuiShellAction::BeginLocalChatSend(message) => {
                    plan.extend(plan_chat_submit(state, message));
                }
                other => plan.shell_actions.push(other),
            }
        }
        plan
    }

    fn extend(&mut self, other: Self) {
        self.shell_actions.extend(other.shell_actions);
        self.runtime_requests.extend(other.runtime_requests);
    }
}

fn plan_chat_submit(state: &SyncplayGuiShellAppState, message: String) -> GuiShellDispatchPlan {
    if message == "/" {
        return GuiShellDispatchPlan {
            shell_actions: vec![GuiShellAction::BeginLocalChatSend(message)],
            runtime_requests: Vec::new(),
        };
    }

    if let Some(literal_chat) = message.strip_prefix("//") {
        return GuiShellDispatchPlan {
            shell_actions: vec![GuiShellAction::BeginLocalChatSend(format!(
                "/{literal_chat}"
            ))],
            runtime_requests: Vec::new(),
        };
    }

    let Some(command_text) = message.strip_prefix('/') else {
        return GuiShellDispatchPlan {
            shell_actions: vec![GuiShellAction::BeginLocalChatSend(message)],
            runtime_requests: Vec::new(),
        };
    };
    let command_text = command_text.to_owned();

    let mut plan = GuiShellDispatchPlan {
        shell_actions: vec![
            clear_chat_draft_action(),
            GuiShellAction::AnnounceSystemChatEvent(message),
        ],
        runtime_requests: Vec::new(),
    };

    let Some(command) = parse_local_input_command(&command_text) else {
        return plan;
    };

    let current_room = current_room_for_local_commands(state);
    let configured_room = configured_room_for_local_commands(state);
    let planning_context = LocalInputCommandPlanningContext {
        current_room: current_room.as_deref(),
        configured_room: &configured_room,
    };
    let planned_command = plan_local_input_command_legacy_compatible(command, &planning_context);
    let dispatch = plan_local_input_dispatch_legacy_compatible(
        planned_command,
        state.shared_playlist_events_enabled(),
    );
    extend_plan_for_dispatch(&mut plan, state, dispatch);
    plan
}

fn extend_plan_for_dispatch(
    plan: &mut GuiShellDispatchPlan,
    state: &SyncplayGuiShellAppState,
    dispatch: PlannedLocalInputDispatch,
) {
    match dispatch {
        PlannedLocalInputDispatch::Suppressed => {}
        PlannedLocalInputDispatch::EmitPlaylist => {
            append_system_chat_lines(plan, render_playlist_lines(state));
        }
        PlannedLocalInputDispatch::EmitHelp
        | PlannedLocalInputDispatch::EmitUnknownCommandHelp
        | PlannedLocalInputDispatch::EmitError(_) => append_system_chat_lines(
            plan,
            render_local_input_display_lines_legacy_compatible(
                &dispatch,
                &ClientSession::default(),
                None,
                LEGACY_SYNCPLAY_VERSION,
            )
            .unwrap_or_default(),
        ),
        PlannedLocalInputDispatch::Run(action) => {
            extend_plan_for_runtime_action(plan, state, action)
        }
    }
}

fn extend_plan_for_runtime_action(
    plan: &mut GuiShellDispatchPlan,
    state: &SyncplayGuiShellAppState,
    action: PlannedLocalRuntimeAction,
) {
    match action {
        PlannedLocalRuntimeAction::SendChat(message) => {
            if normalized_editable_text(&message).is_none() {
                return;
            }
            plan.runtime_requests
                .push(GuiRuntimeRequest::SendChatMessage(message));
        }
        PlannedLocalRuntimeAction::RequestUserList => {
            append_system_chat_lines(plan, render_user_list_lines(state));
        }
        PlannedLocalRuntimeAction::SetPlaylistIndex(index) => {
            if let Some(index) = validated_playlist_index(state, index) {
                plan.runtime_requests
                    .push(GuiRuntimeRequest::SetPlaylistIndex(index));
            } else {
                append_playlist_index_error(plan);
            }
        }
        PlannedLocalRuntimeAction::AdvancePlaylistIndex => {
            plan.runtime_requests
                .push(GuiRuntimeRequest::AdvancePlaylistIndex);
        }
        PlannedLocalRuntimeAction::QueuePlaylistItem {
            file_name,
            select_after_queue,
        } => {
            plan.runtime_requests
                .push(GuiRuntimeRequest::QueuePlaylistEntry {
                    entry: file_name,
                    select_after_queue,
                });
        }
        PlannedLocalRuntimeAction::DeletePlaylistIndex(index) => {
            if let Some(index) = validated_playlist_index(state, index) {
                plan.runtime_requests
                    .push(GuiRuntimeRequest::DeletePlaylistIndex(index));
            } else {
                append_playlist_index_error(plan);
            }
        }
        PlannedLocalRuntimeAction::UndoPlaylistChange => {
            plan.runtime_requests
                .push(GuiRuntimeRequest::UndoPlaylistChange);
        }
        PlannedLocalRuntimeAction::ShuffleRemainingPlaylist => {
            plan.runtime_requests
                .push(GuiRuntimeRequest::ShuffleRemainingPlaylist);
        }
        PlannedLocalRuntimeAction::ShuffleEntirePlaylist => {
            plan.runtime_requests
                .push(GuiRuntimeRequest::ShuffleEntirePlaylist);
        }
        PlannedLocalRuntimeAction::UndoSeek => {
            plan.runtime_requests.push(GuiRuntimeRequest::UndoSeek);
        }
        PlannedLocalRuntimeAction::SetUserOffset(command) => {
            plan.runtime_requests
                .push(GuiRuntimeRequest::SetOffset(command));
        }
        PlannedLocalRuntimeAction::SeekToPosition(position_seconds) => {
            plan.runtime_requests
                .push(GuiRuntimeRequest::SeekToPosition(position_seconds));
        }
        PlannedLocalRuntimeAction::SeekByOffset(offset_seconds) => {
            plan.runtime_requests
                .push(GuiRuntimeRequest::SeekOffset(offset_seconds));
        }
        PlannedLocalRuntimeAction::TogglePause => {
            plan.runtime_requests
                .push(GuiRuntimeRequest::TogglePlaybackPause);
        }
        PlannedLocalRuntimeAction::ToggleReady => {
            let target_ready = !local_user_ready_state(state);
            plan.shell_actions
                .push(local_ready_shell_action(target_ready));
            plan.runtime_requests
                .push(GuiRuntimeRequest::SetLocalReady(target_ready));
        }
        PlannedLocalRuntimeAction::SetUserReady { username, ready } => {
            if normalized_editable_text(&username).is_none() {
                plan.shell_actions.push(local_ready_shell_action(ready));
                plan.runtime_requests
                    .push(GuiRuntimeRequest::SetLocalReady(ready));
            } else {
                plan.runtime_requests
                    .push(GuiRuntimeRequest::SetReadyForUser { username, ready });
            }
        }
        PlannedLocalRuntimeAction::RequestControllerAuth { room, password } => {
            plan.runtime_requests
                .push(GuiRuntimeRequest::RequestControllerAuth { room, password });
        }
        PlannedLocalRuntimeAction::SetRoomWithLegacyFallback(_) => {
            plan.runtime_requests
                .push(GuiRuntimeRequest::ReturnToDefaultRoom);
        }
        PlannedLocalRuntimeAction::SetRoom(room) => {
            plan.runtime_requests.push(GuiRuntimeRequest::SetRoom(room));
        }
    }
}

fn clear_chat_draft_action() -> GuiShellAction {
    GuiShellAction::ApplyGuiDraftRuntimeSnapshot(GuiDraftRuntimeSnapshot {
        outgoing_chat_message: None,
    })
}

fn append_system_chat_lines(plan: &mut GuiShellDispatchPlan, lines: Vec<String>) {
    plan.shell_actions.extend(
        lines
            .into_iter()
            .filter_map(|line| normalized_editable_text(&line))
            .map(GuiShellAction::AnnounceSystemChatEvent),
    );
}

fn append_playlist_index_error(plan: &mut GuiShellDispatchPlan) {
    append_system_chat_lines(
        plan,
        vec![local_input_error_output_line_legacy_compatible(
            syncplay_client_app::app_boundary::commands::LocalInputCommandErrorKind::PlaylistInvalidIndex,
            None,
        )],
    );
}

fn local_ready_shell_action(ready: bool) -> GuiShellAction {
    if ready {
        GuiShellAction::AnnounceLocalUserReady
    } else {
        GuiShellAction::AnnounceLocalUserNotReady
    }
}

fn local_user_ready_state(state: &SyncplayGuiShellAppState) -> bool {
    state
        .main_window
        .users
        .iter()
        .find(|user| user.is_self)
        .map(|user| user.is_ready)
        .unwrap_or(false)
}

fn validated_playlist_index(state: &SyncplayGuiShellAppState, index: i64) -> Option<usize> {
    let Ok(index) = usize::try_from(index) else {
        return None;
    };
    (index < state.main_window.playlist.len()).then_some(index)
}

fn render_playlist_lines(state: &SyncplayGuiShellAppState) -> Vec<String> {
    if state.main_window.playlist.is_empty() {
        return vec![PLAYLIST_EMPTY_MESSAGE_LEGACY.to_owned()];
    }

    state
        .main_window
        .playlist
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let marker = if state.selection.selected_main_window_playlist == Some(index) {
                " *"
            } else {
                ""
            };
            format!("{marker}\t{}: {}", index + 1, row.label)
        })
        .collect()
}

fn render_user_list_lines(state: &SyncplayGuiShellAppState) -> Vec<String> {
    let mut ordered_rooms = state
        .main_window
        .rooms
        .iter()
        .map(|room| room.room_name.clone())
        .collect::<Vec<_>>();
    for user in &state.main_window.users {
        if !ordered_rooms.iter().any(|room| room == &user.room_name) {
            ordered_rooms.push(user.room_name.clone());
        }
    }
    if ordered_rooms.is_empty() {
        let room_name = current_room_for_local_commands(state)
            .or_else(|| {
                let configured_room = configured_room_for_local_commands(state);
                (!configured_room.is_empty()).then_some(configured_room)
            })
            .unwrap_or_else(|| "(no room joined)".to_owned());
        ordered_rooms.push(room_name);
    }

    let mut lines = Vec::new();
    for room_name in ordered_rooms {
        lines.push(format!("In room '{room_name}':"));
        let room_users = state
            .main_window
            .users
            .iter()
            .filter(|user| user.room_name == room_name)
            .collect::<Vec<_>>();
        if room_users.is_empty() {
            lines.push("  (no users)".to_owned());
            continue;
        }
        for user in room_users {
            let mut flags = String::new();
            if user.is_controller {
                flags.push_str("(Controller) ");
            }
            if user.is_ready {
                flags.push_str("(Ready) ");
            }
            let username = if user.is_self {
                format!("{}*<{}>*", flags, user.username)
            } else {
                format!("{}<{}>", flags, user.username)
            };
            if user.has_file {
                let mut details = format!("{username}: {}", user.file_name_label);
                if !user.file_duration_label.is_empty() {
                    details.push_str(&format!(" ({})", user.file_duration_label));
                }
                if !user.file_size_label.is_empty() {
                    details.push_str(&format!(" [{}]", user.file_size_label));
                }
                lines.push(details);
            } else {
                lines.push(format!("{username}: (no file played)"));
            }
        }
    }
    lines
}

fn current_room_for_local_commands(state: &SyncplayGuiShellAppState) -> Option<String> {
    joined_room_name_text(&state.main_window.room_name).map(str::to_owned)
}

fn configured_room_for_local_commands(state: &SyncplayGuiShellAppState) -> String {
    configured_room_name_text(
        state
            .configuration
            .control_value("Connection", "Room")
            .unwrap_or_default(),
    )
    .unwrap_or_default()
}
