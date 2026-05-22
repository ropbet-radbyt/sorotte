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
fn persist_sorotte_cli_stored_settings_uses_env_config_root_when_path_absent() {
    let env = TestEnvGuard::lock(&STORED_SETTINGS_CONFIG_PATH_ENV_LOCK);
    let path_key = "SOROTTE_CLIENT_CONFIG_PATH";
    let root_key = "SOROTTE_CLIENT_CONFIG_ROOT";
    let prior_path = std::env::var_os(path_key);
    let prior_root = std::env::var_os(root_key);

    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be monotonic enough for test")
        .as_nanos();
    let temp_root =
        std::env::temp_dir().join(format!("sorotte-cli-config-root-test-{unique_suffix}"));
    env.remove_var(path_key);
    env.set_var(root_key, &temp_root);

    let config = ClientLoopConfig {
        host: "root.example".to_owned(),
        room: "RootRoom".to_owned(),
        ..test_client_loop_config()
    };
    persist_sorotte_cli_stored_settings_mvp_legacy_compatible(&config)
        .expect("settings should persist via config root");

    let config_path = temp_root.join("sorotte.ini");
    assert!(
        config_path.exists(),
        "config root should store sorotte.ini under the selected folder"
    );
    let loaded = load_sorotte_cli_stored_settings_mvp_legacy_compatible()
        .expect("load should succeed")
        .expect("settings should exist");
    assert_eq!(loaded.host.as_deref(), Some("root.example"));

    match prior_path {
        Some(value) => env.set_var(path_key, value),
        None => env.remove_var(path_key),
    }
    match prior_root {
        Some(value) => env.set_var(root_key, value),
        None => env.remove_var(root_key),
    }
    let _ = std::fs::remove_dir_all(&temp_root);
}

#[test]
fn persist_sorotte_cli_stored_settings_prefers_env_path_over_env_config_root() {
    let env = TestEnvGuard::lock(&STORED_SETTINGS_CONFIG_PATH_ENV_LOCK);
    let path_key = "SOROTTE_CLIENT_CONFIG_PATH";
    let root_key = "SOROTTE_CLIENT_CONFIG_ROOT";
    let prior_path = std::env::var_os(path_key);
    let prior_root = std::env::var_os(root_key);

    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be monotonic enough for test")
        .as_nanos();
    let temp_root = std::env::temp_dir().join(format!(
        "sorotte-cli-config-precedence-root-{unique_suffix}"
    ));
    let temp_path_dir = std::env::temp_dir().join(format!(
        "sorotte-cli-config-precedence-path-{unique_suffix}"
    ));
    std::fs::create_dir_all(&temp_path_dir).expect("path override dir should be created");
    let config_path = temp_path_dir.join("custom.ini");
    env.set_var(path_key, &config_path);
    env.set_var(root_key, &temp_root);

    let config = ClientLoopConfig {
        host: "path.example".to_owned(),
        ..test_client_loop_config()
    };
    persist_sorotte_cli_stored_settings_mvp_legacy_compatible(&config)
        .expect("settings should persist via config path");

    assert!(
        config_path.exists(),
        "explicit config path should be written"
    );
    assert!(
        !temp_root.join("sorotte.ini").exists(),
        "env config root should not win over env config path"
    );

    match prior_path {
        Some(value) => env.set_var(path_key, value),
        None => env.remove_var(path_key),
    }
    match prior_root {
        Some(value) => env.set_var(root_key, value),
        None => env.remove_var(root_key),
    }
    let _ = std::fs::remove_dir_all(&temp_root);
    let _ = std::fs::remove_dir_all(&temp_path_dir);
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

#[test]
fn clear_sorotte_cli_gui_state_uses_effective_config_root_without_legacy_override() {
    let config_env = TestEnvGuard::lock(&STORED_SETTINGS_CONFIG_PATH_ENV_LOCK);
    let gui_env = TestEnvGuard::lock(&SOROTTE_GUI_STATE_ROOT_ENV_LOCK);
    let path_key = "SOROTTE_CLIENT_CONFIG_PATH";
    let root_key = "SOROTTE_CLIENT_CONFIG_ROOT";
    let legacy_root_key = "SOROTTE_CLIENT_GUI_STATE_ROOT";
    let prior_path = std::env::var_os(path_key);
    let prior_root = std::env::var_os(root_key);
    let prior_legacy_root = std::env::var_os(legacy_root_key);

    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be monotonic enough for test")
        .as_nanos();
    let temp_root =
        std::env::temp_dir().join(format!("sorotte-cli-gui-state-config-root-{unique_suffix}"));
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
    config_env.remove_var(path_key);
    config_env.set_var(root_key, &temp_root);
    gui_env.remove_var(legacy_root_key);

    let cleared = clear_sorotte_cli_gui_state().expect("clearing GUI state should succeed");
    assert!(
        cleared,
        "effective config root state files should be cleared"
    );
    for path in &known_store_paths {
        assert!(
            !path.exists(),
            "known GUI state file should be removed from config root: {}",
            path.display()
        );
    }

    match prior_path {
        Some(value) => config_env.set_var(path_key, value),
        None => config_env.remove_var(path_key),
    }
    match prior_root {
        Some(value) => config_env.set_var(root_key, value),
        None => config_env.remove_var(root_key),
    }
    match prior_legacy_root {
        Some(value) => gui_env.set_var(legacy_root_key, value),
        None => gui_env.remove_var(legacy_root_key),
    }
    let _ = std::fs::remove_dir_all(&temp_root);
}
