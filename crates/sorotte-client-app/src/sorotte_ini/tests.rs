use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use super::{
    clear_sorotte_ini_stored_client_settings_mvp_at_path,
    load_sorotte_ini_stored_client_settings_mvp_from_path,
    parse_sorotte_ini_stored_client_settings_mvp,
    update_sorotte_ini_stored_client_settings_mvp_at_path,
    upsert_sorotte_ini_stored_client_settings_mvp,
    upsert_sorotte_ini_stored_client_settings_mvp_at_path,
    upsert_sorotte_ini_stored_client_settings_mvp_clearing_plex_identity,
};
use crate::legacy_settings::{AutoplayThresholdOverride, StoredClientSettingsMvp};

fn unique_temp_sorotte_ini_path(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    std::env::temp_dir()
        .join(format!("sorotte-client-app-{label}-{unique}"))
        .join("sorotte.ini")
}

#[test]
fn parse_sorotte_ini_stored_client_settings_mvp_normalizes_and_reads_known_sections() {
    let settings = parse_sorotte_ini_stored_client_settings_mvp(
        "[general]\n\
         language = PT-br\n\
         updateChannel = DEV\n\
         [server_data]\n\
         port = 8999\n\
         [client_settings]\n\
         autoplayMinUsers = 3\n\
         [plex]\n\
         syncEnabled = yes\n\
         streamingEnabled = true\n\
         userToken = user-token\n\
         selectedServerId = machine-id\n\
         selectedServerUrl = http://plex.local:32400\n\
         selectedServerToken = server-token\n\
         [plugins]\n\
         streamSupportEnabled = false\n\
         mediaMatchingEnabled = yes\n\
         plexEnabled = no\n\
         [gui]\n\
         chatInputRelativeFontSize = 2\n",
    );

    assert_eq!(settings.language.as_deref(), Some("pt_BR"));
    assert_eq!(settings.update_channel.as_deref(), Some("dev"));
    assert_eq!(settings.port, Some(8999));
    assert_eq!(
        settings.autoplay_min_users,
        Some(AutoplayThresholdOverride::Set(3))
    );
    assert_eq!(settings.plex_sync_enabled, Some(true));
    assert_eq!(settings.plex_streaming_enabled, Some(true));
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
    assert_eq!(settings.stream_support_plugin_enabled, Some(false));
    assert_eq!(settings.media_matching_plugin_enabled, Some(true));
    assert_eq!(settings.plex_plugin_enabled, Some(false));
    assert_eq!(settings.chat_input_relative_font_size, Some(2));
}

#[test]
fn parse_sorotte_ini_stored_client_settings_mvp_leaves_missing_plugin_gates_unset() {
    let settings =
        parse_sorotte_ini_stored_client_settings_mvp("[client_settings]\nname = alice\n");

    assert_eq!(settings.stream_support_plugin_enabled, None);
    assert_eq!(settings.media_matching_plugin_enabled, None);
    assert_eq!(settings.plex_plugin_enabled, None);
}

#[test]
fn parse_sorotte_ini_stored_client_settings_mvp_filters_blank_media_search_directories() {
    let settings = parse_sorotte_ini_stored_client_settings_mvp(
        "[client_settings]\n\
         mediaSearchDirectories = ['Z:/Anime/Seasonal', '', ' Z:/Anime/Temp ', '', 'Z:/Anime/Anime Shows', '']\n",
    );

    assert_eq!(
        settings.media_search_directories,
        Some(vec![
            "Z:/Anime/Seasonal".to_owned(),
            "Z:/Anime/Temp".to_owned(),
            "Z:/Anime/Anime Shows".to_owned(),
        ])
    );
}

#[test]
fn parse_sorotte_ini_stored_client_settings_mvp_reads_media_match_settings() {
    let settings = parse_sorotte_ini_stored_client_settings_mvp(
        "[client_settings]\n\
         mediaMatchFingerprintingEnabled = True\n\
         mediaMatchBackgroundWarmupEnabled = False\n\
         mediaMatchWireSharingEnabled = False\n\
         mediaMatchRuntimeToleranceEnabled = False\n\
         mediaMatchAutoplayPolicy = AllowStrongSameMedia\n",
    );

    assert_eq!(settings.media_match_fingerprinting_enabled, Some(true));
    assert_eq!(settings.media_match_background_warmup_enabled, Some(false));
    assert_eq!(settings.media_match_wire_sharing_enabled, Some(false));
    assert_eq!(settings.media_match_runtime_tolerance_enabled, Some(false));
    assert_eq!(
        settings.media_match_autoplay_policy.as_deref(),
        Some("AllowStrongSameMedia")
    );
}

#[test]
fn upsert_sorotte_ini_stored_client_settings_mvp_preserves_existing_entries() {
    let updated = upsert_sorotte_ini_stored_client_settings_mvp(
        "[misc]\nfoo = bar\n[client_settings]\nname = old\n",
        &StoredClientSettingsMvp {
            username: Some("alice".to_owned()),
            update_channel: Some("dev".to_owned()),
            ..StoredClientSettingsMvp::default()
        },
    );

    assert!(updated.contains("[misc]\nfoo = bar\n"));
    assert!(updated.contains("[client_settings]\nname = alice\n"));
    assert!(updated.contains("updateChannel = dev\n"));
}

