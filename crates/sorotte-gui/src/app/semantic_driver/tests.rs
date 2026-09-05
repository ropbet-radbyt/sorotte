use sorotte_client_app::app_boundary::state::AutoplayThresholdOverride;
use sorotte_client_core::{PrivacyMode, UnpauseActionMode};

use crate::app::SettingId;
use crate::app::semantic_smoke::gui_semantic_scenario_named;

#[test]
fn gui_semantic_driver_runs_widget_id_scenario_without_platform_ui() {
    let scenario = gui_semantic_scenario_named("configuration-surface-flow")
        .expect("configuration semantic scenario should exist");
    let driver = scenario
        .run()
        .unwrap_or_else(|error| panic!("{} should execute successfully: {error}", scenario.name()));

    let stored = driver.state().configuration.to_stored_settings();
    let saved = &driver.state().saved_configuration;
    assert!(
        driver
            .widget(SettingId::ConnectionHost.automation_id())
            .is_ok(),
        "typed setting automation ID should resolve"
    );
    assert!(
        driver.widget("config:Connection:Host").is_err(),
        "semantic driver must not retain visible-label setting IDs"
    );
    assert_eq!(stored.host.as_deref(), Some("syncplay.example"));
    assert_eq!(stored.port, Some(8999));
    assert_eq!(stored.username.as_deref(), Some("smoke-user-after-clear"));
    assert_eq!(stored.room.as_deref(), Some("smoke-room"));
    assert_eq!(
        stored.media_search_directories,
        Some(vec!["C:/Media".to_owned(), "D:/Archive".to_owned()])
    );
    assert_eq!(saved.host.as_deref(), Some("syncplay.example"));
    assert_eq!(saved.port, Some(8999));
    assert_eq!(saved.username.as_deref(), Some("smoke-user-after-clear"));
    assert!(stored.server_password.is_none());
    assert!(saved.server_password.is_none());
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
    assert_eq!(saved.streaming_quality_preset.as_deref(), Some("720p"));
    assert_eq!(saved.streaming_buffer_target_seconds, Some(8.0));
    assert_eq!(saved.streaming_read_ahead_seconds, Some(45.0));
    assert_eq!(saved.streaming_memory_cache_mebibytes, Some(256));
    assert_eq!(saved.streaming_disk_cache_enabled, Some(true));
    assert_eq!(saved.streaming_recovery_policy.as_deref(), Some("balanced"));
    assert_eq!(saved.streaming_max_catchup_rate, Some(1.06));
    assert_eq!(saved.streaming_hard_seek_threshold_seconds, Some(9.0));
    assert_eq!(saved.streaming_max_hard_seeks_per_episode, Some(1));
    assert_eq!(
        saved.streaming_room_buffering_policy.as_deref(),
        Some("quorum")
    );
    assert_eq!(saved.streaming_start_policy.as_deref(), Some("wait-all"));
    assert_eq!(
        saved.streaming_start_timeout_action.as_deref(),
        Some("remain-paused")
    );
    assert_eq!(
        saved.media_search_directories,
        Some(vec!["C:/Media".to_owned(), "D:/Archive".to_owned()])
    );
    assert_eq!(saved.folder_search_first_file_timeout_seconds, Some(3.0));
    assert_eq!(saved.folder_search_timeout_seconds, Some(30.0));
    assert_eq!(saved.folder_search_double_check_interval_seconds, Some(2.5));
    assert_eq!(saved.folder_search_warning_threshold_seconds, Some(7.5));
    assert_eq!(saved.loop_at_end_of_playlist, Some(true));
    assert_eq!(saved.loop_single_files, Some(true));
    assert_eq!(saved.chat_input_enabled, Some(true));
    assert_eq!(saved.chat_output_enabled, Some(true));
    assert_eq!(saved.chat_direct_input, Some(true));
    assert_eq!(saved.chat_move_osd, Some(true));
    assert_eq!(saved.chat_max_lines, Some(7));
    assert_eq!(saved.chat_input_font_family.as_deref(), Some("Consolas"));
    assert_eq!(saved.chat_input_position.as_deref(), Some("Bottom"));
    assert_eq!(saved.chat_input_relative_font_size, Some(24));
    assert_eq!(saved.chat_input_font_weight, Some(50));
    assert_eq!(saved.chat_input_font_color.as_deref(), Some("#abcdef"));
    assert_eq!(
        saved.chat_output_font_family.as_deref(),
        Some("Cascadia Mono")
    );
    assert_eq!(saved.chat_output_mode.as_deref(), Some("Scrolling"));
    assert_eq!(saved.chat_output_relative_font_size, Some(20));
    assert_eq!(saved.chat_output_font_weight, Some(60));
    assert_eq!(saved.chat_top_margin, Some(25));
    assert_eq!(saved.chat_left_margin, Some(20));
    assert_eq!(saved.chat_bottom_margin, Some(30));
    assert_eq!(saved.chat_osd_margin, Some(110));
    assert_eq!(saved.show_osd, Some(true));
    assert_eq!(saved.show_duration_notification, Some(true));
    assert_eq!(saved.show_same_room_osd, Some(true));
    assert_eq!(saved.show_osd_warnings, Some(true));
    assert_eq!(saved.show_slowdown_osd, Some(true));
    assert_eq!(saved.show_noncontroller_osd, Some(true));
    assert_eq!(saved.show_different_room_osd, Some(true));
    assert_eq!(saved.show_contact_info, Some(true));
    assert_eq!(saved.notification_timeout_seconds, Some(3));
    assert_eq!(saved.alert_timeout_seconds, Some(5));
    assert_eq!(saved.chat_timeout_seconds, Some(7));
    assert_eq!(saved.language.as_deref(), Some("pt_BR"));
    assert_eq!(saved.check_for_updates_automatically, Some(true));
    assert_eq!(saved.autosave_joins_to_list, Some(true));
    assert_eq!(saved.force_gui_prompt, Some(true));
    assert!(driver.state().menus.tls_prompt_expected);
    assert!(!driver.state().menus.update_notice_expected);
    assert_eq!(stored.public_servers, Some(Vec::new()));
    assert_eq!(driver.state().selected_public_server_index(), None);
    assert_eq!(
        driver.state().selection.selected_media_search_directory,
        Some(0)
    );
}

