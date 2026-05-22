use super::*;

use super::super::remote_services::{UpdateCandidate, UpdateCandidateSource, UpdateChannel};
use crate::app::{
    GuiDraftRuntimeSnapshot, GuiRuntimeRequest, GuiShellAction, MainWindowRuntimeSnapshot,
    MainWindowRuntimeUserSnapshot, SorotteGuiShellAppState, StoredClientSettingsMvp,
};

fn runtime_ready_state() -> SorotteGuiShellAppState {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        chat_input_enabled: Some(true),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "room1".to_owned(),
            shared_playlist_enabled: true,
            users: vec![MainWindowRuntimeUserSnapshot {
                username: "alice".to_owned(),
                room_name: "room1".to_owned(),
                is_self: true,
                is_ready: false,
                ..Default::default()
            }],
            playlist: vec!["episode1.mkv".to_owned(), "episode2.mkv".to_owned()],
            can_toggle_pause: true,
            can_set_ready: true,
            can_manage_playlist: true,
            ..Default::default()
        }
    )));
    state
}

fn update_candidate() -> UpdateCandidate {
    UpdateCandidate {
        channel: UpdateChannel::Stable,
        version: "0.2.0".to_owned(),
        git_sha: Some("abcdef123456".to_owned()),
        created_at_utc: "2026-05-20T00:00:00Z".to_owned(),
        target: "windows-x86_64".to_owned(),
        package: "sorotte-gui-0.2.0-windows-x86_64.zip".to_owned(),
        sha256: "a".repeat(64),
        download_url: "https://example.invalid/sorotte-gui.zip".to_owned(),
        details_url: Some("https://example.invalid/release".to_owned()),
        source: UpdateCandidateSource::ReleaseAsset,
    }
}

fn menu_action_index(
    state: &SorotteGuiShellAppState,
    section_title: &str,
    action_label: &str,
) -> (usize, usize) {
    state
        .menus
        .sections
        .iter()
        .enumerate()
        .find_map(|(section_index, section)| {
            (section.title == section_title).then(|| {
                section
                    .actions
                    .iter()
                    .position(|action| action.label == action_label)
                    .map(|action_index| (section_index, action_index))
            })?
        })
        .expect("menu action should exist")
}

#[test]
fn gui_shell_dispatch_plan_routes_update_checks_to_runtime_owner() {
    let mut state = runtime_ready_state();
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "System",
        label: "Update Channel",
        value: "dev".to_owned(),
    }));
    let plan = GuiShellDispatchPlan::from_shell_actions(
        &state,
        vec![GuiShellAction::BeginUpdateCheck {
            user_initiated: true,
        }],
    );

    assert_eq!(
        plan.shell_actions,
        vec![GuiShellAction::BeginUpdateCheck {
            user_initiated: true,
        }]
    );
    assert_eq!(
        plan.runtime_requests,
        vec![GuiRuntimeRequest::CheckForUpdates {
            language: "en".to_owned(),
            update_channel: Some("dev".to_owned()),
            user_initiated: true,
        }]
    );
}

#[test]
fn gui_shell_dispatch_plan_routes_menu_update_checks_to_runtime_owner() {
    let state = runtime_ready_state();
    let (section_index, action_index) = menu_action_index(&state, "Help", "Check for Updates");
    let select_action = GuiShellAction::SelectMenuAction {
        section_index,
        action_index,
    };
    let plan = GuiShellDispatchPlan::from_shell_actions(
        &state,
        vec![
            select_action.clone(),
            GuiShellAction::TriggerSelectedMenuAction,
        ],
    );

    assert_eq!(
        plan.shell_actions,
        vec![select_action, GuiShellAction::TriggerSelectedMenuAction]
    );
    assert_eq!(
        plan.runtime_requests,
        vec![GuiRuntimeRequest::CheckForUpdates {
            language: "en".to_owned(),
            update_channel: None,
            user_initiated: true,
        }]
    );
}

#[test]
fn gui_shell_dispatch_plan_preserves_install_marker_update_channel_fallback() {
    let state = runtime_ready_state();
    let plan = GuiShellDispatchPlan::from_shell_actions(
        &state,
        vec![GuiShellAction::BeginUpdateCheck {
            user_initiated: true,
        }],
    );

    assert_eq!(
        plan.runtime_requests,
        vec![GuiRuntimeRequest::CheckForUpdates {
            language: "en".to_owned(),
            update_channel: None,
            user_initiated: true,
        }]
    );
}