#[test]
fn upsert_sorotte_ini_stored_client_settings_mvp_writes_media_match_settings() {
    let updated = upsert_sorotte_ini_stored_client_settings_mvp(
        "",
        &StoredClientSettingsMvp {
            media_match_fingerprinting_enabled: Some(true),
            media_match_background_warmup_enabled: Some(false),
            media_match_wire_sharing_enabled: Some(false),
            media_match_runtime_tolerance_enabled: Some(false),
            media_match_autoplay_policy: Some("AllowStrongSameMedia".to_owned()),
            ..StoredClientSettingsMvp::default()
        },
    );

    assert!(updated.contains("[client_settings]\n"));
    assert!(updated.contains("mediaMatchFingerprintingEnabled = True\n"));
    assert!(updated.contains("mediaMatchBackgroundWarmupEnabled = False\n"));
    assert!(updated.contains("mediaMatchWireSharingEnabled = False\n"));
    assert!(updated.contains("mediaMatchRuntimeToleranceEnabled = False\n"));
    assert!(updated.contains("mediaMatchAutoplayPolicy = AllowStrongSameMedia\n"));
}

#[test]
fn upsert_sorotte_ini_stored_client_settings_mvp_writes_plugin_enablement() {
    let updated = upsert_sorotte_ini_stored_client_settings_mvp(
        "",
        &StoredClientSettingsMvp {
            stream_support_plugin_enabled: Some(false),
            media_matching_plugin_enabled: Some(false),
            plex_plugin_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        },
    );

    assert!(updated.contains("[plugins]\n"));
    assert!(updated.contains("streamSupportEnabled = False\n"));
    assert!(updated.contains("mediaMatchingEnabled = False\n"));
    assert!(updated.contains("plexEnabled = True\n"));
}

#[test]
fn upsert_sorotte_ini_disabling_plugins_preserves_existing_plugin_data() {
    let updated = upsert_sorotte_ini_stored_client_settings_mvp(
        "[plugins]\n\
         streamSupportEnabled = True\n\
         mediaMatchingEnabled = True\n\
         plexEnabled = True\n\
         [client_settings]\n\
         mediaMatchFingerprintingEnabled = True\n\
         mediaMatchBackgroundWarmupEnabled = True\n\
         [plex]\n\
         syncEnabled = True\n\
         streamingEnabled = True\n\
         userToken = old-user-token\n\
         selectedServerId = old-machine\n\
         selectedServerUrl = http://old-plex.local:32400\n\
         selectedServerToken = old-server-token\n",
        &StoredClientSettingsMvp {
            stream_support_plugin_enabled: Some(false),
            media_matching_plugin_enabled: Some(false),
            plex_plugin_enabled: Some(false),
            ..StoredClientSettingsMvp::default()
        },
    );

    assert!(updated.contains("streamSupportEnabled = False\n"));
    assert!(updated.contains("mediaMatchingEnabled = False\n"));
    assert!(updated.contains("plexEnabled = False\n"));
    assert!(updated.contains("mediaMatchFingerprintingEnabled = True\n"));
    assert!(updated.contains("mediaMatchBackgroundWarmupEnabled = True\n"));
    assert!(updated.contains("syncEnabled = True\n"));
    assert!(updated.contains("streamingEnabled = True\n"));
    assert!(updated.contains("userToken = old-user-token\n"));
    assert!(updated.contains("selectedServerId = old-machine\n"));
    assert!(updated.contains("selectedServerUrl = http://old-plex.local:32400\n"));
    assert!(updated.contains("selectedServerToken = old-server-token\n"));
}

#[test]
fn upsert_sorotte_ini_stored_client_settings_mvp_writes_plex_settings() {
    let updated = upsert_sorotte_ini_stored_client_settings_mvp(
        "",
        &StoredClientSettingsMvp {
            plex_sync_enabled: Some(true),
            plex_streaming_enabled: Some(true),
            plex_user_token: Some("user-token".to_owned()),
            plex_selected_server_id: Some("machine-id".to_owned()),
            plex_selected_server_url: Some("http://plex.local:32400".to_owned()),
            plex_selected_server_token: Some("server-token".to_owned()),
            ..StoredClientSettingsMvp::default()
        },
    );

    assert!(updated.contains("[plex]\n"));
    assert!(updated.contains("syncEnabled = True\n"));
    assert!(updated.contains("streamingEnabled = True\n"));
    assert!(updated.contains("userToken = user-token\n"));
    assert!(updated.contains("selectedServerId = machine-id\n"));
    assert!(updated.contains("selectedServerUrl = http://plex.local:32400\n"));
    assert!(updated.contains("selectedServerToken = server-token\n"));
}

