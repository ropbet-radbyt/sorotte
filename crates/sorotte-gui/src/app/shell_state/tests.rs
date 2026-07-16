use std::collections::BTreeMap;

use super::{
    FirstRunConfigurationDialogState, GuiCommandAvailabilityState, GuiCommandRuntimeSnapshot,
    GuiConfigurationDraftRuntimeSnapshot, GuiConfigurationRuntimeSnapshot, GuiConfigurationTab,
    GuiConfigurationTextValue, GuiControllerAuthEditSessionState, GuiDialogControl,
    GuiDialogControlKind, GuiDraftRuntimeSnapshot, GuiErrorRuntimeSnapshot,
    GuiFeedbackRuntimeSnapshot, GuiFocusedConfigurationControlRuntimeSnapshot,
    GuiInteractionRuntimeSnapshot, GuiMainWindowUserEditSessionRuntimeSnapshot,
    GuiMediaSourceProviderId, GuiPendingOperationKind, GuiPlaylistDefaultSourceId,
    GuiPlaylistEntryId, GuiPlaylistResolutionStep, GuiPlaylistSourcePolicy,
    GuiPlaylistSourceSelectionOrigin, GuiPlaylistSourceState, GuiPlaylistSourceStatus,
    GuiPlaylistTextEditSessionRuntimeSnapshot, GuiPlaylistTextEditSessionState,
    GuiPlexPlaylistSearchResult, GuiPlexRuntimeSnapshot, GuiPluginSelection,
    GuiPublicServerEditSessionRuntimeSnapshot, GuiSavedConfigurationRuntimeSnapshot,
    GuiSavedSessionConnectTarget, GuiSelectionState, GuiShellAction, GuiShellModal, GuiShellView,
    GuiStreamTargetKind, GuiTextEditSessionRuntimeSnapshot, GuiTextEditSessionState,
    GuiTransientNotification, GuiTransientNotificationLevel, GuiUrlEditSessionRuntimeSnapshot,
    GuiUrlEditSessionState, GuiValidationIssue, GuiWidgetKind, MainWindowChatRow,
    MainWindowPlaylistRow, MainWindowRuntimeChatSnapshot, MainWindowRuntimeSnapshot,
    MainWindowRuntimeUserSnapshot, MainWindowShellState, MediaSearchDirectoryRow,
    MediaSearchWorkflowShellState, MenuActionId, MenuActionRuntimeOverride,
    MenuDialogRuntimeSnapshot, MenuDialogShellState, PublicServerBrowserRow,
    PublicServerBrowserShellState, SettingId, SorotteGuiRuntimeSnapshot, SorotteGuiShellAppState,
    browser_stream_target_kind, playlist_entries_from_multiline_text,
    save_playlist_entries_to_path,
};

use crate::app::widget_tree::GuiWidgetTextPreviewRenderer;

#[test]
fn projected_media_and_plex_debug_redacts_tokenized_urls() {
    let marker = "gui-projection-token-canary";
    let target = format!("https://media.example/video?X-Plex-Token={marker}");
    let settings = StoredClientSettingsMvp {
        plex_selected_server_url: Some(target.clone()),
        ..StoredClientSettingsMvp::default()
    };

    let mut shell = MainWindowShellState::from_stored_settings(&settings);
    shell.users[0].file_name = Some(target.clone());
    shell.users[0].file_name_label = target.clone();
    shell.playlist = vec![MainWindowPlaylistRow::inferred(target.clone(), true)];
    let runtime = MainWindowRuntimeSnapshot::from_shell_state(&shell);

    let stream = super::GuiStreamHelperState {
        target: Some(target.clone()),
        ..super::GuiStreamHelperState::default()
    };
    let stream_runtime = super::GuiStreamHelperRuntimeSnapshot {
        target: Some(target.clone()),
        ..super::GuiStreamHelperRuntimeSnapshot::default()
    };

    let mut plex = super::GuiPlexState::from_stored_settings(&settings);
    plex.auth_code = Some(target.clone());
    plex.auth_url = Some(target.clone());
    plex.selected_server_url = Some(target.clone());
    plex.servers.push(super::GuiPlexServerRow {
        name: "server".to_owned(),
        machine_identifier: "machine".to_owned(),
        uri: target.clone(),
        reachability: super::GuiPlexServerReachability::Unknown,
        connection_kind: sorotte_plex::PlexServerConnectionKind::Remote,
        has_local_connection: false,
        owned: true,
        selected: true,
    });
    let plex_runtime = GuiPlexRuntimeSnapshot::from(&plex);

    for debug in [
        format!("{settings:?}"),
        format!("{:?}", shell.users[0]),
        format!("{:?}", shell.playlist[0]),
        format!("{shell:?}"),
        format!("{runtime:?}"),
        format!("{stream:?}"),
        format!("{stream_runtime:?}"),
        format!("{plex:?}"),
        format!("{plex_runtime:?}"),
    ] {
        assert!(debug.contains(sorotte_secret::REDACTED_SECRET));
        assert!(!debug.contains(marker), "leaky Debug output: {debug}");
    }
}

