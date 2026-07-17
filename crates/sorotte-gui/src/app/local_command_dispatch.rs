use sorotte_client_app::app_boundary::commands::{
    LocalInputCommandPlanningContext, PlannedLocalInputDispatch, PlannedLocalRuntimeAction,
    local_input_error_output_line_legacy_compatible, parse_local_input_command,
    plan_local_input_command_legacy_compatible, plan_local_input_dispatch_legacy_compatible,
    render_local_input_display_lines_legacy_compatible,
};
use sorotte_client_core::ClientSession;

use super::runtime_bridge::{GuiPlexPlaylistJobCancellationReason, GuiRuntimeRequest};
use super::shell_state::{
    GuiDraftRuntimeSnapshot, GuiShellAction, MenuActionId, SettingId, SorotteGuiShellAppState,
};
use super::support::{configured_room_name_text, joined_room_name_text, normalized_editable_text};
use super::ui_state::GuiUpdateIndicatorAction;

const LEGACY_SYNCPLAY_VERSION: &str = "1.7.5";
const PLAYLIST_EMPTY_MESSAGE_LEGACY: &str = "Playlist is currently empty.";

#[cfg(test)]
mod tests;

#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct GuiShellDispatchPlan {
    pub(super) pre_shell_runtime_requests: Vec<GuiRuntimeRequest>,
    pub(super) shell_actions: Vec<GuiShellAction>,
    pub(super) runtime_requests: Vec<GuiRuntimeRequest>,
}

