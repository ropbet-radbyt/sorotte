use super::*;

use super::super::remote_services::{
    LegacyUpdateCheckStatus, StagedUpdate, UpdateCandidate, UpdateCandidateSource, UpdateChannel,
};
use crate::app::{
    GuiDraftRuntimeSnapshot, GuiPlexPlaylistJobCancellationReason, GuiPlexPlaylistSearchResult,
    GuiPluginSelection, GuiRuntimeRequest, GuiSeekPreparationPhase, GuiSeekPreparationState,
    GuiShellAction, MainWindowRuntimeSnapshot, MainWindowRuntimeUserSnapshot,
    SorotteGuiShellAppState, StoredClientSettingsMvp,
};
use sorotte_plex::PlexMediaType;

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

fn staged_update(candidate: UpdateCandidate) -> StagedUpdate {
    StagedUpdate {
        candidate,
        package_path: "C:/Temp/sorotte.zip".to_owned(),
        source_dir: "C:/Temp/sorotte-update".to_owned(),
        updater_path: "C:/Temp/sorotte-update/sorotte-gui-updater.exe".to_owned(),
        target_exe_path: "C:/Program Files/Sorotte/sorotte-gui.exe".to_owned(),
        backup_dir: "C:/Temp/sorotte-backup".to_owned(),
        log_path: "C:/Temp/sorotte-update.log".to_owned(),
        restart: true,
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
        value: "dev".to_owned().into(),
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
fn gui_shell_dispatch_plan_routes_plugin_enablement_to_runtime_owner() {
    let state = runtime_ready_state();
    let action = GuiShellAction::SetPluginEnabled {
        plugin: GuiPluginSelection::Plex,
        enabled: false,
    };
    let plan = GuiShellDispatchPlan::from_shell_actions(&state, vec![action.clone()]);

    assert_eq!(plan.shell_actions, vec![action]);
    assert_eq!(
        plan.pre_shell_runtime_requests,
        vec![GuiRuntimeRequest::SetPluginEnabled {
            plugin: GuiPluginSelection::Plex,
            enabled: false,
        }]
    );
    assert!(plan.runtime_requests.is_empty());
}

#[test]
fn gui_shell_dispatch_plan_routes_plex_playlist_picker_requests_to_runtime_owner() {
    let mut state = runtime_ready_state();
    state.plex_playlist_search = Some(super::super::shell_state::GuiPlexPlaylistSearchState {
        query: "zero".to_owned(),
        results: vec![GuiPlexPlaylistSearchResult {
            rating_key: "14452".to_owned(),
            title: "Episode 11".to_owned(),
            parent_title: Some("Season 4".to_owned()),
            grandparent_title: Some("Re:Zero".to_owned()),
            media_type: PlexMediaType::Episode,
            duration_millis: Some(1_470_058),
            file_name: Some("Episode 11.mkv".to_owned()),
        }],
        selected_index: Some(0),
        searching: false,
        adding_rating_key: None,
        error: None,
    });

    let search_plan = GuiShellDispatchPlan::from_shell_actions(
        &state,
        vec![GuiShellAction::SubmitPlexPlaylistSearch {
            query: "zero".to_owned(),
        }],
    );
    assert_eq!(
        search_plan.shell_actions,
        vec![GuiShellAction::SubmitPlexPlaylistSearch {
            query: "zero".to_owned(),
        }]
    );
    assert_eq!(
        search_plan.pre_shell_runtime_requests,
        vec![GuiRuntimeRequest::SearchSelectedPlexServerMedia {
            query: "zero".to_owned(),
        }]
    );
    assert!(search_plan.runtime_requests.is_empty());

    let add_plan = GuiShellDispatchPlan::from_shell_actions(
        &state,
        vec![
            GuiShellAction::SelectPlexPlaylistSearchResult(0),
            GuiShellAction::AddSelectedPlexPlaylistSearchResult,
        ],
    );
    assert_eq!(
        add_plan.pre_shell_runtime_requests,
        vec![GuiRuntimeRequest::ResolvePlexPlaylistItem {
            rating_key: "14452".to_owned(),
        }]
    );
    assert!(add_plan.runtime_requests.is_empty());

    let cancel_plan = GuiShellDispatchPlan::from_shell_actions(
        &state,
        vec![GuiShellAction::CancelPlexPlaylistSearch],
    );
    assert_eq!(
        cancel_plan.shell_actions,
        vec![GuiShellAction::CancelPlexPlaylistSearch]
    );
    assert_eq!(
        cancel_plan.pre_shell_runtime_requests,
        vec![GuiRuntimeRequest::CancelPlexPlaylistJobs {
            reason: GuiPlexPlaylistJobCancellationReason::PickerClosed,
        }]
    );
    assert!(cancel_plan.runtime_requests.is_empty());
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
fn gui_shell_dispatch_plan_does_not_route_removed_advanced_update_check() {
    let state = runtime_ready_state();

    assert!(
        state
            .menus
            .sections
            .iter()
            .find(|section| section.title == "Advanced")
            .is_some_and(|section| !section
                .actions
                .iter()
                .any(|action| action.label == "Update Check"))
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
fn gui_shell_dispatch_plan_routes_update_indicator_checks() {
    let state = runtime_ready_state();
    let plan = GuiShellDispatchPlan::from_shell_actions(
        &state,
        vec![GuiShellAction::ActivateUpdateIndicator],
    );

    assert_eq!(
        plan.shell_actions,
        vec![
            GuiShellAction::ActivateUpdateIndicator,
            GuiShellAction::BeginUpdateCheck {
                user_initiated: true,
            }
        ]
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
fn gui_shell_dispatch_plan_routes_update_indicator_one_click_install() {
    let mut state = runtime_ready_state();
    let candidate = update_candidate();
    state.update_check.status = Some(LegacyUpdateCheckStatus::UpdateAvailable);
    state.update_check.candidate = Some(candidate.clone());
    state.update_check.self_update_supported = true;

    let plan = GuiShellDispatchPlan::from_shell_actions(
        &state,
        vec![GuiShellAction::ActivateUpdateIndicator],
    );

    assert_eq!(
        plan.shell_actions,
        vec![
            GuiShellAction::ActivateUpdateIndicator,
            GuiShellAction::BeginUpdateInstall
        ]
    );
    assert_eq!(
        plan.runtime_requests,
        vec![GuiRuntimeRequest::DownloadAndInstallUpdate(candidate)]
    );
}

#[test]
fn gui_shell_dispatch_plan_routes_update_indicator_staged_install() {
    let mut state = runtime_ready_state();
    let staged = staged_update(update_candidate());
    state.update_check.staged_update = Some(staged.clone());
    state.update_check.self_update_supported = true;

    let plan = GuiShellDispatchPlan::from_shell_actions(
        &state,
        vec![GuiShellAction::ActivateUpdateIndicator],
    );

    assert_eq!(
        plan.shell_actions,
        vec![
            GuiShellAction::ActivateUpdateIndicator,
            GuiShellAction::BeginStagedUpdateApply
        ]
    );
    assert_eq!(
        plan.runtime_requests,
        vec![GuiRuntimeRequest::ApplyStagedUpdate(staged)]
    );
}

#[test]
fn gui_shell_dispatch_plan_ignores_update_indicator_while_checking() {
    let mut state = runtime_ready_state();
    state.update_check.status = Some(LegacyUpdateCheckStatus::Checking);

    let plan = GuiShellDispatchPlan::from_shell_actions(
        &state,
        vec![GuiShellAction::ActivateUpdateIndicator],
    );

    assert_eq!(
        plan.shell_actions,
        vec![GuiShellAction::ActivateUpdateIndicator]
    );
    assert!(plan.runtime_requests.is_empty());
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

#[test]
fn gui_shell_dispatch_plan_routes_media_match_actions_to_runtime_requests() {
    let state = runtime_ready_state();
    let plan = GuiShellDispatchPlan::from_shell_actions(
        &state,
        vec![
            GuiShellAction::InstallMediaMatchTools,
            GuiShellAction::ImportMediaMatchFfmpeg("C:/Tools/ffmpeg.exe".to_owned()),
            GuiShellAction::ImportMediaMatchFfprobe("C:/Tools/ffprobe.exe".to_owned()),
            GuiShellAction::OpenMediaMatchInstallLocation,
            GuiShellAction::RecheckMediaMatchTools,
            GuiShellAction::RebuildMediaMatchIndex,
            GuiShellAction::CancelMediaMatchRebuild,
            GuiShellAction::ClearMediaMatchCache,
        ],
    );

    assert_eq!(
        plan.runtime_requests,
        vec![
            GuiRuntimeRequest::InstallMediaMatchTools,
            GuiRuntimeRequest::ImportMediaMatchFfmpeg("C:/Tools/ffmpeg.exe".to_owned()),
            GuiRuntimeRequest::ImportMediaMatchFfprobe("C:/Tools/ffprobe.exe".to_owned()),
            GuiRuntimeRequest::OpenMediaMatchInstallLocation,
            GuiRuntimeRequest::RecheckMediaMatchTools,
            GuiRuntimeRequest::RebuildMediaMatchIndex,
            GuiRuntimeRequest::CancelMediaMatchRebuild,
            GuiRuntimeRequest::ClearMediaMatchCache,
        ]
    );
    assert!(plan.shell_actions.is_empty());
}

#[test]
fn gui_shell_dispatch_plan_routes_media_match_settings_to_shell_and_runtime() {
    let state = runtime_ready_state();
    let plan = GuiShellDispatchPlan::from_shell_actions(
        &state,
        vec![
            GuiShellAction::SetMediaMatchFingerprintingEnabled(true),
            GuiShellAction::SetMediaMatchBackgroundWarmupEnabled(false),
            GuiShellAction::SetMediaMatchWireSharingEnabled(false),
            GuiShellAction::SetMediaMatchRuntimeToleranceEnabled(false),
            GuiShellAction::SetMediaMatchAutoplayPolicy(
                sorotte_media_match::MediaMatchAutoplayPolicy::AllowStrongSameMedia,
            ),
        ],
    );

    assert_eq!(
        plan.shell_actions,
        vec![
            GuiShellAction::SetMediaMatchFingerprintingEnabled(true),
            GuiShellAction::SetMediaMatchBackgroundWarmupEnabled(false),
            GuiShellAction::SetMediaMatchWireSharingEnabled(false),
            GuiShellAction::SetMediaMatchRuntimeToleranceEnabled(false),
            GuiShellAction::SetMediaMatchAutoplayPolicy(
                sorotte_media_match::MediaMatchAutoplayPolicy::AllowStrongSameMedia,
            ),
        ]
    );
    assert_eq!(
        plan.runtime_requests,
        vec![
            GuiRuntimeRequest::SetMediaMatchFingerprintingEnabled(true),
            GuiRuntimeRequest::SetMediaMatchBackgroundWarmupEnabled(false),
            GuiRuntimeRequest::SetMediaMatchWireSharingEnabled(false),
            GuiRuntimeRequest::SetMediaMatchRuntimeToleranceEnabled(false),
            GuiRuntimeRequest::SetMediaMatchAutoplayPolicy(
                sorotte_media_match::MediaMatchAutoplayPolicy::AllowStrongSameMedia,
            ),
        ]
    );
}

#[test]
fn gui_shell_dispatch_plan_routes_only_safe_seek_preparation_controls() {
    let mut state = runtime_ready_state();
    state.seek_preparation = Some(GuiSeekPreparationState {
        phase: GuiSeekPreparationPhase::Refilling,
        frozen_target_seconds: 120.0,
        cache_refill_percent: Some(50.0),
        buffered_ahead_seconds: Some(5.0),
        nearest_safe_buffered_position_seconds: Some(115.0),
        can_keep_waiting: true,
        can_cancel_and_remain: false,
        can_join_nearest_buffered: true,
    });

    let plan = GuiShellDispatchPlan::from_shell_actions(
        &state,
        vec![
            GuiShellAction::RequestSeekPreparationKeepWaiting,
            GuiShellAction::RequestSeekPreparationCancel,
            GuiShellAction::RequestSeekPreparationJoinNearest,
        ],
    );
    assert_eq!(
        plan.runtime_requests,
        vec![
            GuiRuntimeRequest::KeepWaitingForSeekPreparation,
            GuiRuntimeRequest::JoinNearestBufferedSeekPreparation,
        ]
    );
    assert!(plan.shell_actions.is_empty());

    if let Some(preparation) = state.seek_preparation.as_mut() {
        preparation.can_cancel_and_remain = true;
        preparation.can_join_nearest_buffered = false;
    }
    let cancellable = GuiShellDispatchPlan::from_shell_actions(
        &state,
        vec![GuiShellAction::RequestSeekPreparationCancel],
    );
    assert_eq!(
        cancellable.runtime_requests,
        vec![GuiRuntimeRequest::CancelSeekPreparation]
    );

    state.seek_preparation = None;
    let unavailable = GuiShellDispatchPlan::from_shell_actions(
        &state,
        vec![
            GuiShellAction::RequestSeekPreparationKeepWaiting,
            GuiShellAction::RequestSeekPreparationCancel,
            GuiShellAction::RequestSeekPreparationJoinNearest,
        ],
    );
    assert!(unavailable.runtime_requests.is_empty());
    assert!(unavailable.shell_actions.is_empty());
}

#[test]
fn gui_local_commands_route_only_available_seek_preparation_controls() {
    let mut state = runtime_ready_state();
    state.seek_preparation = Some(GuiSeekPreparationState {
        phase: GuiSeekPreparationPhase::Refilling,
        frozen_target_seconds: 120.0,
        cache_refill_percent: Some(50.0),
        buffered_ahead_seconds: Some(5.0),
        nearest_safe_buffered_position_seconds: Some(115.0),
        can_keep_waiting: true,
        can_cancel_and_remain: false,
        can_join_nearest_buffered: true,
    });

    let plan = GuiShellDispatchPlan::from_shell_actions(
        &state,
        vec![
            GuiShellAction::BeginLocalChatSend("/keep-waiting".to_owned()),
            GuiShellAction::BeginLocalChatSend("/cancel-and-remain".to_owned()),
            GuiShellAction::BeginLocalChatSend("/join-nearest-buffered-position".to_owned()),
        ],
    );
    assert_eq!(
        plan.runtime_requests,
        vec![
            GuiRuntimeRequest::KeepWaitingForSeekPreparation,
            GuiRuntimeRequest::JoinNearestBufferedSeekPreparation,
        ]
    );

    state.seek_preparation = None;
    let unavailable = GuiShellDispatchPlan::from_shell_actions(
        &state,
        vec![GuiShellAction::BeginLocalChatSend(
            "/keep-waiting".to_owned(),
        )],
    );
    assert!(unavailable.runtime_requests.is_empty());
}
