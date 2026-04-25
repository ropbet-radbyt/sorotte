use super::*;

#[test]
fn gui_persisted_config_runtime_owner_warm_starts_shared_playlist_resolution_from_persisted_cache()
{
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        opened_paths: Vec<String>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn open_file(&mut self, path: &str) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .opened_paths
                .push(path.to_owned());
            Ok(())
        }
    }

    let root = test_temp_root("persisted-shared-playlist-warm-start");
    let config_path = root.join("syncplay.ini");
    let nested_directory = root.join("nested");
    std::fs::create_dir_all(&nested_directory)
        .expect("warm-start shared-playlist fixture directory should be created");
    let selected_media_path = nested_directory.join("episode2.mkv");
    std::fs::write(&selected_media_path, b"test")
        .expect("warm-start shared-playlist fixture should be written");
    write_persisted_media_search_root_index(
        &root,
        &root,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_millis() as u64,
        &[("episode2.mkv", &["nested\\episode2.mkv"])],
    );

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) =
        GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path))
            .with_client_core_chat_session_runtime("alice", "room1")
            .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_lines([
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#
            .to_owned(),
        r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"bob"}}}"#
            .to_owned(),
        r#"{"Set":{"playlistIndex":{"index":1,"user":"bob"}}}"#.to_owned(),
    ]);

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths,
        vec![selected_media_path.to_string_lossy().into_owned()]
    );
    assert!(
        owner
            .attached_media_search_index
            .as_ref()
            .is_some_and(|index| {
                index.root_indexes_by_key.contains_key(
                    &crate::app::media_search_cache::normalized_media_search_root_key(&root),
                )
            }),
        "warm-start media resolution should load the initial root segment from the persisted cache before any later root warming occurs"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_resolves_from_stale_persisted_cache_and_refreshes_in_background()
 {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        opened_paths: Vec<String>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn open_file(&mut self, path: &str) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .opened_paths
                .push(path.to_owned());
            Ok(())
        }
    }

    let root = test_temp_root("persisted-shared-playlist-stale-cache");
    let config_path = root.join("syncplay.ini");
    let nested_directory = root.join("nested");
    std::fs::create_dir_all(&nested_directory)
        .expect("stale shared-playlist fixture directory should be created");
    let selected_media_path = nested_directory.join("episode2.mkv");
    std::fs::write(&selected_media_path, b"test")
        .expect("stale shared-playlist fixture should be written");
    let stale_built_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_millis()
        .saturating_sub(120_000) as u64;
    write_persisted_media_search_root_index(
        &root,
        &root,
        stale_built_at,
        &[("episode2.mkv", &["nested\\episode2.mkv"])],
    );

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) =
        GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path))
            .with_client_core_chat_session_runtime("alice", "room1")
            .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_lines([
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#
            .to_owned(),
        r#"{"Set":{"playlistChange":{"files":["episode2.mkv"],"user":"bob"}}}"#.to_owned(),
        r#"{"Set":{"playlistIndex":{"index":0,"user":"bob"}}}"#.to_owned(),
    ]);

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths,
        vec![selected_media_path.to_string_lossy().into_owned()]
    );

    let refresh_deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < refresh_deadline {
        if owner.pending_attached_media_resolution.is_none()
            && owner
                .attached_media_search_index
                .as_ref()
                .and_then(|index| {
                    index
                        .root_indexes_by_key
                        .get(
                            &crate::app::media_search_cache::normalized_media_search_root_key(
                                &root,
                            ),
                        )
                        .map(|root_index| root_index.built_at_unix_ms)
                })
                .is_some_and(|built_at| built_at > stale_built_at)
        {
            break;
        }
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    assert!(
        owner
            .attached_media_search_index
            .as_ref()
            .and_then(|index| {
                index
                    .root_indexes_by_key
                    .get(&crate::app::media_search_cache::normalized_media_search_root_key(&root))
                    .map(|root_index| root_index.built_at_unix_ms)
            })
            .is_some_and(|built_at| built_at > stale_built_at),
        "stale persisted cache entries should refresh in the background after the immediate warm-start hit"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_prefers_current_player_locality_for_duplicate_cached_names() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        opened_paths: Vec<String>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn open_file(&mut self, path: &str) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .opened_paths
                .push(path.to_owned());
            Ok(())
        }
    }

    let root = test_temp_root("persisted-shared-playlist-duplicate-ranking");
    let config_path = root.join("syncplay.ini");
    let preferred_root = root.join("preferred");
    let preferred_season = preferred_root.join("season-1");
    let fallback_root = root.join("fallback");
    std::fs::create_dir_all(&preferred_season)
        .expect("preferred duplicate-ranking fixture directory should be created");
    std::fs::create_dir_all(&fallback_root)
        .expect("fallback duplicate-ranking fixture directory should be created");
    let preferred_current_path = preferred_season.join("episode1.mkv");
    let preferred_target_path = preferred_season.join("episode2.mkv");
    let fallback_target_path = fallback_root.join("episode2.mkv");
    std::fs::write(&preferred_current_path, b"test")
        .expect("preferred duplicate-ranking current fixture should be written");
    std::fs::write(&preferred_target_path, b"test")
        .expect("preferred duplicate-ranking target fixture should be written");
    std::fs::write(&fallback_target_path, b"test")
        .expect("fallback duplicate-ranking target fixture should be written");

    let built_at_unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_millis() as u64;
    write_persisted_media_search_root_index(
        &root,
        &preferred_root,
        built_at_unix_ms,
        &[("episode2.mkv", &["season-1\\episode2.mkv"])],
    );
    write_persisted_media_search_root_index(
        &root,
        &fallback_root,
        built_at_unix_ms,
        &[("episode2.mkv", &["episode2.mkv"])],
    );

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) =
        GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path))
            .with_client_core_chat_session_runtime("alice", "room1")
            .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        syncplay_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path(preferred_current_path.to_string_lossy().into_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![
            fallback_root.to_string_lossy().into_owned(),
            preferred_root.to_string_lossy().into_owned(),
        ]),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_lines([
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#
            .to_owned(),
        r#"{"Set":{"playlistChange":{"files":["episode2.mkv"],"user":"bob"}}}"#.to_owned(),
        r#"{"Set":{"playlistIndex":{"index":0,"user":"bob"}}}"#.to_owned(),
    ]);

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths,
        vec![preferred_target_path.to_string_lossy().into_owned()]
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_keeps_cached_roots_when_one_refresh_result_fails() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        opened_paths: Vec<String>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn open_file(&mut self, path: &str) -> Result<(), syncplay_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .opened_paths
                .push(path.to_owned());
            Ok(())
        }
    }

    let root = test_temp_root("persisted-shared-playlist-partial-refresh");
    let config_path = root.join("syncplay.ini");
    let good_root = root.join("good");
    let bad_root = root.join("bad");
    std::fs::create_dir_all(&good_root)
        .expect("partial-refresh good fixture directory should be created");
    std::fs::create_dir_all(&bad_root)
        .expect("partial-refresh bad fixture directory should be created");
    let selected_media_path = good_root.join("episode2.mkv");
    std::fs::write(&selected_media_path, b"test")
        .expect("partial-refresh good fixture should be written");

    let good_key = crate::app::media_search_cache::normalized_media_search_root_key(&good_root);
    let bad_key = crate::app::media_search_cache::normalized_media_search_root_key(&bad_root);
    let stale_built_at = 1;

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, session_transport) =
        GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path))
            .with_client_core_chat_session_runtime("alice", "room1")
            .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![
            good_root.to_string_lossy().into_owned(),
            bad_root.to_string_lossy().into_owned(),
        ]),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();
    owner.attached_media_search_index = Some(GuiAttachedMediaSearchIndex {
        roots: vec![good_key.clone(), bad_key.clone()],
        root_indexes_by_key: std::collections::HashMap::from([
            (
                good_key.clone(),
                GuiAttachedMediaSearchRootIndex {
                    root_key: good_key.clone(),
                    root_path: good_root.clone(),
                    built_at_unix_ms: stale_built_at,
                    candidates_by_name: std::collections::HashMap::from([(
                        "episode2.mkv".to_owned(),
                        vec!["episode2.mkv".to_owned()],
                    )]),
                },
            ),
            (
                bad_key.clone(),
                GuiAttachedMediaSearchRootIndex {
                    root_key: bad_key.clone(),
                    root_path: bad_root.clone(),
                    built_at_unix_ms: stale_built_at,
                    candidates_by_name: std::collections::HashMap::new(),
                },
            ),
        ]),
        roots_requiring_refresh: [good_key.clone(), bad_key.clone()].into_iter().collect(),
    });
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    result_tx
        .send(GuiAttachedMediaSearchBuildStatus::Completed(vec![
            GuiAttachedMediaSearchRootRefreshResult {
                root_key: good_key.clone(),
                index: Some(GuiAttachedMediaSearchRootIndex {
                    root_key: good_key.clone(),
                    root_path: good_root.clone(),
                    built_at_unix_ms: stale_built_at + 1,
                    candidates_by_name: std::collections::HashMap::from([(
                        "episode2.mkv".to_owned(),
                        vec!["episode2.mkv".to_owned()],
                    )]),
                }),
                error: None,
            },
            GuiAttachedMediaSearchRootRefreshResult {
                root_key: bad_key.clone(),
                index: None,
                error: Some("simulated refresh failure".to_owned()),
            },
        ]))
        .expect("partial-refresh result fixture should be queued");
    owner.pending_attached_media_resolution = Some(GuiPendingAttachedMediaResolution {
        roots: vec![good_key.clone(), bad_key.clone()],
        cancel_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        latest_progress: std::sync::Arc::new(std::sync::Mutex::new(None)),
        result_rx,
    });
    session_transport.push_inbound_protocol_lines([
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#
            .to_owned(),
        r#"{"Set":{"playlistChange":{"files":["episode2.mkv"],"user":"bob"}}}"#.to_owned(),
        r#"{"Set":{"playlistIndex":{"index":0,"user":"bob"}}}"#.to_owned(),
    ]);

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths,
        vec![selected_media_path.to_string_lossy().into_owned()]
    );
    assert!(
        owner.attached_media_search_next_retry_at.is_some(),
        "a failed per-root refresh should schedule a retry without dropping successful roots"
    );
    assert!(
        owner
            .attached_media_search_index
            .as_ref()
            .is_some_and(|index| index.root_indexes_by_key.contains_key(&bad_key)),
        "failed roots should keep their prior cached segment until a later refresh succeeds"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_projects_media_index_progress_into_shell_state() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.shared_playlist_enabled = true;
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let latest_progress = std::sync::Arc::new(std::sync::Mutex::new(None));
    owner.pending_attached_media_resolution = Some(GuiPendingAttachedMediaResolution {
        roots: vec!["c:/media/anime".to_owned(), "d:/archive".to_owned()],
        cancel_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        latest_progress: latest_progress.clone(),
        result_rx,
    });
    *latest_progress
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
        Some(GuiAttachedMediaSearchBuildProgress {
            total_roots: 2,
            completed_roots: 0,
            current_root_key: "c:/media/anime".to_owned(),
            current_root_path: PathBuf::from("C:/Media/Anime"),
            scanned_directories: 14,
            indexed_files: 2048,
        });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(state.media_index_status.active);
    assert_eq!(
        state.media_index_status.message.as_deref(),
        Some("Indexing media 1/2: 14 folders, 2048 files (Anime)")
    );

    result_tx
        .send(GuiAttachedMediaSearchBuildStatus::Cancelled)
        .expect("media-index cancel fixture should be queued");
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(!state.media_index_status.active);
    assert_eq!(state.media_index_status.message, None);
}

