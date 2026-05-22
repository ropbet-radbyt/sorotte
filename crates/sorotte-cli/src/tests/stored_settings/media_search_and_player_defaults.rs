use super::*;

#[test]
fn resolve_legacy_startup_file_with_media_search_fallback_legacy_compatible_resolves_recursive_media_match()
 {
    let unique_suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after epoch")
        .as_nanos();
    let temp_root = std::env::temp_dir().join(format!(
        "sorotte-cli-media-search-recursive-{unique_suffix}"
    ));
    let nested_dir = temp_root.join("Season1");
    std::fs::create_dir_all(&nested_dir).expect("nested media dir should be created");
    let media_file = nested_dir.join("episode1.mkv");
    std::fs::write(&media_file, b"").expect("media file should be created");

    let resolution = resolve_legacy_startup_file_with_media_search_fallback_legacy_compatible(
        Some("episode1.mkv"),
        Some(&StoredClientSettingsMvp {
            media_search_directories: Some(vec![temp_root.to_string_lossy().into_owned()]),
            ..StoredClientSettingsMvp::default()
        }),
    );

    assert_eq!(
        resolution.file.as_deref(),
        Some(media_file.to_string_lossy().as_ref())
    );
    assert!(
        resolution.warning_lines.is_empty(),
        "successful recursive startup-file resolution should not emit warnings: {:?}",
        resolution.warning_lines
    );

    let _ = std::fs::remove_dir_all(&temp_root);
}

#[test]
fn resolve_legacy_startup_file_with_media_search_fallback_legacy_compatible_respects_folder_search_timeout_zero()
 {
    let unique_suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after epoch")
        .as_nanos();
    let temp_root =
        std::env::temp_dir().join(format!("sorotte-cli-media-search-timeout-{unique_suffix}"));
    let nested_dir = temp_root.join("Season1");
    std::fs::create_dir_all(&nested_dir).expect("nested media dir should be created");
    let media_file = nested_dir.join("episode1.mkv");
    std::fs::write(&media_file, b"").expect("media file should be created");

    let resolution = resolve_legacy_startup_file_with_media_search_fallback_legacy_compatible(
        Some("episode1.mkv"),
        Some(&StoredClientSettingsMvp {
            media_search_directories: Some(vec![temp_root.to_string_lossy().into_owned()]),
            folder_search_timeout_seconds: Some(0.0),
            ..StoredClientSettingsMvp::default()
        }),
    );

    assert_eq!(resolution.file.as_deref(), Some("episode1.mkv"));
    assert!(
        resolution
            .warning_lines
            .iter()
            .any(|line| line.contains("folderSearchTimeout is 0")),
        "expected deterministic timeout warning, got {:?}",
        resolution.warning_lines
    );

    let _ = std::fs::remove_dir_all(&temp_root);
}

