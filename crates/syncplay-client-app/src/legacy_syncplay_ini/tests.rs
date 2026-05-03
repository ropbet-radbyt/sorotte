use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use super::{
    clear_syncplay_ini_stored_client_settings_mvp_at_path,
    load_syncplay_ini_stored_client_settings_mvp_from_path,
    parse_syncplay_ini_stored_client_settings_mvp,
    update_syncplay_ini_stored_client_settings_mvp_at_path,
    upsert_syncplay_ini_stored_client_settings_mvp,
    upsert_syncplay_ini_stored_client_settings_mvp_at_path,
};
use crate::legacy_settings::{AutoplayThresholdOverride, StoredClientSettingsMvp};

fn unique_temp_syncplay_ini_path(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    std::env::temp_dir()
        .join(format!("syncplay-client-app-{label}-{unique}"))
        .join("syncplay.ini")
}

#[test]
fn parse_syncplay_ini_stored_client_settings_mvp_normalizes_and_reads_known_sections() {
    let settings = parse_syncplay_ini_stored_client_settings_mvp(
        "[general]\n\
         language = PT-br\n\
         [server_data]\n\
         port = 8999\n\
         [client_settings]\n\
         autoplayMinUsers = 3\n\
         [plex]\n\
         syncEnabled = yes\n\
         userToken = user-token\n\
         selectedServerId = machine-id\n\
         selectedServerUrl = http://plex.local:32400\n\
         selectedServerToken = server-token\n\
         [gui]\n\
         chatInputRelativeFontSize = 2\n",
    );

    assert_eq!(settings.language.as_deref(), Some("pt_BR"));
    assert_eq!(settings.port, Some(8999));
    assert_eq!(
        settings.autoplay_min_users,
        Some(AutoplayThresholdOverride::Set(3))
    );
    assert_eq!(settings.plex_sync_enabled, Some(true));
    assert_eq!(settings.plex_user_token.as_deref(), Some("user-token"));
    assert_eq!(
        settings.plex_selected_server_id.as_deref(),
        Some("machine-id")
    );
    assert_eq!(
        settings.plex_selected_server_url.as_deref(),
        Some("http://plex.local:32400")
    );
    assert_eq!(
        settings.plex_selected_server_token.as_deref(),
        Some("server-token")
    );
    assert_eq!(settings.chat_input_relative_font_size, Some(2));
}

#[test]
fn upsert_syncplay_ini_stored_client_settings_mvp_preserves_existing_entries() {
    let updated = upsert_syncplay_ini_stored_client_settings_mvp(
        "[misc]\nfoo = bar\n[client_settings]\nname = old\n",
        &StoredClientSettingsMvp {
            username: Some("alice".to_owned()),
            ..StoredClientSettingsMvp::default()
        },
    );

    assert!(updated.contains("[misc]\nfoo = bar\n"));
    assert!(updated.contains("[client_settings]\nname = alice\n"));
}

#[test]
fn upsert_syncplay_ini_stored_client_settings_mvp_writes_plex_settings() {
    let updated = upsert_syncplay_ini_stored_client_settings_mvp(
        "",
        &StoredClientSettingsMvp {
            plex_sync_enabled: Some(true),
            plex_user_token: Some("user-token".to_owned()),
            plex_selected_server_id: Some("machine-id".to_owned()),
            plex_selected_server_url: Some("http://plex.local:32400".to_owned()),
            plex_selected_server_token: Some("server-token".to_owned()),
            ..StoredClientSettingsMvp::default()
        },
    );

    assert!(updated.contains("[plex]\n"));
    assert!(updated.contains("syncEnabled = True\n"));
    assert!(updated.contains("userToken = user-token\n"));
    assert!(updated.contains("selectedServerId = machine-id\n"));
    assert!(updated.contains("selectedServerUrl = http://plex.local:32400\n"));
    assert!(updated.contains("selectedServerToken = server-token\n"));
}

#[test]
fn stored_settings_debug_redacts_plex_tokens() {
    let settings = StoredClientSettingsMvp {
        plex_user_token: Some("secret-user-token".to_owned()),
        plex_selected_server_token: Some("secret-server-token".to_owned()),
        ..StoredClientSettingsMvp::default()
    };

    let rendered = format!("{settings:?}");

    assert!(rendered.contains("<redacted>"));
    assert!(!rendered.contains("secret-user-token"));
    assert!(!rendered.contains("secret-server-token"));
}

#[test]
fn path_helpers_roundtrip_settings_file_contents() {
    let path = unique_temp_syncplay_ini_path("roundtrip");
    let settings = StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("lobby".to_owned()),
        ..StoredClientSettingsMvp::default()
    };

    upsert_syncplay_ini_stored_client_settings_mvp_at_path(&path, &settings)
        .expect("settings should write");
    let loaded = load_syncplay_ini_stored_client_settings_mvp_from_path(&path)
        .expect("settings should load")
        .expect("settings file should exist");

    assert_eq!(loaded.username.as_deref(), Some("alice"));
    assert_eq!(loaded.room.as_deref(), Some("lobby"));

    std::fs::remove_dir_all(path.parent().expect("syncplay.ini path should have parent"))
        .expect("temp test directory should be removable");
}

#[test]
fn load_syncplay_ini_stored_client_settings_mvp_from_path_returns_none_for_missing_file() {
    let path = unique_temp_syncplay_ini_path("missing");

    let loaded = load_syncplay_ini_stored_client_settings_mvp_from_path(&path)
        .expect("missing path should not error");

    assert_eq!(loaded, None);
}

#[test]
fn update_helper_loads_mutates_and_rewrites_existing_settings() {
    let path = unique_temp_syncplay_ini_path("update");
    upsert_syncplay_ini_stored_client_settings_mvp_at_path(
        &path,
        &StoredClientSettingsMvp {
            username: Some("alice".to_owned()),
            ..StoredClientSettingsMvp::default()
        },
    )
    .expect("initial settings should write");

    update_syncplay_ini_stored_client_settings_mvp_at_path(&path, |settings| {
        settings.room = Some("lobby".to_owned());
    })
    .expect("settings should update");
    let loaded = load_syncplay_ini_stored_client_settings_mvp_from_path(&path)
        .expect("settings should load")
        .expect("settings file should exist");

    assert_eq!(loaded.username.as_deref(), Some("alice"));
    assert_eq!(loaded.room.as_deref(), Some("lobby"));

    std::fs::remove_dir_all(path.parent().expect("syncplay.ini path should have parent"))
        .expect("temp test directory should be removable");
}

#[test]
fn clear_helper_removes_existing_settings_file() {
    let path = unique_temp_syncplay_ini_path("clear");
    upsert_syncplay_ini_stored_client_settings_mvp_at_path(
        &path,
        &StoredClientSettingsMvp {
            username: Some("alice".to_owned()),
            ..StoredClientSettingsMvp::default()
        },
    )
    .expect("initial settings should write");

    let cleared =
        clear_syncplay_ini_stored_client_settings_mvp_at_path(&path).expect("clear should succeed");

    assert!(cleared);
    assert!(!path.exists());

    std::fs::remove_dir_all(path.parent().expect("syncplay.ini path should have parent"))
        .expect("temp test directory should be removable");
}