#[test]
fn playlist_source_debug_redacts_target_bearing_details() {
    let path_marker = "playlist-local-path-debug-canary";
    let url_marker = "playlist-tokenized-url-debug-canary";
    let local_path = format!("C:/Private/{path_marker}/movie.mkv");
    let tokenized_url = format!("https://media.example/movie?X-Plex-Token={url_marker}");
    let mut source_state = GuiPlaylistSourceState::for_provider(GuiMediaSourceProviderId::local());
    source_state.status = GuiPlaylistSourceStatus::Active;
    source_state.detail = Some(format!(
        "Loaded local target: {local_path}; remote fallback: {tokenized_url}."
    ));
    source_state.options[0].detail = Some(format!("Resolved option target: {tokenized_url}."));
    let resolution_step = GuiPlaylistResolutionStep {
        provider_id: GuiMediaSourceProviderId::local(),
        label: "Local".to_owned(),
        status: GuiPlaylistSourceStatus::Active,
        detail: Some(format!("Loaded target: {tokenized_url}.")),
    };
    source_state.resolution_steps = vec![resolution_step.clone()];

    let row = MainWindowPlaylistRow {
        entry_id: GuiPlaylistEntryId::next(),
        label: tokenized_url,
        is_selected: true,
        source_state: source_state.clone(),
    };
    let mut shell = MainWindowShellState::from_stored_settings(&StoredClientSettingsMvp::default());
    shell.playlist = vec![row.clone()];
    let runtime = MainWindowRuntimeSnapshot::from_shell_state(&shell);

    let step_debug = format!("{resolution_step:?}");
    assert!(step_debug.contains("local"));
    assert!(step_debug.contains("Active"));

    let source_debug = format!("{source_state:?}");
    assert!(source_debug.contains("selection_origin"));
    assert!(source_debug.contains("policy"));
    assert!(source_debug.contains("GuiPlaylistSourceOption"));

    for debug in [
        step_debug,
        source_debug,
        format!("{row:?}"),
        format!("{shell:?}"),
        format!("{runtime:?}"),
    ] {
        assert!(debug.contains(sorotte_secret::REDACTED_SECRET));
        assert!(!debug.contains(path_marker), "leaky Debug output: {debug}");
        assert!(!debug.contains(url_marker), "leaky Debug output: {debug}");
    }
}
use crate::app::{
    GuiDroppedFilesTarget, GuiLaunchMode, GuiWidgetEguiRenderer,
    remote_services::{LegacyUpdateCheckResult, LegacyUpdateCheckStatus},
    testing::support::{TEST_USERNAME, test_temp_root},
};
use sorotte_client_app::app_boundary::state::{AutoplayThresholdOverride, StoredClientSettingsMvp};
use sorotte_client_core::{PrivacyMode, UnpauseActionMode};
use sorotte_plex::PlexMediaType;

mod command_tests;
mod main_window_playlist_tests;
mod menu_public_server_tests;
mod runtime_snapshot_tests;

fn assert_chat_pane_ready(chat: &[super::MainWindowChatRow]) {
    assert_eq!(chat.len(), 1);
    assert_eq!(chat[0].sender, "system");
    assert_eq!(chat[0].message, "Chat pane ready");
}