#[test]
fn apply_stored_media_search_startup_file_fallback_if_missing_legacy_compatible_updates_startup_file()
 {
    let unique_suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after epoch")
        .as_nanos();
    let temp_root =
        std::env::temp_dir().join(format!("sorotte-cli-media-search-apply-{unique_suffix}"));
    let nested_dir = temp_root.join("Season1");
    std::fs::create_dir_all(&nested_dir).expect("nested media dir should be created");
    let media_file = nested_dir.join("episode1.mkv");
    std::fs::write(&media_file, b"").expect("media file should be created");

    let mut overrides = LegacyClientArgOverrides {
        file: Some("episode1.mkv".to_owned()),
        ..LegacyClientArgOverrides::default()
    };
    let settings = StoredClientSettingsMvp {
        media_search_directories: Some(vec![temp_root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    };

    apply_stored_media_search_startup_file_fallback_if_missing_legacy_compatible(
        &mut overrides,
        Some(&settings),
    );

    assert_eq!(
        overrides.file.as_deref(),
        Some(media_file.to_string_lossy().as_ref())
    );

    let _ = std::fs::remove_dir_all(&temp_root);
}

#[test]
fn apply_stored_legacy_startup_player_defaults_if_arg_absent_uses_stored_player_path() {
    let mut overrides = LegacyClientArgOverrides {
        connect_requested: true,
        ..LegacyClientArgOverrides::default()
    };
    let settings = StoredClientSettingsMvp {
        player_path: Some("C:/players/stored-mpv.exe".to_owned()),
        per_player_arguments: Some(std::collections::BTreeMap::from([(
            "C:/players/stored-mpv.exe".to_owned(),
            vec!["--fs".to_owned(), "--profile=fast".to_owned()],
        )])),
        ..StoredClientSettingsMvp::default()
    };

    apply_stored_legacy_startup_player_defaults_if_arg_absent(&mut overrides, &settings);
    assert_eq!(
        overrides.player_path.as_deref(),
        Some("C:/players/stored-mpv.exe")
    );
    assert_eq!(
        overrides.player_args,
        vec!["--fs".to_owned(), "--profile=fast".to_owned()]
    );

    let mut arg_overrides = LegacyClientArgOverrides {
        connect_requested: true,
        player_path: Some("C:/players/arg-mpv.exe".to_owned()),
        ..LegacyClientArgOverrides::default()
    };
    apply_stored_legacy_startup_player_defaults_if_arg_absent(&mut arg_overrides, &settings);
    assert_eq!(
        arg_overrides.player_path.as_deref(),
        Some("C:/players/arg-mpv.exe"),
        "explicit legacy arg should take precedence over stored playerPath"
    );
    assert!(
        arg_overrides.player_args.is_empty(),
        "stored per-player args should only apply when the selected player path matches a stored entry"
    );
}

#[test]
fn apply_stored_legacy_startup_player_defaults_if_arg_absent_appends_stored_per_player_arguments_after_cli_args()
 {
    let mut overrides = LegacyClientArgOverrides {
        connect_requested: true,
        player_path: Some("C:/players/stored-mpv.exe".to_owned()),
        player_args: vec!["--profile=fast".to_owned(), "--msg-level=all=v".to_owned()],
        ..LegacyClientArgOverrides::default()
    };
    let settings = StoredClientSettingsMvp {
        per_player_arguments: Some(std::collections::BTreeMap::from([(
            "C:/players/stored-mpv.exe".to_owned(),
            vec!["--fs".to_owned(), "--script-opts=osc=no".to_owned()],
        )])),
        ..StoredClientSettingsMvp::default()
    };

    apply_stored_legacy_startup_player_defaults_if_arg_absent(&mut overrides, &settings);
    assert_eq!(
        overrides.player_args,
        vec![
            "--profile=fast".to_owned(),
            "--msg-level=all=v".to_owned(),
            "--fs".to_owned(),
            "--script-opts=osc=no".to_owned(),
        ]
    );
}

#[test]
fn apply_stored_legacy_startup_player_defaults_if_arg_absent_matches_windows_path_case_and_slashes()
{
    let mut overrides = LegacyClientArgOverrides {
        connect_requested: true,
        player_path: Some(r"C:\Players\MPV.EXE".to_owned()),
        ..LegacyClientArgOverrides::default()
    };
    let settings = StoredClientSettingsMvp {
        per_player_arguments: Some(std::collections::BTreeMap::from([
            (
                "c:/players/mpv.exe".to_owned(),
                vec!["--profile=fast".to_owned()],
            ),
            ("C:/Players/MPV.EXE".to_owned(), vec!["--exact".to_owned()]),
        ])),
        ..StoredClientSettingsMvp::default()
    };

    apply_stored_legacy_startup_player_defaults_if_arg_absent(&mut overrides, &settings);
    assert_eq!(
        overrides.player_args,
        vec!["--exact".to_owned()],
        "exact key should win before normalized fallback lookup"
    );

    let mut normalized_only_overrides = LegacyClientArgOverrides {
        connect_requested: true,
        player_path: Some(r"C:\Players\mpv.exe".to_owned()),
        ..LegacyClientArgOverrides::default()
    };
    let normalized_only_settings = StoredClientSettingsMvp {
        per_player_arguments: Some(std::collections::BTreeMap::from([(
            "c:/players/MPV.EXE".to_owned(),
            vec!["--fs".to_owned()],
        )])),
        ..StoredClientSettingsMvp::default()
    };

    apply_stored_legacy_startup_player_defaults_if_arg_absent(
        &mut normalized_only_overrides,
        &normalized_only_settings,
    );
    assert_eq!(
        normalized_only_overrides.player_args,
        vec!["--fs".to_owned()]
    );
}

#[test]
fn apply_stored_legacy_startup_player_defaults_if_arg_absent_keeps_unix_path_matching_case_sensitive()
 {
    let mut overrides = LegacyClientArgOverrides {
        connect_requested: true,
        player_path: Some("/usr/bin/MPV".to_owned()),
        ..LegacyClientArgOverrides::default()
    };
    let settings = StoredClientSettingsMvp {
        per_player_arguments: Some(std::collections::BTreeMap::from([(
            "/usr/bin/mpv".to_owned(),
            vec!["--fs".to_owned()],
        )])),
        ..StoredClientSettingsMvp::default()
    };

    apply_stored_legacy_startup_player_defaults_if_arg_absent(&mut overrides, &settings);
    assert!(
        overrides.player_args.is_empty(),
        "unix-style player paths should not match with case-only differences"
    );
}
