use super::*;

#[test]
fn legacy_external_player_launch_spec_from_overrides_orders_player_args_before_file() {
    let overrides = LegacyClientArgOverrides {
        connect_requested: true,
        no_store: false,
        debug_requested: false,
        force_gui_prompt_requested: false,
        no_gui_requested: false,
        clear_gui_data_requested: false,
        language: None,
        player_path: Some("C:/players/mpv.exe".to_owned()),
        file: Some("C:/media/movie.mkv".to_owned()),
        player_args: vec!["--fs".to_owned(), "--volume=50".to_owned()],
        load_playlist_from_file: None,
        host: None,
        port: None,
        username: None,
        room: None,
        controlled_room_password_override: None,
        show_help: false,
        show_version: false,
        unknown_options: vec![],
    };

    let spec = legacy_external_player_launch_spec_from_overrides_legacy_compatible(&overrides)
        .expect("player-path should produce a launch spec");
    assert_eq!(
        spec,
        LegacyExternalPlayerLaunchSpec {
            program: PathBuf::from("C:/players/mpv.exe"),
            args: vec![
                "--fs".to_owned(),
                "--volume=50".to_owned(),
                "C:/media/movie.mkv".to_owned()
            ],
        }
    );
}

#[test]
fn legacy_external_player_launch_spec_from_overrides_preserves_launch_only_args_for_unmanaged_launch()
 {
    let overrides = LegacyClientArgOverrides {
        connect_requested: true,
        no_store: false,
        debug_requested: false,
        force_gui_prompt_requested: false,
        no_gui_requested: false,
        clear_gui_data_requested: false,
        language: None,
        player_path: Some("C:/players/mpv.exe".to_owned()),
        file: Some("C:/media/movie.mkv".to_owned()),
        player_args: vec!["--profile=fast".to_owned(), "--msg-level=all=v".to_owned()],
        load_playlist_from_file: None,
        host: None,
        port: None,
        username: None,
        room: None,
        controlled_room_password_override: None,
        show_help: false,
        show_version: false,
        unknown_options: vec![],
    };

    let spec = legacy_external_player_launch_spec_from_overrides_legacy_compatible(&overrides)
        .expect("player-path should produce a launch spec");
    assert_eq!(
        spec.args,
        vec![
            "--profile=fast".to_owned(),
            "--msg-level=all=v".to_owned(),
            "C:/media/movie.mkv".to_owned(),
        ]
    );
}

#[test]
fn legacy_external_player_launch_spec_from_overrides_returns_none_without_player_path() {
    let overrides = LegacyClientArgOverrides {
        connect_requested: true,
        no_store: false,
        debug_requested: false,
        force_gui_prompt_requested: false,
        no_gui_requested: false,
        clear_gui_data_requested: false,
        language: None,
        player_path: None,
        file: Some("C:/media/movie.mkv".to_owned()),
        player_args: vec!["--fs".to_owned()],
        load_playlist_from_file: None,
        host: None,
        port: None,
        username: None,
        room: None,
        controlled_room_password_override: None,
        show_help: false,
        show_version: false,
        unknown_options: vec![],
    };
    assert!(
        legacy_external_player_launch_spec_from_overrides_legacy_compatible(&overrides).is_none()
    );
}

#[test]
fn should_skip_legacy_external_player_launch_due_to_mpv_integration_env_respects_mpv_envs() {
    let env = TestEnvGuard::lock(&LEGACY_EXTERNAL_PLAYER_ENV_LOCK);
    let key_managed = "SYNCPLAY_CLIENT_MPV_MANAGED_LAUNCH";
    let key_client_ipc = "SYNCPLAY_CLIENT_MPV_IPC_PATH";
    let key_fallback_ipc = "SYNCPLAY_MPV_IPC_PATH";
    let old_managed = std::env::var_os(key_managed);
    let old_client_ipc = std::env::var_os(key_client_ipc);
    let old_fallback_ipc = std::env::var_os(key_fallback_ipc);
    env.remove_var(key_managed);
    env.remove_var(key_client_ipc);
    env.remove_var(key_fallback_ipc);

    assert!(
        !should_skip_legacy_external_player_launch_due_to_mpv_integration_env(),
        "no explicit IPC or managed launch env should allow legacy external spawn path"
    );
    env.set_var(key_client_ipc, r"\\.\pipe\syncplay-test");
    assert!(
        should_skip_legacy_external_player_launch_due_to_mpv_integration_env(),
        "explicit client IPC path should skip unmanaged external spawn"
    );
    env.remove_var(key_client_ipc);
    env.set_var(key_managed, "1");

    assert!(
        should_skip_legacy_external_player_launch_due_to_mpv_integration_env(),
        "managed mpv launch env should skip unmanaged external spawn"
    );

    match old_managed {
        Some(value) => env.set_var(key_managed, value),
        None => env.remove_var(key_managed),
    }
    match old_client_ipc {
        Some(value) => env.set_var(key_client_ipc, value),
        None => env.remove_var(key_client_ipc),
    }
    match old_fallback_ipc {
        Some(value) => env.set_var(key_fallback_ipc, value),
        None => env.remove_var(key_fallback_ipc),
    }
}