#[test]
fn gui_shell_dispatch_plan_routes_update_downloads_to_runtime_owner() {
    let mut state = runtime_ready_state();
    let candidate = update_candidate();
    state.update_check.candidate = Some(candidate.clone());
    state.update_check.self_update_supported = true;

    let plan =
        GuiShellDispatchPlan::from_shell_actions(&state, vec![GuiShellAction::BeginUpdateDownload]);

    assert_eq!(
        plan.shell_actions,
        vec![GuiShellAction::BeginUpdateDownload]
    );
    assert_eq!(
        plan.runtime_requests,
        vec![GuiRuntimeRequest::DownloadUpdate(candidate)]
    );
}

#[test]
fn gui_shell_dispatch_plan_routes_help_commands_to_system_chat_output() {
    let state = runtime_ready_state();
    let plan = GuiShellDispatchPlan::from_shell_actions(
        &state,
        vec![
            GuiShellAction::ApplyGuiDraftRuntimeSnapshot(GuiDraftRuntimeSnapshot {
                outgoing_chat_message: Some("/help".to_owned()),
            }),
            GuiShellAction::BeginLocalChatSend("/help".to_owned()),
        ],
    );

    assert!(plan.runtime_requests.is_empty());
    assert!(
        plan.shell_actions
            .contains(&GuiShellAction::ApplyGuiDraftRuntimeSnapshot(
                GuiDraftRuntimeSnapshot {
                    outgoing_chat_message: None,
                },
            ))
    );
    assert!(
        plan.shell_actions
            .contains(&GuiShellAction::AnnounceSystemChatEvent("/help".to_owned()))
    );
    assert!(
        plan.shell_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::AnnounceSystemChatEvent(message)
                if message.contains("room")
        )),
        "help output should include command help lines"
    );
    assert!(
        !plan
            .shell_actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::BeginLocalChatSend(_)))
    );
}

#[test]
fn gui_shell_dispatch_plan_preserves_literal_double_slash_chat() {
    let state = runtime_ready_state();
    let plan = GuiShellDispatchPlan::from_shell_actions(
        &state,
        vec![
            GuiShellAction::ApplyGuiDraftRuntimeSnapshot(GuiDraftRuntimeSnapshot {
                outgoing_chat_message: Some("//literal".to_owned()),
            }),
            GuiShellAction::BeginLocalChatSend("//literal".to_owned()),
        ],
    );

    assert_eq!(
        plan.runtime_requests,
        vec![GuiRuntimeRequest::SendChatMessage("/literal".to_owned())]
    );
    assert_eq!(
        plan.shell_actions,
        vec![
            GuiShellAction::ApplyGuiDraftRuntimeSnapshot(GuiDraftRuntimeSnapshot {
                outgoing_chat_message: Some("//literal".to_owned()),
            }),
            GuiShellAction::ApplyGuiDraftRuntimeSnapshot(GuiDraftRuntimeSnapshot {
                outgoing_chat_message: None,
            }),
        ]
    );
}

#[test]
fn gui_shell_dispatch_plan_routes_plain_chat_to_nonblocking_runtime_send() {
    let state = runtime_ready_state();
    let plan = GuiShellDispatchPlan::from_shell_actions(
        &state,
        vec![
            GuiShellAction::ApplyGuiDraftRuntimeSnapshot(GuiDraftRuntimeSnapshot {
                outgoing_chat_message: Some("hello room".to_owned()),
            }),
            GuiShellAction::BeginLocalChatSend("hello room".to_owned()),
        ],
    );

    assert_eq!(
        plan.runtime_requests,
        vec![GuiRuntimeRequest::SendChatMessage("hello room".to_owned())]
    );
    assert_eq!(
        plan.shell_actions,
        vec![
            GuiShellAction::ApplyGuiDraftRuntimeSnapshot(GuiDraftRuntimeSnapshot {
                outgoing_chat_message: Some("hello room".to_owned()),
            }),
            GuiShellAction::ApplyGuiDraftRuntimeSnapshot(GuiDraftRuntimeSnapshot {
                outgoing_chat_message: None,
            }),
        ]
    );
    assert!(
        !plan
            .shell_actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::BeginLocalChatSend(_)))
    );
}

