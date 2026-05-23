use super::*;

#[test]
fn gui_persisted_config_runtime_owner_persists_save_and_reload_requests() {
    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "sorotte-gui-persisted-config-owner-{}-{unique_suffix}.ini",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(path.clone()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    let saved_settings = StoredClientSettingsMvp {
        host: Some("persisted.example".to_owned()),
        room: Some("Cinema".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SaveConfiguration(saved_settings.clone()),
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let save_actions = handle.drain_actions();
    assert!(
        save_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::CompleteConfigurationSave(settings) if settings == &saved_settings
        )),
        "save should emit a completion action with the persisted settings"
    );
    for action in save_actions {
        assert!(state.apply(action));
    }
    assert_eq!(
        load_sorotte_ini_stored_client_settings_mvp_from_path(&path)
            .expect("save should leave a readable config file"),
        Some(saved_settings.clone())
    );

    let reloaded_settings = StoredClientSettingsMvp {
        host: Some("reloaded.example".to_owned()),
        room: Some("Rewatch".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(&path, &reloaded_settings)
        .expect("updating the config file should succeed");
    assert!(state.apply(GuiShellAction::BeginConfigurationReload));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::ReloadConfiguration(StoredClientSettingsMvp::default()),
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let reload_actions = handle.drain_actions();
    assert!(
        reload_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::CompleteConfigurationReload(settings) if settings == &reloaded_settings
        )),
        "reload should emit a completion action with the reloaded settings"
    );
    for action in reload_actions {
        assert!(state.apply(action));
    }

    std::fs::remove_file(&path).expect("temporary config file should be removable");
}

#[test]
fn gui_persisted_config_runtime_owner_plex_cache_uses_sorotte_cache_directory() {
    let root = test_temp_root("plex-cache-path-owner");
    let config_path = root.join("sorotte.ini");
    let owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path));

    assert_eq!(
        owner.plex_cache_path(),
        Some(root.join("cache").join("plex-watch-cache.json"))
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_persists_media_match_settings() {
    let root = test_temp_root("media-match-settings-owner");
    let config_path = root.join("sorotte.ini");
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path.clone()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    handle.push_request(GuiRuntimeRequest::SetMediaMatchFingerprintingEnabled(true));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    handle.push_request(GuiRuntimeRequest::SetMediaMatchBackgroundWarmupEnabled(
        false,
    ));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    handle.push_request(GuiRuntimeRequest::SetMediaMatchWireSharingEnabled(false));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    handle.push_request(GuiRuntimeRequest::SetMediaMatchRuntimeToleranceEnabled(
        false,
    ));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    handle.push_request(GuiRuntimeRequest::SetMediaMatchAutoplayPolicy(
        sorotte_media_match::MediaMatchAutoplayPolicy::AllowStrongSameMedia,
    ));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let settings = load_sorotte_ini_stored_client_settings_mvp_from_path(&config_path)
        .expect("media-match settings config should be readable")
        .expect("media-match settings config should exist");
    assert_eq!(settings.media_match_fingerprinting_enabled, Some(true));
    assert_eq!(settings.media_match_background_warmup_enabled, Some(false));
    assert_eq!(settings.media_match_wire_sharing_enabled, Some(false));
    assert_eq!(settings.media_match_runtime_tolerance_enabled, Some(false));
    assert_eq!(
        settings.media_match_autoplay_policy.as_deref(),
        Some("AllowStrongSameMedia")
    );

    let restarted_owner =
        GuiPersistedConfigRuntimeOwner::with_config_path_and_startup_player(Some(config_path));
    assert!(
        restarted_owner
            .media_match_runtime_snapshot
            .settings
            .fingerprinting_enabled
    );
    assert!(
        !restarted_owner
            .media_match_runtime_snapshot
            .settings
            .background_warmup_enabled
    );
    assert!(
        !restarted_owner
            .media_match_runtime_snapshot
            .settings
            .wire_sharing_enabled
    );
    assert!(
        !restarted_owner
            .media_match_runtime_snapshot
            .settings
            .runtime_tolerance_enabled
    );
    assert_eq!(
        restarted_owner
            .media_match_runtime_snapshot
            .settings
            .autoplay_policy,
        sorotte_media_match::MediaMatchAutoplayPolicy::AllowStrongSameMedia
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_recovers_media_match_config_path_from_storage_snapshot() {
    let root = test_temp_root("media-match-settings-storage-snapshot");
    let config_path = root.join("sorotte.ini");
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigStorageRuntimeSnapshot(
            GuiConfigStorageRuntimeSnapshot {
                config_path: Some(config_path.to_string_lossy().into_owned()),
                storage_root: Some(root.to_string_lossy().into_owned()),
                default_storage_root: Some(root.to_string_lossy().into_owned()),
                source_label: "test".to_owned(),
                external_override_active: false,
            },
        ))
    );

    handle.push_request(GuiRuntimeRequest::SetMediaMatchFingerprintingEnabled(true));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert_eq!(owner.config_path, Some(config_path.clone()));
    let settings = load_sorotte_ini_stored_client_settings_mvp_from_path(&config_path)
        .expect("media-match settings config should be readable")
        .expect("media-match settings config should exist");
    assert_eq!(settings.media_match_fingerprinting_enabled, Some(true));
    let expected_install_location = root
        .join("tools")
        .join("media-match")
        .join("bin")
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        state.media_match.install_location.as_deref(),
        Some(expected_install_location.as_str())
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_preserves_media_match_tools_when_player_cache_clears() {
    let root = test_temp_root("media-match-player-cache-clear");
    let config_path = root.join("sorotte.ini");
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path));
    owner.refresh_startup_media_match_snapshot(None);
    owner.media_match_runtime_snapshot.current_decision = Some("strong: seeded".to_owned());
    owner.media_match_runtime_snapshot.last_evidence = Some("seeded evidence".to_owned());
    let before = owner.media_match_runtime_snapshot.clone();

    owner.sync_player_from_lookup_and_settings(
        &|name| (name == "SOROTTE_GUI_ENABLE_TEST_PLAYER").then(|| "true".to_owned()),
        None,
        true,
    );

    assert_eq!(
        owner.media_match_runtime_snapshot.install_location,
        before.install_location
    );
    assert_eq!(
        owner.media_match_runtime_snapshot.ffmpeg_status,
        before.ffmpeg_status
    );
    assert_eq!(
        owner.media_match_runtime_snapshot.ffprobe_status,
        before.ffprobe_status
    );
    assert_eq!(
        owner.media_match_runtime_snapshot.fpcalc_status,
        before.fpcalc_status
    );
    assert_eq!(
        owner.media_match_runtime_snapshot.cache_status,
        before.cache_status
    );
    assert_eq!(owner.media_match_runtime_snapshot.current_decision, None);
    assert_eq!(owner.media_match_runtime_snapshot.last_evidence, None);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_changes_config_storage_root_and_copies_known_files() {
    let env = TestEnvGuard::lock(&CONFIG_ROOT_ENV_LOCK);
    let prior_appdata = std::env::var_os("APPDATA");
    let prior_home = std::env::var_os("HOME");
    let prior_xdg_config_home = std::env::var_os("XDG_CONFIG_HOME");
    let prior_install_root = std::env::var_os(SOROTTE_CLIENT_INSTALL_ROOT_ENV);

    let default_env_parent = test_temp_root("config-storage-default-parent");
    let install_root = test_temp_root("config-storage-install-root");
    env.set_var(SOROTTE_CLIENT_INSTALL_ROOT_ENV, &install_root);
    if cfg!(windows) {
        env.set_var("APPDATA", &default_env_parent);
    } else if cfg!(target_os = "macos") {
        env.set_var("HOME", &default_env_parent);
    } else {
        env.set_var("XDG_CONFIG_HOME", &default_env_parent);
    }

    let old_root = test_temp_root("config-storage-old-root");
    let new_root = test_temp_root("config-storage-new-root");
    let _ = std::fs::remove_dir_all(&new_root);
    let old_config_path = old_root.join("sorotte.ini");
    let saved_settings = StoredClientSettingsMvp {
        host: Some("portable.example".to_owned()),
        room: Some("Portable".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(&old_config_path, &saved_settings)
        .expect("old config should be written");
    std::fs::write(
        legacy_gui_qsettings_store_path(&old_root, "MainWindow"),
        "[MainWindow]\nactiveView = setup\n",
    )
    .expect("old GUI state should be written");
    std::fs::create_dir_all(old_root.join("cache")).expect("cache directory should be created");
    std::fs::write(old_root.join("cache").join("plex-watch-cache.json"), "{}")
        .expect("Plex cache should be written");
    std::fs::create_dir_all(old_root.join("tools").join("stream-helper"))
        .expect("tools directory should be created");
    std::fs::write(
        old_root
            .join("tools")
            .join("stream-helper")
            .join("helper.txt"),
        "tool",
    )
    .expect("tool file should be written");
    std::fs::create_dir_all(old_root.join("tools").join("media-match").join("bin"))
        .expect("media-match tools directory should be created");
    std::fs::write(
        old_root
            .join("tools")
            .join("media-match")
            .join("bin")
            .join("ffmpeg.exe"),
        "tool",
    )
    .expect("media-match tool file should be written");
    std::fs::create_dir_all(old_root.join("updates")).expect("updates directory should be created");
    std::fs::write(old_root.join("updates").join("stage.txt"), "update")
        .expect("update staging file should be written");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(old_config_path.clone()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&saved_settings);
    assert!(state.apply(GuiShellAction::BeginConfigStorageRootChange(
        new_root.display().to_string(),
    )));
    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::ChangeConfigStorageRoot {
            target: GuiConfigStorageChangeTarget::CustomRoot(new_root.display().to_string()),
            settings: saved_settings.clone(),
        },
    ));

    let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        actions.iter().any(|action| {
            matches!(
                action,
                GuiShellAction::CompleteConfigStorageRootChange { snapshot, settings }
                    if snapshot.storage_root.as_deref()
                        == Some(new_root.to_string_lossy().as_ref())
                        && settings == &saved_settings
            )
        }),
        "root change should complete with the new storage snapshot"
    );
    assert_eq!(owner.config_path, Some(new_root.join("sorotte.ini")));
    assert!(
        old_config_path.exists(),
        "changing storage roots should preserve the old config file"
    );
    assert_eq!(
        load_sorotte_ini_stored_client_settings_mvp_from_path(&new_root.join("sorotte.ini"))
            .expect("new config should be readable"),
        Some(saved_settings)
    );
    assert!(
        legacy_gui_qsettings_store_path(&new_root, "MainWindow").exists(),
        "known GUI state should be copied to the new root"
    );
    assert!(
        new_root
            .join("cache")
            .join("plex-watch-cache.json")
            .exists(),
        "cache files should be copied to the new root"
    );
    assert!(
        new_root
            .join("tools")
            .join("stream-helper")
            .join("helper.txt")
            .exists(),
        "stream-helper tools should be copied to the new root"
    );
    assert!(
        new_root
            .join("tools")
            .join("media-match")
            .join("bin")
            .join("ffmpeg.exe")
            .exists(),
        "media-match tools should be copied to the new root"
    );
    assert!(
        new_root.join("updates").join("stage.txt").exists(),
        "update staging should be copied to the new root"
    );
    let install_locator_path = sorotte_client_install_locator_path(&install_root);
    let locator_contents =
        std::fs::read_to_string(&install_locator_path).expect("install locator should be readable");
    assert_eq!(
        parse_sorotte_client_install_locator_config_root(&locator_contents, &install_root),
        Some(new_root.clone())
    );

    match prior_appdata {
        Some(value) => env.set_var("APPDATA", value),
        None => env.remove_var("APPDATA"),
    }
    match prior_home {
        Some(value) => env.set_var("HOME", value),
        None => env.remove_var("HOME"),
    }
    match prior_xdg_config_home {
        Some(value) => env.set_var("XDG_CONFIG_HOME", value),
        None => env.remove_var("XDG_CONFIG_HOME"),
    }
    match prior_install_root {
        Some(value) => env.set_var(SOROTTE_CLIENT_INSTALL_ROOT_ENV, value),
        None => env.remove_var(SOROTTE_CLIENT_INSTALL_ROOT_ENV),
    }
    let _ = std::fs::remove_dir_all(&old_root);
    let _ = std::fs::remove_dir_all(&new_root);
    let _ = std::fs::remove_dir_all(&default_env_parent);
    let _ = std::fs::remove_dir_all(&install_root);
}