#[test]
fn resolve_managed_mpv_launch_program_legacy_compatible_expands_python_style_directory_inputs() {
    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be monotonic enough for test")
        .as_nanos();
    let portable_dir = std::env::temp_dir().join(format!(
        "syncplay-cli-managed-mpv-resolution-{unique_suffix}-portable-mpv"
    ));
    std::fs::create_dir_all(&portable_dir).expect("portable mpv dir should be created");
    #[cfg(windows)]
    let mpv_executable = portable_dir.join("mpv.exe");
    #[cfg(not(windows))]
    let mpv_executable = portable_dir.join("mpv");
    std::fs::write(&mpv_executable, b"").expect("portable mpv executable should be created");

    assert_eq!(
        crate::resolve_managed_mpv_launch_program_legacy_compatible(&portable_dir),
        mpv_executable
    );
    assert_eq!(
        crate::resolve_managed_mpv_launch_program_legacy_compatible(&portable_dir.join("mpv")),
        mpv_executable
    );

    let _ = std::fs::remove_file(&mpv_executable);
    let _ = std::fs::remove_dir(&portable_dir);
}

#[test]
fn legacy_player_path_requests_managed_mpv_legacy_compatible_matches_python_style_mpv_paths() {
    assert!(crate::legacy_player_path_requests_managed_mpv_legacy_compatible("mpv"));
    assert!(crate::legacy_player_path_requests_managed_mpv_legacy_compatible("C:/players/mpv.exe"));
    assert!(
        crate::legacy_player_path_requests_managed_mpv_legacy_compatible(r"C:\players\MPV.COM")
    );
    assert!(crate::legacy_player_path_requests_managed_mpv_legacy_compatible("/usr/bin/mpv"));
    assert!(
        !crate::legacy_player_path_requests_managed_mpv_legacy_compatible("C:/players/vlc.exe")
    );
    assert!(
        !crate::legacy_player_path_requests_managed_mpv_legacy_compatible("C:/players/mpvnet.exe")
    );

    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be monotonic enough for test")
        .as_nanos();
    let portable_dir = std::env::temp_dir().join(format!(
        "syncplay-cli-mpv-path-resolution-{unique_suffix}-portable-mpv"
    ));
    std::fs::create_dir_all(&portable_dir).expect("portable mpv dir should be created");
    #[cfg(windows)]
    let mpv_executable = portable_dir.join("mpv.exe");
    #[cfg(not(windows))]
    let mpv_executable = portable_dir.join("mpv");
    std::fs::write(&mpv_executable, b"").expect("portable mpv executable should be created");

    assert!(
        crate::legacy_player_path_requests_managed_mpv_legacy_compatible(
            portable_dir.to_string_lossy().as_ref()
        )
    );
    let unresolved_prefix = portable_dir.join("mpv");
    assert!(
        crate::legacy_player_path_requests_managed_mpv_legacy_compatible(
            unresolved_prefix.to_string_lossy().as_ref()
        )
    );

    let _ = std::fs::remove_file(&mpv_executable);
    let _ = std::fs::remove_dir(&portable_dir);
}

#[test]
fn legacy_player_path_compatibility_warning_line_legacy_compatible_distinguishes_mpv_and_non_mpv_values()
 {
    let mpv_overrides = LegacyClientArgOverrides {
        player_path: Some("C:/players/mpv.exe".to_owned()),
        ..Default::default()
    };
    let vlc_overrides = LegacyClientArgOverrides {
        player_path: Some("C:/players/vlc.exe".to_owned()),
        ..Default::default()
    };

    assert_eq!(
        crate::legacy_player_path_compatibility_warning_line_legacy_compatible(&mpv_overrides),
        Some(
            "warning: legacy --player-path selects managed mpv integration for Python-style mpv paths; non-mpv values remain launch-only unmanaged fallback"
        )
    );
    assert_eq!(
        crate::legacy_player_path_compatibility_warning_line_legacy_compatible(&vlc_overrides),
        Some(
            "warning: legacy non-mpv --player-path is launch-only unmanaged fallback; it is not adapter-integrated and is ignored when managed mpv or explicit-mpv-IPC is active"
        )
    );
}

#[test]
fn legacy_non_mpv_player_path_ignored_by_mpv_integration_warning_line_legacy_compatible_only_warns_for_non_mpv_values()
 {
    let mpv_overrides = LegacyClientArgOverrides {
        player_path: Some("C:/players/mpv.exe".to_owned()),
        ..Default::default()
    };
    let vlc_overrides = LegacyClientArgOverrides {
        player_path: Some("C:/players/vlc.exe".to_owned()),
        ..Default::default()
    };

    assert_eq!(
        crate::legacy_non_mpv_player_path_ignored_by_mpv_integration_warning_line_legacy_compatible(
            &mpv_overrides
        ),
        None
    );
    assert_eq!(
        crate::legacy_non_mpv_player_path_ignored_by_mpv_integration_warning_line_legacy_compatible(
            &vlc_overrides
        ),
        Some(
            "warning: legacy non-mpv --player-path was ignored because managed mpv or explicit-mpv-IPC integration is active"
        )
    );
}