#[test]
fn configuration_surface_defaults_to_first_run_mode() {
    let state =
        FirstRunConfigurationDialogState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert_eq!(state.launch_mode, GuiLaunchMode::FirstRun);
    assert_eq!(state.system.language_tag, "en");
    assert_eq!(state.readiness.unpause_action.stored_override, None);
    assert_eq!(state.readiness.unpause_action.effective, "IfOthersReady");
    assert_eq!(state.readiness.autoplay_min_users.stored_override, None);
    assert_eq!(state.readiness.autoplay_min_users.effective, "0");
    assert!(state.chat.chat_input_enabled);
    assert!(state.chat.chat_output_enabled);
    assert_eq!(state.chat.chat_input_position_label, "Top");
    assert_eq!(state.chat.chat_output_mode_label, "Chatroom");
    assert_eq!(state.connection.public_server_count, 0);
}

#[test]
fn browser_stream_target_kind_classifies_direct_and_extractor_urls() {
    assert_eq!(
        browser_stream_target_kind("C:/Media/movie.mkv", None),
        GuiStreamTargetKind::LocalPath
    );
    assert_eq!(
        browser_stream_target_kind("https://cdn.example.com/live/stream.m3u8", None),
        GuiStreamTargetKind::DirectMediaUrl
    );
    assert_eq!(
        browser_stream_target_kind("https://www.youtube.com/watch?v=UyjIPZfygTk", None),
        GuiStreamTargetKind::ExtractorPageUrl
    );
    assert_eq!(
        browser_stream_target_kind("https://youtu.be/UyjIPZfygTk", None),
        GuiStreamTargetKind::ExtractorPageUrl
    );
    assert_eq!(
        browser_stream_target_kind("https://cdn.example.com/watch/trailer.m3u8", None),
        GuiStreamTargetKind::DirectMediaUrl
    );
    assert_eq!(
        browser_stream_target_kind("https://cdn.example.com/shorts/trailer.mp4", None),
        GuiStreamTargetKind::DirectMediaUrl
    );
    assert_eq!(
        browser_stream_target_kind(
            "plex://machine-1/metadata/123?title=Episode%201&file=Episode%201.mkv",
            None,
        ),
        GuiStreamTargetKind::PlexUri
    );

    let trusted_domains = vec!["example.org".to_owned()];
    assert_eq!(
        browser_stream_target_kind(
            "https://www.youtube.com/watch?v=UyjIPZfygTk",
            Some((true, trusted_domains.as_slice())),
        ),
        GuiStreamTargetKind::UntrustedUrl
    );
}

#[test]
fn gui_shell_app_state_defaults_to_setup_connection() {
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert_eq!(state.active_view, GuiShellView::Setup);
    assert_eq!(
        state.selected_configuration_tab,
        GuiConfigurationTab::Connection
    );
}

#[test]
fn configuration_surface_preserves_explicit_false_chat_settings() {
    let state = FirstRunConfigurationDialogState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(false),
        chat_output_enabled: Some(false),
        ..StoredClientSettingsMvp::default()
    });

    assert!(!state.chat.chat_input_enabled);
    assert!(!state.chat.chat_output_enabled);
}

#[test]
fn gui_shell_app_state_opens_room_for_room_workflows_and_preserves_hidden_sessions() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        room: Some("+room:ABCDEF123456".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::BeginSharedPlaylistTextEdit));
    assert_eq!(state.active_view, GuiShellView::Room);
    assert!(state.playlist_text_edit_session.is_some());

    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::Setup)));
    assert!(state.playlist_text_edit_session.is_some());

    assert!(state.apply(GuiShellAction::BeginMediaUrlEdit));
    assert_eq!(state.active_view, GuiShellView::Room);
    assert!(state.media_url_edit_session.is_some());

    assert!(state.apply(GuiShellAction::BeginRoomHistoryEdit));
    assert_eq!(
        state.selected_configuration_tab,
        GuiConfigurationTab::Connection
    );
    assert!(state.room_history_edit_session.is_some());

    assert!(state.apply(GuiShellAction::FocusConfigurationControl(
        SettingId::PrivacyTrustedDomains,
    )));
    assert_eq!(
        state.selected_configuration_tab,
        GuiConfigurationTab::PrivacyChat
    );
}