#[test]
fn gui_persisted_config_runtime_owner_coalesces_latest_media_index_progress_per_pump() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let latest_progress = std::sync::Arc::new(std::sync::Mutex::new(None));
    owner.pending_attached_media_resolution = Some(GuiPendingAttachedMediaResolution {
        roots: vec!["c:/media/anime".to_owned(), "d:/archive".to_owned()],
        cancel_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        latest_progress: latest_progress.clone(),
        result_rx,
    });

    *latest_progress
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
        Some(GuiAttachedMediaSearchBuildProgress {
            total_roots: 2,
            completed_roots: 0,
            current_root_key: "c:/media/anime".to_owned(),
            current_root_path: PathBuf::from("C:/Media/Anime"),
            scanned_directories: 32,
            indexed_files: 4096,
        });
    *latest_progress
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
        Some(GuiAttachedMediaSearchBuildProgress {
            total_roots: 2,
            completed_roots: 0,
            current_root_key: "c:/media/anime".to_owned(),
            current_root_path: PathBuf::from("C:/Media/Anime"),
            scanned_directories: 64,
            indexed_files: 8192,
        });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert_eq!(
        state.media_index_status.message.as_deref(),
        Some("Indexing media 1/2: 64 folders, 8192 files (Anime)")
    );

    *latest_progress
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
        Some(GuiAttachedMediaSearchBuildProgress {
            total_roots: 2,
            completed_roots: 1,
            current_root_key: "d:/archive".to_owned(),
            current_root_path: PathBuf::from("D:/Archive"),
            scanned_directories: 8,
            indexed_files: 512,
        });
    *latest_progress
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
        Some(GuiAttachedMediaSearchBuildProgress {
            total_roots: 2,
            completed_roots: 1,
            current_root_key: "d:/archive".to_owned(),
            current_root_path: PathBuf::from("D:/Archive"),
            scanned_directories: 12,
            indexed_files: 768,
        });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert_eq!(
        state.media_index_status.message.as_deref(),
        Some("Indexing media 2/2: 12 folders, 768 files (Archive)")
    );

    result_tx
        .send(GuiAttachedMediaSearchBuildStatus::Cancelled)
        .expect("media-index cancel fixture should be queued");
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(!state.media_index_status.active);
    assert_eq!(state.media_index_status.message, None);
}

