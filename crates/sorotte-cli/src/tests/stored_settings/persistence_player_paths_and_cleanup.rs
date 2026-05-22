use super::*;

#[test]
fn persist_sorotte_cli_player_path_setting_legacy_compatible_updates_client_settings_player_path() {
    let env = TestEnvGuard::lock(&STORED_SETTINGS_CONFIG_PATH_ENV_LOCK);
    let key = "SOROTTE_CLIENT_CONFIG_PATH";
    let prior = std::env::var_os(key);

    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be monotonic enough for test")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!(
        "sorotte-cli-config-player-path-test-{unique_suffix}"
    ));
    std::fs::create_dir_all(&temp_dir).expect("temp config dir should be created");
    let config_path = temp_dir.join("sorotte.ini");
    std::fs::write(
        &config_path,
        "[client_settings]\nplayerPath = C:/players/old.exe\n",
    )
    .expect("seed config should write");
    env.set_var(key, &config_path);
    persist_sorotte_cli_player_path_setting_legacy_compatible("C:/players/new.exe")
        .expect("player path setting should persist");
    let loaded = load_sorotte_cli_stored_settings_mvp_legacy_compatible()
        .expect("load should succeed")
        .expect("settings should exist");
    assert_eq!(loaded.player_path.as_deref(), Some("C:/players/new.exe"));
    let written_contents =
        std::fs::read_to_string(&config_path).expect("written config should be readable");
    assert!(written_contents.contains("playerPath = C:/players/new.exe\n"));

    match prior {
        Some(value) => env.set_var(key, value),
        None => env.remove_var(key),
    }
    let _ = std::fs::remove_file(&config_path);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn persist_sorotte_cli_per_player_arguments_setting_legacy_compatible_updates_client_settings_per_player_arguments()
 {
    let env = TestEnvGuard::lock(&STORED_SETTINGS_CONFIG_PATH_ENV_LOCK);
    let key = "SOROTTE_CLIENT_CONFIG_PATH";
    let prior = std::env::var_os(key);

    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be monotonic enough for test")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!(
        "sorotte-cli-config-per-player-args-test-{unique_suffix}"
    ));
    std::fs::create_dir_all(&temp_dir).expect("temp config dir should be created");
    let config_path = temp_dir.join("sorotte.ini");
    std::fs::write(
        &config_path,
        "[client_settings]\nperPlayerArguments = {'C:/players/old.exe': ['--old']}\n",
    )
    .expect("seed config should write");
    env.set_var(key, &config_path);
    persist_sorotte_cli_per_player_arguments_setting_legacy_compatible(
        "C:/players/mpv.exe",
        &["--fs".to_owned(), "--profile=fast".to_owned()],
    )
    .expect("per-player arguments should persist");
    let loaded = load_sorotte_cli_stored_settings_mvp_legacy_compatible()
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
        Some(value) => env.set_var(key, value),
        None => env.remove_var(key),
    }
    let _ = std::fs::remove_file(&config_path);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn persist_sorotte_cli_per_player_arguments_setting_legacy_compatible_dedupes_windows_path_variants()
 {
    let env = TestEnvGuard::lock(&STORED_SETTINGS_CONFIG_PATH_ENV_LOCK);
    let key = "SOROTTE_CLIENT_CONFIG_PATH";
    let prior = std::env::var_os(key);

    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be monotonic enough for test")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!(
        "sorotte-cli-config-per-player-args-dedupe-test-{unique_suffix}"
    ));
    std::fs::create_dir_all(&temp_dir).expect("temp config dir should be created");
    let config_path = temp_dir.join("sorotte.ini");
    std::fs::write(
            &config_path,
            "[client_settings]\nperPlayerArguments = {'c:/players/MPV.EXE': ['--old'], 'C:/players/other.exe': ['--other']}\n",
        )
        .expect("seed config should write");
    env.set_var(key, &config_path);
    persist_sorotte_cli_per_player_arguments_setting_legacy_compatible(
        r"C:\Players\mpv.exe",
        &["--new".to_owned()],
    )
    .expect("per-player arguments should persist");

    let loaded = load_sorotte_cli_stored_settings_mvp_legacy_compatible()
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
        Some(value) => env.set_var(key, value),
        None => env.remove_var(key),
    }
    let _ = std::fs::remove_file(&config_path);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn clear_sorotte_cli_stored_settings_legacy_compatible_removes_config_file_via_env_override_path() {
    let env = TestEnvGuard::lock(&STORED_SETTINGS_CONFIG_PATH_ENV_LOCK);
    let key = "SOROTTE_CLIENT_CONFIG_PATH";
    let prior = std::env::var_os(key);

    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be monotonic enough for test")
        .as_nanos();
    let temp_dir =
        std::env::temp_dir().join(format!("sorotte-cli-config-clear-test-{unique_suffix}"));
    std::fs::create_dir_all(&temp_dir).expect("temp config dir should be created");
    let config_path = temp_dir.join("sorotte.ini");
    std::fs::write(&config_path, "[server_data]\nhost = example.org\n")
        .expect("seed config should write");
    env.set_var(key, &config_path);
    let cleared = clear_sorotte_cli_stored_settings_legacy_compatible()
        .expect("clearing stored settings should succeed");
    assert!(cleared, "existing config file should be cleared");
    assert!(
        !config_path.exists(),
        "config file should be removed after clear-gui-data handling"
    );

    let cleared_again = clear_sorotte_cli_stored_settings_legacy_compatible()
        .expect("clearing missing settings should be a no-op");
    assert!(!cleared_again, "missing config file should report no-op");

    match prior {
        Some(value) => env.set_var(key, value),
        None => env.remove_var(key),
    }
    let _ = std::fs::remove_file(&config_path);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn clear_sorotte_cli_gui_state_removes_known_store_files_via_env_override_root() {
    let env = TestEnvGuard::lock(&SOROTTE_GUI_STATE_ROOT_ENV_LOCK);
    let key = "SOROTTE_CLIENT_GUI_STATE_ROOT";
    let prior = std::env::var_os(key);

    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be monotonic enough for test")
        .as_nanos();
    let temp_root =
        std::env::temp_dir().join(format!("sorotte-cli-gui-state-clear-test-{unique_suffix}"));
    std::fs::create_dir_all(&temp_root).expect("GUI state root should be created");
    let known_store_paths = [
        temp_root.join("PlayerList.ini"),
        temp_root.join("MediaBrowseDialog.ini"),
        temp_root.join("MainWindow.ini"),
        temp_root.join("Interface.ini"),
        temp_root.join("MoreSettings.ini"),
    ];
    for path in &known_store_paths {
        std::fs::write(path, "[dummy]\nvalue = 1\n").expect("seed GUI state file should write");
    }
    let unrelated_path = temp_root.join("Unrelated.ini");
    std::fs::write(&unrelated_path, "[keep]\nvalue = 1\n")
        .expect("unrelated GUI state file should write");
    env.set_var(key, &temp_root);
    let cleared = clear_sorotte_cli_gui_state().expect("clearing GUI state should succeed");
    assert!(cleared, "existing GUI state files should be cleared");
    for path in &known_store_paths {
        assert!(
            !path.exists(),
            "known GUI state file should be removed: {}",
            path.display()
        );
    }
    assert!(
        unrelated_path.exists(),
        "unrelated files in the GUI state root should not be removed"
    );

    let cleared_again =
        clear_sorotte_cli_gui_state().expect("clearing missing GUI state should be a no-op");
    assert!(
        !cleared_again,
        "missing GUI state files should report no-op"
    );

    match prior {
        Some(value) => env.set_var(key, value),
        None => env.remove_var(key),
    }
    let _ = std::fs::remove_file(&unrelated_path);
    let _ = std::fs::remove_dir_all(&temp_root);
}
