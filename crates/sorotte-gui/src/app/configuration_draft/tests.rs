use std::collections::BTreeMap;

use super::FirstRunConfigurationDialogDraft;

use sorotte_client_app::app_boundary::state::{AutoplayThresholdOverride, StoredClientSettingsMvp};
use sorotte_client_core::UnpauseActionMode;

#[test]
fn configuration_draft_applies_edits_and_round_trips_to_stored_settings() {
    let mut draft =
        FirstRunConfigurationDialogDraft::from_stored_settings(&StoredClientSettingsMvp::default());

    assert!(draft.apply_text_value("Connection", "Host", "syncplay.example"));
    assert!(draft.apply_text_value("Connection", "Port", "8995"));
    assert!(draft.apply_text_value("Connection", "Server Password", "secret"));
    assert!(draft.apply_text_value("Connection", "Player Path", "C:/Program Files/mpv/mpv.exe"));
    assert!(draft.apply_text_value(
        "Connection",
        "Player Arguments",
        "--profile=fast --no-border"
    ));
    assert!(draft.apply_text_value("Connection", "Room History", "main-room\nbackup-room"));
    assert!(draft.apply_bool_value("Readiness", "Autoplay", true));
    assert!(draft.apply_bool_value("Readiness", "Loop At End Of Playlist", true));
    assert!(draft.apply_bool_value("Readiness", "Loop Single Files", true));
    assert!(draft.apply_text_value("Readiness", "Unpause Action", "Always"));
    assert!(draft.apply_text_value("Readiness", "Autoplay Min Users", "3"));
    assert!(draft.apply_text_value(
        "Privacy",
        "Trusted Domains",
        "youtube.com\n*.example.com/videos"
    ));
    assert!(draft.apply_text_value("Media Search", "Directories", "C:/Media\nD:/Archive"));
    assert!(draft.apply_text_value("Chat", "Input Position", "Bottom"));
    assert!(draft.apply_text_value("Chat", "Output Mode", "Scrolling"));
    assert!(draft.apply_text_value("Chat", "Input Font Size", "24"));
    assert!(draft.apply_text_value("Chat", "Output Font Weight", "50"));
    assert!(draft.apply_text_value("OSD", "Notification Timeout", "3"));
    assert!(draft.apply_bool_value("OSD", "Show Slowdown", true));
    assert!(draft.apply_bool_value("System", "Autosave Joins To List", true));
    assert!(draft.apply_bool_value("System", "Force GUI Prompt", true));
    assert!(draft.apply_text_value("System", "Language", "pt-br"));
    assert!(draft.apply_text_value("System", "Update Channel", "DEV"));

    let saved = draft.to_stored_settings();
    assert_eq!(saved.host.as_deref(), Some("syncplay.example"));
    assert_eq!(saved.port, Some(8995));
    assert_eq!(
        saved
            .server_password
            .as_ref()
            .map(|password| password.expose_secret()),
        Some("secret")
    );
    assert_eq!(
        saved.player_path.as_deref(),
        Some("C:/Program Files/mpv/mpv.exe")
    );
    let mut expected_arguments = BTreeMap::new();
    expected_arguments.insert(
        "C:/Program Files/mpv/mpv.exe".to_owned(),
        vec!["--profile=fast".to_owned(), "--no-border".to_owned()],
    );
    assert_eq!(saved.per_player_arguments, Some(expected_arguments));
    assert_eq!(
        saved.room_list,
        Some(vec!["backup-room".to_owned(), "main-room".to_owned()])
    );
    assert_eq!(saved.autoplay_initial_state, Some(true));
    assert_eq!(saved.loop_at_end_of_playlist, Some(true));
    assert_eq!(saved.loop_single_files, Some(true));
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
    assert_eq!(
        saved.media_search_directories,
        Some(vec!["C:/Media".to_owned(), "D:/Archive".to_owned()])
    );
    assert_eq!(saved.chat_input_position.as_deref(), Some("Bottom"));
    assert_eq!(saved.chat_output_mode.as_deref(), Some("Scrolling"));
    assert_eq!(saved.chat_input_relative_font_size, Some(24));
    assert_eq!(saved.chat_output_font_weight, Some(50));
    assert_eq!(saved.notification_timeout_seconds, Some(3));
    assert_eq!(saved.show_slowdown_osd, Some(true));
    assert_eq!(saved.autosave_joins_to_list, Some(true));
    assert_eq!(saved.force_gui_prompt, Some(true));
    assert_eq!(saved.language.as_deref(), Some("pt_BR"));
    assert_eq!(saved.update_channel.as_deref(), Some("dev"));
    assert_eq!(
        draft.control_value("Privacy", "Trusted Domain Count"),
        Some("2")
    );
    assert_eq!(
        draft.control_value("Media Search", "Directory Count"),
        Some("2")
    );
    assert_eq!(
        draft.control_value("Connection", "Player Arguments"),
        Some("--profile=fast --no-border")
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
fn configuration_draft_refreshes_player_arguments_when_player_path_changes() {
    let mut per_player_arguments = BTreeMap::new();
    per_player_arguments.insert("mpv".to_owned(), vec!["--idle=yes".to_owned()]);
    per_player_arguments.insert(
        "C:/Program Files/mpv/mpv.exe".to_owned(),
        vec!["--profile=fast".to_owned()],
    );
    let mut draft =
        FirstRunConfigurationDialogDraft::from_stored_settings(&StoredClientSettingsMvp {
            player_path: Some("mpv".to_owned()),
            per_player_arguments: Some(per_player_arguments),
            ..StoredClientSettingsMvp::default()
        });

    assert_eq!(
        draft.control_value("Connection", "Player Arguments"),
        Some("--idle=yes")
    );

    assert!(draft.apply_text_value("Connection", "Player Path", "C:/Program Files/mpv/mpv.exe"));

    assert_eq!(
        draft.control_value("Connection", "Player Arguments"),
        Some("--profile=fast")
    );
}
