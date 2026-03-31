use std::collections::BTreeMap;

use super::{
    FirstRunConfigurationDialogState, GuiCommandAvailabilityState, GuiCommandRuntimeSnapshot,
    GuiConfigurationDraftRuntimeSnapshot, GuiConfigurationRuntimeSnapshot, GuiConfigurationTab,
    GuiDialogControlKind, GuiDraftRuntimeSnapshot, GuiErrorRuntimeSnapshot,
    GuiFeedbackRuntimeSnapshot, GuiFocusedConfigurationControlRuntimeSnapshot,
    GuiInteractionRuntimeSnapshot, GuiMainWindowTab, GuiMainWindowUserEditSessionRuntimeSnapshot,
    GuiPendingOperationKind, GuiPublicServerEditSessionRuntimeSnapshot,
    GuiSavedConfigurationRuntimeSnapshot, GuiSelectionState, GuiShellAction, GuiShellModal,
    GuiShellView, GuiTextEditSessionRuntimeSnapshot, GuiTransientNotification,
    GuiTransientNotificationLevel, GuiValidationIssue, GuiWidgetKind, MainWindowPlaylistRow,
    MainWindowRuntimeChatSnapshot, MainWindowRuntimeSnapshot, MainWindowRuntimeUserSnapshot,
    MainWindowShellState, MediaSearchDirectoryRow, MediaSearchWorkflowShellState,
    MenuActionRuntimeOverride, MenuDialogRuntimeSnapshot, MenuDialogShellState,
    PublicServerBrowserRow, PublicServerBrowserShellState, SyncplayGuiRuntimeSnapshot,
    SyncplayGuiShellAppState, load_playlist_entries_from_path,
    playlist_entries_from_multiline_text, save_playlist_entries_to_path,
};

use crate::app::{
    GuiDroppedFilesTarget, GuiLaunchMode, GuiWidgetEguiRenderer,
    remote_services::{LegacyUpdateCheckResult, LegacyUpdateCheckStatus},
    testing::support::{TEST_USERNAME, test_temp_root},
};
use syncplay_client_app::app_boundary::state::{
    AutoplayThresholdOverride, StoredClientSettingsMvp,
};
use syncplay_client_core::{PrivacyMode, UnpauseActionMode};

#[path = "tests/command_tests.rs"]
mod command_tests;
#[path = "tests/main_window_playlist_tests.rs"]
mod main_window_playlist_tests;
#[path = "tests/menu_public_server_tests.rs"]
mod menu_public_server_tests;
#[path = "tests/runtime_snapshot_tests.rs"]
mod runtime_snapshot_tests;

#[test]
fn configuration_surface_defaults_to_first_run_mode() {
    let state =
        FirstRunConfigurationDialogState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert_eq!(state.launch_mode, GuiLaunchMode::FirstRun);
    assert_eq!(state.system.language_tag, "en");
    assert_eq!(state.readiness.unpause_action_label, "IfAlreadyReady");
    assert_eq!(state.readiness.autoplay_min_users_label, "app-default");
    assert_eq!(state.chat.chat_input_position_label, "Top");
    assert_eq!(state.chat.chat_output_mode_label, "Chatroom");
    assert_eq!(state.connection.public_server_count, 0);
}

#[test]
fn gui_shell_app_state_defaults_tabs_to_overview() {
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert_eq!(state.selected_main_window_tab, GuiMainWindowTab::Overview);
    assert_eq!(
        state.selected_configuration_tab,
        GuiConfigurationTab::Overview
    );
}

#[test]
fn gui_shell_app_state_auto_switches_tabs_for_owned_workflows_and_preserves_hidden_sessions() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        room: Some("+room:ABCDEF123456".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::BeginSharedPlaylistTextEdit));
    assert_eq!(state.selected_main_window_tab, GuiMainWindowTab::Playlist);
    assert!(state.playlist_text_edit_session.is_some());

    assert!(state.apply(GuiShellAction::SelectMainWindowTab(GuiMainWindowTab::Chat,)));
    assert!(state.playlist_text_edit_session.is_some());

    assert!(state.apply(GuiShellAction::BeginMediaUrlEdit));
    assert_eq!(state.selected_main_window_tab, GuiMainWindowTab::Playback);
    assert!(state.media_url_edit_session.is_some());

    assert!(state.apply(GuiShellAction::BeginRoomHistoryEdit));
    assert_eq!(
        state.selected_configuration_tab,
        GuiConfigurationTab::Connection
    );
    assert!(state.room_history_edit_session.is_some());

    assert!(state.apply(GuiShellAction::FocusConfigurationControl {
        section: "Privacy",
        label: "Trusted Domains",
    }));
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
        force_gui_prompt: Some(true),
        host: Some("syncplay.example".to_owned()),
        port: Some(8995),
        server_password: Some("secret".to_owned()),
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
    assert_eq!(state.readiness.unpause_action_label, "IfMinUsersReady");
    assert_eq!(state.readiness.autoplay_min_users_label, "3");
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
    assert!(
        window
            .actions
            .iter()
            .find(|item| item.label == "Show Chat")
            .is_some_and(|item| item.enabled)
    );
    assert!(
        window
            .actions
            .iter()
            .find(|item| item.label == "Show Playlist")
            .is_some_and(|item| item.enabled)
    );

    assert!(state.tls_prompt_expected);
    assert!(!state.update_notice_expected);
    assert!(state.about_dialog_available);
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
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
            .control_value("Desync", "Rewind Threshold"),
        Some("1.25")
    );
    assert_eq!(
        state
            .configuration
            .control_value("Media Search", "First File Timeout"),
        Some("3")
    );
    assert_eq!(
        state
            .configuration
            .control_value("Media Search", "Search Timeout"),
        Some("30")
    );
    assert_eq!(
        state
            .configuration
            .control_value("Media Search", "Double Check Interval"),
        Some("2.5")
    );
    assert_eq!(
        state
            .configuration
            .control_value("Media Search", "Warning Threshold"),
        Some("7.5")
    );
    assert_eq!(
        state.configuration.control_value("Chat", "Input Font Size"),
        Some("24")
    );
    assert_eq!(
        state
            .configuration
            .control_value("Chat", "Output Font Size"),
        Some("26")
    );
    assert_eq!(
        state
            .configuration
            .control_value("OSD", "Notification Timeout"),
        Some("3")
    );
    assert!(state.validation.issues.is_empty());
    assert!(state.commands.can_save_configuration);
}

#[test]
fn configuration_validation_flags_invalid_chat_mode_controls() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Chat",
        label: "Input Position",
        value: "Sideways".to_owned(),
    }));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Chat",
        label: "Input Font Size",
        value: "0".to_owned(),
    }));

    assert_eq!(state.validation.issues.len(), 2);
    assert!(state.validation.issues.iter().any(|issue| {
        issue.scope == "Chat"
            && issue.label == "Input Position"
            && issue.message == "must be Top, Middle, or Bottom."
    }));
    assert!(state.validation.issues.iter().any(|issue| {
        issue.scope == "Chat"
            && issue.label == "Input Font Size"
            && issue.message == "must be a positive integer."
    }));
    assert!(!state.commands.can_save_configuration);
}