#[test]
fn gui_persisted_config_runtime_owner_clears_gui_data_files_and_returns_first_run_state() {
    let root = test_temp_root("clear-gui-data-owner");
    let path = root.join("sorotte.ini");
    let saved_settings = StoredClientSettingsMvp {
        host: Some("persisted.example".to_owned()),
        room: Some("Cinema".to_owned()),
        public_servers: Some(vec![("Saved".to_owned(), "saved.example:8999".to_owned())]),
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        ..StoredClientSettingsMvp::default()
    };
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(&path, &saved_settings)
        .expect("saved configuration should be written");
    persist_gui_ui_state_at_root(
        &root,
        &GuiPersistedUiState {
            active_view: Some(GuiShellView::Setup),
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
    crate::app::media_search_cache::persist_media_search_root_index_at_root(
        &root,
        &crate::app::media_search_cache::PersistedMediaSearchRootIndexV2 {
            version: 2,
            root_key: crate::app::media_search_cache::normalized_media_search_root_key(
                std::path::Path::new("C:/Media"),
            ),
            root_path: "C:/Media".to_owned(),
            built_at_unix_ms: 1,
            candidates_by_name: std::collections::HashMap::from([(
                "episode1.mkv".to_owned(),
                vec!["Season 1\\episode1.mkv".to_owned()],
            )]),
        },
    )
    .expect("media-search cache should be written");
    let plex_cache_path = root.join("cache").join("plex-watch-cache.json");
    std::fs::create_dir_all(
        plex_cache_path
            .parent()
            .expect("Plex cache path should have parent"),
    )
    .expect("Plex cache directory should be created");
    std::fs::write(&plex_cache_path, "{}").expect("Plex cache should be written");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(path.clone()));
    let media_root_key = crate::app::media_search_cache::normalized_media_search_root_key(
        std::path::Path::new("C:/Media"),
    );
    owner.attached_media_search_index = Some(GuiAttachedMediaSearchIndex {
        roots: vec![media_root_key.clone()],
        root_indexes_by_key: std::collections::HashMap::from([(
            media_root_key.clone(),
            GuiAttachedMediaSearchRootIndex {
                root_key: media_root_key.clone(),
                root_path: std::path::PathBuf::from("C:/Media"),
                built_at_unix_ms: 1,
                candidates_by_name: std::collections::HashMap::from([(
                    "episode1.mkv".to_owned(),
                    vec!["Season 1\\episode1.mkv".to_owned()],
                )]),
            },
        )]),
        roots_requiring_refresh: [media_root_key.clone()].into_iter().collect(),
    });
    owner.attached_media_search_next_retry_at = Some(std::time::Instant::now());
    owner.attached_media_search_progress = Some(GuiAttachedMediaSearchBuildProgress {
        total_roots: 1,
        completed_roots: 0,
        current_root_key: media_root_key.clone(),
        current_root_path: std::path::PathBuf::from("C:/Media"),
        scanned_directories: 1,
        indexed_files: 1,
    });
    owner.attached_media_search_build_state = GuiAttachedMediaSearchBuildState::Building;
    owner.attached_media_search_build_roots = vec![media_root_key.clone()];
    owner.unresolved_attached_media_target = Some("episode1.mkv".to_owned());
    let (_result_tx, result_rx) = std::sync::mpsc::channel();
    let cancel_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    owner.pending_attached_media_resolution = Some(GuiPendingAttachedMediaResolution {
        roots: vec![media_root_key.clone()],
        cancel_flag: cancel_flag.clone(),
        latest_progress: std::sync::Arc::new(std::sync::Mutex::new(None)),
        result_rx,
    });
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&saved_settings);

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
    assert!(!path.exists(), "clear-GUI-data should remove sorotte.ini");
    for store_name in ["MainWindow", "Interface", "MediaBrowseDialog"] {
        assert!(
            !legacy_gui_qsettings_store_path(&root, store_name).exists(),
            "clear-GUI-data should remove legacy GUI state store {store_name}"
        );
    }
    assert!(
        !crate::app::media_search_cache::persisted_media_search_cache_root_at_root(&root).exists(),
        "clear-GUI-data should remove the persisted media-search cache"
    );
    assert!(
        !plex_cache_path.exists(),
        "clear-GUI-data should remove the persisted Plex watch cache"
    );
    assert_eq!(state.configuration.launch_mode, GuiLaunchMode::FirstRun);
    assert_eq!(state.active_view, GuiShellView::Setup);
    assert_eq!(
        state.saved_configuration,
        StoredClientSettingsMvp::default()
    );
    assert_eq!(state.last_media_dialog_directory, None);
    assert!(state.public_servers.servers.is_empty());
    assert!(state.media_search.directories.is_empty());
    assert!(cancel_flag.load(std::sync::atomic::Ordering::Relaxed));
    assert!(owner.attached_media_search_index.is_none());
    assert!(owner.pending_attached_media_resolution.is_none());
    assert!(owner.attached_media_search_next_retry_at.is_none());
    assert!(owner.attached_media_search_progress.is_none());
    assert_eq!(
        owner.attached_media_search_build_state,
        GuiAttachedMediaSearchBuildState::Idle
    );
    assert!(owner.attached_media_search_build_roots.is_empty());
    assert!(owner.unresolved_attached_media_target.is_none());

    let _ = std::fs::remove_dir_all(&root);
}