#[test]
fn configuration_surface_maps_existing_stored_settings_into_sections() {
    let mut per_player_arguments = BTreeMap::new();
    per_player_arguments.insert(
        "C:/Program Files/mpv/mpv.exe".to_owned(),
        vec!["--profile=fast".to_owned(), "--no-border".to_owned()],
    );
    let state = FirstRunConfigurationDialogState::from_stored_settings(&StoredClientSettingsMvp {
        language: Some("pt-br".to_owned()),
        check_for_updates_automatically: Some(true),
        update_channel: Some("dev".to_owned()),
        force_gui_prompt: Some(true),
        host: Some("syncplay.example".to_owned()),
        port: Some(8995),
        server_password: Some("secret".into()),
        username: Some(TEST_USERNAME.to_owned()),
        room: Some("room-a".to_owned()),
        room_list: Some(vec!["room-a".to_owned(), "room-b".to_owned()]),
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        per_player_arguments: Some(per_player_arguments),
        public_servers: Some(vec![("Public".to_owned(), "example.org:8999".to_owned())]),
        media_search_directories: Some(vec!["C:/Media".to_owned(), "D:/Archive".to_owned()]),
        folder_search_first_file_timeout_seconds: Some(2.0),
        folder_search_timeout_seconds: Some(5.5),
        folder_search_double_check_interval_seconds: Some(0.5),
        folder_search_warning_threshold_seconds: Some(3.0),
        autoplay_initial_state: Some(true),
        autoplay_require_same_filenames: Some(true),
        ready_at_start: Some(true),
        shared_playlist_enabled: Some(true),
        pause_on_leave: Some(true),
        loop_at_end_of_playlist: Some(true),
        loop_single_files: Some(true),
        only_switch_to_trusted_domains: Some(true),
        trusted_domains: Some(vec!["example.org".to_owned(), "syncplay.pl".to_owned()]),
        rewind_on_desync: Some(true),
        fastforward_on_desync: Some(true),
        slow_on_desync: Some(true),
        dont_slow_down_with_me: Some(true),
        rewind_threshold_seconds: Some(1.5),
        fastforward_threshold_seconds: Some(4.0),
        slowdown_threshold_seconds: Some(0.75),
        unpause_action: Some(UnpauseActionMode::IfMinUsersReady),
        autoplay_min_users: Some(AutoplayThresholdOverride::Set(3)),
        filename_privacy_mode: Some(PrivacyMode::SendHashed),
        filesize_privacy_mode: Some(PrivacyMode::DoNotSend),
        show_duration_notification: Some(true),
        autosave_joins_to_list: Some(true),
        show_osd: Some(true),
        chat_input_enabled: Some(true),
        chat_input_font_family: Some("Consolas".to_owned()),
        chat_input_relative_font_size: Some(26),
        chat_input_font_weight: Some(50),
        chat_input_font_color: Some("#abcdef".to_owned()),
        chat_input_position: Some("Bottom".to_owned()),
        chat_direct_input: Some(true),
        chat_output_enabled: Some(true),
        chat_output_font_family: Some("Segoe UI".to_owned()),
        chat_output_relative_font_size: Some(20),
        chat_output_font_weight: Some(60),
        chat_output_mode: Some("Scrolling".to_owned()),
        chat_move_osd: Some(true),
        chat_max_lines: Some(7),
        chat_top_margin: Some(25),
        chat_left_margin: Some(20),
        chat_bottom_margin: Some(30),
        chat_osd_margin: Some(110),
        notification_timeout_seconds: Some(3),
        alert_timeout_seconds: Some(5),
        chat_timeout_seconds: Some(7),
        show_same_room_osd: Some(true),
        show_osd_warnings: Some(true),
        show_slowdown_osd: Some(true),
        show_noncontroller_osd: Some(true),
        show_different_room_osd: Some(true),
        show_contact_info: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert_eq!(state.launch_mode, GuiLaunchMode::ExistingConfig);
    assert_eq!(state.system.language_tag, "pt_BR");
    assert_eq!(state.system.update_channel_label, "dev");
    assert_eq!(state.connection.host.as_deref(), Some("syncplay.example"));
    assert_eq!(state.connection.port, Some(8995));
    assert!(state.connection.server_password_set);
    assert_eq!(state.connection.public_server_count, 1);
    assert_eq!(
        state.connection.player_arguments_text,
        "--profile=fast --no-border"
    );
    assert_eq!(state.connection.room_history_text, "room-a\nroom-b");
    assert_eq!(state.connection.room_history_count, 2);
    assert!(state.readiness.loop_at_end_of_playlist);
    assert!(state.readiness.loop_single_files);
    assert_eq!(
        state.readiness.unpause_action.stored_override.as_deref(),
        Some("IfMinUsersReady")
    );
    assert_eq!(state.readiness.unpause_action.effective, "IfMinUsersReady");
    assert_eq!(
        state
            .readiness
            .autoplay_min_users
            .stored_override
            .as_deref(),
        Some("3")
    );
    assert_eq!(state.readiness.autoplay_min_users.effective, "3");
    assert_eq!(state.privacy.filename_privacy_mode_label, "SendHashed");
    assert_eq!(state.privacy.filesize_privacy_mode_label, "DoNotSend");
    assert_eq!(
        state.privacy.trusted_domains_text,
        "example.org\nsyncplay.pl"
    );
    assert_eq!(state.privacy.trusted_domain_count, 2);
    assert_eq!(
        state.media_search.media_directories_text,
        "C:/Media\nD:/Archive"
    );
    assert_eq!(state.media_search.media_directory_count, 2);
    assert_eq!(state.chat.chat_input_position_label, "Bottom");
    assert_eq!(
        state.chat.chat_input_font_family.as_deref(),
        Some("Consolas")
    );
    assert_eq!(state.chat.chat_input_relative_font_size, Some(26));
    assert_eq!(state.chat.chat_input_font_weight, Some(50));
    assert_eq!(state.chat.chat_input_font_color.as_deref(), Some("#abcdef"));
    assert_eq!(state.chat.chat_output_mode_label, "Scrolling");
    assert_eq!(
        state.chat.chat_output_font_family.as_deref(),
        Some("Segoe UI")
    );
    assert_eq!(state.chat.chat_output_relative_font_size, Some(20));
    assert_eq!(state.chat.chat_output_font_weight, Some(60));
    assert_eq!(state.chat.chat_top_margin, Some(25));
    assert_eq!(state.chat.chat_left_margin, Some(20));
    assert_eq!(state.chat.chat_bottom_margin, Some(30));
    assert_eq!(state.chat.chat_osd_margin, Some(110));
    assert!(state.osd.show_slowdown_osd);
    assert_eq!(state.osd.notification_timeout_seconds, Some(3));
    assert_eq!(state.osd.alert_timeout_seconds, Some(5));
    assert_eq!(state.osd.chat_timeout_seconds, Some(7));
    assert!(state.osd.show_contact_info);
    assert!(state.system.check_for_updates_automatically);
    assert!(state.system.autosave_joins_to_list);
    assert!(state.system.force_gui_prompt);
}

#[test]
fn configuration_surface_exposes_typed_dialog_controls_for_editable_fields() {
    let state =
        FirstRunConfigurationDialogState::from_stored_settings(&StoredClientSettingsMvp::default());
    let sections = state.dialog_sections();

    let connection = sections
        .iter()
        .find(|section| section.title == "Connection")
        .expect("connection section should exist");
    assert!(connection.controls.iter().any(|control| {
        control.label == "Host" && control.kind == GuiDialogControlKind::TextInput
    }));
    assert!(connection.controls.iter().any(|control| {
        control.label == "Server Password" && control.kind == GuiDialogControlKind::PasswordInput
    }));
    assert!(connection.controls.iter().any(|control| {
        control.label == "Room History" && control.kind == GuiDialogControlKind::TextArea
    }));

    let readiness = sections
        .iter()
        .find(|section| section.title == "Readiness")
        .expect("readiness section should exist");
    assert!(readiness.controls.iter().any(|control| {
        control.label == "Autoplay" && control.kind == GuiDialogControlKind::Checkbox
    }));
    assert!(readiness.controls.iter().any(|control| {
        control.label == "Unpause Action" && control.kind == GuiDialogControlKind::Select
    }));
    let privacy = sections
        .iter()
        .find(|section| section.title == "Privacy")
        .expect("privacy section should exist");
    assert!(privacy.controls.iter().any(|control| {
        control.label == "Trusted Domains" && control.kind == GuiDialogControlKind::TextArea
    }));
    let system = sections
        .iter()
        .find(|section| section.title == "System")
        .expect("system section should exist");
    assert!(system.controls.iter().any(|control| {
        control.label == "Update Channel" && control.kind == GuiDialogControlKind::Select
    }));
}

#[test]
fn main_window_shell_state_uses_settings_for_room_user_and_controls() {
    let state = MainWindowShellState::from_stored_settings(&StoredClientSettingsMvp {
        room: Some("+room:ABCDEF123456".to_owned()),
        username: Some(TEST_USERNAME.to_owned()),
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        shared_playlist_enabled: Some(true),
        ready_at_start: Some(true),
        chat_output_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert_eq!(state.room_name, "+room:ABCDEF123456");
    assert!(state.shared_playlist_enabled);
    assert!(state.controlled_room_active);
    assert_eq!(state.users.len(), 1);
    assert_eq!(state.users[0].username, TEST_USERNAME);
    assert!(!state.users[0].is_ready);
    assert!(state.users[0].is_controller);
    assert!(!state.playback.can_toggle_pause);
    assert!(!state.playback.can_seek);
    assert!(!state.playback.can_manage_playlist);
    assert_eq!(state.playlist.len(), 1);
    assert_eq!(state.chat.len(), 1);
}

#[test]
fn main_window_shell_state_uses_legacy_chat_output_default() {
    let state = MainWindowShellState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert_eq!(state.chat.len(), 1);
    assert_eq!(state.chat[0].message, "Chat pane ready");

    let explicit_false = MainWindowShellState::from_stored_settings(&StoredClientSettingsMvp {
        chat_output_enabled: Some(false),
        ..StoredClientSettingsMvp::default()
    });
    assert!(explicit_false.chat.is_empty());
}

#[test]
fn menu_dialog_shell_state_uses_settings_for_enabled_actions_and_prompts() {
    let state = MenuDialogShellState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        shared_playlist_enabled: Some(true),
        chat_output_enabled: Some(true),
        only_switch_to_trusted_domains: Some(true),
        check_for_updates_automatically: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    let file = state
        .sections
        .iter()
        .find(|section| section.title == "File")
        .expect("file section should exist");
    assert!(
        file.actions
            .iter()
            .find(|item| item.label == "Open Media File")
            .is_some_and(|item| !item.enabled)
    );

    let playback = state
        .sections
        .iter()
        .find(|section| section.title == "Playback")
        .expect("playback section should exist");
    assert!(playback.actions.iter().all(|item| !item.enabled));

    let window = state
        .sections
        .iter()
        .find(|section| section.title == "Window")
        .expect("window section should exist");
    assert_eq!(window.actions.len(), 1);
    assert!(window.actions.iter().all(|item| item.enabled));

    assert!(state.tls_prompt_expected);
    assert!(!state.update_notice_expected);
    assert!(state.about_dialog_available);
}

#[test]
fn menu_dialog_shell_state_does_not_expose_chat_visibility_without_view_state() {
    let state = MenuDialogShellState::from_stored_settings(&StoredClientSettingsMvp::default());
    assert!(state.action(MenuActionId::TogglePlaybackButtons).is_some());
    assert!(
        state
            .sections
            .iter()
            .flat_map(|section| &section.actions)
            .all(|action| action.label != "Show Chat")
    );

    let explicit_false = MenuDialogShellState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(false),
        chat_output_enabled: Some(false),
        ..StoredClientSettingsMvp::default()
    });
    assert_eq!(state.sections, explicit_false.sections);
}

#[test]
fn public_server_browser_shell_state_uses_stored_server_entries() {
    let state = PublicServerBrowserShellState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![
            ("Primary".to_owned(), "syncplay.pl:8999".to_owned()),
            ("Backup".to_owned(), "syncplay.example:8995".to_owned()),
        ]),
        ..StoredClientSettingsMvp::default()
    });

    assert_eq!(state.servers.len(), 2);
    assert!(state.can_connect);
    assert!(state.can_refresh);
    assert!(state.can_add_custom_server);
    assert_eq!(state.servers[0].label, "Primary");
    assert!(state.servers[0].is_selected);
    assert!(!state.servers[1].is_selected);
}

