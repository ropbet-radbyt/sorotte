use super::*;

use crate::app::GuiClientCoreChatSessionRuntimeAdapter;

fn wait_for_media_match_remote_lookup(owner: &mut GuiPersistedConfigRuntimeOwner) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        owner.pump_media_match_remote_lookup_worker();
        if owner.media_match_remote_lookup_rx.is_none() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("timed out waiting for cached media-match remote lookup completion");
}

#[test]
fn gui_persisted_config_runtime_owner_opens_probable_media_match_candidate_for_selected_playlist() {
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

        fn open_file(&mut self, path: &str) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .opened_paths
                .push(path.to_owned());
            Ok(())
        }
    }

    #[derive(Debug, Clone)]
    struct MediaMatchPeerSessionRuntimeAdapter {
        peer_files: Vec<sorotte_client_core::ClientMediaMatchPeerFileState>,
    }

    impl GuiSessionRuntimeAdapter for MediaMatchPeerSessionRuntimeAdapter {
        fn current_room_media_match_peer_file_states(
            &self,
        ) -> Vec<sorotte_client_core::ClientMediaMatchPeerFileState> {
            self.peer_files.clone()
        }

        fn send_chat_message(&mut self, _message: String) -> Result<(), String> {
            Ok(())
        }

        fn connect_public_server(
            &mut self,
            _selected_server: Option<(String, String)>,
        ) -> Result<(), String> {
            Ok(())
        }

        fn refresh_public_servers(
            &mut self,
            _current_servers: Vec<(String, String)>,
            _language: Option<&str>,
        ) -> Result<Vec<(String, String)>, String> {
            Ok(Vec::new())
        }

        fn search_missing_media(
            &mut self,
            _directories: Vec<String>,
        ) -> Result<Option<String>, String> {
            Ok(None)
        }
    }

    fn media_match_record_for_file(
        path: &std::path::Path,
    ) -> sorotte_media_match::MediaFingerprintRecord {
        let metadata = std::fs::metadata(path).expect("test media should exist");
        let modified_unix_millis = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
            .unwrap_or(0);
        let mut record = sorotte_media_match::MediaFingerprintRecord {
            identity: sorotte_media_match::MediaFileIdentity::new(
                path,
                modified_unix_millis,
                metadata.len(),
            ),
            algorithm_version: sorotte_media_match::MEDIA_MATCH_ALGORITHM_VERSION,
            extraction_settings:
                sorotte_media_match::MediaExtractionSettings::sampled_fast_audio_index_v3(),
            duration_seconds: Some(900.0),
            container_fingerprint: format!("container:{}", path.display()),
            audio_anchors: Vec::new(),
            audio_error: None,
        };
        seed_media_match_strong_anchor_fixture(&mut record);
        record
    }

    fn remote_media_match_record(path: &str) -> sorotte_media_match::MediaFingerprintRecord {
        let mut record = sorotte_media_match::MediaFingerprintRecord {
            identity: sorotte_media_match::MediaFileIdentity::new(path, 1000, 2000),
            algorithm_version: sorotte_media_match::MEDIA_MATCH_ALGORITHM_VERSION,
            extraction_settings:
                sorotte_media_match::MediaExtractionSettings::sampled_fast_audio_index_v3(),
            duration_seconds: Some(900.0),
            container_fingerprint: format!("container:{path}"),
            audio_anchors: Vec::new(),
            audio_error: None,
        };
        seed_media_match_strong_anchor_fixture(&mut record);
        record
    }

    fn seed_media_match_strong_anchor_fixture(
        record: &mut sorotte_media_match::MediaFingerprintRecord,
    ) {
        record.audio_anchors = (0u32..24)
            .map(|index| sorotte_media_match::AudioAnchor {
                bucket: 100 + index,
                t_ms: 30_000 + (index * 30_000),
                weight: 4,
            })
            .collect();
    }

    let root = test_temp_root("playlist-media-match-alternate-encode");
    let config_path = root.join("sorotte.ini");
    let media_root = root.join("local-media");
    std::fs::create_dir_all(&media_root).expect("alternate-encode media root should be created");
    let local_file_name = "[ANE] Bakemonogatari - Ep04 [BDRip 1080p x264 FLAC].mkv";
    let local_media_path = media_root.join(local_file_name);
    std::fs::write(&local_media_path, b"alternate encode")
        .expect("alternate-encode fixture should be written");
    let unindexed_file_name = "[Decoy] Bakemonogatari - Ep04 [1080p].mkv";
    let unindexed_media_path = media_root.join(unindexed_file_name);
    std::fs::write(&unindexed_media_path, b"unindexed alternate encode")
        .expect("unindexed alternate fixture should be written");
    let remote_file_name = "[MTBB-Minis] Bakemonogatari - 04 [19103080].mkv";

    let local_record = media_match_record_for_file(&local_media_path);
    let unindexed_record = media_match_record_for_file(&unindexed_media_path);
    let mut cache = sorotte_media_match::MediaMatchCache::default();
    cache.insert(local_record);
    cache.insert(unindexed_record.clone());
    crate::app::media_match_support::save_media_match_cache_for_test(&root, &cache)
        .expect("media-match cache should be written");
    let mut remote_record = remote_media_match_record(remote_file_name);
    remote_record.container_fingerprint = unindexed_record.container_fingerprint;
    let remote_signature = sorotte_media_match::media_match_wire_value_from_records(
        std::slice::from_ref(&remote_record),
    )
    .expect("remote media-match signature should serialize");

    let media_root_key =
        crate::app::media_search_cache::normalized_media_search_root_key(&media_root);
    let mut root_index_candidates = std::collections::HashMap::new();
    root_index_candidates.insert(local_file_name.to_owned(), vec![local_file_name.to_owned()]);
    let mut root_indexes_by_key = std::collections::HashMap::new();
    root_indexes_by_key.insert(
        media_root_key.clone(),
        GuiAttachedMediaSearchRootIndex {
            root_key: media_root_key.clone(),
            root_path: media_root.clone(),
            built_at_unix_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_millis() as u64,
            candidates_by_name: root_index_candidates,
        },
    );

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path))
        .with_session_runtime(Box::new(MediaMatchPeerSessionRuntimeAdapter {
            peer_files: vec![sorotte_client_core::ClientMediaMatchPeerFileState {
                username: "remote".to_owned(),
                has_file: true,
                file_name: Some(remote_file_name.to_owned()),
                file_size: None,
                file_duration: None,
                media_match_signature: Some(remote_signature),
            }],
        }));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.attached_media_search_index = Some(GuiAttachedMediaSearchIndex {
        roots: vec![media_root_key],
        root_indexes_by_key,
        roots_requiring_refresh: std::collections::BTreeSet::new(),
    });
    owner.active_shared_playlist_index = Some(0);

    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![media_root.to_string_lossy().into_owned()]),
        media_match_fingerprinting_enabled: Some(true),
        media_match_wire_sharing_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(vec![remote_file_name.to_owned()], Some(0), false);

    assert_eq!(
        owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state),
        SelectedPlaylistMediaSyncOutcome::NoChange,
        "cached media-match candidate lookup should not block playlist sync"
    );
    wait_for_media_match_remote_lookup(&mut owner);
    assert_eq!(
        owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state),
        SelectedPlaylistMediaSyncOutcome::OpenedNewMedia
    );
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths,
        vec![sorotte_media_match::normalize_media_path(&local_media_path)],
        "probable sampled-fast media-match signatures should use indexed candidates instead of scanning every search-root file"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_retries_media_match_when_peer_signature_changes() {
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

        fn open_file(&mut self, path: &str) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .opened_paths
                .push(path.to_owned());
            Ok(())
        }
    }

    fn seeded_record_for_path(
        path: impl AsRef<std::path::Path>,
        anchor_seed: u32,
    ) -> sorotte_media_match::MediaFingerprintRecord {
        let path = path.as_ref();
        let metadata = std::fs::metadata(path).expect("test media should exist");
        let modified_unix_millis = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
            .unwrap_or(0);
        let mut record = sorotte_media_match::MediaFingerprintRecord {
            identity: sorotte_media_match::MediaFileIdentity::new(
                path,
                modified_unix_millis,
                metadata.len(),
            ),
            algorithm_version: sorotte_media_match::MEDIA_MATCH_ALGORITHM_VERSION,
            extraction_settings:
                sorotte_media_match::MediaExtractionSettings::sampled_fast_audio_index_v3(),
            duration_seconds: Some(900.0),
            container_fingerprint: format!("container:{}", path.display()),
            audio_anchors: Vec::new(),
            audio_error: None,
        };
        record.audio_anchors = (0u32..24)
            .map(|index| sorotte_media_match::AudioAnchor {
                bucket: anchor_seed + index,
                t_ms: 30_000 + (index * 30_000),
                weight: 4,
            })
            .collect();
        record
    }

    fn seeded_remote_record(
        path: &str,
        anchor_seed: u32,
    ) -> sorotte_media_match::MediaFingerprintRecord {
        let mut record = sorotte_media_match::MediaFingerprintRecord {
            identity: sorotte_media_match::MediaFileIdentity::new(path, 1000, 2000),
            algorithm_version: sorotte_media_match::MEDIA_MATCH_ALGORITHM_VERSION,
            extraction_settings:
                sorotte_media_match::MediaExtractionSettings::sampled_fast_audio_index_v3(),
            duration_seconds: Some(900.0),
            container_fingerprint: format!("container:{path}"),
            audio_anchors: Vec::new(),
            audio_error: None,
        };
        record.audio_anchors = (0u32..24)
            .map(|index| sorotte_media_match::AudioAnchor {
                bucket: anchor_seed + index,
                t_ms: 30_000 + (index * 30_000),
                weight: 4,
            })
            .collect();
        record
    }

    let root = test_temp_root("shared-playlist-media-match-peer-update-retry");
    let config_path = root.join("sorotte.ini");
    let media_root = root.join("local-media");
    std::fs::create_dir_all(&media_root)
        .expect("media-match peer-update fixture directory should be created");
    let current_media_path = media_root.join("episode1.mkv");
    let local_candidate_name = "local-alt-episode2.mkv";
    let local_candidate_path = media_root.join(local_candidate_name);
    std::fs::write(&current_media_path, b"current item")
        .expect("current media fixture should be written");
    std::fs::write(&local_candidate_path, b"alternate item")
        .expect("alternate media-match fixture should be written");

    let local_record = seeded_record_for_path(&local_candidate_path, 100);
    let mut cache = sorotte_media_match::MediaMatchCache::default();
    cache.insert(local_record);
    crate::app::media_match_support::save_media_match_cache_for_test(&root, &cache)
        .expect("media-match cache should be written");

    let stale_signature =
        sorotte_media_match::media_match_wire_value_from_records(&[seeded_remote_record(
            "episode1.mkv",
            900,
        )])
        .expect("stale media-match signature should serialize");
    let selected_signature =
        sorotte_media_match::media_match_wire_value_from_records(&[seeded_remote_record(
            "remote-episode2.mkv",
            100,
        )])
        .expect("selected media-match signature should serialize");

    let media_root_key =
        crate::app::media_search_cache::normalized_media_search_root_key(&media_root);
    let mut root_index_candidates = std::collections::HashMap::new();
    root_index_candidates.insert(
        local_candidate_name.to_owned(),
        vec![local_candidate_name.to_owned()],
    );
    let mut root_indexes_by_key = std::collections::HashMap::new();
    root_indexes_by_key.insert(
        media_root_key.clone(),
        GuiAttachedMediaSearchRootIndex {
            root_key: media_root_key.clone(),
            root_path: media_root.clone(),
            built_at_unix_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_millis() as u64,
            candidates_by_name: root_index_candidates,
        },
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
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path(current_media_path.to_string_lossy().into_owned()),
    );
    owner.attached_media_search_index = Some(GuiAttachedMediaSearchIndex {
        roots: vec![media_root_key],
        root_indexes_by_key,
        roots_requiring_refresh: std::collections::BTreeSet::new(),
    });

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![media_root.to_string_lossy().into_owned()]),
        media_match_fingerprinting_enabled: Some(true),
        media_match_wire_sharing_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    owner.media_match_runtime_snapshot.settings = state.media_match.settings.clone();
    owner.media_match_runtime_snapshot.health = crate::app::GuiMediaMatchToolHealth::Healthy;

    owner.sync_player_runtime_state(&handle, &state);
    let _ = handle.drain_actions();
    let _ = session_transport.drain_outbound_protocol_lines();

    session_transport.push_inbound_protocol_lines([
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"sharedPlaylists":true}}}"#
            .to_owned(),
        serde_json::json!({
            "Set": {
                "user": {
                    "bob": {
                        "room": { "name": "room1" },
                        "file": {
                            "name": "episode1.mkv",
                            "mediaMatch": stale_signature,
                        }
                    }
                }
            }
        })
        .to_string(),
        r#"{"Set":{"playlistChange":{"files":["episode1.mkv","remote-episode2.mkv"],"user":"bob"}}}"#
            .to_owned(),
        r#"{"Set":{"playlistIndex":{"index":1,"user":"bob"}}}"#.to_owned(),
    ]);
    owner.sync_player_runtime_state(&handle, &state);

    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths,
        Vec::<String>::new(),
        "stale peer metadata for the previous playlist item must not open an alternate match"
    );

    session_transport.push_inbound_protocol_line(
        serde_json::json!({
            "Set": {
                "user": {
                    "bob": {
                        "room": { "name": "room1" },
                        "file": {
                            "name": "remote-episode2.mkv",
                            "mediaMatch": selected_signature,
                        }
                    }
                }
            }
        })
        .to_string(),
    );
    owner.sync_player_runtime_state(&handle, &state);

    let retry_deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < retry_deadline {
        if !player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths
            .is_empty()
        {
            break;
        }
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .opened_paths,
        vec![sorotte_media_match::normalize_media_path(
            &local_candidate_path
        )],
        "updated peer media-match metadata for the selected playlist item should retrigger automatic resolution"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_prefers_local_media_for_plex_playlist_uri() {
    let root = test_temp_root("plex-playlist-uri-local-first");
    let selected_media_path = root.join("Episode 1.mkv");
    std::fs::write(&selected_media_path, b"test")
        .expect("Plex local-first fixture should be written");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.active_shared_playlist_index = Some(0);

    let plex_uri = "plex://machine-1/metadata/123?title=Episode%201&file=Episode%201.mkv";
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(vec![plex_uri.to_owned()], Some(0), false);

    let outcome = owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);
    let selected_media_path = selected_media_path.to_string_lossy().into_owned();

    assert_eq!(outcome, SelectedPlaylistMediaSyncOutcome::OpenedNewMedia);
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(selected_media_path.as_str())
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_uses_indexed_nested_local_media_for_plex_playlist_uri() {
    let root = test_temp_root("plex-playlist-uri-nested-index-local-first");
    let nested = root.join("Show Title").join("Season 01");
    std::fs::create_dir_all(&nested).expect("Plex nested local-first fixture should be created");
    let file_name = "[Group] Episode 01.mkv";
    let selected_media_path = nested.join(file_name);
    std::fs::write(&selected_media_path, b"test")
        .expect("Plex nested local-first fixture should be written");
    let relative_path = std::path::Path::new("Show Title")
        .join("Season 01")
        .join(file_name);
    let indexed_relative_path = if cfg!(windows) {
        relative_path.to_string_lossy().to_ascii_lowercase()
    } else {
        relative_path.to_string_lossy().into_owned()
    };
    let media_root_key = crate::app::media_search_cache::normalized_media_search_root_key(&root);
    let mut candidates_by_name = std::collections::HashMap::new();
    candidates_by_name.insert(
        GuiClientCoreChatSessionRuntimeAdapter::missing_media_file_name_lookup_key(
            "[group] episode 01.mkv",
        )
        .expect("Plex file name lookup key should be available"),
        vec![indexed_relative_path],
    );
    let mut root_indexes_by_key = std::collections::HashMap::new();
    root_indexes_by_key.insert(
        media_root_key.clone(),
        GuiAttachedMediaSearchRootIndex {
            root_key: media_root_key.clone(),
            root_path: root.clone(),
            built_at_unix_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_millis() as u64,
            candidates_by_name,
        },
    );

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.attached_media_search_index = Some(GuiAttachedMediaSearchIndex {
        roots: vec![media_root_key],
        root_indexes_by_key,
        roots_requiring_refresh: std::collections::BTreeSet::new(),
    });
    owner.active_shared_playlist_index = Some(0);

    let plex_uri =
        "plex://machine-1/metadata/123?title=Episode%2001&file=%5Bgroup%5D%20episode%2001.mkv";
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(vec![plex_uri.to_owned()], Some(0), false);

    let outcome = owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);
    let selected_media_path = selected_media_path.to_string_lossy().into_owned();

    assert_eq!(outcome, SelectedPlaylistMediaSyncOutcome::OpenedNewMedia);
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(selected_media_path.as_str()),
        "Plex playlist URI should use the indexed local file, preserving filesystem casing, before considering Plex streaming"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_opens_stale_cached_local_media_before_refresh_finishes() {
    let root = test_temp_root("stale-cache-opens-before-refresh");
    let config_path = root.join("sorotte.ini");
    let nested_directory = root.join("nested");
    std::fs::create_dir_all(&nested_directory)
        .expect("stale cache local fixture directory should be created");
    let selected_media_path = nested_directory.join("episode2.mkv");
    std::fs::write(&selected_media_path, b"test")
        .expect("stale cache local fixture should be written");
    write_persisted_media_search_root_index(
        &root,
        &root,
        0,
        &[("episode2.mkv", &["nested\\episode2.mkv"])],
    );

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path));
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.active_shared_playlist_index = Some(0);

    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(vec!["episode2.mkv".to_owned()], Some(0), false);

    let outcome = owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);
    let selected_media_path = selected_media_path.to_string_lossy().into_owned();

    assert_eq!(outcome, SelectedPlaylistMediaSyncOutcome::OpenedNewMedia);
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(selected_media_path.as_str()),
        "a stale but valid local index hit should open immediately"
    );
    assert!(
        owner.pending_attached_media_resolution.is_none(),
        "a valid cache hit should not start a full media-search refresh on the playback path"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_queues_plex_stream_resolution_for_automatic_playlist_sync() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.active_shared_playlist_index = Some(0);

    let plex_uri = "plex://machine-1/metadata/123?title=Episode%201&file=Episode%201.mkv";
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(vec![plex_uri.to_owned()], Some(0), false);

    let outcome = owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);

    assert_eq!(outcome, SelectedPlaylistMediaSyncOutcome::NoChange);
    assert!(
        owner.plex_stream_resolve_rx.is_some(),
        "automatic Plex stream resolution should run on a worker instead of blocking playlist sync"
    );
    assert!(owner.plex_stream_resolve_trigger_key.is_some());
    assert!(
        owner.player_local_file.is_none(),
        "the player should not open until the background Plex stream target is resolved"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_queues_plex_stream_while_media_search_indexes() {
    let root = test_temp_root("plex-stream-while-media-search-indexes");
    std::fs::create_dir_all(&root).expect("Plex stream index fixture root should be created");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.active_shared_playlist_index = Some(0);

    let plex_uri = "plex://machine-1/metadata/123?title=Episode%201&file=Episode%201.mkv";
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        plex_streaming_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(vec![plex_uri.to_owned()], Some(0), false);

    let outcome = owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);

    assert_eq!(outcome, SelectedPlaylistMediaSyncOutcome::NoChange);
    assert!(
        owner.pending_attached_media_resolution.is_some(),
        "local media search should continue in the background"
    );
    assert!(
        owner.plex_stream_resolve_rx.is_some(),
        "Plex stream resolution should be queued without waiting for local indexing to finish"
    );
    assert!(
        owner.player_local_file.is_none(),
        "the player should wait until the Plex stream worker resolves a playable URL"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_queues_selected_plex_stream_without_blocking_on_indexing() {
    let root = test_temp_root("selected-plex-stream-with-index-pending");
    std::fs::create_dir_all(&root).expect("selected Plex stream index fixture root should exist");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.active_shared_playlist_index = Some(0);

    let plex_uri = "plex://machine-1/metadata/123?title=Episode%201&file=Episode%201.mkv";
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        plex_streaming_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    let outcome = owner.open_selected_playlist_media_path_through_attached_player_impl(
        &state,
        &[plex_uri.to_owned()],
    );

    assert_eq!(outcome, SelectedPlaylistMediaSyncOutcome::NoChange);
    assert!(
        owner.pending_attached_media_resolution.is_some(),
        "explicit Plex selection should not cancel local indexing"
    );
    assert!(
        owner.plex_stream_resolve_rx.is_some(),
        "explicit Plex selection should use the background stream resolver instead of blocking the runtime"
    );
    assert!(
        owner.player_local_file.is_none(),
        "the player should not receive a raw plex:// URI while stream resolution is pending"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_retries_playlist_open_when_media_index_completes() {
    let root = test_temp_root("media-index-completion-retries-playlist-open");
    let nested = root.join("season-1");
    std::fs::create_dir_all(&nested).expect("media index completion fixture should be created");
    let selected_media_path = nested.join("episode2.mkv");
    std::fs::write(&selected_media_path, b"test")
        .expect("media index completion fixture should be written");
    let root_key = crate::app::media_search_cache::normalized_media_search_root_key(&root);
    let mut candidates_by_name = std::collections::HashMap::new();
    let relative_path = std::path::Path::new("season-1")
        .join("episode2.mkv")
        .to_string_lossy()
        .into_owned();
    candidates_by_name.insert(
        GuiClientCoreChatSessionRuntimeAdapter::missing_media_file_name_lookup_key("episode2.mkv")
            .expect("episode2 lookup key should be available"),
        vec![relative_path],
    );
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    result_tx
        .send(GuiAttachedMediaSearchBuildStatus::Completed(vec![
            GuiAttachedMediaSearchRootRefreshResult {
                root_key: root_key.clone(),
                index: Some(GuiAttachedMediaSearchRootIndex {
                    root_key: root_key.clone(),
                    root_path: root.clone(),
                    built_at_unix_ms: 1,
                    candidates_by_name,
                }),
                error: None,
            },
        ]))
        .expect("media index completion result should be queued");
    drop(result_tx);

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.active_shared_playlist_index = Some(0);
    owner.pending_attached_media_resolution = Some(GuiPendingAttachedMediaResolution {
        roots: vec![root_key],
        cancel_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        latest_progress: std::sync::Arc::new(std::sync::Mutex::new(None)),
        result_rx,
    });

    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(vec!["episode2.mkv".to_owned()], Some(0), false);
    state.main_window.active_playlist_index = Some(0);
    let handle = GuiQueuedRuntimeBridgeHandle::default();

    owner.sync_player_runtime_state(&handle, &state);

    assert!(
        owner.attached_media_search_index_revision > 0,
        "media index completion should advance the index revision"
    );
    let selected_media_path = selected_media_path.to_string_lossy().into_owned();
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(selected_media_path.as_str()),
        "media index completion should immediately retry the selected playlist item instead of waiting for a later retry tick"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_uses_media_match_inventory_for_exact_playlist_target() {
    let root = test_temp_root("media-match-inventory-exact-playlist-target");
    let config_path = root.join("sorotte.ini");
    let media_root = root.join("library");
    let nested_directory = media_root.join("season-1");
    std::fs::create_dir_all(&nested_directory)
        .expect("Media Match inventory exact fixture directory should be created");
    let selected_media_path = nested_directory.join("episode2.mkv");
    std::fs::write(&selected_media_path, b"test")
        .expect("Media Match inventory exact fixture should be written");

    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![media_root.to_string_lossy().into_owned()]),
        media_matching_plugin_enabled: Some(true),
        media_match_fingerprinting_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(vec!["episode2.mkv".to_owned()], Some(0), false);

    let extraction_settings =
        sorotte_media_match::MediaExtractionSettings::sampled_fast_audio_index_v3();
    crate::app::media_match_support::rebuild_persisted_media_match_index_with_extraction_settings_and_cancel(
        &root,
        std::slice::from_ref(&media_root),
        None,
        &state.media_match.settings,
        &extraction_settings,
        None,
        |_| {},
    )
    .expect("Media Match inventory should be persisted without fingerprint extraction");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path));
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.active_shared_playlist_index = Some(0);

    let outcome = owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);
    let selected_media_path = sorotte_media_match::normalize_media_path(&selected_media_path);

    assert_eq!(outcome, SelectedPlaylistMediaSyncOutcome::OpenedNewMedia);
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(selected_media_path.as_str())
    );
    assert!(
        owner.pending_attached_media_resolution.is_none(),
        "exact Media Match inventory resolution should not start a media-search build"
    );
    assert!(
        owner
            .attached_media_search_index
            .as_ref()
            .is_some_and(|index| !index.roots_requiring_refresh.is_empty()),
        "exact Media Match inventory resolution may initialize the filename cache, but should leave refresh work deferred"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_does_not_use_media_match_inventory_when_plugin_disabled() {
    let root = test_temp_root("media-match-inventory-plugin-disabled");
    let config_path = root.join("sorotte.ini");
    let media_root = root.join("library");
    let nested_directory = media_root.join("season-1");
    std::fs::create_dir_all(&nested_directory)
        .expect("Media Match disabled fixture directory should be created");
    let selected_media_path = nested_directory.join("episode2.mkv");
    std::fs::write(&selected_media_path, b"test")
        .expect("Media Match disabled fixture should be written");

    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![media_root.to_string_lossy().into_owned()]),
        media_matching_plugin_enabled: Some(false),
        media_match_fingerprinting_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(vec!["episode2.mkv".to_owned()], Some(0), false);

    let extraction_settings =
        sorotte_media_match::MediaExtractionSettings::sampled_fast_audio_index_v3();
    crate::app::media_match_support::rebuild_persisted_media_match_index_with_extraction_settings_and_cancel(
        &root,
        std::slice::from_ref(&media_root),
        None,
        &state.media_match.settings,
        &extraction_settings,
        None,
        |_| {},
    )
    .expect("Media Match inventory should be persisted without fingerprint extraction");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path));
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.active_shared_playlist_index = Some(0);

    let outcome = owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);

    assert_eq!(outcome, SelectedPlaylistMediaSyncOutcome::NoChange);
    assert!(
        owner.player_local_file.is_none(),
        "disabled Media Matching must not resolve playlist media from the inventory cache"
    );
    assert!(
        owner.pending_attached_media_resolution.is_some(),
        "with Media Matching disabled, the normal filename index remains the only local fallback"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_prefers_media_search_casing_over_media_match_inventory() {
    let root = test_temp_root("media-search-casing-over-media-match-inventory");
    let config_path = root.join("sorotte.ini");
    let media_root = root.join("Library");
    let nested_directory = media_root.join("Season-1");
    std::fs::create_dir_all(&nested_directory)
        .expect("Media Match casing fixture directory should be created");
    let selected_media_path = nested_directory.join("Episode2.mkv");
    std::fs::write(&selected_media_path, b"test")
        .expect("Media Match casing fixture should be written");
    let built_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_millis() as u64;
    write_persisted_media_search_root_index(
        &root,
        &media_root,
        built_at,
        &[("episode2.mkv", &["Season-1\\Episode2.mkv"])],
    );

    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![media_root.to_string_lossy().into_owned()]),
        media_matching_plugin_enabled: Some(true),
        media_match_fingerprinting_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(vec!["Episode2.mkv".to_owned()], Some(0), false);

    let extraction_settings =
        sorotte_media_match::MediaExtractionSettings::sampled_fast_audio_index_v3();
    crate::app::media_match_support::rebuild_persisted_media_match_index_with_extraction_settings_and_cancel(
        &root,
        std::slice::from_ref(&media_root),
        None,
        &state.media_match.settings,
        &extraction_settings,
        None,
        |_| {},
    )
    .expect("Media Match inventory should be persisted without fingerprint extraction");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path));
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.active_shared_playlist_index = Some(0);

    let outcome = owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);
    let selected_media_path = selected_media_path.to_string_lossy().into_owned();

    assert_eq!(outcome, SelectedPlaylistMediaSyncOutcome::OpenedNewMedia);
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(selected_media_path.as_str()),
        "the filename index should preserve filesystem casing before Media Match inventory is considered"
    );
    assert!(
        owner.pending_attached_media_resolution.is_none(),
        "a valid filename-index hit should not trigger a full media-search refresh"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_queues_media_match_remote_lookup_while_media_search_indexes()
{
    #[derive(Debug, Clone)]
    struct MediaMatchPeerSessionRuntimeAdapter {
        peer_files: Vec<sorotte_client_core::ClientMediaMatchPeerFileState>,
    }

    impl GuiSessionRuntimeAdapter for MediaMatchPeerSessionRuntimeAdapter {
        fn current_room_media_match_peer_file_states(
            &self,
        ) -> Vec<sorotte_client_core::ClientMediaMatchPeerFileState> {
            self.peer_files.clone()
        }

        fn send_chat_message(&mut self, _message: String) -> Result<(), String> {
            Ok(())
        }

        fn connect_public_server(
            &mut self,
            _selected_server: Option<(String, String)>,
        ) -> Result<(), String> {
            Ok(())
        }

        fn refresh_public_servers(
            &mut self,
            _current_servers: Vec<(String, String)>,
            _language: Option<&str>,
        ) -> Result<Vec<(String, String)>, String> {
            Ok(Vec::new())
        }

        fn search_missing_media(
            &mut self,
            _directories: Vec<String>,
        ) -> Result<Option<String>, String> {
            Ok(None)
        }
    }

    let root = test_temp_root("media-match-remote-queued-during-index");
    let config_path = root.join("sorotte.ini");
    let media_root = root.join("library");
    std::fs::create_dir_all(&media_root)
        .expect("Media Match remote lookup scheduling fixture directory should be created");
    let playlist_target = "peer-only-episode.mkv";
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path))
        .with_session_runtime(Box::new(MediaMatchPeerSessionRuntimeAdapter {
            peer_files: vec![sorotte_client_core::ClientMediaMatchPeerFileState {
                username: "remote".to_owned(),
                has_file: true,
                file_name: Some(playlist_target.to_owned()),
                file_size: None,
                file_duration: None,
                media_match_signature: Some(serde_json::json!({
                    "algorithm": "test",
                    "records": [],
                })),
            }],
        }));
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.active_shared_playlist_index = Some(0);

    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![media_root.to_string_lossy().into_owned()]),
        media_match_fingerprinting_enabled: Some(true),
        media_match_wire_sharing_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(vec![playlist_target.to_owned()], Some(0), false);

    let outcome = owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);

    assert_eq!(outcome, SelectedPlaylistMediaSyncOutcome::NoChange);
    assert!(
        owner.pending_attached_media_resolution.is_some(),
        "missing local media should start the media-search index worker"
    );
    assert!(
        owner.media_match_remote_lookup_rx.is_some(),
        "Media Match remote lookup should be queued without waiting for media-search indexing to finish"
    );
    assert!(owner.media_match_remote_lookup_trigger_key.is_some());

    let _ = std::fs::remove_dir_all(&root);
}

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

        fn open_file(&mut self, path: &str) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .opened_paths
                .push(path.to_owned());
            Ok(())
        }
    }

    let root = test_temp_root("persisted-shared-playlist-warm-start");
    let config_path = root.join("sorotte.ini");
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
fn gui_persisted_config_runtime_owner_resolves_from_stale_persisted_cache_without_background_refresh()
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

        fn open_file(&mut self, path: &str) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .opened_paths
                .push(path.to_owned());
            Ok(())
        }
    }

    let root = test_temp_root("persisted-shared-playlist-stale-cache");
    let config_path = root.join("sorotte.ini");
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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

    assert!(
        owner.pending_attached_media_resolution.is_none(),
        "a stale cache hit should not start a full media-search refresh on the playback path"
    );
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
            .is_some_and(|built_at| built_at == stale_built_at),
        "valid stale persisted cache entries should remain warm-start data until a miss requires refresh"
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

        fn open_file(&mut self, path: &str) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .opened_paths
                .push(path.to_owned());
            Ok(())
        }
    }

    let root = test_temp_root("persisted-shared-playlist-duplicate-ranking");
    let config_path = root.join("sorotte.ini");
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
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path(preferred_current_path.to_string_lossy().into_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
fn gui_persisted_config_runtime_owner_does_not_match_path_bearing_targets_by_basename() {
    let root = test_temp_root("path-bearing-current-player-match");
    let current_path = root.join("season-1").join("episode2.mkv");
    let target_path = root.join("season-2").join("episode2.mkv");
    std::fs::create_dir_all(
        current_path
            .parent()
            .expect("current path should have parent"),
    )
    .expect("current fixture directory should be created");
    std::fs::create_dir_all(
        target_path
            .parent()
            .expect("target path should have parent"),
    )
    .expect("target fixture directory should be created");
    std::fs::write(&current_path, b"current").expect("current fixture should be written");
    std::fs::write(&target_path, b"target").expect("target fixture should be written");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode2.mkv")
            .with_path(current_path.to_string_lossy().into_owned()),
    );

    assert!(
        owner.current_player_matches_media_target("episode2.mkv"),
        "bare filename targets can still match by basename"
    );
    assert!(
        owner.current_player_matches_media_target(&current_path.to_string_lossy()),
        "resolved absolute path targets should match by normalized path"
    );
    assert!(
        !owner.current_player_matches_media_target("season-2/episode2.mkv"),
        "path-bearing relative targets should not match a different file by basename"
    );
    assert!(
        !owner.current_player_matches_media_target(&target_path.to_string_lossy()),
        "path-bearing absolute targets should not match a different file by basename"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_does_not_add_nested_current_player_root_when_configured_root_covers_it()
 {
    let root = test_temp_root("covered-current-player-search-root");
    let media_root = root.join("anime shows");
    let current_directory = media_root.join("bakemonogatari");
    let current_path = current_directory.join("[mtbb-minis] bakemonogatari - 08.mkv");
    std::fs::create_dir_all(&current_directory)
        .expect("current player fixture directory should be created");
    std::fs::write(&current_path, b"current").expect("current player fixture should be written");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("[mtbb-minis] bakemonogatari - 08.mkv")
            .with_path(current_path.to_string_lossy().into_owned()),
    );
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        media_search_directories: Some(vec![media_root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });

    let roots = owner.automatic_media_search_roots(&state);
    assert_eq!(
        roots,
        vec![media_root.clone()],
        "a current player file under a configured media root should not add a nested second scan root"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_uses_current_player_parent_as_search_root_without_configured_roots()
 {
    let root = test_temp_root("current-player-fallback-search-root");
    let current_directory = root.join("bakemonogatari");
    let current_path = current_directory.join("[mtbb-minis] bakemonogatari - 08.mkv");
    std::fs::create_dir_all(&current_directory)
        .expect("current player fixture directory should be created");
    std::fs::write(&current_path, b"current").expect("current player fixture should be written");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("[mtbb-minis] bakemonogatari - 08.mkv")
            .with_path(current_path.to_string_lossy().into_owned()),
    );
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    let roots = owner.automatic_media_search_roots(&state);
    assert_eq!(
        roots,
        vec![current_directory.clone()],
        "the current player folder remains a fallback when no media-search roots are configured"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_prefers_exact_cached_relative_path_for_path_bearing_targets()
{
    let root = test_temp_root("path-bearing-cache-ranking");
    let direct_path = root.join("episode2.mkv");
    let exact_directory = root.join("season-1");
    let exact_path = exact_directory.join("episode2.mkv");
    std::fs::create_dir_all(&exact_directory).expect("exact fixture directory should be created");
    std::fs::write(&direct_path, b"direct").expect("direct duplicate fixture should be written");
    std::fs::write(&exact_path, b"exact").expect("exact duplicate fixture should be written");

    let root_key = crate::app::media_search_cache::normalized_media_search_root_key(&root);
    let owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let index = GuiAttachedMediaSearchIndex {
        roots: vec![root_key.clone()],
        root_indexes_by_key: std::collections::HashMap::from([(
            root_key.clone(),
            GuiAttachedMediaSearchRootIndex {
                root_key,
                root_path: root.clone(),
                built_at_unix_ms: 1234,
                candidates_by_name: std::collections::HashMap::from([(
                    "episode2.mkv".to_owned(),
                    vec![
                        "episode2.mkv".to_owned(),
                        "season-1/episode2.mkv".to_owned(),
                    ],
                )]),
            },
        )]),
        roots_requiring_refresh: std::collections::BTreeSet::new(),
    };

    let resolved = owner
        .cached_missing_media_target_path(&index, "season-1/episode2.mkv")
        .map(|path| path.replace('\\', "/"));
    assert_eq!(
        resolved,
        Some(exact_path.to_string_lossy().replace('\\', "/"))
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

        fn open_file(&mut self, path: &str) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .opened_paths
                .push(path.to_owned());
            Ok(())
        }
    }

    let root = test_temp_root("persisted-shared-playlist-partial-refresh");
    let config_path = root.join("sorotte.ini");
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
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
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
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