#[test]
fn gui_persisted_config_runtime_owner_preserves_media_index_status_when_pending_search_build_completes_before_projection()
 {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let latest_progress = std::sync::Arc::new(std::sync::Mutex::new(Some(
        GuiAttachedMediaSearchBuildProgress {
            total_roots: 1,
            completed_roots: 0,
            current_root_key: "c:/media".to_owned(),
            current_root_path: PathBuf::from("C:/Media"),
            scanned_directories: 4,
            indexed_files: 32,
        },
    )));
    owner.pending_attached_media_resolution = Some(GuiPendingAttachedMediaResolution {
        roots: vec!["c:/media".to_owned()],
        cancel_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        latest_progress,
        result_rx,
    });
    state.pending_operation = Some(crate::app::GuiPendingOperationState {
        kind: GuiPendingOperationKind::SearchMissingMedia,
    });
    result_tx
        .send(GuiAttachedMediaSearchBuildStatus::Completed(Vec::new()))
        .expect("media-index completion fixture should be queued");

    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let actions = handle.drain_actions();

    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::ApplyGuiMediaIndexRuntimeSnapshot(snapshot)
                if snapshot.active
                    && snapshot.message.as_deref()
                        == Some("Indexing media 1/1: 4 folders, 32 files (Media)")
        )),
        "pending missing-media searches should still surface the last background index status even when the build completes before projection"
    );
}
