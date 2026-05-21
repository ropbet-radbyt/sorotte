use super::*;

#[test]
fn gui_persisted_config_runtime_owner_persists_save_and_reload_requests() {
    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "syncplay-gui-persisted-config-owner-{}-{unique_suffix}.ini",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(path.clone()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

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
        load_syncplay_ini_stored_client_settings_mvp_from_path(&path)
            .expect("save should leave a readable config file"),
        Some(saved_settings.clone())
    );

    let reloaded_settings = StoredClientSettingsMvp {
        host: Some("reloaded.example".to_owned()),
        room: Some("Rewatch".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    upsert_syncplay_ini_stored_client_settings_mvp_at_path(&path, &reloaded_settings)
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
fn gui_persisted_config_runtime_owner_plex_cache_uses_syncplay_cache_directory() {
    let root = test_temp_root("plex-cache-path-owner");
    let config_path = root.join("syncplay.ini");
    let owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path));

    assert_eq!(
        owner.plex_cache_path(),
        Some(
            root.join("Syncplay")
                .join("cache")
                .join("plex-watch-cache.json")
        )
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_clears_gui_data_files_and_returns_first_run_state() {
    let root = test_temp_root("clear-gui-data-owner");
    let path = root.join("syncplay.ini");
    let saved_settings = StoredClientSettingsMvp {
        host: Some("persisted.example".to_owned()),
        room: Some("Cinema".to_owned()),
        public_servers: Some(vec![("Saved".to_owned(), "saved.example:8999".to_owned())]),
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        ..StoredClientSettingsMvp::default()
    };
    upsert_syncplay_ini_stored_client_settings_mvp_at_path(&path, &saved_settings)
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
    let plex_cache_path = root
        .join("Syncplay")
        .join("cache")
        .join("plex-watch-cache.json");
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
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&saved_settings);

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
    assert!(!path.exists(), "clear-GUI-data should remove syncplay.ini");
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