#[test]
fn media_search_workflow_shell_state_uses_stored_directories_and_timing() {
    let state = MediaSearchWorkflowShellState::from_stored_settings(&StoredClientSettingsMvp {
        media_search_directories: Some(vec!["C:/Media".to_owned(), "D:/Archive".to_owned()]),
        folder_search_first_file_timeout_seconds: Some(1.5),
        folder_search_timeout_seconds: Some(5.0),
        folder_search_double_check_interval_seconds: Some(0.25),
        folder_search_warning_threshold_seconds: Some(2.0),
        ..StoredClientSettingsMvp::default()
    });

    assert_eq!(state.directories.len(), 2);
    assert!(state.can_browse_directories);
    assert!(state.can_search_missing_media);
    assert_eq!(state.first_file_timeout_seconds, Some(1.5));
    assert_eq!(state.search_timeout_seconds, Some(5.0));
    assert_eq!(state.double_check_interval_seconds, Some(0.25));
    assert_eq!(state.warning_threshold_seconds, Some(2.0));
}

#[test]
fn configuration_dialog_uses_parseable_numeric_text_for_loaded_thresholds() {
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        rewind_threshold_seconds: Some(1.25),
        fastforward_threshold_seconds: Some(3.5),
        slowdown_threshold_seconds: Some(2.25),
        folder_search_first_file_timeout_seconds: Some(3.0),
        folder_search_timeout_seconds: Some(30.0),
        folder_search_double_check_interval_seconds: Some(2.5),
        folder_search_warning_threshold_seconds: Some(7.5),
        chat_input_relative_font_size: Some(24),
        chat_output_relative_font_size: Some(26),
        notification_timeout_seconds: Some(3),
        ..StoredClientSettingsMvp::default()
    });

    assert_eq!(
        state
            .configuration
            .control_value(SettingId::SyncRewindThreshold),
        Some("1.25")
    );
    assert_eq!(
        state
            .configuration
            .control_value(SettingId::MediaLibraryFirstFileTimeout),
        Some("3")
    );
    assert_eq!(
        state
            .configuration
            .control_value(SettingId::MediaLibrarySearchTimeout),
        Some("30")
    );
    assert_eq!(
        state
            .configuration
            .control_value(SettingId::MediaLibraryDoubleCheckInterval),
        Some("2.5")
    );
    assert_eq!(
        state
            .configuration
            .control_value(SettingId::MediaLibraryWarningThreshold),
        Some("7.5")
    );
    assert_eq!(
        state
            .configuration
            .control_value(SettingId::ChatInputFontSize),
        Some("24")
    );
    assert_eq!(
        state
            .configuration
            .control_value(SettingId::ChatOutputFontSize),
        Some("26")
    );
    assert_eq!(
        state
            .configuration
            .control_value(SettingId::OsdNotificationTimeout),
        Some("3")
    );
    assert!(state.validation.issues.is_empty());
    assert!(!state.commands.can_save_configuration);
}