impl GuiShellDispatchPlan {
    pub(super) fn from_shell_actions(
        state: &SorotteGuiShellAppState,
        actions: Vec<GuiShellAction>,
    ) -> Self {
        let mut plan = Self::default();
        let mut selected_menu_action = state.selection.selected_menu_action;
        let mut selected_plex_playlist_search_result = state
            .plex_playlist_search
            .as_ref()
            .and_then(|search| search.selected_index);
        for action in actions {
            match action {
                GuiShellAction::BeginLocalChatSend(message) => {
                    plan.extend(plan_chat_submit(state, message));
                }
                GuiShellAction::BeginUpdateCheck { user_initiated } => {
                    plan.shell_actions
                        .push(GuiShellAction::BeginUpdateCheck { user_initiated });
                    push_update_check_request(&mut plan, state, user_initiated);
                }
                GuiShellAction::ActivateUpdateIndicator => {
                    plan.shell_actions
                        .push(GuiShellAction::ActivateUpdateIndicator);
                    match state.update_check.update_indicator_activation_action() {
                        Some(GuiUpdateIndicatorAction::Check) => {
                            plan.shell_actions.push(GuiShellAction::BeginUpdateCheck {
                                user_initiated: true,
                            });
                            push_update_check_request(&mut plan, state, true);
                        }
                        Some(GuiUpdateIndicatorAction::InstallAvailable) => {
                            plan.shell_actions.push(GuiShellAction::BeginUpdateInstall);
                            if let Some(candidate) = state.update_check.candidate.clone() {
                                plan.runtime_requests
                                    .push(GuiRuntimeRequest::DownloadAndInstallUpdate(candidate));
                            }
                        }
                        Some(GuiUpdateIndicatorAction::ApplyStaged) => {
                            plan.shell_actions
                                .push(GuiShellAction::BeginStagedUpdateApply);
                            if let Some(staged_update) = state.update_check.staged_update.clone() {
                                plan.runtime_requests
                                    .push(GuiRuntimeRequest::ApplyStagedUpdate(staged_update));
                            }
                        }
                        None => {}
                    }
                }
                GuiShellAction::SelectMenuAction {
                    section_index,
                    action_index,
                } => {
                    selected_menu_action = Some((section_index, action_index));
                    plan.shell_actions.push(GuiShellAction::SelectMenuAction {
                        section_index,
                        action_index,
                    });
                }
                GuiShellAction::TriggerSelectedMenuAction => {
                    if let Some(action_id) =
                        selected_menu_action.and_then(|(section_index, action_index)| {
                            state.menus.action_id_at(section_index, action_index)
                        })
                    {
                        plan.shell_actions
                            .push(GuiShellAction::InvokeMenuAction(action_id));
                        if menu_action_starts_update_check(state, action_id) {
                            push_update_check_request(&mut plan, state, true);
                        }
                    } else {
                        plan.shell_actions
                            .push(GuiShellAction::TriggerSelectedMenuAction);
                    }
                }
                GuiShellAction::InvokeMenuAction(action_id) => {
                    plan.shell_actions
                        .push(GuiShellAction::InvokeMenuAction(action_id));
                    if menu_action_starts_update_check(state, action_id) {
                        push_update_check_request(&mut plan, state, true);
                    }
                }
                GuiShellAction::BeginUpdateDownload => {
                    plan.shell_actions.push(GuiShellAction::BeginUpdateDownload);
                    if let Some(candidate) = state.update_check.candidate.clone() {
                        plan.runtime_requests
                            .push(GuiRuntimeRequest::DownloadUpdate(candidate));
                    }
                }
                GuiShellAction::BeginUpdateInstall => {
                    plan.shell_actions.push(GuiShellAction::BeginUpdateInstall);
                    if state.update_check.self_update_supported
                        && let Some(candidate) = state.update_check.candidate.clone()
                    {
                        plan.runtime_requests
                            .push(GuiRuntimeRequest::DownloadAndInstallUpdate(candidate));
                    }
                }
                GuiShellAction::BeginStagedUpdateApply => {
                    plan.shell_actions
                        .push(GuiShellAction::BeginStagedUpdateApply);
                    if let Some(staged_update) = state.update_check.staged_update.clone() {
                        plan.runtime_requests
                            .push(GuiRuntimeRequest::ApplyStagedUpdate(staged_update));
                    }
                }
                GuiShellAction::RetryPlayerLaunch => {
                    plan.runtime_requests
                        .push(GuiRuntimeRequest::RetryPlayerLaunch);
                }
                GuiShellAction::RetryChatOsdIntegration => {
                    plan.runtime_requests
                        .push(GuiRuntimeRequest::RetryChatOsdIntegration);
                }
                GuiShellAction::RequestSeekPreparationKeepWaiting => {
                    if state
                        .seek_preparation
                        .as_ref()
                        .is_some_and(|preparation| preparation.can_keep_waiting)
                    {
                        plan.runtime_requests
                            .push(GuiRuntimeRequest::KeepWaitingForSeekPreparation);
                    }
                }
                GuiShellAction::RequestSeekPreparationCancel => {
                    if state
                        .seek_preparation
                        .as_ref()
                        .is_some_and(|preparation| preparation.can_cancel_and_remain)
                    {
                        plan.runtime_requests
                            .push(GuiRuntimeRequest::CancelSeekPreparation);
                    }
                }
                GuiShellAction::RequestSeekPreparationJoinNearest => {
                    if state.seek_preparation.as_ref().is_some_and(|preparation| {
                        preparation.can_join_nearest_buffered
                            && preparation.nearest_safe_buffered_position_seconds.is_some()
                    }) {
                        plan.runtime_requests
                            .push(GuiRuntimeRequest::JoinNearestBufferedSeekPreparation);
                    }
                }
                GuiShellAction::SetPluginEnabled { plugin, enabled } => {
                    plan.pre_shell_runtime_requests
                        .push(GuiRuntimeRequest::SetPluginEnabled { plugin, enabled });
                }
                GuiShellAction::InstallStreamHelper => {
                    plan.runtime_requests
                        .push(GuiRuntimeRequest::InstallStreamHelper);
                }
                GuiShellAction::IntegrateStreamHelperDownloader(path) => {
                    plan.runtime_requests
                        .push(GuiRuntimeRequest::IntegrateStreamHelperDownloader(path));
                }
                GuiShellAction::IntegrateStreamHelperJsRuntime(path) => {
                    plan.runtime_requests
                        .push(GuiRuntimeRequest::IntegrateStreamHelperJsRuntime(path));
                }
                GuiShellAction::RecheckStreamHelper => {
                    plan.runtime_requests
                        .push(GuiRuntimeRequest::RecheckStreamHelper);
                }
                GuiShellAction::OpenStreamHelperInstallLocation => {
                    plan.runtime_requests
                        .push(GuiRuntimeRequest::OpenStreamHelperInstallLocation);
                }
                GuiShellAction::RetryPendingStreamMediaOpen => {
                    plan.runtime_requests
                        .push(GuiRuntimeRequest::RetryPendingStreamMediaOpen);
                }
                GuiShellAction::InstallMediaMatchTools => {
                    plan.runtime_requests
                        .push(GuiRuntimeRequest::InstallMediaMatchTools);
                }
                GuiShellAction::ImportMediaMatchFfmpeg(path) => {
                    plan.runtime_requests
                        .push(GuiRuntimeRequest::ImportMediaMatchFfmpeg(path));
                }
                GuiShellAction::ImportMediaMatchFfprobe(path) => {
                    plan.runtime_requests
                        .push(GuiRuntimeRequest::ImportMediaMatchFfprobe(path));
                }
                GuiShellAction::OpenMediaMatchInstallLocation => {
                    plan.runtime_requests
                        .push(GuiRuntimeRequest::OpenMediaMatchInstallLocation);
                }
                GuiShellAction::RecheckMediaMatchTools => {
                    plan.runtime_requests
                        .push(GuiRuntimeRequest::RecheckMediaMatchTools);
                }
                GuiShellAction::RebuildMediaMatchIndex => {
                    plan.runtime_requests
                        .push(GuiRuntimeRequest::RebuildMediaMatchIndex);
                }
                GuiShellAction::CancelMediaMatchRebuild => {
                    plan.runtime_requests
                        .push(GuiRuntimeRequest::CancelMediaMatchRebuild);
                }
                GuiShellAction::ClearMediaMatchCache => {
                    plan.runtime_requests
                        .push(GuiRuntimeRequest::ClearMediaMatchCache);
                }
                GuiShellAction::SetMediaMatchFingerprintingEnabled(enabled) => {
                    plan.pre_shell_runtime_requests.push(
                        GuiRuntimeRequest::SetMediaMatchFingerprintingEnabled(enabled),
                    );
                }
                GuiShellAction::SetMediaMatchBackgroundWarmupEnabled(enabled) => {
                    plan.pre_shell_runtime_requests.push(
                        GuiRuntimeRequest::SetMediaMatchBackgroundWarmupEnabled(enabled),
                    );
                }
                GuiShellAction::SetMediaMatchWireSharingEnabled(enabled) => {
                    plan.pre_shell_runtime_requests
                        .push(GuiRuntimeRequest::SetMediaMatchWireSharingEnabled(enabled));
                }
                GuiShellAction::SetMediaMatchRuntimeToleranceEnabled(enabled) => {
                    plan.pre_shell_runtime_requests.push(
                        GuiRuntimeRequest::SetMediaMatchRuntimeToleranceEnabled(enabled),
                    );
                }
                GuiShellAction::SetMediaMatchAutoplayPolicy(policy) => {
                    plan.pre_shell_runtime_requests
                        .push(GuiRuntimeRequest::SetMediaMatchAutoplayPolicy(policy));
                }
                GuiShellAction::StartPlexAuth => {
                    plan.runtime_requests.push(GuiRuntimeRequest::StartPlexAuth);
                }
                GuiShellAction::PollPlexAuth => {
                    plan.runtime_requests.push(GuiRuntimeRequest::PollPlexAuth);
                }
                GuiShellAction::RefreshPlexServers => {
                    plan.runtime_requests
                        .push(GuiRuntimeRequest::RefreshPlexServers);
                }
                GuiShellAction::SelectPlexServer {
                    machine_identifier,
                    uri,
                } => {
                    plan.pre_shell_runtime_requests
                        .push(GuiRuntimeRequest::SelectPlexServer {
                            machine_identifier,
                            uri,
                        });
                }
                GuiShellAction::TogglePlexSync(enabled) => {
                    plan.pre_shell_runtime_requests
                        .push(GuiRuntimeRequest::TogglePlexSync(enabled));
                }
                GuiShellAction::TogglePlexStreaming(enabled) => {
                    plan.pre_shell_runtime_requests
                        .push(GuiRuntimeRequest::TogglePlexStreaming(enabled));
                }
                GuiShellAction::DisconnectPlex => {
                    plan.pre_shell_runtime_requests
                        .push(GuiRuntimeRequest::DisconnectPlex);
                }
                GuiShellAction::SubmitPlexPlaylistSearch { query } => {
                    plan.shell_actions
                        .push(GuiShellAction::SubmitPlexPlaylistSearch {
                            query: query.clone(),
                        });
                    plan.pre_shell_runtime_requests
                        .push(GuiRuntimeRequest::SearchSelectedPlexServerMedia { query });
                }
                GuiShellAction::SelectPlexPlaylistSearchResult(index) => {
                    selected_plex_playlist_search_result = Some(index);
                    plan.shell_actions
                        .push(GuiShellAction::SelectPlexPlaylistSearchResult(index));
                }
                GuiShellAction::AddSelectedPlexPlaylistSearchResult => {
                    plan.shell_actions
                        .push(GuiShellAction::AddSelectedPlexPlaylistSearchResult);
                    if let Some(rating_key) = selected_plex_playlist_search_result
                        .and_then(|index| state.plex_playlist_search.as_ref()?.results.get(index))
                        .map(|result| result.rating_key.clone())
                    {
                        plan.pre_shell_runtime_requests
                            .push(GuiRuntimeRequest::ResolvePlexPlaylistItem { rating_key });
                    }
                }
                GuiShellAction::CancelPlexPlaylistSearch => {
                    plan.shell_actions
                        .push(GuiShellAction::CancelPlexPlaylistSearch);
                    plan.pre_shell_runtime_requests.push(
                        GuiRuntimeRequest::CancelPlexPlaylistJobs {
                            reason: GuiPlexPlaylistJobCancellationReason::PickerClosed,
                        },
                    );
                }
                GuiShellAction::SelectMainWindowPlaylistSource { index, provider_id } => {
                    let enabled = state
                        .main_window
                        .playlist
                        .get(index)
                        .and_then(|row| {
                            row.source_state
                                .options
                                .iter()
                                .find(|option| option.provider_id == provider_id && option.enabled)
                        })
                        .is_some();
                    plan.shell_actions
                        .push(GuiShellAction::SelectMainWindowPlaylistSource {
                            index,
                            provider_id: provider_id.clone(),
                        });
                    if enabled {
                        plan.runtime_requests
                            .push(GuiRuntimeRequest::ResolvePlaylistSource { index, provider_id });
                    }
                }
                GuiShellAction::RequestSharedPlaylistFileImport { path, shuffled } => {
                    plan.shell_actions
                        .push(GuiShellAction::RequestSharedPlaylistFileImport {
                            path: path.clone(),
                            shuffled,
                        });
                    plan.runtime_requests
                        .push(GuiRuntimeRequest::ImportSharedPlaylistFile { path, shuffled });
                }
                GuiShellAction::RequestSharedPlaylistMediaFilesAdd {
                    paths,
                    playlist_insert_slot,
                } => {
                    plan.shell_actions
                        .push(GuiShellAction::RequestSharedPlaylistMediaFilesAdd {
                            paths: paths.clone(),
                            playlist_insert_slot,
                        });
                    plan.runtime_requests
                        .push(GuiRuntimeRequest::OpenMediaFiles {
                            paths,
                            load_into_shared_playlist: true,
                            playlist_insert_slot: Some(playlist_insert_slot),
                        });
                }
                other => plan.shell_actions.push(other),
            }
        }
        plan
    }

