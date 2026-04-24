use super::*;

#[test]
fn persist_syncplay_cli_player_path_setting_legacy_compatible_updates_client_settings_player_path()
{
    let _env_lock = STORED_SETTINGS_CONFIG_PATH_ENV_LOCK
        .lock()
        .expect("lock poisoned");
    let key = "SYNCPLAY_CLIENT_CONFIG_PATH";
    let prior = std::env::var_os(key);

    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be monotonic enough for test")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!(
        "syncplay-cli-config-player-path-test-{unique_suffix}"
    ));
    std::fs::create_dir_all(&temp_dir).expect("temp config dir should be created");
    let config_path = temp_dir.join("syncplay.ini");
    std::fs::write(
        &config_path,
        "[client_settings]\nplayerPath = C:/players/old.exe\n",
    )
    .expect("seed config should write");
    unsafe {
        std::env::set_var(key, &config_path);
    }

    persist_syncplay_cli_player_path_setting_legacy_compatible("C:/players/new.exe")
        .expect("player path setting should persist");
    let loaded = load_syncplay_cli_stored_settings_mvp_legacy_compatible()
        .expect("load should succeed")
        .expect("settings should exist");
    assert_eq!(loaded.player_path.as_deref(), Some("C:/players/new.exe"));
    let written_contents =
        std::fs::read_to_string(&config_path).expect("written config should be readable");
    assert!(written_contents.contains("playerPath = C:/players/new.exe\n"));

    match prior {
        Some(value) => unsafe { std::env::set_var(key, value) },
        None => unsafe { std::env::remove_var(key) },
    }
    let _ = std::fs::remove_file(&config_path);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn persist_syncplay_cli_per_player_arguments_setting_legacy_compatible_updates_client_settings_per_player_arguments()
 {
    let _env_lock = STORED_SETTINGS_CONFIG_PATH_ENV_LOCK
        .lock()
        .expect("lock poisoned");
    let key = "SYNCPLAY_CLIENT_CONFIG_PATH";
    let prior = std::env::var_os(key);

    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be monotonic enough for test")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!(
        "syncplay-cli-config-per-player-args-test-{unique_suffix}"
    ));
    std::fs::create_dir_all(&temp_dir).expect("temp config dir should be created");
    let config_path = temp_dir.join("syncplay.ini");
    std::fs::write(
        &config_path,
        "[client_settings]\nperPlayerArguments = {'C:/players/old.exe': ['--old']}\n",
    )
    .expect("seed config should write");
    unsafe {
        std::env::set_var(key, &config_path);
    }

    persist_syncplay_cli_per_player_arguments_setting_legacy_compatible(
        "C:/players/mpv.exe",
        &["--fs".to_owned(), "--profile=fast".to_owned()],
    )
    .expect("per-player arguments should persist");
    let loaded = load_syncplay_cli_stored_settings_mvp_legacy_compatible()
        .expect("load should succeed")
        .expect("settings should exist");
    let per_player_arguments = loaded
        .per_player_arguments
        .expect("perPlayerArguments map should be loaded");
    assert_eq!(
        per_player_arguments.get("C:/players/mpv.exe"),
        Some(&vec!["--fs".to_owned(), "--profile=fast".to_owned()])
    );
    assert_eq!(
        per_player_arguments.get("C:/players/old.exe"),
        Some(&vec!["--old".to_owned()]),
        "persist helper should merge with existing perPlayerArguments entries"
    );
    let written_contents =
        std::fs::read_to_string(&config_path).expect("written config should be readable");
    assert!(written_contents.contains("perPlayerArguments = "));
    assert!(written_contents.contains("'C:/players/mpv.exe': ['--fs', '--profile=fast']"));
    assert!(written_contents.contains("'C:/players/old.exe': ['--old']"));

    match prior {
        Some(value) => unsafe { std::env::set_var(key, value) },
        None => unsafe { std::env::remove_var(key) },
    }
    let _ = std::fs::remove_file(&config_path);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn persist_syncplay_cli_per_player_arguments_setting_legacy_compatible_dedupes_windows_path_variants()
 {
    let _env_lock = STORED_SETTINGS_CONFIG_PATH_ENV_LOCK
        .lock()
        .expect("lock poisoned");
    let key = "SYNCPLAY_CLIENT_CONFIG_PATH";
    let prior = std::env::var_os(key);

    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be monotonic enough for test")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!(
        "syncplay-cli-config-per-player-args-dedupe-test-{unique_suffix}"
    ));
    std::fs::create_dir_all(&temp_dir).expect("temp config dir should be created");
    let config_path = temp_dir.join("syncplay.ini");
    std::fs::write(
            &config_path,
            "[client_settings]\nperPlayerArguments = {'c:/players/MPV.EXE': ['--old'], 'C:/players/other.exe': ['--other']}\n",
        )
        .expect("seed config should write");
    unsafe {
        std::env::set_var(key, &config_path);
    }

    persist_syncplay_cli_per_player_arguments_setting_legacy_compatible(
        r"C:\Players\mpv.exe",
        &["--new".to_owned()],
    )
    .expect("per-player arguments should persist");

    let loaded = load_syncplay_cli_stored_settings_mvp_legacy_compatible()
        .expect("load should succeed")
        .expect("settings should exist");
    let per_player_arguments = loaded
        .per_player_arguments
        .expect("perPlayerArguments map should be loaded");

    assert_eq!(per_player_arguments.len(), 2);
    assert_eq!(
        per_player_arguments.get(r"C:\Players\mpv.exe"),
        Some(&vec!["--new".to_owned()])
    );
    assert!(
        !per_player_arguments.contains_key("c:/players/MPV.EXE"),
        "normalized duplicate key should be replaced by the latest persisted player_path form"
    );
    assert_eq!(
        per_player_arguments.get("C:/players/other.exe"),
        Some(&vec!["--other".to_owned()])
    );

    let written_contents =
        std::fs::read_to_string(&config_path).expect("written config should be readable");
    assert!(written_contents.contains(r"'C:\\Players\\mpv.exe': ['--new']"));
    assert!(!written_contents.contains("'c:/players/MPV.EXE': ['--old']"));

    match prior {
        Some(value) => unsafe { std::env::set_var(key, value) },
        None => unsafe { std::env::remove_var(key) },
    }
    let _ = std::fs::remove_file(&config_path);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn clear_syncplay_cli_stored_settings_legacy_compatible_removes_config_file_via_env_override_path()
{
    let _env_lock = STORED_SETTINGS_CONFIG_PATH_ENV_LOCK
        .lock()
        .expect("lock poisoned");
    let key = "SYNCPLAY_CLIENT_CONFIG_PATH";
    let prior = std::env::var_os(key);

    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be monotonic enough for test")
        .as_nanos();
    let temp_dir =
        std::env::temp_dir().join(format!("syncplay-cli-config-clear-test-{unique_suffix}"));
    std::fs::create_dir_all(&temp_dir).expect("temp config dir should be created");
    let config_path = temp_dir.join("syncplay.ini");
    std::fs::write(&config_path, "[server_data]\nhost = example.org\n")
        .expect("seed config should write");
    unsafe {
        std::env::set_var(key, &config_path);
    }

    let cleared = clear_syncplay_cli_stored_settings_legacy_compatible()
        .expect("clearing stored settings should succeed");
    assert!(cleared, "existing config file should be cleared");
    assert!(
        !config_path.exists(),
        "config file should be removed after clear-gui-data handling"
    );

    let cleared_again = clear_syncplay_cli_stored_settings_legacy_compatible()
        .expect("clearing missing settings should be a no-op");
    assert!(!cleared_again, "missing config file should report no-op");

    match prior {
        Some(value) => unsafe { std::env::set_var(key, value) },
        None => unsafe { std::env::remove_var(key) },
    }
    let _ = std::fs::remove_file(&config_path);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn clear_syncplay_cli_gui_qsettings_legacy_compatible_removes_known_store_files_via_env_override_root()
 {
    let _env_lock = LEGACY_GUI_QSETTINGS_ROOT_ENV_LOCK
        .lock()
        .expect("lock poisoned");
    let key = "SYNCPLAY_CLIENT_LEGACY_QSETTINGS_ROOT";
    let prior = std::env::var_os(key);

    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be monotonic enough for test")
        .as_nanos();
    let temp_root =
        std::env::temp_dir().join(format!("syncplay-cli-qsettings-clear-test-{unique_suffix}"));
    let syncplay_dir = temp_root.join("Syncplay");
    std::fs::create_dir_all(&syncplay_dir).expect("qsettings root should be created");
    let known_store_paths = [
        syncplay_dir.join("PlayerList.conf"),
        syncplay_dir.join("MediaBrowseDialog.conf"),
        syncplay_dir.join("MainWindow.conf"),
        syncplay_dir.join("Interface.conf"),
        syncplay_dir.join("MoreSettings.conf"),
    ];
    for path in &known_store_paths {
        std::fs::write(path, "[dummy]\nvalue = 1\n").expect("seed qsettings file should write");
    }
    let unrelated_path = syncplay_dir.join("Unrelated.conf");
    std::fs::write(&unrelated_path, "[keep]\nvalue = 1\n")
        .expect("unrelated qsettings file should write");
    unsafe {
        std::env::set_var(key, &temp_root);
    }

    let cleared = clear_syncplay_cli_gui_qsettings_legacy_compatible()
        .expect("clearing legacy GUI QSettings should succeed");
    assert!(cleared, "existing QSettings store files should be cleared");
    for path in &known_store_paths {
        assert!(
            !path.exists(),
            "known QSettings store file should be removed: {}",
            path.display()
        );
    }
    assert!(
        unrelated_path.exists(),
        "unrelated files in the QSettings root should not be removed"
    );

    let cleared_again = clear_syncplay_cli_gui_qsettings_legacy_compatible()
        .expect("clearing missing legacy GUI QSettings should be a no-op");
    assert!(
        !cleared_again,
        "missing QSettings files should report no-op"
    );

    match prior {
        Some(value) => unsafe { std::env::set_var(key, value) },
        None => unsafe { std::env::remove_var(key) },
    }
    let _ = std::fs::remove_file(&unrelated_path);
    let _ = std::fs::remove_dir_all(&temp_root);
}