#[test]
fn configuration_validation_flags_invalid_chat_mode_controls() {
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ChatInputPosition,
        value: "Sideways".to_owned().into(),
    }));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ChatInputFontSize,
        value: "0".to_owned().into(),
    }));

    assert_eq!(state.validation.issues.len(), 2);
    assert!(state.validation.issues.iter().any(|issue| {
        issue.setting_id == Some(SettingId::ChatInputPosition)
            && issue.scope == "Chat"
            && issue.label == "Input Position"
            && issue.message == "must be Top, Middle, or Bottom."
    }));
    assert!(state.validation.issues.iter().any(|issue| {
        issue.setting_id == Some(SettingId::ChatInputFontSize)
            && issue.scope == "Chat"
            && issue.label == "Input Font Size"
            && issue.message == "must be a positive integer."
    }));
    assert!(!state.commands.can_save_configuration);
}

#[test]
fn gui_shell_credential_state_and_actions_redact_debug_output() {
    let secret = "gui-shell-password-canary";
    let target = GuiSavedSessionConnectTarget {
        address: "sync.example:8999".to_owned(),
        username: "alice".to_owned(),
        room: "room".to_owned(),
        controlled_room_password_override: Some(secret.into()),
    };
    let edit = GuiControllerAuthEditSessionState {
        room_name: "room".to_owned(),
        password_buffer: secret.into(),
        is_dirty: true,
    };
    let action = GuiShellAction::RequestControllerAuth {
        room: "room".to_owned(),
        password: secret.into(),
    };

    for debug in [
        format!("{target:?}"),
        format!("{edit:?}"),
        format!("{action:?}"),
    ] {
        assert!(debug.contains(sorotte_secret::REDACTED_SECRET));
        assert!(!debug.contains(secret));
    }
}