#[test]
fn gui_semantic_driver_runs_runtime_snapshot_chat_scenario_without_platform_ui() {
    let scenario = gui_semantic_scenario_named("runtime-chat-flow")
        .expect("runtime chat semantic scenario should exist");
    let driver = scenario
        .run()
        .unwrap_or_else(|error| panic!("{} should execute successfully: {error}", scenario.name()));

    assert_eq!(driver.state().main_window.room_name, "sync-room");
    assert_eq!(
        driver.state().selection.selected_main_window_playlist,
        Some(1)
    );
    let chat_rows = &driver.state().main_window.chat;
    assert_eq!(chat_rows[2].sender, "smoke-user");
    assert_eq!(chat_rows[2].message, "hello room");
    assert_eq!(chat_rows[3].sender, "system");
    assert_eq!(chat_rows[3].message, "/undo");
    assert_eq!(chat_rows[4].sender, "system");
    assert_eq!(chat_rows[4].message, "Undo seek requested.");
    let last_chat = chat_rows
        .last()
        .expect("literal slash chat completion should append a row");
    assert_eq!(last_chat.sender, "smoke-user");
    assert_eq!(last_chat.message, "/literal");
}

#[test]
fn gui_semantic_driver_runs_core_shell_smoke_scenario_without_platform_ui() {
    let scenario = gui_semantic_scenario_named("core-shell-smoke-flow")
        .expect("core shell smoke semantic scenario should exist");
    let driver = scenario
        .run()
        .unwrap_or_else(|error| panic!("{} should execute successfully: {error}", scenario.name()));

    let stored = driver.state().configuration.to_stored_settings();
    assert_eq!(stored.host.as_deref(), Some("custom.example"));
    assert_eq!(stored.port, Some(9001));
    assert_eq!(stored.public_servers.as_ref().map(Vec::len), Some(3));
    assert_eq!(driver.active_view_label(), "room");
    assert_eq!(driver.active_modal_label(), "none");
    assert_eq!(driver.pending_operation_label(), "none");
}

#[test]
fn gui_semantic_driver_runs_playlist_workflow_scenario_without_platform_ui() {
    let scenario = gui_semantic_scenario_named("playlist-workflow-flow")
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
        vec!["episode1.mkv", "https://example.com/live"]
    );
    assert_eq!(
        driver.state().selection.selected_main_window_playlist,
        Some(1)
    );
    assert!(driver.state().playlist_text_edit_session.is_none());
    assert!(driver.state().playlist_url_edit_session.is_none());
    assert!(driver.state().media_url_edit_session.is_none());
}

#[test]
fn gui_semantic_driver_runs_player_setup_scenario_without_platform_ui() {
    let scenario = gui_semantic_scenario_named("player-setup-flow")
        .expect("player setup semantic scenario should exist");
    let driver = scenario
        .run()
        .unwrap_or_else(|error| panic!("{} should execute successfully: {error}", scenario.name()));

    assert_eq!(driver.active_view_label(), "setup");
    assert_eq!(driver.active_modal_label(), "player-setup");
    assert_eq!(
        driver
            .state()
            .player_setup_issue
            .as_ref()
            .map(|issue| issue.kind.label()),
        Some("bridge-degraded")
    );
    assert!(
        driver
            .widget("config-player-setup:retry")
            .expect("player setup retry button should exist")
            .enabled
    );
    assert!(
        driver.state().notifications.iter().any(|notification| {
            notification.message == "Retrying mpv launch with the current player settings."
        }),
        "retry button should route through runtime preview dispatch"
    );
    assert!(
        driver.state().notifications.iter().any(|notification| {
            notification.message == "Retrying mpv Chat/OSD integration in place."
        }),
        "degraded retry button should route through the distinct integration request"
    );
}