    fn extend(&mut self, other: Self) {
        self.pre_shell_runtime_requests
            .extend(other.pre_shell_runtime_requests);
        self.shell_actions.extend(other.shell_actions);
        self.runtime_requests.extend(other.runtime_requests);
    }
}

fn push_update_check_request(
    plan: &mut GuiShellDispatchPlan,
    state: &SorotteGuiShellAppState,
    user_initiated: bool,
) {
    plan.runtime_requests
        .push(GuiRuntimeRequest::CheckForUpdates {
            language: state.update_check_language(),
            update_channel: state.update_check_channel(),
            user_initiated,
        });
}

fn menu_action_starts_update_check(
    state: &SorotteGuiShellAppState,
    action_id: MenuActionId,
) -> bool {
    action_id == MenuActionId::CheckForUpdates
        && state
            .menus
            .action(action_id)
            .is_some_and(|action| action.enabled)
}

fn plan_chat_submit(state: &SorotteGuiShellAppState, message: String) -> GuiShellDispatchPlan {
    if message == "/" {
        return plan_direct_chat_send(message);
    }

    if let Some(literal_chat) = message.strip_prefix("//") {
        return plan_direct_chat_send(format!("/{literal_chat}"));
    }

    let Some(command_text) = message.strip_prefix('/') else {
        return plan_direct_chat_send(message);
    };
    let command_text = command_text.to_owned();

    let mut plan = GuiShellDispatchPlan {
        pre_shell_runtime_requests: Vec::new(),
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

fn plan_direct_chat_send(message: String) -> GuiShellDispatchPlan {
    let Some(message) = normalized_editable_text(&message) else {
        return GuiShellDispatchPlan {
            pre_shell_runtime_requests: Vec::new(),
            shell_actions: vec![GuiShellAction::BeginLocalChatSend(message)],
            runtime_requests: Vec::new(),
        };
    };

    GuiShellDispatchPlan {
        pre_shell_runtime_requests: Vec::new(),
        shell_actions: vec![clear_chat_draft_action()],
        runtime_requests: vec![GuiRuntimeRequest::SendChatMessage(message)],
    }
}

fn extend_plan_for_dispatch(
    plan: &mut GuiShellDispatchPlan,
    state: &SorotteGuiShellAppState,
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
    state: &SorotteGuiShellAppState,
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
        PlannedLocalRuntimeAction::KeepWaitingForSeekPreparation => {
            if state
                .seek_preparation
                .as_ref()
                .is_some_and(|preparation| preparation.can_keep_waiting)
            {
                plan.runtime_requests
                    .push(GuiRuntimeRequest::KeepWaitingForSeekPreparation);
            }
        }
        PlannedLocalRuntimeAction::JoinNearestBufferedSeekPreparation => {
            if state.seek_preparation.as_ref().is_some_and(|preparation| {
                preparation.can_join_nearest_buffered
                    && preparation.nearest_safe_buffered_position_seconds.is_some()
            }) {
                plan.runtime_requests
                    .push(GuiRuntimeRequest::JoinNearestBufferedSeekPreparation);
            }
        }
        PlannedLocalRuntimeAction::CancelSeekPreparation => {
            if state
                .seek_preparation
                .as_ref()
                .is_some_and(|preparation| preparation.can_cancel_and_remain)
            {
                plan.runtime_requests
                    .push(GuiRuntimeRequest::CancelSeekPreparation);
            }
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
        PlannedLocalRuntimeAction::Play => {
            plan.runtime_requests
                .push(GuiRuntimeRequest::SetPlaybackPaused(false));
        }
        PlannedLocalRuntimeAction::Pause => {
            plan.runtime_requests
                .push(GuiRuntimeRequest::SetPlaybackPaused(true));
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
            sorotte_client_app::app_boundary::commands::LocalInputCommandErrorKind::PlaylistInvalidIndex,
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

fn local_user_ready_state(state: &SorotteGuiShellAppState) -> bool {
    state.displayed_local_main_window_user_ready()
}

fn validated_playlist_index(state: &SorotteGuiShellAppState, index: i64) -> Option<usize> {
    let Ok(index) = usize::try_from(index) else {
        return None;
    };
    (index < state.main_window.playlist.len()).then_some(index)
}

fn render_playlist_lines(state: &SorotteGuiShellAppState) -> Vec<String> {
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

fn render_user_list_lines(state: &SorotteGuiShellAppState) -> Vec<String> {
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

fn current_room_for_local_commands(state: &SorotteGuiShellAppState) -> Option<String> {
    joined_room_name_text(&state.main_window.room_name).map(str::to_owned)
}

fn configured_room_for_local_commands(state: &SorotteGuiShellAppState) -> String {
    configured_room_name_text(
        state
            .configuration
            .control_value(SettingId::ConnectionRoom)
            .unwrap_or_default(),
    )
    .unwrap_or_default()
}