#[test]
fn configuration_password_edit_values_redact_actions_controls_and_snapshots() {
    let secret = "configuration-password-debug-canary";
    let value = GuiConfigurationTextValue::for_control(GuiDialogControlKind::PasswordInput, secret);
    let control = GuiDialogControl {
        id: SettingId::ConnectionServerPassword,
        label: "Server Password",
        kind: GuiDialogControlKind::PasswordInput,
        value: secret.to_owned(),
    };
    let edit = GuiTextEditSessionState {
        id: SettingId::ConnectionServerPassword,
        buffer: value.clone(),
        is_dirty: true,
    };
    let snapshot = GuiTextEditSessionRuntimeSnapshot {
        setting_id: SettingId::ConnectionServerPassword
            .automation_id()
            .to_owned(),
        buffer: value.clone(),
        is_dirty: true,
    };
    let actions = vec![
        GuiShellAction::UpdateConfigurationTextEdit(value.clone()),
        GuiShellAction::EditConfigurationText {
            id: SettingId::ConnectionServerPassword,
            value,
        },
    ];

    for debug in [
        format!("{control:?}"),
        format!("{edit:?}"),
        format!("{snapshot:?}"),
        format!("{actions:?}"),
    ] {
        assert!(debug.contains(sorotte_secret::REDACTED_SECRET));
        assert!(!debug.contains(secret));
    }
}

