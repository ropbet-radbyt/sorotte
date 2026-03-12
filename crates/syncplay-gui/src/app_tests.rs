use std::path::PathBuf;
use syncplay_client_app::app_boundary::state::AutoplayThresholdOverride;
use syncplay_client_core::{PrivacyMode, UnpauseActionMode};

use super::{
    FirstRunConfigurationDialogDraft, FirstRunConfigurationDialogState, GuiAppHost,
    GuiCommandAvailabilityState, GuiCommandRuntimeSnapshot, GuiConfigurationDraftRuntimeSnapshot,
    GuiConfigurationRuntimeSnapshot, GuiDialogControlKind, GuiDraftRuntimeSnapshot,
    GuiDroppedFilesRequest, GuiDroppedFilesTarget, GuiEframeNativeHost, GuiErrorRuntimeSnapshot,
    GuiFeedbackRuntimeSnapshot, GuiFocusedConfigurationControlRuntimeSnapshot,
    GuiInteractionRuntimeSnapshot, GuiLaunchMode, GuiMainWindowUserEditSessionRuntimeSnapshot,
    GuiNativeApp, GuiNativeRuntimeBridge, GuiPendingCompletionRequest, GuiPendingOperationKind,
    GuiPreviewRuntimeBridge, GuiPublicServerEditSessionRuntimeSnapshot, GuiQueuedRuntimeBridge,
    GuiRuntimeRequest, GuiSavedConfigurationRuntimeSnapshot, GuiSelectionState, GuiShellAction,
    GuiShellModal, GuiShellView, GuiTextEditSessionRuntimeSnapshot, GuiTextPreviewHost,
    GuiTransientNotification, GuiTransientNotificationLevel, GuiValidationIssue,
    GuiWidgetEguiRenderer, GuiWidgetKind, GuiWidgetNode, GuiWidgetRenderer,
    GuiWidgetTextPreviewRenderer, MainWindowPlaylistRow, MainWindowRuntimeChatSnapshot,
    MainWindowRuntimeRoomSnapshot, MainWindowRuntimeSnapshot, MainWindowRuntimeUserSnapshot,
    MainWindowShellState, MediaSearchDirectoryRow, MediaSearchWorkflowShellState,
    MenuActionRuntimeOverride, MenuDialogRuntimeSnapshot, MenuDialogShellState,
    PublicServerBrowserRow, PublicServerBrowserShellState, SyncplayGuiRuntimeSnapshot,
    SyncplayGuiShellAppState, run_gui_host, shell_widget_preview, startup_notice, startup_preview,
};
use syncplay_client_app::app_boundary::state::StoredClientSettingsMvp;

const TEST_USERNAME: &str = "test-user";

fn browser_runtime_user(
    username: &str,
    room_name: &str,
    is_self: bool,
    is_ready: bool,
    is_controller: bool,
) -> MainWindowRuntimeUserSnapshot {
    MainWindowRuntimeUserSnapshot {
        username: username.to_owned(),
        room_name: room_name.to_owned(),
        is_self,
        is_ready,
        is_controller,
        file_is_trusted: true,
        ..Default::default()
    }
}

fn browser_runtime_rooms(
    room_name: &str,
    is_controlled: bool,
    has_named_users: bool,
) -> Vec<MainWindowRuntimeRoomSnapshot> {
    vec![MainWindowRuntimeRoomSnapshot {
        room_name: room_name.to_owned(),
        is_controlled,
        has_named_users,
    }]
}

fn test_default_syncplay_config_env_root() -> std::path::PathBuf {
    if cfg!(windows) {
        std::path::PathBuf::from("test-appdata-root")
    } else {
        std::path::PathBuf::from("test-home-root")
    }
}

fn test_default_syncplay_config_root() -> std::path::PathBuf {
    if cfg!(windows) {
        test_default_syncplay_config_env_root()
    } else {
        test_default_syncplay_config_env_root().join(".config")
    }
}

fn test_default_syncplay_config_target() -> std::path::PathBuf {
    test_default_syncplay_config_root().join("syncplay.ini")
}

fn test_temp_root(label: &str) -> std::path::PathBuf {
    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "syncplay-gui-{label}-{}-{unique_suffix}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("test temp root should be created");
    root
}

fn pump_and_apply_runtime_owner_actions(
    owner: &mut super::GuiPersistedConfigRuntimeOwner,
    handle: &super::GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
) -> Vec<GuiShellAction> {
    super::GuiQueuedRuntimeOwner::pump(owner, handle, state);
    let actions = handle.drain_actions();
    for action in actions.iter().cloned() {
        assert!(state.apply(action));
    }
    actions
}

fn pump_and_apply_runtime_owner_actions_until<P>(
    owner: &mut super::GuiPersistedConfigRuntimeOwner,
    handle: &super::GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
    timeout: std::time::Duration,
    predicate: P,
    context: &str,
) -> Vec<GuiShellAction>
where
    P: Fn(&SyncplayGuiShellAppState) -> bool,
{
    let deadline = std::time::Instant::now() + timeout;
    let mut all_actions = Vec::new();
    loop {
        let actions = pump_and_apply_runtime_owner_actions(owner, handle, state);
        all_actions.extend(actions);
        if predicate(state) {
            return all_actions;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {context}"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn configuration_surface_defaults_to_first_run_mode() {
    let state =
        FirstRunConfigurationDialogState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert_eq!(state.launch_mode, GuiLaunchMode::FirstRun);
    assert_eq!(state.system.language_tag, "en");
    assert_eq!(state.readiness.unpause_action_label, "IfAlreadyReady");
    assert_eq!(state.readiness.autoplay_min_users_label, "app-default");
    assert_eq!(state.connection.public_server_count, 0);
}

#[test]
fn configuration_surface_maps_existing_stored_settings_into_sections() {
    let state = FirstRunConfigurationDialogState::from_stored_settings(&StoredClientSettingsMvp {
        language: Some("pt-br".to_owned()),
        check_for_updates_automatically: Some(true),
        host: Some("syncplay.example".to_owned()),
        port: Some(8995),
        server_password: Some("secret".to_owned()),
        username: Some(TEST_USERNAME.to_owned()),
        room: Some("room-a".to_owned()),
        room_list: Some(vec!["room-a".to_owned(), "room-b".to_owned()]),
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
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
        show_osd: Some(true),
        chat_input_enabled: Some(true),
        chat_input_font_family: Some("Consolas".to_owned()),
        chat_direct_input: Some(true),
        chat_output_enabled: Some(true),
        chat_output_font_family: Some("Segoe UI".to_owned()),
        chat_move_osd: Some(true),
        chat_max_lines: Some(7),
        show_same_room_osd: Some(true),
        show_osd_warnings: Some(true),
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
    assert_eq!(state.connection.room_history_count, 2);
    assert_eq!(state.readiness.unpause_action_label, "IfMinUsersReady");
    assert_eq!(state.readiness.autoplay_min_users_label, "3");
    assert_eq!(state.privacy.filename_privacy_mode_label, "SendHashed");
    assert_eq!(state.privacy.filesize_privacy_mode_label, "DoNotSend");
    assert_eq!(
        state.privacy.trusted_domains_label,
        "example.org; syncplay.pl"
    );
    assert_eq!(state.privacy.trusted_domain_count, 2);
    assert_eq!(state.media_search.media_directory_count, 2);
    assert_eq!(
        state.chat.chat_input_font_family.as_deref(),
        Some("Consolas")
    );
    assert_eq!(
        state.chat.chat_output_font_family.as_deref(),
        Some("Segoe UI")
    );
    assert!(state.osd.show_contact_info);
    assert!(state.system.check_for_updates_automatically);
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
        control.label == "Trusted Domains" && control.kind == GuiDialogControlKind::TextInput
    }));
}

#[test]
fn configuration_draft_applies_edits_and_round_trips_to_stored_settings() {
    let mut draft =
        FirstRunConfigurationDialogDraft::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(draft.apply_text_value("Connection", "Host", "syncplay.example"));
    assert!(draft.apply_text_value("Connection", "Port", "8995"));
    assert!(draft.apply_text_value("Connection", "Server Password", "secret"));
    assert!(draft.apply_bool_value("Readiness", "Autoplay", true));
    assert!(draft.apply_text_value("Readiness", "Unpause Action", "Always"));
    assert!(draft.apply_text_value("Readiness", "Autoplay Min Users", "3"));
    assert!(draft.apply_text_value(
        "Privacy",
        "Trusted Domains",
        "youtube.com; *.example.com/videos"
    ));
    assert!(draft.apply_text_value("System", "Language", "pt-br"));

    let saved = draft.to_stored_settings();
    assert_eq!(saved.host.as_deref(), Some("syncplay.example"));
    assert_eq!(saved.port, Some(8995));
    assert_eq!(saved.server_password.as_deref(), Some("secret"));
    assert_eq!(saved.autoplay_initial_state, Some(true));
    assert_eq!(saved.unpause_action, Some(UnpauseActionMode::Always));
    assert_eq!(
        saved.autoplay_min_users,
        Some(AutoplayThresholdOverride::Set(3))
    );
    assert_eq!(
        saved.trusted_domains,
        Some(vec![
            "youtube.com".to_owned(),
            "*.example.com/videos".to_owned()
        ])
    );
    assert_eq!(saved.language.as_deref(), Some("pt_BR"));
    assert_eq!(
        draft.control_value("Privacy", "Trusted Domain Count"),
        Some("2")
    );
}

#[test]
fn configuration_draft_rejects_readonly_control_edits() {
    let mut draft =
        FirstRunConfigurationDialogDraft::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!draft.apply_text_value("Connection", "Public Servers", "5"));
    assert_eq!(draft.to_stored_settings().public_servers, None);
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
    assert!(state.users[0].is_ready);
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
fn gui_shell_app_state_only_enables_media_open_after_runtime_support_arrives() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    let initial_tree = state.shell_widget_tree();
    assert!(
        initial_tree
            .find("menus:action:0:0")
            .is_some_and(|node| !node.enabled)
    );
    assert!(
        initial_tree
            .find("shell:quick:open-media-file")
            .is_some_and(|node| !node.enabled)
    );
    assert!(!super::GuiDroppedFilesTarget::Playlist.load_into_shared_playlist(&state));

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "Room".to_owned(),
            shared_playlist_enabled: true,
            controlled_room_active: false,
            users: vec![MainWindowRuntimeUserSnapshot {
                username: TEST_USERNAME.to_owned(),
                is_self: true,
                is_ready: false,
                is_controller: false,
                ..Default::default()
            }],
            playlist: vec!["Episode 1".to_owned()],
            chat: Vec::new(),
            can_toggle_pause: false,
            can_seek: false,
            can_set_ready: false,
            can_manage_playlist: true,
            playback_paused: false,
            autoplay_active: false,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            ..Default::default()
        }
    )));

    let runtime_tree = state.shell_widget_tree();
    assert!(
        runtime_tree
            .find("menus:action:0:0")
            .is_some_and(|node| node.enabled)
    );
    assert!(
        runtime_tree
            .find("shell:quick:open-media-file")
            .is_some_and(|node| node.enabled)
    );
    assert!(super::GuiDroppedFilesTarget::Playlist.load_into_shared_playlist(&state));
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
    assert!(state.validation.issues.is_empty());
    assert!(state.commands.can_save_configuration);
}

#[test]
fn gui_shell_app_state_resyncs_surfaces_from_configuration_edits() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: Vec::new(),
            tls_prompt_expected: true,
            update_notice_expected: true,
            about_dialog_available: false,
        },
    )));
    assert!(state.apply(GuiShellAction::SelectMenuAction {
        section_index: 0,
        action_index: 0,
    }));
    assert!(state.apply(GuiShellAction::ApplyGuiCommandRuntimeSnapshot(
        GuiCommandRuntimeSnapshot {
            command_availability: GuiCommandAvailabilityState {
                can_save_configuration: false,
                can_reset_configuration: false,
                can_reload_configuration: false,
                can_connect_public_server: false,
                can_connect_saved_server: false,
                can_refresh_public_servers: false,
                can_disconnect_session: false,
                can_search_missing_media: false,
                can_toggle_pause: false,
                can_send_chat_message: false,
            },
            pending_operation: Some(GuiPendingOperationKind::RefreshPublicServers),
        },
    )));

    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        section: "Readiness",
        label: "Shared Playlists",
        value: true,
    }));

    assert!(state.main_window.shared_playlist_enabled);
    assert!(!state.main_window.playback.can_manage_playlist);
    let window = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Window")
        .expect("window section should exist");
    assert!(
        window
            .actions
            .iter()
            .find(|item| item.label == "Show Playlist")
            .is_some_and(|item| item.enabled)
    );
    assert!(state.menus.tls_prompt_expected);
    assert!(state.menus.update_notice_expected);
    assert!(!state.menus.about_dialog_available);
    assert_eq!(state.selection.selected_menu_action, Some((0, 1)));
    let file = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "File")
        .expect("file section should exist");
    assert!(
        file.actions
            .iter()
            .find(|item| item.label == "Open Media File")
            .is_some_and(|item| !item.enabled && !item.is_selected)
    );
    assert!(
        file.actions
            .iter()
            .find(|item| item.label == "Open Media Search")
            .is_some_and(|item| item.enabled && item.is_selected)
    );
    let playback = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Playback")
        .expect("playback section should exist");
    assert!(playback.actions.iter().all(|item| !item.enabled));
    let help = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Help")
        .expect("help section should exist");
    assert!(
        help.actions
            .iter()
            .find(|item| item.label == "About")
            .is_some_and(|item| !item.enabled)
    );
}

#[test]
fn gui_shell_app_state_preserves_runtime_main_window_surface_across_configuration_edits() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "RuntimeRoom".to_owned(),
            shared_playlist_enabled: true,
            controlled_room_active: false,
            users: vec![
                MainWindowRuntimeUserSnapshot {
                    username: "alice".to_owned(),
                    is_self: true,
                    is_ready: true,
                    is_controller: false,
                    ..Default::default()
                },
                MainWindowRuntimeUserSnapshot {
                    username: "bob".to_owned(),
                    is_self: false,
                    is_ready: false,
                    is_controller: false,
                    ..Default::default()
                },
            ],
            playlist: vec!["Episode 1".to_owned()],
            chat: vec![MainWindowRuntimeChatSnapshot {
                sender: "bob".to_owned(),
                message: "synced".to_owned(),
            }],
            can_toggle_pause: true,
            can_seek: true,
            can_set_ready: false,
            can_manage_playlist: true,
            playback_paused: true,
            autoplay_active: true,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            ..Default::default()
        },
    )));
    assert!(state.apply(GuiShellAction::ApplyGuiCommandRuntimeSnapshot(
        GuiCommandRuntimeSnapshot {
            command_availability: GuiCommandAvailabilityState {
                can_save_configuration: true,
                can_reset_configuration: false,
                can_reload_configuration: true,
                can_connect_public_server: false,
                can_connect_saved_server: false,
                can_refresh_public_servers: true,
                can_disconnect_session: false,
                can_search_missing_media: false,
                can_toggle_pause: true,
                can_send_chat_message: false,
            },
            pending_operation: None,
        },
    )));

    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        section: "Chat",
        label: "Chat Input",
        value: true,
    }));

    assert_eq!(state.main_window.room_name, "RuntimeRoom");
    assert_eq!(state.main_window.users.len(), 2);
    assert_eq!(state.main_window.users[1].username, "bob");
    assert_eq!(state.main_window.playlist[0].label, "Episode 1");
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("synced")
    );
    assert!(state.main_window.playback.can_toggle_pause);
    assert!(state.main_window.playback.can_seek);
    assert!(state.main_window.playback_paused);
    assert!(state.main_window.autoplay_active);
    assert!(state.commands.can_send_chat_message);
}

#[test]
fn gui_shell_app_state_merges_runtime_main_window_users_with_configuration_room_edits() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "(no room joined)".to_owned(),
            shared_playlist_enabled: false,
            controlled_room_active: false,
            users: vec![
                MainWindowRuntimeUserSnapshot {
                    username: "You".to_owned(),
                    is_self: true,
                    is_ready: false,
                    is_controller: false,
                    ..Default::default()
                },
                MainWindowRuntimeUserSnapshot {
                    username: "bob".to_owned(),
                    is_self: false,
                    is_ready: false,
                    is_controller: false,
                    ..Default::default()
                },
            ],
            playlist: Vec::new(),
            chat: Vec::new(),
            can_toggle_pause: false,
            can_seek: false,
            can_set_ready: true,
            can_manage_playlist: false,
            playback_paused: false,
            autoplay_active: false,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            ..Default::default()
        },
    )));

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Room",
        value: "MergedRoom".to_owned(),
    }));

    assert_eq!(state.main_window.room_name, "MergedRoom");
    assert_eq!(state.main_window.users.len(), 2);
    assert_eq!(state.main_window.users[1].username, "bob");
}

#[test]
fn gui_shell_app_state_preserves_connected_room_surface_across_configuration_room_edits() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "RuntimeRoom".to_owned(),
            shared_playlist_enabled: false,
            controlled_room_active: false,
            users: vec![
                MainWindowRuntimeUserSnapshot {
                    username: "You".to_owned(),
                    is_self: true,
                    is_ready: false,
                    is_controller: false,
                    ..Default::default()
                },
                MainWindowRuntimeUserSnapshot {
                    username: "bob".to_owned(),
                    is_self: false,
                    is_ready: true,
                    is_controller: false,
                    ..Default::default()
                },
            ],
            playlist: Vec::new(),
            chat: Vec::new(),
            can_toggle_pause: false,
            can_seek: false,
            can_set_ready: true,
            can_manage_playlist: false,
            playback_paused: false,
            autoplay_active: false,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            ..Default::default()
        },
    )));
    assert!(state.apply(GuiShellAction::ApplyGuiCommandRuntimeSnapshot(
        GuiCommandRuntimeSnapshot {
            command_availability: GuiCommandAvailabilityState {
                can_save_configuration: true,
                can_reset_configuration: false,
                can_reload_configuration: true,
                can_connect_public_server: false,
                can_connect_saved_server: false,
                can_refresh_public_servers: true,
                can_disconnect_session: true,
                can_search_missing_media: false,
                can_toggle_pause: false,
                can_send_chat_message: false,
            },
            pending_operation: None,
        },
    )));

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Room",
        value: "DraftRoom".to_owned(),
    }));

    assert_eq!(state.main_window.room_name, "RuntimeRoom");
    assert_eq!(
        state.configuration.to_stored_settings().room.as_deref(),
        Some("DraftRoom")
    );
    assert_eq!(state.main_window.users.len(), 2);
    assert_eq!(state.main_window.users[1].username, "bob");
}

#[test]
fn gui_shell_app_state_merges_runtime_main_window_users_with_configuration_runtime_room_updates() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "(no room joined)".to_owned(),
            shared_playlist_enabled: false,
            controlled_room_active: false,
            users: vec![
                MainWindowRuntimeUserSnapshot {
                    username: "You".to_owned(),
                    is_self: true,
                    is_ready: false,
                    is_controller: false,
                    ..Default::default()
                },
                MainWindowRuntimeUserSnapshot {
                    username: "bob".to_owned(),
                    is_self: false,
                    is_ready: false,
                    is_controller: false,
                    ..Default::default()
                },
            ],
            playlist: Vec::new(),
            chat: Vec::new(),
            can_toggle_pause: false,
            can_seek: false,
            can_set_ready: true,
            can_manage_playlist: false,
            playback_paused: false,
            autoplay_active: false,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            ..Default::default()
        },
    )));

    let mut draft = state.configuration.to_stored_settings();
    draft.room = Some("RuntimeMergedRoom".to_owned());
    let saved = state.saved_configuration.clone();

    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft,
                saved_settings: saved,
            }
        ))
    );

    assert_eq!(state.main_window.room_name, "RuntimeMergedRoom");
    assert_eq!(state.main_window.users.len(), 2);
    assert_eq!(state.main_window.users[1].username, "bob");
}

#[test]
fn gui_shell_app_state_switches_views_and_tracks_modal_lifecycle() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::MainWindow)));
    assert_eq!(state.active_view, GuiShellView::MainWindow);
    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::MenusAndDialogs)));
    assert_eq!(state.active_view, GuiShellView::MenusAndDialogs);
    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::PublicServers)));
    assert_eq!(state.active_view, GuiShellView::PublicServers);
    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::MediaSearch)));
    assert_eq!(state.active_view, GuiShellView::MediaSearch);

    assert!(state.apply(GuiShellAction::OpenModal(GuiShellModal::About)));
    assert_eq!(state.open_modal, Some(GuiShellModal::About));
    assert!(state.apply(GuiShellAction::OpenModal(GuiShellModal::UpdateNotice)));
    assert_eq!(state.open_modal, Some(GuiShellModal::UpdateNotice));
    assert!(state.apply(GuiShellAction::OpenModal(
        GuiShellModal::TlsCertificatePrompt
    )));
    assert_eq!(state.open_modal, Some(GuiShellModal::TlsCertificatePrompt));

    assert!(state.apply(GuiShellAction::CloseModal));
    assert_eq!(state.open_modal, None);
    assert!(!state.apply(GuiShellAction::CloseModal));
}

#[test]
fn gui_shell_app_state_announces_menu_and_dialog_events() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::AnnounceTlsCertificatePromptRequired));
    assert!(state.menus.tls_prompt_expected);
    assert_eq!(state.open_modal, Some(GuiShellModal::TlsCertificatePrompt));
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("TLS certificate prompt opened.")
    );

    assert!(state.apply(GuiShellAction::AnnounceUpdateNoticeAvailable));
    assert!(state.menus.update_notice_expected);
    assert_eq!(state.open_modal, Some(GuiShellModal::UpdateNotice));

    assert!(state.apply(GuiShellAction::AnnounceAboutDialogRequested));
    assert_eq!(state.open_modal, Some(GuiShellModal::About));

    assert!(state.apply(GuiShellAction::AnnounceHelpRequested));
    assert_eq!(state.active_view, GuiShellView::MenusAndDialogs);
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Help opened.")
    );
}

#[test]
fn gui_shell_app_state_dismisses_update_notice_and_completes_tls_prompt() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::AnnounceUpdateNoticeAvailable));
    assert!(state.apply(GuiShellAction::DismissUpdateNotice));
    assert!(!state.menus.update_notice_expected);
    assert_eq!(state.open_modal, None);
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Update notice dismissed.")
    );

    assert!(state.apply(GuiShellAction::AnnounceTlsCertificatePromptRequired));
    assert!(state.apply(GuiShellAction::CloseModal));
    assert!(state.menus.tls_prompt_expected);
    assert_eq!(state.open_modal, None);

    assert!(state.apply(GuiShellAction::TrustTlsCertificatePrompt));
    assert!(!state.menus.tls_prompt_expected);
    assert_eq!(state.open_modal, None);
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("TLS certificate trusted for this session.")
    );
}

#[test]
fn gui_shell_app_state_applies_user_initiated_update_check_results() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::ApplyUpdateCheckResult(
        super::remote_services::LegacyUpdateCheckResult {
            status: super::remote_services::LegacyUpdateCheckStatus::UpdateAvailable,
            message: "A new version of Syncplay is available.".to_owned(),
            url: Some("https://syncplay.pl/download/".to_owned()),
            public_servers: Some(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned(),)]),
            checked_at_utc: "2026-03-08 09:10:11.123".to_owned(),
            user_initiated: true,
        }
    )));

    assert!(state.menus.update_notice_expected);
    assert_eq!(state.open_modal, Some(GuiShellModal::UpdateNotice));
    assert_eq!(
        state.update_check.message.as_deref(),
        Some("A new version of Syncplay is available.")
    );
    assert_eq!(
        state
            .configuration
            .to_stored_settings()
            .last_checked_for_updates
            .as_deref(),
        Some("2026-03-08 09:10:11.123")
    );
    assert_eq!(state.public_servers.servers.len(), 1);
    assert_eq!(
        state
            .public_servers
            .servers
            .first()
            .map(|row| (row.label.as_str(), row.address.as_str())),
        Some(("Primary", "syncplay.pl:8999"))
    );
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("A new version of Syncplay is available.")
    );
}

#[test]
fn gui_shell_app_state_applies_automatic_update_check_results_without_modal_when_up_to_date() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::ApplyUpdateCheckResult(
        super::remote_services::LegacyUpdateCheckResult {
            status: super::remote_services::LegacyUpdateCheckStatus::UpToDate,
            message: "Syncplay is up to date".to_owned(),
            url: None,
            public_servers: None,
            checked_at_utc: "2026-03-08 09:10:11.123".to_owned(),
            user_initiated: false,
        }
    )));

    assert!(!state.menus.update_notice_expected);
    assert_eq!(state.open_modal, None);
    assert_eq!(
        state.update_check.message.as_deref(),
        Some("Syncplay is up to date")
    );
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Syncplay is up to date")
    );
}

#[test]
fn gui_shell_app_state_auto_opens_new_runtime_prompt_flags() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: Vec::new(),
            tls_prompt_expected: false,
            update_notice_expected: true,
            about_dialog_available: true,
        },
    )));
    assert_eq!(state.open_modal, Some(GuiShellModal::UpdateNotice));

    assert!(state.apply(GuiShellAction::DismissUpdateNotice));
    assert_eq!(state.open_modal, None);

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: Vec::new(),
            tls_prompt_expected: true,
            update_notice_expected: false,
            about_dialog_available: true,
        },
    )));
    assert_eq!(state.open_modal, Some(GuiShellModal::TlsCertificatePrompt));
}

#[test]
fn gui_shell_app_state_applies_menu_dialog_runtime_snapshots() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SelectMenuAction {
        section_index: 1,
        action_index: 0,
    }));
    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![
                MenuActionRuntimeOverride {
                    section_title: "Playback",
                    action_label: "Toggle Pause",
                    enabled: false,
                },
                MenuActionRuntimeOverride {
                    section_title: "Window",
                    action_label: "Show Chat",
                    enabled: true,
                },
                MenuActionRuntimeOverride {
                    section_title: "Help",
                    action_label: "Check for Updates",
                    enabled: false,
                },
            ],
            tls_prompt_expected: true,
            update_notice_expected: false,
            about_dialog_available: false,
        },
    )));

    let playback = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Playback")
        .expect("playback section should exist");
    assert!(
        playback
            .actions
            .iter()
            .find(|action| action.label == "Toggle Pause")
            .is_some_and(|action| !action.enabled && !action.is_selected)
    );
    assert_eq!(state.selection.selected_menu_action, Some((0, 1)));
    assert!(
        playback
            .actions
            .iter()
            .find(|action| action.label == "Seek")
            .is_some_and(|action| !action.enabled && !action.is_selected)
    );
    let window = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Window")
        .expect("window section should exist");
    assert!(
        window
            .actions
            .iter()
            .find(|action| action.label == "Show Chat")
            .is_some_and(|action| action.enabled)
    );
    assert!(state.menus.tls_prompt_expected);
    assert!(!state.menus.update_notice_expected);
    assert!(!state.menus.about_dialog_available);
    let help = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Help")
        .expect("help section should exist");
    assert!(
        help.actions
            .iter()
            .find(|action| action.label == "About")
            .is_some_and(|action| !action.enabled)
    );
    assert!(state.notifications.is_empty());
}

#[test]
fn gui_shell_app_state_rejects_invalid_menu_dialog_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Invalid",
                action_label: "Missing",
                enabled: true,
            }],
            tls_prompt_expected: false,
            update_notice_expected: false,
            about_dialog_available: true,
        },
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No menu action exists for 'Invalid / Missing' in the runtime snapshot.")
    );
}

#[test]
fn gui_shell_app_state_applies_gui_feedback_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Port",
        value: "70000".to_owned(),
    }));
    assert!(state.apply(GuiShellAction::ApplyGuiFeedbackRuntimeSnapshot(
        GuiFeedbackRuntimeSnapshot {
            validation_issues: vec![GuiValidationIssue {
                scope: "Runtime".to_owned(),
                label: "Sync".to_owned(),
                message: "Server health degraded.".to_owned(),
            }],
            notifications: vec![
                GuiTransientNotification {
                    level: GuiTransientNotificationLevel::Warning,
                    message: "Server warning broadcast.".to_owned(),
                },
                GuiTransientNotification {
                    level: GuiTransientNotificationLevel::Info,
                    message: "Server status feed refreshed.".to_owned(),
                },
            ],
        },
    )));

    assert_eq!(state.validation.last_action_error, None);
    assert_eq!(state.validation.issues.len(), 2);
    assert!(
        state
            .validation
            .issues
            .iter()
            .any(|issue| issue.scope == "Connection" && issue.label == "Port")
    );
    assert!(state.validation.issues.iter().any(|issue| {
        issue.scope == "Runtime"
            && issue.label == "Sync"
            && issue.message == "Server health degraded."
    }));
    assert_eq!(state.notifications.len(), 2);
    assert_eq!(
        state.notifications[0].message.as_str(),
        "Server warning broadcast."
    );
    assert_eq!(
        state.notifications[1].message.as_str(),
        "Server status feed refreshed."
    );

    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::MainWindow)));
    assert!(
        state
            .validation
            .issues
            .iter()
            .any(|issue| issue.scope == "Runtime" && issue.label == "Sync")
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_gui_feedback_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(
        !state.apply(GuiShellAction::ApplyGuiFeedbackRuntimeSnapshot(
            GuiFeedbackRuntimeSnapshot {
                validation_issues: vec![GuiValidationIssue {
                    scope: "   ".to_owned(),
                    label: "Sync".to_owned(),
                    message: "Degraded.".to_owned(),
                }],
                notifications: Vec::new(),
            },
        ))
    );
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("GUI feedback runtime snapshots cannot contain empty validation scopes.")
    );

    assert!(
        !state.apply(GuiShellAction::ApplyGuiFeedbackRuntimeSnapshot(
            GuiFeedbackRuntimeSnapshot {
                validation_issues: Vec::new(),
                notifications: vec![GuiTransientNotification {
                    level: GuiTransientNotificationLevel::Warning,
                    message: "   ".to_owned(),
                }],
            },
        ))
    );
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("GUI feedback runtime snapshots cannot contain empty notification messages.")
    );
}

#[test]
fn gui_shell_app_state_applies_gui_error_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::ApplyGuiErrorRuntimeSnapshot(
        GuiErrorRuntimeSnapshot {
            last_action_error: Some("  runtime error  ".to_owned()),
        },
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("runtime error")
    );

    assert!(state.apply(GuiShellAction::ApplyGuiErrorRuntimeSnapshot(
        GuiErrorRuntimeSnapshot {
            last_action_error: None,
        },
    )));
    assert_eq!(state.validation.last_action_error, None);
}

#[test]
fn gui_shell_app_state_rejects_invalid_gui_error_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::ApplyGuiErrorRuntimeSnapshot(
        GuiErrorRuntimeSnapshot {
            last_action_error: Some("   ".to_owned()),
        },
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("GUI error runtime snapshots cannot contain an empty action error message.")
    );
}

#[test]
fn gui_shell_app_state_applies_gui_command_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::ApplyGuiCommandRuntimeSnapshot(
        GuiCommandRuntimeSnapshot {
            command_availability: GuiCommandAvailabilityState {
                can_save_configuration: false,
                can_reset_configuration: false,
                can_reload_configuration: false,
                can_connect_public_server: false,
                can_connect_saved_server: false,
                can_refresh_public_servers: false,
                can_disconnect_session: false,
                can_search_missing_media: false,
                can_toggle_pause: false,
                can_send_chat_message: false,
            },
            pending_operation: Some(GuiPendingOperationKind::RefreshPublicServers),
        },
    )));

    assert_eq!(
        state.pending_operation.as_ref().map(|item| item.kind),
        Some(GuiPendingOperationKind::RefreshPublicServers)
    );
    assert_eq!(
        state.commands,
        GuiCommandAvailabilityState {
            can_save_configuration: false,
            can_reset_configuration: false,
            can_reload_configuration: false,
            can_connect_public_server: false,
            can_connect_saved_server: false,
            can_refresh_public_servers: false,
            can_disconnect_session: false,
            can_search_missing_media: false,
            can_toggle_pause: false,
            can_send_chat_message: false,
        }
    );

    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::MainWindow)));
    assert_eq!(
        state.pending_operation.as_ref().map(|item| item.kind),
        Some(GuiPendingOperationKind::RefreshPublicServers)
    );
    assert!(!state.commands.can_refresh_public_servers);
    assert!(!state.commands.can_send_chat_message);
}

#[test]
fn gui_shell_app_state_keeps_unrelated_command_flags_live_when_runtime_overrides_chat_send() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    let mut command_availability = state.commands.clone();
    command_availability.can_send_chat_message = false;

    assert!(state.apply(GuiShellAction::ApplyGuiCommandRuntimeSnapshot(
        GuiCommandRuntimeSnapshot {
            command_availability,
            pending_operation: None,
        },
    )));
    assert!(!state.commands.can_send_chat_message);

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Port",
        value: "0".to_owned(),
    }));

    assert!(!state.commands.can_send_chat_message);
    assert!(!state.commands.can_save_configuration);
    assert!(state.commands.can_reset_configuration);
    assert!(state.commands.can_reload_configuration);
}

#[test]
fn gui_shell_app_state_clears_stale_runtime_chat_command_override_when_configuration_runtime_snapshot_catches_up()
 {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    let mut command_availability = state.commands.clone();
    command_availability.can_send_chat_message = false;

    assert!(state.apply(GuiShellAction::ApplyGuiCommandRuntimeSnapshot(
        GuiCommandRuntimeSnapshot {
            command_availability,
            pending_operation: None,
        },
    )));
    assert_eq!(
        state
            .runtime_command_availability_override
            .can_send_chat_message,
        Some(false)
    );

    let mut draft = state.configuration.to_stored_settings();
    draft.chat_input_enabled = Some(false);
    let saved = state.saved_configuration.clone();
    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft.clone(),
                saved_settings: saved.clone(),
            }
        ))
    );
    assert_eq!(
        state
            .runtime_command_availability_override
            .can_send_chat_message,
        None
    );

    draft.chat_input_enabled = Some(true);
    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft,
                saved_settings: saved,
            }
        ))
    );
    assert!(state.commands.can_send_chat_message);
}

#[test]
fn gui_shell_app_state_rejects_invalid_gui_command_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::ApplyGuiCommandRuntimeSnapshot(
        GuiCommandRuntimeSnapshot {
            command_availability: GuiCommandAvailabilityState {
                can_save_configuration: true,
                can_reset_configuration: false,
                can_reload_configuration: false,
                can_connect_public_server: false,
                can_connect_saved_server: false,
                can_refresh_public_servers: false,
                can_disconnect_session: false,
                can_search_missing_media: false,
                can_toggle_pause: false,
                can_send_chat_message: false,
            },
            pending_operation: Some(GuiPendingOperationKind::SaveConfiguration),
        },
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some(
            "GUI command runtime snapshots cannot leave command actions enabled while a pending operation is active."
        )
    );
}

#[test]
fn gui_shell_app_state_syncs_playback_menu_actions_from_gui_command_runtime_snapshots() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "Room".to_owned(),
            shared_playlist_enabled: true,
            controlled_room_active: false,
            users: vec![MainWindowRuntimeUserSnapshot {
                username: "alice".to_owned(),
                is_self: true,
                is_ready: false,
                is_controller: false,
                ..Default::default()
            }],
            playlist: vec!["One".to_owned()],
            chat: Vec::new(),
            can_toggle_pause: true,
            can_seek: true,
            can_set_ready: false,
            can_manage_playlist: true,
            playback_paused: false,
            autoplay_active: false,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            ..Default::default()
        },
    )));
    assert!(state.apply(GuiShellAction::SelectMenuAction {
        section_index: 1,
        action_index: 0,
    }));

    assert!(state.apply(GuiShellAction::ApplyGuiCommandRuntimeSnapshot(
        GuiCommandRuntimeSnapshot {
            command_availability: GuiCommandAvailabilityState {
                can_save_configuration: false,
                can_reset_configuration: false,
                can_reload_configuration: false,
                can_connect_public_server: false,
                can_connect_saved_server: false,
                can_refresh_public_servers: false,
                can_disconnect_session: false,
                can_search_missing_media: false,
                can_toggle_pause: false,
                can_send_chat_message: false,
            },
            pending_operation: Some(GuiPendingOperationKind::RefreshPublicServers),
        },
    )));

    assert_eq!(state.selection.selected_menu_action, Some((0, 1)));
    let file = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "File")
        .expect("file section should exist");
    assert!(
        file.actions
            .iter()
            .find(|action| action.label == "Open Media File")
            .is_some_and(|action| !action.enabled && !action.is_selected)
    );
    assert!(
        file.actions
            .iter()
            .find(|action| action.label == "Open Media Search")
            .is_some_and(|action| action.enabled && action.is_selected)
    );
    let playback = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Playback")
        .expect("playback section should exist");
    assert!(
        playback
            .actions
            .iter()
            .find(|action| action.label == "Toggle Pause")
            .is_some_and(|action| !action.enabled && !action.is_selected)
    );
    assert!(
        playback
            .actions
            .iter()
            .find(|action| action.label == "Seek")
            .is_some_and(|action| !action.enabled && !action.is_selected)
    );
    assert!(
        playback
            .actions
            .iter()
            .find(|action| action.label == "Playlist Actions")
            .is_some_and(|action| !action.enabled && !action.is_selected)
    );
}

#[test]
fn gui_shell_app_state_applies_gui_interaction_runtime_snapshots() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![("Alpha".to_owned(), "alpha.example:8999".to_owned())]),
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        shared_playlist_enabled: Some(true),
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "One".to_owned(),
            "Two".to_owned(),
        ]))
    );
    assert!(state.apply(GuiShellAction::AddMainWindowUser("Bob".to_owned())));

    assert!(
        state.apply(GuiShellAction::ApplyGuiInteractionRuntimeSnapshot(
            GuiInteractionRuntimeSnapshot {
                selection: GuiSelectionState {
                    selected_main_window_user: Some(1),
                    selected_main_window_playlist: Some(1),
                    selected_menu_action: Some((1, 1)),
                    selected_media_search_directory: Some(0),
                },
                selected_public_server_index: Some(0),
                focused_configuration_control: Some(
                    GuiFocusedConfigurationControlRuntimeSnapshot {
                        section: "Connection".to_owned(),
                        label: "Host".to_owned(),
                        activation_count: 3,
                    }
                ),
                public_server_edit_session: Some(GuiPublicServerEditSessionRuntimeSnapshot {
                    editing_index: Some(0),
                    label_buffer: "Alpha Edited".to_owned(),
                    address_buffer: "alpha.example:9999".to_owned(),
                    is_dirty: true,
                }),
                main_window_user_edit_session: Some(GuiMainWindowUserEditSessionRuntimeSnapshot {
                    editing_index: 1,
                    username_buffer: "Bob Runtime".to_owned(),
                    is_dirty: true,
                }),
                text_edit_session: Some(GuiTextEditSessionRuntimeSnapshot {
                    section: "Connection".to_owned(),
                    label: "Host".to_owned(),
                    buffer: "runtime.example".to_owned(),
                    is_dirty: true,
                }),
                playlist_text_edit_session: None,
                playlist_url_edit_session: None,
                media_url_edit_session: None,
            }
        ))
    );

    assert_eq!(state.selection.selected_main_window_user, Some(1));
    assert!(state.main_window.users[1].is_selected);
    assert_eq!(state.selection.selected_main_window_playlist, Some(1));
    assert!(state.main_window.playlist[1].is_selected);
    assert_eq!(state.selection.selected_menu_action, Some((0, 1)));
    assert!(state.menus.sections[0].actions[1].is_selected);
    assert_eq!(state.selection.selected_media_search_directory, Some(0));
    assert!(state.media_search.directories[0].is_selected);
    assert!(state.public_servers.servers[0].is_selected);
    assert_eq!(
        state.focused_configuration_control.as_ref().map(|focused| (
            focused.section,
            focused.label,
            focused.activation_count
        )),
        Some(("Connection", "Host", 3))
    );
    assert_eq!(
        state
            .public_server_edit_session
            .as_ref()
            .map(|session| session.editing_index),
        Some(Some(0))
    );
    assert_eq!(
        state
            .main_window_user_edit_session
            .as_ref()
            .map(|session| session.editing_index),
        Some(1)
    );
    assert_eq!(
        state
            .text_edit_session
            .as_ref()
            .map(|session| session.buffer.as_str()),
        Some("runtime.example")
    );

    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::MainWindow)));
    assert_eq!(state.selection.selected_main_window_user, Some(1));
    assert!(state.main_window.users[1].is_selected);
}

#[test]
fn gui_shell_app_state_normalizes_disabled_menu_selection_in_gui_interaction_runtime_snapshots() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Playback",
                action_label: "Seek",
                enabled: false,
            }],
            tls_prompt_expected: state.menus.tls_prompt_expected,
            update_notice_expected: state.menus.update_notice_expected,
            about_dialog_available: state.menus.about_dialog_available,
        },
    )));

    assert!(
        state.apply(GuiShellAction::ApplyGuiInteractionRuntimeSnapshot(
            GuiInteractionRuntimeSnapshot {
                selection: GuiSelectionState {
                    selected_main_window_user: state.selection.selected_main_window_user,
                    selected_main_window_playlist: state.selection.selected_main_window_playlist,
                    selected_menu_action: Some((1, 1)),
                    selected_media_search_directory: state
                        .selection
                        .selected_media_search_directory,
                },
                selected_public_server_index: state.selected_public_server_index(),
                focused_configuration_control: None,
                public_server_edit_session: None,
                main_window_user_edit_session: None,
                text_edit_session: None,
                playlist_text_edit_session: None,
                playlist_url_edit_session: None,
                media_url_edit_session: None,
            }
        ))
    );

    assert_eq!(state.selection.selected_menu_action, Some((0, 1)));
    assert!(
        state.menus.sections[0]
            .actions
            .iter()
            .find(|action| action.label == "Open Media Search")
            .is_some_and(|action| action.enabled && action.is_selected)
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_gui_interaction_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(
        !state.apply(GuiShellAction::ApplyGuiInteractionRuntimeSnapshot(
            GuiInteractionRuntimeSnapshot {
                selection: GuiSelectionState {
                    selected_main_window_user: Some(0),
                    selected_main_window_playlist: None,
                    selected_menu_action: None,
                    selected_media_search_directory: None,
                },
                selected_public_server_index: None,
                focused_configuration_control: Some(
                    GuiFocusedConfigurationControlRuntimeSnapshot {
                        section: "Connection".to_owned(),
                        label: "Missing".to_owned(),
                        activation_count: 0,
                    }
                ),
                public_server_edit_session: None,
                main_window_user_edit_session: None,
                text_edit_session: None,
                playlist_text_edit_session: None,
                playlist_url_edit_session: None,
                media_url_edit_session: None,
            }
        ))
    );
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("GUI interaction runtime snapshots cannot focus an unknown configuration control.")
    );
}

#[test]
fn gui_shell_app_state_applies_gui_draft_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::ApplyGuiDraftRuntimeSnapshot(
        GuiDraftRuntimeSnapshot {
            outgoing_chat_message: Some("  runtime draft  ".to_owned()),
        }
    )));
    assert_eq!(
        state.outgoing_chat_message.as_deref(),
        Some("runtime draft")
    );

    assert!(state.apply(GuiShellAction::BeginPendingOperation(
        GuiPendingOperationKind::SendChatMessage,
    )));
    assert!(state.apply(GuiShellAction::ApplyGuiDraftRuntimeSnapshot(
        GuiDraftRuntimeSnapshot {
            outgoing_chat_message: Some("updated runtime draft".to_owned()),
        }
    )));
    assert_eq!(
        state.outgoing_chat_message.as_deref(),
        Some("updated runtime draft")
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_gui_draft_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::BeginPendingOperation(
        GuiPendingOperationKind::SaveConfiguration,
    )));
    assert!(!state.apply(GuiShellAction::ApplyGuiDraftRuntimeSnapshot(
        GuiDraftRuntimeSnapshot {
            outgoing_chat_message: Some("runtime draft".to_owned()),
        }
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some(
            "GUI draft runtime snapshots cannot stage an outgoing chat message while a different pending operation is active."
        )
    );
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::SaveConfiguration)
    );
    assert_eq!(state.outgoing_chat_message, None);
}

#[test]
fn gui_shell_app_state_applies_gui_configuration_draft_runtime_snapshots() {
    let saved = StoredClientSettingsMvp {
        host: Some("saved.example".to_owned()),
        room: Some("SavedRoom".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&saved);

    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::MainWindow)));

    let replacement = StoredClientSettingsMvp {
        host: Some("draft.example".to_owned()),
        room: Some("DraftRoom".to_owned()),
        player_path: Some("mpv".to_owned()),
        public_servers: Some(vec![(
            "Primary".to_owned(),
            "syncplay.example:8999".to_owned(),
        )]),
        ..StoredClientSettingsMvp::default()
    };
    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationDraftRuntimeSnapshot(
            GuiConfigurationDraftRuntimeSnapshot {
                settings: replacement.clone(),
            }
        ))
    );

    assert_eq!(state.configuration.to_stored_settings(), replacement);
    assert_eq!(state.saved_configuration, saved);
    assert_eq!(state.active_view, GuiShellView::MainWindow);
    assert_eq!(state.main_window.room_name, "DraftRoom");
    assert_eq!(
        state
            .public_servers
            .servers
            .first()
            .map(|row| row.address.as_str()),
        Some("syncplay.example:8999")
    );
    assert!(state.commands.can_reset_configuration);
}

#[test]
fn gui_shell_app_state_rejects_invalid_gui_configuration_draft_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::BeginConfigurationReload));
    assert!(
        !state.apply(GuiShellAction::ApplyGuiConfigurationDraftRuntimeSnapshot(
            GuiConfigurationDraftRuntimeSnapshot {
                settings: StoredClientSettingsMvp {
                    host: Some("draft.example".to_owned()),
                    ..StoredClientSettingsMvp::default()
                },
            }
        ))
    );
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some(
            "GUI configuration draft runtime snapshots cannot apply while a configuration command is already in progress."
        )
    );
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::ReloadConfiguration)
    );
}

#[test]
fn gui_shell_app_state_applies_gui_saved_configuration_runtime_snapshots() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        host: Some("saved.example".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Host",
        value: "dirty.example".to_owned(),
    }));
    assert!(state.commands.can_reset_configuration);

    let replacement = StoredClientSettingsMvp {
        host: Some("dirty.example".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    assert!(
        state.apply(GuiShellAction::ApplyGuiSavedConfigurationRuntimeSnapshot(
            GuiSavedConfigurationRuntimeSnapshot {
                settings: replacement.clone(),
            }
        ))
    );

    assert_eq!(state.saved_configuration, replacement);
    assert_eq!(
        state.configuration.to_stored_settings().host.as_deref(),
        Some("dirty.example")
    );
    assert!(!state.commands.can_reset_configuration);
}

#[test]
fn gui_shell_app_state_rejects_invalid_gui_saved_configuration_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    assert!(
        !state.apply(GuiShellAction::ApplyGuiSavedConfigurationRuntimeSnapshot(
            GuiSavedConfigurationRuntimeSnapshot {
                settings: StoredClientSettingsMvp {
                    host: Some("saved.example".to_owned()),
                    ..StoredClientSettingsMvp::default()
                },
            }
        ))
    );
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some(
            "GUI saved-configuration runtime snapshots cannot apply while a configuration command is already in progress."
        )
    );
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::SaveConfiguration)
    );
}

#[test]
fn gui_shell_app_state_applies_gui_configuration_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::MainWindow)));
    assert!(state.apply(GuiShellAction::ApplyGuiCommandRuntimeSnapshot(
        GuiCommandRuntimeSnapshot {
            command_availability: GuiCommandAvailabilityState {
                can_save_configuration: true,
                can_reset_configuration: true,
                can_reload_configuration: true,
                can_connect_public_server: false,
                can_connect_saved_server: false,
                can_refresh_public_servers: true,
                can_disconnect_session: false,
                can_search_missing_media: false,
                can_toggle_pause: false,
                can_send_chat_message: false,
            },
            pending_operation: None,
        },
    )));

    let draft = StoredClientSettingsMvp {
        host: Some("draft.example".to_owned()),
        room: Some("DraftRoom".to_owned()),
        player_path: Some("mpv".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    let saved = StoredClientSettingsMvp {
        host: Some("saved.example".to_owned()),
        room: Some("SavedRoom".to_owned()),
        player_path: Some("mpv".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft.clone(),
                saved_settings: saved.clone(),
            }
        ))
    );

    assert_eq!(state.configuration.to_stored_settings(), draft);
    assert_eq!(state.saved_configuration, saved);
    assert_eq!(state.active_view, GuiShellView::MainWindow);
    assert_eq!(state.main_window.room_name, "DraftRoom");
    assert!(state.commands.can_reset_configuration);
    assert!(!state.commands.can_toggle_pause);
    let playback = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Playback")
        .expect("playback section should exist");
    assert!(
        playback
            .actions
            .iter()
            .find(|action| action.label == "Toggle Pause")
            .is_some_and(|action| !action.enabled)
    );
}

#[test]
fn gui_shell_app_state_preserves_runtime_main_window_surface_across_configuration_runtime_snapshots()
 {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "RuntimeRoom".to_owned(),
            shared_playlist_enabled: true,
            controlled_room_active: false,
            users: vec![
                MainWindowRuntimeUserSnapshot {
                    username: "alice".to_owned(),
                    is_self: true,
                    is_ready: true,
                    is_controller: false,
                    ..Default::default()
                },
                MainWindowRuntimeUserSnapshot {
                    username: "bob".to_owned(),
                    is_self: false,
                    is_ready: false,
                    is_controller: false,
                    ..Default::default()
                },
            ],
            playlist: vec!["Episode 1".to_owned()],
            chat: vec![MainWindowRuntimeChatSnapshot {
                sender: "bob".to_owned(),
                message: "synced".to_owned(),
            }],
            can_toggle_pause: true,
            can_seek: true,
            can_set_ready: false,
            can_manage_playlist: true,
            playback_paused: true,
            autoplay_active: true,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            ..Default::default()
        },
    )));
    assert!(state.apply(GuiShellAction::ApplyGuiCommandRuntimeSnapshot(
        GuiCommandRuntimeSnapshot {
            command_availability: GuiCommandAvailabilityState {
                can_save_configuration: true,
                can_reset_configuration: false,
                can_reload_configuration: true,
                can_connect_public_server: false,
                can_connect_saved_server: false,
                can_refresh_public_servers: true,
                can_disconnect_session: false,
                can_search_missing_media: false,
                can_toggle_pause: true,
                can_send_chat_message: false,
            },
            pending_operation: None,
        },
    )));

    let draft = StoredClientSettingsMvp {
        host: Some("draft.example".to_owned()),
        room: Some("DraftRoom".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    let saved = StoredClientSettingsMvp {
        host: Some("saved.example".to_owned()),
        room: Some("SavedRoom".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft.clone(),
                saved_settings: saved.clone(),
            }
        ))
    );

    assert_eq!(state.configuration.to_stored_settings(), draft);
    assert_eq!(state.saved_configuration, saved);
    assert_eq!(state.main_window.room_name, "RuntimeRoom");
    assert_eq!(state.main_window.users.len(), 2);
    assert_eq!(state.main_window.users[1].username, "bob");
    assert_eq!(state.main_window.playlist[0].label, "Episode 1");
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("synced")
    );
    assert!(state.main_window.playback.can_toggle_pause);
    assert!(state.main_window.playback.can_seek);
    assert!(state.main_window.playback_paused);
    assert!(state.main_window.autoplay_active);
}

#[test]
fn gui_shell_app_state_preserves_public_server_selection_across_configuration_edits() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![
            ("Alpha".to_owned(), "alpha.example:8999".to_owned()),
            ("Beta".to_owned(), "beta.example:8999".to_owned()),
        ]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SelectPublicServer(1)));
    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        section: "Chat",
        label: "Chat Input",
        value: true,
    }));

    assert_eq!(state.selected_public_server_index(), Some(1));
    assert!(!state.public_servers.servers[0].is_selected);
    assert!(state.public_servers.servers[1].is_selected);
    assert_eq!(state.public_servers.servers[1].address, "beta.example:8999");
}

#[test]
fn gui_shell_app_state_preserves_public_server_selection_across_configuration_runtime_snapshots() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![
            ("Alpha".to_owned(), "alpha.example:8999".to_owned()),
            ("Beta".to_owned(), "beta.example:8999".to_owned()),
        ]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SelectPublicServer(1)));

    let mut draft = state.configuration.to_stored_settings();
    draft.chat_input_enabled = Some(true);
    let mut saved = state.saved_configuration.clone();
    saved.chat_input_enabled = Some(false);

    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft.clone(),
                saved_settings: saved.clone(),
            }
        ))
    );

    assert_eq!(state.configuration.to_stored_settings(), draft);
    assert_eq!(state.saved_configuration, saved);
    assert_eq!(state.selected_public_server_index(), Some(1));
    assert!(!state.public_servers.servers[0].is_selected);
    assert!(state.public_servers.servers[1].is_selected);
    assert_eq!(state.public_servers.servers[1].address, "beta.example:8999");
}

#[test]
fn gui_shell_app_state_preserves_runtime_show_chat_override_across_configuration_edits() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Chat",
                enabled: false,
            }],
            tls_prompt_expected: false,
            update_notice_expected: false,
            about_dialog_available: true,
        }
    )));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Host",
        value: "syncplay.example".to_owned(),
    }));

    let window = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Window")
        .expect("window section should exist");
    assert!(
        window
            .actions
            .iter()
            .find(|action| action.label == "Show Chat")
            .is_some_and(|action| !action.enabled)
    );
}

#[test]
fn gui_shell_app_state_preserves_runtime_show_chat_override_across_configuration_runtime_snapshots()
{
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Chat",
                enabled: false,
            }],
            tls_prompt_expected: false,
            update_notice_expected: false,
            about_dialog_available: true,
        }
    )));

    let mut draft = state.configuration.to_stored_settings();
    draft.host = Some("draft.example".to_owned());
    let mut saved = state.saved_configuration.clone();
    saved.host = Some("saved.example".to_owned());

    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft.clone(),
                saved_settings: saved.clone(),
            }
        ))
    );

    assert_eq!(state.configuration.to_stored_settings(), draft);
    assert_eq!(state.saved_configuration, saved);
    let window = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Window")
        .expect("window section should exist");
    assert!(
        window
            .actions
            .iter()
            .find(|action| action.label == "Show Chat")
            .is_some_and(|action| !action.enabled)
    );
}

#[test]
fn gui_shell_app_state_clears_stale_runtime_show_chat_override_when_configuration_catches_up() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Chat",
                enabled: false,
            }],
            tls_prompt_expected: false,
            update_notice_expected: false,
            about_dialog_available: true,
        }
    )));
    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        section: "Chat",
        label: "Chat Input",
        value: false,
    }));
    assert!(state.runtime_menu_action_overrides.is_empty());

    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        section: "Chat",
        label: "Chat Input",
        value: true,
    }));

    let window = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Window")
        .expect("window section should exist");
    assert!(
        window
            .actions
            .iter()
            .find(|action| action.label == "Show Chat")
            .is_some_and(|action| action.enabled)
    );
}

#[test]
fn gui_shell_app_state_clears_stale_runtime_show_chat_override_when_configuration_runtime_snapshot_catches_up()
 {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Chat",
                enabled: false,
            }],
            tls_prompt_expected: false,
            update_notice_expected: false,
            about_dialog_available: true,
        }
    )));

    let mut draft = state.configuration.to_stored_settings();
    draft.chat_input_enabled = Some(false);
    let saved = state.saved_configuration.clone();
    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft.clone(),
                saved_settings: saved.clone(),
            }
        ))
    );
    assert!(state.runtime_menu_action_overrides.is_empty());

    draft.chat_input_enabled = Some(true);
    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft.clone(),
                saved_settings: saved,
            }
        ))
    );

    let window = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Window")
        .expect("window section should exist");
    assert!(
        window
            .actions
            .iter()
            .find(|action| action.label == "Show Chat")
            .is_some_and(|action| action.enabled)
    );
}

#[test]
fn gui_shell_app_state_preserves_runtime_show_playlist_override_across_configuration_edits() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Playlist",
                enabled: false,
            }],
            tls_prompt_expected: false,
            update_notice_expected: false,
            about_dialog_available: true,
        }
    )));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Host",
        value: "syncplay.example".to_owned(),
    }));

    let window = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Window")
        .expect("window section should exist");
    assert!(
        window
            .actions
            .iter()
            .find(|action| action.label == "Show Playlist")
            .is_some_and(|action| !action.enabled)
    );
}

#[test]
fn gui_shell_app_state_preserves_runtime_show_playlist_override_across_configuration_runtime_snapshots()
 {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Playlist",
                enabled: false,
            }],
            tls_prompt_expected: false,
            update_notice_expected: false,
            about_dialog_available: true,
        }
    )));

    let mut draft = state.configuration.to_stored_settings();
    draft.host = Some("draft.example".to_owned());
    let mut saved = state.saved_configuration.clone();
    saved.host = Some("saved.example".to_owned());

    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft.clone(),
                saved_settings: saved.clone(),
            }
        ))
    );

    assert_eq!(state.configuration.to_stored_settings(), draft);
    assert_eq!(state.saved_configuration, saved);
    let window = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Window")
        .expect("window section should exist");
    assert!(
        window
            .actions
            .iter()
            .find(|action| action.label == "Show Playlist")
            .is_some_and(|action| !action.enabled)
    );
}

#[test]
fn gui_shell_app_state_preserves_generic_runtime_menu_overrides_across_configuration_edits() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Help",
                action_label: "Check for Updates",
                enabled: false,
            }],
            tls_prompt_expected: false,
            update_notice_expected: false,
            about_dialog_available: true,
        }
    )));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Host",
        value: "syncplay.example".to_owned(),
    }));

    let help = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Help")
        .expect("help section should exist");
    assert!(
        help.actions
            .iter()
            .find(|action| action.label == "Check for Updates")
            .is_some_and(|action| !action.enabled)
    );
}

#[test]
fn gui_shell_app_state_preserves_generic_runtime_menu_overrides_across_configuration_runtime_snapshots()
 {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Help",
                action_label: "Check for Updates",
                enabled: false,
            }],
            tls_prompt_expected: false,
            update_notice_expected: false,
            about_dialog_available: true,
        }
    )));

    let mut draft = state.configuration.to_stored_settings();
    draft.host = Some("draft.example".to_owned());
    let mut saved = state.saved_configuration.clone();
    saved.host = Some("saved.example".to_owned());

    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft.clone(),
                saved_settings: saved.clone(),
            }
        ))
    );

    assert_eq!(state.configuration.to_stored_settings(), draft);
    assert_eq!(state.saved_configuration, saved);
    let help = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Help")
        .expect("help section should exist");
    assert!(
        help.actions
            .iter()
            .find(|action| action.label == "Check for Updates")
            .is_some_and(|action| !action.enabled)
    );
}

#[test]
fn gui_shell_app_state_preserves_runtime_public_server_and_media_search_flags_across_configuration_edits()
 {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())]),
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    let mut runtime_public_servers = state.public_servers.clone();
    runtime_public_servers.can_connect = false;
    runtime_public_servers.can_refresh = false;
    runtime_public_servers.can_add_custom_server = false;
    let mut runtime_media_search = state.media_search.clone();
    runtime_media_search.can_browse_directories = false;
    runtime_media_search.can_search_missing_media = false;

    assert!(state.apply(GuiShellAction::ApplyGuiRuntimeSnapshot(
        SyncplayGuiRuntimeSnapshot {
            active_view: state.active_view,
            open_modal: state.open_modal,
            main_window: MainWindowRuntimeSnapshot::from_shell_state(&state.main_window),
            public_servers: runtime_public_servers,
            media_search: runtime_media_search,
            tls_prompt_expected: state.menus.tls_prompt_expected,
            update_notice_expected: state.menus.update_notice_expected,
            about_dialog_available: state.menus.about_dialog_available,
        }
    )));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Host",
        value: "syncplay.example".to_owned(),
    }));

    assert!(!state.public_servers.can_connect);
    assert!(!state.public_servers.can_refresh);
    assert!(!state.public_servers.can_add_custom_server);
    assert!(!state.media_search.can_browse_directories);
    assert!(!state.media_search.can_search_missing_media);
}

#[test]
fn gui_shell_app_state_preserves_runtime_public_server_and_media_search_flags_across_configuration_runtime_snapshots()
 {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())]),
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    let mut runtime_public_servers = state.public_servers.clone();
    runtime_public_servers.can_connect = false;
    runtime_public_servers.can_refresh = false;
    runtime_public_servers.can_add_custom_server = false;
    let mut runtime_media_search = state.media_search.clone();
    runtime_media_search.can_browse_directories = false;
    runtime_media_search.can_search_missing_media = false;

    assert!(state.apply(GuiShellAction::ApplyGuiRuntimeSnapshot(
        SyncplayGuiRuntimeSnapshot {
            active_view: state.active_view,
            open_modal: state.open_modal,
            main_window: MainWindowRuntimeSnapshot::from_shell_state(&state.main_window),
            public_servers: runtime_public_servers,
            media_search: runtime_media_search,
            tls_prompt_expected: state.menus.tls_prompt_expected,
            update_notice_expected: state.menus.update_notice_expected,
            about_dialog_available: state.menus.about_dialog_available,
        }
    )));

    let mut draft = state.configuration.to_stored_settings();
    draft.host = Some("draft.example".to_owned());
    let mut saved = state.saved_configuration.clone();
    saved.host = Some("saved.example".to_owned());

    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft.clone(),
                saved_settings: saved.clone(),
            }
        ))
    );

    assert_eq!(state.configuration.to_stored_settings(), draft);
    assert_eq!(state.saved_configuration, saved);
    assert!(!state.public_servers.can_connect);
    assert!(!state.public_servers.can_refresh);
    assert!(!state.public_servers.can_add_custom_server);
    assert!(!state.media_search.can_browse_directories);
    assert!(!state.media_search.can_search_missing_media);
}

#[test]
fn gui_shell_app_state_preserves_runtime_public_server_and_media_search_rows_across_configuration_edits()
 {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())]),
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    let runtime_public_servers = PublicServerBrowserShellState {
        servers: vec![
            PublicServerBrowserRow {
                label: "Runtime Primary".to_owned(),
                address: "runtime.example:9000".to_owned(),
                is_selected: false,
            },
            PublicServerBrowserRow {
                label: "Runtime Backup".to_owned(),
                address: "backup.example:9001".to_owned(),
                is_selected: true,
            },
        ],
        can_connect: true,
        can_refresh: true,
        can_add_custom_server: true,
    };
    let runtime_media_search = MediaSearchWorkflowShellState {
        directories: vec![
            MediaSearchDirectoryRow {
                path: "D:/Runtime".to_owned(),
                is_selected: false,
            },
            MediaSearchDirectoryRow {
                path: "E:/Runtime".to_owned(),
                is_selected: true,
            },
        ],
        can_browse_directories: true,
        can_search_missing_media: true,
        first_file_timeout_seconds: state.media_search.first_file_timeout_seconds,
        search_timeout_seconds: state.media_search.search_timeout_seconds,
        double_check_interval_seconds: state.media_search.double_check_interval_seconds,
        warning_threshold_seconds: state.media_search.warning_threshold_seconds,
    };

    assert!(state.apply(GuiShellAction::ApplyGuiRuntimeSnapshot(
        SyncplayGuiRuntimeSnapshot {
            active_view: state.active_view,
            open_modal: state.open_modal,
            main_window: MainWindowRuntimeSnapshot::from_shell_state(&state.main_window),
            public_servers: runtime_public_servers,
            media_search: runtime_media_search,
            tls_prompt_expected: state.menus.tls_prompt_expected,
            update_notice_expected: state.menus.update_notice_expected,
            about_dialog_available: state.menus.about_dialog_available,
        }
    )));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Host",
        value: "syncplay.example".to_owned(),
    }));

    assert_eq!(state.public_servers.servers[0].label, "Runtime Primary");
    assert_eq!(state.public_servers.servers[1].label, "Runtime Backup");
    assert_eq!(state.selected_public_server_index(), Some(1));
    assert_eq!(state.media_search.directories[0].path, "D:/Runtime");
    assert_eq!(state.media_search.directories[1].path, "E:/Runtime");
    assert_eq!(state.selection.selected_media_search_directory, Some(1));
}

#[test]
fn gui_shell_app_state_preserves_runtime_public_server_and_media_search_rows_across_configuration_runtime_snapshots()
 {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())]),
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    let runtime_public_servers = PublicServerBrowserShellState {
        servers: vec![
            PublicServerBrowserRow {
                label: "Runtime Primary".to_owned(),
                address: "runtime.example:9000".to_owned(),
                is_selected: false,
            },
            PublicServerBrowserRow {
                label: "Runtime Backup".to_owned(),
                address: "backup.example:9001".to_owned(),
                is_selected: true,
            },
        ],
        can_connect: true,
        can_refresh: true,
        can_add_custom_server: true,
    };
    let runtime_media_search = MediaSearchWorkflowShellState {
        directories: vec![
            MediaSearchDirectoryRow {
                path: "D:/Runtime".to_owned(),
                is_selected: false,
            },
            MediaSearchDirectoryRow {
                path: "E:/Runtime".to_owned(),
                is_selected: true,
            },
        ],
        can_browse_directories: true,
        can_search_missing_media: true,
        first_file_timeout_seconds: state.media_search.first_file_timeout_seconds,
        search_timeout_seconds: state.media_search.search_timeout_seconds,
        double_check_interval_seconds: state.media_search.double_check_interval_seconds,
        warning_threshold_seconds: state.media_search.warning_threshold_seconds,
    };

    assert!(state.apply(GuiShellAction::ApplyGuiRuntimeSnapshot(
        SyncplayGuiRuntimeSnapshot {
            active_view: state.active_view,
            open_modal: state.open_modal,
            main_window: MainWindowRuntimeSnapshot::from_shell_state(&state.main_window),
            public_servers: runtime_public_servers,
            media_search: runtime_media_search,
            tls_prompt_expected: state.menus.tls_prompt_expected,
            update_notice_expected: state.menus.update_notice_expected,
            about_dialog_available: state.menus.about_dialog_available,
        }
    )));

    let mut draft = state.configuration.to_stored_settings();
    draft.host = Some("draft.example".to_owned());
    let mut saved = state.saved_configuration.clone();
    saved.host = Some("saved.example".to_owned());

    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft.clone(),
                saved_settings: saved.clone(),
            }
        ))
    );

    assert_eq!(state.configuration.to_stored_settings(), draft);
    assert_eq!(state.saved_configuration, saved);
    assert_eq!(state.public_servers.servers[0].label, "Runtime Primary");
    assert_eq!(state.public_servers.servers[1].label, "Runtime Backup");
    assert_eq!(state.selected_public_server_index(), Some(1));
    assert_eq!(state.media_search.directories[0].path, "D:/Runtime");
    assert_eq!(state.media_search.directories[1].path, "E:/Runtime");
    assert_eq!(state.selection.selected_media_search_directory, Some(1));
}

#[test]
fn gui_shell_app_state_updates_dialog_expectations_from_configuration_edits_without_runtime_overrides()
 {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        section: "Privacy",
        label: "Trusted Domains Only",
        value: true,
    }));
    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        section: "System",
        label: "Auto Update",
        value: true,
    }));

    assert!(state.menus.tls_prompt_expected);
    assert!(!state.menus.update_notice_expected);
}

#[test]
fn gui_shell_app_state_preserves_runtime_dialog_expectations_across_configuration_runtime_snapshots()
 {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::AnnounceTlsCertificatePromptRequired));
    assert!(state.apply(GuiShellAction::AnnounceUpdateNoticeAvailable));

    let mut draft = state.configuration.to_stored_settings();
    draft.host = Some("draft.example".to_owned());
    let mut saved = state.saved_configuration.clone();
    saved.host = Some("saved.example".to_owned());

    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft.clone(),
                saved_settings: saved.clone(),
            }
        ))
    );

    assert_eq!(state.configuration.to_stored_settings(), draft);
    assert_eq!(state.saved_configuration, saved);
    assert!(state.menus.tls_prompt_expected);
    assert!(state.menus.update_notice_expected);
}

#[test]
fn gui_shell_app_state_rejects_invalid_gui_configuration_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::BeginConfigurationReload));
    assert!(
        !state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: StoredClientSettingsMvp {
                    host: Some("draft.example".to_owned()),
                    ..StoredClientSettingsMvp::default()
                },
                saved_settings: StoredClientSettingsMvp {
                    host: Some("saved.example".to_owned()),
                    ..StoredClientSettingsMvp::default()
                },
            }
        ))
    );
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some(
            "GUI configuration runtime snapshots cannot apply while a configuration command is already in progress."
        )
    );
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::ReloadConfiguration)
    );
}

#[test]
fn gui_shell_app_state_projects_configuration_widget_trees() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        host: Some("syncplay.example".to_owned()),
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::FocusConfigurationControl {
        section: "Connection",
        label: "Host",
    }));
    assert!(state.apply(GuiShellAction::BeginConfigurationTextEdit {
        section: "Connection",
        label: "Host",
    }));
    assert!(state.apply(GuiShellAction::UpdateConfigurationTextEdit(
        "widget.example".to_owned(),
    )));

    let tree = state.configuration_widget_tree();
    let host = tree
        .find("config:Connection:Host")
        .expect("host control should exist in widget tree");
    assert_eq!(host.kind, GuiWidgetKind::TextInput);
    assert_eq!(host.value.as_deref(), Some("widget.example"));
    assert!(host.enabled);
    assert!(host.selected);

    let save = tree
        .find("config-command:save")
        .expect("save command should exist in widget tree");
    assert_eq!(save.kind, GuiWidgetKind::Button);
    assert!(save.enabled);
}

#[test]
fn gui_shell_app_state_projects_main_window_widget_trees() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        shared_playlist_enabled: Some(true),
        player_path: Some("mpv".to_owned()),
        room: Some("Lounge".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::AddMainWindowUser("Bob".to_owned())));
    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "One".to_owned(),
            "Two".to_owned(),
        ]))
    );
    assert!(state.apply(GuiShellAction::SelectMainWindowUser(1)));
    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(1)));
    assert!(state.apply(GuiShellAction::BeginLocalChatSend(
        "hello widget".to_owned(),
    )));

    let tree = state.main_window_widget_tree();
    let browser = tree
        .find("main-window:browser")
        .expect("room browser should exist in widget tree");
    assert_eq!(browser.kind, GuiWidgetKind::Panel);
    let room_group = tree
        .find("main-window:room-group:0")
        .expect("current room group should exist in widget tree");
    assert_eq!(room_group.kind, GuiWidgetKind::Panel);
    let room_group_state = tree
        .find("main-window:room-group:0:state")
        .expect("room-group state should exist in widget tree");
    assert_eq!(room_group_state.kind, GuiWidgetKind::Status);
    let user_state = tree
        .find("main-window:user:1:state")
        .expect("selected user state should exist in widget tree");
    assert_eq!(user_state.kind, GuiWidgetKind::Status);
    assert!(user_state.selected);
    assert!(tree.find("main-window:user:new").is_none());
    let room_input = tree
        .find("main-window:room-input")
        .expect("room input should exist in widget tree");
    assert_eq!(room_input.kind, GuiWidgetKind::TextInput);
    assert_eq!(room_input.value.as_deref(), Some("Lounge"));
    assert!(!room_input.enabled);

    let playlist = tree
        .find("main-window:playlist:1")
        .expect("selected playlist row should exist in widget tree");
    assert_eq!(playlist.kind, GuiWidgetKind::ListItem);
    assert!(playlist.selected);
    let new_playlist = tree
        .find("main-window:playlist:new")
        .expect("new playlist input should exist in widget tree");
    assert_eq!(new_playlist.kind, GuiWidgetKind::TextInput);
    assert_eq!(new_playlist.value.as_deref(), Some(""));
    let playlist_add = tree
        .find("main-window:playlist:add")
        .expect("playlist add button should exist in widget tree");
    assert_eq!(playlist_add.kind, GuiWidgetKind::Button);
    assert!(!playlist_add.enabled);
    assert!(tree.find("main-window:user:add").is_none());

    let chat_input = tree
        .find("main-window:chat-input")
        .expect("chat input should exist in widget tree");
    assert_eq!(chat_input.kind, GuiWidgetKind::TextInput);
    assert_eq!(chat_input.value.as_deref(), Some("hello widget"));
    assert_eq!(chat_input.enabled, state.commands.can_send_chat_message);
}

#[test]
fn gui_shell_app_state_projects_menu_dialog_widget_trees() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SelectMenuAction {
        section_index: 1,
        action_index: 0,
    }));
    assert!(state.apply(GuiShellAction::AnnounceAboutDialogRequested));

    let tree = state.menu_dialog_widget_tree();
    let pause = tree
        .find("menus:action:1:0")
        .expect("playback toggle action should exist");
    assert_eq!(pause.kind, GuiWidgetKind::Button);
    assert!(!pause.enabled);
    assert!(pause.selected);

    let about = tree
        .find("menus:dialog:about")
        .expect("about dialog status should exist");
    assert_eq!(about.kind, GuiWidgetKind::Status);
    assert!(about.enabled);
    assert!(about.selected);
    assert_eq!(about.value.as_deref(), Some("yes"));
}

#[test]
fn gui_shell_app_state_projects_public_server_and_media_search_widget_trees() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![("Alpha".to_owned(), "alpha.example:8999".to_owned())]),
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        folder_search_first_file_timeout_seconds: Some(3.0),
        folder_search_timeout_seconds: Some(30.0),
        folder_search_double_check_interval_seconds: Some(2.5),
        folder_search_warning_threshold_seconds: Some(7.5),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::BeginAddPublicServer));
    assert!(state.apply(GuiShellAction::UpdatePublicServerEditLabel(
        "Beta".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::UpdatePublicServerEditAddress(
        "beta.example:9000".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::SelectMediaSearchDirectory(0)));

    let server_tree = state.public_server_widget_tree();
    let row = server_tree
        .find("public-servers:row:0")
        .expect("public server row should exist");
    assert_eq!(row.kind, GuiWidgetKind::ListItem);
    assert!(row.selected);
    assert_eq!(row.value.as_deref(), Some("alpha.example:8999"));

    let edit_label = server_tree
        .find("public-servers:edit:label")
        .expect("public server edit label should exist");
    assert_eq!(edit_label.kind, GuiWidgetKind::TextInput);
    assert_eq!(edit_label.value.as_deref(), Some("Beta"));
    let edit_button = server_tree
        .find("public-servers:command:edit")
        .expect("public server edit command should exist");
    assert_eq!(edit_button.kind, GuiWidgetKind::Button);
    assert!(!edit_button.enabled);
    let commit_button = server_tree
        .find("public-servers:edit:commit")
        .expect("public server edit commit should exist");
    assert_eq!(commit_button.kind, GuiWidgetKind::Button);
    assert!(commit_button.enabled);
    let cancel_button = server_tree
        .find("public-servers:edit:cancel")
        .expect("public server edit cancel should exist");
    assert_eq!(cancel_button.kind, GuiWidgetKind::Button);
    assert!(cancel_button.enabled);

    let media_tree = state.media_search_widget_tree();
    let directory = media_tree
        .find("media-search:directory:0")
        .expect("media search directory should exist");
    assert_eq!(directory.kind, GuiWidgetKind::ListItem);
    assert!(directory.selected);

    let search = media_tree
        .find("media-search:command:search")
        .expect("media search command should exist");
    assert_eq!(search.kind, GuiWidgetKind::Button);
    assert!(search.enabled);
    let remove = media_tree
        .find("media-search:directory:remove")
        .expect("media-search remove command should exist");
    assert_eq!(remove.kind, GuiWidgetKind::Button);
    assert!(remove.enabled);
    let first_file_timing = media_tree
        .find("media-search:timing:first-file")
        .expect("media-search first-file timing status should exist");
    assert_eq!(first_file_timing.value.as_deref(), Some("3.00s"));
    let search_timing = media_tree
        .find("media-search:timing:search")
        .expect("media-search search timing status should exist");
    assert_eq!(search_timing.value.as_deref(), Some("30.00s"));
    let double_check_timing = media_tree
        .find("media-search:timing:double-check")
        .expect("media-search double-check timing status should exist");
    assert_eq!(double_check_timing.value.as_deref(), Some("2.50s"));
    let warning_timing = media_tree
        .find("media-search:timing:warning-threshold")
        .expect("media-search warning-threshold timing status should exist");
    assert_eq!(warning_timing.value.as_deref(), Some("7.50s"));
}

#[test]
fn gui_shell_app_state_projects_shell_widget_trees() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![("Alpha".to_owned(), "alpha.example:8999".to_owned())]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::PublicServers)));
    assert!(state.apply(GuiShellAction::OpenModal(GuiShellModal::UpdateNotice)));
    assert!(state.apply(GuiShellAction::PushTransientNotification {
        level: GuiTransientNotificationLevel::Info,
        message: "Widget tree ready".to_owned(),
    }));

    let tree = state.shell_widget_tree();
    assert_eq!(tree.kind, GuiWidgetKind::Panel);

    let active_view = tree
        .find("shell:active-view")
        .expect("active view status should exist");
    assert_eq!(active_view.value.as_deref(), Some("public-servers"));

    let open_modal = tree
        .find("shell:open-modal")
        .expect("open modal status should exist");
    assert_eq!(open_modal.value.as_deref(), Some("update-notice"));

    let modal_kind = tree
        .find("shell:modal:kind")
        .expect("modal kind status should exist");
    assert_eq!(modal_kind.value.as_deref(), Some("update-notice"));
    let dismiss_notice = tree
        .find("shell:modal:update:dismiss")
        .expect("update notice dismiss button should exist");
    assert_eq!(dismiss_notice.kind, GuiWidgetKind::Button);

    let notification = tree
        .find("shell:notification:0")
        .expect("notification row should exist");
    assert_eq!(notification.kind, GuiWidgetKind::ListItem);
    assert_eq!(notification.value.as_deref(), Some("Widget tree ready"));

    let save_status = tree
        .find("shell:command:save")
        .expect("command status row should exist");
    assert_eq!(save_status.kind, GuiWidgetKind::Status);
    assert_eq!(save_status.value.as_deref(), Some("enabled"));

    let validation_status = tree
        .find("shell:validation:status")
        .expect("validation status row should exist");
    assert_eq!(validation_status.value.as_deref(), Some("clean"));

    let last_action_error = tree
        .find("shell:validation:last-action-error")
        .expect("last action error row should exist");
    assert_eq!(last_action_error.value.as_deref(), Some("(none)"));

    let public_servers = tree
        .find("public-servers-root")
        .expect("public server subtree should exist");
    assert!(public_servers.selected);
}

#[test]
fn gui_shell_app_state_projects_validation_and_busy_command_status_into_widget_tree() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Port",
        value: "70000".to_owned(),
    }));
    let invalid_tree = state.shell_widget_tree();
    assert_eq!(
        invalid_tree
            .find("shell:validation:status")
            .and_then(|node| node.value.as_deref()),
        Some("1 issue(s)")
    );
    assert_eq!(
        invalid_tree
            .find("shell:validation:issue:0")
            .map(|node| (node.label.as_str(), node.value.as_deref())),
        Some((
            "Connection / Port",
            Some("must be a valid TCP port from 1 to 65535."),
        ))
    );
    assert_eq!(
        invalid_tree
            .find("shell:command:save")
            .and_then(|node| node.value.as_deref()),
        Some("disabled")
    );

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Port",
        value: "8999".to_owned(),
    }));
    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    let busy_tree = state.shell_widget_tree();
    assert_eq!(
        busy_tree
            .find("shell:command:busy")
            .and_then(|node| node.value.as_deref()),
        Some("yes")
    );
    for widget_id in [
        "shell:command:save",
        "shell:command:reset",
        "shell:command:reload",
        "shell:command:connect-public-server",
        "shell:command:refresh-public-servers",
        "shell:command:search-missing-media",
        "shell:command:toggle-pause",
        "shell:command:send-chat-message",
    ] {
        assert_eq!(
            busy_tree
                .find(widget_id)
                .and_then(|node| node.value.as_deref()),
            Some("disabled"),
            "{widget_id} should surface as disabled while a pending operation is active",
        );
    }
}

#[test]
fn gui_shell_app_state_renders_shell_widget_trees_through_renderer() {
    #[derive(Default)]
    struct RecordingRenderer {
        events: Vec<String>,
    }

    impl GuiWidgetRenderer for RecordingRenderer {
        fn begin_node(&mut self, node: &GuiWidgetNode, depth: usize) {
            self.events.push(format!("begin:{depth}:{}", node.id));
        }

        fn end_node(&mut self, node: &GuiWidgetNode, depth: usize) {
            self.events.push(format!("end:{depth}:{}", node.id));
        }
    }

    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![("Alpha".to_owned(), "alpha.example:8999".to_owned())]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::PublicServers)));
    assert!(state.apply(GuiShellAction::PushTransientNotification {
        level: GuiTransientNotificationLevel::Info,
        message: "Renderer adapter ready".to_owned(),
    }));

    let mut renderer = RecordingRenderer::default();
    state.render_shell_widgets(&mut renderer);

    assert_eq!(
        renderer.events.first().map(String::as_str),
        Some("begin:0:shell-root")
    );
    assert_eq!(
        renderer.events.last().map(String::as_str),
        Some("end:0:shell-root")
    );
    assert!(
        renderer
            .events
            .iter()
            .any(|event| event == "begin:1:shell:notifications")
    );
    assert!(
        renderer
            .events
            .iter()
            .any(|event| event == "begin:2:shell:notification:0")
    );
    assert!(
        renderer
            .events
            .iter()
            .any(|event| event == "begin:1:shell:commands")
    );
    assert!(
        renderer
            .events
            .iter()
            .any(|event| event == "begin:2:shell:command:save")
    );
    assert!(
        renderer
            .events
            .iter()
            .any(|event| event == "begin:1:shell:validation")
    );
    assert!(
        renderer
            .events
            .iter()
            .any(|event| event == "begin:2:shell:validation:status")
    );
    assert!(
        renderer
            .events
            .iter()
            .any(|event| event == "begin:1:public-servers-root")
    );
    assert!(
        renderer
            .events
            .iter()
            .any(|event| event == "end:1:public-servers-root")
    );
}

#[test]
fn gui_shell_app_state_triggers_selected_menu_actions() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_toggle_pause = true;
    state.refresh_validation();
    state.sync_playback_menu_actions_from_runtime_state(state.commands.can_toggle_pause);

    assert!(state.apply(GuiShellAction::SelectMenuAction {
        section_index: 0,
        action_index: 2,
    }));
    assert!(state.apply(GuiShellAction::TriggerSelectedMenuAction));
    assert_eq!(state.active_view, GuiShellView::PublicServers);

    assert!(state.apply(GuiShellAction::SelectMenuAction {
        section_index: 2,
        action_index: 4,
    }));
    assert!(state.apply(GuiShellAction::TriggerSelectedMenuAction));
    assert_eq!(state.open_modal, Some(GuiShellModal::TlsCertificatePrompt));

    assert!(state.apply(GuiShellAction::SelectMenuAction {
        section_index: 1,
        action_index: 1,
    }));
    assert!(state.apply(GuiShellAction::TriggerSelectedMenuAction));
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::TogglePlaybackPause)
    );
    assert!(state.apply(GuiShellAction::CompletePlaybackPauseToggle));
    assert!(state.main_window.playback_paused);

    let mut disabled_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    assert!(disabled_state.apply(GuiShellAction::SelectMenuAction {
        section_index: 1,
        action_index: 1,
    }));
    assert!(!disabled_state.apply(GuiShellAction::TriggerSelectedMenuAction));
    assert_eq!(
        disabled_state.validation.last_action_error.as_deref(),
        Some("The selected menu action is currently disabled.")
    );
}

#[test]
fn gui_shell_app_state_selects_public_server_and_updates_config_host_port() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![
            ("Primary".to_owned(), "syncplay.pl:8999".to_owned()),
            ("Backup".to_owned(), "syncplay.example:8995".to_owned()),
        ]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SelectPublicServer(1)));
    assert!(!state.public_servers.servers[0].is_selected);
    assert!(state.public_servers.servers[1].is_selected);

    let saved = state.configuration.to_stored_settings();
    assert_eq!(saved.host.as_deref(), Some("syncplay.example"));
    assert_eq!(saved.port, Some(8995));
}

#[test]
fn gui_shell_app_state_handles_public_server_browser_event_actions() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::AnnouncePublicServerSelectionChanged(0)));
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("Public server selected: Primary.")
    );

    assert!(state.apply(GuiShellAction::BeginSelectedPublicServerConnect));
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::ConnectPublicServer)
    );
    assert!(state.apply(GuiShellAction::CompleteSelectedPublicServerConnect));
    assert_eq!(state.pending_operation, None);
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Connected to public server: Primary.")
    );

    assert!(state.apply(GuiShellAction::BeginPublicServerRefresh));
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::RefreshPublicServers)
    );
    assert!(
        state.apply(GuiShellAction::CompletePublicServerRefresh(vec![
            ("Refreshed".to_owned(), "syncplay.example:8995".to_owned()),
            ("Backup".to_owned(), "backup.example:8998".to_owned()),
        ]))
    );
    assert_eq!(state.pending_operation, None);
    assert_eq!(state.public_servers.servers.len(), 2);
    assert!(state.public_servers.servers[0].is_selected);
    assert_eq!(
        state.configuration.to_stored_settings().host.as_deref(),
        Some("syncplay.example")
    );

    assert!(
        state.apply(GuiShellAction::AnnounceCustomPublicServerAdded {
            label: "Custom".to_owned(),
            address: "custom.example:9000".to_owned(),
        })
    );
    assert_eq!(state.public_servers.servers.len(), 3);
    assert!(state.public_servers.servers[2].is_selected);
    assert_eq!(
        state.configuration.to_stored_settings().host.as_deref(),
        Some("custom.example")
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_public_server_browser_event_actions() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::BeginSelectedPublicServerConnect));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Public server connect is unavailable when browser connect actions are disabled.")
    );

    assert!(!state.apply(GuiShellAction::CompletePublicServerRefresh(vec![])));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No public server refresh is currently in progress.")
    );

    assert!(state.apply(GuiShellAction::BeginPublicServerRefresh));
    assert!(!state.apply(GuiShellAction::BeginPublicServerRefresh));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Another GUI operation is already in progress.")
    );

    assert!(
        !state.apply(GuiShellAction::AnnounceCustomPublicServerAdded {
            label: "Broken".to_owned(),
            address: ":8999".to_owned(),
        })
    );
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Custom public-server address is not valid.")
    );
}

#[test]
fn gui_shell_app_state_adds_edits_and_removes_public_server_rows() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::BeginAddPublicServer));
    assert!(state.apply(GuiShellAction::UpdatePublicServerEditLabel(
        "Primary".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::UpdatePublicServerEditAddress(
        "syncplay.pl:8999".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::CommitPublicServerEdit));
    assert_eq!(state.public_servers.servers.len(), 1);
    assert_eq!(state.public_servers.servers[0].label, "Primary");
    assert!(state.public_servers.servers[0].is_selected);
    assert_eq!(
        state.configuration.to_stored_settings().public_servers,
        Some(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())])
    );
    let saved = state.configuration.to_stored_settings();
    assert_eq!(saved.host.as_deref(), Some("syncplay.pl"));
    assert_eq!(saved.port, Some(8999));

    assert!(state.apply(GuiShellAction::BeginEditSelectedPublicServer));
    assert!(state.apply(GuiShellAction::UpdatePublicServerEditLabel(
        "Primary EU".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::UpdatePublicServerEditAddress(
        "syncplay.example:8995".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::CommitPublicServerEdit));
    assert_eq!(state.public_servers.servers[0].label, "Primary EU");
    assert_eq!(
        state.public_servers.servers[0].address,
        "syncplay.example:8995"
    );
    let saved = state.configuration.to_stored_settings();
    assert_eq!(saved.host.as_deref(), Some("syncplay.example"));
    assert_eq!(saved.port, Some(8995));

    assert!(state.apply(GuiShellAction::BeginAddPublicServer));
    assert!(state.apply(GuiShellAction::UpdatePublicServerEditLabel(
        "Secondary".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::UpdatePublicServerEditAddress(
        "backup.example:8998".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::CancelPublicServerEdit));
    assert!(state.public_server_edit_session.is_none());
    assert_eq!(state.public_servers.servers.len(), 1);

    assert!(state.apply(GuiShellAction::RemoveSelectedPublicServer));
    assert!(state.public_servers.servers.is_empty());
    assert_eq!(
        state.configuration.to_stored_settings().public_servers,
        None
    );
}

#[test]
fn gui_shell_app_state_remaps_public_server_edit_sessions_by_row_identity_across_configuration_runtime_snapshots()
 {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![
            ("Alpha".to_owned(), "alpha.example:8999".to_owned()),
            ("Beta".to_owned(), "beta.example:8999".to_owned()),
        ]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SelectPublicServer(1)));
    assert!(state.apply(GuiShellAction::BeginEditSelectedPublicServer));
    assert!(state.apply(GuiShellAction::UpdatePublicServerEditLabel(
        "Beta Edited".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::SelectPublicServer(0)));

    let mut draft = state.configuration.to_stored_settings();
    draft.public_servers = Some(vec![
        ("Inserted".to_owned(), "inserted.example:8999".to_owned()),
        ("Alpha".to_owned(), "alpha.example:8999".to_owned()),
        ("Beta".to_owned(), "beta.example:8999".to_owned()),
    ]);
    let saved = state.saved_configuration.clone();

    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft,
                saved_settings: saved,
            }
        ))
    );

    assert_eq!(
        state
            .public_server_edit_session
            .as_ref()
            .map(|session| session.editing_index),
        Some(Some(2))
    );
    assert_eq!(
        state
            .public_server_edit_session
            .as_ref()
            .map(|session| session.label_buffer.as_str()),
        Some("Beta Edited")
    );
    assert_eq!(state.selected_public_server_index(), Some(2));
    assert!(state.public_servers.servers[2].is_selected);
}

#[test]
fn gui_shell_app_state_clears_public_server_edit_sessions_when_the_edited_row_disappears() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![
            ("Alpha".to_owned(), "alpha.example:8999".to_owned()),
            ("Beta".to_owned(), "beta.example:8999".to_owned()),
        ]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SelectPublicServer(1)));
    assert!(state.apply(GuiShellAction::BeginEditSelectedPublicServer));

    let mut draft = state.configuration.to_stored_settings();
    draft.public_servers = Some(vec![("Alpha".to_owned(), "alpha.example:8999".to_owned())]);
    let saved = state.saved_configuration.clone();

    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigurationRuntimeSnapshot(
            GuiConfigurationRuntimeSnapshot {
                draft_settings: draft,
                saved_settings: saved,
            }
        ))
    );

    assert!(state.public_server_edit_session.is_none());
}

#[test]
fn gui_shell_app_state_keeps_public_server_selection_on_the_active_edit_row() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![
            ("Alpha".to_owned(), "alpha.example:8999".to_owned()),
            ("Beta".to_owned(), "beta.example:8999".to_owned()),
        ]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SelectPublicServer(1)));
    assert!(state.apply(GuiShellAction::BeginEditSelectedPublicServer));
    assert!(state.apply(GuiShellAction::SelectPublicServer(0)));

    assert_eq!(state.selected_public_server_index(), Some(1));
    assert!(state.public_servers.servers[1].is_selected);
    assert!(
        state
            .public_server_edit_session
            .as_ref()
            .is_some_and(|session| session.editing_index == Some(1))
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_public_server_edit_sessions() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::BeginEditSelectedPublicServer));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No public server is currently selected.")
    );

    assert!(!state.apply(GuiShellAction::CommitPublicServerEdit));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No public-server edit session is currently active.")
    );

    assert!(state.apply(GuiShellAction::BeginAddPublicServer));
    assert!(state.apply(GuiShellAction::UpdatePublicServerEditLabel(
        "Broken".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::UpdatePublicServerEditAddress(
        ":8999".to_owned(),
    )));
    assert!(!state.apply(GuiShellAction::CommitPublicServerEdit));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Public-server address is not valid.")
    );
}

#[test]
fn gui_shell_app_state_tracks_transient_notification_queue() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    for (level, message) in [
        (GuiTransientNotificationLevel::Info, "one"),
        (GuiTransientNotificationLevel::Success, "two"),
        (GuiTransientNotificationLevel::Warning, "three"),
        (GuiTransientNotificationLevel::Error, "four"),
        (GuiTransientNotificationLevel::Info, "five"),
        (GuiTransientNotificationLevel::Success, "six"),
    ] {
        assert!(state.apply(GuiShellAction::PushTransientNotification {
            level,
            message: message.to_owned(),
        }));
    }

    assert_eq!(state.notifications.len(), 5);
    assert_eq!(state.notifications[0].message, "two");
    assert_eq!(state.notifications[4].message, "six");

    let rendered = state.render_lines().join("\n");
    assert!(rendered.contains("[Notifications] count=5"));
    assert!(rendered.contains("- success: two"));
    assert!(rendered.contains("- success: six"));

    assert!(state.apply(GuiShellAction::DismissTransientNotification(1)));
    assert_eq!(state.notifications.len(), 4);
    assert!(state.apply(GuiShellAction::ClearTransientNotifications));
    assert!(state.notifications.is_empty());
    assert!(!state.apply(GuiShellAction::ClearTransientNotifications));
}

#[test]
fn gui_shell_app_state_rejects_invalid_transient_notification_actions() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::PushTransientNotification {
        level: GuiTransientNotificationLevel::Info,
        message: "   ".to_owned(),
    }));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Transient notification messages must be non-empty.")
    );

    assert!(!state.apply(GuiShellAction::DismissTransientNotification(0)));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No transient notification exists at the requested index.")
    );
}

#[test]
fn gui_shell_app_state_adds_media_directory_and_pushes_chat_messages() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::AddMediaSearchDirectory(
        "C:/Media".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::PushChatMessage {
        sender: "system".to_owned(),
        message: "Connected".to_owned(),
    }));

    assert_eq!(state.media_search.directories.len(), 1);
    assert_eq!(state.media_search.directories[0].path, "C:/Media");
    assert!(state.media_search.directories[0].is_selected);
    assert!(state.media_search.can_search_missing_media);
    assert_eq!(state.main_window.chat.len(), 1);
    assert_eq!(state.main_window.chat[0].message, "Connected");

    let saved = state.configuration.to_stored_settings();
    assert_eq!(
        saved.media_search_directories,
        Some(vec!["C:/Media".to_owned()])
    );
}

#[test]
fn gui_shell_app_state_tracks_local_and_remote_chat_event_actions() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::BeginLocalChatSend("hello world".to_owned(),)));
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::SendChatMessage)
    );
    assert_eq!(state.outgoing_chat_message.as_deref(), Some("hello world"));
    assert!(
        state
            .render_lines()
            .join("\n")
            .contains("[Chat Send] pending_message=hello world")
    );

    assert!(state.apply(GuiShellAction::CompleteLocalChatSend));
    assert_eq!(state.pending_operation, None);
    assert_eq!(state.outgoing_chat_message, None);
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("hello world")
    );
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Chat sent.")
    );

    assert!(state.apply(GuiShellAction::AnnounceRemoteChatMessage {
        sender: "alice".to_owned(),
        message: "hi there".to_owned(),
    }));
    assert_eq!(
        state.main_window.chat.last().map(|row| row.sender.as_str()),
        Some("alice")
    );
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("hi there")
    );

    assert!(state.apply(GuiShellAction::AnnounceSystemChatEvent(
        "Connection stabilized.".to_owned(),
    )));
    assert_eq!(
        state.main_window.chat.last().map(|row| row.sender.as_str()),
        Some("system")
    );
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("Connection stabilized.")
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_chat_event_actions() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::BeginLocalChatSend("hello".to_owned())));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Local chat sending is unavailable when chat input is disabled.")
    );

    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(!state.apply(GuiShellAction::BeginLocalChatSend("   ".to_owned())));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Local chat messages must be non-empty.")
    );

    assert!(!state.apply(GuiShellAction::CompleteLocalChatSend));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No local chat send is currently in progress.")
    );

    assert!(state.apply(GuiShellAction::BeginLocalChatSend("hello".to_owned())));
    assert!(!state.apply(GuiShellAction::BeginLocalChatSend("again".to_owned())));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Another GUI operation is already in progress.")
    );
    assert!(state.apply(GuiShellAction::CancelLocalChatSend));
    assert_eq!(state.pending_operation, None);
    assert_eq!(state.outgoing_chat_message, None);
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Chat send canceled.")
    );

    assert!(!state.apply(GuiShellAction::AnnounceRemoteChatMessage {
        sender: " ".to_owned(),
        message: "hi".to_owned(),
    }));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Remote chat sender and message must both be non-empty.")
    );
}

#[test]
fn gui_shell_app_state_handles_text_edits_and_room_switches() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Username",
        value: TEST_USERNAME.to_owned(),
    }));
    assert!(state.apply(GuiShellAction::SetMainWindowRoom(
        "+room:ABCDEF123456".to_owned(),
    )));

    let saved = state.configuration.to_stored_settings();
    assert_eq!(saved.username.as_deref(), Some(TEST_USERNAME));
    assert_eq!(saved.room.as_deref(), Some("+room:ABCDEF123456"));
    assert_eq!(state.main_window.room_name, "+room:ABCDEF123456");
    assert!(state.main_window.controlled_room_active);
    assert!(state.main_window.users[0].is_controller);
}

#[test]
fn gui_shell_app_state_defers_room_join_and_leave_to_runtime_confirmation() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        room: Some("+room:ABCDEF123456".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let baseline_chat_len = state.main_window.chat.len();
    let baseline_notification_len = state.notifications.len();

    assert!(state.apply(GuiShellAction::JoinMainWindowRoom(
        "+room:NEEDS_RUNTIME".to_owned(),
    )));
    assert_eq!(state.main_window.room_name, "+room:ABCDEF123456");
    assert!(state.main_window.controlled_room_active);
    assert!(state.main_window.users[0].is_controller);
    assert_eq!(
        state.configuration.to_stored_settings().room.as_deref(),
        Some("+room:ABCDEF123456")
    );
    assert_eq!(state.main_window.chat.len(), baseline_chat_len);
    assert_eq!(state.notifications.len(), baseline_notification_len);

    assert!(state.apply(GuiShellAction::LeaveMainWindowRoom));
    assert_eq!(state.main_window.room_name, "+room:ABCDEF123456");
    assert!(state.main_window.controlled_room_active);
    assert!(state.main_window.users[0].is_controller);
    assert_eq!(
        state.configuration.to_stored_settings().room.as_deref(),
        Some("+room:ABCDEF123456")
    );
    assert_eq!(state.main_window.chat.len(), baseline_chat_len);
    assert_eq!(state.notifications.len(), baseline_notification_len);
}

#[test]
fn gui_shell_app_state_rejects_invalid_room_status_actions() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::JoinMainWindowRoom("   ".to_owned(),)));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Room name cannot be empty.")
    );

    assert!(!state.apply(GuiShellAction::LeaveMainWindowRoom));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No joined room is currently active.")
    );
}

#[test]
fn gui_shell_app_state_applies_main_window_runtime_snapshots() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        room: Some("InitialRoom".to_owned()),
        shared_playlist_enabled: Some(true),
        player_path: Some("mpv".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "RuntimeRoom".to_owned(),
            shared_playlist_enabled: true,
            controlled_room_active: false,
            users: vec![
                MainWindowRuntimeUserSnapshot {
                    username: "alice".to_owned(),
                    is_self: true,
                    is_ready: true,
                    is_controller: false,
                    ..Default::default()
                },
                MainWindowRuntimeUserSnapshot {
                    username: "bob".to_owned(),
                    is_self: false,
                    is_ready: false,
                    is_controller: false,
                    ..Default::default()
                },
            ],
            playlist: vec!["One".to_owned(), "Two".to_owned()],
            chat: vec![MainWindowRuntimeChatSnapshot {
                sender: "alice".to_owned(),
                message: "hello".to_owned(),
            }],
            can_toggle_pause: true,
            can_seek: true,
            can_set_ready: true,
            can_manage_playlist: true,
            playback_paused: false,
            autoplay_active: true,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            ..Default::default()
        },
    )));
    assert_eq!(state.main_window.room_name, "RuntimeRoom");
    assert_eq!(state.main_window.users.len(), 2);
    assert_eq!(state.main_window.playlist.len(), 2);
    assert!(state.notifications.is_empty());
    assert_eq!(state.selection.selected_main_window_user, Some(0));
    assert_eq!(state.selection.selected_main_window_playlist, Some(0));

    assert!(state.apply(GuiShellAction::SelectMainWindowUser(1)));
    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(1)));

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "+RuntimeRoom".to_owned(),
            shared_playlist_enabled: true,
            controlled_room_active: true,
            users: vec![
                MainWindowRuntimeUserSnapshot {
                    username: "bob".to_owned(),
                    is_self: false,
                    is_ready: true,
                    is_controller: true,
                    ..Default::default()
                },
                MainWindowRuntimeUserSnapshot {
                    username: "carol".to_owned(),
                    is_self: false,
                    is_ready: false,
                    is_controller: false,
                    ..Default::default()
                },
                MainWindowRuntimeUserSnapshot {
                    username: "alice".to_owned(),
                    is_self: true,
                    is_ready: true,
                    is_controller: false,
                    ..Default::default()
                },
            ],
            playlist: vec!["Two".to_owned(), "Three".to_owned()],
            chat: vec![
                MainWindowRuntimeChatSnapshot {
                    sender: "system".to_owned(),
                    message: "room sync".to_owned(),
                },
                MainWindowRuntimeChatSnapshot {
                    sender: "bob".to_owned(),
                    message: "ready".to_owned(),
                },
            ],
            can_toggle_pause: true,
            can_seek: true,
            can_set_ready: true,
            can_manage_playlist: true,
            playback_paused: true,
            autoplay_active: false,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            ..Default::default()
        },
    )));
    assert_eq!(state.main_window.room_name, "+RuntimeRoom");
    assert!(state.main_window.controlled_room_active);
    assert!(state.main_window.playback_paused);
    assert!(!state.main_window.autoplay_active);
    assert_eq!(state.selection.selected_main_window_user, Some(0));
    assert_eq!(state.main_window.users[0].username.as_str(), "bob");
    assert_eq!(state.selection.selected_main_window_playlist, Some(0));
    assert_eq!(state.main_window.playlist[0].label.as_str(), "Two");
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("ready")
    );
}

#[test]
fn gui_shell_app_state_syncs_playback_menu_actions_from_main_window_runtime_snapshots() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SelectMenuAction {
        section_index: 1,
        action_index: 0,
    }));

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "RuntimeRoom".to_owned(),
            shared_playlist_enabled: false,
            controlled_room_active: false,
            users: vec![MainWindowRuntimeUserSnapshot {
                username: "alice".to_owned(),
                is_self: true,
                is_ready: false,
                is_controller: false,
                ..Default::default()
            }],
            playlist: vec!["One".to_owned()],
            chat: Vec::new(),
            can_toggle_pause: false,
            can_seek: false,
            can_set_ready: false,
            can_manage_playlist: false,
            playback_paused: true,
            autoplay_active: false,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            ..Default::default()
        },
    )));

    assert_eq!(state.selection.selected_menu_action, Some((0, 1)));
    let file = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "File")
        .expect("file section should exist");
    assert!(
        file.actions
            .iter()
            .find(|action| action.label == "Open Media File")
            .is_some_and(|action| !action.enabled && !action.is_selected)
    );
    let playback = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Playback")
        .expect("playback section should exist");
    assert!(
        playback
            .actions
            .iter()
            .find(|action| action.label == "Toggle Pause")
            .is_some_and(|action| !action.enabled && !action.is_selected)
    );
    assert!(
        playback
            .actions
            .iter()
            .find(|action| action.label == "Seek")
            .is_some_and(|action| !action.enabled && !action.is_selected)
    );
    assert!(
        playback
            .actions
            .iter()
            .find(|action| action.label == "Playlist Actions")
            .is_some_and(|action| !action.enabled && !action.is_selected)
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_main_window_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "   ".to_owned(),
            shared_playlist_enabled: false,
            controlled_room_active: false,
            users: Vec::new(),
            playlist: Vec::new(),
            chat: Vec::new(),
            can_toggle_pause: false,
            can_seek: false,
            can_set_ready: false,
            can_manage_playlist: false,
            playback_paused: false,
            autoplay_active: false,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            ..Default::default()
        },
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Main-window runtime snapshots must include a non-empty room name.")
    );

    assert!(!state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "Room".to_owned(),
            shared_playlist_enabled: false,
            controlled_room_active: false,
            users: vec![
                MainWindowRuntimeUserSnapshot {
                    username: "alice".to_owned(),
                    is_self: false,
                    is_ready: false,
                    is_controller: false,
                    ..Default::default()
                },
                MainWindowRuntimeUserSnapshot {
                    username: "Alice".to_owned(),
                    is_self: false,
                    is_ready: false,
                    is_controller: false,
                    ..Default::default()
                },
            ],
            playlist: Vec::new(),
            chat: Vec::new(),
            can_toggle_pause: false,
            can_seek: false,
            can_set_ready: false,
            can_manage_playlist: false,
            playback_paused: false,
            autoplay_active: false,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            ..Default::default()
        },
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Main-window runtime snapshots cannot contain duplicate user names.")
    );
}

#[test]
fn gui_shell_app_state_applies_full_gui_runtime_snapshots() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![("Keep".to_owned(), "keep.example:8999".to_owned())]),
        media_search_directories: Some(vec!["C:/Existing".to_owned()]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "SeedRoom".to_owned(),
            shared_playlist_enabled: true,
            controlled_room_active: false,
            users: vec![
                MainWindowRuntimeUserSnapshot {
                    username: "alice".to_owned(),
                    is_self: true,
                    is_ready: false,
                    is_controller: false,
                    ..Default::default()
                },
                MainWindowRuntimeUserSnapshot {
                    username: "bob".to_owned(),
                    is_self: false,
                    is_ready: false,
                    is_controller: false,
                    ..Default::default()
                },
            ],
            playlist: vec!["A".to_owned(), "B".to_owned()],
            chat: vec![MainWindowRuntimeChatSnapshot {
                sender: "system".to_owned(),
                message: "seed".to_owned(),
            }],
            can_toggle_pause: true,
            can_seek: true,
            can_set_ready: true,
            can_manage_playlist: true,
            playback_paused: false,
            autoplay_active: false,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            ..Default::default()
        },
    )));
    assert!(state.apply(GuiShellAction::SelectMainWindowUser(1)));
    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(1)));
    assert!(state.apply(GuiShellAction::SelectMediaSearchDirectory(0)));

    assert!(state.apply(GuiShellAction::ApplyGuiRuntimeSnapshot(
        SyncplayGuiRuntimeSnapshot {
            active_view: GuiShellView::PublicServers,
            open_modal: Some(GuiShellModal::UpdateNotice),
            main_window: MainWindowRuntimeSnapshot {
                room_name: "+LiveRoom".to_owned(),
                shared_playlist_enabled: true,
                controlled_room_active: true,
                users: vec![
                    MainWindowRuntimeUserSnapshot {
                        username: "bob".to_owned(),
                        is_self: false,
                        is_ready: true,
                        is_controller: true,
                        ..Default::default()
                    },
                    MainWindowRuntimeUserSnapshot {
                        username: "carol".to_owned(),
                        is_self: false,
                        is_ready: false,
                        is_controller: false,
                        ..Default::default()
                    },
                ],
                playlist: vec!["B".to_owned(), "C".to_owned()],
                chat: vec![MainWindowRuntimeChatSnapshot {
                    sender: "bob".to_owned(),
                    message: "synced".to_owned(),
                }],
                can_toggle_pause: true,
                can_seek: false,
                can_set_ready: true,
                can_manage_playlist: true,
                playback_paused: true,
                autoplay_active: true,
                hide_empty_rooms: false,
                rooms: Vec::new(),
                ..Default::default()
            },
            public_servers: PublicServerBrowserShellState {
                servers: vec![
                    PublicServerBrowserRow {
                        label: "Alpha".to_owned(),
                        address: "alpha.example:8999".to_owned(),
                        is_selected: false,
                    },
                    PublicServerBrowserRow {
                        label: "Beta".to_owned(),
                        address: "beta.example:8999".to_owned(),
                        is_selected: true,
                    },
                ],
                can_connect: true,
                can_refresh: true,
                can_add_custom_server: true,
            },
            media_search: MediaSearchWorkflowShellState {
                directories: vec![
                    MediaSearchDirectoryRow {
                        path: "D:/Media".to_owned(),
                        is_selected: false,
                    },
                    MediaSearchDirectoryRow {
                        path: "E:/Library".to_owned(),
                        is_selected: true,
                    },
                ],
                can_browse_directories: true,
                can_search_missing_media: true,
                first_file_timeout_seconds: Some(1.0),
                search_timeout_seconds: Some(15.0),
                double_check_interval_seconds: Some(2.0),
                warning_threshold_seconds: Some(5.0),
            },
            tls_prompt_expected: true,
            update_notice_expected: true,
            about_dialog_available: false,
        },
    )));

    assert_eq!(state.active_view, GuiShellView::PublicServers);
    assert_eq!(state.open_modal, Some(GuiShellModal::UpdateNotice));
    assert_eq!(state.main_window.room_name, "+LiveRoom");
    assert!(state.main_window.playback_paused);
    assert!(state.main_window.autoplay_active);
    assert_eq!(state.selection.selected_main_window_user, Some(0));
    assert_eq!(state.main_window.users[0].username.as_str(), "bob");
    assert_eq!(state.selection.selected_main_window_playlist, Some(0));
    assert_eq!(state.main_window.playlist[0].label.as_str(), "B");
    assert_eq!(state.selected_public_server_index(), Some(1));
    assert_eq!(state.selection.selected_media_search_directory, Some(1));
    let playback = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Playback")
        .expect("playback section should exist");
    let file = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "File")
        .expect("file section should exist");
    assert!(
        file.actions
            .iter()
            .find(|action| action.label == "Open Media File")
            .is_some_and(|action| action.enabled)
    );
    assert!(
        playback
            .actions
            .iter()
            .find(|action| action.label == "Toggle Pause")
            .is_some_and(|action| action.enabled)
    );
    assert!(
        playback
            .actions
            .iter()
            .find(|action| action.label == "Seek")
            .is_some_and(|action| !action.enabled)
    );
    assert!(
        playback
            .actions
            .iter()
            .find(|action| action.label == "Playlist Actions")
            .is_some_and(|action| action.enabled)
    );
    assert!(state.menus.tls_prompt_expected);
    assert!(state.menus.update_notice_expected);
    assert!(!state.menus.about_dialog_available);
    let help = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Help")
        .expect("help section should exist");
    assert!(
        help.actions
            .iter()
            .find(|action| action.label == "About")
            .is_some_and(|action| !action.enabled)
    );
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("GUI runtime snapshot applied.")
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_full_gui_runtime_snapshots() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::ApplyGuiRuntimeSnapshot(
        SyncplayGuiRuntimeSnapshot {
            active_view: GuiShellView::MainWindow,
            open_modal: None,
            main_window: MainWindowRuntimeSnapshot {
                room_name: "   ".to_owned(),
                shared_playlist_enabled: false,
                controlled_room_active: false,
                users: Vec::new(),
                playlist: Vec::new(),
                chat: Vec::new(),
                can_toggle_pause: false,
                can_seek: false,
                can_set_ready: false,
                can_manage_playlist: false,
                playback_paused: false,
                autoplay_active: false,
                hide_empty_rooms: false,
                rooms: Vec::new(),
                ..Default::default()
            },
            public_servers: PublicServerBrowserShellState {
                servers: Vec::new(),
                can_connect: false,
                can_refresh: true,
                can_add_custom_server: true,
            },
            media_search: MediaSearchWorkflowShellState {
                directories: Vec::new(),
                can_browse_directories: true,
                can_search_missing_media: false,
                first_file_timeout_seconds: None,
                search_timeout_seconds: None,
                double_check_interval_seconds: None,
                warning_threshold_seconds: None,
            },
            tls_prompt_expected: false,
            update_notice_expected: false,
            about_dialog_available: true,
        },
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Main-window runtime snapshots must include a non-empty room name.")
    );
}

#[test]
fn gui_shell_app_state_announces_main_window_user_membership_events() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::AnnounceMainWindowUserJoined(
        "alice".to_owned(),
    )));
    assert_eq!(state.main_window.users.len(), 2);
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("alice joined the room.")
    );
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("User joined: alice.")
    );

    assert!(
        state.apply(GuiShellAction::AnnounceSelectedMainWindowUserRenamed(
            "alice-prime".to_owned(),
        ))
    );
    assert_eq!(state.main_window.users[1].username, "alice-prime");
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("alice is now known as alice-prime.")
    );
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("User renamed: alice -> alice-prime.")
    );

    assert!(state.apply(GuiShellAction::AnnounceSelectedMainWindowUserLeft));
    assert_eq!(state.main_window.users.len(), 1);
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("alice-prime left the room.")
    );
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("User removed: alice-prime.")
    );
}

#[test]
fn gui_shell_app_state_commits_native_add_drafts_and_clears_them_after_success() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        player_path: Some("mpv".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::UpdateNewMainWindowUserDraft(
        "alice".to_owned(),
    )));
    assert_eq!(state.new_main_window_user_draft, "alice");
    assert!(state.apply(GuiShellAction::CommitNewMainWindowUser));
    assert_eq!(state.new_main_window_user_draft, "");
    assert_eq!(
        state
            .main_window
            .users
            .last()
            .map(|user| user.username.as_str()),
        Some("alice")
    );

    assert!(state.apply(GuiShellAction::UpdateNewPlaylistEntryDraft(
        "Episode 1.mkv".to_owned(),
    )));
    assert_eq!(state.new_playlist_entry_draft, "Episode 1.mkv");
    assert!(state.apply(GuiShellAction::CommitNewPlaylistEntry));
    assert_eq!(state.new_playlist_entry_draft, "");
    assert_eq!(
        state
            .main_window
            .playlist
            .last()
            .map(|row| row.label.as_str()),
        Some("Episode 1.mkv")
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_main_window_user_announcement_actions() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(
        !state.apply(GuiShellAction::AnnounceSelectedMainWindowUserRenamed(
            "   ".to_owned(),
        ))
    );
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Renamed main-window user names must be non-empty.")
    );

    assert!(!state.apply(GuiShellAction::AnnounceSelectedMainWindowUserLeft));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("The local user row cannot be removed from the main-window shell.")
    );
}

#[test]
fn gui_shell_app_state_announces_playback_readiness_and_autoplay_events() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_toggle_pause = true;

    assert!(state.apply(GuiShellAction::AnnouncePlaybackPaused));
    assert!(state.main_window.playback_paused);
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("Playback paused.")
    );
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Playback paused.")
    );

    assert!(state.apply(GuiShellAction::AnnouncePlaybackResumed));
    assert!(!state.main_window.playback_paused);
    assert!(state.apply(GuiShellAction::AnnounceLocalUserReady));
    assert!(state.main_window.users[0].is_ready);
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("You are now marked ready.")
    );

    assert!(state.apply(GuiShellAction::AnnounceLocalUserNotReady));
    assert!(!state.main_window.users[0].is_ready);
    assert!(state.apply(GuiShellAction::AnnounceAutoplayState(true)));
    assert!(state.main_window.autoplay_active);
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("Autoplay enabled.")
    );
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Autoplay enabled.")
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_playback_readiness_and_autoplay_events() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::AnnouncePlaybackPaused));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Playback pause state cannot change when pause controls are unavailable.")
    );

    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        ready_at_start: Some(true),
        autoplay_initial_state: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_toggle_pause = true;

    assert!(!state.apply(GuiShellAction::AnnouncePlaybackResumed));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Playback is already running.")
    );
    assert!(!state.apply(GuiShellAction::AnnounceLocalUserReady));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("The local user is already marked ready.")
    );
    assert!(!state.apply(GuiShellAction::AnnounceAutoplayState(true)));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Autoplay is already active.")
    );
}

#[test]
fn gui_shell_app_state_starts_controlled_room_and_controller_auth_edit_sessions() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        room: Some("Lounge".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::BeginCreateControlledRoomEdit));
    assert_eq!(state.active_view, GuiShellView::MainWindow);
    assert!(
        state
            .controlled_room_create_session
            .as_ref()
            .is_some_and(|session| !session.is_dirty && session.room_buffer == "Lounge")
    );

    assert!(state.apply(GuiShellAction::UpdateCreateControlledRoomEdit(
        "Studio".to_owned(),
    )));
    assert!(
        state
            .controlled_room_create_session
            .as_ref()
            .is_some_and(|session| session.is_dirty && session.room_buffer == "Studio")
    );
    assert!(state.apply(GuiShellAction::CancelCreateControlledRoomEdit));
    assert!(state.controlled_room_create_session.is_none());

    assert!(state.apply(GuiShellAction::SetMainWindowRoom(
        "+Lounge:ABCDEF123456".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::BeginControllerAuthEdit));
    assert!(
        state
            .controller_auth_edit_session
            .as_ref()
            .is_some_and(|session| {
                !session.is_dirty
                    && session.room_name == "+Lounge:ABCDEF123456"
                    && session.password_buffer.is_empty()
            })
    );

    assert!(
        state.apply(GuiShellAction::UpdateControllerAuthPasswordEdit(
            "ab-123-456".to_owned(),
        ))
    );
    assert!(
        state
            .controller_auth_edit_session
            .as_ref()
            .is_some_and(|session| { session.is_dirty && session.password_buffer == "ab-123-456" })
    );
    assert!(state.apply(GuiShellAction::CancelControllerAuthEdit));
    assert!(state.controller_auth_edit_session.is_none());
}

#[test]
fn gui_shell_app_state_rejects_invalid_controlled_room_and_controller_auth_edit_sessions() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::BeginCreateControlledRoomEdit));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("A joined room is required before creating a controlled room.")
    );

    assert!(!state.apply(GuiShellAction::UpdateCreateControlledRoomEdit(
        "Studio".to_owned(),
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No controlled-room creation editor is currently active.")
    );

    assert!(!state.apply(GuiShellAction::CancelCreateControlledRoomEdit));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No controlled-room creation editor is currently active.")
    );

    let mut joined_room_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            room: Some("Lounge".to_owned()),
            ..StoredClientSettingsMvp::default()
        });
    assert!(!joined_room_state.apply(GuiShellAction::BeginControllerAuthEdit));
    assert_eq!(
        joined_room_state.validation.last_action_error.as_deref(),
        Some("Controller access can only be requested while a controlled room is active.")
    );

    assert!(
        !joined_room_state.apply(GuiShellAction::UpdateControllerAuthPasswordEdit(
            "ab-123-456".to_owned(),
        ))
    );
    assert_eq!(
        joined_room_state.validation.last_action_error.as_deref(),
        Some("No controller-auth editor is currently active.")
    );

    assert!(!joined_room_state.apply(GuiShellAction::CancelControllerAuthEdit));
    assert_eq!(
        joined_room_state.validation.last_action_error.as_deref(),
        Some("No controller-auth editor is currently active.")
    );
}

#[test]
fn gui_shell_app_state_renames_main_window_users_through_edit_sessions() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::AddMainWindowUser("alice".to_owned(),)));
    assert!(state.apply(GuiShellAction::BeginEditSelectedMainWindowUser));
    assert!(state.apply(GuiShellAction::UpdateMainWindowUserEdit(
        "alice-prime".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::CommitMainWindowUserEdit));
    assert_eq!(state.main_window.users[1].username, "alice-prime");
    assert!(state.main_window_user_edit_session.is_none());
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("User renamed: alice -> alice-prime.")
    );

    assert!(state.apply(GuiShellAction::SelectMainWindowUser(0)));
    assert!(state.apply(GuiShellAction::BeginEditSelectedMainWindowUser));
    assert!(state.apply(GuiShellAction::UpdateMainWindowUserEdit(
        TEST_USERNAME.to_owned(),
    )));
    assert!(state.apply(GuiShellAction::CommitMainWindowUserEdit));
    assert_eq!(state.main_window.users[0].username, TEST_USERNAME);
    assert_eq!(
        state.configuration.to_stored_settings().username.as_deref(),
        Some(TEST_USERNAME)
    );
}

#[test]
fn gui_shell_app_state_remaps_main_window_user_edit_sessions_across_runtime_row_reorders() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::AddMainWindowUser("bob".to_owned(),)));
    assert!(state.apply(GuiShellAction::BeginEditSelectedMainWindowUser));
    assert!(state.apply(GuiShellAction::UpdateMainWindowUserEdit(
        "bob-local".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::SelectMainWindowUser(0)));

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "(no room joined)".to_owned(),
            shared_playlist_enabled: false,
            controlled_room_active: false,
            users: vec![
                MainWindowRuntimeUserSnapshot {
                    username: "bob".to_owned(),
                    is_self: false,
                    is_ready: false,
                    is_controller: false,
                    ..Default::default()
                },
                MainWindowRuntimeUserSnapshot {
                    username: "You".to_owned(),
                    is_self: true,
                    is_ready: false,
                    is_controller: false,
                    ..Default::default()
                },
            ],
            playlist: Vec::new(),
            chat: Vec::new(),
            can_toggle_pause: false,
            can_seek: false,
            can_set_ready: true,
            can_manage_playlist: false,
            playback_paused: false,
            autoplay_active: false,
            hide_empty_rooms: false,
            rooms: Vec::new(),
            ..Default::default()
        },
    )));

    assert_eq!(
        state
            .main_window_user_edit_session
            .as_ref()
            .map(|session| session.editing_index),
        Some(0)
    );
    assert_eq!(
        state
            .main_window_user_edit_session
            .as_ref()
            .map(|session| session.username_buffer.as_str()),
        Some("bob-local")
    );
    assert_eq!(state.selection.selected_main_window_user, Some(0));
    assert!(state.main_window.users[0].is_selected);
}

#[test]
fn gui_shell_app_state_keeps_main_window_selection_on_the_active_user_edit_row() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::AddMainWindowUser("bob".to_owned(),)));
    assert!(state.apply(GuiShellAction::BeginEditSelectedMainWindowUser));
    assert!(state.apply(GuiShellAction::SelectMainWindowUser(0)));

    assert_eq!(state.selection.selected_main_window_user, Some(1));
    assert!(state.main_window.users[1].is_selected);
    assert!(
        state
            .main_window_user_edit_session
            .as_ref()
            .is_some_and(|session| session.editing_index == 1)
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_main_window_user_edit_sessions() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::UpdateMainWindowUserEdit(
        "nobody".to_owned(),
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No main-window user edit session is currently active.")
    );

    assert!(state.apply(GuiShellAction::BeginEditSelectedMainWindowUser));
    assert!(state.apply(GuiShellAction::UpdateMainWindowUserEdit("   ".to_owned(),)));
    assert!(!state.apply(GuiShellAction::CommitMainWindowUserEdit));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Renamed main-window user names must be non-empty.")
    );

    assert!(state.apply(GuiShellAction::CancelMainWindowUserEdit));
    assert!(state.apply(GuiShellAction::AddMainWindowUser("alice".to_owned(),)));
    assert!(state.apply(GuiShellAction::SelectMainWindowUser(1)));
    assert!(state.apply(GuiShellAction::BeginEditSelectedMainWindowUser));
    assert!(state.apply(GuiShellAction::UpdateMainWindowUserEdit("You".to_owned(),)));
    assert!(!state.apply(GuiShellAction::CommitMainWindowUserEdit));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("A main-window user with that name already exists.")
    );
}

#[test]
fn gui_shell_app_state_tracks_cross_surface_selection_and_preserves_it_across_resync() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec!["C:/Media".to_owned(), "D:/Archive".to_owned()]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SelectMainWindowUser(0)));
    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(0)));
    assert!(state.apply(GuiShellAction::SelectMenuAction {
        section_index: 3,
        action_index: 1,
    }));
    assert!(state.apply(GuiShellAction::SelectMediaSearchDirectory(1)));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Username",
        value: TEST_USERNAME.to_owned(),
    }));

    assert_eq!(state.selection.selected_main_window_user, Some(0));
    assert_eq!(state.selection.selected_main_window_playlist, Some(0));
    assert_eq!(state.selection.selected_menu_action, Some((3, 1)));
    assert_eq!(state.selection.selected_media_search_directory, Some(1));
    assert!(state.main_window.users[0].is_selected);
    assert!(state.main_window.playlist[0].is_selected);
    assert!(state.menus.sections[3].actions[1].is_selected);
    assert!(!state.menus.sections[3].actions[0].is_selected);
    assert!(!state.media_search.directories[0].is_selected);
    assert!(state.media_search.directories[1].is_selected);

    let rendered = state.render_lines().join("\n");
    assert!(rendered.contains("[Selection] user=0, playlist=0, menu=3:1, media_directory=1"));
}

#[test]
fn gui_shell_app_state_moves_and_removes_playlist_rows() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_manage_playlist = true;
    state.main_window.playlist.push(MainWindowPlaylistRow {
        label: "Second".to_owned(),
        is_selected: false,
    });
    state.main_window.playlist.push(MainWindowPlaylistRow {
        label: "Third".to_owned(),
        is_selected: false,
    });

    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(2)));
    assert!(state.apply(GuiShellAction::MoveSelectedMainWindowPlaylistUp));
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["Playlist pane ready for shared entries", "Third", "Second"]
    );
    assert_eq!(state.selection.selected_main_window_playlist, Some(1));
    assert!(state.apply(GuiShellAction::MoveSelectedMainWindowPlaylistDown));
    assert_eq!(state.selection.selected_main_window_playlist, Some(2));
    assert!(state.apply(GuiShellAction::MoveSelectedMainWindowPlaylistUp));
    assert_eq!(state.selection.selected_main_window_playlist, Some(1));

    assert!(state.apply(GuiShellAction::RemoveSelectedMainWindowPlaylist));
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["Playlist pane ready for shared entries", "Second"]
    );
    assert_eq!(state.selection.selected_main_window_playlist, Some(1));

    assert!(state.apply(GuiShellAction::MoveSelectedMainWindowPlaylistUp));
    assert!(!state.apply(GuiShellAction::MoveSelectedMainWindowPlaylistUp));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("The selected playlist row cannot move further.")
    );
}

#[test]
fn gui_shell_app_state_announces_shared_playlist_events() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "One".to_owned(),
            "Two".to_owned(),
        ]))
    );
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["One", "Two"]
    );
    assert_eq!(state.selection.selected_main_window_playlist, Some(0));
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("Shared playlist loaded (2 entries).")
    );

    assert!(state.apply(GuiShellAction::AnnounceSharedPlaylistSelectionChanged(1)));
    assert_eq!(state.selection.selected_main_window_playlist, Some(1));
    assert!(state.main_window.playlist[1].is_selected);
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Shared playlist selected: Two.")
    );

    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistEntryAdded(
            "Three".to_owned(),
        ))
    );
    assert_eq!(state.selection.selected_main_window_playlist, Some(2));
    assert_eq!(state.main_window.playlist[2].label, "Three");

    assert!(state.apply(GuiShellAction::AnnounceSelectedSharedPlaylistEntryRemoved));
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["One", "Two"]
    );
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Shared playlist entry removed: Three.")
    );

    assert!(state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(Vec::new())));
    assert!(state.main_window.playlist.is_empty());
    assert_eq!(state.selection.selected_main_window_playlist, None);
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("Shared playlist cleared.")
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_shared_playlist_events() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(
        !state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "One".to_owned(),
        ]))
    );
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Shared playlist events are unavailable when shared playlists are disabled.")
    );

    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(
        !state.apply(GuiShellAction::AnnounceSharedPlaylistEntryAdded(
            "   ".to_owned(),
        ))
    );
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Shared playlist entries must be non-empty.")
    );
    assert!(!state.apply(GuiShellAction::AnnounceSharedPlaylistSelectionChanged(1)));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No shared playlist entry exists at the requested index.")
    );
}

#[test]
fn gui_shell_app_state_tracks_playlist_workflow_editors_undo_and_shuffle() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_manage_playlist = true;

    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "Episode 1.mkv".to_owned(),
            "Episode 2.mkv".to_owned(),
            "Episode 3.mkv".to_owned(),
            "Episode 4.mkv".to_owned(),
        ]))
    );
    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(1)));

    assert!(state.apply(GuiShellAction::BeginSharedPlaylistTextEdit));
    assert_eq!(
        state
            .playlist_text_edit_session
            .as_ref()
            .map(|session| session.buffer.as_str()),
        Some("Episode 1.mkv\nEpisode 2.mkv\nEpisode 3.mkv\nEpisode 4.mkv")
    );
    assert!(state.apply(GuiShellAction::UpdateSharedPlaylistTextEdit(
        "Episode 1.mkv\nhttps://example.com/live".to_owned(),
    )));
    let replacement_entries = super::playlist_entries_from_multiline_text(
        state
            .playlist_text_edit_session
            .as_ref()
            .expect("playlist text edit session should remain active")
            .buffer
            .as_str(),
    );
    assert_eq!(
        replacement_entries,
        vec![
            "Episode 1.mkv".to_owned(),
            "https://example.com/live".to_owned(),
        ]
    );
    assert!(state.apply(GuiShellAction::ReplaceSharedPlaylistEntries(
        replacement_entries.clone(),
    )));
    assert_eq!(state.current_shared_playlist_entries(), replacement_entries);
    assert!(state.apply(GuiShellAction::CancelSharedPlaylistTextEdit));
    assert!(state.playlist_text_edit_session.is_none());

    assert!(state.apply(GuiShellAction::BeginSharedPlaylistUrlEdit));
    assert!(
        state.apply(GuiShellAction::UpdateSharedPlaylistUrlEdit(
            "https://example.com/next\nhttps://example.com/bonus\nhttps://example.com/finale"
                .to_owned(),
        ))
    );
    let appended_entries = super::playlist_entries_from_multiline_text(
        state
            .playlist_url_edit_session
            .as_ref()
            .expect("playlist URL edit session should remain active")
            .buffer
            .as_str(),
    );
    assert_eq!(
        appended_entries,
        vec![
            "https://example.com/next".to_owned(),
            "https://example.com/bonus".to_owned(),
            "https://example.com/finale".to_owned(),
        ]
    );
    assert!(state.apply(GuiShellAction::AppendSharedPlaylistEntries(
        appended_entries.clone(),
    )));
    assert!(state.apply(GuiShellAction::CancelSharedPlaylistUrlEdit));
    assert!(state.playlist_url_edit_session.is_none());
    let entries_before_shuffle = state.current_shared_playlist_entries();
    assert_eq!(
        entries_before_shuffle,
        vec![
            "Episode 1.mkv".to_owned(),
            "https://example.com/live".to_owned(),
            "https://example.com/next".to_owned(),
            "https://example.com/bonus".to_owned(),
            "https://example.com/finale".to_owned(),
        ]
    );
    assert_eq!(state.selection.selected_main_window_playlist, Some(4));

    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(1)));
    let mut shuffled_remaining = false;
    for _ in 0..4 {
        if state.apply(GuiShellAction::ShuffleRemainingSharedPlaylist) {
            shuffled_remaining = true;
            break;
        }
    }
    assert!(
        shuffled_remaining,
        "remaining-playlist shuffle should eventually permute the tail"
    );
    let entries_after_remaining_shuffle = state.current_shared_playlist_entries();
    assert_eq!(
        &entries_after_remaining_shuffle[..2],
        &entries_before_shuffle[..2]
    );
    let mut expected_tail = entries_before_shuffle[2..].to_vec();
    let mut actual_tail = entries_after_remaining_shuffle[2..].to_vec();
    expected_tail.sort();
    actual_tail.sort();
    assert_eq!(actual_tail, expected_tail);
    assert_eq!(state.selection.selected_main_window_playlist, Some(1));

    assert!(state.apply(GuiShellAction::UndoSharedPlaylistChange));
    assert_eq!(
        state.current_shared_playlist_entries(),
        entries_before_shuffle
    );
    assert_eq!(state.selection.selected_main_window_playlist, Some(1));

    assert!(state.apply(GuiShellAction::UndoSharedPlaylistChange));
    assert_eq!(
        state.current_shared_playlist_entries(),
        entries_after_remaining_shuffle
    );

    let mut shuffled_entire = false;
    for _ in 0..4 {
        if state.apply(GuiShellAction::ShuffleEntireSharedPlaylist) {
            shuffled_entire = true;
            break;
        }
    }
    assert!(
        shuffled_entire,
        "entire-playlist shuffle should eventually permute the playlist"
    );
    let entries_after_entire_shuffle = state.current_shared_playlist_entries();
    let mut expected_entries = entries_after_remaining_shuffle.clone();
    let mut actual_entries = entries_after_entire_shuffle.clone();
    expected_entries.sort();
    actual_entries.sort();
    assert_eq!(actual_entries, expected_entries);
    assert_eq!(state.selection.selected_main_window_playlist, Some(0));

    assert!(state.apply(GuiShellAction::UndoSharedPlaylistChange));
    assert_eq!(
        state.current_shared_playlist_entries(),
        entries_after_remaining_shuffle
    );

    assert!(state.apply(GuiShellAction::BeginMediaUrlEdit));
    assert!(state.apply(GuiShellAction::UpdateMediaUrlEdit(
        "https://media.example/stream".to_owned(),
    )));
    assert_eq!(
        state
            .media_url_edit_session
            .as_ref()
            .map(|session| (session.buffer.as_str(), session.is_dirty)),
        Some(("https://media.example/stream", true))
    );
    assert!(state.apply(GuiShellAction::CancelMediaUrlEdit));
    assert!(state.media_url_edit_session.is_none());
}

#[test]
fn gui_playlist_file_helpers_roundtrip_and_track_file_actions() {
    let root = test_temp_root("playlist-file-helpers");
    let playlist_path = root.join("shared-playlist.m3u");
    let playlist_path_string = playlist_path.to_string_lossy().into_owned();

    super::save_playlist_entries_to_path(
        &playlist_path_string,
        &[
            "Episode 1.mkv".to_owned(),
            "https://example.com/live".to_owned(),
        ],
    )
    .expect("playlist entries should save to disk");
    assert_eq!(
        std::fs::read_to_string(&playlist_path).expect("saved playlist file should be readable"),
        "Episode 1.mkv\nhttps://example.com/live"
    );

    std::fs::write(
        &playlist_path,
        " Episode 1.mkv \n\n https://example.com/live \n",
    )
    .expect("playlist fixture should be updated");
    assert_eq!(
        super::load_playlist_entries_from_path(&playlist_path_string)
            .expect("playlist entries should load from disk"),
        vec![
            "Episode 1.mkv".to_owned(),
            "https://example.com/live".to_owned(),
        ]
    );

    assert_eq!(
        GuiWidgetEguiRenderer::playlist_load_override_path_from_lookup(&|name| {
            (name == "SYNCPLAY_GUI_TEST_LOAD_PLAYLIST_PATH")
                .then(|| format!("  {playlist_path_string} "))
        }),
        Some(playlist_path_string.clone())
    );
    assert_eq!(
        GuiWidgetEguiRenderer::playlist_save_override_path_from_lookup(&|name| {
            (name == "SYNCPLAY_GUI_TEST_SAVE_PLAYLIST_PATH")
                .then(|| format!("  {playlist_path_string} "))
        }),
        Some(playlist_path_string.clone())
    );

    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    assert!(state.apply(GuiShellAction::LoadSharedPlaylistFromFile {
        path: playlist_path_string.clone(),
        entries: vec![
            "Episode 1.mkv".to_owned(),
            "https://example.com/live".to_owned(),
        ],
        shuffled: false,
    }));
    let expected_load_message =
        format!("Shared playlist loaded from file: {playlist_path_string}.");
    assert_eq!(
        state.current_shared_playlist_entries(),
        vec![
            "Episode 1.mkv".to_owned(),
            "https://example.com/live".to_owned(),
        ]
    );
    assert_eq!(
        state.last_media_dialog_directory.as_deref(),
        Some(root.to_string_lossy().as_ref())
    );
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some(expected_load_message.as_str())
    );

    assert!(state.apply(GuiShellAction::SaveSharedPlaylistToFile(
        playlist_path_string.clone(),
    )));
    let expected_save_message = format!("Shared playlist saved to file: {playlist_path_string}.");
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some(expected_save_message.as_str())
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_shell_app_state_moves_and_removes_media_search_rows() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        media_search_directories: Some(vec![
            "C:/Media".to_owned(),
            "D:/Archive".to_owned(),
            "E:/Incoming".to_owned(),
        ]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SelectMediaSearchDirectory(2)));
    assert!(state.apply(GuiShellAction::MoveSelectedMediaSearchDirectoryUp));
    assert_eq!(
        state
            .media_search
            .directories
            .iter()
            .map(|row| row.path.as_str())
            .collect::<Vec<_>>(),
        vec!["C:/Media", "E:/Incoming", "D:/Archive"]
    );
    assert_eq!(state.selection.selected_media_search_directory, Some(1));
    assert!(state.apply(GuiShellAction::MoveSelectedMediaSearchDirectoryDown));
    assert_eq!(state.selection.selected_media_search_directory, Some(2));
    assert!(state.apply(GuiShellAction::MoveSelectedMediaSearchDirectoryUp));
    assert_eq!(state.selection.selected_media_search_directory, Some(1));

    assert!(state.apply(GuiShellAction::RemoveSelectedMediaSearchDirectory));
    assert_eq!(
        state
            .media_search
            .directories
            .iter()
            .map(|row| row.path.as_str())
            .collect::<Vec<_>>(),
        vec!["C:/Media", "D:/Archive"]
    );
    assert_eq!(state.selection.selected_media_search_directory, Some(1));
    assert_eq!(
        state
            .configuration
            .to_stored_settings()
            .media_search_directories,
        Some(vec!["C:/Media".to_owned(), "D:/Archive".to_owned()])
    );

    assert!(state.apply(GuiShellAction::RemoveSelectedMediaSearchDirectory));
    assert!(state.apply(GuiShellAction::RemoveSelectedMediaSearchDirectory));
    assert!(!state.apply(GuiShellAction::RemoveSelectedMediaSearchDirectory));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No media-search directory is currently selected.")
    );
    assert!(!state.commands.can_search_missing_media);
}

#[test]
fn gui_shell_app_state_handles_media_search_event_actions() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::AnnounceMediaSearchDirectorySelected(0)));
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("Media search directory selected: C:/Media.")
    );

    assert!(
        state.apply(GuiShellAction::AnnounceMediaSearchDirectoryBrowsed(
            "D:/Archive".to_owned(),
        ))
    );
    assert_eq!(state.media_search.directories.len(), 2);
    assert_eq!(state.selection.selected_media_search_directory, Some(1));
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("Media search directory added: D:/Archive.")
    );

    assert!(state.apply(GuiShellAction::BeginMissingMediaSearch));
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::SearchMissingMedia)
    );
    assert!(state.apply(GuiShellAction::CompleteMissingMediaSearch(Some(
        "movie.mkv".to_owned(),
    ))));
    assert_eq!(state.pending_operation, None);
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Missing media found: movie.mkv.")
    );

    assert!(state.apply(GuiShellAction::BeginMissingMediaSearch));
    assert!(state.apply(GuiShellAction::CompleteMissingMediaSearch(None)));
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("Missing media search completed: no match found.")
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_media_search_event_actions() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::AnnounceMediaSearchDirectorySelected(0)));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No media-search directory exists at the requested index.")
    );

    assert!(
        !state.apply(GuiShellAction::AnnounceMediaSearchDirectoryBrowsed(
            "   ".to_owned(),
        ))
    );
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Media search directory cannot be empty.")
    );

    assert!(!state.apply(GuiShellAction::BeginMissingMediaSearch));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Missing-media search is unavailable when search actions are disabled.")
    );

    assert!(
        !state.apply(GuiShellAction::CompleteMissingMediaSearch(Some(
            "movie.mkv".to_owned(),
        )))
    );
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No missing-media search is currently in progress.")
    );
}

#[test]
fn gui_shell_app_state_handles_save_and_playback_toggle_command_actions() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("mpv".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_toggle_pause = true;
    state.refresh_validation();

    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::SaveConfiguration)
    );
    assert!(!state.commands.can_save_configuration);
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Configuration save started.")
    );

    assert!(state.apply(GuiShellAction::CancelConfigurationSave));
    assert_eq!(state.pending_operation, None);
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Configuration save canceled.")
    );

    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    assert!(state.apply(GuiShellAction::CompleteConfigurationSave(
        state.configuration.to_stored_settings(),
    )));
    assert_eq!(state.pending_operation, None);
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("Configuration saved.")
    );

    assert!(state.apply(GuiShellAction::BeginPlaybackPauseToggle));
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::TogglePlaybackPause)
    );
    assert!(!state.commands.can_toggle_pause);
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Playback pause requested.")
    );

    assert!(state.apply(GuiShellAction::CompletePlaybackPauseToggle));
    assert_eq!(state.pending_operation, None);
    assert!(state.main_window.playback_paused);
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("Playback paused.")
    );

    assert!(state.apply(GuiShellAction::BeginPlaybackPauseToggle));
    assert!(state.apply(GuiShellAction::CancelPlaybackPauseToggle));
    assert_eq!(state.pending_operation, None);
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Playback toggle canceled.")
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_save_and_playback_toggle_command_actions() {
    let mut invalid_configuration_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    assert!(
        invalid_configuration_state.apply(GuiShellAction::EditConfigurationText {
            section: "Connection",
            label: "Port",
            value: "70000".to_owned(),
        })
    );
    assert!(!invalid_configuration_state.commands.can_save_configuration);
    assert!(!invalid_configuration_state.apply(GuiShellAction::BeginConfigurationSave));
    assert_eq!(
        invalid_configuration_state
            .validation
            .last_action_error
            .as_deref(),
        Some("Configuration cannot be saved while validation issues remain.")
    );

    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::CompleteConfigurationSave(
        StoredClientSettingsMvp::default(),
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No configuration save is currently in progress.")
    );

    assert!(!state.apply(GuiShellAction::BeginPlaybackPauseToggle));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Playback pause toggling is unavailable when pause controls are disabled.")
    );

    assert!(!state.apply(GuiShellAction::CompletePlaybackPauseToggle));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No playback toggle is currently in progress.")
    );
}

#[test]
fn gui_shell_app_state_handles_configuration_reset_command_actions() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        host: Some("saved.example".to_owned()),
        room: Some("SavedRoom".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(!state.commands.can_reset_configuration);
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Host",
        value: "draft.example".to_owned(),
    }));
    assert_eq!(
        state.configuration.to_stored_settings().host.as_deref(),
        Some("draft.example")
    );
    assert!(state.commands.can_reset_configuration);

    assert!(state.apply(GuiShellAction::BeginConfigurationReset));
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::ResetConfiguration)
    );
    assert!(!state.commands.can_reset_configuration);
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Configuration reset started.")
    );

    assert!(state.apply(GuiShellAction::CancelConfigurationReset));
    assert_eq!(state.pending_operation, None);
    assert_eq!(
        state.configuration.to_stored_settings().host.as_deref(),
        Some("draft.example")
    );
    assert!(state.commands.can_reset_configuration);

    assert!(state.apply(GuiShellAction::BeginConfigurationReset));
    assert!(state.apply(GuiShellAction::CompleteConfigurationReset(
        state.saved_configuration.clone(),
    )));
    assert_eq!(state.pending_operation, None);
    assert_eq!(
        state.configuration.to_stored_settings().host.as_deref(),
        Some("saved.example")
    );
    assert_eq!(
        state.configuration.to_stored_settings().room.as_deref(),
        Some("SavedRoom")
    );
    assert!(!state.commands.can_reset_configuration);
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("Configuration reset to the last saved state.")
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_configuration_reset_command_actions() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::BeginConfigurationReset));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Configuration reset is unavailable with no unsaved changes.")
    );

    assert!(!state.apply(GuiShellAction::CompleteConfigurationReset(
        StoredClientSettingsMvp::default(),
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No configuration reset is currently in progress.")
    );

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Host",
        value: "dirty.example".to_owned(),
    }));
    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    assert!(!state.apply(GuiShellAction::BeginConfigurationReset));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Another GUI operation is already in progress.")
    );
    assert!(!state.apply(GuiShellAction::CancelConfigurationReset));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("The active GUI operation is not a configuration reset.")
    );
}

#[test]
fn gui_shell_app_state_handles_configuration_reload_command_actions() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        host: Some("before.example".to_owned()),
        room: Some("BeforeRoom".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.commands.can_reload_configuration);
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Host",
        value: "dirty.example".to_owned(),
    }));
    assert!(state.commands.can_reset_configuration);

    assert!(state.apply(GuiShellAction::BeginConfigurationReload));
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::ReloadConfiguration)
    );
    assert!(!state.commands.can_reload_configuration);
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Configuration reload started.")
    );

    assert!(state.apply(GuiShellAction::CancelConfigurationReload));
    assert_eq!(state.pending_operation, None);
    assert_eq!(
        state.configuration.to_stored_settings().host.as_deref(),
        Some("dirty.example")
    );
    assert!(state.commands.can_reload_configuration);

    let replacement = StoredClientSettingsMvp {
        host: Some("after.example".to_owned()),
        room: Some("AfterRoom".to_owned()),
        player_path: Some("mpv".to_owned()),
        public_servers: Some(vec![(
            "Primary".to_owned(),
            "syncplay.example:8999".to_owned(),
        )]),
        ..StoredClientSettingsMvp::default()
    };
    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: Vec::new(),
            tls_prompt_expected: true,
            update_notice_expected: true,
            about_dialog_available: false,
        },
    )));
    assert!(state.apply(GuiShellAction::BeginConfigurationReload));
    assert!(state.apply(GuiShellAction::CompleteConfigurationReload(
        replacement.clone(),
    )));
    assert_eq!(state.pending_operation, None);
    assert_eq!(state.configuration.to_stored_settings(), replacement);
    assert_eq!(state.saved_configuration, replacement);
    assert!(!state.commands.can_reset_configuration);
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("Configuration snapshot loaded.")
    );
    assert_eq!(state.active_view, GuiShellView::Configuration);
    assert!(state.menus.tls_prompt_expected);
    assert!(state.menus.update_notice_expected);
    assert!(!state.menus.about_dialog_available);
    let help = state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Help")
        .expect("help section should exist");
    assert!(
        help.actions
            .iter()
            .find(|item| item.label == "About")
            .is_some_and(|item| !item.enabled)
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_configuration_reload_command_actions() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::CompleteConfigurationReload(
        StoredClientSettingsMvp::default(),
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No configuration reload is currently in progress.")
    );

    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    assert!(!state.apply(GuiShellAction::BeginConfigurationReload));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Another GUI operation is already in progress.")
    );
    assert!(!state.apply(GuiShellAction::CancelConfigurationReload));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("The active GUI operation is not a configuration reload.")
    );
}

#[test]
fn gui_shell_app_state_handles_clear_gui_data_command_actions() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        host: Some("saved.example".to_owned()),
        room: Some("SavedRoom".to_owned()),
        public_servers: Some(vec![("Saved".to_owned(), "saved.example:8999".to_owned())]),
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    state.active_view = GuiShellView::PublicServers;
    state.last_media_dialog_directory = Some("D:/Dialogs".to_owned());

    assert!(state.apply(GuiShellAction::BeginClearGuiData));
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::ClearGuiData)
    );
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Clear GUI data started.")
    );

    assert!(state.apply(GuiShellAction::CancelClearGuiData));
    assert_eq!(state.pending_operation, None);
    assert_eq!(state.active_view, GuiShellView::PublicServers);
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Clear GUI data canceled.")
    );

    assert!(state.apply(GuiShellAction::BeginClearGuiData));
    assert!(state.apply(GuiShellAction::CompleteClearGuiData));
    assert_eq!(state.pending_operation, None);
    assert_eq!(state.configuration.launch_mode, GuiLaunchMode::FirstRun);
    assert_eq!(state.active_view, GuiShellView::Configuration);
    assert_eq!(
        state.saved_configuration,
        StoredClientSettingsMvp::default()
    );
    assert!(state.public_servers.servers.is_empty());
    assert!(state.media_search.directories.is_empty());
    assert_eq!(state.last_media_dialog_directory, None);
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("GUI data cleared. First-run configuration restored.")
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_clear_gui_data_command_actions() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::CompleteClearGuiData));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No clear-GUI-data operation is currently in progress.")
    );

    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    assert!(!state.apply(GuiShellAction::BeginClearGuiData));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Another GUI operation is already in progress.")
    );
    assert!(!state.apply(GuiShellAction::CancelClearGuiData));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("The active GUI operation is not a clear-GUI-data request.")
    );
}

#[test]
fn gui_shell_app_state_tracks_configuration_text_edit_session_lifecycle() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::BeginConfigurationTextEdit {
        section: "Connection",
        label: "Host",
    }));
    assert!(state.apply(GuiShellAction::UpdateConfigurationTextEdit(
        "syncplay.example".to_owned(),
    )));
    let rendered = state.render_lines().join("\n");
    assert!(
        rendered
            .contains("[Text Edit] editing=Connection / Host, dirty=yes, buffer=syncplay.example")
    );

    assert!(state.apply(GuiShellAction::CommitConfigurationTextEdit));
    assert!(state.text_edit_session.is_none());
    assert_eq!(
        state.configuration.to_stored_settings().host.as_deref(),
        Some("syncplay.example")
    );
    assert!(
        state
            .render_lines()
            .join("\n")
            .contains("[Text Edit] editing=(none)")
    );

    assert!(state.apply(GuiShellAction::BeginConfigurationTextEdit {
        section: "Connection",
        label: "Host",
    }));
    assert!(state.apply(GuiShellAction::UpdateConfigurationTextEdit(
        "syncplay.cancelled".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::CancelConfigurationTextEdit));
    assert!(state.text_edit_session.is_none());
    assert_eq!(
        state.configuration.to_stored_settings().host.as_deref(),
        Some("syncplay.example")
    );
}

#[test]
fn gui_shell_app_state_tracks_focused_configuration_controls_and_activation() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::FocusConfigurationControl {
        section: "Readiness",
        label: "Autoplay",
    }));
    assert!(state.apply(GuiShellAction::ActivateFocusedConfigurationControl));
    assert_eq!(
        state
            .configuration
            .to_stored_settings()
            .autoplay_initial_state,
        Some(true)
    );
    assert_eq!(
        state
            .focused_configuration_control
            .as_ref()
            .map(|focused| focused.activation_count),
        Some(1)
    );

    assert!(state.apply(GuiShellAction::FocusConfigurationControl {
        section: "Connection",
        label: "Host",
    }));
    assert!(state.apply(GuiShellAction::ActivateFocusedConfigurationControl));
    assert_eq!(
        state
            .text_edit_session
            .as_ref()
            .map(|session| session.label),
        Some("Host")
    );
    assert_eq!(
        state
            .focused_configuration_control
            .as_ref()
            .map(|focused| focused.activation_count),
        Some(1)
    );

    let rendered = state.render_lines().join("\n");
    assert!(
        rendered.contains("[Control Focus] focused=Connection / Host, kind=text, activations=1")
    );
    assert!(rendered.contains("[Text Edit] editing=Connection / Host"));

    assert!(state.apply(GuiShellAction::FocusConfigurationControl {
        section: "Readiness",
        label: "Autoplay",
    }));
    assert_eq!(
        state
            .focused_configuration_control
            .as_ref()
            .map(|focused| (focused.section, focused.label)),
        Some(("Connection", "Host"))
    );

    assert!(state.apply(GuiShellAction::ClearConfigurationControlFocus));
    assert_eq!(
        state
            .focused_configuration_control
            .as_ref()
            .map(|focused| (focused.section, focused.label)),
        Some(("Connection", "Host"))
    );
    assert!(state.apply(GuiShellAction::CancelConfigurationTextEdit));
    assert!(state.apply(GuiShellAction::ClearConfigurationControlFocus));
    assert!(state.focused_configuration_control.is_none());
    assert!(!state.apply(GuiShellAction::ClearConfigurationControlFocus));
}

#[test]
fn gui_shell_app_state_rejects_invalid_configuration_focus_and_activation() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::ActivateFocusedConfigurationControl));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No configuration control is currently focused.")
    );

    assert!(!state.apply(GuiShellAction::FocusConfigurationControl {
        section: "Privacy",
        label: "Trusted Domain Count",
    }));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("The requested configuration control is not focusable.")
    );
}

#[test]
fn gui_shell_app_state_rejects_invalid_configuration_text_edit_sessions() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::BeginConfigurationTextEdit {
        section: "OSD",
        label: "Show OSD",
    }));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("The requested configuration control does not support text-edit sessions.")
    );

    assert!(!state.apply(GuiShellAction::UpdateConfigurationTextEdit(
        "orphan".to_owned(),
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No configuration text-edit session is currently active.")
    );

    assert!(!state.apply(GuiShellAction::CommitConfigurationTextEdit));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No configuration text-edit session is currently active.")
    );
}

#[test]
fn gui_shell_app_state_tracks_pending_operations_and_busy_command_availability() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        public_servers: Some(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())]),
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_toggle_pause = true;
    state.refresh_validation();

    assert!(state.commands.can_save_configuration);
    assert!(!state.commands.can_reset_configuration);
    assert!(state.commands.can_reload_configuration);
    assert!(state.commands.can_connect_public_server);
    assert!(state.commands.can_refresh_public_servers);
    assert!(state.commands.can_search_missing_media);
    assert!(state.commands.can_toggle_pause);
    assert!(state.commands.can_send_chat_message);

    assert!(state.apply(GuiShellAction::BeginPendingOperation(
        GuiPendingOperationKind::RefreshPublicServers,
    )));
    assert_eq!(
        state.pending_operation.as_ref().map(|pending| pending.kind),
        Some(GuiPendingOperationKind::RefreshPublicServers)
    );
    assert!(!state.commands.can_save_configuration);
    assert!(!state.commands.can_reset_configuration);
    assert!(!state.commands.can_reload_configuration);
    assert!(!state.commands.can_connect_public_server);
    assert!(!state.commands.can_refresh_public_servers);
    assert!(!state.commands.can_search_missing_media);
    assert!(!state.commands.can_toggle_pause);
    assert!(!state.commands.can_send_chat_message);

    let busy_render = state.render_lines().join("\n");
    assert!(busy_render.contains("[Commands] busy=yes"));
    assert!(busy_render.contains("[Pending] operation=refresh-public-servers"));

    assert!(!state.apply(GuiShellAction::BeginPendingOperation(
        GuiPendingOperationKind::SendChatMessage,
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Another GUI operation is already in progress.")
    );

    assert!(state.apply(GuiShellAction::CompletePendingOperation));
    assert_eq!(state.pending_operation, None);
    assert!(state.commands.can_save_configuration);
    assert!(!state.commands.can_reset_configuration);
    assert!(state.commands.can_reload_configuration);
    assert!(state.commands.can_connect_public_server);
    assert!(state.commands.can_refresh_public_servers);
    assert!(state.commands.can_search_missing_media);
    assert!(state.commands.can_toggle_pause);
    assert!(state.commands.can_send_chat_message);

    assert!(!state.apply(GuiShellAction::CompletePendingOperation));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("No GUI operation is currently in progress.")
    );
}

#[test]
fn gui_pending_operation_kind_labels_are_stable() {
    let labels = [
        GuiPendingOperationKind::SaveConfiguration.label(),
        GuiPendingOperationKind::ResetConfiguration.label(),
        GuiPendingOperationKind::ReloadConfiguration.label(),
        GuiPendingOperationKind::ConnectPublicServer.label(),
        GuiPendingOperationKind::RefreshPublicServers.label(),
        GuiPendingOperationKind::SearchMissingMedia.label(),
        GuiPendingOperationKind::TogglePlaybackPause.label(),
        GuiPendingOperationKind::SendChatMessage.label(),
    ];

    assert_eq!(
        labels,
        [
            "save-configuration",
            "reset-configuration",
            "reload-configuration",
            "connect-public-server",
            "refresh-public-servers",
            "search-missing-media",
            "toggle-playback-pause",
            "send-chat-message",
        ]
    );
}

#[test]
fn gui_shell_app_state_tracks_validation_issues_and_preserves_view_modal_across_resync() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::SwitchView(GuiShellView::PublicServers)));
    assert!(state.apply(GuiShellAction::OpenModal(GuiShellModal::About)));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Connection",
        label: "Port",
        value: "70000".to_owned(),
    }));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "System",
        label: "Language",
        value: "zz".to_owned(),
    }));

    assert_eq!(state.active_view, GuiShellView::PublicServers);
    assert_eq!(state.open_modal, Some(GuiShellModal::About));
    assert_eq!(state.validation.issues.len(), 2);
    assert!(
        state
            .validation
            .issues
            .iter()
            .any(|issue| issue.scope == "Connection" && issue.label == "Port")
    );
    assert!(
        state
            .validation
            .issues
            .iter()
            .any(|issue| issue.scope == "System" && issue.label == "Language")
    );

    let rendered = state.render_lines().join("\n");
    assert!(rendered.contains("[Validation] status=2 issue(s), last_action_error=(none)"));
    assert!(rendered.contains("Connection / Port: must be a valid TCP port from 1 to 65535."));
    assert!(
        rendered.contains("System / Language: must be one of the supported legacy language tags.")
    );
    assert!(rendered.contains("active_view=public-servers"));
    assert!(rendered.contains("open_modal=about"));
}

#[test]
fn gui_shell_app_state_validates_trusted_domain_configuration_text() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        section: "Privacy",
        label: "Trusted Domains",
        value: "['trusted.example',".to_owned(),
    }));

    assert!(
        state
            .validation
            .issues
            .iter()
            .any(|issue| issue.scope == "Privacy" && issue.label == "Trusted Domains")
    );
    assert!(
            state.render_lines().join("\n").contains(
                "Privacy / Trusted Domains: must be a comma/semicolon-separated list or legacy bracketed list."
            )
        );
    assert_eq!(
        state.configuration.to_stored_settings().trusted_domains,
        None
    );
}

#[test]
fn gui_shell_app_state_tracks_action_errors_for_rejected_inputs() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(!state.apply(GuiShellAction::AddMediaSearchDirectory("   ".to_owned(),)));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Media search directory cannot be empty.")
    );

    assert!(state.apply(GuiShellAction::AddMediaSearchDirectory(
        "C:/Media".to_owned(),
    )));
    assert_eq!(state.validation.last_action_error, None);

    assert!(!state.apply(GuiShellAction::AddMediaSearchDirectory(
        "C:/Media".to_owned(),
    )));
    assert_eq!(
        state.validation.last_action_error.as_deref(),
        Some("Media search directory is already present.")
    );
}

#[test]
fn startup_notice_mentions_configuration_surface_and_grouped_sections() {
    let notice = startup_notice(&StoredClientSettingsMvp::default());

    assert!(notice.contains("[Shell App State]"));
    assert!(notice.contains("active_view=configuration"));
    assert!(notice.contains("open_modal=(none)"));
    assert!(
        notice.contains("[Selection] user=0, playlist=(none), menu=0:0, media_directory=(none)")
    );
    assert!(notice.contains(
            "[Commands] busy=no, save_configuration=yes, reset_configuration=no, reload_configuration=yes, connect_saved_server=no, disconnect_session=no, connect_public_server=no, refresh_public_servers=yes, search_missing_media=no, toggle_pause=no, send_chat_message=no"
        ));
    assert!(notice.contains("[Pending] operation=(none)"));
    assert!(notice.contains("[Control Focus] focused=(none)"));
    assert!(notice.contains("[Public Server Edit] editing=(none)"));
    assert!(notice.contains("[Text Edit] editing=(none)"));
    assert!(notice.contains("[Notifications] count=0"));
    assert!(notice.contains("[Validation] status=clean, last_action_error=(none)"));
    assert!(notice.contains("configuration surface initialized"));
    assert!(notice.contains("[Connection]"));
    assert!(notice.contains("[Readiness]"));
    assert!(notice.contains("[Privacy]"));
    assert!(notice.contains("[Media Search]"));
    assert!(notice.contains("[System]"));
    assert!(notice.contains("[Main Window]"));
    assert!(notice.contains("[Menus & Dialogs]"));
    assert!(notice.contains("[Public Server Browser]"));
    assert!(notice.contains("[Media Search Workflow]"));
    assert!(notice.contains("Playback Controls:"));
    assert!(notice.contains("Dialog Prompts:"));
    assert!(notice.contains("Servers (0):"));
    assert!(notice.contains("Directories (0):"));
    assert!(notice.contains("unified shell app state and action reducer"));
    assert!(notice.contains("Users (1):"));
    assert!(notice.contains("- Host [text]:"));
    assert!(notice.contains("- Server Password [password]:"));
    assert!(notice.contains("Native window widgets are still pending"));
    assert!(!notice.contains("bootstrap placeholder"));
    assert!(notice.contains("de/en/es"));
}

#[test]
fn shell_widget_preview_renders_tree_through_text_preview_renderer() {
    let preview = shell_widget_preview(&StoredClientSettingsMvp::default());

    assert!(!preview.contains("[Widget Tree]"));
    assert!(preview.contains("- Syncplay GUI [panel] id=shell-root"));
    assert!(preview.contains(
        "  - Configuration [panel] id=configuration-root, enabled=yes, selected=yes, value=(none)"
    ));
    assert!(preview.contains(
        "    - Host [text-input] id=config:Connection:Host, enabled=yes, selected=no, value=(unset)"
    ));
    assert!(preview.contains(
        "  - Main Window [panel] id=main-window-root, enabled=yes, selected=no, value=(none)"
    ));
}

#[test]
fn startup_preview_includes_shell_summary_and_widget_tree_preview() {
    let preview = startup_preview(&StoredClientSettingsMvp::default());

    assert!(preview.contains("[Shell App State]"));
    assert!(preview.contains("[Widget Tree]"));
    assert!(preview.contains("- Syncplay GUI [panel] id=shell-root"));
}

#[test]
fn gui_startup_remote_actions_run_due_automatic_update_checks() {
    let settings = StoredClientSettingsMvp {
        check_for_updates_automatically: Some(true),
        last_checked_for_updates: None,
        ..StoredClientSettingsMvp::default()
    };
    let expected = super::remote_services::LegacyUpdateCheckResult {
        status: super::remote_services::LegacyUpdateCheckStatus::UpdateAvailable,
        message: "Remote startup update available.".to_owned(),
        url: Some("https://syncplay.pl/download/".to_owned()),
        public_servers: None,
        checked_at_utc: "2026-03-08 09:10:11.123".to_owned(),
        user_initiated: false,
    };

    let actions = super::gui_startup_remote_actions_with_fetchers(
        &settings,
        std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_800_000_000),
        |_| expected.clone(),
        |_| Ok(Vec::new()),
    );

    assert_eq!(
        actions,
        vec![GuiShellAction::ApplyUpdateCheckResult(expected)]
    );
}

#[test]
fn gui_startup_remote_actions_seed_public_servers_when_cache_is_empty() {
    let settings = StoredClientSettingsMvp {
        check_for_updates_automatically: Some(true),
        last_checked_for_updates: Some("2027-01-14 09:10:11.123".to_owned()),
        ..StoredClientSettingsMvp::default()
    };

    let actions = super::gui_startup_remote_actions_with_fetchers(
        &settings,
        std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_800_000_000),
        |_| panic!("update check should not run when the timestamp is still fresh"),
        |_| Ok(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())]),
    );

    assert_eq!(
        actions,
        vec![GuiShellAction::ApplyStartupPublicServerCache(vec![(
            "Primary".to_owned(),
            "syncplay.pl:8999".to_owned(),
        )])]
    );
}

#[test]
fn gui_startup_actions_from_lookup_prefers_file_public_server_source() {
    let settings = StoredClientSettingsMvp {
        public_servers: Some(vec![("Primary".to_owned(), "file.example:8999".to_owned())]),
        ..StoredClientSettingsMvp::default()
    };

    let actions = super::gui_startup_actions_from_lookup_and_config_path_source(
        |name| match name {
            "SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS_PATH" => Some("public-servers.txt".to_owned()),
            "SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS" => {
                Some(r#"[["Ignored", "inline.example:8999"]]"#.to_owned())
            }
            _ => None,
        },
        &settings,
        None,
    );

    assert_eq!(
            actions,
            vec![
                GuiShellAction::AnnounceSystemChatEvent(
                    "Startup loaded 1 public server from SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS_PATH (public-servers.txt)."
                        .to_owned(),
                ),
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Info,
                    message:
                        "Startup loaded 1 public server from SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS_PATH (public-servers.txt)."
                            .to_owned(),
                },
            ]
        );
}

#[test]
fn gui_startup_actions_from_lookup_reports_client_core_chat_tcp_bootstrap() {
    let actions = super::gui_startup_actions_from_lookup_and_config_path_source(
        |name| match name {
            "SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP" => Some("true".to_owned()),
            "SYNCPLAY_CLIENT_HOST" => Some("syncplay.example".to_owned()),
            "SYNCPLAY_CLIENT_PORT" => Some("8995".to_owned()),
            "SYNCPLAY_CLIENT_USERNAME" => Some(TEST_USERNAME.to_owned()),
            "SYNCPLAY_CLIENT_ROOM" => Some("room-a".to_owned()),
            _ => None,
        },
        &StoredClientSettingsMvp::default(),
        None,
    );
    let expected_message = format!(
        "Startup enabled client-core chat TCP via SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP for syncplay.example:8995 as {TEST_USERNAME} in room room-a."
    );

    assert_eq!(
        actions,
        vec![
            GuiShellAction::AnnounceSystemChatEvent(expected_message.clone()),
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: expected_message,
            },
        ]
    );
}

#[test]
fn gui_startup_actions_from_lookup_reports_client_core_chat_loopback_bootstrap() {
    let actions = super::gui_startup_actions_from_lookup_and_config_path_source(
        |name| match name {
            "SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_LOOPBACK" => Some("true".to_owned()),
            "SYNCPLAY_CLIENT_USERNAME" => Some(TEST_USERNAME.to_owned()),
            "SYNCPLAY_CLIENT_ROOM" => Some("room-a".to_owned()),
            _ => None,
        },
        &StoredClientSettingsMvp::default(),
        None,
    );
    let expected_message = format!(
        "Startup enabled client-core chat loopback via SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_LOOPBACK as {TEST_USERNAME} in room room-a."
    );

    assert_eq!(
        actions,
        vec![
            GuiShellAction::AnnounceSystemChatEvent(expected_message.clone()),
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: expected_message,
            },
        ]
    );
}

#[test]
fn gui_startup_actions_from_lookup_reports_client_core_chat_tcp_defaults() {
    let actions = super::gui_startup_actions_from_lookup_and_config_path_source(
        |name| match name {
            "SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP" => Some("true".to_owned()),
            _ => None,
        },
        &StoredClientSettingsMvp::default(),
        None,
    );

    assert_eq!(
            actions,
            vec![
                GuiShellAction::AnnounceSystemChatEvent(
                    "Startup enabled client-core chat TCP via SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP for 127.0.0.1:8999 as gui-user in room gui-demo. Defaults: host=127.0.0.1, port=8999, user=gui-user, room=gui-demo."
                        .to_owned(),
                ),
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Info,
                    message:
                        "Startup enabled client-core chat TCP via SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP for 127.0.0.1:8999 as gui-user in room gui-demo. Defaults: host=127.0.0.1, port=8999, user=gui-user, room=gui-demo."
                            .to_owned(),
                },
            ]
        );
}

#[test]
fn run_gui_host_with_startup_actions_surfaces_public_server_refresh_source() {
    let settings = StoredClientSettingsMvp {
        public_servers: Some(vec![("Primary".to_owned(), "file.example:8999".to_owned())]),
        ..StoredClientSettingsMvp::default()
    };
    let startup_actions = super::gui_startup_actions_from_lookup_and_config_path_source(
        |name| match name {
            "SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS_PATH" => Some("public-servers.txt".to_owned()),
            _ => None,
        },
        &settings,
        None,
    );
    let mut host = GuiTextPreviewHost;

    let preview = super::run_gui_host_with_startup_actions(&settings, startup_actions, &mut host);

    assert!(preview.contains(
            "Startup loaded 1 public server from SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS_PATH (public-servers.txt)."
        ));
    assert!(preview.contains("[Notifications] count=1"));
}

#[test]
fn run_gui_host_with_startup_actions_surfaces_tcp_bootstrap_and_public_server_sources() {
    let settings = StoredClientSettingsMvp {
        public_servers: Some(vec![("Primary".to_owned(), "file.example:8999".to_owned())]),
        ..StoredClientSettingsMvp::default()
    };
    let startup_actions = super::gui_startup_actions_from_lookup_and_config_path_source(
        |name| match name {
            "SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP" => Some("true".to_owned()),
            "SYNCPLAY_CLIENT_HOST" => Some("syncplay.example".to_owned()),
            "SYNCPLAY_CLIENT_PORT" => Some("8995".to_owned()),
            "SYNCPLAY_CLIENT_USERNAME" => Some(TEST_USERNAME.to_owned()),
            "SYNCPLAY_CLIENT_ROOM" => Some("room-a".to_owned()),
            "SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS_PATH" => Some("public-servers.txt".to_owned()),
            _ => None,
        },
        &settings,
        None,
    );
    let mut host = GuiTextPreviewHost;
    let expected_message = format!(
        "Startup enabled client-core chat TCP via SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP for syncplay.example:8995 as {TEST_USERNAME} in room room-a."
    );

    let preview = super::run_gui_host_with_startup_actions(&settings, startup_actions, &mut host);

    assert!(preview.contains(expected_message.as_str()));
    assert!(preview.contains(
            "Startup loaded 1 public server from SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS_PATH (public-servers.txt)."
        ));
    assert!(
        preview
            .contains("Startup summary: 2 startup notices active. Check system chat for details.")
    );
    assert!(preview.contains("[Notifications] count=1"));
}

#[test]
fn gui_startup_actions_from_lookup_reports_config_path_source() {
    let default_target = test_default_syncplay_config_target();
    let expected_message =
        super::GuiStartupConfigPathSource::DefaultConfigTarget(default_target.clone())
            .startup_message();
    let actions = super::gui_startup_actions_from_lookup_and_config_path_source(
        |_name| None,
        &StoredClientSettingsMvp::default(),
        Some(super::GuiStartupConfigPathSource::DefaultConfigTarget(
            default_target,
        )),
    );

    assert_eq!(
        actions,
        vec![
            GuiShellAction::AnnounceSystemChatEvent(expected_message.clone()),
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: expected_message,
            },
        ]
    );
}

#[test]
fn gui_startup_actions_from_lookup_reports_player_ipc_source_with_client_precedence() {
    let actions = super::gui_startup_actions_from_lookup_and_config_path_source(
        |name| match name {
            "SYNCPLAY_CLIENT_MPV_IPC_PATH" => Some(r#"\\.\pipe\syncplay-mpv"#.to_owned()),
            "SYNCPLAY_MPV_IPC_PATH" => Some("/tmp/ignored-mpv.sock".to_owned()),
            _ => None,
        },
        &StoredClientSettingsMvp::default(),
        None,
    );

    assert_eq!(
            actions,
            vec![
                GuiShellAction::AnnounceSystemChatEvent(
                    r"Startup will try mpv JSON IPC from SYNCPLAY_CLIENT_MPV_IPC_PATH (\\.\pipe\syncplay-mpv)."
                        .to_owned(),
                ),
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Info,
                    message:
                        r"Startup will try mpv JSON IPC from SYNCPLAY_CLIENT_MPV_IPC_PATH (\\.\pipe\syncplay-mpv)."
                            .to_owned(),
                },
            ]
        );
}

#[test]
fn gui_startup_actions_from_lookup_and_config_path_source_consolidates_multi_message_toasts() {
    let actions = super::gui_startup_actions_from_lookup_and_config_path_source(
        |name| match name {
            "SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP" => Some("true".to_owned()),
            "SYNCPLAY_CLIENT_HOST" => Some("syncplay.example".to_owned()),
            "SYNCPLAY_CLIENT_PORT" => Some("8995".to_owned()),
            "SYNCPLAY_CLIENT_USERNAME" => Some(TEST_USERNAME.to_owned()),
            "SYNCPLAY_CLIENT_ROOM" => Some("room-a".to_owned()),
            "SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS_PATH" => Some("public-servers.txt".to_owned()),
            _ => None,
        },
        &StoredClientSettingsMvp {
            public_servers: Some(vec![("Primary".to_owned(), "file.example:8999".to_owned())]),
            ..StoredClientSettingsMvp::default()
        },
        None,
    );

    let notification_messages = actions
        .iter()
        .filter_map(|action| match action {
            GuiShellAction::PushTransientNotification { message, .. } => Some(message.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(notification_messages.len(), 1);
    assert_eq!(
        notification_messages[0],
        "Startup summary: 2 startup notices active. Check system chat for details."
    );
}

#[test]
fn gui_startup_actions_from_lookup_reports_missing_player_ipc_source() {
    let actions =
        super::gui_startup_actions_from_lookup(|_name| None, &StoredClientSettingsMvp::default());

    assert!(actions.iter().any(|action| {
            matches!(
                action,
                GuiShellAction::AnnounceSystemChatEvent(message)
                    if message == "Startup has no explicit mpv JSON IPC path. The GUI will use the saved playerPath when it points to mpv; otherwise set SYNCPLAY_CLIENT_MPV_IPC_PATH or SYNCPLAY_MPV_IPC_PATH to attach an mpv JSON IPC endpoint."
            )
        }));
}

#[test]
fn resolve_syncplay_gui_config_path_source_legacy_compatible_with_reports_default_target() {
    let env_root = test_default_syncplay_config_env_root();
    let env_root_string = env_root.display().to_string();
    let source = super::resolve_syncplay_gui_config_path_source_legacy_compatible_with(
        &|name| match name {
            "APPDATA" if cfg!(windows) => Some(env_root_string.clone()),
            "HOME" if !cfg!(windows) => Some(env_root_string.clone()),
            _ => None,
        },
        || None,
        |_path| false,
    );

    assert_eq!(
        source,
        Some(super::GuiStartupConfigPathSource::DefaultConfigTarget(
            test_default_syncplay_config_target(),
        ))
    );
}

#[test]
fn run_gui_host_with_startup_actions_surfaces_config_path_source() {
    let override_path = std::path::PathBuf::from("custom-config-root").join("syncplay.ini");
    let expected_message =
        super::GuiStartupConfigPathSource::Override(override_path.clone()).startup_message();
    let startup_actions = super::gui_startup_actions_from_lookup_and_config_path_source(
        |_name| None,
        &StoredClientSettingsMvp::default(),
        Some(super::GuiStartupConfigPathSource::Override(override_path)),
    );
    let mut host = GuiTextPreviewHost;

    let preview = super::run_gui_host_with_startup_actions(
        &StoredClientSettingsMvp::default(),
        startup_actions,
        &mut host,
    );

    assert!(preview.contains(expected_message.as_str()));
    assert!(preview.contains("[Notifications] count=1"));
}

#[test]
fn run_gui_host_with_startup_actions_surfaces_player_ipc_source() {
    let startup_actions = super::gui_startup_actions_from_lookup_and_config_path_source(
        |name| match name {
            "SYNCPLAY_MPV_IPC_PATH" => Some("/tmp/syncplay-mpv.sock".to_owned()),
            _ => None,
        },
        &StoredClientSettingsMvp::default(),
        None,
    );
    let mut host = GuiTextPreviewHost;

    let preview = super::run_gui_host_with_startup_actions(
        &StoredClientSettingsMvp::default(),
        startup_actions,
        &mut host,
    );

    assert!(preview.contains(
        "Startup will try mpv JSON IPC from SYNCPLAY_MPV_IPC_PATH (/tmp/syncplay-mpv.sock)."
    ));
    assert!(preview.contains("[Notifications] count=1"));
}

#[test]
fn run_gui_host_passes_shell_state_through_host_boundary() {
    #[derive(Default)]
    struct RecordingHost {
        saw_configuration_view: bool,
    }

    impl GuiAppHost for RecordingHost {
        type Output = String;

        fn render(&mut self, state: SyncplayGuiShellAppState) -> Self::Output {
            self.saw_configuration_view = state.active_view == GuiShellView::Configuration;
            format!("host:{}", state.active_view.label())
        }
    }

    let mut host = RecordingHost::default();
    let rendered = run_gui_host(&StoredClientSettingsMvp::default(), &mut host);

    assert_eq!(rendered, "host:configuration");
    assert!(host.saw_configuration_view);
}

#[test]
fn gui_persisted_ui_state_roundtrips_at_root() {
    let root = test_temp_root("persisted-ui-roundtrip");
    let expected = super::GuiPersistedUiState {
        active_view: Some(GuiShellView::PublicServers),
        selected_public_server_address: Some("custom.example:9001".to_owned()),
        selected_media_search_directory: Some("D:/Media".to_owned()),
        last_media_dialog_directory: Some("E:/Dialogs".to_owned()),
        last_checked_for_updates: None,
        hide_empty_rooms: false,
        public_servers: vec![("Custom".to_owned(), "custom.example:9001".to_owned())],
        ..Default::default()
    };

    super::persist_gui_ui_state_at_root(&root, &expected)
        .expect("persisted GUI state should be written");

    let loaded = super::load_gui_ui_state_from_root(&root)
        .expect("persisted GUI state should be readable")
        .expect("persisted GUI state should not be empty");
    assert_eq!(loaded, expected);
    assert!(super::legacy_gui_qsettings_store_path(&root, "MainWindow").exists());
    assert!(super::legacy_gui_qsettings_store_path(&root, "Interface").exists());
    assert!(super::legacy_gui_qsettings_store_path(&root, "MediaBrowseDialog").exists());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn run_gui_host_with_startup_actions_and_gui_state_restores_non_ini_state() {
    #[derive(Default)]
    struct RecordingHost;

    impl GuiAppHost for RecordingHost {
        type Output = SyncplayGuiShellAppState;

        fn render(&mut self, state: SyncplayGuiShellAppState) -> Self::Output {
            state
        }
    }

    let settings = StoredClientSettingsMvp {
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        ..StoredClientSettingsMvp::default()
    };
    let persisted_ui_state = super::GuiPersistedUiState {
        active_view: Some(GuiShellView::PublicServers),
        selected_public_server_address: Some("custom.example:9001".to_owned()),
        selected_media_search_directory: Some("C:/Media".to_owned()),
        last_media_dialog_directory: Some("D:/Dialogs".to_owned()),
        last_checked_for_updates: None,
        hide_empty_rooms: false,
        public_servers: vec![("Custom".to_owned(), "custom.example:9001".to_owned())],
        ..Default::default()
    };

    let mut host = RecordingHost;
    let state = super::run_gui_host_with_startup_actions_and_gui_state(
        &settings,
        Some(&persisted_ui_state),
        Vec::new(),
        &mut host,
    );

    assert_eq!(state.active_view, GuiShellView::PublicServers);
    assert_eq!(
        state.last_media_dialog_directory.as_deref(),
        Some("D:/Dialogs")
    );
    assert_eq!(
        state
            .public_servers
            .servers
            .iter()
            .map(|row| (row.label.clone(), row.address.clone()))
            .collect::<Vec<_>>(),
        persisted_ui_state.public_servers
    );
    assert_eq!(state.selected_public_server_index(), Some(0));
    assert_eq!(state.selection.selected_media_search_directory, Some(0));
    assert_eq!(
        state.saved_configuration.public_servers,
        Some(vec![(
            "Custom".to_owned(),
            "custom.example:9001".to_owned()
        )])
    );
    assert_eq!(
        state.configuration.to_stored_settings().host.as_deref(),
        Some("custom.example")
    );
    assert_eq!(state.configuration.to_stored_settings().port, Some(9001));
}

#[test]
fn run_gui_host_with_startup_actions_and_gui_state_prefers_gui_public_servers_over_ini_rows() {
    #[derive(Default)]
    struct RecordingHost;

    impl GuiAppHost for RecordingHost {
        type Output = SyncplayGuiShellAppState;

        fn render(&mut self, state: SyncplayGuiShellAppState) -> Self::Output {
            state
        }
    }

    let settings = StoredClientSettingsMvp {
        host: Some("saved.example".to_owned()),
        port: Some(8999),
        public_servers: Some(vec![("Saved".to_owned(), "saved.example:8999".to_owned())]),
        ..StoredClientSettingsMvp::default()
    };
    let persisted_ui_state = super::GuiPersistedUiState {
        active_view: Some(GuiShellView::PublicServers),
        selected_public_server_address: Some("custom.example:9001".to_owned()),
        selected_media_search_directory: None,
        last_media_dialog_directory: None,
        last_checked_for_updates: None,
        hide_empty_rooms: false,
        public_servers: vec![("Custom".to_owned(), "custom.example:9001".to_owned())],
        ..Default::default()
    };

    let mut host = RecordingHost;
    let state = super::run_gui_host_with_startup_actions_and_gui_state(
        &settings,
        Some(&persisted_ui_state),
        Vec::new(),
        &mut host,
    );

    assert_eq!(state.active_view, GuiShellView::PublicServers);
    assert_eq!(
        state
            .public_servers
            .servers
            .iter()
            .map(|row| (row.label.clone(), row.address.clone()))
            .collect::<Vec<_>>(),
        vec![("Custom".to_owned(), "custom.example:9001".to_owned())]
    );
    assert_eq!(
        state.saved_configuration.public_servers,
        Some(vec![(
            "Custom".to_owned(),
            "custom.example:9001".to_owned()
        )])
    );
    assert_eq!(
        state.configuration.to_stored_settings().host.as_deref(),
        Some("custom.example")
    );
    assert_eq!(state.configuration.to_stored_settings().port, Some(9001));
}

#[test]
fn gui_client_core_chat_tcp_bootstrap_from_lookup_uses_existing_client_env_keys() {
    let bootstrap = super::gui_client_core_chat_tcp_bootstrap_from_lookup(|name| match name {
        "SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP" => Some("true".to_owned()),
        "SYNCPLAY_CLIENT_HOST" => Some("syncplay.example".to_owned()),
        "SYNCPLAY_CLIENT_PORT" => Some("8995".to_owned()),
        "SYNCPLAY_CLIENT_USERNAME" => Some(TEST_USERNAME.to_owned()),
        "SYNCPLAY_CLIENT_ROOM" => Some("room-a".to_owned()),
        _ => None,
    })
    .expect("bootstrap lookup should succeed")
    .expect("bootstrap should be enabled");

    assert_eq!(
        bootstrap,
        super::GuiClientCoreChatTcpBootstrap {
            host: "syncplay.example".to_owned(),
            port: 8995,
            username: TEST_USERNAME.to_owned(),
            room: "room-a".to_owned(),
        }
    );
    assert_eq!(bootstrap.host_arg(), "syncplay.example:8995");
}

#[test]
fn gui_client_core_chat_loopback_bootstrap_from_lookup_uses_existing_client_env_keys() {
    let bootstrap = super::gui_client_core_chat_loopback_bootstrap_from_lookup(|name| match name {
        "SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_LOOPBACK" => Some("true".to_owned()),
        "SYNCPLAY_CLIENT_USERNAME" => Some(TEST_USERNAME.to_owned()),
        "SYNCPLAY_CLIENT_ROOM" => Some("room-a".to_owned()),
        _ => None,
    })
    .expect("bootstrap lookup should succeed")
    .expect("bootstrap should be enabled");

    assert_eq!(
        bootstrap,
        super::GuiClientCoreChatLoopbackBootstrap {
            username: TEST_USERNAME.to_owned(),
            room: "room-a".to_owned(),
        }
    );
}

#[test]
fn gui_client_core_chat_tcp_bootstrap_overrides_settings_enable_chat_and_seed_connection() {
    let settings = super::gui_startup_settings_from_lookup_with(
        |name| match name {
            "SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP" => Some("true".to_owned()),
            "SYNCPLAY_CLIENT_HOST" => Some("2001:db8::1".to_owned()),
            "SYNCPLAY_CLIENT_PORT" => Some("9000".to_owned()),
            "SYNCPLAY_CLIENT_USERNAME" => Some("gui-user".to_owned()),
            "SYNCPLAY_CLIENT_ROOM" => Some("gui-room".to_owned()),
            _ => None,
        },
        |_path| Err("unexpected file read".to_owned()),
        || None,
        |_path| false,
        |_path| Ok(None),
    )
    .expect("startup settings lookup should succeed");

    assert_eq!(settings.host.as_deref(), Some("2001:db8::1"));
    assert_eq!(settings.port, Some(9000));
    assert_eq!(settings.username.as_deref(), Some("gui-user"));
    assert_eq!(settings.room.as_deref(), Some("gui-room"));
    assert_eq!(settings.chat_input_enabled, Some(true));
    assert_eq!(settings.chat_output_enabled, Some(true));
}

#[test]
fn gui_client_core_chat_loopback_bootstrap_overlays_settings_enable_chat() {
    let settings = super::gui_startup_settings_from_lookup_with(
        |name| match name {
            "SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_LOOPBACK" => Some("true".to_owned()),
            "SYNCPLAY_CLIENT_USERNAME" => Some("gui-user".to_owned()),
            "SYNCPLAY_CLIENT_ROOM" => Some("gui-room".to_owned()),
            _ => None,
        },
        |_path| Err("unexpected file read".to_owned()),
        || None,
        |_path| false,
        |_path| Ok(None),
    )
    .expect("startup settings lookup should succeed");

    assert_eq!(settings.host, None);
    assert_eq!(settings.port, None);
    assert_eq!(settings.username.as_deref(), Some("gui-user"));
    assert_eq!(settings.room.as_deref(), Some("gui-room"));
    assert_eq!(settings.chat_input_enabled, Some(true));
    assert_eq!(settings.chat_output_enabled, Some(true));
}

#[test]
fn gui_startup_settings_from_lookup_seeds_public_servers_without_tcp_bootstrap() {
    let settings = super::gui_startup_settings_from_lookup(
        |name| match name {
            "SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS" => Some(
                r#"[[" Primary ", " syncplay.pl:8999 "], ["Duplicate", "SYNCPLAY.PL:8999"]]"#
                    .to_owned(),
            ),
            _ => None,
        },
        |_path| Err("unexpected file read".to_owned()),
    )
    .expect("startup settings lookup should succeed");

    assert_eq!(
        settings.public_servers,
        Some(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())])
    );
    assert_eq!(settings.host, None);
    assert_eq!(settings.username, None);
}

#[test]
fn gui_startup_settings_from_lookup_merges_tcp_bootstrap_and_file_public_servers() {
    let settings = super::gui_startup_settings_from_lookup(
        |name| match name {
            "SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP" => Some("true".to_owned()),
            "SYNCPLAY_CLIENT_HOST" => Some("syncplay.example".to_owned()),
            "SYNCPLAY_CLIENT_PORT" => Some("8995".to_owned()),
            "SYNCPLAY_CLIENT_USERNAME" => Some(TEST_USERNAME.to_owned()),
            "SYNCPLAY_CLIENT_ROOM" => Some("room-a".to_owned()),
            "SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS_PATH" => Some("public-servers.txt".to_owned()),
            _ => None,
        },
        |path| {
            if path == "public-servers.txt" {
                Ok(r#"[[" File Primary ", " file.example:8999 "]]"#.to_owned())
            } else {
                Err("unexpected file read".to_owned())
            }
        },
    )
    .expect("startup settings lookup should succeed");

    assert_eq!(settings.host.as_deref(), Some("syncplay.example"));
    assert_eq!(settings.port, Some(8995));
    assert_eq!(settings.username.as_deref(), Some(TEST_USERNAME));
    assert_eq!(settings.room.as_deref(), Some("room-a"));
    assert_eq!(settings.chat_input_enabled, Some(true));
    assert_eq!(settings.chat_output_enabled, Some(true));
    assert_eq!(
        settings.public_servers,
        Some(vec![(
            "File Primary".to_owned(),
            "file.example:8999".to_owned()
        )])
    );
}

#[test]
fn gui_startup_settings_from_lookup_loads_stored_config_before_rendering() {
    let settings = super::gui_startup_settings_from_lookup_with(
        |name| match name {
            "SYNCPLAY_CLIENT_CONFIG_PATH" => Some("stored-syncplay.ini".to_owned()),
            _ => None,
        },
        |_path| Err("unexpected file read".to_owned()),
        || None,
        |_path| false,
        |path| {
            assert_eq!(path, std::path::Path::new("stored-syncplay.ini"));
            Ok(Some(StoredClientSettingsMvp {
                host: Some("persisted.example".to_owned()),
                port: Some(8999),
                username: Some("persisted-user".to_owned()),
                room: Some("persisted-room".to_owned()),
                player_path: Some("C:/Players/mpv.exe".to_owned()),
                ..StoredClientSettingsMvp::default()
            }))
        },
    )
    .expect("startup settings lookup should succeed");

    assert_eq!(settings.host.as_deref(), Some("persisted.example"));
    assert_eq!(settings.port, Some(8999));
    assert_eq!(settings.username.as_deref(), Some("persisted-user"));
    assert_eq!(settings.room.as_deref(), Some("persisted-room"));
    assert_eq!(settings.player_path.as_deref(), Some("C:/Players/mpv.exe"));

    let state = SyncplayGuiShellAppState::from_stored_settings(&settings);
    assert_eq!(
        state.configuration.settings.host.as_deref(),
        Some("persisted.example")
    );
    assert_eq!(state.configuration.settings.port, Some(8999));
    assert_eq!(
        state.configuration.settings.username.as_deref(),
        Some("persisted-user")
    );
    assert_eq!(
        state.configuration.settings.room.as_deref(),
        Some("persisted-room")
    );
    assert_eq!(
        state.configuration.settings.player_path.as_deref(),
        Some("C:/Players/mpv.exe")
    );
}

#[test]
fn gui_startup_settings_from_lookup_overlays_bootstrap_on_loaded_config() {
    let settings = super::gui_startup_settings_from_lookup_with(
        |name| match name {
            "SYNCPLAY_CLIENT_CONFIG_PATH" => Some("stored-syncplay.ini".to_owned()),
            "SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP" => Some("true".to_owned()),
            "SYNCPLAY_CLIENT_HOST" => Some("runtime.example".to_owned()),
            "SYNCPLAY_CLIENT_PORT" => Some("8995".to_owned()),
            "SYNCPLAY_CLIENT_USERNAME" => Some("runtime-user".to_owned()),
            "SYNCPLAY_CLIENT_ROOM" => Some("runtime-room".to_owned()),
            _ => None,
        },
        |_path| Err("unexpected file read".to_owned()),
        || None,
        |_path| false,
        |_path| {
            Ok(Some(StoredClientSettingsMvp {
                host: Some("persisted.example".to_owned()),
                port: Some(7777),
                username: Some("persisted-user".to_owned()),
                room: Some("persisted-room".to_owned()),
                player_path: Some("C:/Players/mpv.exe".to_owned()),
                ready_at_start: Some(true),
                ..StoredClientSettingsMvp::default()
            }))
        },
    )
    .expect("startup settings lookup should succeed");

    assert_eq!(settings.host.as_deref(), Some("runtime.example"));
    assert_eq!(settings.port, Some(8995));
    assert_eq!(settings.username.as_deref(), Some("runtime-user"));
    assert_eq!(settings.room.as_deref(), Some("runtime-room"));
    assert_eq!(settings.chat_input_enabled, Some(true));
    assert_eq!(settings.chat_output_enabled, Some(true));
    assert_eq!(settings.player_path.as_deref(), Some("C:/Players/mpv.exe"));
    assert_eq!(settings.ready_at_start, Some(true));
}

#[test]
fn gui_client_core_chat_tcp_bootstrap_from_lookup_rejects_invalid_port() {
    let error = super::gui_client_core_chat_tcp_bootstrap_from_lookup(|name| match name {
        "SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP" => Some("on".to_owned()),
        "SYNCPLAY_CLIENT_PORT" => Some("70000".to_owned()),
        _ => None,
    })
    .expect_err("invalid port should be rejected");

    assert_eq!(
        error,
        "SYNCPLAY_CLIENT_PORT must be a valid TCP port from 1 to 65535.".to_owned()
    );
}

#[test]
fn gui_text_preview_host_uses_summary_and_widget_tree_output() {
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    let mut host = GuiTextPreviewHost;
    let rendered = host.render(state);

    assert!(rendered.contains("[Shell App State]"));
    assert!(rendered.contains("[Widget Tree]"));
    assert!(rendered.contains("- Syncplay GUI [panel] id=shell-root"));
}

#[test]
fn gui_widget_egui_renderer_rebuilds_widget_tree_from_renderer_contract() {
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    let expected_tree = state.shell_widget_tree();
    let mut renderer = GuiWidgetEguiRenderer::default();

    state.render_shell_widgets(&mut renderer);

    assert_eq!(renderer.root(), Some(&expected_tree));
}

#[test]
fn gui_widget_egui_renderer_exposes_modal_specific_titles_and_actions() {
    assert_eq!(
        GuiWidgetEguiRenderer::modal_window_title(GuiShellModal::TlsCertificatePrompt),
        "TLS Certificate Prompt"
    );
    assert_eq!(
        GuiWidgetEguiRenderer::modal_actions(GuiShellModal::TlsCertificatePrompt),
        vec![
            (
                "shell:modal:tls:trust",
                "Trust Certificate",
                GuiShellAction::TrustTlsCertificatePrompt,
            ),
            (
                "shell:modal:tls:reject",
                "Reject Certificate",
                GuiShellAction::RejectTlsCertificatePrompt,
            ),
            (
                "shell:modal:tls:help",
                "Open Help",
                GuiShellAction::AnnounceHelpRequested,
            ),
        ]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::modal_actions(GuiShellModal::UpdateNotice),
        vec![
            (
                "shell:modal:update:dismiss",
                "Dismiss Notice",
                GuiShellAction::DismissUpdateNotice,
            ),
            (
                "shell:modal:update:help",
                "Open Help",
                GuiShellAction::AnnounceHelpRequested,
            ),
            (
                "shell:modal:update:check-again",
                "Check Again",
                GuiShellAction::AnnounceUpdateNoticeAvailable,
            ),
        ]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::modal_actions(GuiShellModal::About),
        vec![
            (
                "shell:modal:about:help",
                "Open Help",
                GuiShellAction::AnnounceHelpRequested,
            ),
            (
                "shell:modal:about:update",
                "Check for Updates",
                GuiShellAction::AnnounceUpdateNoticeAvailable
            ),
        ]
    );
}

#[test]
fn gui_widget_egui_renderer_prefers_selected_media_search_directory_for_native_browse_dialog() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        media_search_directories: Some(vec!["C:/Media".to_owned(), "D:/AltMedia".to_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    assert!(state.apply(GuiShellAction::SelectMediaSearchDirectory(1)));

    assert_eq!(
        GuiWidgetEguiRenderer::media_search_dialog_start_directory(&state),
        Some("D:/AltMedia")
    );
}

#[test]
fn gui_widget_egui_renderer_prefers_last_media_dialog_directory_for_native_browse_dialog() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        media_search_directories: Some(vec!["C:/Media".to_owned(), "D:/AltMedia".to_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    state.last_media_dialog_directory = Some("E:/Dialogs".to_owned());
    assert!(state.apply(GuiShellAction::SelectMediaSearchDirectory(1)));

    assert_eq!(
        GuiWidgetEguiRenderer::media_search_dialog_start_directory(&state),
        Some("E:/Dialogs")
    );
}

#[test]
fn gui_widget_egui_renderer_reads_media_search_browse_override_path_from_lookup() {
    assert_eq!(
        GuiWidgetEguiRenderer::media_search_browse_override_path_from_lookup(&|name| match name {
            "SYNCPLAY_GUI_TEST_MEDIA_SEARCH_BROWSE_PATH" => {
                Some("  C:/Smoke/Media Search  ".to_owned())
            }
            _ => None,
        }),
        Some("C:/Smoke/Media Search".to_owned())
    );
    assert_eq!(
        GuiWidgetEguiRenderer::media_search_browse_override_path_from_lookup(&|_name| None),
        None
    );
}

#[test]
fn gui_widget_egui_renderer_reads_media_file_pick_override_paths_from_lookup() {
    assert_eq!(
        GuiWidgetEguiRenderer::media_file_pick_override_paths_from_lookup(&|name| match name {
            "SYNCPLAY_GUI_TEST_OPEN_MEDIA_FILE_PATHS" => {
                Some("  C:/Smoke/episode1.mkv | | D:/Alt/episode2.mp4  ".to_owned())
            }
            _ => None,
        }),
        Some(vec![
            "C:/Smoke/episode1.mkv".to_owned(),
            "D:/Alt/episode2.mp4".to_owned(),
        ])
    );
    assert_eq!(
        GuiWidgetEguiRenderer::media_file_pick_override_paths_from_lookup(&|_name| None),
        None
    );
    assert_eq!(
        GuiWidgetEguiRenderer::media_file_pick_override_paths_from_lookup(&|name| match name {
            "SYNCPLAY_GUI_TEST_OPEN_MEDIA_FILE_PATHS" => Some("   |  ".to_owned()),
            _ => None,
        }),
        None
    );
}

#[test]
fn gui_widget_egui_renderer_prefers_playlist_target_for_hovered_shared_playlist_drops() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_manage_playlist = true;
    let request = GuiWidgetEguiRenderer::dropped_files_request_for_input(
        &state,
        true,
        None,
        None,
        vec![egui::DroppedFile {
            path: Some(PathBuf::from("C:/Media/episode1.mkv")),
            ..Default::default()
        }],
    )
    .expect("dropped-file request should be derived");

    assert_eq!(
        request,
        GuiDroppedFilesRequest {
            target: GuiDroppedFilesTarget::Playlist,
            paths: vec!["C:/Media/episode1.mkv".to_owned()],
        }
    );
}

#[test]
fn gui_widget_egui_renderer_falls_back_to_window_target_when_shared_playlist_drop_is_unavailable() {
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    let request = GuiWidgetEguiRenderer::dropped_files_request_for_input(
        &state,
        true,
        None,
        None,
        vec![egui::DroppedFile {
            path: Some(PathBuf::from("C:/Media/movie.mkv")),
            ..Default::default()
        }],
    )
    .expect("dropped-file request should be derived");

    assert_eq!(
        request.target,
        GuiDroppedFilesTarget::Window,
        "non-shared-playlist drops should fall back to the generic window open path"
    );
}

#[test]
fn gui_native_app_reads_drag_and_drop_test_override_from_lookup() {
    assert_eq!(
        GuiNativeApp::test_drop_request_from_lookup(&|name| match name {
            "SYNCPLAY_GUI_TEST_DROP_FILE_PATHS" => {
                Some("  C:/Drops/episode1.mkv | D:/Alt/episode2.mp4 ".to_owned())
            }
            "SYNCPLAY_GUI_TEST_DROP_TARGET" => Some(" playlist ".to_owned()),
            _ => None,
        })
        .expect("drop override should parse"),
        Some(GuiDroppedFilesRequest {
            target: GuiDroppedFilesTarget::Playlist,
            paths: vec![
                "C:/Drops/episode1.mkv".to_owned(),
                "D:/Alt/episode2.mp4".to_owned(),
            ],
        })
    );
    assert_eq!(
        GuiNativeApp::test_drop_request_from_lookup(&|_name| None)
            .expect("missing drop override should not fail"),
        None
    );
}

#[test]
fn gui_persisted_config_runtime_owner_startup_player_lookup_honors_test_player_env() {
    let owner = super::GuiPersistedConfigRuntimeOwner::with_config_path_and_startup_player_lookup(
        Some(PathBuf::from("C:/Config/syncplay.ini")),
        &|name| match name {
            "SYNCPLAY_GUI_ENABLE_TEST_PLAYER" => Some("true".to_owned()),
            _ => None,
        },
    );
    assert_eq!(
        owner.player.as_ref().map(|player| player.name()),
        Some("test")
    );
    assert_eq!(owner.player_unavailability_reason, None);

    let detached_owner =
        super::GuiPersistedConfigRuntimeOwner::with_config_path_and_startup_player_lookup(
            Some(PathBuf::from("C:/Config/syncplay.ini")),
            &|_name| None,
        );
    assert!(detached_owner.player.is_none());
    assert_eq!(detached_owner.player_unavailability_reason, None);
}

#[test]
fn gui_persisted_config_runtime_owner_uses_saved_player_path_for_managed_mpv_launch_state() {
    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    let config_path =
        std::env::temp_dir().join(format!("syncplay-gui-startup-player-{unique_suffix}.ini"));
    let mut per_player_arguments = std::collections::BTreeMap::new();
    per_player_arguments.insert(
        "C:/missing/mpv.exe".to_owned(),
        vec![
            "--profile=syncplay".to_owned(),
            "--keep-open=yes".to_owned(),
        ],
    );
    super::upsert_syncplay_ini_stored_client_settings_mvp_at_path(
        &config_path,
        &StoredClientSettingsMvp {
            player_path: Some("C:/missing/mpv.exe".to_owned()),
            per_player_arguments: Some(per_player_arguments),
            chat_input_enabled: Some(true),
            show_osd: Some(false),
            ..StoredClientSettingsMvp::default()
        },
    )
    .expect("startup-player seed should write syncplay.ini");

    let owner = super::GuiPersistedConfigRuntimeOwner::with_config_path_and_startup_player_lookup(
        Some(config_path.clone()),
        &|_name| None,
    );
    match &owner.player_launch_state {
        super::GuiPlayerLaunchRuntimeState::ManagedMpv(config) => {
            assert_eq!(config.requested_player_path, "C:/missing/mpv.exe");
            assert_eq!(
                config.extra_args,
                vec![
                    "--profile=syncplay".to_owned(),
                    "--keep-open=yes".to_owned()
                ]
            );
            assert!(!config.ui_settings.show_osd);
            assert!(config.ui_settings.chat_input_enabled);
        }
        other => panic!("expected managed-mpv launch state, got {other:?}"),
    }
    assert!(owner.player.is_none());
    assert!(
        owner
            .player_unavailability_reason
            .as_deref()
            .is_some_and(|message| {
                message.contains("GUI-owned mpv launch failed from saved player path")
            }),
        "startup attach should fail deterministically for a missing mpv binary"
    );

    let _ = std::fs::remove_file(config_path);
}

#[cfg(windows)]
#[test]
#[ignore = "local smoke test; requires standalone mpv binary"]
fn gui_persisted_config_runtime_owner_starts_real_managed_mpv_from_saved_config() {
    let default_mpv = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mpv/mpv.exe");
    let mpv_path = std::env::var_os("SYNCPLAY_MPV_SMOKE_BIN")
        .map(PathBuf::from)
        .unwrap_or(default_mpv);
    if !mpv_path.is_file() {
        panic!("expected mpv binary at {}", mpv_path.display());
    }

    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    let config_path =
        std::env::temp_dir().join(format!("syncplay-gui-real-mpv-startup-{unique_suffix}.ini"));
    super::upsert_syncplay_ini_stored_client_settings_mvp_at_path(
        &config_path,
        &StoredClientSettingsMvp {
            player_path: Some(mpv_path.to_string_lossy().into_owned()),
            ..StoredClientSettingsMvp::default()
        },
    )
    .expect("real-mpv startup seed should write syncplay.ini");

    let owner = super::GuiPersistedConfigRuntimeOwner::with_config_path_and_startup_player_lookup(
        Some(config_path.clone()),
        &|_name| None,
    );
    assert_eq!(
        owner.player.as_ref().map(|player| player.name()),
        Some("mpv")
    );
    assert!(owner.managed_mpv_process.is_some());
    assert_eq!(owner.player_unavailability_reason, None);

    drop(owner);
    let _ = std::fs::remove_file(config_path);
}

#[test]
fn gui_shell_app_state_edits_room_history_from_configuration_surface() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        room_list: Some(vec!["beta".to_owned(), "alpha".to_owned()]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::BeginRoomHistoryEdit));
    let configuration_tree = state.configuration_widget_tree();
    let editor = configuration_tree
        .find("room-history:edit:entries")
        .expect("room-history text area should exist while editing");
    assert_eq!(editor.kind, GuiWidgetKind::TextArea);
    assert_eq!(editor.value.as_deref(), Some("beta\nalpha"));

    assert!(state.apply(GuiShellAction::UpdateRoomHistoryEdit(
        "zeta\n\nalpha\nbeta".to_owned()
    )));
    assert!(state.apply(GuiShellAction::CommitRoomHistoryEdit));

    assert_eq!(
        state.configuration.to_stored_settings().room_list,
        Some(vec![
            "alpha".to_owned(),
            "beta".to_owned(),
            "zeta".to_owned(),
        ])
    );
    assert_eq!(
        state
            .configuration
            .control_value("Connection", "Room History"),
        Some("3")
    );
    assert!(state.room_history_edit_session.is_none());
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Room history updated: 3 entries.")
    );
}

#[test]
fn gui_shell_app_state_cancels_room_history_edit_without_changing_settings() {
    let original = vec!["beta".to_owned(), "alpha".to_owned()];
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        room_list: Some(original.clone()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::BeginRoomHistoryEdit));
    assert!(state.apply(GuiShellAction::UpdateRoomHistoryEdit(
        "zeta\nalpha".to_owned()
    )));
    assert!(state.apply(GuiShellAction::CancelRoomHistoryEdit));

    assert_eq!(
        state.configuration.to_stored_settings().room_list,
        Some(original)
    );
    assert!(state.room_history_edit_session.is_none());
}

#[test]
fn gui_preview_runtime_bridge_maps_selected_media_files_to_preview_actions() {
    let shared_playlist_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            shared_playlist_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        });
    let fallback_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    let mut runtime = GuiPreviewRuntimeBridge;

    assert_eq!(
        runtime.actions_for_selected_media_files(
            &shared_playlist_state,
            vec![
                "C:/Media/Episode 1.mkv".to_owned(),
                "C:/Media/Episode 2.mkv".to_owned()
            ],
        ),
        vec![
            GuiShellAction::SwitchView(GuiShellView::MainWindow),
            GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
                "Episode 1.mkv".to_owned(),
                "Episode 2.mkv".to_owned(),
            ]),
        ]
    );
    assert_eq!(
        runtime.actions_for_selected_media_files(
            &fallback_state,
            vec!["C:/Media/movie.mkv".to_owned()],
        ),
        vec![
            GuiShellAction::SwitchView(GuiShellView::MainWindow),
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "Media file selected: C:/Media/movie.mkv.".to_owned(),
            },
            GuiShellAction::AnnounceSystemChatEvent(
                "Media file selected: C:/Media/movie.mkv.".to_owned(),
            ),
        ]
    );
}

#[test]
fn gui_preview_runtime_bridge_imports_playlist_files_for_shared_playlist_ingest() {
    let root = test_temp_root("preview-shared-playlist-drop");
    let playlist_path = root.join("drop-list.m3u");
    std::fs::write(&playlist_path, "episode1.mkv\nhttps://example.com/live\n")
        .expect("preview playlist import fixture should be written");

    assert_eq!(
        GuiPreviewRuntimeBridge::preview_open_media_file_actions(
            vec![playlist_path.to_string_lossy().into_owned()],
            true,
        ),
        vec![
            GuiShellAction::SwitchView(GuiShellView::MainWindow),
            GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
                "episode1.mkv".to_owned(),
                "https://example.com/live".to_owned(),
            ]),
        ]
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_widget_egui_renderer_maps_playlist_workflow_controls_to_actions() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_manage_playlist = true;
    state.main_window.playback.can_toggle_pause = true;

    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "Episode 1.mkv".to_owned(),
            "https://example.com/live".to_owned(),
        ]))
    );
    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(1)));

    let shell_tree = state.shell_widget_tree();
    let add_url_button = shell_tree.find("main-window:playlist:add-url").unwrap();
    let open_url_button = shell_tree.find("main-window:playlist:open-url").unwrap();
    let open_selected_button = shell_tree
        .find("main-window:playlist:open-selected")
        .unwrap();
    let open_selected_folder_button = shell_tree
        .find("main-window:playlist:open-selected-folder")
        .unwrap();
    let trust_selected_button = shell_tree
        .find("main-window:playlist:trust-selected")
        .unwrap();
    let shuffle_remaining_button = shell_tree
        .find("main-window:playlist:shuffle-remaining")
        .unwrap();
    let shuffle_entire_button = shell_tree
        .find("main-window:playlist:shuffle-entire")
        .unwrap();
    let undo_button = shell_tree.find("main-window:playlist:undo").unwrap();
    let edit_button = shell_tree.find("main-window:playlist:edit").unwrap();

    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, add_url_button),
        vec![GuiShellAction::BeginSharedPlaylistUrlEdit]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, open_url_button),
        vec![GuiShellAction::BeginMediaUrlEdit]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, open_selected_button),
        vec![GuiShellAction::RequestMainWindowUserMediaOpen(
            "https://example.com/live".to_owned(),
        )]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, open_selected_folder_button),
        vec![GuiShellAction::RequestMainWindowUserContainingFolderOpen(
            "https://example.com/live".to_owned(),
        )]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, trust_selected_button),
        vec![GuiShellAction::AddTrustedDomain("example.com".to_owned())]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, shuffle_remaining_button),
        vec![GuiShellAction::ShuffleRemainingSharedPlaylist]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, shuffle_entire_button),
        vec![GuiShellAction::ShuffleEntireSharedPlaylist]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, undo_button),
        vec![GuiShellAction::UndoSharedPlaylistChange]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, edit_button),
        vec![GuiShellAction::BeginSharedPlaylistTextEdit]
    );

    assert!(state.apply(GuiShellAction::BeginSharedPlaylistTextEdit));
    assert!(state.apply(GuiShellAction::UpdateSharedPlaylistTextEdit(
        "Episode 9.mkv\nhttps://example.com/live".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::BeginSharedPlaylistUrlEdit));
    assert!(state.apply(GuiShellAction::UpdateSharedPlaylistUrlEdit(
        "https://example.com/extra".to_owned(),
    )));
    assert!(state.apply(GuiShellAction::BeginMediaUrlEdit));

    let shell_tree = state.shell_widget_tree();
    let playlist_text_node = shell_tree.find("main-window:playlist-edit:text").unwrap();
    let playlist_text_commit = shell_tree.find("main-window:playlist-edit:commit").unwrap();
    let playlist_text_cancel = shell_tree.find("main-window:playlist-edit:cancel").unwrap();
    let playlist_url_text_node = shell_tree
        .find("main-window:playlist-url-edit:text")
        .unwrap();
    let playlist_url_commit = shell_tree
        .find("main-window:playlist-url-edit:commit")
        .unwrap();
    let playlist_url_cancel = shell_tree
        .find("main-window:playlist-url-edit:cancel")
        .unwrap();
    let media_url_text_node = shell_tree.find("main-window:media-url-edit:text").unwrap();
    let media_url_cancel = shell_tree
        .find("main-window:media-url-edit:cancel")
        .unwrap();

    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, playlist_text_commit),
        vec![
            GuiShellAction::ReplaceSharedPlaylistEntries(vec![
                "Episode 9.mkv".to_owned(),
                "https://example.com/live".to_owned(),
            ]),
            GuiShellAction::CancelSharedPlaylistTextEdit,
        ]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, playlist_text_cancel),
        vec![GuiShellAction::CancelSharedPlaylistTextEdit]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, playlist_url_commit),
        vec![
            GuiShellAction::AppendSharedPlaylistEntries(vec![
                "https://example.com/extra".to_owned(),
            ]),
            GuiShellAction::CancelSharedPlaylistUrlEdit,
        ]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, playlist_url_cancel),
        vec![GuiShellAction::CancelSharedPlaylistUrlEdit]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, media_url_cancel),
        vec![GuiShellAction::CancelMediaUrlEdit]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_text_input_node(
            &state,
            playlist_text_node,
            "Episode 10.mkv",
            true,
            false,
        ),
        Some(vec![GuiShellAction::UpdateSharedPlaylistTextEdit(
            "Episode 10.mkv".to_owned(),
        )])
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_text_input_node(
            &state,
            playlist_url_text_node,
            "https://example.com/final",
            true,
            false,
        ),
        Some(vec![GuiShellAction::UpdateSharedPlaylistUrlEdit(
            "https://example.com/final".to_owned(),
        )])
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_text_input_node(
            &state,
            media_url_text_node,
            "https://media.example/stream",
            true,
            true,
        ),
        Some(vec![
            GuiShellAction::UpdateMediaUrlEdit("https://media.example/stream".to_owned()),
            GuiShellAction::RequestMainWindowUserMediaOpen(
                "https://media.example/stream".to_owned(),
            ),
            GuiShellAction::CancelMediaUrlEdit,
        ])
    );
}

#[test]
fn gui_widget_egui_renderer_maps_surface_button_and_list_nodes_to_actions() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())]),
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        shared_playlist_enabled: Some(true),
        player_path: Some("mpv".to_owned()),
        room: Some("Lounge".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        MainWindowRuntimeSnapshot {
            room_name: "Lounge".to_owned(),
            shared_playlist_enabled: true,
            controlled_room_active: false,
            hide_empty_rooms: false,
            rooms: vec![
                MainWindowRuntimeRoomSnapshot {
                    room_name: "Lounge".to_owned(),
                    is_controlled: false,
                    has_named_users: true,
                },
                MainWindowRuntimeRoomSnapshot {
                    room_name: "Cinema".to_owned(),
                    is_controlled: false,
                    has_named_users: true,
                },
            ],
            users: vec![
                browser_runtime_user(TEST_USERNAME, "Lounge", true, false, false),
                MainWindowRuntimeUserSnapshot {
                    has_file: true,
                    file_name: Some("https://example.com/live".to_owned()),
                    file_is_url: true,
                    file_is_trusted: false,
                    ..browser_runtime_user("Bob", "Lounge", false, false, false)
                },
            ],
            playlist: vec!["Episode 1".to_owned()],
            can_toggle_pause: true,
            can_set_ready: true,
            can_set_others_ready: true,
            ..Default::default()
        }
    )));
    state.commands.can_disconnect_session = true;
    let shell_tree = state.shell_widget_tree();
    let public_servers_surface = shell_tree.find("public-servers-root").unwrap();
    let menu_action = shell_tree.find("menus:action:0:0").unwrap();
    let exit_menu_action = shell_tree.find("menus:action:0:3").unwrap();
    let seek_menu_action = shell_tree.find("menus:action:1:3").unwrap();
    let quick_open_media = shell_tree.find("shell:quick:open-media-file").unwrap();
    let playlist_row = shell_tree.find("main-window:playlist:0").unwrap();
    let browser_join_button = shell_tree.find("main-window:room-group:1:join").unwrap();
    let user_open_button = shell_tree.find("main-window:user:1:open").unwrap();
    let user_trust_button = shell_tree.find("main-window:user:1:trust").unwrap();
    let user_ready_button = shell_tree.find("main-window:user:1:ready").unwrap();
    let room_set_button = shell_tree.find("main-window:room:set").unwrap();
    let room_join_button = shell_tree.find("main-window:room:join").unwrap();
    let room_leave_button = shell_tree.find("main-window:room:leave").unwrap();
    let pause_button = shell_tree.find("main-window:control:toggle-pause").unwrap();
    let playlist_add_input = shell_tree.find("main-window:playlist:new").unwrap();
    let playlist_add_button = shell_tree.find("main-window:playlist:add").unwrap();
    let playlist_remove_button = shell_tree.find("main-window:playlist:remove").unwrap();
    let edit_button = shell_tree.find("public-servers:command:edit").unwrap();
    let directory_remove_button = shell_tree.find("media-search:directory:remove").unwrap();

    assert_eq!(
        GuiWidgetEguiRenderer::action_for_surface_node(public_servers_surface),
        Some(GuiShellAction::SwitchView(GuiShellView::PublicServers))
    );
    assert!(GuiWidgetEguiRenderer::is_open_media_file_menu_action(
        &state,
        menu_action
    ));
    assert!(!GuiWidgetEguiRenderer::is_exit_menu_action(
        &state,
        menu_action
    ));
    assert!(!GuiWidgetEguiRenderer::is_open_media_file_menu_action(
        &state,
        exit_menu_action
    ));
    assert!(GuiWidgetEguiRenderer::is_exit_menu_action(
        &state,
        exit_menu_action
    ));
    assert!(!GuiWidgetEguiRenderer::is_seek_menu_action(
        &state,
        menu_action
    ));
    assert!(GuiWidgetEguiRenderer::is_seek_menu_action(
        &state,
        seek_menu_action
    ));
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, menu_action),
        vec![
            GuiShellAction::SelectMenuAction {
                section_index: 0,
                action_index: 0,
            },
            GuiShellAction::TriggerSelectedMenuAction,
        ]
    );
    assert_eq!(quick_open_media.kind, GuiWidgetKind::Button);
    assert!(quick_open_media.enabled);
    assert_eq!(
        GuiWidgetEguiRenderer::action_for_list_item_node(playlist_row),
        Some(GuiShellAction::SelectMainWindowPlaylist(0))
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, browser_join_button),
        vec![GuiShellAction::JoinMainWindowRoom("Cinema".to_owned())]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, user_open_button),
        vec![GuiShellAction::RequestMainWindowUserMediaOpen(
            "https://example.com/live".to_owned()
        )]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, user_trust_button),
        vec![GuiShellAction::AddTrustedDomain("example.com".to_owned())]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, user_ready_button),
        vec![GuiShellAction::RequestMainWindowUserReady {
            username: "Bob".to_owned(),
            ready: true,
        }]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, room_set_button),
        vec![GuiShellAction::SetMainWindowRoom("Lounge".to_owned())]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, room_join_button),
        vec![GuiShellAction::JoinMainWindowRoom("Lounge".to_owned())]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, room_leave_button),
        vec![GuiShellAction::LeaveMainWindowRoom]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, pause_button),
        vec![GuiShellAction::BeginPlaybackPauseToggle]
    );
    assert_eq!(playlist_add_input.kind, GuiWidgetKind::TextInput);
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, playlist_add_button),
        vec![GuiShellAction::CommitNewPlaylistEntry]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, playlist_remove_button),
        vec![GuiShellAction::RemoveSelectedMainWindowPlaylist]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, edit_button),
        vec![GuiShellAction::BeginEditSelectedPublicServer]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::media_search_dialog_start_directory(&state),
        Some("C:/Media")
    );
    assert!(GuiWidgetEguiRenderer::should_show_manual_pending_controls(
        "save-configuration",
        true
    ));
    assert!(!GuiWidgetEguiRenderer::should_show_manual_pending_controls(
        "save-configuration",
        false
    ));
    assert!(!GuiWidgetEguiRenderer::should_show_manual_pending_controls(
        "(none)", true
    ));
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, directory_remove_button),
        vec![GuiShellAction::RemoveSelectedMediaSearchDirectory]
    );

    let mut controlled_room_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            room: Some("Lounge".to_owned()),
            ..StoredClientSettingsMvp::default()
        });
    assert!(controlled_room_state.apply(GuiShellAction::BeginCreateControlledRoomEdit));
    let controlled_room_tree = controlled_room_state.main_window_widget_tree();
    let create_commit_button = controlled_room_tree
        .find("main-window:controlled-room-create:commit")
        .unwrap();
    let create_cancel_button = controlled_room_tree
        .find("main-window:controlled-room-create:cancel")
        .unwrap();
    let create_actions = GuiWidgetEguiRenderer::actions_for_button_node(
        &controlled_room_state,
        create_commit_button,
    );
    assert_eq!(create_actions.len(), 2);
    assert!(matches!(
        &create_actions[0],
        GuiShellAction::RequestControllerAuth { room, password }
            if room == "Lounge"
                && password.len() == 10
                && password.chars().enumerate().all(|(index, c)| match index {
                    2 | 6 => c == '-',
                    0 | 1 => c.is_ascii_uppercase(),
                    _ => c.is_ascii_digit(),
                })
    ));
    assert_eq!(
        create_actions[1],
        GuiShellAction::CancelCreateControlledRoomEdit
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(
            &controlled_room_state,
            create_cancel_button
        ),
        vec![GuiShellAction::CancelCreateControlledRoomEdit]
    );

    let mut controller_auth_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            room: Some("+Lounge:ABCDEF123456".to_owned()),
            ..StoredClientSettingsMvp::default()
        });
    assert!(controller_auth_state.apply(GuiShellAction::BeginControllerAuthEdit));
    assert!(
        controller_auth_state.apply(GuiShellAction::UpdateControllerAuthPasswordEdit(
            "ab-123-456".to_owned(),
        ))
    );
    let controller_auth_tree = controller_auth_state.main_window_widget_tree();
    let controller_auth_commit_button = controller_auth_tree
        .find("main-window:controller-auth:commit")
        .unwrap();
    let controller_auth_cancel_button = controller_auth_tree
        .find("main-window:controller-auth:cancel")
        .unwrap();
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(
            &controller_auth_state,
            controller_auth_commit_button
        ),
        vec![
            GuiShellAction::RequestControllerAuth {
                room: "+Lounge:ABCDEF123456".to_owned(),
                password: "ab-123-456".to_owned(),
            },
            GuiShellAction::CancelControllerAuthEdit,
        ]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(
            &controller_auth_state,
            controller_auth_cancel_button
        ),
        vec![GuiShellAction::CancelControllerAuthEdit]
    );
}

#[test]
fn gui_semantic_driver_runs_widget_id_scenario_without_platform_ui() {
    let scenario = super::gui_semantic_scenario_named("configuration-surface-flow")
        .expect("configuration semantic scenario should exist");
    let driver = scenario
        .run()
        .unwrap_or_else(|error| panic!("{} should execute successfully: {error}", scenario.name()));

    let stored = driver.state().configuration.to_stored_settings();
    let saved = &driver.state().saved_configuration;
    assert_eq!(stored.host.as_deref(), Some("syncplay.pl"));
    assert_eq!(stored.port, Some(8999));
    assert_eq!(stored.username.as_deref(), Some("smoke-user"));
    assert_eq!(stored.room.as_deref(), Some("smoke-room"));
    assert_eq!(
        stored.media_search_directories,
        Some(vec!["C:/Media".to_owned()])
    );
    assert_eq!(saved.host.as_deref(), Some("syncplay.example"));
    assert_eq!(saved.port, Some(8999));
    assert_eq!(saved.username.as_deref(), Some("smoke-user"));
    assert_eq!(saved.room.as_deref(), Some("smoke-room"));
    assert_eq!(
        saved.player_path.as_deref(),
        Some("C:/Windows/System32/notepad.exe")
    );
    assert_eq!(saved.ready_at_start, Some(true));
    assert_eq!(saved.autoplay_initial_state, Some(true));
    assert_eq!(saved.autoplay_require_same_filenames, Some(true));
    assert_eq!(saved.shared_playlist_enabled, Some(true));
    assert_eq!(saved.pause_on_leave, Some(true));
    assert_eq!(saved.unpause_action, Some(UnpauseActionMode::Always));
    assert_eq!(
        saved.autoplay_min_users,
        Some(AutoplayThresholdOverride::Set(3))
    );
    assert_eq!(saved.filename_privacy_mode, Some(PrivacyMode::SendHashed));
    assert_eq!(saved.filesize_privacy_mode, Some(PrivacyMode::DoNotSend));
    assert_eq!(saved.only_switch_to_trusted_domains, Some(true));
    assert_eq!(
        saved.trusted_domains,
        Some(vec![
            "youtube.com".to_owned(),
            "*.example.com/videos".to_owned()
        ])
    );
    assert_eq!(saved.rewind_on_desync, Some(true));
    assert_eq!(saved.fastforward_on_desync, Some(true));
    assert_eq!(saved.slow_on_desync, Some(true));
    assert_eq!(saved.dont_slow_down_with_me, Some(true));
    assert_eq!(saved.rewind_threshold_seconds, Some(1.25));
    assert_eq!(saved.fastforward_threshold_seconds, Some(3.5));
    assert_eq!(saved.slowdown_threshold_seconds, Some(2.25));
    assert_eq!(
        saved.media_search_directories,
        Some(vec!["C:/Media".to_owned()])
    );
    assert_eq!(saved.folder_search_first_file_timeout_seconds, Some(3.0));
    assert_eq!(saved.folder_search_timeout_seconds, Some(30.0));
    assert_eq!(saved.folder_search_double_check_interval_seconds, Some(2.5));
    assert_eq!(saved.folder_search_warning_threshold_seconds, Some(7.5));
    assert_eq!(saved.chat_input_enabled, Some(true));
    assert_eq!(saved.chat_output_enabled, Some(true));
    assert_eq!(saved.chat_direct_input, Some(true));
    assert_eq!(saved.chat_move_osd, Some(true));
    assert_eq!(saved.chat_max_lines, Some(7));
    assert_eq!(saved.chat_input_font_family.as_deref(), Some("Consolas"));
    assert_eq!(
        saved.chat_output_font_family.as_deref(),
        Some("Cascadia Mono")
    );
    assert_eq!(saved.show_osd, Some(true));
    assert_eq!(saved.show_duration_notification, Some(true));
    assert_eq!(saved.show_same_room_osd, Some(true));
    assert_eq!(saved.show_osd_warnings, Some(true));
    assert_eq!(saved.show_noncontroller_osd, Some(true));
    assert_eq!(saved.show_different_room_osd, Some(true));
    assert_eq!(saved.show_contact_info, Some(true));
    assert_eq!(saved.language.as_deref(), Some("pt_BR"));
    assert_eq!(saved.check_for_updates_automatically, Some(true));
    assert!(driver.state().menus.tls_prompt_expected);
    assert!(!driver.state().menus.update_notice_expected);
    assert!(
        driver
            .widget("public-servers:row:0")
            .expect("public-server row should exist")
            .selected
    );
    assert_eq!(
        driver.state().selection.selected_media_search_directory,
        Some(0)
    );
}

#[test]
fn gui_semantic_driver_runs_runtime_snapshot_chat_scenario_without_platform_ui() {
    let scenario = super::gui_semantic_scenario_named("runtime-chat-flow")
        .expect("runtime chat semantic scenario should exist");
    let driver = scenario
        .run()
        .unwrap_or_else(|error| panic!("{} should execute successfully: {error}", scenario.name()));

    assert_eq!(driver.state().main_window.room_name, "sync-room");
    assert_eq!(
        driver.state().selection.selected_main_window_playlist,
        Some(1)
    );
    let last_chat = driver
        .state()
        .main_window
        .chat
        .last()
        .expect("local chat completion should append a row");
    assert_eq!(last_chat.sender, "smoke-user");
    assert_eq!(last_chat.message, "hello room");
}

#[test]
fn gui_semantic_driver_runs_core_shell_smoke_scenario_without_platform_ui() {
    let scenario = super::gui_semantic_scenario_named("core-shell-smoke-flow")
        .expect("core shell smoke semantic scenario should exist");
    let driver = scenario
        .run()
        .unwrap_or_else(|error| panic!("{} should execute successfully: {error}", scenario.name()));

    let stored = driver.state().configuration.to_stored_settings();
    assert_eq!(stored.host.as_deref(), Some("custom.example"));
    assert_eq!(stored.port, Some(9001));
    assert_eq!(stored.public_servers.as_ref().map(Vec::len), Some(3));
    assert_eq!(driver.active_view_label(), "main-window");
    assert_eq!(driver.active_modal_label(), "none");
    assert_eq!(driver.pending_operation_label(), "none");
}

#[test]
fn gui_semantic_driver_runs_playlist_workflow_scenario_without_platform_ui() {
    let scenario = super::gui_semantic_scenario_named("playlist-workflow-flow")
        .expect("playlist workflow semantic scenario should exist");
    let driver = scenario
        .run()
        .unwrap_or_else(|error| panic!("{} should execute successfully: {error}", scenario.name()));

    assert_eq!(driver.state().main_window.room_name, "sync-room");
    assert_eq!(
        driver
            .state()
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["episode3.mkv"]
    );
    assert_eq!(
        driver.state().selection.selected_main_window_playlist,
        Some(0)
    );
    assert!(driver.state().playlist_text_edit_session.is_none());
    assert!(driver.state().playlist_url_edit_session.is_none());
    assert!(driver.state().media_url_edit_session.is_none());
}

#[test]
fn gui_semantic_scenarios_expose_named_catalog_and_parse_scripts() {
    assert_eq!(
        super::gui_semantic_scenario_names(),
        &[
            "configuration-surface-flow",
            "core-shell-smoke-flow",
            "runtime-chat-flow",
            "runtime-transport-churn-flow",
            "drag-and-drop-ingest-flow",
            "playlist-workflow-flow",
            "persistence-reset-flow",
            "detached-runtime-ownership-flow",
            "live-python-peer-connect-flow",
            "live-python-peer-controlled-room-flow",
        ]
    );
    assert!(
        super::semantic_smoke::gui_semantic_scenario_script("configuration-surface-flow")
            .expect("built-in configuration scenario should expose a script")
            .contains("setting\tpublic-server\tPrimary\tsyncplay.pl:8999")
    );
    assert!(
        super::semantic_smoke::gui_semantic_scenario_script("core-shell-smoke-flow")
            .expect("built-in core shell smoke scenario should expose a script")
            .contains("close-modal")
    );
    assert!(
        super::semantic_smoke::gui_semantic_scenario_script("runtime-chat-flow")
            .expect("built-in runtime scenario should expose a script")
            .contains("push-chat-message\tbob\thello from tcp")
    );
    assert!(
        super::semantic_smoke::gui_semantic_scenario_script("runtime-transport-churn-flow")
            .expect("built-in runtime churn scenario should expose a script")
            .contains("apply-main-window-runtime\tsmoke-room\ttrue\ttrue\tfalse")
    );
    assert!(
        super::semantic_smoke::gui_semantic_scenario_script("drag-and-drop-ingest-flow")
            .expect("drag-and-drop scenario should expose a script")
            .contains("drop-media-files\tplaylist")
    );
    assert!(
        super::semantic_smoke::gui_semantic_scenario_script("playlist-workflow-flow")
            .expect("playlist workflow scenario should expose a script")
            .contains("main-window:playlist:edit")
    );
    assert!(
        super::semantic_smoke::gui_semantic_scenario_script("persistence-reset-flow")
            .expect("persistence/reset scenario should expose a script description")
            .contains("PersistenceRoom")
    );
    assert!(
        super::semantic_smoke::gui_semantic_scenario_script("detached-runtime-ownership-flow")
            .expect("detached runtime ownership scenario should expose a script description")
            .contains("semantic-user")
    );
    assert!(
        super::semantic_smoke::gui_semantic_scenario_script("live-python-peer-connect-flow")
            .expect("live Python interop scenario should expose a script description")
            .contains("interop-py-peer")
    );
    assert!(
        super::semantic_smoke::gui_semantic_scenario_script(
            "live-python-peer-controlled-room-flow"
        )
        .expect("live Python controlled-room scenario should expose a script description")
        .contains("+interop-room:447CE7E3548D:AB-123-456")
    );
    assert!(
        super::semantic_smoke::gui_semantic_scenario_script("missing-scenario").is_none(),
        "unknown semantic scenario scripts should not resolve"
    );
    let descriptors = super::semantic_smoke::gui_semantic_scenario_descriptors();
    assert_eq!(descriptors.len(), 10);
    assert_eq!(descriptors[0].name, "configuration-surface-flow");
    assert!(descriptors[0].description.contains("configuration fields"));
    assert!(
        descriptors[0]
            .script
            .contains("setting\tpublic-server\tPrimary\tsyncplay.pl:8999")
    );
    assert_eq!(descriptors[1].name, "core-shell-smoke-flow");
    assert!(descriptors[1].description.contains("non-transport"));
    assert!(descriptors[1].script.contains("clear-notifications"));
    assert_eq!(descriptors[3].name, "runtime-transport-churn-flow");
    assert!(
        descriptors[3]
            .description
            .contains("startup/post-chat/reconnect")
    );
    assert!(descriptors[3].script.contains("reconnect-post2.mkv"));
    assert_eq!(descriptors[4].name, "drag-and-drop-ingest-flow");
    assert!(
        descriptors[4]
            .description
            .contains("window drops open media")
    );
    assert!(descriptors[4].script.contains("drop-media-files\twindow"));
    assert_eq!(descriptors[5].name, "playlist-workflow-flow");
    assert!(descriptors[5].description.contains("playlist editor"));
    assert!(
        descriptors[5]
            .script
            .contains("main-window:playlist:add-url")
    );
    assert_eq!(descriptors[6].name, "persistence-reset-flow");
    assert!(descriptors[6].description.contains("clear-GUI-data"));
    assert!(descriptors[6].script.contains("PersistenceRoom"));
    assert_eq!(descriptors[7].name, "detached-runtime-ownership-flow");
    assert!(
        descriptors[7]
            .description
            .contains("detached public-server connect")
    );
    assert!(descriptors[7].script.contains("semantic-user"));
    assert_eq!(descriptors[8].name, "live-python-peer-connect-flow");
    assert!(descriptors[8].description.contains("Python reference peer"));
    assert!(descriptors[8].script.contains("interop-room"));
    assert_eq!(descriptors[9].name, "live-python-peer-controlled-room-flow");
    assert!(descriptors[9].description.contains("controlled room"));
    assert!(descriptors[9].script.contains("+interop-room:447CE7E3548D"));
    assert!(
        super::gui_semantic_scenario_named("missing-scenario").is_none(),
        "unknown semantic scenarios should not resolve"
    );

    let parsed = super::GuiSemanticStep::parse_script(
        "\
# comment\n\
activate\tconfiguration-root\n\
assert-selected\tconfiguration-root\ttrue\n\
assert-value\tconfig:Connection:Host\t<none>\n\
assert-pending\tnone\n\
complete-pending\n\
complete-pending-runtime\n\
open-media-files\tC:/Media/open-target.mkv\n\
drop-media-files\tplaylist\tC:/Media/episode1.mkv|C:/Media/episode2.mkv\n\
close-modal\n\
clear-notifications\n",
    )
    .expect("semantic step script should parse");
    assert_eq!(
        parsed,
        vec![
            super::GuiSemanticStep::activate("configuration-root"),
            super::GuiSemanticStep::assert_widget_selected("configuration-root", true),
            super::GuiSemanticStep::assert_widget_value("config:Connection:Host", None),
            super::GuiSemanticStep::assert_pending(None),
            super::GuiSemanticStep::CompletePending,
            super::GuiSemanticStep::CompletePendingRuntime,
            super::GuiSemanticStep::OpenMediaFiles(vec!["C:/Media/open-target.mkv".to_owned(),]),
            super::GuiSemanticStep::DropMediaFiles {
                target: super::GuiDroppedFilesTarget::Playlist,
                paths: vec![
                    "C:/Media/episode1.mkv".to_owned(),
                    "C:/Media/episode2.mkv".to_owned(),
                ],
            },
            super::GuiSemanticStep::CloseModal,
            super::GuiSemanticStep::ClearNotifications,
        ]
    );

    let parsed_runtime = super::GuiSemanticStep::parse_script(
            "\
apply-main-window-runtime\troom-a\ttrue\tfalse\tfalse\ttrue\ttrue\tfalse\ttrue\tself,true,true,false|bob,false,false,true\tvideo1.mkv|video2.mkv\tsystem>connected\n\
apply-main-window-playlist-selection\t1\n\
push-chat-message\tbob\thello\n\
assert-value\tmain-window:chat-input\t<empty>\n",
        )
        .expect("runtime semantic step script should parse");
    assert_eq!(parsed_runtime.len(), 4);
    assert!(matches!(
        &parsed_runtime[0],
        super::GuiSemanticStep::ApplyMainWindowRuntimeSnapshot(snapshot)
            if snapshot.room_name == "room-a"
            && snapshot.shared_playlist_enabled
            && !snapshot.playback_paused
            && snapshot.playlist == vec!["video1.mkv".to_owned(), "video2.mkv".to_owned()]
            && snapshot.users.len() == 2
            && snapshot.chat.len() == 1
    ));
    assert_eq!(
        parsed_runtime[1],
        super::GuiSemanticStep::ApplyMainWindowPlaylistSelection(Some(1))
    );
    assert_eq!(
        parsed_runtime[2],
        super::GuiSemanticStep::PushChatMessage {
            sender: "bob".to_owned(),
            message: "hello".to_owned(),
        }
    );
    assert_eq!(
        parsed_runtime[3],
        super::GuiSemanticStep::assert_widget_value("main-window:chat-input", Some(""))
    );
}

#[test]
fn gui_semantic_scenario_runner_reports_named_results_from_lookup() {
    assert_eq!(
        super::gui_semantic_scenario_name_from_lookup(|name| {
            (name == "SYNCPLAY_GUI_SEMANTIC_SCENARIO")
                .then(|| "configuration-surface-flow".to_owned())
        }),
        Some("configuration-surface-flow".to_owned())
    );
    let report = super::run_gui_semantic_scenario_from_lookup(|name| {
        (name == "SYNCPLAY_GUI_SEMANTIC_SCENARIO").then(|| "configuration-surface-flow".to_owned())
    })
    .expect("named semantic scenario should run")
    .expect("lookup should produce a report");
    assert_eq!(report.scenario, "configuration-surface-flow");
    assert_eq!(report.view, "media-search");
    assert_eq!(report.modal, "none");
    assert_eq!(report.pending, "none");
    assert!(report.widgets > 0);
    assert!(
        report
            .render(super::semantic_smoke::GuiSemanticOutputFormat::Text)
            .contains("result=ok\n")
    );

    let json_report =
        super::semantic_smoke::run_syncplay_gui_semantic_cli_from_lookup(|name| match name {
            "SYNCPLAY_GUI_SEMANTIC_SCENARIO" => Some("configuration-surface-flow".to_owned()),
            "SYNCPLAY_GUI_SEMANTIC_OUTPUT" => Some("json".to_owned()),
            _ => None,
        })
        .expect("json semantic scenario should run")
        .expect("lookup should produce json output");
    assert!(json_report.starts_with("{\"result\":\"ok\","));
    assert!(json_report.contains("\"scenario\":\"configuration-surface-flow\""));
    assert!(
        super::gui_semantic_output_format_from_lookup(|name| {
            (name == "SYNCPLAY_GUI_SEMANTIC_OUTPUT").then(|| "yaml".to_owned())
        })
        .expect_err("unknown semantic output format should fail")
        .contains("Expected 'text' or 'json'")
    );

    let mut script_path = std::env::temp_dir();
    let unique_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_nanos();
    script_path.push(format!(
        "syncplay-gui-semantic-scenario-{}-{unique_id}.txt",
        std::process::id()
    ));
    std::fs::write(
        &script_path,
        "\
meta\tname\tfile-seeded-flow\n\
meta\texpect-view\tpublic-servers\n\
meta\texpect-modal\tnone\n\
meta\texpect-pending\tnone\n\
setting\thost\tfile-script.example\n\
setting\tport\t8999\n\
setting\tpublic-server\tMirror\tmirror.example:8999\n\
assert-selected\tconfiguration-root\ttrue\n\
assert-value\tconfig:Connection:Host\tfile-script.example\n\
assert-value\tconfig:Connection:Port\t8999\n\
activate\tpublic-servers-root\n\
assert-selected\tpublic-servers-root\ttrue\n\
assert-label\tpublic-servers:row:0\tMirror\n",
    )
    .expect("semantic script file should be created");
    let script_path_string = script_path.to_string_lossy().into_owned();
    let script_report = super::run_gui_semantic_scenario_from_lookup(|name| match name {
        "SYNCPLAY_GUI_SEMANTIC_SCENARIO_PATH" => Some(script_path_string.clone()),
        "SYNCPLAY_GUI_SEMANTIC_SCENARIO" => Some("configuration-surface-flow".to_owned()),
        _ => None,
    })
    .expect("script semantic scenario should run")
    .expect("lookup should produce a script report");
    assert_eq!(script_report.scenario, "file-seeded-flow");
    assert_eq!(script_report.view, "public-servers");

    std::fs::write(
        &script_path,
        "\
meta\texpect-view\tmain-window\n\
assert-selected\tconfiguration-root\ttrue\n",
    )
    .expect("mismatch semantic script file should be updated");
    assert!(
        super::run_gui_semantic_scenario_from_lookup(|name| match name {
            "SYNCPLAY_GUI_SEMANTIC_SCENARIO_PATH" => Some(script_path_string.clone()),
            _ => None,
        })
        .expect_err("mismatched script metadata should fail")
        .contains("expected final view")
    );

    std::fs::remove_file(&script_path).expect("semantic script file should be removed");

    assert!(
            super::run_gui_semantic_scenario_named("missing-scenario")
                .expect_err("unknown scenario should fail")
                .contains(
                    "Available: configuration-surface-flow, core-shell-smoke-flow, runtime-chat-flow, runtime-transport-churn-flow, drag-and-drop-ingest-flow, playlist-workflow-flow, persistence-reset-flow, detached-runtime-ownership-flow, live-python-peer-connect-flow, live-python-peer-controlled-room-flow"
                )
        );
}

#[test]
fn syncplay_gui_semantic_cli_wrapper_renders_lookup_output() {
    let output =
        super::semantic_smoke::run_syncplay_gui_semantic_cli_from_lookup(|name| match name {
            "SYNCPLAY_GUI_SEMANTIC_SCENARIO" => Some("configuration-surface-flow".to_owned()),
            "SYNCPLAY_GUI_SEMANTIC_OUTPUT" => Some("json".to_owned()),
            _ => None,
        })
        .expect("semantic cli wrapper should run")
        .expect("semantic cli wrapper should produce output");
    assert!(output.starts_with("{\"result\":\"ok\","));
    assert!(output.contains("\"view\":\"media-search\""));
}

#[test]
fn syncplay_gui_semantic_cli_wrapper_runs_explicit_args() {
    let output = super::semantic_smoke::run_syncplay_gui_semantic_cli_from_args([
        "--scenario",
        "runtime-chat-flow",
        "--format",
        "json",
    ])
    .expect("semantic cli args wrapper should run")
    .expect("semantic cli args wrapper should produce output");
    assert!(output.starts_with("{\"result\":\"ok\","));
    assert!(output.contains("\"scenario\":\"runtime-chat-flow\""));

    let listed = super::semantic_smoke::run_syncplay_gui_semantic_cli_from_args(["--list"])
        .expect("semantic cli list should run")
        .expect("semantic cli list should produce output");
    assert!(listed.contains("configuration-surface-flow"));
    assert!(listed.contains("core-shell-smoke-flow"));
    assert!(listed.contains("runtime-chat-flow"));
    assert!(listed.contains("runtime-transport-churn-flow"));
    assert!(listed.contains("detached-runtime-ownership-flow"));
    assert!(listed.contains("live-python-peer-connect-flow"));
    assert!(listed.contains("live-python-peer-controlled-room-flow"));

    let printed = super::semantic_smoke::run_syncplay_gui_semantic_cli_from_args([
        "--print-script",
        "configuration-surface-flow",
    ])
    .expect("semantic cli print-script should run")
    .expect("semantic cli print-script should produce output");
    assert!(printed.contains("setting\tpublic-server\tPrimary\tsyncplay.pl:8999"));
    assert!(printed.contains("activate\tmedia-search:command:search"));

    let mut append_script_path = std::env::temp_dir();
    let unique_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_nanos();
    append_script_path.push(format!(
        "syncplay-gui-semantic-append-{}-{unique_id}.txt",
        std::process::id()
    ));
    std::fs::write(
        &append_script_path,
        "\
# delta script\n\
enter-text\tconfig:Connection:Host\tfalse\toverride.example\n\
assert-value\tconfig:Connection:Host\toverride.example\n",
    )
    .expect("semantic append script file should be created");
    let append_script_path_string = append_script_path.to_string_lossy().into_owned();
    let appended = super::semantic_smoke::run_syncplay_gui_semantic_cli_from_args([
        "--scenario",
        "configuration-surface-flow",
        "--append-script",
        &append_script_path_string,
        "--format",
        "json",
    ])
    .expect("semantic cli append-script should run")
    .expect("semantic cli append-script should produce output");
    assert!(appended.starts_with("{\"result\":\"ok\","));
    assert!(appended.contains("\"scenario\":\"configuration-surface-flow\""));
    assert!(appended.contains("\"view\":\"media-search\""));
    std::fs::remove_file(&append_script_path)
        .expect("semantic append script file should be removed");

    let described = super::semantic_smoke::run_syncplay_gui_semantic_cli_from_args([
        "--describe-scenarios",
        "--format",
        "json",
    ])
    .expect("semantic cli describe-scenarios should run")
    .expect("semantic cli describe-scenarios should produce output");
    assert!(described.starts_with("{\"result\":\"ok\",\"scenarios\":["));
    assert!(described.contains("\"name\":\"configuration-surface-flow\""));
    assert!(described.contains("\"description\":\"Edits configuration fields, surfaces validation and command availability, saves, then exercises public-server and media-search pending flows.\""));
    assert!(described.contains("\"script\":\"# Configuration save and follow-on cross-surface workflow\\nsetting\\tpublic-server\\tPrimary\\tsyncplay.pl:8999"));
    assert!(described.contains("\"name\":\"core-shell-smoke-flow\""));
    assert!(described.contains("\"description\":\"Ports the non-transport Windows smoke path into a platform-neutral shell scenario.\""));
    assert!(described.contains("\"script\":\"# Core shell smoke flow ported from the legacy non-transport Windows smoke path\\nsetting\\tpublic-server\\tAlpha\\talpha.example:8999"));
    assert!(described.contains("\"name\":\"runtime-transport-churn-flow\""));
    assert!(described.contains("\"description\":\"Applies startup/post-chat/reconnect runtime snapshots, verifies chat round-trips and user churn/removals, and completes local chat sends.\""));
    assert!(described.contains("\"script\":\"# Runtime-backed transport churn/reconnect flow without platform UI dependencies\\nsetting\\tusername\\tsmoke-user"));
    assert!(described.contains("\"name\":\"live-python-peer-connect-flow\""));
    assert!(described.contains("\"description\":\"Connects the GUI runtime to a live legacy Syncplay server that already has a Python reference peer attached, switches the GUI between rooms and back, verifies shared-room projection plus bidirectional readiness, chat, and playlist propagation, then forces a transient peer disconnect/reconnect and re-validates post-reconnect chat.\""));
    assert!(described.contains("\"script\":\"# Live Python reference-peer connect, readiness, chat, playlist, and reconnect flow against the legacy Syncplay server\\n# Peer: interop-py-peer\\n# Executed by a code-driven semantic runner; append-script is not supported for this scenario.\\nsetting\\tusername\\tinterop-gui-user\\nsetting\\troom\\tinterop-room\\nsetting\\tshared-playlist-enabled\\ttrue"));
    assert!(described.contains("\"name\":\"live-python-peer-controlled-room-flow\""));
    assert!(described.contains("\"description\":\"Connects the GUI runtime to a live legacy Syncplay server in a controlled room, auto-authenticates the GUI as controller from the stored room password, and verifies controller-state projection plus controller-only playlist enablement against the Python reference peer.\""));
    assert!(described.contains("\"script\":\"# Live Python reference-peer controlled-room flow against the legacy Syncplay server\\n# Peer: interop-py-peer\\n# Executed by a code-driven semantic runner; append-script is not supported for this scenario.\\nsetting\\tusername\\tinterop-gui-user\\nsetting\\troom\\t+interop-room:447CE7E3548D:AB-123-456\\nsetting\\tshared-playlist-enabled\\ttrue"));
}

#[test]
fn syncplay_gui_semantic_report_wrapper_returns_structured_lookup_output() {
    let report =
        super::semantic_smoke::run_syncplay_gui_semantic_report_from_lookup(|name| match name {
            "SYNCPLAY_GUI_SEMANTIC_SCENARIO" => Some("configuration-surface-flow".to_owned()),
            _ => None,
        })
        .expect("semantic report wrapper should run")
        .expect("semantic report wrapper should return a report");
    assert_eq!(report.scenario, "configuration-surface-flow");
    assert_eq!(report.view, "media-search");
    assert_eq!(report.modal, "none");
    assert_eq!(report.pending, "none");
    assert!(report.widgets > 0);
}

#[test]
fn syncplay_gui_semantic_report_wrapper_runs_persistence_reset_flow() {
    let report = super::run_gui_semantic_scenario_named("persistence-reset-flow")
        .expect("persistence/reset semantic scenario should run");
    assert_eq!(report.scenario, "persistence-reset-flow");
    assert_eq!(report.view, "configuration");
    assert_eq!(report.modal, "none");
    assert_eq!(report.pending, "none");
    assert!(report.widgets > 0);
}

#[test]
fn syncplay_gui_semantic_report_wrapper_runs_inline_script() {
    let report = super::semantic_smoke::run_syncplay_gui_semantic_report(
        super::semantic_smoke::GuiSemanticScenarioSource::InlineScript(
            "\
meta\tname\tinline-check\n\
meta\texpect-view\tconfiguration\n\
assert-selected\tconfiguration-root\ttrue\n\
assert-pending\tnone\n"
                .to_owned(),
        ),
    )
    .expect("inline semantic script should run");
    assert_eq!(report.scenario, "inline-check");
    assert_eq!(report.view, "configuration");
    assert_eq!(report.modal, "none");
    assert_eq!(report.pending, "none");
    assert!(report.widgets > 0);
}

#[test]
fn gui_widget_egui_renderer_maps_text_and_checkbox_edits_to_actions() {
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    let configuration_tree = state.configuration_widget_tree();
    let host = configuration_tree.find("config:Connection:Host").unwrap();
    let autoplay = configuration_tree
        .find("config:Readiness:Autoplay")
        .unwrap();
    let trusted_domains = configuration_tree
        .find("config:Privacy:Trusted Domains")
        .unwrap();
    let unpause_action = configuration_tree
        .find("config:Readiness:Unpause Action")
        .unwrap();

    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_text_input_node(
            &state,
            host,
            "syncplay.example",
            true,
            false,
        ),
        Some(vec![GuiShellAction::EditConfigurationText {
            section: "Connection",
            label: "Host",
            value: "syncplay.example".to_owned(),
        }])
    );
    assert_eq!(
        GuiWidgetEguiRenderer::action_for_checkbox_node(&state, autoplay, true),
        Some(GuiShellAction::EditConfigurationBool {
            section: "Readiness",
            label: "Autoplay",
            value: true,
        })
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_text_input_node(
            &state,
            trusted_domains,
            "youtube.com; *.example.com/videos",
            true,
            false,
        ),
        Some(vec![GuiShellAction::EditConfigurationText {
            section: "Privacy",
            label: "Trusted Domains",
            value: "youtube.com; *.example.com/videos".to_owned(),
        }])
    );
    assert_eq!(
        GuiWidgetEguiRenderer::configuration_select_options_for_node(&state, unpause_action),
        Some(vec![
            "IfAlreadyReady".to_owned(),
            "IfOthersReady".to_owned(),
            "IfMinUsersReady".to_owned(),
            "Always".to_owned(),
        ])
    );

    let chat_state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    let chat_tree = chat_state.main_window_widget_tree();
    let chat_input = chat_tree.find("main-window:chat-input").unwrap();

    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_text_input_node(
            &chat_state,
            chat_input,
            "Hello world",
            true,
            true,
        ),
        Some(vec![
            GuiShellAction::ApplyGuiDraftRuntimeSnapshot(GuiDraftRuntimeSnapshot {
                outgoing_chat_message: Some("Hello world".to_owned()),
            }),
            GuiShellAction::BeginLocalChatSend("Hello world".to_owned()),
        ])
    );

    let room_state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        room: Some("Lounge".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let room_tree = room_state.main_window_widget_tree();
    let room_input = room_tree.find("main-window:room-input").unwrap();

    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_text_input_node(
            &room_state,
            room_input,
            "TeamRoom",
            true,
            true,
        ),
        Some(vec![
            GuiShellAction::EditConfigurationText {
                section: "Connection",
                label: "Room",
                value: "TeamRoom".to_owned(),
            },
            GuiShellAction::JoinMainWindowRoom("TeamRoom".to_owned()),
        ])
    );

    assert!(room_tree.find("main-window:user:new").is_none());

    let add_playlist_input = room_tree.find("main-window:playlist:new").unwrap();
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_text_input_node(
            &room_state,
            add_playlist_input,
            "Episode 1.mkv",
            true,
            true,
        ),
        Some(vec![
            GuiShellAction::UpdateNewPlaylistEntryDraft("Episode 1.mkv".to_owned()),
            GuiShellAction::CommitNewPlaylistEntry,
        ])
    );

    let mut user_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    assert!(user_state.apply(GuiShellAction::AddMainWindowUser("Bob".to_owned())));
    assert!(user_state.apply(GuiShellAction::SelectMainWindowUser(1)));
    assert!(user_state.apply(GuiShellAction::BeginEditSelectedMainWindowUser));
    let user_tree = user_state.main_window_widget_tree();
    assert!(user_tree.find("main-window:user-edit:username").is_none());

    let mut controlled_room_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            room: Some("Lounge".to_owned()),
            ..StoredClientSettingsMvp::default()
        });
    assert!(controlled_room_state.apply(GuiShellAction::BeginCreateControlledRoomEdit));
    let controlled_room_tree = controlled_room_state.main_window_widget_tree();
    let controlled_room_input = controlled_room_tree
        .find("main-window:controlled-room-create:room")
        .unwrap();
    let controlled_room_actions = GuiWidgetEguiRenderer::actions_for_text_input_node(
        &controlled_room_state,
        controlled_room_input,
        "Studio",
        true,
        true,
    )
    .expect("controlled-room input should map edits");
    assert_eq!(controlled_room_actions.len(), 3);
    assert_eq!(
        controlled_room_actions[0],
        GuiShellAction::UpdateCreateControlledRoomEdit("Studio".to_owned())
    );
    assert!(matches!(
        &controlled_room_actions[1],
        GuiShellAction::RequestControllerAuth { room, password }
            if room == "Studio"
                && password.len() == 10
                && password.chars().enumerate().all(|(index, c)| match index {
                    2 | 6 => c == '-',
                    0 | 1 => c.is_ascii_uppercase(),
                    _ => c.is_ascii_digit(),
                })
    ));
    assert_eq!(
        controlled_room_actions[2],
        GuiShellAction::CancelCreateControlledRoomEdit
    );

    let mut controller_auth_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            room: Some("+Lounge:ABCDEF123456".to_owned()),
            ..StoredClientSettingsMvp::default()
        });
    assert!(controller_auth_state.apply(GuiShellAction::BeginControllerAuthEdit));
    let controller_auth_tree = controller_auth_state.main_window_widget_tree();
    let controller_auth_input = controller_auth_tree
        .find("main-window:controller-auth:password")
        .unwrap();
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_text_input_node(
            &controller_auth_state,
            controller_auth_input,
            "ab-123-456",
            true,
            true,
        ),
        Some(vec![
            GuiShellAction::UpdateControllerAuthPasswordEdit("ab-123-456".to_owned()),
            GuiShellAction::RequestControllerAuth {
                room: "+Lounge:ABCDEF123456".to_owned(),
                password: "ab-123-456".to_owned(),
            },
            GuiShellAction::CancelControllerAuthEdit,
        ])
    );
}

#[test]
fn gui_preview_runtime_bridge_maps_pending_operations_to_preview_actions() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        public_servers: Some(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())]),
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut runtime = GuiPreviewRuntimeBridge;

    assert!(runtime.shows_manual_pending_controls());

    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    assert_eq!(
        runtime.actions_for_pending_completion(&state),
        vec![GuiShellAction::CompleteConfigurationSave(
            state.configuration.to_stored_settings(),
        )]
    );
    assert_eq!(
        runtime.actions_for_pending_cancel(&state),
        vec![GuiShellAction::CancelPendingOperation]
    );
    for action in runtime.actions_for_pending_cancel(&state) {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());

    state.main_window.playback.can_toggle_pause = true;
    assert!(state.apply(GuiShellAction::BeginPlaybackPauseToggle));
    assert_eq!(
        runtime.actions_for_pending_completion(&state),
        vec![GuiShellAction::CompletePlaybackPauseToggle]
    );
    for action in runtime.actions_for_pending_cancel(&state) {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());

    assert!(state.apply(GuiShellAction::BeginSelectedPublicServerConnect));
    assert_eq!(
        runtime.actions_for_pending_completion(&state),
        vec![GuiShellAction::CompleteSelectedPublicServerConnect]
    );
    for action in runtime.actions_for_pending_cancel(&state) {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());

    assert!(state.apply(GuiShellAction::BeginLocalChatSend("hello".to_owned())));
    assert_eq!(
        runtime.actions_for_pending_completion(&state),
        vec![GuiShellAction::CompleteLocalChatSend]
    );
    for action in runtime.actions_for_pending_completion(&state) {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("hello")
    );
    assert!(runtime.actions_for_pending_completion(&state).is_empty());
    assert!(runtime.actions_for_pending_cancel(&state).is_empty());
}

#[test]
fn gui_queued_runtime_bridge_and_preview_owner_cover_runtime_requests() {
    let (_host, host_handle) = GuiEframeNativeHost::with_queued_runtime();
    assert!(host_handle.drain_requests().is_empty());

    let (mut runtime, handle) = GuiQueuedRuntimeBridge::new();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(!runtime.shows_manual_pending_controls());
    assert!(runtime.drain_runtime_actions().is_empty());
    assert!(handle.drain_requests().is_empty());

    let (preview_runtime, preview_handle) =
        GuiQueuedRuntimeBridge::new_with_manual_pending_controls(true);
    assert!(preview_runtime.shows_manual_pending_controls());
    let mut preview_pump = super::GuiQueuedRuntimeOwnerPump::new(
        preview_handle.clone(),
        super::GuiPreviewRuntimeOwner,
    );
    preview_handle.push_request(GuiRuntimeRequest::SeekOffset(3.5));
    super::GuiNativeRuntimePump::pump(&mut preview_pump, &state);
    assert_eq!(
        preview_handle.drain_actions(),
        vec![
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "Seek requested: 3.5 seconds.".to_owned(),
            },
            GuiShellAction::AnnounceSystemChatEvent("Seek requested: 3.5 seconds.".to_owned(),),
        ]
    );
    assert!(
        runtime
            .actions_for_room_join(&state, "joined-room".to_owned())
            .is_empty()
    );
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::SetRoom("joined-room".to_owned())]
    );
    assert!(runtime.actions_for_room_leave(&state).is_empty());
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::ReturnToDefaultRoom]
    );

    handle.push_action(GuiShellAction::PushTransientNotification {
        level: GuiTransientNotificationLevel::Info,
        message: "Runtime callback queued.".to_owned(),
    });
    handle.push_action(GuiShellAction::AnnounceSystemChatEvent(
        "Runtime callback applied.".to_owned(),
    ));

    assert_eq!(
        runtime.drain_runtime_actions(),
        vec![
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "Runtime callback queued.".to_owned(),
            },
            GuiShellAction::AnnounceSystemChatEvent("Runtime callback applied.".to_owned(),),
        ]
    );
    assert!(runtime.drain_runtime_actions().is_empty());

    assert!(
        runtime
            .actions_for_selected_media_files(&state, Vec::new())
            .is_empty()
    );
    assert!(handle.drain_requests().is_empty());

    assert!(
        runtime
            .actions_for_selected_media_files(&state, vec!["C:/Media/movie.mkv".to_owned()])
            .is_empty()
    );
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::OpenMediaFiles {
            paths: vec!["C:/Media/movie.mkv".to_owned()],
            load_into_shared_playlist: false,
        }]
    );

    assert!(runtime.actions_for_seek_offset(12.5).is_empty());
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::SeekOffset(12.5)]
    );
    assert!(
        runtime
            .actions_for_playlist_entry_commit(&state, "Episode 1.mkv".to_owned(), true)
            .is_empty()
    );
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::QueuePlaylistEntry {
            entry: "Episode 1.mkv".to_owned(),
            select_after_queue: true,
        }]
    );
    assert!(
        runtime
            .actions_for_playlist_selection_change(&state, 1)
            .is_empty()
    );
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::SetPlaylistIndex(1)]
    );
    assert!(
        runtime
            .actions_for_playlist_entry_removal(&state, 0)
            .is_empty()
    );
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::DeletePlaylistIndex(0)]
    );
    assert!(
            runtime
                .actions_for_playlist_reorder(
                    &state,
                    vec!["One".to_owned(), "Two".to_owned()],
                    Some(1),
                )
                .is_empty()
        );
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::ReplacePlaylist {
            files: vec!["One".to_owned(), "Two".to_owned()],
            selected_index: Some(1),
        }]
    );
    assert!(runtime.actions_for_playlist_undo(&state).is_empty());
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::UndoPlaylistChange]
    );
    assert!(
        runtime
            .actions_for_playlist_shuffle_remaining(&state)
            .is_empty()
    );
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::ShuffleRemainingPlaylist]
    );
    assert!(
        runtime
            .actions_for_playlist_shuffle_entire(&state)
            .is_empty()
    );
    assert_eq!(
        handle.drain_requests(),
        vec![GuiRuntimeRequest::ShuffleEntirePlaylist]
    );
    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![
            "C:/Media/episode1.mkv".to_owned(),
            "C:/Media/episode2.mkv".to_owned(),
        ],
        load_into_shared_playlist: true,
    });
    assert_eq!(
        handle.drain_preview_response_actions(),
        vec![
            GuiShellAction::SwitchView(GuiShellView::MainWindow),
            GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
                "episode1.mkv".to_owned(),
                "episode2.mkv".to_owned(),
            ]),
        ]
    );

    assert!(state.apply(GuiShellAction::BeginLocalChatSend("hello".to_owned())));
    assert!(runtime.actions_for_pending_completion(&state).is_empty());
    assert!(runtime.actions_for_pending_cancel(&state).is_empty());
    assert_eq!(
        handle.drain_requests(),
        vec![
            GuiRuntimeRequest::CompletePendingOperation(
                GuiPendingCompletionRequest::SendChatMessage("hello".to_owned())
            ),
            GuiRuntimeRequest::CancelPendingOperation(GuiPendingOperationKind::SendChatMessage),
        ]
    );
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SendChatMessage("hello".to_owned()),
    ));
    handle.push_request(GuiRuntimeRequest::CancelPendingOperation(
        GuiPendingOperationKind::SendChatMessage,
    ));
    assert_eq!(
        handle.drain_preview_response_actions(),
        vec![
            GuiShellAction::CompleteLocalChatSend,
            GuiShellAction::CancelPendingOperation,
        ]
    );
}

#[test]
fn gui_persisted_config_runtime_owner_persists_save_and_reload_requests() {
    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "syncplay-gui-persisted-config-owner-{}-{unique_suffix}.ini",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);

    let mut owner = super::GuiPersistedConfigRuntimeOwner::with_config_path(Some(path.clone()));
    let handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    let saved_settings = StoredClientSettingsMvp {
        host: Some("persisted.example".to_owned()),
        room: Some("Cinema".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SaveConfiguration(saved_settings.clone()),
    ));
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert_eq!(
        handle.drain_actions(),
        vec![GuiShellAction::CompleteConfigurationSave(
            saved_settings.clone()
        )]
    );
    assert_eq!(
        super::load_syncplay_ini_stored_client_settings_mvp_from_path(&path)
            .expect("save should leave a readable config file"),
        Some(saved_settings.clone())
    );

    let reloaded_settings = StoredClientSettingsMvp {
        host: Some("reloaded.example".to_owned()),
        room: Some("Rewatch".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    super::upsert_syncplay_ini_stored_client_settings_mvp_at_path(&path, &reloaded_settings)
        .expect("updating the config file should succeed");
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::ReloadConfiguration(StoredClientSettingsMvp::default()),
    ));
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert_eq!(
        handle.drain_actions(),
        vec![GuiShellAction::CompleteConfigurationReload(
            reloaded_settings.clone()
        )]
    );

    std::fs::remove_file(&path).expect("temporary config file should be removable");
}

#[test]
fn gui_persisted_config_runtime_owner_clears_gui_data_files_and_returns_first_run_state() {
    let root = test_temp_root("clear-gui-data-owner");
    let path = root.join("syncplay.ini");
    let saved_settings = StoredClientSettingsMvp {
        host: Some("persisted.example".to_owned()),
        room: Some("Cinema".to_owned()),
        public_servers: Some(vec![("Saved".to_owned(), "saved.example:8999".to_owned())]),
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        ..StoredClientSettingsMvp::default()
    };
    super::upsert_syncplay_ini_stored_client_settings_mvp_at_path(&path, &saved_settings)
        .expect("saved configuration should be written");
    super::persist_gui_ui_state_at_root(
        &root,
        &super::GuiPersistedUiState {
            active_view: Some(GuiShellView::PublicServers),
            selected_public_server_address: Some("custom.example:9001".to_owned()),
            selected_media_search_directory: None,
            last_media_dialog_directory: Some("D:/Dialogs".to_owned()),
            last_checked_for_updates: None,
            hide_empty_rooms: false,
            public_servers: vec![("Custom".to_owned(), "custom.example:9001".to_owned())],
            ..Default::default()
        },
    )
    .expect("GUI state should be written");

    let mut owner = super::GuiPersistedConfigRuntimeOwner::with_config_path(Some(path.clone()));
    let handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&saved_settings);

    assert!(state.apply(GuiShellAction::BeginClearGuiData));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::ClearGuiData,
    ));
    let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(
        actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteClearGuiData)),
        "clear-GUI-data runtime completion should round-trip through the queued owner"
    );
    assert!(!path.exists(), "clear-GUI-data should remove syncplay.ini");
    for store_name in ["MainWindow", "Interface", "MediaBrowseDialog"] {
        assert!(
            !super::legacy_gui_qsettings_store_path(&root, store_name).exists(),
            "clear-GUI-data should remove legacy GUI state store {store_name}"
        );
    }
    assert_eq!(state.configuration.launch_mode, GuiLaunchMode::FirstRun);
    assert_eq!(state.active_view, GuiShellView::Configuration);
    assert_eq!(
        state.saved_configuration,
        StoredClientSettingsMvp::default()
    );
    assert_eq!(state.last_media_dialog_directory, None);
    assert!(state.public_servers.servers.is_empty());
    assert!(state.media_search.directories.is_empty());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_reports_runtime_gaps_explicitly() {
    let mut owner = super::GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec!["C:/Media/episode1.mkv".to_owned()],
        load_into_shared_playlist: true,
    });
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert_eq!(
            handle.drain_actions(),
            vec![
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Error,
                    message: "Opening media into the shared playlist requires a session or playback runtime connection; the selected file was not opened or queued."
                        .to_owned(),
                },
                GuiShellAction::AnnounceSystemChatEvent(
                    "Opening media into the shared playlist requires a session or playback runtime connection; the selected file was not opened or queued."
                        .to_owned(),
                ),
            ]
        );

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec!["C:/Media/movie.mkv".to_owned()],
        load_into_shared_playlist: false,
    });
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert_eq!(
            handle.drain_actions(),
            vec![
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Error,
                    message: "Opening media requires a playback runtime connection; the selected file was not opened."
                        .to_owned(),
                },
                GuiShellAction::AnnounceSystemChatEvent(
                    "Opening media requires a playback runtime connection; the selected file was not opened."
                        .to_owned(),
                ),
            ]
        );

    handle.push_request(GuiRuntimeRequest::SeekOffset(12.5));
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert_eq!(
            handle.drain_actions(),
            vec![
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Error,
                    message: "Playback seek requires a playback runtime connection; the 12.5 second request was not applied."
                        .to_owned(),
                },
                GuiShellAction::AnnounceSystemChatEvent(
                    "Playback seek requires a playback runtime connection; the 12.5 second request was not applied."
                        .to_owned(),
                ),
            ]
        );

    let mut cancel_chat_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            chat_input_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        });
    assert!(cancel_chat_state.apply(GuiShellAction::BeginLocalChatSend("cancel me".to_owned(),)));
    handle.push_request(GuiRuntimeRequest::CancelPendingOperation(
        GuiPendingOperationKind::SendChatMessage,
    ));
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &cancel_chat_state);
    let cancel_actions = handle.drain_actions();
    assert_eq!(cancel_actions, vec![GuiShellAction::CancelPendingOperation]);
    for action in cancel_actions {
        assert!(cancel_chat_state.apply(action));
    }
    assert!(cancel_chat_state.pending_operation.is_none());
    assert!(cancel_chat_state.outgoing_chat_message.is_none());

    let mut toggle_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    toggle_state.main_window.playback.can_toggle_pause = true;
    toggle_state.commands.can_toggle_pause = true;
    assert!(toggle_state.apply(GuiShellAction::BeginPlaybackPauseToggle));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::TogglePlaybackPause,
    ));
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &toggle_state);
    assert_eq!(
            handle.drain_actions(),
            vec![
                GuiShellAction::CancelPlaybackPauseToggle,
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Error,
                    message: "Playback toggle requires a playback runtime connection; the pause request was not applied."
                        .to_owned(),
                },
                GuiShellAction::ApplyMainWindowRuntimeSnapshot(MainWindowRuntimeSnapshot {
                    room_name: "(no room joined)".to_owned(),
                    shared_playlist_enabled: false,
                    controlled_room_active: false,
                    users: vec![browser_runtime_user(
                        "You",
                        "(no room joined)",
                        true,
                        false,
                        false,
                    )],
                    playlist: Vec::new(),
                    chat: Vec::new(),
                    can_toggle_pause: false,
                    can_seek: false,
                    can_set_ready: true,
                    can_manage_playlist: false,
                    playback_paused: false,
                    autoplay_active: false,
                    hide_empty_rooms: false,
                    rooms: browser_runtime_rooms("(no room joined)", false, true),
                    ..Default::default()
                }),
                GuiShellAction::ApplyGuiCommandRuntimeSnapshot(GuiCommandRuntimeSnapshot {
                    command_availability: GuiCommandAvailabilityState {
                        can_save_configuration: true,
                        can_reset_configuration: false,
                        can_reload_configuration: true,
                        can_connect_public_server: false,
                        can_connect_saved_server: false,
                        can_refresh_public_servers: true,
                        can_disconnect_session: false,
                        can_search_missing_media: false,
                        can_toggle_pause: false,
                        can_send_chat_message: false,
                    },
                    pending_operation: None,
                }),
            ]
        );

    let mut chat_state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    assert!(chat_state.apply(GuiShellAction::BeginLocalChatSend("hello".to_owned())));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SendChatMessage("hello".to_owned()),
    ));
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &chat_state);
    let chat_actions = handle.drain_actions();
    assert_eq!(
        chat_actions,
        vec![
            GuiShellAction::ApplyGuiCommandRuntimeSnapshot(GuiCommandRuntimeSnapshot {
                command_availability: GuiCommandAvailabilityState {
                    can_save_configuration: true,
                    can_reset_configuration: false,
                    can_reload_configuration: true,
                    can_connect_public_server: false,
                    can_connect_saved_server: false,
                    can_refresh_public_servers: true,
                    can_disconnect_session: false,
                    can_search_missing_media: false,
                    can_toggle_pause: false,
                    can_send_chat_message: true,
                },
                pending_operation: None,
            }),
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message:
                    "Chat sending requires a session runtime connection; the message was not sent."
                        .to_owned(),
            },
        ]
    );
    for action in chat_actions {
        assert!(chat_state.apply(action));
    }
    assert_eq!(chat_state.outgoing_chat_message.as_deref(), Some("hello"));
    assert!(chat_state.pending_operation.is_none());
}

#[test]
fn gui_persisted_config_runtime_owner_routes_shared_playlist_open_through_client_core_session_and_player()
 {
    let mut owner = super::GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(super::GuiOwnedPlayer::Test(
        super::GuiTestPlayerAdapter::default(),
    ));

    let handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![
            "C:/Media/episode1.mkv".to_owned(),
            "C:/Media/episode2.mkv".to_owned(),
        ],
        load_into_shared_playlist: true,
    });
    let actions = pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| state.main_window.playlist.len() == 2,
        "shared-playlist open through loopback session and player",
    );

    assert!(
            actions.iter().any(|action| matches!(
                action,
                GuiShellAction::PushTransientNotification { level, message }
                    if *level == GuiTransientNotificationLevel::Success
                        && message
                            == "Opened the first selected media file through the attached test player: C:/Media/episode1.mkv. Ignored 1 additional selections."
            )),
            "shared-playlist open should still drive the attached player"
        );
    assert_eq!(state.active_view, GuiShellView::MainWindow);
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.clone())
            .collect::<Vec<_>>(),
        vec!["episode1.mkv".to_owned(), "episode2.mkv".to_owned()]
    );
    assert_eq!(state.selection.selected_main_window_playlist, Some(0));
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .map(|file| file.name.as_str()),
        Some("episode1.mkv")
    );
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some("C:/Media/episode1.mkv")
    );
}

#[test]
fn gui_persisted_config_runtime_owner_imports_playlist_files_through_client_core_session() {
    let root = test_temp_root("shared-playlist-import");
    let playlist_path = root.join("room-playlist.txt");
    std::fs::write(&playlist_path, "episode1.mkv\nhttps://example.com/live\n")
        .expect("shared playlist import fixture should be written");

    let mut owner = super::GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    let handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![playlist_path.to_string_lossy().into_owned()],
        load_into_shared_playlist: true,
    });
    let actions = pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| state.main_window.playlist.len() == 2,
        "shared-playlist import through loopback session",
    );

    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Success
                    && message == "Imported 2 entries into the shared playlist."
        )),
        "shared-playlist imports should report a runtime-backed success"
    );
    assert_eq!(state.active_view, GuiShellView::MainWindow);
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.clone())
            .collect::<Vec<_>>(),
        vec![
            "episode1.mkv".to_owned(),
            "https://example.com/live".to_owned(),
        ]
    );
    assert_eq!(state.selection.selected_main_window_playlist, Some(0));
    assert!(owner.player_local_file.is_none());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_imports_playlist_files_queued_before_startup_pump() {
    let root = test_temp_root("shared-playlist-import-startup-queue");
    let playlist_path = root.join("startup-room-playlist.txt");
    std::fs::write(&playlist_path, "episode1.mkv\nhttps://example.com/live\n")
        .expect("startup shared playlist import fixture should be written");

    let mut owner = super::GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    let handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![playlist_path.to_string_lossy().into_owned()],
        load_into_shared_playlist: true,
    });
    let actions = pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| state.main_window.playlist.len() == 2,
        "shared-playlist import queued before startup runtime pump",
    );

    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Success
                    && message == "Imported 2 entries into the shared playlist."
        )),
        "startup-queued shared-playlist imports should still report a runtime-backed success"
    );
    assert_eq!(state.active_view, GuiShellView::MainWindow);
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.clone())
            .collect::<Vec<_>>(),
        vec![
            "episode1.mkv".to_owned(),
            "https://example.com/live".to_owned(),
        ]
    );
    assert_eq!(state.selection.selected_main_window_playlist, Some(0));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_bridges_chat_protocol_and_notifications() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        chat_output_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    let mut adapter = super::GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);
    assert!(startup_lines[0].contains("\"Hello\""));
    assert!(startup_lines[0].contains("\"alice\""));
    assert!(startup_lines[0].contains("\"room1\""));
    assert!(startup_lines[0].contains("\"chat\":true"));
    assert!(
        super::GuiSessionRuntimeAdapter::send_chat_message(&mut adapter, "hello room".to_owned(),)
            .is_err(),
        "chat should stay blocked until the adapter receives a server Hello"
    );

    adapter
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("inbound server hello should apply");
    assert!(
        super::GuiSessionRuntimeAdapter::send_chat_message(&mut adapter, "hello room".to_owned(),)
            .is_ok(),
        "chat-capable client-core adapter should queue outbound chat"
    );
    let outbound_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("queued outbound protocol lines should encode");
    assert_eq!(outbound_lines.len(), 1);
    assert!(outbound_lines[0].contains("\"Chat\""));
    assert!(outbound_lines[0].contains("hello room"));
    for action in super::GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state) {
        assert!(state.apply(action));
    }

    adapter
        .apply_message_json(r#"{"Chat":{"username":"alice","message":"hello room"}}"#)
        .expect("inbound server echo should apply");
    assert_eq!(
        super::GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state),
        vec![GuiShellAction::PushChatMessage {
            sender: "alice".to_owned(),
            message: "hello room".to_owned(),
        }]
    );
    assert!(super::GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state).is_empty());
}

#[test]
fn gui_persisted_config_runtime_owner_bootstraps_detached_public_server_connect() {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
        time::Duration,
    };

    let listener = TcpListener::bind("127.0.0.1:0")
        .expect("detached public-server connect test should bind a TCP listener");
    let address = listener
        .local_addr()
        .expect("detached public-server connect test listener should expose an address");
    let (hello_tx, hello_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server_thread = thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("detached public-server connect test should accept a GUI connection");
        let mut reader = BufReader::new(
            stream
                .try_clone()
                .expect("detached public-server connect test stream should clone"),
        );
        let mut hello_line = String::new();
        reader
            .read_line(&mut hello_line)
            .expect("detached public-server connect test should read the GUI hello");
        hello_tx
            .send(hello_line)
            .expect("detached public-server connect test should report the hello");
        stream
                .write_all(
                    br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
                )
                .expect("detached public-server connect test should write the server hello");
        stream
            .write_all(b"\r\n")
            .expect("detached public-server connect test should terminate the server hello");
        stream
            .flush()
            .expect("detached public-server connect test should flush the server hello");
        release_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("detached public-server connect test should release the server");
    });

    let mut owner = super::GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        public_servers: Some(vec![("Primary".to_owned(), address.to_string())]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::SelectPublicServer(0)));
    assert!(state.apply(GuiShellAction::BeginSelectedPublicServerConnect));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::ConnectPublicServer,
    ));
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let connect_actions = handle.drain_actions();
    let projected_hello_in_connect_actions = connect_actions.iter().any(|action| {
        matches!(
            action,
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot)
                if snapshot.room_name == "room1"
                    && snapshot
                        .users
                        .iter()
                        .any(|user| user.username == "alice" && user.is_self)
        )
    });
    assert!(
        connect_actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteSelectedPublicServerConnect)),
        "detached public-server connect should complete through a bootstrapped client-core session runtime"
    );
    for action in connect_actions {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());
    assert!(
        state
            .notifications
            .iter()
            .any(|notification| { notification.message == "Connected to public server: Primary." }),
        "detached public-server connect should report the selected server connection"
    );

    let hello_line = hello_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("detached public-server connect should emit a GUI hello");
    assert!(hello_line.contains("\"Hello\""));
    assert!(hello_line.contains("\"alice\""));
    assert!(hello_line.contains("\"room1\""));

    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let hello_actions = handle.drain_actions();
    let projected_hello_in_followup_actions = hello_actions.iter().any(|action| {
        matches!(
            action,
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot)
                if snapshot.room_name == "room1"
                    && snapshot
                        .users
                        .iter()
                        .any(|user| user.username == "alice" && user.is_self)
        )
    });
    assert!(
        projected_hello_in_connect_actions || projected_hello_in_followup_actions,
        "detached public-server connect should leave an attached session runtime that projects server hello state"
    );
    for action in hello_actions {
        assert!(state.apply(action));
    }

    release_tx
        .send(())
        .expect("detached public-server connect test should release the server");
    server_thread
        .join()
        .expect("detached public-server connect server thread should complete");
}

#[test]
fn gui_persisted_config_runtime_owner_refreshes_public_servers_without_session() {
    let mut owner = super::GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![
            (" Primary ".to_owned(), " syncplay.pl:8999 ".to_owned()),
            ("Duplicate".to_owned(), "SYNCPLAY.PL:8999".to_owned()),
            ("Invalid".to_owned(), " :9000 ".to_owned()),
            ("Backup".to_owned(), "backup.example:9000".to_owned()),
        ]),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::BeginPublicServerRefresh));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::RefreshPublicServers(vec![]),
    ));
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let actions = handle.drain_actions();
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::CompletePublicServerRefresh(servers)
                if servers
                    == &vec![
                        ("Primary".to_owned(), "syncplay.pl:8999".to_owned()),
                        ("Backup".to_owned(), "backup.example:9000".to_owned()),
                    ]
        )),
        "detached public-server refresh should normalize and complete without a preexisting session runtime"
    );
    for action in actions {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());
    assert_eq!(
        state
            .notifications
            .last()
            .map(|notification| notification.message.as_str()),
        Some("Public servers refreshed: 2 entries.")
    );
}

#[test]
fn gui_persisted_config_runtime_owner_searches_missing_media_without_session() {
    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "syncplay-gui-detached-missing-media-search-{}-{unique_suffix}",
        std::process::id()
    ));
    let nested = root.join("nested");
    let found_path = nested.join("missing-target.mkv");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&nested)
        .expect("detached missing-media search test should create a directory tree");
    std::fs::write(&found_path, b"detached-missing-media-target")
        .expect("detached missing-media search test should create the target file");

    let mut owner = super::GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "missing-target.mkv".to_owned(),
        ]))
    );
    assert!(state.apply(GuiShellAction::BeginMissingMediaSearch));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SearchMissingMedia,
    ));
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let actions = handle.drain_actions();
    let found_path_text = found_path.to_string_lossy().into_owned();
    let expected_message = format!("Missing media found: {found_path_text}.");
    assert_eq!(
        actions,
        vec![GuiShellAction::CompleteMissingMediaSearch(Some(
            found_path_text.clone(),
        ))]
    );
    for action in actions {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());
    assert_eq!(
        state
            .notifications
            .last()
            .map(|notification| notification.message.as_str()),
        Some(expected_message.as_str())
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_projects_session_state_into_main_window_snapshot() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut playback_ready_snapshot =
        MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
    playback_ready_snapshot.can_toggle_pause = true;
    playback_ready_snapshot.can_seek = true;
    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        playback_ready_snapshot
    )));
    let mut adapter = super::GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    adapter
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("inbound server hello should apply");
    for action in super::GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state) {
        assert!(state.apply(action));
    }

    adapter
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist-change set message should apply");
    adapter
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
        .expect("playlist-index set message should apply");
    adapter
            .apply_message_json(
                r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"alice"}}}"#,
            )
            .expect("playstate message should apply");
    let actions = super::GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    assert_eq!(actions.len(), 3);
    let GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot) = &actions[0] else {
        panic!("session state changes should become a main-window runtime snapshot");
    };
    assert_eq!(snapshot.room_name, "room1");
    assert!(snapshot.shared_playlist_enabled);
    assert_eq!(
        snapshot.users,
        vec![browser_runtime_user("alice", "room1", true, false, false)]
    );
    assert_eq!(
        snapshot.playlist,
        vec!["episode1.mkv".to_owned(), "episode2.mkv".to_owned()]
    );
    assert!(snapshot.playback_paused);
    let GuiShellAction::ApplyGuiInteractionRuntimeSnapshot(interaction) = &actions[1] else {
        panic!("session playlist index should become a GUI interaction runtime snapshot");
    };
    assert_eq!(interaction.selection.selected_main_window_playlist, Some(1));
    let GuiShellAction::ApplyMenuDialogRuntimeSnapshot(menu_snapshot) = &actions[2] else {
        panic!("session playlist availability should become a menu runtime snapshot");
    };
    assert_eq!(
        menu_snapshot.action_overrides,
        vec![
            MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Playlist",
                enabled: true,
            },
            MenuActionRuntimeOverride {
                section_title: "Playback",
                action_label: "Playlist Actions",
                enabled: true,
            },
        ]
    );
    for action in actions {
        assert!(state.apply(action));
    }
    assert!(state.main_window.shared_playlist_enabled);
    assert_eq!(state.selection.selected_main_window_playlist, Some(1));
    assert!(
        state
            .menus
            .sections
            .iter()
            .find(|section| section.title == "Window")
            .and_then(|section| {
                section
                    .actions
                    .iter()
                    .find(|action| action.label == "Show Playlist")
            })
            .is_some_and(|action| action.enabled)
    );
    assert!(
        state
            .menus
            .sections
            .iter()
            .find(|section| section.title == "Playback")
            .and_then(|section| {
                section
                    .actions
                    .iter()
                    .find(|action| action.label == "Playlist Actions")
            })
            .is_some_and(|action| action.enabled)
    );
    assert!(super::GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state).is_empty());
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_surfaces_user_changes_as_system_chat_events() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut adapter = super::GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    adapter
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("inbound server hello should apply");
    for action in super::GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state) {
        assert!(state.apply(action));
    }

    adapter
        .apply_message_json(
            r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"controller":true}}}}"#,
        )
        .expect("user join message should apply");
    let actions = super::GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    assert_eq!(actions.len(), 2);
    let GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot) = &actions[0] else {
        panic!("user changes should still refresh the main-window runtime snapshot");
    };
    assert_eq!(
        snapshot.users,
        vec![
            browser_runtime_user("alice", "room1", true, false, false),
            browser_runtime_user("bob", "room1", false, false, true),
        ]
    );
    assert_eq!(
        actions[1],
        GuiShellAction::AnnounceSystemChatEvent("bob joined room1.".to_owned())
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_searches_missing_media_from_session_playlist() {
    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "syncplay-gui-missing-media-search-{}-{unique_suffix}",
        std::process::id()
    ));
    let nested = root.join("nested");
    let found_path = nested.join("episode2.mkv");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&nested)
        .expect("test missing-media search directory tree should be created");
    std::fs::write(&found_path, b"test").expect("test missing-media search file should be written");

    let mut adapter = super::GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");
    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    adapter
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("inbound server hello should apply");
    adapter
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist-change set message should apply");
    adapter
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
        .expect("playlist-index set message should apply");

    let search_result = super::GuiSessionRuntimeAdapter::search_missing_media(
        &mut adapter,
        vec![root.to_string_lossy().into_owned()],
    )
    .expect("missing-media search should succeed");
    assert_eq!(
        search_result,
        Some(found_path.to_string_lossy().into_owned())
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_normalizes_public_server_refresh_rows() {
    let mut adapter = super::GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let refreshed = super::GuiSessionRuntimeAdapter::refresh_public_servers(
        &mut adapter,
        vec![
            (" Primary ".to_owned(), " syncplay.pl:8999 ".to_owned()),
            ("Duplicate".to_owned(), "SYNCPLAY.PL:8999".to_owned()),
            (" ".to_owned(), "backup.example:9000".to_owned()),
            ("Invalid".to_owned(), " :9000 ".to_owned()),
            ("IPv6".to_owned(), "[::1]:8999".to_owned()),
        ],
    )
    .expect("public-server refresh should normalize rows");

    assert_eq!(
        refreshed,
        vec![
            ("Primary".to_owned(), "syncplay.pl:8999".to_owned()),
            ("IPv6".to_owned(), "[::1]:8999".to_owned()),
        ]
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_uses_lookup_public_server_refresh_source() {
    let refreshed =
            super::GuiClientCoreChatSessionRuntimeAdapter::refreshed_public_server_rows_from_lookup(
                &|name| match name {
                    "SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS" => Some(
                        r#"[[" Gui Primary ", " syncplay.pl:8999 "], ["Duplicate", "SYNCPLAY.PL:8999"]]"#
                            .to_owned(),
                    ),
                    _ => None,
                },
            )
            .expect("lookup-backed public-server refresh should parse")
            .expect("lookup-backed public-server refresh should produce rows");

    assert_eq!(
        refreshed,
        vec![("Gui Primary".to_owned(), "syncplay.pl:8999".to_owned())]
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_uses_file_lookup_public_server_refresh_source() {
    let refreshed = super::GuiClientCoreChatSessionRuntimeAdapter::refreshed_public_server_rows_from_sources(
            &|name| match name {
                "SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS_PATH" => Some("public-servers.txt".to_owned()),
                "SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS" => Some(
                    r#"[["Inline", "inline.example:9000"]]"#.to_owned(),
                ),
                _ => None,
            },
            &|path| {
                if path == "public-servers.txt" {
                    Ok(
                        r#"[[" File Primary ", " file.example:8999 "], ["Duplicate", "FILE.EXAMPLE:8999"]]"#
                            .to_owned(),
                    )
                } else {
                    Err("unexpected path".to_owned())
                }
            },
        )
        .expect("file-backed public-server refresh should parse")
        .expect("file-backed public-server refresh should produce rows");

    assert_eq!(
        refreshed,
        vec![("File Primary".to_owned(), "file.example:8999".to_owned())]
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_rejects_invalid_lookup_public_server_refresh_source()
 {
    let error =
        super::GuiClientCoreChatSessionRuntimeAdapter::refreshed_public_server_rows_from_lookup(
            &|name| {
                (name == "SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS")
                    .then_some("not-a-serialized-public-server-list".to_owned())
            },
        )
        .expect_err("invalid lookup-backed public-server refresh should fail");

    assert!(
        error.contains("SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS"),
        "error should identify the invalid lookup source"
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_clears_stale_session_state_before_server_hello() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    let mut stale_main_window = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
    stale_main_window.room_name = "live-room".to_owned();
    stale_main_window.shared_playlist_enabled = true;
    stale_main_window.controlled_room_active = true;
    stale_main_window.users = vec![
        MainWindowRuntimeUserSnapshot {
            username: "alice".to_owned(),
            is_self: true,
            is_ready: true,
            is_controller: true,
            ..Default::default()
        },
        MainWindowRuntimeUserSnapshot {
            username: "bob".to_owned(),
            is_self: false,
            is_ready: false,
            is_controller: false,
            ..Default::default()
        },
    ];
    stale_main_window.playlist = vec!["episode2.mkv".to_owned()];
    stale_main_window.can_set_ready = false;
    stale_main_window.can_manage_playlist = true;
    stale_main_window.playback_paused = true;
    stale_main_window.autoplay_active = true;
    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        stale_main_window
    )));
    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Playlist",
                enabled: true,
            }],
            tls_prompt_expected: state.menus.tls_prompt_expected,
            update_notice_expected: state.menus.update_notice_expected,
            about_dialog_available: state.menus.about_dialog_available,
        }
    )));

    let mut adapter = super::GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let actions = super::GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    let snapshot = actions
        .iter()
        .find_map(|action| match action {
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot) => Some(snapshot),
            _ => None,
        })
        .expect("pre-Hello session state should clear stale main-window runtime state");
    assert_eq!(snapshot.room_name, "room1");
    assert!(!snapshot.shared_playlist_enabled);
    assert!(!snapshot.controlled_room_active);
    assert_eq!(
        snapshot.users,
        vec![browser_runtime_user("alice", "room1", true, false, false)]
    );
    assert!(snapshot.playlist.is_empty());
    assert!(!snapshot.can_set_ready);
    assert!(!snapshot.can_manage_playlist);
    assert!(!snapshot.playback_paused);
    assert!(!snapshot.autoplay_active);

    let snapshot = actions
        .iter()
        .find_map(|action| match action {
            GuiShellAction::ApplyMenuDialogRuntimeSnapshot(snapshot) => Some(snapshot),
            _ => None,
        })
        .expect("pre-Hello session state should clear stale menu runtime state");
    assert!(
        snapshot
            .action_overrides
            .contains(&MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Chat",
                enabled: false,
            })
    );
    assert!(
        snapshot
            .action_overrides
            .contains(&MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Playlist",
                enabled: false,
            })
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_persists_reconnect_transitions_to_system_chat() {
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut adapter = super::GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    adapter
        .runtime
        .run_reconnect_retry(0)
        .expect("reconnect retry should queue notifications");
    adapter
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("inbound server hello should apply");
    let actions = super::GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Warning
                    && message == "Reconnect attempt 1 in 0.1 seconds."
        )),
        "reconnect retry should surface a warning notification"
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::AnnounceSystemChatEvent(message)
                if message == "Reconnect attempt 1 in 0.1 seconds."
        )),
        "reconnect retry should persist a system chat entry"
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Success
                    && message == "Session reconnected."
        )),
        "reconnect success should surface a success notification"
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::AnnounceSystemChatEvent(message)
                if message == "Session reconnected."
        )),
        "reconnect success should persist a system chat entry"
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_persists_reconnect_state_restore_details_to_system_chat()
 {
    assert_eq!(
        super::GuiClientCoreChatSessionRuntimeAdapter::reconnect_transition_actions(
            super::ReconnectTransitionNotification::StateRestoreValidationMismatch {
                local_paused: false,
                room_paused: true,
                local_position: 5.0,
                room_position: 7.5,
                position_diff_seconds: 2.5,
            },
        ),
        vec![
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Warning,
                message: "Session state restore mismatch detected (2.500 seconds).".to_owned(),
            },
            GuiShellAction::AnnounceSystemChatEvent(
                "Session state restore mismatch detected (2.500 seconds).".to_owned(),
            ),
        ]
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_dispatches_remote_ready_changes_when_supported() {
    let mut adapter = super::GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    adapter
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#,
            )
            .expect("inbound server hello should apply");

    assert!(
        super::GuiSessionRuntimeAdapter::set_user_ready(&mut adapter, "bob".to_owned(), true)
            .is_ok(),
        "newer readiness-capable servers should allow remote readiness changes"
    );

    let outbound_protocol_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("remote readiness lines should encode");
    assert_eq!(outbound_protocol_lines.len(), 1);
    assert!(outbound_protocol_lines[0].contains("\"ready\""));
    assert!(outbound_protocol_lines[0].contains("\"username\":\"bob\""));
    assert!(outbound_protocol_lines[0].contains("\"isReady\":true"));
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_rejects_remote_ready_changes_when_unsupported() {
    let mut adapter = super::GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    adapter
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.1","features":{"chat":true,"readiness":true}}}"#,
            )
            .expect("inbound server hello should apply");

    let error =
        super::GuiSessionRuntimeAdapter::set_user_ready(&mut adapter, "bob".to_owned(), true)
            .expect_err("older readiness-capable servers should reject remote readiness changes");
    assert!(
        error.contains("remote readiness changes"),
        "error should identify the missing remote readiness capability"
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_surfaces_controller_auth_transitions_as_notifications()
 {
    let room = "+room:ABCDEF123456";
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some(room.to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut adapter = super::GuiClientCoreChatSessionRuntimeAdapter::new("alice", room)
        .expect("client-core chat adapter should bootstrap");

    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    adapter
        .runtime
        .session_mut()
        .remember_control_password_for_room(room, "ab-123-456");
    adapter
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("inbound server hello should apply");
    let hello_actions = super::GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    assert!(
        hello_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Info
                    && message == "Requesting controller access for +room:ABCDEF123456."
        )),
        "controller reidentify should surface an attempt notification"
    );
    assert!(
        hello_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::AnnounceSystemChatEvent(message)
                if message == "Requesting controller access for +room:ABCDEF123456."
        )),
        "controller reidentify should persist the attempt message in system chat"
    );
    for action in hello_actions {
        assert!(state.apply(action));
    }

    adapter
            .apply_message_json(
                r#"{"Set":{"controllerAuth":{"user":"alice","room":"+room:ABCDEF123456","success":true}}}"#,
            )
            .expect("controller auth success should apply");
    let actions = super::GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Success
                    && message == "alice received controller access for +room:ABCDEF123456."
        )),
        "controller auth success should surface a success notification"
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::AnnounceSystemChatEvent(message)
                if message == "alice received controller access for +room:ABCDEF123456."
        )),
        "controller auth success should persist the success message in system chat"
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot)
                if snapshot.controlled_room_active
                    && snapshot.users.iter().any(|user| {
                        user.username == "alice" && user.is_self && user.is_controller
                    })
        )),
        "controller auth success should refresh the main-window runtime snapshot"
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_surfaces_controlled_room_creation_before_reidentify()
 {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut adapter = super::GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    adapter
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("inbound server hello should apply");
    for action in super::GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state) {
        assert!(state.apply(action));
    }

    adapter
            .apply_message_json(
                r#"{"Set":{"newControlledRoom":{"roomName":"+room:ABCDEF123456","password":"ab 123 456"}}}"#,
            )
            .expect("new controlled room message should apply");
    let actions = super::GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    let created_notice_index = actions
        .iter()
        .position(|action| {
            matches!(
                action,
                GuiShellAction::PushTransientNotification { level, message }
                    if *level == GuiTransientNotificationLevel::Success
                        && message == "Controlled room created: +room:ABCDEF123456."
            )
        })
        .expect("new controlled room should surface a success notification");
    let created_chat_index = actions
            .iter()
            .position(|action| {
                matches!(
                    action,
                    GuiShellAction::AnnounceSystemChatEvent(message)
                        if message == "Created controlled room +room:ABCDEF123456 with password AB123456 (+room:ABCDEF123456:AB123456)."
                )
            })
            .expect("new controlled room should surface a system chat entry");
    let reidentify_notice_index = actions
        .iter()
        .position(|action| {
            matches!(
                action,
                GuiShellAction::PushTransientNotification { level, message }
                    if *level == GuiTransientNotificationLevel::Info
                        && message == "Requesting controller access for +room:ABCDEF123456."
            )
        })
        .expect("new controlled room should trigger controller reidentify");
    let reidentify_chat_index = actions
        .iter()
        .position(|action| {
            matches!(
                action,
                GuiShellAction::AnnounceSystemChatEvent(message)
                    if message == "Requesting controller access for +room:ABCDEF123456."
            )
        })
        .expect("controller reidentify should be persisted in system chat");
    assert!(
        created_notice_index < reidentify_notice_index,
        "created-room notification should appear before the controller reidentify attempt"
    );
    assert!(
        created_chat_index < reidentify_chat_index,
        "created-room system chat should appear before the controller reidentify entry"
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot)
                if snapshot.room_name == "+room:ABCDEF123456"
                    && snapshot.controlled_room_active
        )),
        "new controlled room should still refresh the main-window snapshot"
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_auto_reidentifies_controlled_room_when_password_is_stored()
 {
    let room = "+room:ABCDEF123456";
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some(room.to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut adapter = super::GuiClientCoreChatSessionRuntimeAdapter::new_with_control_password(
        "alice",
        room,
        Some("ab-123-456".to_owned()),
    )
    .expect("client-core chat adapter should bootstrap");

    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    adapter
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("inbound server hello should apply");
    let hello_actions = super::GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    assert!(
        hello_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Info
                    && message == "Requesting controller access for +room:ABCDEF123456."
        )),
        "draining GUI actions should auto-dispatch the controller reidentify attempt"
    );

    let outbound_protocol_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("controller auth lines should encode");
    assert_eq!(outbound_protocol_lines.len(), 1);
    assert!(outbound_protocol_lines[0].contains("\"controllerAuth\""));
    assert!(outbound_protocol_lines[0].contains("\"+room:ABCDEF123456\""));
    assert!(outbound_protocol_lines[0].contains("\"AB-123-456\""));
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_surfaces_autoplay_countdown_notifications() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut adapter = super::GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    adapter
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("inbound server hello should apply");
    let hello_actions = super::GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    for action in hello_actions {
        assert!(state.apply(action));
    }

    adapter
        .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
        .expect("local ready should apply");
    adapter
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"bob.mp4"},"isReady":true,"controller":true}}}}"#,
            )
            .expect("remote ready user should apply");
    adapter.runtime.session_mut().set_autoplay_enabled(true);
    adapter
        .runtime
        .session_mut()
        .readiness_autoplay_config_mut()
        .auto_play_threshold = Some(2);
    adapter
        .runtime
        .session_mut()
        .apply_player_playback_telemetry_update(
            &syncplay_player_api::PlayerPlaybackTelemetryUpdate::default().with_paused(true),
        );
    adapter
        .runtime
        .update_autoplay_check(true, true, false, false);
    adapter
        .runtime
        .tick_autoplay(true, true, false, false)
        .expect("first autoplay tick should emit notification");
    adapter
        .runtime
        .tick_autoplay(true, true, false, false)
        .expect("second autoplay tick should emit notification");

    let actions = super::GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Info
                    && message == "Autoplay in 3 seconds with 2 ready users."
        )),
        "first autoplay tick should surface a countdown notification"
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::AnnounceSystemChatEvent(message)
                if message == "Autoplay in 3 seconds with 2 ready users."
        )),
        "first autoplay tick should persist a countdown entry in system chat"
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Info
                    && message == "Autoplay in 2 seconds with 2 ready users."
        )),
        "second autoplay tick should surface a countdown notification"
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::AnnounceSystemChatEvent(message)
                if message == "Autoplay in 2 seconds with 2 ready users."
        )),
        "second autoplay tick should persist a countdown entry in system chat"
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_dispatches_shared_playlist_operations() {
    let mut adapter = super::GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);
    assert!(
        !super::GuiSessionRuntimeAdapter::playlist_control_available(&adapter),
        "playlist controls should remain unavailable before server hello"
    );

    adapter
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#,
            )
            .expect("inbound server hello should apply");
    assert!(
        super::GuiSessionRuntimeAdapter::playlist_control_available(&adapter),
        "playlist controls should become available after a successful room hello"
    );

    super::GuiSessionRuntimeAdapter::queue_playlist_entry(
        &mut adapter,
        "episode1.mkv".to_owned(),
        true,
    )
    .expect("queueing the first playlist entry should dispatch");
    let first_queue_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("first queue lines should encode");
    assert_eq!(first_queue_lines.len(), 2);
    assert!(first_queue_lines[0].contains("\"playlistChange\""));
    assert!(first_queue_lines[0].contains("episode1.mkv"));
    assert!(first_queue_lines[1].contains("\"playlistIndex\""));
    assert!(first_queue_lines[1].contains("\"index\":0"));
    for line in &first_queue_lines {
        adapter
            .apply_message_json(line)
            .expect("first queue echo should apply");
    }

    super::GuiSessionRuntimeAdapter::queue_playlist_entry(
        &mut adapter,
        "episode2.mkv".to_owned(),
        true,
    )
    .expect("queueing the second playlist entry should dispatch");
    let second_queue_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("second queue lines should encode");
    assert_eq!(second_queue_lines.len(), 2);
    assert!(second_queue_lines[0].contains("episode1.mkv"));
    assert!(second_queue_lines[0].contains("episode2.mkv"));
    assert!(second_queue_lines[1].contains("\"index\":1"));
    for line in &second_queue_lines {
        adapter
            .apply_message_json(line)
            .expect("second queue echo should apply");
    }

    super::GuiSessionRuntimeAdapter::set_playlist_index(&mut adapter, 0)
        .expect("playlist selection should dispatch");
    let selection_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("selection lines should encode");
    assert_eq!(selection_lines.len(), 1);
    assert!(selection_lines[0].contains("\"playlistIndex\""));
    assert!(selection_lines[0].contains("\"index\":0"));
    adapter
        .apply_message_json(&selection_lines[0])
        .expect("selection echo should apply");

    super::GuiSessionRuntimeAdapter::delete_playlist_index(&mut adapter, 0)
        .expect("playlist removal should dispatch");
    let delete_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("delete lines should encode");
    assert_eq!(delete_lines.len(), 2);
    assert!(delete_lines[0].contains("\"playlistChange\""));
    assert!(delete_lines[0].contains("episode2.mkv"));
    assert!(delete_lines[1].contains("\"playlistIndex\""));
    assert!(delete_lines[1].contains("\"index\":0"));
    for line in &delete_lines {
        adapter
            .apply_message_json(line)
            .expect("delete echo should apply");
    }

    super::GuiSessionRuntimeAdapter::replace_playlist(
        &mut adapter,
        vec!["episode3.mkv".to_owned(), "episode2.mkv".to_owned()],
        Some(1),
    )
    .expect("playlist reorder should dispatch");
    let replace_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("replace lines should encode");
    assert_eq!(replace_lines.len(), 2);
    assert!(replace_lines[0].contains("\"playlistChange\""));
    assert!(replace_lines[0].contains("episode3.mkv"));
    assert!(replace_lines[0].contains("episode2.mkv"));
    assert!(replace_lines[1].contains("\"playlistIndex\""));
    assert!(replace_lines[1].contains("\"index\":1"));
    for line in &replace_lines {
        adapter
            .apply_message_json(line)
            .expect("replace echo should apply");
    }

    let playlist = adapter
        .runtime
        .session()
        .current_room_playlist()
        .expect("playlist should exist after the echoed operations");
    assert_eq!(playlist.files, vec!["episode3.mkv", "episode2.mkv"]);
    assert_eq!(playlist.index, Some(1));
}

#[test]
fn gui_persisted_config_runtime_owner_normalizes_controlled_room_input_and_remembers_password() {
    let room_input = "+room1:CB39A19549E8:ab-123-456";
    let canonical_room = "+room1:CB39A19549E8";
    let (mut owner, session_transport) =
        super::GuiPersistedConfigRuntimeOwner::with_config_path(None)
            .with_client_core_chat_session_runtime("alice", room_input)
            .expect("client-core chat runtime owner should bootstrap");
    let handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some(room_input.to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    handle.drain_actions();

    let startup_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert_eq!(startup_protocol_lines.len(), 1);
    assert!(startup_protocol_lines[0].contains("\"Hello\""));
    assert!(startup_protocol_lines[0].contains(canonical_room));
    assert!(
        !startup_protocol_lines[0].contains("AB-123-456"),
        "startup hello should not leak the controlled-room password"
    );

    session_transport.push_inbound_protocol_line(format!(
            r#"{{"Hello":{{"username":"alice","room":{{"name":"{canonical_room}"}},"version":"1.7.5","features":{{"chat":true}}}}}}"#
        ));
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    handle.drain_actions();

    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert_eq!(outbound_protocol_lines.len(), 1);
    assert!(outbound_protocol_lines[0].contains("\"controllerAuth\""));
    assert!(outbound_protocol_lines[0].contains(canonical_room));
    assert!(outbound_protocol_lines[0].contains("\"AB-123-456\""));
}

#[test]
fn gui_persisted_config_runtime_owner_routes_client_core_chat_transport_lines() {
    let (mut owner, session_transport) =
        super::GuiPersistedConfigRuntimeOwner::with_config_path(None)
            .with_client_core_chat_session_runtime("alice", "room1")
            .expect("client-core chat runtime owner should bootstrap");
    let handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let startup_actions = handle.drain_actions();
    assert_eq!(
        startup_actions,
        vec![
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(MainWindowRuntimeSnapshot {
                room_name: "room1".to_owned(),
                shared_playlist_enabled: false,
                controlled_room_active: false,
                users: vec![browser_runtime_user("alice", "room1", true, false, false)],
                playlist: Vec::new(),
                chat: Vec::new(),
                can_toggle_pause: false,
                can_seek: false,
                can_set_ready: false,
                can_manage_playlist: false,
                playback_paused: false,
                autoplay_active: false,
                hide_empty_rooms: false,
                rooms: browser_runtime_rooms("room1", false, true),
                ..Default::default()
            }),
            GuiShellAction::ApplyMenuDialogRuntimeSnapshot(MenuDialogRuntimeSnapshot {
                action_overrides: vec![MenuActionRuntimeOverride {
                    section_title: "Window",
                    action_label: "Show Chat",
                    enabled: false,
                }],
                tls_prompt_expected: state.menus.tls_prompt_expected,
                update_notice_expected: state.menus.update_notice_expected,
                about_dialog_available: state.menus.about_dialog_available,
            }),
            GuiShellAction::ApplyGuiCommandRuntimeSnapshot(GuiCommandRuntimeSnapshot {
                command_availability: GuiCommandAvailabilityState {
                    can_save_configuration: true,
                    can_reset_configuration: false,
                    can_reload_configuration: true,
                    can_connect_public_server: false,
                    can_connect_saved_server: false,
                    can_refresh_public_servers: true,
                    can_disconnect_session: true,
                    can_search_missing_media: false,
                    can_toggle_pause: false,
                    can_send_chat_message: false,
                },
                pending_operation: None,
            }),
        ]
    );
    for action in startup_actions {
        assert!(state.apply(action));
    }
    assert!(
        state
            .menus
            .sections
            .iter()
            .find(|section| section.title == "Window")
            .and_then(|section| {
                section
                    .actions
                    .iter()
                    .find(|action| action.label == "Show Chat")
            })
            .is_some_and(|action| !action.enabled)
    );
    assert!(!state.commands.can_send_chat_message);

    let startup_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert_eq!(startup_protocol_lines.len(), 1);
    assert!(startup_protocol_lines[0].contains("\"Hello\""));
    session_transport.push_inbound_protocol_line(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        );
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let hello_actions = handle.drain_actions();
    assert_eq!(
        hello_actions,
        vec![
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(MainWindowRuntimeSnapshot {
                room_name: "room1".to_owned(),
                shared_playlist_enabled: false,
                controlled_room_active: false,
                users: vec![browser_runtime_user("alice", "room1", true, false, false)],
                playlist: Vec::new(),
                chat: Vec::new(),
                can_toggle_pause: false,
                can_seek: false,
                can_set_ready: true,
                can_set_others_ready: true,
                can_manage_playlist: false,
                playback_paused: false,
                autoplay_active: false,
                hide_empty_rooms: false,
                rooms: browser_runtime_rooms("room1", false, true),
                ..Default::default()
            }),
            GuiShellAction::ApplyMenuDialogRuntimeSnapshot(MenuDialogRuntimeSnapshot {
                action_overrides: vec![
                    MenuActionRuntimeOverride {
                        section_title: "Window",
                        action_label: "Show Chat",
                        enabled: true,
                    },
                    MenuActionRuntimeOverride {
                        section_title: "Advanced",
                        action_label: "Create Controlled Room",
                        enabled: true,
                    },
                ],
                tls_prompt_expected: state.menus.tls_prompt_expected,
                update_notice_expected: state.menus.update_notice_expected,
                about_dialog_available: state.menus.about_dialog_available,
            }),
            GuiShellAction::ApplyGuiCommandRuntimeSnapshot(GuiCommandRuntimeSnapshot {
                command_availability: GuiCommandAvailabilityState {
                    can_save_configuration: true,
                    can_reset_configuration: false,
                    can_reload_configuration: true,
                    can_connect_public_server: false,
                    can_connect_saved_server: false,
                    can_refresh_public_servers: true,
                    can_disconnect_session: true,
                    can_search_missing_media: false,
                    can_toggle_pause: false,
                    can_send_chat_message: true,
                },
                pending_operation: None,
            }),
        ]
    );
    for action in hello_actions {
        assert!(state.apply(action));
    }

    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert!(outbound_protocol_lines.is_empty());
    assert!(
        state
            .menus
            .sections
            .iter()
            .find(|section| section.title == "Window")
            .and_then(|section| {
                section
                    .actions
                    .iter()
                    .find(|action| action.label == "Show Chat")
            })
            .is_some_and(|action| action.enabled)
    );
    assert!(state.commands.can_send_chat_message);

    assert!(state.apply(GuiShellAction::BeginLocalChatSend("hello room".to_owned(),)));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SendChatMessage("hello room".to_owned()),
    ));
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let outbound_actions = handle.drain_actions();
    assert!(
        outbound_actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteLocalChatSend)),
        "queued owner should still complete the local chat send when the session runtime accepts it"
    );
    for action in outbound_actions {
        assert!(state.apply(action));
    }

    let outbound_protocol_lines = session_transport.drain_outbound_protocol_lines();
    assert_eq!(outbound_protocol_lines.len(), 1);
    assert!(outbound_protocol_lines[0].contains("\"Chat\""));
    assert!(outbound_protocol_lines[0].contains("hello room"));

    session_transport
        .push_inbound_protocol_line(r#"{"Chat":{"username":"alice","message":"hello room"}}"#);
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert!(
        handle.drain_actions().iter().any(|action| matches!(
            action,
            GuiShellAction::PushChatMessage { sender, message }
                if sender == "alice" && message == "hello room"
        )),
        "queued owner should turn inbound protocol chat into a GUI chat message action"
    );
    assert!(session_transport.drain_outbound_protocol_lines().is_empty());
}

#[test]
fn gui_persisted_config_runtime_owner_routes_missing_media_search_through_client_core_session() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        opened_paths: Vec<String>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl super::PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn open_file(&mut self, path: &str) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .opened_paths
                .push(path.to_owned());
            Ok(())
        }
    }

    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "syncplay-gui-owner-missing-media-search-{}-{unique_suffix}",
        std::process::id()
    ));
    let nested = root.join("nested");
    let found_path = nested.join("episode2.mkv");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&nested)
        .expect("test missing-media search directory tree should be created");
    std::fs::write(&found_path, b"test").expect("test missing-media search file should be written");
    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));

    let (mut owner, session_transport) =
        super::GuiPersistedConfigRuntimeOwner::with_config_path(None)
            .with_client_core_chat_session_runtime("alice", "room1")
            .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(super::GuiOwnedPlayer::Custom(Box::new(
        RecordingPlayerAdapter {
            state: player_state.clone(),
        },
    )));
    let handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });

    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    for action in handle.drain_actions() {
        assert!(state.apply(action));
    }
    assert_eq!(session_transport.drain_outbound_protocol_lines().len(), 1);

    session_transport.push_inbound_protocol_lines([
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#
                .to_owned(),
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#
                .to_owned(),
            r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#.to_owned(),
        ]);
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    for action in handle.drain_actions() {
        assert!(state.apply(action));
    }

    assert!(state.apply(GuiShellAction::BeginMissingMediaSearch));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SearchMissingMedia,
    ));
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let actions = handle.drain_actions();
    let found_path_text = found_path.to_string_lossy().into_owned();
    let expected_message =
        format!("Opened media file through the attached recording player: {found_path_text}.");
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::ApplyGuiCommandRuntimeSnapshot(GuiCommandRuntimeSnapshot {
                pending_operation: None,
                ..
            })
        )),
        "queued owner should clear the pending search before continuing the session"
    );
    assert!(
            actions.iter().any(|action| matches!(
                action,
                GuiShellAction::PushTransientNotification { level, message }
                    if *level == GuiTransientNotificationLevel::Success
                        && message
                            == &format!(
                                "Opened media file through the attached recording player: {found_path_text}."
                            )
            )),
            "queued owner should continue the session with the located file through the attached player"
        );
    assert!(
        !actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteMissingMediaSearch(_))),
        "queued owner should continue through the player path instead of stopping at a found-path completion"
    );
    for action in actions {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some(expected_message.as_str())
    );
    assert_eq!(state.active_view, GuiShellView::MainWindow);
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths,
        vec![found_path_text]
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_routes_public_server_refresh_through_client_core_session() {
    let (mut owner, session_transport) =
        super::GuiPersistedConfigRuntimeOwner::with_config_path(None)
            .with_client_core_chat_session_runtime("alice", "room1")
            .expect("client-core chat runtime owner should bootstrap");
    let handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        public_servers: Some(vec![
            (" Primary ".to_owned(), " syncplay.pl:8999 ".to_owned()),
            ("Duplicate".to_owned(), "SYNCPLAY.PL:8999".to_owned()),
            ("Invalid".to_owned(), " :9000 ".to_owned()),
            ("Backup".to_owned(), "backup.example:9000".to_owned()),
        ]),
        ..StoredClientSettingsMvp::default()
    });

    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    for action in handle.drain_actions() {
        assert!(state.apply(action));
    }
    assert_eq!(session_transport.drain_outbound_protocol_lines().len(), 1);

    assert!(state.apply(GuiShellAction::BeginPublicServerRefresh));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::RefreshPublicServers(vec![]),
    ));
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let actions = handle.drain_actions();
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::CompletePublicServerRefresh(servers)
                if servers
                    == &vec![
                        ("Primary".to_owned(), "syncplay.pl:8999".to_owned()),
                        ("Backup".to_owned(), "backup.example:9000".to_owned()),
                    ]
        )),
        "queued owner should route public-server refresh through the client-core session runtime"
    );
    for action in actions {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());
    assert_eq!(
        state
            .public_servers
            .servers
            .iter()
            .map(|row| (row.label.clone(), row.address.clone()))
            .collect::<Vec<_>>(),
        vec![
            ("Primary".to_owned(), "syncplay.pl:8999".to_owned()),
            ("Backup".to_owned(), "backup.example:9000".to_owned()),
        ]
    );
}

#[test]
fn gui_persisted_config_runtime_owner_keeps_chat_disabled_until_server_hello_reports_support() {
    let (mut owner, session_transport) =
        super::GuiPersistedConfigRuntimeOwner::with_config_path(None)
            .with_client_core_chat_session_runtime("alice", "room1")
            .expect("client-core chat runtime owner should bootstrap");
    let handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let startup_actions = handle.drain_actions();
    assert_eq!(
        startup_actions,
        vec![
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(MainWindowRuntimeSnapshot {
                room_name: "room1".to_owned(),
                shared_playlist_enabled: false,
                controlled_room_active: false,
                users: vec![browser_runtime_user("alice", "room1", true, false, false)],
                playlist: Vec::new(),
                chat: Vec::new(),
                can_toggle_pause: false,
                can_seek: false,
                can_set_ready: false,
                can_manage_playlist: false,
                playback_paused: false,
                autoplay_active: false,
                hide_empty_rooms: false,
                rooms: browser_runtime_rooms("room1", false, true),
                ..Default::default()
            }),
            GuiShellAction::ApplyMenuDialogRuntimeSnapshot(MenuDialogRuntimeSnapshot {
                action_overrides: vec![MenuActionRuntimeOverride {
                    section_title: "Window",
                    action_label: "Show Chat",
                    enabled: false,
                }],
                tls_prompt_expected: state.menus.tls_prompt_expected,
                update_notice_expected: state.menus.update_notice_expected,
                about_dialog_available: state.menus.about_dialog_available,
            }),
            GuiShellAction::ApplyGuiCommandRuntimeSnapshot(GuiCommandRuntimeSnapshot {
                command_availability: GuiCommandAvailabilityState {
                    can_save_configuration: true,
                    can_reset_configuration: false,
                    can_reload_configuration: true,
                    can_connect_public_server: false,
                    can_connect_saved_server: false,
                    can_refresh_public_servers: true,
                    can_disconnect_session: true,
                    can_search_missing_media: false,
                    can_toggle_pause: false,
                    can_send_chat_message: false,
                },
                pending_operation: None,
            }),
        ]
    );
    for action in startup_actions {
        assert!(state.apply(action));
    }
    assert_eq!(session_transport.drain_outbound_protocol_lines().len(), 1);
    assert!(
        state
            .menus
            .sections
            .iter()
            .find(|section| section.title == "Window")
            .and_then(|section| {
                section
                    .actions
                    .iter()
                    .find(|action| action.label == "Show Chat")
            })
            .is_some_and(|action| !action.enabled)
    );
    assert!(!state.commands.can_send_chat_message);

    session_transport.push_inbound_protocol_line(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        );
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let hello_actions = handle.drain_actions();
    assert_eq!(
        hello_actions,
        vec![
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(MainWindowRuntimeSnapshot {
                room_name: "room1".to_owned(),
                shared_playlist_enabled: false,
                controlled_room_active: false,
                users: vec![browser_runtime_user("alice", "room1", true, false, false)],
                playlist: Vec::new(),
                chat: Vec::new(),
                can_toggle_pause: false,
                can_seek: false,
                can_set_ready: true,
                can_set_others_ready: true,
                can_manage_playlist: false,
                playback_paused: false,
                autoplay_active: false,
                hide_empty_rooms: false,
                rooms: browser_runtime_rooms("room1", false, true),
                ..Default::default()
            }),
            GuiShellAction::ApplyMenuDialogRuntimeSnapshot(MenuDialogRuntimeSnapshot {
                action_overrides: vec![
                    MenuActionRuntimeOverride {
                        section_title: "Window",
                        action_label: "Show Chat",
                        enabled: true,
                    },
                    MenuActionRuntimeOverride {
                        section_title: "Advanced",
                        action_label: "Create Controlled Room",
                        enabled: true,
                    },
                ],
                tls_prompt_expected: state.menus.tls_prompt_expected,
                update_notice_expected: state.menus.update_notice_expected,
                about_dialog_available: state.menus.about_dialog_available,
            }),
            GuiShellAction::ApplyGuiCommandRuntimeSnapshot(GuiCommandRuntimeSnapshot {
                command_availability: GuiCommandAvailabilityState {
                    can_save_configuration: true,
                    can_reset_configuration: false,
                    can_reload_configuration: true,
                    can_connect_public_server: false,
                    can_connect_saved_server: false,
                    can_refresh_public_servers: true,
                    can_disconnect_session: true,
                    can_search_missing_media: false,
                    can_toggle_pause: false,
                    can_send_chat_message: true,
                },
                pending_operation: None,
            }),
        ]
    );
    for action in hello_actions {
        assert!(state.apply(action));
    }
    assert!(
        state
            .menus
            .sections
            .iter()
            .find(|section| section.title == "Window")
            .and_then(|section| {
                section
                    .actions
                    .iter()
                    .find(|action| action.label == "Show Chat")
            })
            .is_some_and(|action| action.enabled)
    );
    assert!(state.commands.can_send_chat_message);
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_restores_readiness_controls_after_server_hello() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut stale_snapshot = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
    stale_snapshot.can_set_ready = false;
    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        stale_snapshot
    )));

    let mut adapter = super::GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");
    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    adapter
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("inbound server hello should apply");

    let mut expected_snapshot = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
    expected_snapshot.can_set_ready = true;
    expected_snapshot.can_set_others_ready = true;
    let actions = super::GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    assert_eq!(
        actions,
        vec![
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(expected_snapshot),
            GuiShellAction::ApplyMenuDialogRuntimeSnapshot(MenuDialogRuntimeSnapshot {
                action_overrides: vec![MenuActionRuntimeOverride {
                    section_title: "Advanced",
                    action_label: "Create Controlled Room",
                    enabled: true,
                }],
                tls_prompt_expected: state.menus.tls_prompt_expected,
                update_notice_expected: state.menus.update_notice_expected,
                about_dialog_available: state.menus.about_dialog_available,
            }),
        ]
    );
    for action in actions {
        assert!(state.apply(action));
    }
    assert!(state.main_window.playback.can_set_ready);
    assert!(super::GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state).is_empty());
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_clears_stale_shared_playlist_when_session_has_none()
{
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut stale_snapshot = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
    stale_snapshot.shared_playlist_enabled = true;
    stale_snapshot.playlist = vec!["episode1.mkv".to_owned(), "episode2.mkv".to_owned()];
    stale_snapshot.can_manage_playlist = true;
    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        stale_snapshot
    )));
    let mut stale_interaction = GuiInteractionRuntimeSnapshot::from_shell_state(&state);
    stale_interaction.selection.selected_main_window_playlist = Some(1);
    assert!(
        state.apply(GuiShellAction::ApplyGuiInteractionRuntimeSnapshot(
            stale_interaction
        ))
    );
    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![
                MenuActionRuntimeOverride {
                    section_title: "Window",
                    action_label: "Show Playlist",
                    enabled: true,
                },
                MenuActionRuntimeOverride {
                    section_title: "Playback",
                    action_label: "Playlist Actions",
                    enabled: true,
                },
            ],
            tls_prompt_expected: state.menus.tls_prompt_expected,
            update_notice_expected: state.menus.update_notice_expected,
            about_dialog_available: state.menus.about_dialog_available,
        },
    )));

    let mut adapter = super::GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");
    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    adapter
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("inbound server hello should apply");

    let actions = super::GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    assert_eq!(actions.len(), 3);
    let GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot) = &actions[0] else {
        panic!("stale shared-playlist state should be corrected through a main-window snapshot");
    };
    assert!(!snapshot.shared_playlist_enabled);
    assert!(snapshot.playlist.is_empty());
    assert!(!snapshot.can_manage_playlist);
    let GuiShellAction::ApplyGuiInteractionRuntimeSnapshot(interaction_snapshot) = &actions[1]
    else {
        panic!(
            "stale shared-playlist selection should be corrected through an interaction snapshot"
        );
    };
    assert_eq!(
        interaction_snapshot.selection.selected_main_window_playlist,
        None
    );
    let GuiShellAction::ApplyMenuDialogRuntimeSnapshot(menu_snapshot) = &actions[2] else {
        panic!("stale shared-playlist menu state should be corrected through a menu snapshot");
    };
    assert_eq!(
        menu_snapshot.action_overrides,
        vec![
            MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Playlist",
                enabled: false,
            },
            MenuActionRuntimeOverride {
                section_title: "Playback",
                action_label: "Playlist Actions",
                enabled: false,
            },
            MenuActionRuntimeOverride {
                section_title: "Advanced",
                action_label: "Create Controlled Room",
                enabled: true,
            },
        ]
    );
    for action in actions {
        assert!(state.apply(action));
    }
    assert!(!state.main_window.shared_playlist_enabled);
    assert!(state.main_window.playlist.is_empty());
    assert!(!state.main_window.playback.can_manage_playlist);
    assert_eq!(state.selection.selected_main_window_playlist, None);
    assert!(
        state
            .menus
            .sections
            .iter()
            .find(|section| section.title == "Window")
            .and_then(|section| {
                section
                    .actions
                    .iter()
                    .find(|action| action.label == "Show Playlist")
            })
            .is_some_and(|action| !action.enabled)
    );
    assert!(
        state
            .menus
            .sections
            .iter()
            .find(|section| section.title == "Playback")
            .and_then(|section| {
                section
                    .actions
                    .iter()
                    .find(|action| action.label == "Playlist Actions")
            })
            .is_some_and(|action| !action.enabled)
    );
    assert!(super::GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state).is_empty());
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_clears_stale_playback_pause_when_session_has_no_playstate()
 {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut stale_snapshot = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
    stale_snapshot.playback_paused = true;
    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        stale_snapshot
    )));
    assert!(state.main_window.playback_paused);

    let mut adapter = super::GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");
    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    adapter
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("inbound server hello should apply");

    let mut expected_snapshot = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
    expected_snapshot.playback_paused = false;
    expected_snapshot.can_set_others_ready = true;
    let actions = super::GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    assert_eq!(
        actions,
        vec![
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(expected_snapshot),
            GuiShellAction::ApplyMenuDialogRuntimeSnapshot(MenuDialogRuntimeSnapshot {
                action_overrides: vec![MenuActionRuntimeOverride {
                    section_title: "Advanced",
                    action_label: "Create Controlled Room",
                    enabled: true,
                }],
                tls_prompt_expected: state.menus.tls_prompt_expected,
                update_notice_expected: state.menus.update_notice_expected,
                about_dialog_available: state.menus.about_dialog_available,
            }),
        ]
    );
    for action in actions {
        assert!(state.apply(action));
    }
    assert!(!state.main_window.playback_paused);
    assert!(super::GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state).is_empty());
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_clears_stale_autoplay_state_when_session_has_no_override()
 {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut stale_snapshot = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
    stale_snapshot.autoplay_active = true;
    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        stale_snapshot
    )));
    assert!(state.main_window.autoplay_active);

    let mut adapter = super::GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");
    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    adapter
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("inbound server hello should apply");

    let mut expected_snapshot = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
    expected_snapshot.autoplay_active = false;
    expected_snapshot.can_set_others_ready = true;
    let actions = super::GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    assert_eq!(
        actions,
        vec![
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(expected_snapshot),
            GuiShellAction::ApplyMenuDialogRuntimeSnapshot(MenuDialogRuntimeSnapshot {
                action_overrides: vec![MenuActionRuntimeOverride {
                    section_title: "Advanced",
                    action_label: "Create Controlled Room",
                    enabled: true,
                }],
                tls_prompt_expected: state.menus.tls_prompt_expected,
                update_notice_expected: state.menus.update_notice_expected,
                about_dialog_available: state.menus.about_dialog_available,
            }),
        ]
    );
    for action in actions {
        assert!(state.apply(action));
    }
    assert!(!state.main_window.autoplay_active);
    assert!(super::GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state).is_empty());
}

#[test]
fn gui_persisted_config_runtime_owner_routes_client_core_chat_over_tcp_transport() {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        time::Duration,
    };

    let listener =
        TcpListener::bind("127.0.0.1:0").expect("test session transport listener should bind");
    let address = listener
        .local_addr()
        .expect("test session transport listener should expose a local address");
    let (hello_ready_tx, hello_ready_rx) = mpsc::channel();
    let server_thread = std::thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("test session transport server should accept one client");
        let reader_stream = stream
            .try_clone()
            .expect("test session transport server should clone the accepted stream");
        let mut reader = BufReader::new(reader_stream);
        let mut hello_line = String::new();
        reader
            .read_line(&mut hello_line)
            .expect("test session transport server should read one startup hello line");
        stream
                .write_all(
                    br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
                )
                .expect("test session transport server should write one inbound hello line");
        stream
            .write_all(b"\n")
            .expect("test session transport server should terminate the inbound hello line");
        hello_ready_tx
            .send(())
            .expect("test session transport server should signal hello readiness");
        let mut chat_line = String::new();
        reader
            .read_line(&mut chat_line)
            .expect("test session transport server should read one outbound chat line");
        stream
            .write_all(br#"{"Chat":{"username":"alice","message":"hello room"}}"#)
            .expect("test session transport server should write one inbound line");
        stream
            .write_all(b"\n")
            .expect("test session transport server should terminate the inbound line");
        (hello_line, chat_line)
    });

    let mut owner = super::GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime("alice", "room1", address.to_string())
        .expect("client-core tcp chat runtime owner should bootstrap");
    let handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let mut combined_actions = handle.drain_actions();
    for action in combined_actions.iter().cloned() {
        assert!(state.apply(action));
    }
    hello_ready_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("test session transport server should send its hello promptly");

    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let hello_sync_actions = handle.drain_actions();
    for action in hello_sync_actions.iter().cloned() {
        assert!(state.apply(action));
    }
    combined_actions.extend(hello_sync_actions);

    assert!(state.apply(GuiShellAction::BeginLocalChatSend("hello room".to_owned(),)));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SendChatMessage("hello room".to_owned()),
    ));
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let second_actions = handle.drain_actions();
    for action in second_actions.iter().cloned() {
        assert!(state.apply(action));
    }
    combined_actions.extend(second_actions);

    let (hello_line, chat_line) = server_thread
        .join()
        .expect("test session transport server thread should complete");
    assert!(hello_line.contains("\"Hello\""));
    assert!(hello_line.contains("\"alice\""));
    assert!(chat_line.contains("\"Chat\""));
    assert!(chat_line.contains("hello room"));

    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let third_actions = handle.drain_actions();
    for action in third_actions.iter().cloned() {
        assert!(state.apply(action));
    }
    combined_actions.extend(third_actions);

    assert!(
        combined_actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteLocalChatSend)),
        "tcp transport should preserve the local send completion"
    );
    assert!(
        combined_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushChatMessage { sender, message }
                if sender == "alice" && message == "hello room"
        )),
        "tcp transport should feed the server response back through the client-core chat adapter"
    );
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|entry| (entry.sender.clone(), entry.message.clone())),
        Some(("alice".to_owned(), "hello room".to_owned()))
    );
}

#[test]
fn gui_persisted_config_runtime_owner_routes_local_readiness_over_tcp_transport() {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        time::Duration,
    };

    let listener =
        TcpListener::bind("127.0.0.1:0").expect("test session transport listener should bind");
    let address = listener
        .local_addr()
        .expect("test session transport listener should expose a local address");
    let (hello_ready_tx, hello_ready_rx) = mpsc::channel();
    let server_thread = std::thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("test session transport server should accept one client");
        let reader_stream = stream
            .try_clone()
            .expect("test session transport server should clone the accepted stream");
        let mut reader = BufReader::new(reader_stream);
        let mut hello_line = String::new();
        reader
            .read_line(&mut hello_line)
            .expect("test session transport server should read one startup hello line");
        stream
                .write_all(
                    br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#,
                )
                .expect("test session transport server should write one inbound hello line");
        stream
            .write_all(b"\n")
            .expect("test session transport server should terminate the inbound hello line");
        hello_ready_tx
            .send(())
            .expect("test session transport server should signal hello readiness");
        let mut ready_line = String::new();
        reader
            .read_line(&mut ready_line)
            .expect("test session transport server should read one outbound ready line");
        stream
            .write_all(br#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
            .expect("test session transport server should write one inbound ready line");
        stream
            .write_all(b"\n")
            .expect("test session transport server should terminate the inbound ready line");
        ready_line
    });

    let mut owner = super::GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime("alice", "room1", address.to_string())
        .expect("client-core tcp chat runtime owner should bootstrap");
    let handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    hello_ready_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("test session transport server should send its hello promptly");

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    handle.push_request(GuiRuntimeRequest::SetLocalReady(true));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let ready_line = server_thread
        .join()
        .expect("test session transport server thread should complete");
    assert!(ready_line.contains("\"Set\""));
    assert!(ready_line.contains("\"ready\""));
    assert!(ready_line.contains("\"isReady\":true"));

    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        Duration::from_secs(1),
        |state| {
            state
                .main_window
                .users
                .iter()
                .any(|user| user.username == "alice" && user.is_self && user.is_ready)
        },
        "local readiness update over TCP transport",
    );
}

#[test]
fn gui_persisted_config_runtime_owner_routes_room_changes_over_tcp_transport() {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        time::Duration,
    };

    let listener =
        TcpListener::bind("127.0.0.1:0").expect("test session transport listener should bind");
    let address = listener
        .local_addr()
        .expect("test session transport listener should expose a local address");
    let (hello_ready_tx, hello_ready_rx) = mpsc::channel();
    let server_thread = std::thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("test session transport server should accept one client");
        let reader_stream = stream
            .try_clone()
            .expect("test session transport server should clone the accepted stream");
        let mut reader = BufReader::new(reader_stream);
        let mut hello_line = String::new();
        reader
            .read_line(&mut hello_line)
            .expect("test session transport server should read one startup hello line");
        stream
                .write_all(
                    br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#,
                )
                .expect("test session transport server should write one inbound hello line");
        stream
            .write_all(b"\n")
            .expect("test session transport server should terminate the inbound hello line");
        hello_ready_tx
            .send(())
            .expect("test session transport server should signal hello readiness");
        let mut room_line = String::new();
        reader
            .read_line(&mut room_line)
            .expect("test session transport server should read one outbound room-change line");
        stream
            .write_all(br#"{"Set":{"room":{"name":"room2"}}}"#)
            .expect("test session transport server should write one inbound room line");
        stream
            .write_all(b"\n")
            .expect("test session transport server should terminate the inbound room line");
        room_line
    });

    let mut owner = super::GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime("alice", "room1", address.to_string())
        .expect("client-core tcp chat runtime owner should bootstrap");
    let handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    hello_ready_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("test session transport server should send its hello promptly");

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    handle.push_request(GuiRuntimeRequest::SetRoom("room2".to_owned()));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let room_line = server_thread
        .join()
        .expect("test session transport server thread should complete");
    assert!(room_line.contains("\"Set\""));
    assert!(room_line.contains("\"room\""));
    assert!(room_line.contains("\"room2\""));

    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        Duration::from_secs(1),
        |state| state.main_window.room_name == "room2",
        "room change over TCP transport",
    );
    assert_eq!(state.main_window.room_name, "room2");
}

#[test]
fn gui_persisted_config_runtime_owner_rejects_room_changes_before_server_hello_without_optimistic_room_updates()
 {
    let (mut owner, _session_transport) =
        super::GuiPersistedConfigRuntimeOwner::with_config_path(None)
            .with_client_core_chat_session_runtime("alice", "room1")
            .expect("client-core chat runtime owner should bootstrap");
    let handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    handle.push_request(GuiRuntimeRequest::SetRoom("room2".to_owned()));
    let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert_eq!(state.main_window.room_name, "room1");
    assert!(
            actions.iter().any(|action| matches!(
                action,
                GuiShellAction::PushTransientNotification { level: GuiTransientNotificationLevel::Error, message }
                    if message.contains("server Hello completes")
            )),
            "pre-Hello room requests should surface the runtime error without changing the joined room",
        );
}

#[test]
fn gui_persisted_config_runtime_owner_emits_periodic_state_heartbeat_over_tcp_transport() {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        time::{Duration, Instant},
    };

    let listener =
        TcpListener::bind("127.0.0.1:0").expect("test session transport listener should bind");
    let address = listener
        .local_addr()
        .expect("test session transport listener should expose a local address");
    let (hello_tx, hello_rx) = mpsc::channel();
    let (heartbeat_tx, heartbeat_rx) = mpsc::channel();
    let server_thread = std::thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("test session transport server should accept one client");
        stream
            .set_read_timeout(Some(Duration::from_secs(4)))
            .expect("test session transport server should set a read timeout");
        let reader_stream = stream
            .try_clone()
            .expect("test session transport server should clone the accepted stream");
        let mut reader = BufReader::new(reader_stream);
        let mut hello_line = String::new();
        reader
            .read_line(&mut hello_line)
            .expect("test session transport server should read one startup hello line");
        hello_tx
            .send(hello_line)
            .expect("test session transport server should report the startup hello");
        stream
                .write_all(
                    br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#,
                )
                .expect("test session transport server should write one inbound hello line");
        stream
            .write_all(b"\n")
            .expect("test session transport server should terminate the inbound hello line");

        let mut heartbeat_line = String::new();
        reader
            .read_line(&mut heartbeat_line)
            .expect("test session transport server should read one outbound heartbeat line");
        heartbeat_tx
            .send(heartbeat_line)
            .expect("test session transport server should report the heartbeat line");
    });

    let mut owner = super::GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime("alice", "room1", address.to_string())
        .expect("client-core tcp chat runtime owner should bootstrap");
    let handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let hello_line = hello_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("test session transport server should receive the startup hello");
    assert!(hello_line.contains("\"Hello\""));
    assert!(hello_line.contains("\"alice\""));
    assert!(hello_line.contains("\"room1\""));

    let deadline = Instant::now() + Duration::from_secs(2);
    let heartbeat_line = loop {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        if let Ok(line) = heartbeat_rx.try_recv() {
            break line;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for a GUI heartbeat line over TCP transport"
        );
        std::thread::sleep(Duration::from_millis(10));
    };

    assert!(heartbeat_line.contains("\"State\""));
    assert!(heartbeat_line.contains("\"ping\""));
    assert!(
        heartbeat_line.contains("\"clientLatencyCalculation\""),
        "heartbeat should include client ping metrics"
    );

    server_thread
        .join()
        .expect("test session transport server thread should complete");
}

#[test]
fn gui_persisted_config_runtime_owner_returns_to_default_room_over_tcp_transport() {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        time::Duration,
    };

    let listener =
        TcpListener::bind("127.0.0.1:0").expect("test session transport listener should bind");
    let address = listener
        .local_addr()
        .expect("test session transport listener should expose a local address");
    let (hello_ready_tx, hello_ready_rx) = mpsc::channel();
    let (release_leave_tx, release_leave_rx) = mpsc::channel();
    let server_thread = std::thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("test session transport server should accept one client");
        let reader_stream = stream
            .try_clone()
            .expect("test session transport server should clone the accepted stream");
        let mut reader = BufReader::new(reader_stream);
        let mut hello_line = String::new();
        reader
            .read_line(&mut hello_line)
            .expect("test session transport server should read one startup hello line");
        stream
                .write_all(
                    br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#,
                )
                .expect("test session transport server should write one inbound hello line");
        stream
            .write_all(b"\n")
            .expect("test session transport server should terminate the inbound hello line");
        hello_ready_tx
            .send(())
            .expect("test session transport server should signal hello readiness");

        let mut join_line = String::new();
        reader
            .read_line(&mut join_line)
            .expect("test session transport server should read one outbound room-change line");
        stream
            .write_all(br#"{"Set":{"room":{"name":"room2"}}}"#)
            .expect("test session transport server should write one inbound room line");
        stream
            .write_all(b"\n")
            .expect("test session transport server should terminate the inbound room line");

        let mut leave_line = String::new();
        reader
            .read_line(&mut leave_line)
            .expect("test session transport server should read one outbound default-room line");
        release_leave_rx
            .recv_timeout(Duration::from_secs(1))
            .expect(
                "test session transport server should be released for the default-room response",
            );
        stream
            .write_all(br#"{"Set":{"room":{"name":"room1"}}}"#)
            .expect("test session transport server should write one inbound default-room line");
        stream
            .write_all(b"\n")
            .expect("test session transport server should terminate the inbound default-room line");

        (join_line, leave_line)
    });

    let mut owner = super::GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime("alice", "room1", address.to_string())
        .expect("client-core tcp chat runtime owner should bootstrap");
    let handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    hello_ready_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("test session transport server should send its hello promptly");
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    handle.push_request(GuiRuntimeRequest::SetRoom("room2".to_owned()));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        Duration::from_secs(1),
        |state| state.main_window.room_name == "room2",
        "room join before default-room return",
    );
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Room joined: room2.")
    );

    handle.push_request(GuiRuntimeRequest::ReturnToDefaultRoom);
    let leave_request_actions =
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert_eq!(state.main_window.room_name, "room2");
    assert!(
        leave_request_actions.iter().all(|action| !matches!(
            action,
            GuiShellAction::PushTransientNotification { message, .. }
                if message == "Returned to default room: room1."
        )),
        "the room should not be reported as left before the runtime confirms the fallback room",
    );

    release_leave_tx
        .send(())
        .expect("test session transport server should be releasable for the default-room response");
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        Duration::from_secs(1),
        |state| state.main_window.room_name == "room1",
        "default-room return over TCP transport",
    );

    let (join_line, leave_line) = server_thread
        .join()
        .expect("test session transport server thread should complete");
    assert!(join_line.contains("\"room2\""));
    assert!(leave_line.contains("\"room1\""));
    assert_eq!(state.main_window.room_name, "room1");
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Returned to default room: room1.")
    );
    assert_eq!(
        state.configuration.to_stored_settings().room.as_deref(),
        Some("room1")
    );
}

#[test]
fn gui_persisted_config_runtime_owner_reconnects_client_core_tcp_session_for_public_server_connect()
{
    use std::{
        io::{BufRead, BufReader},
        net::TcpListener,
        sync::mpsc,
        time::Duration,
    };

    let first_listener = TcpListener::bind("127.0.0.1:0")
        .expect("first test session transport listener should bind");
    let first_address = first_listener
        .local_addr()
        .expect("first test session transport listener should expose a local address");
    let second_listener = TcpListener::bind("127.0.0.1:0")
        .expect("second test session transport listener should bind");
    let second_address = second_listener
        .local_addr()
        .expect("second test session transport listener should expose a local address");

    let (first_hello_tx, first_hello_rx) = mpsc::channel();
    let (release_first_tx, release_first_rx) = mpsc::channel();
    let first_server_thread = std::thread::spawn(move || {
        let (stream, _) = first_listener
            .accept()
            .expect("first test session transport server should accept one client");
        let mut reader = BufReader::new(stream);
        let mut hello_line = String::new();
        reader
            .read_line(&mut hello_line)
            .expect("first test session transport server should read one startup hello line");
        first_hello_tx
            .send(hello_line)
            .expect("first test session transport server should report its hello");
        release_first_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("first test session transport server should be released after reconnect");
    });

    let (second_hello_tx, second_hello_rx) = mpsc::channel();
    let second_server_thread = std::thread::spawn(move || {
        let (stream, _) = second_listener
            .accept()
            .expect("second test session transport server should accept one client");
        let mut reader = BufReader::new(stream);
        let mut hello_line = String::new();
        reader
            .read_line(&mut hello_line)
            .expect("second test session transport server should read one reconnect hello line");
        second_hello_tx
            .send(hello_line)
            .expect("second test session transport server should report its hello");
    });

    let mut owner = super::GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime("alice", "room1", first_address.to_string())
        .expect("client-core tcp chat runtime owner should bootstrap");
    let handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        public_servers: Some(vec![("Secondary".to_owned(), second_address.to_string())]),
        ..StoredClientSettingsMvp::default()
    });

    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    for action in handle.drain_actions() {
        assert!(state.apply(action));
    }

    let first_hello_line = first_hello_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("first test session transport server should receive the startup hello");
    assert!(first_hello_line.contains("\"Hello\""));
    assert!(first_hello_line.contains("\"alice\""));

    let mut stale_main_window = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
    stale_main_window.shared_playlist_enabled = true;
    stale_main_window.playlist = vec!["episode2.mkv".to_owned()];
    stale_main_window.can_set_ready = true;
    stale_main_window.playback_paused = true;
    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        stale_main_window
    )));

    let mut stale_interaction = GuiInteractionRuntimeSnapshot::from_shell_state(&state);
    stale_interaction.selection.selected_main_window_playlist = Some(0);
    assert!(
        state.apply(GuiShellAction::ApplyGuiInteractionRuntimeSnapshot(
            stale_interaction
        ))
    );

    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Playlist",
                enabled: true,
            }],
            tls_prompt_expected: state.menus.tls_prompt_expected,
            update_notice_expected: state.menus.update_notice_expected,
            about_dialog_available: state.menus.about_dialog_available,
        }
    )));
    assert!(state.main_window.shared_playlist_enabled);
    assert_eq!(state.main_window.playlist.len(), 1);
    assert_eq!(state.selection.selected_main_window_playlist, Some(0));
    assert!(
        state
            .menus
            .sections
            .iter()
            .find(|section| section.title == "Window")
            .and_then(|section| {
                section
                    .actions
                    .iter()
                    .find(|action| action.label == "Show Playlist")
            })
            .is_some_and(|action| action.enabled)
    );

    assert!(state.apply(GuiShellAction::BeginSelectedPublicServerConnect));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::ConnectPublicServer,
    ));
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let reconnect_actions = handle.drain_actions();
    assert!(
        reconnect_actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteSelectedPublicServerConnect)),
        "public-server connect should complete through the client-core session runtime"
    );
    assert!(
        reconnect_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot)
                if !snapshot.shared_playlist_enabled
                    && snapshot.playlist.is_empty()
                    && !snapshot.can_set_ready
                    && !snapshot.playback_paused
        )),
        "public-server reconnect should clear stale session-owned main-window state before the new server replies"
    );
    assert!(
        reconnect_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::ApplyGuiInteractionRuntimeSnapshot(snapshot)
                if snapshot.selection.selected_main_window_playlist.is_none()
        )),
        "public-server reconnect should clear stale playlist selection before the new server replies"
    );
    assert!(
        reconnect_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::ApplyMenuDialogRuntimeSnapshot(snapshot)
                if snapshot.action_overrides.contains(&MenuActionRuntimeOverride {
                    section_title: "Window",
                    action_label: "Show Playlist",
                    enabled: false,
                })
        )),
        "public-server reconnect should clear stale playlist menu state before the new server replies"
    );
    for action in reconnect_actions {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());
    assert!(!state.main_window.shared_playlist_enabled);
    assert!(state.main_window.playlist.is_empty());
    assert!(!state.main_window.playback.can_set_ready);
    assert!(!state.main_window.playback_paused);
    assert_eq!(state.selection.selected_main_window_playlist, None);
    assert!(
        state
            .menus
            .sections
            .iter()
            .find(|section| section.title == "Window")
            .and_then(|section| {
                section
                    .actions
                    .iter()
                    .find(|action| action.label == "Show Playlist")
            })
            .is_some_and(|action| !action.enabled)
    );

    let second_hello_line = second_hello_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("second test session transport server should receive the reconnect hello");
    assert!(second_hello_line.contains("\"Hello\""));
    assert!(second_hello_line.contains("\"alice\""));
    assert!(second_hello_line.contains("\"room1\""));

    release_first_tx
        .send(())
        .expect("first test session transport server should be releasable");
    first_server_thread
        .join()
        .expect("first test session transport server thread should complete");
    second_server_thread
        .join()
        .expect("second test session transport server thread should complete");
}

#[test]
fn gui_persisted_config_runtime_owner_projects_live_python_peer_chat_interop() {
    let result = match super::live_python_interop::run_live_python_peer_connect_flow() {
        Ok(result) => result,
        Err(error)
            if super::live_python_interop::live_python_interop_prerequisites_missing(&error) =>
        {
            eprintln!(
                "live Python GUI interop chat test skipped due to missing local prerequisites"
            );
            return;
        }
        Err(error) => {
            panic!("live Python GUI interop chat flow should succeed, got: {error}")
        }
    };

    assert_eq!(
        result.room_name,
        super::live_python_interop::LIVE_PYTHON_INTEROP_ROOM
    );
    assert!(result.local_user_present);
    assert!(result.peer_user_present);
    assert!(result.room_switch_observed);
    assert!(result.room_rejoin_observed);
    assert!(result.peer_disconnect_observed);
    assert!(result.peer_reconnect_observed);
    assert_eq!(
        result.gui_playlist,
        vec![
            super::live_python_interop::LIVE_PYTHON_INTEROP_PEER_PLAYLIST_ENTRY_ONE.to_owned(),
            super::live_python_interop::LIVE_PYTHON_INTEROP_PEER_PLAYLIST_ENTRY_TWO.to_owned(),
        ]
    );
    assert_eq!(result.gui_playlist_index, Some(1));
    assert_eq!(
        result.peer_playlist,
        vec![
            super::live_python_interop::LIVE_PYTHON_INTEROP_PEER_PLAYLIST_ENTRY_ONE.to_owned(),
            super::live_python_interop::LIVE_PYTHON_INTEROP_PEER_PLAYLIST_ENTRY_TWO.to_owned(),
        ]
    );
    assert_eq!(result.peer_playlist_index, Some(1));
    assert!(result.gui_chat_messages.iter().any(|message| {
        message.sender == super::live_python_interop::LIVE_PYTHON_INTEROP_LOCAL_USERNAME
            && message.message == super::live_python_interop::LIVE_PYTHON_INTEROP_LOCAL_CHAT_MESSAGE
    }));
    assert!(result.gui_chat_messages.iter().any(|message| {
        message.sender == super::live_python_interop::LIVE_PYTHON_INTEROP_PEER_USERNAME
            && message.message == super::live_python_interop::LIVE_PYTHON_INTEROP_PEER_CHAT_MESSAGE
    }));
    assert!(result.gui_chat_messages.iter().any(|message| {
        message.sender == super::live_python_interop::LIVE_PYTHON_INTEROP_LOCAL_USERNAME
            && message.message
                == super::live_python_interop::LIVE_PYTHON_INTEROP_LOCAL_RECONNECT_CHAT_MESSAGE
    }));
    assert!(result.gui_chat_messages.iter().any(|message| {
        message.sender == super::live_python_interop::LIVE_PYTHON_INTEROP_PEER_USERNAME
            && message.message
                == super::live_python_interop::LIVE_PYTHON_INTEROP_PEER_RECONNECT_CHAT_MESSAGE
    }));
    assert!(result.peer_chat_messages.iter().any(|message| {
        message.sender == super::live_python_interop::LIVE_PYTHON_INTEROP_LOCAL_USERNAME
            && message.message == super::live_python_interop::LIVE_PYTHON_INTEROP_LOCAL_CHAT_MESSAGE
    }));
    assert!(result.peer_chat_messages.iter().any(|message| {
        message.sender == super::live_python_interop::LIVE_PYTHON_INTEROP_PEER_USERNAME
            && message.message == super::live_python_interop::LIVE_PYTHON_INTEROP_PEER_CHAT_MESSAGE
    }));
    assert!(result.peer_chat_messages.iter().any(|message| {
        message.sender == super::live_python_interop::LIVE_PYTHON_INTEROP_LOCAL_USERNAME
            && message.message
                == super::live_python_interop::LIVE_PYTHON_INTEROP_LOCAL_RECONNECT_CHAT_MESSAGE
    }));
    assert!(result.peer_chat_messages.iter().any(|message| {
        message.sender == super::live_python_interop::LIVE_PYTHON_INTEROP_PEER_USERNAME
            && message.message
                == super::live_python_interop::LIVE_PYTHON_INTEROP_PEER_RECONNECT_CHAT_MESSAGE
    }));
    assert!(result.widget_count > 0);
}

#[test]
fn gui_persisted_config_runtime_owner_projects_live_python_peer_detached_connect_interop() {
    let result =
        match super::live_python_interop::run_live_python_peer_detached_public_server_connect_flow()
        {
            Ok(result) => result,
            Err(error)
                if super::live_python_interop::live_python_interop_prerequisites_missing(
                    &error,
                ) =>
            {
                eprintln!(
                    "live Python GUI detached-connect test skipped due to missing local prerequisites"
                );
                return;
            }
            Err(error) => {
                panic!(
                    "live Python GUI detached public-server connect flow should succeed, got: {error}"
                )
            }
        };

    assert_eq!(
        result.room_name,
        super::live_python_interop::LIVE_PYTHON_INTEROP_ROOM
    );
    assert!(result.local_user_present);
    assert!(result.peer_user_present);
    assert!(!result.local_user_ready);
    assert!(!result.peer_user_ready);
    assert!(result.widget_count > 0);
}

#[test]
fn gui_persisted_config_runtime_owner_projects_live_python_peer_controlled_room_interop() {
    let result = match super::live_python_interop::run_live_python_peer_controlled_room_flow() {
        Ok(result) => result,
        Err(error)
            if super::live_python_interop::live_python_interop_prerequisites_missing(&error) =>
        {
            eprintln!(
                "live Python GUI controlled-room test skipped due to missing local prerequisites"
            );
            return;
        }
        Err(error) => {
            panic!("live Python GUI controlled-room flow should succeed, got: {error}")
        }
    };

    assert_eq!(
        result.room_name,
        super::live_python_interop::LIVE_PYTHON_INTEROP_CONTROLLED_ROOM
    );
    assert!(result.local_user_present);
    assert!(result.peer_user_present);
    assert!(result.local_user_controller);
    assert!(!result.peer_user_controller);
    assert!(!result.peer_local_controller);
    assert!(result.can_manage_playlist);
    assert!(result.widget_count > 0);
}

#[test]
fn gui_persisted_config_runtime_owner_loopback_transport_echoes_client_core_chat() {
    let mut owner = super::GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback chat runtime owner should bootstrap");
    let handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::BeginLocalChatSend("hello room".to_owned(),)));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SendChatMessage("hello room".to_owned()),
    ));
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);

    let actions = handle.drain_actions();
    assert!(
        actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteLocalChatSend)),
        "loopback transport should preserve the local send completion"
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushChatMessage { sender, message }
                if sender == "alice" && message == "hello room"
        )),
        "loopback transport should feed the encoded chat line back through inbound handling"
    );
    for action in actions {
        assert!(state.apply(action));
    }
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|entry| (entry.sender.clone(), entry.message.clone())),
        Some(("alice".to_owned(), "hello room".to_owned()))
    );
}

#[test]
fn gui_portable_smoke_regression_sequences_persistence_and_transport_flows() {
    use std::{
        io::{BufRead, BufReader},
        net::TcpListener,
        sync::mpsc,
        time::Duration,
    };

    // Persistence save + reload (portable equivalent of the isolated config checks).
    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "syncplay-gui-portable-smoke-{}-{unique_suffix}.ini",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);

    let mut persisted_owner =
        super::GuiPersistedConfigRuntimeOwner::with_config_path(Some(path.clone()));
    let persisted_handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let mut persisted_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    let saved_settings = StoredClientSettingsMvp {
        host: Some("portable-save.example".to_owned()),
        room: Some("portable-room-a".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    assert!(persisted_state.apply(GuiShellAction::BeginConfigurationSave));
    persisted_handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SaveConfiguration(saved_settings.clone()),
    ));
    super::GuiQueuedRuntimeOwner::pump(&mut persisted_owner, &persisted_handle, &persisted_state);
    let save_actions = persisted_handle.drain_actions();
    assert_eq!(
        save_actions,
        vec![GuiShellAction::CompleteConfigurationSave(
            saved_settings.clone()
        )]
    );
    for action in save_actions {
        assert!(persisted_state.apply(action));
    }
    assert_eq!(
        super::load_syncplay_ini_stored_client_settings_mvp_from_path(&path)
            .expect("portable smoke save should leave a readable config"),
        Some(saved_settings.clone())
    );

    let reloaded_settings = StoredClientSettingsMvp {
        host: Some("portable-reload.example".to_owned()),
        room: Some("portable-room-b".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    super::upsert_syncplay_ini_stored_client_settings_mvp_at_path(&path, &reloaded_settings)
        .expect("portable smoke reload seed should write config");
    assert!(persisted_state.apply(GuiShellAction::BeginConfigurationReload));
    persisted_handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::ReloadConfiguration(StoredClientSettingsMvp::default()),
    ));
    super::GuiQueuedRuntimeOwner::pump(&mut persisted_owner, &persisted_handle, &persisted_state);
    let reload_actions = persisted_handle.drain_actions();
    assert!(
        reload_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::CompleteConfigurationReload(settings)
                if settings == &reloaded_settings
        )),
        "portable nontransport smoke reload should emit completion with reloaded settings"
    );
    for action in reload_actions {
        assert!(persisted_state.apply(action));
    }
    assert_eq!(
        persisted_state.saved_configuration.host.as_deref(),
        Some("portable-reload.example")
    );
    let _ = std::fs::remove_file(&path);

    // Loopback transport chat echo.
    let mut loopback_owner = super::GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("portable-user", "portable-room")
        .expect("portable smoke loopback runtime owner should bootstrap");
    let loopback_handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let mut loopback_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            chat_input_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        });
    assert!(loopback_state.apply(GuiShellAction::BeginLocalChatSend(
        "portable-loopback".to_owned()
    )));
    loopback_handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SendChatMessage("portable-loopback".to_owned()),
    ));
    super::GuiQueuedRuntimeOwner::pump(&mut loopback_owner, &loopback_handle, &loopback_state);
    let loopback_actions = loopback_handle.drain_actions();
    assert!(
        loopback_actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteLocalChatSend)),
        "portable smoke loopback segment should complete local chat sends"
    );
    assert!(
        loopback_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushChatMessage { sender, message }
                if sender == "portable-user" && message == "portable-loopback"
        )),
        "portable smoke loopback segment should echo chat through inbound handling"
    );
    for action in loopback_actions {
        assert!(loopback_state.apply(action));
    }
    assert_eq!(
        loopback_state
            .main_window
            .chat
            .last()
            .map(|entry| (entry.sender.clone(), entry.message.clone())),
        Some(("portable-user".to_owned(), "portable-loopback".to_owned()))
    );

    // TCP startup + reconnect swap.
    let first_listener =
        TcpListener::bind("127.0.0.1:0").expect("portable smoke first tcp listener should bind");
    let first_address = first_listener
        .local_addr()
        .expect("portable smoke first tcp listener should expose a local address");
    let second_listener =
        TcpListener::bind("127.0.0.1:0").expect("portable smoke second tcp listener should bind");
    let second_address = second_listener
        .local_addr()
        .expect("portable smoke second tcp listener should expose a local address");

    let (first_hello_tx, first_hello_rx) = mpsc::channel();
    let (release_first_tx, release_first_rx) = mpsc::channel();
    let first_server_thread = std::thread::spawn(move || {
        let (stream, _) = first_listener
            .accept()
            .expect("portable smoke first tcp server should accept one client");
        let mut reader = BufReader::new(stream);
        let mut hello_line = String::new();
        reader
            .read_line(&mut hello_line)
            .expect("portable smoke first tcp server should read one startup hello line");
        first_hello_tx
            .send(hello_line)
            .expect("portable smoke first tcp server should report its hello");
        release_first_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("portable smoke first tcp server should be released after reconnect");
    });

    let (second_hello_tx, second_hello_rx) = mpsc::channel();
    let second_server_thread = std::thread::spawn(move || {
        let (stream, _) = second_listener
            .accept()
            .expect("portable smoke second tcp server should accept one client");
        let mut reader = BufReader::new(stream);
        let mut hello_line = String::new();
        reader
            .read_line(&mut hello_line)
            .expect("portable smoke second tcp server should read one reconnect hello line");
        second_hello_tx
            .send(hello_line)
            .expect("portable smoke second tcp server should report its hello");
    });

    let mut tcp_owner = super::GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime(
            "portable-user",
            "portable-room",
            first_address.to_string(),
        )
        .expect("portable smoke tcp runtime owner should bootstrap");
    let tcp_handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let mut tcp_state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("portable-user".to_owned()),
        room: Some("portable-room".to_owned()),
        public_servers: Some(vec![("Reconnect".to_owned(), second_address.to_string())]),
        ..StoredClientSettingsMvp::default()
    });

    super::GuiQueuedRuntimeOwner::pump(&mut tcp_owner, &tcp_handle, &tcp_state);
    for action in tcp_handle.drain_actions() {
        assert!(tcp_state.apply(action));
    }
    let first_hello_line = first_hello_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("portable smoke first tcp server should receive startup hello");
    assert!(first_hello_line.contains("\"Hello\""));
    assert!(first_hello_line.contains("\"portable-user\""));

    assert!(tcp_state.apply(GuiShellAction::BeginSelectedPublicServerConnect));
    tcp_handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::ConnectPublicServer,
    ));
    super::GuiQueuedRuntimeOwner::pump(&mut tcp_owner, &tcp_handle, &tcp_state);
    let reconnect_actions = tcp_handle.drain_actions();
    assert!(
        reconnect_actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteSelectedPublicServerConnect)),
        "portable smoke reconnect segment should complete selected public-server connect"
    );
    for action in reconnect_actions {
        assert!(tcp_state.apply(action));
    }

    let second_hello_line = second_hello_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("portable smoke second tcp server should receive reconnect hello");
    assert!(second_hello_line.contains("\"Hello\""));
    assert!(second_hello_line.contains("\"portable-user\""));
    assert!(second_hello_line.contains("\"portable-room\""));

    release_first_tx
        .send(())
        .expect("portable smoke first tcp server should be releasable");
    first_server_thread
        .join()
        .expect("portable smoke first tcp server thread should complete");
    second_server_thread
        .join()
        .expect("portable smoke second tcp server thread should complete");
}

#[test]
fn gui_portable_smoke_regression_covers_nontransport_script_parity() {
    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "syncplay-gui-portable-nontransport-{}-{unique_suffix}.ini",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);

    let mut persisted_owner =
        super::GuiPersistedConfigRuntimeOwner::with_config_path(Some(path.clone()));
    let persisted_handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let mut persisted_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    let saved_settings = StoredClientSettingsMvp {
        host: Some("syncplay.example".to_owned()),
        port: Some(8999),
        username: Some("smoke-user".to_owned()),
        room: Some("smoke-room".to_owned()),
        player_path: Some("C:/Windows/System32/notepad.exe".to_owned()),
        ready_at_start: Some(true),
        autoplay_initial_state: Some(true),
        autoplay_require_same_filenames: Some(true),
        shared_playlist_enabled: Some(true),
        pause_on_leave: Some(true),
        unpause_action: Some(UnpauseActionMode::Always),
        autoplay_min_users: Some(AutoplayThresholdOverride::Set(3)),
        filename_privacy_mode: Some(PrivacyMode::SendHashed),
        filesize_privacy_mode: Some(PrivacyMode::DoNotSend),
        only_switch_to_trusted_domains: Some(true),
        trusted_domains: Some(vec![
            "youtube.com".to_owned(),
            "*.example.com/videos".to_owned(),
        ]),
        rewind_on_desync: Some(true),
        fastforward_on_desync: Some(true),
        slow_on_desync: Some(true),
        dont_slow_down_with_me: Some(true),
        rewind_threshold_seconds: Some(1.25),
        fastforward_threshold_seconds: Some(3.5),
        slowdown_threshold_seconds: Some(2.25),
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        folder_search_first_file_timeout_seconds: Some(3.0),
        folder_search_timeout_seconds: Some(30.0),
        folder_search_double_check_interval_seconds: Some(2.5),
        folder_search_warning_threshold_seconds: Some(7.5),
        chat_input_enabled: Some(true),
        chat_output_enabled: Some(true),
        chat_direct_input: Some(true),
        chat_move_osd: Some(true),
        chat_max_lines: Some(7),
        chat_input_font_family: Some("Consolas".to_owned()),
        chat_output_font_family: Some("Cascadia Mono".to_owned()),
        show_osd: Some(true),
        show_duration_notification: Some(true),
        show_same_room_osd: Some(true),
        show_osd_warnings: Some(true),
        show_noncontroller_osd: Some(true),
        show_different_room_osd: Some(true),
        show_contact_info: Some(true),
        language: Some("pt_BR".to_owned()),
        check_for_updates_automatically: Some(true),
        ..StoredClientSettingsMvp::default()
    };
    assert!(persisted_state.apply(GuiShellAction::BeginConfigurationSave));
    persisted_handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SaveConfiguration(saved_settings.clone()),
    ));
    super::GuiQueuedRuntimeOwner::pump(&mut persisted_owner, &persisted_handle, &persisted_state);
    let save_actions = persisted_handle.drain_actions();
    assert_eq!(
        save_actions,
        vec![GuiShellAction::CompleteConfigurationSave(
            saved_settings.clone()
        )]
    );
    for action in save_actions {
        assert!(persisted_state.apply(action));
    }
    assert_eq!(
        super::load_syncplay_ini_stored_client_settings_mvp_from_path(&path)
            .expect("portable nontransport smoke save should leave a readable config"),
        Some(saved_settings.clone())
    );

    let saved_contents = std::fs::read_to_string(&path)
        .expect("portable nontransport smoke save should leave ini text");
    for expected_line in [
        "host = syncplay.example",
        "port = 8999",
        "name = smoke-user",
        "room = smoke-room",
        "playerPath = C:/Windows/System32/notepad.exe",
        "sharedPlaylistEnabled = True",
    ] {
        assert!(
            saved_contents.contains(expected_line),
            "portable nontransport smoke save should persist line: {expected_line}"
        );
    }

    let reloaded_settings = StoredClientSettingsMvp {
        host: Some("syncplay.reload.example".to_owned()),
        port: Some(8998),
        username: Some("smoke-reloaded".to_owned()),
        room: Some("smoke-room-b".to_owned()),
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        ready_at_start: Some(false),
        autoplay_initial_state: Some(true),
        autoplay_require_same_filenames: Some(false),
        shared_playlist_enabled: Some(true),
        pause_on_leave: Some(false),
        unpause_action: Some(UnpauseActionMode::IfMinUsersReady),
        autoplay_min_users: Some(AutoplayThresholdOverride::Set(4)),
        filename_privacy_mode: Some(PrivacyMode::DoNotSend),
        filesize_privacy_mode: Some(PrivacyMode::SendHashed),
        only_switch_to_trusted_domains: Some(true),
        trusted_domains: Some(vec!["reload.example".to_owned()]),
        rewind_on_desync: Some(true),
        fastforward_on_desync: Some(false),
        slow_on_desync: Some(true),
        dont_slow_down_with_me: Some(false),
        rewind_threshold_seconds: Some(2.5),
        fastforward_threshold_seconds: Some(4.5),
        slowdown_threshold_seconds: Some(1.5),
        media_search_directories: Some(vec![
            "C:/ReloadMedia".to_owned(),
            "D:/ReloadArchive".to_owned(),
        ]),
        folder_search_first_file_timeout_seconds: Some(4.0),
        folder_search_timeout_seconds: Some(40.0),
        folder_search_double_check_interval_seconds: Some(3.0),
        folder_search_warning_threshold_seconds: Some(8.0),
        chat_input_enabled: Some(true),
        chat_output_enabled: Some(true),
        chat_direct_input: Some(false),
        chat_move_osd: Some(true),
        chat_max_lines: Some(9),
        chat_input_font_family: Some("Consolas".to_owned()),
        chat_output_font_family: Some("Segoe UI".to_owned()),
        show_osd: Some(true),
        show_duration_notification: Some(false),
        show_same_room_osd: Some(true),
        show_osd_warnings: Some(true),
        show_noncontroller_osd: Some(false),
        show_different_room_osd: Some(true),
        show_contact_info: Some(true),
        language: Some("es".to_owned()),
        check_for_updates_automatically: Some(true),
        ..StoredClientSettingsMvp::default()
    };
    super::upsert_syncplay_ini_stored_client_settings_mvp_at_path(&path, &reloaded_settings)
        .expect("portable nontransport smoke reload seed should write config");
    assert!(persisted_state.apply(GuiShellAction::BeginConfigurationReload));
    persisted_handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::ReloadConfiguration(StoredClientSettingsMvp::default()),
    ));
    super::GuiQueuedRuntimeOwner::pump(&mut persisted_owner, &persisted_handle, &persisted_state);
    let reload_actions = persisted_handle.drain_actions();
    assert!(
        reload_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::CompleteConfigurationReload(settings)
                if settings == &reloaded_settings
        )),
        "portable nontransport smoke reload should emit completion with reloaded settings"
    );
    for action in reload_actions {
        assert!(persisted_state.apply(action));
    }
    assert_eq!(
        persisted_state.saved_configuration, reloaded_settings,
        "portable nontransport smoke reload should project saved settings into shell state"
    );
    assert_eq!(
        persisted_state
            .configuration
            .control_value("Readiness", "Unpause Action"),
        Some("IfMinUsersReady")
    );
    assert_eq!(
        persisted_state
            .configuration
            .control_value("Readiness", "Autoplay Min Users"),
        Some("4")
    );
    assert_eq!(
        persisted_state
            .configuration
            .control_value("Privacy", "Trusted Domain Count"),
        Some("1")
    );
    assert_eq!(
        persisted_state
            .configuration
            .control_value("System", "Language"),
        Some("es")
    );
    assert_eq!(persisted_state.media_search.directories.len(), 2);
    assert_eq!(
        persisted_state.media_search.directories[0].path,
        "C:/ReloadMedia"
    );
    assert!(persisted_state.main_window.shared_playlist_enabled);
    assert!(persisted_state.menus.tls_prompt_expected);
    assert!(!persisted_state.menus.update_notice_expected);
    let window = persisted_state
        .menus
        .sections
        .iter()
        .find(|section| section.title == "Window")
        .expect("window menu should exist after reload");
    assert!(
        window
            .actions
            .iter()
            .find(|action| action.label == "Show Chat")
            .is_some_and(|action| action.enabled)
    );
    assert!(
        window
            .actions
            .iter()
            .find(|action| action.label == "Show Playlist")
            .is_some_and(|action| action.enabled)
    );

    let mut no_runtime_owner = super::GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let no_runtime_handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let mut no_runtime_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            shared_playlist_enabled: Some(true),
            public_servers: Some(vec![
                ("Alpha".to_owned(), "alpha.example:8999".to_owned()),
                ("Beta".to_owned(), "beta.example:9000".to_owned()),
            ]),
            ..StoredClientSettingsMvp::default()
        });

    assert!(no_runtime_state.apply(GuiShellAction::SelectPublicServer(0)));
    assert!(no_runtime_state.apply(GuiShellAction::BeginSelectedPublicServerConnect));
    no_runtime_handle.push_request(GuiRuntimeRequest::CancelPendingOperation(
        GuiPendingOperationKind::ConnectPublicServer,
    ));
    super::GuiQueuedRuntimeOwner::pump(
        &mut no_runtime_owner,
        &no_runtime_handle,
        &no_runtime_state,
    );
    for action in no_runtime_handle.drain_actions() {
        assert!(no_runtime_state.apply(action));
    }
    assert!(no_runtime_state.pending_operation.is_none());
    assert_eq!(
        no_runtime_state
            .public_servers
            .servers
            .iter()
            .map(|row| (row.label.as_str(), row.address.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("Alpha", "alpha.example:8999"),
            ("Beta", "beta.example:9000"),
        ]
    );
    assert_eq!(
        no_runtime_state
            .notifications
            .last()
            .map(|notification| notification.message.as_str()),
        Some("Public server connect canceled.")
    );

    assert!(no_runtime_state.apply(GuiShellAction::BeginPublicServerRefresh));
    no_runtime_handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::RefreshPublicServers(vec![
            ("Alpha".to_owned(), "alpha.example:8999".to_owned()),
            ("Beta".to_owned(), "beta.example:9000".to_owned()),
        ]),
    ));
    super::GuiQueuedRuntimeOwner::pump(
        &mut no_runtime_owner,
        &no_runtime_handle,
        &no_runtime_state,
    );
    for action in no_runtime_handle.drain_actions() {
        assert!(no_runtime_state.apply(action));
    }
    assert!(no_runtime_state.pending_operation.is_none());
    assert_eq!(
        no_runtime_state
            .notifications
            .last()
            .map(|notification| notification.message.as_str()),
        Some("Public servers refreshed: 2 entries.")
    );

    assert!(
        no_runtime_state.apply(GuiShellAction::AnnounceMediaSearchDirectoryBrowsed(
            "C:/SmokeMedia".to_owned(),
        ))
    );
    assert_eq!(no_runtime_state.media_search.directories.len(), 1);
    assert!(
        !no_runtime_state.apply(GuiShellAction::AnnounceMediaSearchDirectoryBrowsed(
            "C:/SmokeMedia".to_owned(),
        ))
    );
    assert_eq!(no_runtime_state.media_search.directories.len(), 1);

    assert!(no_runtime_state.apply(GuiShellAction::BeginMissingMediaSearch));
    no_runtime_handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SearchMissingMedia,
    ));
    super::GuiQueuedRuntimeOwner::pump(
        &mut no_runtime_owner,
        &no_runtime_handle,
        &no_runtime_state,
    );
    for action in no_runtime_handle.drain_actions() {
        assert!(no_runtime_state.apply(action));
    }
    assert!(no_runtime_state.pending_operation.is_none());
    assert_eq!(
        no_runtime_state
            .notifications
            .last()
            .map(|notification| notification.message.as_str()),
        Some("Missing media search completed: no match found.")
    );

    let preview_open_actions = super::GuiPreviewRuntimeBridge::preview_open_media_file_actions(
        vec!["C:/SmokeMedia/open-target.mkv".to_owned()],
        true,
    );
    assert_eq!(
        preview_open_actions,
        vec![
            GuiShellAction::SwitchView(GuiShellView::MainWindow),
            GuiShellAction::AnnounceSharedPlaylistLoaded(vec!["open-target.mkv".to_owned(),]),
        ],
    );
    for action in preview_open_actions {
        assert!(no_runtime_state.apply(action));
    }
    assert_eq!(no_runtime_state.active_view, GuiShellView::MainWindow);
    assert_eq!(
        no_runtime_state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["open-target.mkv"]
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn gui_portable_smoke_regression_covers_tcp_state_churn_and_reconnect() {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        time::Duration,
    };

    let first_listener = TcpListener::bind("127.0.0.1:0")
        .expect("portable tcp churn smoke first listener should bind");
    let first_address = first_listener
        .local_addr()
        .expect("portable tcp churn smoke first listener should expose an address");
    let second_listener = TcpListener::bind("127.0.0.1:0")
        .expect("portable tcp churn smoke second listener should bind");
    let second_address = second_listener
        .local_addr()
        .expect("portable tcp churn smoke second listener should expose an address");

    let (first_hello_tx, first_hello_rx) = mpsc::channel();
    let (first_chat_tx, first_chat_rx) = mpsc::channel();
    let (first_state_tx, first_state_rx) = mpsc::channel();
    let (release_first_tx, release_first_rx) = mpsc::channel();
    let first_server_thread = std::thread::spawn(move || {
        let (mut stream, _) = first_listener
            .accept()
            .expect("portable tcp churn smoke first server should accept one client");
        let reader_stream = stream
            .try_clone()
            .expect("portable tcp churn smoke first server should clone stream");
        let mut reader = BufReader::new(reader_stream);

        let mut hello_line = String::new();
        reader
            .read_line(&mut hello_line)
            .expect("portable tcp churn smoke first server should read startup hello");
        first_hello_tx
            .send(hello_line)
            .expect("portable tcp churn smoke first server should report startup hello");

        for line in [
            r#"{"Hello":{"username":"portable-user","room":{"name":"portable-room"},"version":"1.7.5","features":{"chat":true}}}"#,
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"portable-user"}}}"#,
            r#"{"Set":{"playlistIndex":{"index":1,"user":"portable-user"}}}"#,
            r#"{"Set":{"ready":{"isReady":true,"username":"portable-user"}}}"#,
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"portable-user"}}}"#,
            r#"{"Set":{"user":{"bob":{"room":{"name":"portable-room"},"file":{"name":"bob.mp4"},"isReady":true,"controller":true}}}}"#,
        ] {
            stream
                .write_all(line.as_bytes())
                .expect("portable tcp churn smoke first server should write initial line");
            stream
                .write_all(b"\n")
                .expect("portable tcp churn smoke first server should terminate initial line");
        }
        first_state_tx
            .send("initial".to_owned())
            .expect("portable tcp churn smoke first server should signal initial state");

        let mut first_chat_line = String::new();
        reader
            .read_line(&mut first_chat_line)
            .expect("portable tcp churn smoke first server should read first chat");
        first_chat_tx
            .send(first_chat_line)
            .expect("portable tcp churn smoke first server should report first chat");
        for line in [
            r#"{"Chat":{"username":"portable-user","message":"hellotcp"}}"#,
            r#"{"Set":{"playlistChange":{"files":["postchat1.mkv","postchat2.mkv"],"user":"portable-user"}}}"#,
            r#"{"Set":{"playlistIndex":{"index":1,"user":"portable-user"}}}"#,
            r#"{"Set":{"ready":{"isReady":false,"username":"portable-user"}}}"#,
            r#"{"State":{"playstate":{"position":20.0,"paused":false,"doSeek":false,"setBy":"portable-user"}}}"#,
            r#"{"Set":{"user":{"bob":{"room":{"name":"portable-room"},"file":{"name":"bob-post.mp4"},"isReady":false,"controller":false}}}}"#,
        ] {
            stream
                .write_all(line.as_bytes())
                .expect("portable tcp churn smoke first server should write post-chat line");
            stream
                .write_all(b"\n")
                .expect("portable tcp churn smoke first server should terminate post-chat line");
        }
        first_state_tx
            .send("postchat".to_owned())
            .expect("portable tcp churn smoke first server should signal post-chat state");

        let mut second_chat_line = String::new();
        reader
            .read_line(&mut second_chat_line)
            .expect("portable tcp churn smoke first server should read second chat");
        first_chat_tx
            .send(second_chat_line)
            .expect("portable tcp churn smoke first server should report second chat");
        for line in [
            r#"{"Chat":{"username":"portable-user","message":"goodbyeprimary"}}"#,
            r#"{"Set":{"user":{"bob":{"event":{"left":true}}}}}"#,
        ] {
            stream
                .write_all(line.as_bytes())
                .expect("portable tcp churn smoke first server should write user-left line");
            stream
                .write_all(b"\n")
                .expect("portable tcp churn smoke first server should terminate user-left line");
        }
        first_state_tx
            .send("user-left".to_owned())
            .expect("portable tcp churn smoke first server should signal user-left state");

        release_first_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("portable tcp churn smoke first server should be releasable");
    });

    let (second_hello_tx, second_hello_rx) = mpsc::channel();
    let (second_chat_tx, second_chat_rx) = mpsc::channel();
    let (second_state_tx, second_state_rx) = mpsc::channel();
    let second_server_thread = std::thread::spawn(move || {
        let (mut stream, _) = second_listener
            .accept()
            .expect("portable tcp churn smoke second server should accept one client");
        let reader_stream = stream
            .try_clone()
            .expect("portable tcp churn smoke second server should clone stream");
        let mut reader = BufReader::new(reader_stream);

        let mut hello_line = String::new();
        reader
            .read_line(&mut hello_line)
            .expect("portable tcp churn smoke second server should read reconnect hello");
        second_hello_tx
            .send(hello_line)
            .expect("portable tcp churn smoke second server should report reconnect hello");

        for line in [
            r#"{"Hello":{"username":"portable-user","room":{"name":"portable-room"},"version":"1.7.5","features":{"chat":true}}}"#,
            r#"{"Set":{"playlistChange":{"files":["reconnect1.mkv","reconnect2.mkv"],"user":"portable-user"}}}"#,
            r#"{"Set":{"playlistIndex":{"index":1,"user":"portable-user"}}}"#,
            r#"{"Set":{"ready":{"isReady":false,"username":"portable-user"}}}"#,
            r#"{"State":{"playstate":{"position":30.0,"paused":false,"doSeek":false,"setBy":"portable-user"}}}"#,
            r#"{"Set":{"user":{"carol":{"room":{"name":"portable-room"},"file":{"name":"carol.mp4"},"isReady":false,"controller":false}}}}"#,
        ] {
            stream
                .write_all(line.as_bytes())
                .expect("portable tcp churn smoke second server should write initial line");
            stream
                .write_all(b"\n")
                .expect("portable tcp churn smoke second server should terminate initial line");
        }
        second_state_tx
            .send("initial".to_owned())
            .expect("portable tcp churn smoke second server should signal initial state");

        let mut first_chat_line = String::new();
        reader
            .read_line(&mut first_chat_line)
            .expect("portable tcp churn smoke second server should read first chat");
        second_chat_tx
            .send(first_chat_line)
            .expect("portable tcp churn smoke second server should report first chat");
        for line in [
            r#"{"Chat":{"username":"portable-user","message":"helloreconnect"}}"#,
            r#"{"Set":{"playlistChange":{"files":["reconnect-post1.mkv","reconnect-post2.mkv"],"user":"portable-user"}}}"#,
            r#"{"Set":{"playlistIndex":{"index":1,"user":"portable-user"}}}"#,
            r#"{"Set":{"ready":{"isReady":true,"username":"portable-user"}}}"#,
            r#"{"State":{"playstate":{"position":40.0,"paused":true,"doSeek":false,"setBy":"portable-user"}}}"#,
            r#"{"Set":{"user":{"carol":{"room":{"name":"portable-room"},"file":{"name":"carol-post.mp4"},"isReady":true,"controller":true}}}}"#,
        ] {
            stream
                .write_all(line.as_bytes())
                .expect("portable tcp churn smoke second server should write post-chat line");
            stream
                .write_all(b"\n")
                .expect("portable tcp churn smoke second server should terminate post-chat line");
        }
        second_state_tx
            .send("postchat".to_owned())
            .expect("portable tcp churn smoke second server should signal post-chat state");

        let mut second_chat_line = String::new();
        reader
            .read_line(&mut second_chat_line)
            .expect("portable tcp churn smoke second server should read second chat");
        second_chat_tx
            .send(second_chat_line)
            .expect("portable tcp churn smoke second server should report second chat");
        for line in [
            r#"{"Chat":{"username":"portable-user","message":"goodbyereconnect"}}"#,
            r#"{"Set":{"user":{"carol":{"event":{"left":true}}}}}"#,
        ] {
            stream
                .write_all(line.as_bytes())
                .expect("portable tcp churn smoke second server should write user-left line");
            stream
                .write_all(b"\n")
                .expect("portable tcp churn smoke second server should terminate user-left line");
        }
        second_state_tx
            .send("user-left".to_owned())
            .expect("portable tcp churn smoke second server should signal user-left state");
    });

    let mut owner = super::GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime(
            "portable-user",
            "portable-room",
            first_address.to_string(),
        )
        .expect("portable tcp churn smoke owner should bootstrap");
    let handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("portable-user".to_owned()),
        room: Some("portable-room".to_owned()),
        chat_input_enabled: Some(true),
        public_servers: Some(vec![("Reconnect".to_owned(), second_address.to_string())]),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let first_hello = first_hello_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("portable tcp churn smoke first server should receive startup hello");
    assert!(first_hello.contains("\"Hello\""));
    assert!(first_hello.contains("\"portable-user\""));
    assert_eq!(
        first_state_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("portable tcp churn smoke first server should publish initial state"),
        "initial"
    );
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        Duration::from_secs(1),
        |state| {
            state
                .main_window
                .playlist
                .iter()
                .map(|row| row.label.as_str())
                .eq(["episode1.mkv", "episode2.mkv"])
                && state.main_window.playback_paused
                && state.selection.selected_main_window_playlist == Some(1)
                && state
                    .main_window
                    .users
                    .iter()
                    .any(|user| user.username == "bob" && user.is_ready && user.is_controller)
        },
        "portable primary initial state",
    );
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["episode1.mkv", "episode2.mkv"]
    );
    assert!(state.main_window.playback_paused);
    assert_eq!(state.selection.selected_main_window_playlist, Some(1));
    assert!(
        state
            .main_window
            .users
            .iter()
            .any(|user| user.username == "bob" && user.is_ready && user.is_controller)
    );

    assert!(state.apply(GuiShellAction::BeginLocalChatSend("hellotcp".to_owned())));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SendChatMessage("hellotcp".to_owned()),
    ));
    let first_chat_actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        first_chat_actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteLocalChatSend))
    );
    assert!(state.pending_operation.is_none());
    assert!(state.outgoing_chat_message.is_none());
    let first_chat = first_chat_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("portable tcp churn smoke first server should receive first chat");
    assert!(first_chat.contains("\"Chat\""));
    assert!(first_chat.contains("hellotcp"));
    assert_eq!(
        first_state_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("portable tcp churn smoke first server should publish post-chat state"),
        "postchat"
    );
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        Duration::from_secs(1),
        |state| {
            state
                .main_window
                .playlist
                .iter()
                .map(|row| row.label.as_str())
                .eq(["postchat1.mkv", "postchat2.mkv"])
                && !state.main_window.playback_paused
                && state
                    .main_window
                    .users
                    .iter()
                    .any(|user| user.username == "bob" && !user.is_ready && !user.is_controller)
        },
        "portable primary post-chat state",
    );
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["postchat1.mkv", "postchat2.mkv"]
    );
    assert!(!state.main_window.playback_paused);
    assert!(
        state
            .main_window
            .users
            .iter()
            .any(|user| user.username == "bob" && !user.is_ready && !user.is_controller)
    );

    assert!(state.apply(GuiShellAction::BeginLocalChatSend(
        "goodbyeprimary".to_owned(),
    )));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SendChatMessage("goodbyeprimary".to_owned()),
    ));
    let second_primary_chat_actions =
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        second_primary_chat_actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteLocalChatSend))
    );
    let second_primary_chat = first_chat_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("portable tcp churn smoke first server should receive second chat");
    assert!(second_primary_chat.contains("\"Chat\""));
    assert!(second_primary_chat.contains("goodbyeprimary"));
    assert_eq!(
        first_state_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("portable tcp churn smoke first server should publish user-left state"),
        "user-left"
    );
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        Duration::from_secs(1),
        |state| {
            state
                .main_window
                .users
                .iter()
                .all(|user| user.username != "bob")
        },
        "portable primary user-left state",
    );
    assert!(
        state
            .main_window
            .users
            .iter()
            .all(|user| user.username != "bob")
    );

    assert!(state.apply(GuiShellAction::BeginSelectedPublicServerConnect));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::ConnectPublicServer,
    ));
    let reconnect_actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        reconnect_actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteSelectedPublicServerConnect))
    );

    let second_hello = second_hello_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("portable tcp churn smoke second server should receive reconnect hello");
    assert!(second_hello.contains("\"Hello\""));
    assert!(second_hello.contains("\"portable-user\""));
    assert!(second_hello.contains("\"portable-room\""));
    assert_eq!(
        second_state_rx.recv_timeout(Duration::from_secs(1)).expect(
            "portable tcp churn smoke second server should publish initial reconnect state"
        ),
        "initial"
    );
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        Duration::from_secs(1),
        |state| {
            state
                .main_window
                .playlist
                .iter()
                .map(|row| row.label.as_str())
                .eq(["reconnect1.mkv", "reconnect2.mkv"])
                && !state.main_window.playback_paused
                && state
                    .main_window
                    .users
                    .iter()
                    .any(|user| user.username == "carol" && !user.is_ready && !user.is_controller)
        },
        "portable reconnect initial state",
    );
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["reconnect1.mkv", "reconnect2.mkv"]
    );
    assert!(!state.main_window.playback_paused);
    assert!(
        state
            .main_window
            .users
            .iter()
            .any(|user| user.username == "carol" && !user.is_ready && !user.is_controller)
    );

    assert!(state.apply(GuiShellAction::BeginLocalChatSend(
        "helloreconnect".to_owned(),
    )));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SendChatMessage("helloreconnect".to_owned()),
    ));
    let first_reconnect_chat_actions =
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        first_reconnect_chat_actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteLocalChatSend))
    );
    let first_reconnect_chat = second_chat_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("portable tcp churn smoke second server should receive first reconnect chat");
    assert!(first_reconnect_chat.contains("\"Chat\""));
    assert!(first_reconnect_chat.contains("helloreconnect"));
    assert_eq!(
        second_state_rx.recv_timeout(Duration::from_secs(1)).expect(
            "portable tcp churn smoke second server should publish reconnect post-chat state"
        ),
        "postchat"
    );
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        Duration::from_secs(1),
        |state| {
            state
                .main_window
                .playlist
                .iter()
                .map(|row| row.label.as_str())
                .eq(["reconnect-post1.mkv", "reconnect-post2.mkv"])
                && state.main_window.playback_paused
                && state
                    .main_window
                    .users
                    .iter()
                    .any(|user| user.username == "carol" && user.is_ready && user.is_controller)
        },
        "portable reconnect post-chat state",
    );
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["reconnect-post1.mkv", "reconnect-post2.mkv"]
    );
    assert!(state.main_window.playback_paused);
    assert!(
        state
            .main_window
            .users
            .iter()
            .any(|user| user.username == "carol" && user.is_ready && user.is_controller)
    );

    assert!(state.apply(GuiShellAction::BeginLocalChatSend(
        "goodbyereconnect".to_owned(),
    )));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SendChatMessage("goodbyereconnect".to_owned()),
    ));
    let second_reconnect_chat_actions =
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        second_reconnect_chat_actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteLocalChatSend))
    );
    let second_reconnect_chat = second_chat_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("portable tcp churn smoke second server should receive second reconnect chat");
    assert!(second_reconnect_chat.contains("\"Chat\""));
    assert!(second_reconnect_chat.contains("goodbyereconnect"));
    assert_eq!(
        second_state_rx.recv_timeout(Duration::from_secs(1)).expect(
            "portable tcp churn smoke second server should publish reconnect user-left state"
        ),
        "user-left"
    );
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        Duration::from_secs(1),
        |state| {
            state
                .main_window
                .users
                .iter()
                .all(|user| user.username != "carol")
        },
        "portable reconnect user-left state",
    );
    assert!(
        state
            .main_window
            .users
            .iter()
            .all(|user| user.username != "carol")
    );

    release_first_tx
        .send(())
        .expect("portable tcp churn smoke first server should be releasable");
    first_server_thread
        .join()
        .expect("portable tcp churn smoke first server thread should complete");
    second_server_thread
        .join()
        .expect("portable tcp churn smoke second server thread should complete");
}

#[test]
fn gui_persisted_config_runtime_owner_uses_attached_session_runtime_for_session_requests() {
    #[derive(Debug, Default)]
    struct RecordingSessionState {
        queued_gui_actions: Vec<GuiShellAction>,
        room_requests: Vec<String>,
        local_ready_requests: Vec<bool>,
        user_ready_requests: Vec<(String, bool)>,
        controller_auth_requests: Vec<(String, String)>,
        sent_chat_messages: Vec<String>,
        connect_requests: Vec<Option<(String, String)>>,
        refresh_requests: Vec<Vec<(String, String)>>,
        search_requests: Vec<Vec<String>>,
    }

    struct RecordingSessionRuntimeAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingSessionState>>,
    }

    impl super::GuiSessionRuntimeAdapter for RecordingSessionRuntimeAdapter {
        fn drain_gui_actions(&mut self, _state: &SyncplayGuiShellAppState) -> Vec<GuiShellAction> {
            std::mem::take(
                &mut self
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .queued_gui_actions,
            )
        }

        fn set_room(&mut self, room: String) -> Result<(), String> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .room_requests
                .push(room);
            Ok(())
        }

        fn set_local_ready(&mut self, ready: bool) -> Result<(), String> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .local_ready_requests
                .push(ready);
            Ok(())
        }

        fn set_user_ready(&mut self, username: String, ready: bool) -> Result<(), String> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .user_ready_requests
                .push((username, ready));
            Ok(())
        }

        fn request_controller_auth(
            &mut self,
            room: String,
            password: String,
        ) -> Result<(), String> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .controller_auth_requests
                .push((room, password));
            Ok(())
        }

        fn send_chat_message(&mut self, message: String) -> Result<(), String> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .sent_chat_messages
                .push(message);
            Ok(())
        }

        fn connect_public_server(
            &mut self,
            selected_server: Option<(String, String)>,
        ) -> Result<(), String> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .connect_requests
                .push(selected_server);
            Ok(())
        }

        fn refresh_public_servers(
            &mut self,
            current_servers: Vec<(String, String)>,
        ) -> Result<Vec<(String, String)>, String> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .refresh_requests
                .push(current_servers);
            Ok(vec![(
                "Runtime".to_owned(),
                "runtime.example:9000".to_owned(),
            )])
        }

        fn search_missing_media(
            &mut self,
            directories: Vec<String>,
        ) -> Result<Option<String>, String> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .search_requests
                .push(directories);
            Ok(Some("C:/Media/found.mkv".to_owned()))
        }
    }

    let session_state =
        std::sync::Arc::new(std::sync::Mutex::new(RecordingSessionState::default()));
    let mut owner = super::GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_session_runtime(Box::new(RecordingSessionRuntimeAdapter {
            state: session_state.clone(),
        }));
    let handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        public_servers: Some(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())]),
        media_search_directories: Some(vec!["C:/Media".to_owned(), "D:/Archive".to_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    let mut inbound_snapshot = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
    inbound_snapshot.chat.push(MainWindowRuntimeChatSnapshot {
        sender: "Server".to_owned(),
        message: "Welcome.".to_owned(),
    });

    session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .queued_gui_actions = vec![
        GuiShellAction::PushChatMessage {
            sender: "Server".to_owned(),
            message: "Welcome.".to_owned(),
        },
        GuiShellAction::ApplyGuiRuntimeSnapshot(SyncplayGuiRuntimeSnapshot {
            active_view: GuiShellView::PublicServers,
            open_modal: None,
            main_window: inbound_snapshot,
            public_servers: state.public_servers.clone(),
            media_search: state.media_search.clone(),
            tls_prompt_expected: state.menus.tls_prompt_expected,
            update_notice_expected: state.menus.update_notice_expected,
            about_dialog_available: state.menus.about_dialog_available,
        }),
    ];
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let inbound_actions = handle.drain_actions();
    assert_eq!(inbound_actions.len(), 3);
    assert!(matches!(
        &inbound_actions[0],
        GuiShellAction::PushChatMessage { sender, message }
            if sender == "Server" && message == "Welcome."
    ));
    assert!(matches!(
        &inbound_actions[1],
        GuiShellAction::ApplyGuiRuntimeSnapshot(snapshot)
            if snapshot.active_view == GuiShellView::PublicServers
    ));
    assert_eq!(
        inbound_actions[2],
        GuiShellAction::ApplyGuiCommandRuntimeSnapshot(GuiCommandRuntimeSnapshot {
            command_availability: GuiCommandAvailabilityState {
                can_save_configuration: true,
                can_reset_configuration: false,
                can_reload_configuration: true,
                can_connect_public_server: true,
                can_connect_saved_server: false,
                can_refresh_public_servers: true,
                can_disconnect_session: true,
                can_search_missing_media: true,
                can_toggle_pause: false,
                can_send_chat_message: true,
            },
            pending_operation: None,
        })
    );
    for action in inbound_actions {
        assert!(state.apply(action));
    }
    assert_eq!(state.active_view, GuiShellView::PublicServers);
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("Welcome.")
    );

    handle.push_request(GuiRuntimeRequest::SetRoom("runtime-room".to_owned()));
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert!(handle.drain_actions().is_empty());

    assert!(state.apply(GuiShellAction::BeginLocalChatSend("hello".to_owned())));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SendChatMessage("hello".to_owned()),
    ));
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let chat_actions = handle.drain_actions();
    assert_eq!(chat_actions, vec![GuiShellAction::CompleteLocalChatSend]);
    for action in chat_actions {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());
    assert_eq!(
        state
            .main_window
            .chat
            .last()
            .map(|row| row.message.as_str()),
        Some("hello")
    );

    assert!(state.apply(GuiShellAction::BeginSelectedPublicServerConnect));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::ConnectPublicServer,
    ));
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let connect_actions = handle.drain_actions();
    assert_eq!(
        connect_actions,
        vec![GuiShellAction::CompleteSelectedPublicServerConnect]
    );
    for action in connect_actions {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());

    assert!(state.apply(GuiShellAction::BeginPublicServerRefresh));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::RefreshPublicServers(vec![(
            "Ignored".to_owned(),
            "ignored.example:8999".to_owned(),
        )]),
    ));
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let refresh_actions = handle.drain_actions();
    assert_eq!(
        refresh_actions,
        vec![GuiShellAction::CompletePublicServerRefresh(vec![(
            "Runtime".to_owned(),
            "runtime.example:9000".to_owned(),
        )])]
    );
    for action in refresh_actions {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());
    assert_eq!(
        state
            .public_servers
            .servers
            .iter()
            .map(|row| (row.label.as_str(), row.address.as_str()))
            .collect::<Vec<_>>(),
        vec![("Runtime", "runtime.example:9000")]
    );

    assert!(state.apply(GuiShellAction::BeginMissingMediaSearch));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SearchMissingMedia,
    ));
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let search_actions = handle.drain_actions();
    assert_eq!(
        search_actions,
        vec![GuiShellAction::CompleteMissingMediaSearch(Some(
            "C:/Media/found.mkv".to_owned(),
        ))]
    );
    for action in search_actions {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());
    assert_eq!(
        state.notifications.last().map(|item| item.message.as_str()),
        Some("Missing media found: C:/Media/found.mkv.")
    );

    handle.push_request(GuiRuntimeRequest::SetLocalReady(true));
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert!(handle.drain_actions().is_empty());

    handle.push_request(GuiRuntimeRequest::SetReadyForUser {
        username: "bob".to_owned(),
        ready: true,
    });
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert!(handle.drain_actions().is_empty());

    handle.push_request(GuiRuntimeRequest::RequestControllerAuth {
        room: "+room:ABCDEF123456".to_owned(),
        password: "ab-123-456".to_owned(),
    });
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert!(handle.drain_actions().is_empty());

    let session_state = session_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(session_state.room_requests, vec!["runtime-room".to_owned()]);
    assert_eq!(session_state.local_ready_requests, vec![true]);
    assert_eq!(
        session_state.user_ready_requests,
        vec![("bob".to_owned(), true)]
    );
    assert_eq!(
        session_state.controller_auth_requests,
        vec![("+room:ABCDEF123456".to_owned(), "ab-123-456".to_owned())]
    );
    assert_eq!(session_state.sent_chat_messages, vec!["hello".to_owned()]);
    assert_eq!(
        session_state.connect_requests,
        vec![Some(("Primary".to_owned(), "syncplay.pl:8999".to_owned()))]
    );
    assert_eq!(
        session_state.refresh_requests,
        vec![vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())]]
    );
    assert_eq!(
        session_state.search_requests,
        vec![vec!["C:/Media".to_owned(), "D:/Archive".to_owned()]]
    );
}

#[test]
fn gui_persisted_config_runtime_owner_syncs_attached_player_runtime_state() {
    #[derive(Debug, Default)]
    struct TelemetryPlayerState {
        local_file_updates: Vec<syncplay_player_api::LocalFileUpdate>,
        playback_updates: Vec<syncplay_player_api::PlayerPlaybackTelemetryUpdate>,
    }

    struct TelemetryPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<TelemetryPlayerState>>,
    }

    impl super::PlayerAdapter for TelemetryPlayerAdapter {
        fn name(&self) -> &'static str {
            "telemetry"
        }

        fn take_playback_telemetry_update(
            &mut self,
        ) -> Option<syncplay_player_api::PlayerPlaybackTelemetryUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .playback_updates
                .pop()
        }

        fn take_local_file_update(&mut self) -> Option<syncplay_player_api::LocalFileUpdate> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .local_file_updates
                .pop()
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(TelemetryPlayerState::default()));
    let mut owner = super::GuiPersistedConfigRuntimeOwner {
        config_path: None,
        session: None,
        session_projects_to_shell: false,
        session_transport: None,
        session_transport_driver: None,
        session_default_room: None,
        pending_room_change_request: None,
        startup_saved_connect_attempted: false,
        player: Some(super::GuiOwnedPlayer::Custom(Box::new(
            TelemetryPlayerAdapter {
                state: player_state.clone(),
            },
        ))),
        player_launch_state: super::GuiPlayerLaunchRuntimeState::None,
        managed_mpv_process: None,
        player_unavailability_reason: None,
        player_local_file: None,
        player_position_seconds: None,
        player_paused: None,
        user_offset_seconds: 0.0,
    };
    let handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let bootstrap_actions = handle.drain_actions();
    assert_eq!(
        bootstrap_actions,
        vec![
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(MainWindowRuntimeSnapshot {
                room_name: "(no room joined)".to_owned(),
                shared_playlist_enabled: false,
                controlled_room_active: false,
                users: vec![browser_runtime_user(
                    "You",
                    "(no room joined)",
                    true,
                    false,
                    false,
                )],
                playlist: Vec::new(),
                chat: Vec::new(),
                can_toggle_pause: true,
                can_seek: true,
                can_set_offset: true,
                can_set_ready: true,
                can_manage_playlist: false,
                playback_paused: false,
                autoplay_active: false,
                hide_empty_rooms: false,
                rooms: browser_runtime_rooms("(no room joined)", false, true),
                ..Default::default()
            }),
            GuiShellAction::ApplyMenuDialogRuntimeSnapshot(MenuDialogRuntimeSnapshot {
                action_overrides: vec![
                    MenuActionRuntimeOverride {
                        section_title: "Playback",
                        action_label: "Play",
                        enabled: true,
                    },
                    MenuActionRuntimeOverride {
                        section_title: "Playback",
                        action_label: "Pause",
                        enabled: true,
                    },
                    MenuActionRuntimeOverride {
                        section_title: "Playback",
                        action_label: "Toggle Pause",
                        enabled: true,
                    },
                    MenuActionRuntimeOverride {
                        section_title: "Playback",
                        action_label: "Seek",
                        enabled: true,
                    },
                    MenuActionRuntimeOverride {
                        section_title: "Advanced",
                        action_label: "Set Offset",
                        enabled: true,
                    },
                ],
                tls_prompt_expected: false,
                update_notice_expected: false,
                about_dialog_available: true,
            }),
            GuiShellAction::ApplyGuiCommandRuntimeSnapshot(GuiCommandRuntimeSnapshot {
                command_availability: GuiCommandAvailabilityState {
                    can_save_configuration: true,
                    can_reset_configuration: false,
                    can_reload_configuration: true,
                    can_connect_public_server: false,
                    can_connect_saved_server: false,
                    can_refresh_public_servers: true,
                    can_disconnect_session: false,
                    can_search_missing_media: false,
                    can_toggle_pause: true,
                    can_send_chat_message: false,
                },
                pending_operation: None,
            }),
        ]
    );
    for action in bootstrap_actions {
        assert!(state.apply(action));
    }
    assert!(state.main_window.playback.can_toggle_pause);
    assert!(state.main_window.playback.can_seek);
    assert!(state.commands.can_toggle_pause);

    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        section: "Chat",
        label: "Chat Input",
        value: true,
    }));
    assert!(
        state.commands.can_send_chat_message,
        "config-driven chat availability should update immediately when no runtime field override is active"
    );
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let refreshed_command_actions = handle.drain_actions();
    assert!(refreshed_command_actions.is_empty());
    for action in refreshed_command_actions {
        assert!(state.apply(action));
    }
    assert!(state.commands.can_send_chat_message);
    assert!(state.commands.can_reset_configuration);

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .local_file_updates
        .push(
            syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
                .with_duration_seconds(93.5)
                .with_size_bytes(734003200),
        );
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let local_file_actions = handle.drain_actions();
    assert_eq!(
        local_file_actions,
        vec![GuiShellAction::ApplyMainWindowRuntimeSnapshot(
            MainWindowRuntimeSnapshot {
                room_name: "(no room joined)".to_owned(),
                shared_playlist_enabled: false,
                controlled_room_active: false,
                users: vec![browser_runtime_user(
                    "You",
                    "(no room joined)",
                    true,
                    false,
                    false,
                )],
                playlist: vec!["episode1.mkv [93.500s, 734003200 bytes]".to_owned(),],
                chat: Vec::new(),
                can_toggle_pause: true,
                can_seek: true,
                can_set_offset: true,
                can_set_ready: true,
                can_manage_playlist: false,
                playback_paused: false,
                autoplay_active: false,
                hide_empty_rooms: false,
                rooms: browser_runtime_rooms("(no room joined)", false, true),
                ..Default::default()
            },
        )]
    );
    for action in local_file_actions {
        assert!(state.apply(action));
    }
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["episode1.mkv [93.500s, 734003200 bytes]"]
    );

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .playback_updates
        .push(syncplay_player_api::PlayerPlaybackTelemetryUpdate::default().with_paused(true));
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert_eq!(
        handle.drain_actions(),
        vec![GuiShellAction::ApplyMainWindowRuntimeSnapshot(
            MainWindowRuntimeSnapshot {
                room_name: "(no room joined)".to_owned(),
                shared_playlist_enabled: false,
                controlled_room_active: false,
                users: vec![browser_runtime_user(
                    "You",
                    "(no room joined)",
                    true,
                    false,
                    false,
                )],
                playlist: vec!["episode1.mkv [93.500s, 734003200 bytes]".to_owned(),],
                chat: Vec::new(),
                can_toggle_pause: true,
                can_seek: true,
                can_set_offset: true,
                can_set_ready: true,
                can_manage_playlist: false,
                playback_paused: true,
                autoplay_active: false,
                hide_empty_rooms: false,
                rooms: browser_runtime_rooms("(no room joined)", false, true),
                ..Default::default()
            },
        )]
    );
}

#[test]
fn gui_persisted_config_runtime_owner_uses_attached_player_for_media_open_and_seek() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        opened_paths: Vec<String>,
        set_paused_values: Vec<bool>,
        set_positions: Vec<f64>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl super::PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn open_file(&mut self, path: &str) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .opened_paths
                .push(path.to_owned());
            Ok(())
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let mut owner = super::GuiPersistedConfigRuntimeOwner {
        config_path: None,
        session: None,
        session_projects_to_shell: false,
        session_transport: None,
        session_transport_driver: None,
        session_default_room: None,
        pending_room_change_request: None,
        startup_saved_connect_attempted: false,
        player: Some(super::GuiOwnedPlayer::Custom(Box::new(
            RecordingPlayerAdapter {
                state: player_state.clone(),
            },
        ))),
        player_launch_state: super::GuiPlayerLaunchRuntimeState::None,
        managed_mpv_process: None,
        player_unavailability_reason: None,
        player_local_file: None,
        player_position_seconds: None,
        player_paused: None,
        user_offset_seconds: 0.0,
    };
    let handle = super::GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("mpv".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![
            "C:/Media/episode1.mkv".to_owned(),
            "C:/Media/episode2.mkv".to_owned(),
        ],
        load_into_shared_playlist: false,
    });
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let open_actions = handle.drain_actions();
    assert_eq!(
            open_actions,
            vec![
                GuiShellAction::SwitchView(GuiShellView::MainWindow),
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Success,
                    message: "Opened the first selected media file through the attached recording player: C:/Media/episode1.mkv. Ignored 1 additional selections."
                        .to_owned(),
                },
                GuiShellAction::AnnounceSystemChatEvent(
                    "Opened the first selected media file through the attached recording player: C:/Media/episode1.mkv. Ignored 1 additional selections."
                        .to_owned(),
                ),
                GuiShellAction::ApplyMainWindowRuntimeSnapshot(MainWindowRuntimeSnapshot {
                    room_name: "(no room joined)".to_owned(),
                    shared_playlist_enabled: false,
                    controlled_room_active: false,
                    users: vec![browser_runtime_user(
                        "You",
                        "(no room joined)",
                        true,
                        false,
                        false,
                    )],
                    playlist: vec!["episode1.mkv".to_owned()],
                    chat: Vec::new(),
                    can_toggle_pause: true,
                    can_seek: true,
                    can_set_offset: true,
                    can_set_ready: true,
                    can_manage_playlist: false,
                    playback_paused: false,
                    autoplay_active: false,
                    hide_empty_rooms: false,
                    rooms: browser_runtime_rooms("(no room joined)", false, true),
                    ..Default::default()
                }),
                GuiShellAction::ApplyMenuDialogRuntimeSnapshot(MenuDialogRuntimeSnapshot {
                    action_overrides: vec![
                        MenuActionRuntimeOverride {
                            section_title: "Playback",
                            action_label: "Play",
                            enabled: true,
                        },
                        MenuActionRuntimeOverride {
                            section_title: "Playback",
                            action_label: "Pause",
                            enabled: true,
                        },
                        MenuActionRuntimeOverride {
                            section_title: "Playback",
                            action_label: "Toggle Pause",
                            enabled: true,
                        },
                        MenuActionRuntimeOverride {
                            section_title: "Playback",
                            action_label: "Seek",
                            enabled: true,
                        },
                        MenuActionRuntimeOverride {
                            section_title: "Advanced",
                            action_label: "Set Offset",
                            enabled: true,
                        },
                    ],
                    tls_prompt_expected: state.menus.tls_prompt_expected,
                    update_notice_expected: state.menus.update_notice_expected,
                    about_dialog_available: state.menus.about_dialog_available,
                }),
                GuiShellAction::ApplyGuiCommandRuntimeSnapshot(GuiCommandRuntimeSnapshot {
                    command_availability: GuiCommandAvailabilityState {
                        can_save_configuration: true,
                        can_reset_configuration: false,
                        can_reload_configuration: true,
                        can_connect_saved_server: false,
                        can_disconnect_session: false,
                        can_connect_public_server: false,
                        can_refresh_public_servers: true,
                        can_search_missing_media: false,
                        can_toggle_pause: true,
                        can_send_chat_message: false,
                    },
                    pending_operation: None,
                }),
            ]
        );
    for action in open_actions {
        assert!(state.apply(action));
    }
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["episode1.mkv"]
    );

    assert!(state.apply(GuiShellAction::BeginPlaybackPauseToggle));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::TogglePlaybackPause,
    ));
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let toggle_actions = handle.drain_actions();
    assert_eq!(
        toggle_actions,
        vec![GuiShellAction::CompletePlaybackPauseToggle]
    );
    for action in toggle_actions {
        assert!(state.apply(action));
    }
    assert!(state.main_window.playback_paused);
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths,
        vec!["C:/Media/episode1.mkv".to_owned()]
    );
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values,
        vec![true]
    );

    handle.push_request(GuiRuntimeRequest::SeekOffset(12.5));
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert_eq!(
            handle.drain_actions(),
            vec![
                GuiShellAction::SwitchView(GuiShellView::MainWindow),
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Success,
                    message: "Applied a 12.5 second seek via the attached recording player (target 12.500 seconds)."
                        .to_owned(),
                },
                GuiShellAction::AnnounceSystemChatEvent(
                    "Applied a 12.5 second seek via the attached recording player (target 12.500 seconds)."
                        .to_owned(),
                ),
            ]
        );

    handle.push_request(GuiRuntimeRequest::SeekOffset(-2.5));
    super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    assert_eq!(
            handle.drain_actions(),
            vec![
                GuiShellAction::SwitchView(GuiShellView::MainWindow),
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Success,
                    message: "Applied a -2.5 second seek via the attached recording player (target 10.000 seconds)."
                        .to_owned(),
                },
                GuiShellAction::AnnounceSystemChatEvent(
                    "Applied a -2.5 second seek via the attached recording player (target 10.000 seconds)."
                        .to_owned(),
                ),
            ]
        );
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_positions,
        vec![12.5, 10.0]
    );
}

#[test]
fn gui_native_app_and_preview_runtime_map_seek_prompt_input_to_runtime_actions() {
    assert_eq!(
        GuiNativeApp::parse_seek_offset_seconds(" 12.5 "),
        Some(12.5)
    );
    assert_eq!(GuiNativeApp::parse_seek_offset_seconds("NaN"), None);
    assert_eq!(GuiNativeApp::parse_seek_offset_seconds(""), None);

    let mut runtime = GuiPreviewRuntimeBridge;
    assert_eq!(
        runtime.actions_for_seek_offset(12.5),
        vec![
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "Seek requested: 12.5 seconds.".to_owned(),
            },
            GuiShellAction::AnnounceSystemChatEvent("Seek requested: 12.5 seconds.".to_owned(),),
        ]
    );
}

#[test]
fn gui_widget_text_preview_renderer_formats_widget_nodes() {
    let mut renderer = GuiWidgetTextPreviewRenderer::default();
    let widget = GuiWidgetNode::leaf(
        "widget:test",
        "Test Widget",
        GuiWidgetKind::Button,
        Some("click".to_owned()),
        false,
        true,
    );

    widget.render_with(&mut renderer);

    assert_eq!(
        renderer.finish(),
        "- Test Widget [button] id=widget:test, enabled=no, selected=yes, value=click"
    );
}