#[test]
fn gui_shell_dispatch_plan_routes_chat_alias_commands_to_runtime_send() {
    let state = runtime_ready_state();
    let plan = GuiShellDispatchPlan::from_shell_actions(
        &state,
        vec![GuiShellAction::BeginLocalChatSend(
            "/ch hello room".to_owned(),
        )],
    );

    assert_eq!(
        plan.runtime_requests,
        vec![GuiRuntimeRequest::SendChatMessage("hello room".to_owned())]
    );
    assert!(
        plan.shell_actions
            .contains(&GuiShellAction::AnnounceSystemChatEvent(
                "/ch hello room".to_owned()
            ))
    );
    assert!(
        !plan
            .shell_actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::BeginLocalChatSend(_)))
    );
}

#[test]
fn gui_shell_dispatch_plan_routes_pause_commands_to_direct_runtime_toggle() {
    let state = runtime_ready_state();
    let plan = GuiShellDispatchPlan::from_shell_actions(
        &state,
        vec![GuiShellAction::BeginLocalChatSend("/pause".to_owned())],
    );

    assert_eq!(
        plan.runtime_requests,
        vec![GuiRuntimeRequest::TogglePlaybackPause]
    );
    assert!(
        plan.shell_actions
            .contains(&GuiShellAction::AnnounceSystemChatEvent(
                "/pause".to_owned()
            ))
    );
}

#[test]
fn gui_shell_dispatch_plan_routes_local_ready_commands_without_usernames() {
    let state = runtime_ready_state();
    let plan = GuiShellDispatchPlan::from_shell_actions(
        &state,
        vec![GuiShellAction::BeginLocalChatSend("/toggle".to_owned())],
    );

    assert_eq!(
        plan.runtime_requests,
        vec![GuiRuntimeRequest::SetLocalReady(true)]
    );
    assert!(
        plan.shell_actions
            .contains(&GuiShellAction::AnnounceLocalUserReady)
    );
}

#[test]
fn gui_shell_dispatch_plan_toggles_from_displayed_pending_local_ready_state() {
    let mut state = runtime_ready_state();
    state.pending_local_ready_target = Some(true);

    let plan = GuiShellDispatchPlan::from_shell_actions(
        &state,
        vec![GuiShellAction::BeginLocalChatSend("/toggle".to_owned())],
    );

    assert_eq!(
        plan.runtime_requests,
        vec![GuiRuntimeRequest::SetLocalReady(false)]
    );
    assert!(
        plan.shell_actions
            .contains(&GuiShellAction::AnnounceLocalUserNotReady)
    );
}

#[test]
fn gui_shell_dispatch_plan_emits_playlist_index_errors_locally() {
    let state = runtime_ready_state();
    let plan = GuiShellDispatchPlan::from_shell_actions(
        &state,
        vec![GuiShellAction::BeginLocalChatSend("/select 99".to_owned())],
    );

    assert!(plan.runtime_requests.is_empty());
    assert!(plan.shell_actions.iter().any(|action| matches!(
        action,
        GuiShellAction::AnnounceSystemChatEvent(message)
            if message.contains("Invalid playlist index")
    )));
}

#[test]
fn gui_shell_dispatch_plan_routes_stream_helper_import_actions_to_runtime_requests() {
    let state = runtime_ready_state();
    let plan = GuiShellDispatchPlan::from_shell_actions(
        &state,
        vec![
            GuiShellAction::IntegrateStreamHelperDownloader("C:/Tools/yt-dlp.exe".to_owned()),
            GuiShellAction::IntegrateStreamHelperJsRuntime("C:/Tools/deno.exe".to_owned()),
        ],
    );

    assert_eq!(
        plan.runtime_requests,
        vec![
            GuiRuntimeRequest::IntegrateStreamHelperDownloader("C:/Tools/yt-dlp.exe".to_owned()),
            GuiRuntimeRequest::IntegrateStreamHelperJsRuntime("C:/Tools/deno.exe".to_owned()),
        ]
    );
    assert!(plan.shell_actions.is_empty());
}