#[test]
fn controlled_room_secret_stays_redacted_in_actions_and_chat_state_debug() {
    let secret = "controlled-room-debug-canary";
    let action = GuiShellAction::AnnounceControlledRoomCreated {
        room: "+movie-room".to_owned(),
        password: secret.into(),
    };
    let actions = vec![action.clone()];
    assert!(!format!("{actions:?}").contains(secret));

    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_output_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    assert!(state.apply(action));
    let chat = state
        .main_window
        .chat
        .last()
        .expect("controlled-room announcement should remain visible");
    assert!(chat.message.contains(secret));

    let row = MainWindowChatRow {
        sender: chat.sender.clone(),
        message: chat.message.clone(),
    };
    let snapshot = MainWindowRuntimeChatSnapshot {
        sender: chat.sender.clone(),
        message: chat.message.clone(),
    };
    let chat_panel = state.main_window_chat_panel();
    let mut preview_renderer = GuiWidgetTextPreviewRenderer::default();
    chat_panel.render_with(&mut preview_renderer);
    let preview = preview_renderer.finish();
    for debug in [
        format!("{row:?}"),
        format!("{snapshot:?}"),
        format!("{:?}", state.main_window),
        format!(
            "{:?}",
            MainWindowRuntimeSnapshot::from_shell_state(&state.main_window)
        ),
        format!("{chat_panel:?}"),
        preview,
    ] {
        assert!(debug.contains(sorotte_secret::REDACTED_SECRET));
        assert!(!debug.contains(secret));
    }
}

#[test]
fn media_url_and_playlist_edit_state_debug_redacts_tokenized_targets() {
    let secret = "https://media.example/item?token=edit-state-canary";
    let url_state = GuiUrlEditSessionState {
        buffer: secret.to_owned(),
        is_dirty: true,
    };
    let url_snapshot = GuiUrlEditSessionRuntimeSnapshot {
        buffer: secret.to_owned(),
        is_dirty: true,
    };
    let playlist_state = GuiPlaylistTextEditSessionState {
        buffer: secret.to_owned(),
        is_dirty: true,
    };
    let playlist_snapshot = GuiPlaylistTextEditSessionRuntimeSnapshot {
        buffer: secret.to_owned(),
        is_dirty: true,
    };

    for debug in [
        format!("{url_state:?}"),
        format!("{url_snapshot:?}"),
        format!("{playlist_state:?}"),
        format!("{playlist_snapshot:?}"),
    ] {
        assert!(debug.contains(sorotte_secret::REDACTED_SECRET));
        assert!(!debug.contains("edit-state-canary"));
    }
}