#[test]
fn upsert_sorotte_ini_preserves_plex_identity_when_only_disabling_sync() {
    let updated = upsert_sorotte_ini_stored_client_settings_mvp(
        "[plex]\n\
         syncEnabled = True\n\
         userToken = old-user-token\n\
         selectedServerId = old-machine\n\
         selectedServerUrl = http://old-plex.local:32400\n\
         selectedServerToken = old-server-token\n\
         [PLEX]\n\
         userToken = duplicate-user-token\n\
         selectedServerId = duplicate-machine\n\
         selectedServerUrl = http://duplicate-plex.local:32400\n\
         selectedServerToken = duplicate-server-token\n",
        &StoredClientSettingsMvp {
            plex_sync_enabled: Some(false),
            ..StoredClientSettingsMvp::default()
        },
    );

    assert!(updated.contains("[plex]\n"));
    assert!(updated.contains("syncEnabled = False\n"));
    assert!(updated.contains("userToken = old-user-token\n"));
    assert!(updated.contains("selectedServerId = old-machine\n"));
    assert!(updated.contains("selectedServerUrl = http://old-plex.local:32400\n"));
    assert!(updated.contains("selectedServerToken = old-server-token\n"));
}

#[test]
fn upsert_sorotte_ini_stored_client_settings_mvp_clearing_plex_identity_removes_credentials() {
    let updated = upsert_sorotte_ini_stored_client_settings_mvp_clearing_plex_identity(
        "[plex]\n\
         syncEnabled = True\n\
         userToken = old-user-token\n\
         selectedServerId = old-machine\n\
         selectedServerUrl = http://old-plex.local:32400\n\
         selectedServerToken = old-server-token\n",
        &StoredClientSettingsMvp {
            plex_sync_enabled: Some(false),
            ..StoredClientSettingsMvp::default()
        },
    );

    assert!(updated.contains("[plex]\n"));
    assert!(updated.contains("syncEnabled = False\n"));
    assert!(!updated.contains("old-user-token"));
    assert!(!updated.contains("old-machine"));
    assert!(!updated.contains("old-plex.local"));
    assert!(!updated.contains("old-server-token"));
    assert!(!updated.contains("duplicate-user-token"));
    assert!(!updated.contains("duplicate-machine"));
    assert!(!updated.contains("duplicate-plex.local"));
    assert!(!updated.contains("duplicate-server-token"));

    let parsed = parse_sorotte_ini_stored_client_settings_mvp(&updated);
    assert_eq!(parsed.plex_user_token, None);
    assert_eq!(parsed.plex_selected_server_id, None);
    assert_eq!(parsed.plex_selected_server_url, None);
    assert_eq!(parsed.plex_selected_server_token, None);
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
    let path = unique_temp_sorotte_ini_path("roundtrip");
    let settings = StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("lobby".to_owned()),
        ..StoredClientSettingsMvp::default()
    };

    upsert_sorotte_ini_stored_client_settings_mvp_at_path(&path, &settings)
        .expect("settings should write");
    let loaded = load_sorotte_ini_stored_client_settings_mvp_from_path(&path)
        .expect("settings should load")
        .expect("settings file should exist");

    assert_eq!(loaded.username.as_deref(), Some("alice"));
    assert_eq!(loaded.room.as_deref(), Some("lobby"));

    std::fs::remove_dir_all(path.parent().expect("sorotte.ini path should have parent"))
        .expect("temp test directory should be removable");
}

#[test]
fn load_sorotte_ini_stored_client_settings_mvp_from_path_returns_none_for_missing_file() {
    let path = unique_temp_sorotte_ini_path("missing");

    let loaded = load_sorotte_ini_stored_client_settings_mvp_from_path(&path)
        .expect("missing path should not error");

    assert_eq!(loaded, None);
}

#[test]
fn update_helper_loads_mutates_and_rewrites_existing_settings() {
    let path = unique_temp_sorotte_ini_path("update");
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(
        &path,
        &StoredClientSettingsMvp {
            username: Some("alice".to_owned()),
            ..StoredClientSettingsMvp::default()
        },
    )
    .expect("initial settings should write");

    update_sorotte_ini_stored_client_settings_mvp_at_path(&path, |settings| {
        settings.room = Some("lobby".to_owned());
    })
    .expect("settings should update");
    let loaded = load_sorotte_ini_stored_client_settings_mvp_from_path(&path)
        .expect("settings should load")
        .expect("settings file should exist");

    assert_eq!(loaded.username.as_deref(), Some("alice"));
    assert_eq!(loaded.room.as_deref(), Some("lobby"));

    std::fs::remove_dir_all(path.parent().expect("sorotte.ini path should have parent"))
        .expect("temp test directory should be removable");
}

#[test]
fn clear_helper_removes_existing_settings_file() {
    let path = unique_temp_sorotte_ini_path("clear");
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(
        &path,
        &StoredClientSettingsMvp {
            username: Some("alice".to_owned()),
            ..StoredClientSettingsMvp::default()
        },
    )
    .expect("initial settings should write");

    let cleared =
        clear_sorotte_ini_stored_client_settings_mvp_at_path(&path).expect("clear should succeed");

    assert!(cleared);
    assert!(!path.exists());

    std::fs::remove_dir_all(path.parent().expect("sorotte.ini path should have parent"))
        .expect("temp test directory should be removable");
}
