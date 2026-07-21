use super::*;

use sorotte_plex::{PlexServerConnectionKind, discovery::PlexServerConnection};

use crate::app::runtime_owner::player::PlaylistResolutionAttemptState;
use crate::app::runtime_owner::{
    GuiPlexStreamResolveOutcome, GuiPlexStreamResolveWorkerResult, GuiUserMediaTargetResolution,
    GuiUserMediaTargetResolutionSource,
};
use crate::app::{
    GuiClientCoreChatSessionRuntimeAdapter, GuiMediaSourceProviderId, GuiPlaylistDefaultSourceId,
    GuiPlaylistSourcePolicy, GuiPlaylistSourceSelectionOrigin, GuiPlaylistSourceState,
    GuiPlaylistSourceStatus,
};

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

fn test_plex_stream_target(
    file_name: &str,
    rating_key: &str,
) -> (
    sorotte_plex::PlexStreamTarget,
    sorotte_player_api::LocalFileUpdate,
) {
    let playlist_uri = sorotte_plex::PlexPlaylistUri {
        machine_identifier: "machine-1".to_owned(),
        rating_key: rating_key.to_owned(),
        title: Some(file_name.to_owned()),
        file_name: Some(file_name.to_owned()),
        duration_millis: Some(90_000),
        size_bytes: Some(123_456),
        media_type: Some(sorotte_plex::PlexMediaType::Episode),
    };
    let logical_file = sorotte_player_api::LocalFileUpdate::new(file_name)
        .with_path(sorotte_plex::format_plex_playlist_uri(&playlist_uri))
        .with_duration_seconds(90.0)
        .with_size_bytes(123_456);
    let stream_target = sorotte_plex::PlexStreamTarget {
        playlist_uri,
        matched_item: sorotte_plex::PlexMatchedItem {
            rating_key: rating_key.to_owned(),
            title: file_name.to_owned(),
            media_type: sorotte_plex::PlexMediaType::Episode,
            duration_millis: Some(90_000),
        },
        logical_file: logical_file.clone(),
        playback_url: sorotte_plex::SecretPlexPlaybackUrl::new(format!(
            "http://127.0.0.1:32400/library/parts/{rating_key}/file.mkv?X-Plex-Token=secret-token"
        )),
    };
    (stream_target, logical_file)
}

#[derive(Debug, Clone, Copy)]
enum FirstOpenFailureMode {
    Synchronous,
    Tracked,
}

struct FailFirstOpenPlayerAdapter {
    mode: FirstOpenFailureMode,
    opened_paths: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    next_command_id: u64,
    command_progress: std::collections::VecDeque<sorotte_player_api::PlayerCommandProgress>,
    media_load_outcomes: std::collections::VecDeque<sorotte_player_api::PlayerMediaLoadOutcome>,
    local_file_updates: std::collections::VecDeque<sorotte_player_api::LocalFileUpdate>,
}

impl FailFirstOpenPlayerAdapter {
    fn new(mode: FirstOpenFailureMode) -> (Self, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let opened_paths = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        (
            Self {
                mode,
                opened_paths: opened_paths.clone(),
                next_command_id: 1,
                command_progress: std::collections::VecDeque::new(),
                media_load_outcomes: std::collections::VecDeque::new(),
                local_file_updates: std::collections::VecDeque::new(),
            },
            opened_paths,
        )
    }

    fn record_open(&self, path: &str) -> usize {
        let mut opened = self
            .opened_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        opened.push(path.to_owned());
        opened.len()
    }
}

impl PlayerAdapter for FailFirstOpenPlayerAdapter {
    fn name(&self) -> &'static str {
        "fail-first-open"
    }

    fn execute_tracked(
        &mut self,
        command: sorotte_player_api::PlayerCommand,
    ) -> Result<sorotte_player_api::PlayerCommandId, sorotte_player_api::PlayerError> {
        if matches!(self.mode, FirstOpenFailureMode::Synchronous) {
            return Err(sorotte_player_api::PlayerError::Unsupported(
                "execute_tracked",
            ));
        }
        let sorotte_player_api::PlayerCommand::OpenFile(path) = command else {
            return Err(sorotte_player_api::PlayerError::Unsupported("test command"));
        };
        let open_number = self.record_open(&path);
        let command_id = sorotte_player_api::PlayerCommandId::new(self.next_command_id);
        let generation = sorotte_player_api::PlayerMediaGeneration::new(self.next_command_id);
        self.next_command_id += 1;
        self.command_progress
            .push_back(sorotte_player_api::PlayerCommandProgress::accepted(
                command_id,
                Some(generation),
                None,
            ));
        if open_number == 1 {
            self.media_load_outcomes.push_back(
                sorotte_player_api::PlayerMediaLoadOutcome::success(
                    path.clone(),
                    Some(path.clone()),
                ),
            );
            self.local_file_updates.push_back(
                sorotte_player_api::LocalFileUpdate::new("episode.mkv").with_path(path.clone()),
            );
        }
        let result = if open_number == 1 {
            sorotte_player_api::PlayerCommandResult::Failed(
                sorotte_player_api::PlayerCommandFailureKind::Unknown,
            )
        } else {
            sorotte_player_api::PlayerCommandResult::Completed
        };
        self.command_progress
            .push_back(sorotte_player_api::PlayerCommandProgress::finished(
                command_id,
                Some(generation),
                None,
                None,
                result,
            ));
        Ok(command_id)
    }

    fn open_file(&mut self, path: &str) -> Result<(), sorotte_player_api::PlayerError> {
        let open_number = self.record_open(path);
        if open_number == 1 {
            return Err(sorotte_player_api::PlayerError::OperationFailed(
                "simulated first candidate failure".to_owned(),
            ));
        }
        self.media_load_outcomes
            .push_back(sorotte_player_api::PlayerMediaLoadOutcome::success(
                path,
                Some(path.to_owned()),
            ));
        Ok(())
    }

    fn take_command_progress(&mut self) -> Option<sorotte_player_api::PlayerCommandProgress> {
        self.command_progress.pop_front()
    }

    fn take_media_load_outcome(&mut self) -> Option<sorotte_player_api::PlayerMediaLoadOutcome> {
        self.media_load_outcomes.pop_front()
    }

    fn take_local_file_update(&mut self) -> Option<sorotte_player_api::LocalFileUpdate> {
        self.local_file_updates.pop_front()
    }
}

fn assert_failed_local_candidate_falls_back_to_plex(mode: FirstOpenFailureMode) {
    let root = test_temp_root(match mode {
        FirstOpenFailureMode::Synchronous => "sync-local-open-failure-fallback",
        FirstOpenFailureMode::Tracked => "tracked-local-open-failure-fallback",
    });
    let local_path = root.join("episode.mkv");
    std::fs::write(&local_path, b"broken local fixture")
        .expect("local fallback fixture should be written");
    let local_path = local_path.to_string_lossy().into_owned();

    let (adapter, opened_paths) = FailFirstOpenPlayerAdapter::new(mode);
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(adapter)));
    owner.active_shared_playlist_index = Some(0);
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        plex_plugin_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("machine-1".to_owned()),
        plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(vec![local_path.clone()], Some(0), false);

    let first_outcome = owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);
    match mode {
        FirstOpenFailureMode::Synchronous => {
            assert_eq!(first_outcome, SelectedPlaylistMediaSyncOutcome::NoChange);
            assert_eq!(
                owner.playlist_resolution_attempt.as_ref().unwrap().state,
                PlaylistResolutionAttemptState::Failed
            );
            assert!(
                owner.player_local_file.is_none() && !owner.player_local_file_placeholder,
                "a file-loaded success/local observation must remain provisional and be cleared by the matching terminal failure"
            );
            assert_eq!(
                owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state),
                SelectedPlaylistMediaSyncOutcome::NoChange
            );
        }
        FirstOpenFailureMode::Tracked => {
            assert_eq!(
                first_outcome,
                SelectedPlaylistMediaSyncOutcome::StartedLoading
            );
            owner.refresh_player_state_impl();
            assert_eq!(
                owner.playlist_resolution_attempt.as_ref().unwrap().state,
                PlaylistResolutionAttemptState::Failed
            );
            assert_eq!(
                owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state),
                SelectedPlaylistMediaSyncOutcome::NoChange
            );
        }
    }
    assert!(
        owner.plex_stream_resolve_rx.is_some(),
        "a failed local candidate should advance to the Plex worker"
    );

    let (stream_target, _) = test_plex_stream_target("episode.mkv", "fallback-1");
    let playback_url = stream_target.playback_url.as_str().to_owned();
    let trigger_key = owner
        .plex_stream_resolve_trigger_key
        .clone()
        .expect("Plex fallback should retain a trigger key");
    let operation_context = owner
        .plex_stream_resolve_context
        .clone()
        .expect("Plex fallback should retain an operation context");
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    result_tx
        .send(GuiPlexStreamResolveWorkerResult {
            operation_context,
            trigger_key,
            result: Ok(GuiPlexStreamResolveOutcome {
                stream_target: Ok(Some(stream_target)),
                cache: sorotte_plex::PlexMatchCache::default(),
            }),
            staged_cache_write: None,
        })
        .expect("Plex fallback result should queue");
    owner.plex_stream_resolve_rx = Some(result_rx);
    owner.plex_stream_resolve_result = None;
    assert!(owner.pump_plex_stream_resolution_worker(&state));

    assert_eq!(
        owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state),
        SelectedPlaylistMediaSyncOutcome::StartedLoading
    );
    assert_eq!(
        *opened_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        vec![local_path.clone(), playback_url],
        "the exact failed local path must be excluded instead of retried"
    );
    let attempt = owner.playlist_resolution_attempt.as_ref().unwrap();
    assert_eq!(attempt.candidate_failures.len(), 1);
    assert_eq!(
        attempt.candidate_provider,
        Some(GuiMediaSourceProviderId::plex_stream())
    );

    owner.refresh_player_state_impl();
    assert_eq!(
        owner.playlist_resolution_attempt.as_ref().unwrap().state,
        PlaylistResolutionAttemptState::Active
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_falls_back_after_synchronous_local_open_failure() {
    assert_failed_local_candidate_falls_back_to_plex(FirstOpenFailureMode::Synchronous);
}

#[test]
fn gui_persisted_config_runtime_owner_falls_back_after_tracked_local_open_failure() {
    assert_failed_local_candidate_falls_back_to_plex(FirstOpenFailureMode::Tracked);
}

#[test]
fn gui_persisted_config_runtime_owner_retries_repaired_same_path_after_file_evidence_changes() {
    let root = test_temp_root("repaired-local-candidate-retry");
    let local_path = root.join("episode.mkv");
    std::fs::write(&local_path, b"broken").expect("initial failure fixture should be written");
    let local_path = local_path.to_string_lossy().into_owned();

    let (adapter, opened_paths) =
        FailFirstOpenPlayerAdapter::new(FirstOpenFailureMode::Synchronous);
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(adapter)));
    owner.active_shared_playlist_index = Some(0);
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(vec![local_path.clone()], Some(0), false);

    assert_eq!(
        owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state),
        SelectedPlaylistMediaSyncOutcome::NoChange
    );
    assert_eq!(
        owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state),
        SelectedPlaylistMediaSyncOutcome::NoChange,
        "the exact failed path must remain excluded during the immediate fallback pass"
    );
    assert_eq!(
        opened_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_slice(),
        std::slice::from_ref(&local_path)
    );

    std::fs::write(
        &local_path,
        b"repaired after first open with changed file evidence",
    )
    .expect("candidate repair should update file evidence");
    assert_eq!(
        owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state),
        SelectedPlaylistMediaSyncOutcome::StartedLoading,
        "the same path must become eligible after its file evidence changes"
    );
    assert_eq!(
        opened_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_slice(),
        &[local_path.clone(), local_path.clone()]
    );
    owner.refresh_player_state_impl();
    assert_eq!(
        owner.playlist_resolution_attempt.as_ref().unwrap().state,
        PlaylistResolutionAttemptState::Active
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_explicit_same_provider_request_retries_failed_candidate() {
    let root = test_temp_root("explicit-local-candidate-retry");
    let local_path = root.join("episode.mkv");
    std::fs::write(&local_path, b"candidate remains unchanged")
        .expect("explicit retry fixture should be written");
    let local_path = local_path.to_string_lossy().into_owned();

    let (adapter, opened_paths) =
        FailFirstOpenPlayerAdapter::new(FirstOpenFailureMode::Synchronous);
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(adapter)));
    owner.active_shared_playlist_index = Some(0);
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(vec![local_path.clone()], Some(0), false);

    assert_eq!(
        owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state),
        SelectedPlaylistMediaSyncOutcome::NoChange
    );
    assert_eq!(
        owner
            .playlist_resolution_attempt
            .as_ref()
            .unwrap()
            .candidate_failures
            .len(),
        1
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    assert!(owner.handle_resolve_playlist_source_request(
        &handle,
        &mut state,
        0,
        GuiMediaSourceProviderId::local(),
    ));
    assert_eq!(
        opened_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_slice(),
        &[local_path.clone(), local_path]
    );
    assert_eq!(
        owner.playlist_resolution_attempt.as_ref().unwrap().state,
        PlaylistResolutionAttemptState::Loading
    );

    let _ = std::fs::remove_dir_all(&root);
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
                media_match_signature: Some(
                    sorotte_media_match::media_match_wire_signature_from_value(&remote_signature)
                        .expect("remote signature should validate"),
                ),
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
        SelectedPlaylistMediaSyncOutcome::StartedLoading
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

    assert_eq!(outcome, SelectedPlaylistMediaSyncOutcome::StartedLoading);
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(selected_media_path.as_str())
    );
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let source_state = &state.main_window.playlist[0].source_state;
    assert_eq!(source_state.policy, GuiPlaylistSourcePolicy::Automatic);
    assert_eq!(source_state.preferred_provider_id(), None);
    assert_eq!(
        source_state.resolved_provider_id.as_ref(),
        Some(&GuiMediaSourceProviderId::local()),
        "a Plex URI resolved from disk must display Local without changing Automatic policy"
    );
    assert_eq!(
        source_state.current_provider_id,
        GuiMediaSourceProviderId::local()
    );
    assert_eq!(source_state.status, GuiPlaylistSourceStatus::Active);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_prefers_unique_plex_filename_over_ambiguous_title_in_quick_search()
 {
    let root = test_temp_root("plex-alias-priority-quick");
    let first_root = root.join("library-a");
    let second_root = root.join("library-b");
    std::fs::create_dir_all(&first_root).expect("first Plex alias root should be created");
    std::fs::create_dir_all(&second_root).expect("second Plex alias root should be created");
    let expected_path = first_root.join("Show.S01E01.mkv");
    std::fs::write(&expected_path, b"episode").expect("exact Plex filename should be written");
    std::fs::write(first_root.join("Pilot"), b"first")
        .expect("first ambiguous Plex title should be written");
    std::fs::write(second_root.join("Pilot"), b"second")
        .expect("second ambiguous Plex title should be written");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        media_search_directories: Some(vec![
            first_root.to_string_lossy().into_owned(),
            second_root.to_string_lossy().into_owned(),
        ]),
        ..StoredClientSettingsMvp::default()
    });
    let plex_uri = "plex://machine-1/metadata/123?title=Pilot&file=Show.S01E01.mkv";

    let resolution = owner
        .resolve_main_window_user_media_target(&state, plex_uri)
        .expect("quick Plex alias resolution should succeed");

    assert_eq!(
        resolution,
        GuiUserMediaTargetResolution::Resolved {
            path: expected_path.to_string_lossy().into_owned(),
            source: GuiUserMediaTargetResolutionSource::QuickLocal,
        },
        "the exact Plex filename must be evaluated before an ambiguous human-readable title"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_prefers_unique_plex_filename_over_ambiguous_title_in_index() {
    let root = test_temp_root("plex-alias-priority-index");
    let first_directory = root.join("show-a");
    let second_directory = root.join("show-b");
    std::fs::create_dir_all(&first_directory)
        .expect("first indexed Plex alias directory should be created");
    std::fs::create_dir_all(&second_directory)
        .expect("second indexed Plex alias directory should be created");
    let expected_path = first_directory.join("Show.S01E01.mkv");
    std::fs::write(&expected_path, b"episode")
        .expect("indexed exact Plex filename should be written");
    std::fs::write(first_directory.join("Pilot"), b"first")
        .expect("first indexed ambiguous title should be written");
    std::fs::write(second_directory.join("Pilot"), b"second")
        .expect("second indexed ambiguous title should be written");

    let root_key = crate::app::media_search_cache::normalized_media_search_root_key(&root);
    let candidates_by_name = std::collections::HashMap::from([
        (
            GuiClientCoreChatSessionRuntimeAdapter::missing_media_file_name_lookup_key(
                "Show.S01E01.mkv",
            )
            .expect("exact Plex filename key should be available"),
            vec!["show-a/Show.S01E01.mkv".to_owned()],
        ),
        (
            GuiClientCoreChatSessionRuntimeAdapter::missing_media_file_name_lookup_key("Pilot")
                .expect("Plex title key should be available"),
            vec!["show-a/Pilot".to_owned(), "show-b/Pilot".to_owned()],
        ),
    ]);
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.attached_media_search_index = Some(GuiAttachedMediaSearchIndex {
        roots: vec![root_key.clone()],
        root_indexes_by_key: std::collections::HashMap::from([(
            root_key.clone(),
            GuiAttachedMediaSearchRootIndex {
                root_key,
                root_path: root.clone(),
                built_at_unix_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("system time should be after unix epoch")
                    .as_millis() as u64,
                candidates_by_name,
            },
        )]),
        roots_requiring_refresh: std::collections::BTreeSet::new(),
    });
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    let plex_uri = "plex://machine-1/metadata/123?title=Pilot&file=Show.S01E01.mkv";

    let resolution = owner
        .resolve_main_window_user_media_target(&state, plex_uri)
        .expect("indexed Plex alias resolution should succeed");

    assert_eq!(
        resolution,
        GuiUserMediaTargetResolution::Resolved {
            path: expected_path.to_string_lossy().into_owned(),
            source: GuiUserMediaTargetResolutionSource::MediaSearchIndex,
        },
        "an ambiguous title must not terminate indexed lookup before the stronger filename"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_exhausts_indexed_filename_before_quick_title_ambiguity() {
    let root = test_temp_root("plex-alias-priority-cross-layer-index");
    let first_root = root.join("library-a");
    let second_root = root.join("library-b");
    let indexed_directory = first_root.join("nested-show");
    std::fs::create_dir_all(&indexed_directory)
        .expect("nested exact-filename directory should be created");
    std::fs::create_dir_all(&second_root).expect("second quick-title root should be created");
    let expected_path = indexed_directory.join("Show.S01E01.mkv");
    std::fs::write(&expected_path, b"episode")
        .expect("nested exact Plex filename should be written");
    std::fs::write(first_root.join("Pilot"), b"first")
        .expect("first quick ambiguous title should be written");
    std::fs::write(second_root.join("Pilot"), b"second")
        .expect("second quick ambiguous title should be written");

    let first_root_key =
        crate::app::media_search_cache::normalized_media_search_root_key(&first_root);
    let second_root_key =
        crate::app::media_search_cache::normalized_media_search_root_key(&second_root);
    let built_at_unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_millis() as u64;
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.attached_media_search_index = Some(GuiAttachedMediaSearchIndex {
        roots: vec![first_root_key.clone(), second_root_key.clone()],
        root_indexes_by_key: std::collections::HashMap::from([
            (
                first_root_key.clone(),
                GuiAttachedMediaSearchRootIndex {
                    root_key: first_root_key,
                    root_path: first_root.clone(),
                    built_at_unix_ms,
                    candidates_by_name: std::collections::HashMap::from([(
                        GuiClientCoreChatSessionRuntimeAdapter::missing_media_file_name_lookup_key(
                            "Show.S01E01.mkv",
                        )
                        .expect("exact Plex filename key should be available"),
                        vec!["nested-show/Show.S01E01.mkv".to_owned()],
                    )]),
                },
            ),
            (
                second_root_key.clone(),
                GuiAttachedMediaSearchRootIndex {
                    root_key: second_root_key,
                    root_path: second_root.clone(),
                    built_at_unix_ms,
                    candidates_by_name: std::collections::HashMap::new(),
                },
            ),
        ]),
        roots_requiring_refresh: std::collections::BTreeSet::new(),
    });
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        media_search_directories: Some(vec![
            first_root.to_string_lossy().into_owned(),
            second_root.to_string_lossy().into_owned(),
        ]),
        ..StoredClientSettingsMvp::default()
    });
    let plex_uri = "plex://machine-1/metadata/123?title=Pilot&file=Show.S01E01.mkv";
    let expected = GuiUserMediaTargetResolution::Resolved {
        path: expected_path.to_string_lossy().into_owned(),
        source: GuiUserMediaTargetResolutionSource::MediaSearchIndex,
    };

    assert_eq!(
        owner
            .resolve_main_window_user_media_target(&state, plex_uri)
            .expect("cross-layer main-window Plex resolution should succeed"),
        expected,
        "quick title ambiguity must not mask a unique indexed filename"
    );
    assert_eq!(
        owner
            .resolve_main_window_user_media_target_for_automatic_sync(&state, plex_uri)
            .expect("cross-layer Automatic Plex resolution should succeed"),
        expected,
        "Automatic resolution must preserve the same cross-layer evidence priority"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_waits_for_in_flight_filename_index_before_quick_title() {
    let root = test_temp_root("plex-alias-priority-pending-filename-index");
    let quick_title_path = root.join("Pilot");
    std::fs::write(&quick_title_path, b"title")
        .expect("quick Plex title fixture should be written");
    let root_key = crate::app::media_search_cache::normalized_media_search_root_key(&root);
    let (pending_tx, pending_rx) = std::sync::mpsc::channel();
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.attached_media_search_index = Some(GuiAttachedMediaSearchIndex {
        roots: vec![root_key.clone()],
        root_indexes_by_key: std::collections::HashMap::new(),
        roots_requiring_refresh: std::collections::BTreeSet::from([root_key.clone()]),
    });
    owner.pending_attached_media_resolution = Some(GuiPendingAttachedMediaResolution {
        roots: vec![root_key],
        cancel_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        latest_progress: std::sync::Arc::new(std::sync::Mutex::new(None)),
        result_rx: pending_rx,
    });
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    let plex_uri = "plex://machine-1/metadata/123?title=Pilot&file=Show.S01E01.mkv";

    assert_eq!(
        owner
            .resolve_main_window_user_media_target(&state, plex_uri)
            .expect("pending filename evidence should be reported"),
        GuiUserMediaTargetResolution::Pending,
        "a quick title must wait while the index can still establish stronger filename evidence"
    );
    assert_eq!(
        owner
            .resolve_main_window_user_media_target_for_automatic_sync(&state, plex_uri)
            .expect("Automatic pending filename evidence should be reported"),
        GuiUserMediaTargetResolution::Pending,
        "Automatic resolution must also wait at the filename-class boundary"
    );

    drop(pending_tx);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_exhausts_inventory_filename_before_indexed_title() {
    let root = test_temp_root("plex-alias-priority-cross-layer-inventory");
    let media_root = root.join("library");
    let title_directory = media_root.join("indexed-title");
    let filename_directory = media_root.join("inventory-filename");
    std::fs::create_dir_all(&title_directory).expect("indexed title directory should be created");
    std::fs::create_dir_all(&filename_directory)
        .expect("exact-inventory filename directory should be created");
    let title_path = title_directory.join("Pilot.mkv");
    std::fs::write(&title_path, b"title").expect("indexed Plex title should be written");
    let expected_path = filename_directory.join("Show.S01E01.mkv");
    std::fs::write(&expected_path, b"filename")
        .expect("exact-inventory Plex filename should be written");

    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        media_search_directories: Some(vec![media_root.to_string_lossy().into_owned()]),
        media_matching_plugin_enabled: Some(true),
        media_match_fingerprinting_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    crate::app::media_match_support::rebuild_persisted_media_match_index_with_extraction_settings_and_cancel(
        &root,
        std::slice::from_ref(&media_root),
        None,
        &state.media_match.settings,
        &sorotte_media_match::MediaExtractionSettings::sampled_fast_audio_index_v3(),
        None,
        |_| {},
    )
    .expect("cross-layer Plex inventory should be persisted");

    let root_key = crate::app::media_search_cache::normalized_media_search_root_key(&media_root);
    let mut owner =
        GuiPersistedConfigRuntimeOwner::with_config_path(Some(root.join("sorotte.ini")));
    owner.attached_media_search_index = Some(GuiAttachedMediaSearchIndex {
        roots: vec![root_key.clone()],
        root_indexes_by_key: std::collections::HashMap::from([(
            root_key.clone(),
            GuiAttachedMediaSearchRootIndex {
                root_key,
                root_path: media_root.clone(),
                built_at_unix_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("system time should be after unix epoch")
                    .as_millis() as u64,
                // Deliberately model a stale attached index that knows the title but
                // not the stronger filename already present in exact inventory.
                candidates_by_name: std::collections::HashMap::from([(
                    GuiClientCoreChatSessionRuntimeAdapter::missing_media_file_name_lookup_key(
                        "Pilot.mkv",
                    )
                    .expect("Plex title key should be available"),
                    vec!["indexed-title/Pilot.mkv".to_owned()],
                )]),
            },
        )]),
        roots_requiring_refresh: std::collections::BTreeSet::new(),
    });
    let plex_uri = "plex://machine-1/metadata/123?title=Pilot.mkv&file=Show.S01E01.mkv";
    let expected = GuiUserMediaTargetResolution::Resolved {
        path: sorotte_media_match::normalize_media_path(&expected_path),
        source: GuiUserMediaTargetResolutionSource::MediaMatchExactInventory,
    };

    assert_eq!(
        owner
            .resolve_main_window_user_media_target(&state, plex_uri)
            .expect("inventory-priority main-window Plex resolution should succeed"),
        expected,
        "an indexed title must not mask stronger exact-inventory filename evidence"
    );
    assert_eq!(
        owner
            .resolve_main_window_user_media_target_for_automatic_sync(&state, plex_uri)
            .expect("inventory-priority Automatic Plex resolution should succeed"),
        expected,
        "Automatic resolution must exhaust exact-inventory filename evidence before title"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_keeps_plex_filename_ambiguity_authoritative() {
    let root = test_temp_root("plex-filename-ambiguity-authoritative");
    let first_root = root.join("library-a");
    let second_root = root.join("library-b");
    std::fs::create_dir_all(&first_root).expect("first Plex ambiguity root should be created");
    std::fs::create_dir_all(&second_root).expect("second Plex ambiguity root should be created");
    std::fs::write(first_root.join("Show.S01E01.mkv"), b"first")
        .expect("first ambiguous exact filename should be written");
    std::fs::write(second_root.join("Show.S01E01.mkv"), b"second")
        .expect("second ambiguous exact filename should be written");
    std::fs::write(first_root.join("Pilot"), b"unique title")
        .expect("unique fallback title should be written");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        media_search_directories: Some(vec![
            first_root.to_string_lossy().into_owned(),
            second_root.to_string_lossy().into_owned(),
        ]),
        ..StoredClientSettingsMvp::default()
    });
    let plex_uri = "plex://machine-1/metadata/123?title=Pilot&file=Show.S01E01.mkv";

    assert_eq!(
        owner
            .resolve_main_window_user_media_target(&state, plex_uri)
            .expect("ambiguous Plex filename resolution should complete"),
        GuiUserMediaTargetResolution::Ambiguous { candidate_count: 2 },
        "a unique title must not override ambiguity in the stronger exact-filename class"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(target_os = "linux")]
#[test]
fn gui_persisted_config_runtime_owner_prefers_exact_case_search_root_file_over_folded_current_file()
{
    let root = test_temp_root("exact-case-search-root-before-folded-current");
    let current_directory = root.join("current");
    std::fs::create_dir_all(&current_directory)
        .expect("current-player fixture directory should be created");
    let current_path = current_directory.join("Pilot.mkv");
    let expected_path = root.join("pilot.mkv");
    std::fs::write(&current_path, b"current").expect("folded current fixture should be written");
    std::fs::write(&expected_path, b"exact").expect("exact-case fixture should be written");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("Pilot.mkv")
            .with_path(current_path.to_string_lossy().into_owned()),
    );
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });

    assert_eq!(
        owner
            .resolve_main_window_user_media_target(&state, "pilot.mkv")
            .expect("case-sensitive quick resolution should complete"),
        GuiUserMediaTargetResolution::Resolved {
            path: expected_path.to_string_lossy().into_owned(),
            source: GuiUserMediaTargetResolutionSource::QuickLocal,
        },
        "the exact-case search-root file must outrank a case-folded current-player match"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(target_os = "linux")]
#[test]
fn gui_persisted_config_runtime_owner_prefers_exact_case_indexed_file_over_folded_current_file() {
    let root = test_temp_root("exact-case-index-before-folded-current");
    let current_directory = root.join("current");
    let nested_directory = root.join("nested");
    std::fs::create_dir_all(&current_directory)
        .expect("current-player fixture directory should be created");
    std::fs::create_dir_all(&nested_directory)
        .expect("nested exact-case fixture directory should be created");
    let current_path = current_directory.join("Pilot.mkv");
    let expected_path = nested_directory.join("pilot.mkv");
    std::fs::write(&current_path, b"current").expect("folded current fixture should be written");
    std::fs::write(&expected_path, b"exact").expect("indexed exact-case fixture should be written");

    let root_key = crate::app::media_search_cache::normalized_media_search_root_key(&root);
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("Pilot.mkv")
            .with_path(current_path.to_string_lossy().into_owned()),
    );
    owner.attached_media_search_index = Some(GuiAttachedMediaSearchIndex {
        roots: vec![root_key.clone()],
        root_indexes_by_key: std::collections::HashMap::from([(
            root_key.clone(),
            GuiAttachedMediaSearchRootIndex {
                root_key,
                root_path: root.clone(),
                built_at_unix_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("system time should be after unix epoch")
                    .as_millis() as u64,
                candidates_by_name: std::collections::HashMap::from([(
                    GuiClientCoreChatSessionRuntimeAdapter::missing_media_file_name_lookup_key(
                        "pilot.mkv",
                    )
                    .expect("exact-case lookup key should be available"),
                    vec!["nested/pilot.mkv".to_owned()],
                )]),
            },
        )]),
        roots_requiring_refresh: std::collections::BTreeSet::new(),
    });
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });

    assert_eq!(
        owner
            .resolve_main_window_user_media_target(&state, "pilot.mkv")
            .expect("case-sensitive indexed resolution should complete"),
        GuiUserMediaTargetResolution::Resolved {
            path: expected_path.to_string_lossy().into_owned(),
            source: GuiUserMediaTargetResolutionSource::MediaSearchIndex,
        },
        "the exact-case indexed file must outrank a case-folded current-player match"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(target_os = "linux")]
#[test]
fn gui_persisted_config_runtime_owner_uses_folded_current_file_after_exact_search_is_exhausted() {
    let root = test_temp_root("folded-current-after-exact-search");
    let media_root = root.join("library");
    std::fs::create_dir_all(&media_root).expect("folded-current media root should be created");
    let current_path = media_root.join("Pilot.mkv");
    std::fs::write(&current_path, b"current").expect("folded current fixture should be written");

    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        media_search_directories: Some(vec![media_root.to_string_lossy().into_owned()]),
        media_matching_plugin_enabled: Some(true),
        media_match_fingerprinting_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    crate::app::media_match_support::rebuild_persisted_media_match_index_with_extraction_settings_and_cancel(
        &root,
        std::slice::from_ref(&media_root),
        None,
        &state.media_match.settings,
        &sorotte_media_match::MediaExtractionSettings::sampled_fast_audio_index_v3(),
        None,
        |_| {},
    )
    .expect("folded current path should be present in exact inventory");

    let root_key = crate::app::media_search_cache::normalized_media_search_root_key(&media_root);
    let mut owner =
        GuiPersistedConfigRuntimeOwner::with_config_path(Some(root.join("sorotte.ini")));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("Pilot.mkv")
            .with_path(current_path.to_string_lossy().into_owned()),
    );
    owner.attached_media_search_index = Some(GuiAttachedMediaSearchIndex {
        roots: vec![root_key.clone()],
        root_indexes_by_key: std::collections::HashMap::from([(
            root_key.clone(),
            GuiAttachedMediaSearchRootIndex {
                root_key,
                root_path: media_root,
                built_at_unix_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("system time should be after unix epoch")
                    .as_millis() as u64,
                candidates_by_name: std::collections::HashMap::new(),
            },
        )]),
        roots_requiring_refresh: std::collections::BTreeSet::new(),
    });

    assert_eq!(
        owner
            .resolve_main_window_user_media_target(&state, "pilot.mkv")
            .expect("folded current fallback resolution should complete"),
        GuiUserMediaTargetResolution::Resolved {
            path: current_path.to_string_lossy().into_owned(),
            source: GuiUserMediaTargetResolutionSource::QuickLocal,
        },
        "a folded current-player inventory hit must remain deferred until exact evidence is exhausted"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_rejects_uncorroborated_current_player_plex_title_collision() {
    let root = test_temp_root("plex-current-player-title-collision");
    let current_path = root.join("Pilot");
    std::fs::write(&current_path, b"unrelated")
        .expect("unrelated current-player title fixture should be written");
    let current_path = current_path.to_string_lossy().into_owned();

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_local_file =
        Some(sorotte_player_api::LocalFileUpdate::new("Pilot").with_path(current_path.clone()));
    let root_key = crate::app::media_search_cache::normalized_media_search_root_key(&root);
    owner.attached_media_search_index = Some(GuiAttachedMediaSearchIndex {
        roots: vec![root_key.clone()],
        root_indexes_by_key: std::collections::HashMap::from([(
            root_key.clone(),
            GuiAttachedMediaSearchRootIndex {
                root_key,
                root_path: root.clone(),
                built_at_unix_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("system time should be after unix epoch")
                    .as_millis() as u64,
                candidates_by_name: std::collections::HashMap::from([(
                    GuiClientCoreChatSessionRuntimeAdapter::missing_media_file_name_lookup_key(
                        "Pilot",
                    )
                    .expect("current-player title key should be available"),
                    vec!["Pilot".to_owned()],
                )]),
            },
        )]),
        roots_requiring_refresh: std::collections::BTreeSet::new(),
    });
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    let plex_uri = "plex://machine-1/metadata/123?title=Pilot&file=Show.S01E01.mkv";

    assert!(
        !owner.current_player_matches_media_target(plex_uri),
        "a title-only basename collision is not corroborated Plex identity"
    );
    assert_eq!(
        owner
            .resolve_main_window_user_media_target(&state, plex_uri)
            .expect("Plex current-player collision check should complete"),
        GuiUserMediaTargetResolution::Missing,
        "neither quick nor indexed resolution may reclassify the unrelated current file through the title alias"
    );

    let corroborated_uri = sorotte_plex::format_plex_playlist_uri(&sorotte_plex::PlexPlaylistUri {
        machine_identifier: "machine-1".to_owned(),
        rating_key: "124".to_owned(),
        title: Some("Pilot".to_owned()),
        file_name: Some("Show.S01E01.mkv".to_owned()),
        duration_millis: None,
        size_bytes: Some(9),
        media_type: Some(sorotte_plex::PlexMediaType::Episode),
    });
    assert_eq!(
        owner
            .resolve_main_window_user_media_target(&state, &corroborated_uri)
            .expect("size-corroborated Plex title resolution should complete"),
        GuiUserMediaTargetResolution::Resolved {
            path: current_path,
            source: GuiUserMediaTargetResolutionSource::QuickLocal,
        },
        "matching size metadata may corroborate a title-only current-player alias"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_uses_plex_title_when_uri_has_no_filename() {
    let root = test_temp_root("plex-title-only-fallback");
    let expected_path = root.join("Pilot");
    std::fs::write(&expected_path, b"episode")
        .expect("title-only Plex fallback fixture should be written");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    let plex_uri = "plex://machine-1/metadata/123?title=Pilot";

    assert_eq!(
        owner
            .resolve_main_window_user_media_target(&state, plex_uri)
            .expect("title-only Plex fallback should resolve"),
        GuiUserMediaTargetResolution::Resolved {
            path: expected_path.to_string_lossy().into_owned(),
            source: GuiUserMediaTargetResolutionSource::QuickLocal,
        }
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_uses_plex_title_after_filename_class_no_match() {
    let root = test_temp_root("plex-title-after-filename-no-match");
    let expected_path = root.join("Pilot");
    std::fs::write(&expected_path, b"episode")
        .expect("Plex title fallback fixture should be written");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    let plex_uri = "plex://machine-1/metadata/123?title=Pilot&file=Missing.S01E01.mkv";

    assert_eq!(
        owner
            .resolve_main_window_user_media_target(&state, plex_uri)
            .expect("Plex title fallback after filename no-match should resolve"),
        GuiUserMediaTargetResolution::Resolved {
            path: expected_path.to_string_lossy().into_owned(),
            source: GuiUserMediaTargetResolutionSource::QuickLocal,
        }
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_preserves_plex_alias_priority_in_exact_inventory() {
    let root = test_temp_root("plex-alias-priority-exact-inventory");
    let media_root = root.join("library");
    let title_directory = media_root.join("a-title");
    let filename_directory = media_root.join("z-filename");
    std::fs::create_dir_all(&title_directory).expect("title inventory directory should be created");
    std::fs::create_dir_all(&filename_directory)
        .expect("filename inventory directory should be created");
    std::fs::write(title_directory.join("Pilot.mkv"), b"title")
        .expect("title inventory candidate should be written");
    let expected_path = filename_directory.join("Show.S01E01.mkv");
    std::fs::write(&expected_path, b"filename")
        .expect("filename inventory candidate should be written");

    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        media_search_directories: Some(vec![media_root.to_string_lossy().into_owned()]),
        media_matching_plugin_enabled: Some(true),
        media_match_fingerprinting_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    crate::app::media_match_support::rebuild_persisted_media_match_index_with_extraction_settings_and_cancel(
        &root,
        std::slice::from_ref(&media_root),
        None,
        &state.media_match.settings,
        &sorotte_media_match::MediaExtractionSettings::sampled_fast_audio_index_v3(),
        None,
        |_| {},
    )
    .expect("Plex alias inventory should be persisted");
    let mut owner =
        GuiPersistedConfigRuntimeOwner::with_config_path(Some(root.join("sorotte.ini")));
    let plex_uri = "plex://machine-1/metadata/123?title=Pilot.mkv&file=Show.S01E01.mkv";

    assert_eq!(
        owner.media_match_cached_exact_inventory_candidate_for_target(
            &state,
            plex_uri,
            std::slice::from_ref(&media_root),
        ),
        Some(sorotte_media_match::normalize_media_path(&expected_path)),
        "exact inventory must rank the filename alias ahead of a lexically earlier title alias"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_excludes_case_folded_title_only_current_path_from_exact_inventory()
 {
    let root = test_temp_root("plex-title-collision-exact-inventory");
    let media_root = root.join("library");
    std::fs::create_dir_all(&media_root).expect("collision inventory root should be created");
    let current_path = media_root.join("Pilot.mkv");
    std::fs::write(&current_path, b"unrelated")
        .expect("collision inventory candidate should be written");

    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![media_root.to_string_lossy().into_owned()]),
        media_matching_plugin_enabled: Some(true),
        media_match_fingerprinting_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    crate::app::media_match_support::rebuild_persisted_media_match_index_with_extraction_settings_and_cancel(
        &root,
        std::slice::from_ref(&media_root),
        None,
        &state.media_match.settings,
        &sorotte_media_match::MediaExtractionSettings::sampled_fast_audio_index_v3(),
        None,
        |_| {},
    )
    .expect("collision inventory should be persisted");

    let root_key = crate::app::media_search_cache::normalized_media_search_root_key(&media_root);
    let mut owner =
        GuiPersistedConfigRuntimeOwner::with_config_path(Some(root.join("sorotte.ini")));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("Pilot.mkv")
            .with_path(current_path.to_string_lossy().into_owned()),
    );
    owner.attached_media_search_index = Some(GuiAttachedMediaSearchIndex {
        roots: vec![root_key.clone()],
        root_indexes_by_key: std::collections::HashMap::from([(
            root_key.clone(),
            GuiAttachedMediaSearchRootIndex {
                root_key,
                root_path: media_root.clone(),
                built_at_unix_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("system time should be after unix epoch")
                    .as_millis() as u64,
                candidates_by_name: std::collections::HashMap::new(),
            },
        )]),
        roots_requiring_refresh: std::collections::BTreeSet::new(),
    });
    let plex_uri = "plex://machine-1/metadata/123?title=pilot.mkv&file=Missing.S01E01.mkv";
    assert_eq!(
        owner.media_match_cached_exact_inventory_candidate_for_target(
            &state,
            plex_uri,
            std::slice::from_ref(&media_root),
        ),
        Some(sorotte_media_match::normalize_media_path(&current_path)),
        "fixture must expose the title-only current path through exact inventory"
    );

    let resolution = owner
        .resolve_main_window_user_media_target(&state, plex_uri)
        .expect("exact-inventory collision resolution should complete");
    assert!(
        !matches!(resolution, GuiUserMediaTargetResolution::Resolved { .. }),
        "an uncorroborated title-only current path must remain ineligible even when exact inventory sees it"
    );

    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.active_shared_playlist_index = Some(0);
    state.apply_shared_playlist_entries(vec![plex_uri.to_owned()], Some(0), false);
    let source_state = &mut state.main_window.playlist[0].source_state;
    source_state.policy = GuiPlaylistSourcePolicy::ForceMediaMatching;
    source_state.selection_origin = GuiPlaylistSourceSelectionOrigin::UserOverride;
    source_state.current_provider_id = GuiMediaSourceProviderId::media_matching();

    assert_eq!(
        owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state),
        SelectedPlaylistMediaSyncOutcome::NoChange,
        "Force Media Matching must not reopen the excluded current path through folded inventory"
    );
    assert!(
        owner
            .playlist_resolution_attempt
            .as_ref()
            .is_none_or(|attempt| { attempt.state != PlaylistResolutionAttemptState::Loading }),
        "the excluded current path must not start a media-load attempt"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_keeps_matching_local_file_for_plex_uri_without_streaming() {
    let root = test_temp_root("plex-playlist-uri-current-local");
    let selected_media_path = root.join("Episode 1.mkv");
    std::fs::write(&selected_media_path, b"test")
        .expect("Plex current-local fixture should be written");
    let selected_media_path = selected_media_path.to_string_lossy().into_owned();
    let plex_uri = sorotte_plex::format_plex_playlist_uri(&sorotte_plex::PlexPlaylistUri {
        machine_identifier: "machine-1".to_owned(),
        rating_key: "123".to_owned(),
        title: Some("Episode 1".to_owned()),
        file_name: Some("Episode 1.mkv".to_owned()),
        duration_millis: Some(90_000),
        size_bytes: Some(4),
        media_type: Some(sorotte_plex::PlexMediaType::Episode),
    });
    let case_variant_file_name_uri =
        sorotte_plex::format_plex_playlist_uri(&sorotte_plex::PlexPlaylistUri {
            machine_identifier: "machine-1".to_owned(),
            rating_key: "case-variant".to_owned(),
            title: Some("Episode 1".to_owned()),
            file_name: Some("episode 1.MKV".to_owned()),
            duration_millis: Some(90_000),
            size_bytes: Some(4),
            media_type: Some(sorotte_plex::PlexMediaType::Episode),
        });
    let mismatched_size_uri =
        sorotte_plex::format_plex_playlist_uri(&sorotte_plex::PlexPlaylistUri {
            machine_identifier: "machine-1".to_owned(),
            rating_key: "124".to_owned(),
            title: Some("Episode 1".to_owned()),
            file_name: Some("Episode 1.mkv".to_owned()),
            duration_millis: Some(90_000),
            size_bytes: Some(5),
            media_type: Some(sorotte_plex::PlexMediaType::Episode),
        });
    let missing_size_uri = sorotte_plex::format_plex_playlist_uri(&sorotte_plex::PlexPlaylistUri {
        machine_identifier: "machine-1".to_owned(),
        rating_key: "125".to_owned(),
        title: Some("Episode 1".to_owned()),
        file_name: Some("Episode 1.mkv".to_owned()),
        duration_millis: Some(90_000),
        size_bytes: None,
        media_type: Some(sorotte_plex::PlexMediaType::Episode),
    });

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("Episode 1.mkv")
            .with_path(selected_media_path.clone()),
    );
    owner.active_shared_playlist_index = Some(0);

    assert!(
        owner.current_player_matches_media_target(&plex_uri),
        "a Plex URI published for a local file should match the already-open local path by filename and size"
    );
    assert!(
        owner.current_player_matches_media_target(&case_variant_file_name_uri),
        "remote Plex filename aliases should remain case-insensitive when size corroborates the local file"
    );
    assert!(
        !owner.current_player_matches_media_target(&mismatched_size_uri),
        "filename alone is not enough to suppress Plex streaming for another item"
    );
    assert!(
        !owner.current_player_matches_media_target(&missing_size_uri),
        "Plex URIs without a size hint must not create loose basename matches"
    );

    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(vec![plex_uri], Some(0), false);

    let outcome = owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);

    assert_eq!(
        outcome,
        SelectedPlaylistMediaSyncOutcome::MatchedCurrentTarget
    );
    assert!(
        owner.plex_stream_resolve_rx.is_none(),
        "automatic sync must not queue Plex streaming once the local file satisfies the Plex playlist URI"
    );
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

    assert_eq!(outcome, SelectedPlaylistMediaSyncOutcome::StartedLoading);
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

    assert_eq!(outcome, SelectedPlaylistMediaSyncOutcome::StartedLoading);
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
fn gui_persisted_config_runtime_owner_honors_selected_plex_source_when_local_media_exists() {
    let root = test_temp_root("selected-plex-stream-over-local-playlist-activation");
    let config_path = root.join("sorotte.ini");
    let first_media_path = root.join("Episode 1.mkv");
    let second_media_path = root.join("Episode 2.mkv");
    std::fs::write(&first_media_path, b"test")
        .expect("first local playlist fixture should be written");
    std::fs::write(&second_media_path, b"test")
        .expect("second local playlist fixture should be written");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path));
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.active_shared_playlist_index = Some(1);

    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        plex_plugin_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("machine-1".to_owned()),
        plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(
        vec!["Episode 1.mkv".to_owned(), "Episode 2.mkv".to_owned()],
        Some(1),
        false,
    );
    state.main_window.active_playlist_index = Some(1);
    state.main_window.playlist[1].source_state =
        GuiPlaylistSourceState::for_provider(GuiMediaSourceProviderId::plex_stream());

    let outcome = owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);

    assert_eq!(outcome, SelectedPlaylistMediaSyncOutcome::NoChange);
    assert!(
        owner.plex_stream_resolve_rx.is_some(),
        "activating a row manually set to Plex Stream should queue Plex even when a local file exists"
    );
    assert!(
        owner.player_local_file.is_none(),
        "the automatic playlist activation path must not satisfy a Plex-selected row with local media"
    );
    assert!(
        owner.pending_attached_media_resolution.is_none(),
        "a Plex-selected row should not start local media search before Plex resolution"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_honors_forced_media_match_over_available_local_media() {
    let root = test_temp_root("selected-media-match-forced-playlist-activation");
    let selected_media_path = root.join("Episode 2.mkv");
    std::fs::write(&selected_media_path, b"test")
        .expect("selected Media Match force-policy fixture should be written");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.active_shared_playlist_index = Some(0);

    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        media_matching_plugin_enabled: Some(true),
        media_match_fingerprinting_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(vec!["Episode 2.mkv".to_owned()], Some(0), false);
    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylistSource {
        index: 0,
        provider_id: GuiMediaSourceProviderId::media_matching(),
    }));
    assert_eq!(
        state.main_window.playlist[0].source_state.policy,
        GuiPlaylistSourcePolicy::ForceMediaMatching
    );
    assert_eq!(
        state.main_window.playlist[0].source_state.selection_origin,
        GuiPlaylistSourceSelectionOrigin::UserOverride
    );

    let outcome = owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);

    assert_eq!(outcome, SelectedPlaylistMediaSyncOutcome::NoChange);
    assert!(
        owner.player_local_file.is_none(),
        "a manual Media Matching override must not be displaced by an exact local filename match"
    );
    assert!(
        owner.pending_attached_media_resolution.is_none(),
        "ForceMediaMatching must not start the ordinary local media-search path"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_playlist_default_media_match_remains_local_first() {
    let root = test_temp_root("default-media-match-local-first-playlist-activation");
    let selected_media_path = root.join("Episode 2.mkv");
    std::fs::write(&selected_media_path, b"test")
        .expect("default Media Match local-first fixture should be written");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.active_shared_playlist_index = Some(0);

    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        media_matching_plugin_enabled: Some(true),
        media_match_fingerprinting_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    assert!(
        state.apply(GuiShellAction::SelectMainWindowPlaylistDefaultSource {
            source_id: GuiPlaylistDefaultSourceId::provider(
                GuiMediaSourceProviderId::media_matching()
            ),
        })
    );
    state.apply_shared_playlist_entries(vec!["Episode 2.mkv".to_owned()], Some(0), false);
    assert_eq!(
        state.main_window.playlist[0].source_state.policy,
        GuiPlaylistSourcePolicy::PreferMediaMatching
    );
    assert_eq!(
        state.main_window.playlist[0].source_state.selection_origin,
        GuiPlaylistSourceSelectionOrigin::PlaylistDefault
    );

    let outcome = owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);
    let selected_media_path = selected_media_path.to_string_lossy().into_owned();

    assert_eq!(outcome, SelectedPlaylistMediaSyncOutcome::StartedLoading);
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(selected_media_path.as_str()),
        "a Media Matching playlist default should preserve local-first resolution"
    );
    assert!(
        owner.media_match_remote_lookup_rx.is_none(),
        "preferred Media Matching should not run when local media resolved first"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_preferred_media_match_recovers_from_local_open_failure() {
    let root = test_temp_root("preferred-media-match-local-failure");
    let config_path = root.join("sorotte.ini");
    let direct_root = root.join("direct");
    let media_match_root = root.join("media-match");
    let media_match_nested = media_match_root.join("alternate");
    std::fs::create_dir_all(&direct_root).expect("direct fixture root should be created");
    std::fs::create_dir_all(&media_match_nested)
        .expect("Media Matching fixture root should be created");
    let direct_path = direct_root.join("episode.mkv");
    let media_match_path = media_match_nested.join("episode.mkv");
    std::fs::write(&direct_path, b"broken direct candidate")
        .expect("direct fixture should be written");
    std::fs::write(&media_match_path, b"alternate Media Matching candidate")
        .expect("Media Matching fixture should be written");

    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![media_match_root.to_string_lossy().into_owned()]),
        media_matching_plugin_enabled: Some(true),
        media_match_fingerprinting_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    let direct_path = direct_path.to_string_lossy().into_owned();
    state.apply_shared_playlist_entries(vec!["episode.mkv".to_owned()], Some(0), false);
    state.main_window.playlist[0].source_state =
        GuiPlaylistSourceState::for_playlist_default(GuiMediaSourceProviderId::media_matching());

    crate::app::media_match_support::rebuild_persisted_media_match_index_with_extraction_settings_and_cancel(
        &root,
        std::slice::from_ref(&media_match_root),
        None,
        &state.media_match.settings,
        &sorotte_media_match::MediaExtractionSettings::sampled_fast_audio_index_v3(),
        None,
        |_| {},
    )
    .expect("Media Matching inventory should be persisted without fingerprint extraction");

    let (adapter, opened_paths) =
        FailFirstOpenPlayerAdapter::new(FirstOpenFailureMode::Synchronous);
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(adapter)));
    owner.active_shared_playlist_index = Some(0);
    owner.reconcile_local_shared_playlist_media_paths(&state);
    owner.playlist_resolution.local_origins_by_row.insert(
        state.main_window.playlist[0].entry_id,
        std::path::PathBuf::from(&direct_path),
    );
    let media_root_key =
        crate::app::media_search_cache::normalized_media_search_root_key(&media_match_root);
    owner.attached_media_search_index = Some(GuiAttachedMediaSearchIndex {
        roots: vec![media_root_key.clone()],
        root_indexes_by_key: std::collections::HashMap::from([(
            media_root_key.clone(),
            GuiAttachedMediaSearchRootIndex {
                root_key: media_root_key,
                root_path: media_match_root.clone(),
                built_at_unix_ms: 1,
                candidates_by_name: std::collections::HashMap::new(),
            },
        )]),
        roots_requiring_refresh: std::collections::BTreeSet::new(),
    });
    assert_eq!(
        owner.media_match_cached_exact_inventory_candidate_for_target(
            &state,
            "episode.mkv",
            std::slice::from_ref(&media_match_root),
        ),
        Some(sorotte_media_match::normalize_media_path(&media_match_path)),
        "the fallback fixture must expose a distinct exact Media Matching candidate"
    );

    let outcome = owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);
    assert_eq!(
        outcome,
        SelectedPlaylistMediaSyncOutcome::StartedLoading,
        "PreferMediaMatching should continue after the exact local open fails; opened={:?}",
        *opened_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    );
    assert_eq!(
        *opened_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        vec![
            direct_path,
            sorotte_media_match::normalize_media_path(&media_match_path),
        ]
    );
    let attempt = owner.playlist_resolution_attempt.as_ref().unwrap();
    assert_eq!(attempt.candidate_failures.len(), 1);
    assert_eq!(
        attempt.candidate_provider,
        Some(GuiMediaSourceProviderId::media_matching())
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_falls_back_to_plex_after_media_match_open_failure() {
    let root = test_temp_root("media-match-open-failure-plex-fallback");
    let config_path = root.join("sorotte.ini");
    let media_root = root.join("media-match");
    std::fs::create_dir_all(&media_root).expect("Media Matching fixture root should be created");
    let media_match_path = media_root.join("episode.mkv");
    std::fs::write(&media_match_path, b"broken Media Matching candidate")
        .expect("Media Matching fixture should be written");

    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![media_root.to_string_lossy().into_owned()]),
        media_matching_plugin_enabled: Some(true),
        media_match_fingerprinting_enabled: Some(true),
        plex_plugin_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("machine-1".to_owned()),
        plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(vec!["episode.mkv".to_owned()], Some(0), false);

    crate::app::media_match_support::rebuild_persisted_media_match_index_with_extraction_settings_and_cancel(
        &root,
        std::slice::from_ref(&media_root),
        None,
        &state.media_match.settings,
        &sorotte_media_match::MediaExtractionSettings::sampled_fast_audio_index_v3(),
        None,
        |_| {},
    )
    .expect("Media Matching inventory should be persisted without fingerprint extraction");

    let (adapter, opened_paths) =
        FailFirstOpenPlayerAdapter::new(FirstOpenFailureMode::Synchronous);
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(adapter)));
    owner.active_shared_playlist_index = Some(0);

    assert_eq!(
        owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state),
        SelectedPlaylistMediaSyncOutcome::NoChange
    );
    assert_eq!(
        owner.playlist_resolution_attempt.as_ref().unwrap().state,
        PlaylistResolutionAttemptState::Failed
    );
    assert_eq!(
        owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state),
        SelectedPlaylistMediaSyncOutcome::NoChange
    );
    assert!(
        owner.plex_stream_resolve_rx.is_some(),
        "a failed Media Matching candidate should advance Automatic to Plex"
    );

    let (stream_target, _) = test_plex_stream_target("episode.mkv", "media-match-fallback");
    let playback_url = stream_target.playback_url.as_str().to_owned();
    let trigger_key = owner.plex_stream_resolve_trigger_key.clone().unwrap();
    let operation_context = owner.plex_stream_resolve_context.clone().unwrap();
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    result_tx
        .send(GuiPlexStreamResolveWorkerResult {
            operation_context,
            trigger_key,
            result: Ok(GuiPlexStreamResolveOutcome {
                stream_target: Ok(Some(stream_target)),
                cache: sorotte_plex::PlexMatchCache::default(),
            }),
            staged_cache_write: None,
        })
        .expect("Plex fallback result should queue");
    owner.plex_stream_resolve_rx = Some(result_rx);
    owner.plex_stream_resolve_result = None;
    assert!(owner.pump_plex_stream_resolution_worker(&state));

    assert_eq!(
        owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state),
        SelectedPlaylistMediaSyncOutcome::StartedLoading
    );
    assert_eq!(
        *opened_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        vec![
            media_match_path.to_string_lossy().into_owned(),
            playback_url,
        ]
    );
    assert_eq!(
        owner
            .playlist_resolution_attempt
            .as_ref()
            .and_then(|attempt| attempt.candidate_provider.clone()),
        Some(GuiMediaSourceProviderId::plex_stream())
    );

    let _ = std::fs::remove_dir_all(&root);
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
fn gui_persisted_config_runtime_owner_waits_for_pending_local_index_before_ready_plex_fallback() {
    let root = test_temp_root("ready-plex-waits-for-pending-local-index");
    std::fs::create_dir_all(&root).expect("pending local index fixture root should be created");

    let playlist_uri = sorotte_plex::PlexPlaylistUri {
        machine_identifier: "machine-1".to_owned(),
        rating_key: "123".to_owned(),
        title: Some("Episode 1".to_owned()),
        file_name: Some("Episode 1.mkv".to_owned()),
        duration_millis: Some(90_000),
        size_bytes: Some(123_456),
        media_type: Some(sorotte_plex::PlexMediaType::Episode),
    };
    let plex_uri = sorotte_plex::format_plex_playlist_uri(&playlist_uri);
    let logical_file = sorotte_player_api::LocalFileUpdate::new("Episode 1.mkv")
        .with_path(plex_uri.clone())
        .with_duration_seconds(90.0)
        .with_size_bytes(123_456);
    let stream_target = sorotte_plex::PlexStreamTarget {
        playlist_uri,
        matched_item: sorotte_plex::PlexMatchedItem {
            rating_key: "123".to_owned(),
            title: "Episode 1".to_owned(),
            media_type: sorotte_plex::PlexMediaType::Episode,
            duration_millis: Some(90_000),
        },
        logical_file: logical_file.clone(),
        playback_url: sorotte_plex::SecretPlexPlaybackUrl::new(
            "http://127.0.0.1:32400/library/parts/1/file.mkv?X-Plex-Token=secret-token",
        ),
    };

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.active_shared_playlist_index = Some(0);
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        folder_search_timeout_seconds: Some(0.1),
        plex_plugin_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("machine-1".to_owned()),
        plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(vec![plex_uri], Some(0), false);

    let first_outcome = owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);
    assert_eq!(first_outcome, SelectedPlaylistMediaSyncOutcome::NoChange);
    let original_pending_search = owner
        .pending_attached_media_resolution
        .take()
        .expect("automatic resolution should start local indexing");
    original_pending_search
        .cancel_flag
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let search_roots = original_pending_search.roots.clone();
    let (search_tx, search_rx) = std::sync::mpsc::channel();
    owner.pending_attached_media_resolution = Some(GuiPendingAttachedMediaResolution {
        roots: search_roots.clone(),
        cancel_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        latest_progress: std::sync::Arc::new(std::sync::Mutex::new(None)),
        result_rx: search_rx,
    });

    let trigger_key = owner
        .plex_stream_resolve_trigger_key
        .clone()
        .expect("automatic resolution should queue Plex concurrently");
    let operation_context = owner
        .plex_stream_resolve_context
        .clone()
        .expect("queued Plex resolution should retain its operation context");
    let (plex_tx, plex_rx) = std::sync::mpsc::channel();
    plex_tx
        .send(GuiPlexStreamResolveWorkerResult {
            operation_context: operation_context.clone(),
            trigger_key: trigger_key.clone(),
            result: Ok(GuiPlexStreamResolveOutcome {
                stream_target: Ok(Some(stream_target.clone())),
                cache: sorotte_plex::PlexMatchCache::default(),
            }),
            staged_cache_write: None,
        })
        .expect("ready Plex fallback should be queued");
    owner.plex_stream_resolve_rx = Some(plex_rx);
    owner.plex_stream_resolve_result = None;
    assert!(owner.pump_plex_stream_resolution_worker(&state));

    let waiting_outcome = owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);
    assert_eq!(waiting_outcome, SelectedPlaylistMediaSyncOutcome::NoChange);
    assert!(
        owner.player_local_file.is_none(),
        "a ready Plex stream must remain a fallback while higher-priority local indexing is pending"
    );
    assert!(owner.pending_attached_media_resolution.is_some());
    assert!(
        owner.plex_stream_resolve_result.is_some(),
        "the ready Plex result must remain available while the local index has priority"
    );

    let root_key = search_roots
        .first()
        .cloned()
        .expect("the configured search root should have a normalized key");
    search_tx
        .send(GuiAttachedMediaSearchBuildStatus::Completed(vec![
            GuiAttachedMediaSearchRootRefreshResult {
                root_key: root_key.clone(),
                index: Some(GuiAttachedMediaSearchRootIndex {
                    root_key,
                    root_path: root.clone(),
                    built_at_unix_ms: 1,
                    candidates_by_name: std::collections::HashMap::new(),
                }),
                error: None,
            },
        ]))
        .expect("local indexing completion should be queued");
    let _ = owner.poll_attached_media_search_index_build(std::time::Duration::from_secs(1));

    let fallback_outcome =
        owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);
    assert_eq!(
        fallback_outcome,
        SelectedPlaylistMediaSyncOutcome::StartedLoading
    );
    assert_eq!(
        owner.player_local_file,
        Some(logical_file),
        "Plex should become eligible once the configured local-search worker settles without a match"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_does_not_block_ready_plex_for_scheduled_local_refresh() {
    let root = test_temp_root("ready-plex-skips-scheduled-local-refresh");
    std::fs::create_dir_all(&root).expect("scheduled-refresh fixture root should be created");
    let root_key = crate::app::media_search_cache::normalized_media_search_root_key(&root);
    let target = "Episode 1.mkv";
    let (stream_target, logical_file) = test_plex_stream_target(target, "123");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.active_shared_playlist_index = Some(0);
    owner.attached_media_search_index = Some(GuiAttachedMediaSearchIndex {
        roots: vec![root_key.clone()],
        root_indexes_by_key: std::collections::HashMap::new(),
        roots_requiring_refresh: std::collections::BTreeSet::from([root_key]),
    });
    owner.attached_media_search_build_state = GuiAttachedMediaSearchBuildState::Failed;
    owner.attached_media_search_next_retry_at =
        Some(std::time::Instant::now() + std::time::Duration::from_secs(60));

    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        plex_plugin_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("machine-1".to_owned()),
        plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(vec![target.to_owned()], Some(0), false);

    assert!(!owner.attached_media_search_in_flight());
    assert!(owner.attached_media_search_refresh_required());
    assert!(owner.attached_media_search_retry_scheduled());
    let _ = owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);
    assert!(
        !owner.attached_media_search_in_flight(),
        "a future retry deadline must not manufacture an in-flight local lookup"
    );

    let trigger_key = owner
        .plex_stream_resolve_trigger_key
        .clone()
        .expect("automatic resolution should queue Plex while the local retry is scheduled");
    let operation_context = owner
        .plex_stream_resolve_context
        .clone()
        .expect("queued Plex resolution should retain its operation context");
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    result_tx
        .send(GuiPlexStreamResolveWorkerResult {
            operation_context,
            trigger_key,
            result: Ok(GuiPlexStreamResolveOutcome {
                stream_target: Ok(Some(stream_target)),
                cache: sorotte_plex::PlexMatchCache::default(),
            }),
            staged_cache_write: None,
        })
        .expect("ready Plex fallback should be queued");
    owner.plex_stream_resolve_rx = Some(result_rx);
    owner.plex_stream_resolve_result = None;
    assert!(owner.pump_plex_stream_resolution_worker(&state));

    let _ = owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);

    assert_eq!(owner.player_local_file, Some(logical_file));
    assert!(
        owner.attached_media_search_next_retry_at.is_some(),
        "opening Plex must preserve the scheduled background local refresh"
    );
    assert!(owner.attached_media_search_refresh_required());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_queues_plex_stream_while_media_match_misses() {
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

    let root = test_temp_root("plex-stream-while-media-match-misses");
    let config_path = root.join("sorotte.ini");
    let media_root = root.join("library");
    std::fs::create_dir_all(&media_root)
        .expect("Plex stream Media Match miss fixture root should be created");

    let plex_uri = "plex://machine-1/metadata/123?title=Episode%201&file=Episode%201.mkv";
    let mut owner =
        GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path)).with_session_runtime(
            Box::new(MediaMatchPeerSessionRuntimeAdapter {
                peer_files: vec![sorotte_client_core::ClientMediaMatchPeerFileState {
                    username: "remote".to_owned(),
                    has_file: true,
                    file_name: None,
                    file_size: None,
                    file_duration: None,
                    media_match_signature: Some(
                        sorotte_media_match::MediaMatchWireSignature::default(),
                    ),
                }],
            }),
        );
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.active_shared_playlist_index = Some(0);

    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![media_root.to_string_lossy().into_owned()]),
        media_matching_plugin_enabled: Some(true),
        media_match_fingerprinting_enabled: Some(true),
        media_match_wire_sharing_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(vec![plex_uri.to_owned()], Some(0), false);

    let outcome = owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);

    assert_eq!(outcome, SelectedPlaylistMediaSyncOutcome::NoChange);
    assert!(
        owner.pending_attached_media_resolution.is_some(),
        "missing local media should still start filename indexing"
    );
    assert!(
        owner.media_match_remote_lookup_rx.is_some(),
        "Media Match remote lookup should be queued but must not block Plex fallback"
    );
    assert!(
        owner.plex_stream_resolve_rx.is_some(),
        "Plex stream resolution should queue immediately even while Media Match is a miss or pending"
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
fn gui_persisted_config_runtime_owner_releases_only_the_matching_ready_plex_fallback_for_non_plex_policy()
 {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.active_shared_playlist_index = Some(0);
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        plex_plugin_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("machine-1".to_owned()),
        plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(
        vec!["Episode A.mkv".to_owned(), "Episode B.mkv".to_owned()],
        Some(0),
        false,
    );
    state.main_window.active_playlist_index = Some(0);

    let (_sync_tx, sync_rx) = std::sync::mpsc::channel();
    owner.plex_sync_rx = Some(sync_rx);
    assert_eq!(
        owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state),
        SelectedPlaylistMediaSyncOutcome::NoChange
    );
    let trigger_key = owner
        .plex_stream_resolve_trigger_key
        .take()
        .expect("automatic resolution should retain the active row's Plex trigger");
    let operation_context = owner
        .plex_stream_resolve_context
        .take()
        .expect("automatic resolution should retain its Plex operation context");
    owner.plex_sync_rx = None;
    let ready_result = || GuiPlexStreamResolveWorkerResult {
        operation_context: operation_context.clone(),
        trigger_key: trigger_key.clone(),
        result: Ok(GuiPlexStreamResolveOutcome {
            stream_target: Ok(None),
            cache: sorotte_plex::PlexMatchCache::default(),
        }),
        staged_cache_write: None,
    };
    owner.plex_stream_resolve_result = Some(ready_result());
    assert!(owner.plex_stream_resolution_owns_cache_snapshot());

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    assert!(owner.handle_resolve_playlist_source_request(
        &handle,
        &mut state,
        1,
        GuiMediaSourceProviderId::local(),
    ));
    assert!(
        owner.plex_stream_resolution_owns_cache_snapshot(),
        "resolving another row must not discard the active row's ready Plex fallback"
    );

    assert!(owner.handle_resolve_playlist_source_request(
        &handle,
        &mut state,
        0,
        GuiMediaSourceProviderId::local(),
    ));
    assert!(
        !owner.plex_stream_resolution_owns_cache_snapshot(),
        "a manual non-Plex selection must release the matching ready Plex fallback"
    );

    owner.plex_stream_resolve_result = Some(ready_result());
    let source_state = &mut state.main_window.playlist[0].source_state;
    source_state.policy = GuiPlaylistSourcePolicy::ForceMediaMatching;
    source_state.selection_origin = GuiPlaylistSourceSelectionOrigin::UserOverride;
    source_state.current_provider_id = GuiMediaSourceProviderId::media_matching();
    owner.last_attached_media_resolution_trigger = None;
    assert_eq!(
        owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state),
        SelectedPlaylistMediaSyncOutcome::NoChange
    );
    assert!(
        !owner.plex_stream_resolution_owns_cache_snapshot(),
        "an observed non-Plex row policy must release the matching ready Plex fallback"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_releases_plex_result_completed_after_active_target_switch() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.active_shared_playlist_index = Some(0);
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        plex_plugin_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("machine-1".to_owned()),
        plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(
        vec!["Episode A.mkv".to_owned(), "Episode B.mkv".to_owned()],
        Some(0),
        false,
    );
    state.main_window.active_playlist_index = Some(0);

    let (_sync_tx, sync_rx) = std::sync::mpsc::channel();
    owner.plex_sync_rx = Some(sync_rx);
    assert_eq!(
        owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state),
        SelectedPlaylistMediaSyncOutcome::NoChange
    );
    let trigger_key = owner
        .plex_stream_resolve_trigger_key
        .clone()
        .expect("automatic resolution should retain row A's Plex trigger");
    let operation_context = owner
        .plex_stream_resolve_context
        .clone()
        .expect("automatic resolution should retain row A's Plex context");
    owner.plex_sync_rx = None;
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    result_tx
        .send(GuiPlexStreamResolveWorkerResult {
            operation_context,
            trigger_key,
            result: Ok(GuiPlexStreamResolveOutcome {
                stream_target: Ok(None),
                cache: sorotte_plex::PlexMatchCache::default(),
            }),
            staged_cache_write: None,
        })
        .expect("row A's Plex result should queue");
    owner.plex_stream_resolve_rx = Some(result_rx);

    state.main_window.active_playlist_index = Some(1);
    owner.active_shared_playlist_index = Some(1);
    let source_state = &mut state.main_window.playlist[1].source_state;
    source_state.policy = GuiPlaylistSourcePolicy::ForceLocal;
    source_state.selection_origin = GuiPlaylistSourceSelectionOrigin::UserOverride;
    source_state.current_provider_id = GuiMediaSourceProviderId::local();

    assert!(owner.pump_plex_stream_resolution_worker(&state));
    assert!(
        owner.plex_stream_resolution_owns_cache_snapshot(),
        "the late row-A result should first be retained by the worker handoff"
    );
    assert_eq!(
        owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state),
        SelectedPlaylistMediaSyncOutcome::NoChange
    );
    assert!(
        !owner.plex_stream_resolution_owns_cache_snapshot(),
        "row A's completed result must be released once active row B supersedes it"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_consumes_terminal_plex_results_without_candidates() {
    let terminal_results = [
        (
            "no match",
            Ok(GuiPlexStreamResolveOutcome {
                stream_target: Ok(None),
                cache: sorotte_plex::PlexMatchCache::default(),
            }),
        ),
        ("resolution error", Err("terminal Plex failure".to_owned())),
    ];

    for (case, terminal_result) in terminal_results {
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.active_shared_playlist_index = Some(0);
        let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            shared_playlist_enabled: Some(true),
            plex_plugin_enabled: Some(true),
            plex_streaming_enabled: Some(true),
            plex_user_token: Some("user-token".into()),
            plex_selected_server_id: Some("machine-1".to_owned()),
            plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
            plex_selected_server_token: Some("server-token".into()),
            ..StoredClientSettingsMvp::default()
        });
        state.apply_shared_playlist_entries(vec!["Episode A.mkv".to_owned()], Some(0), false);
        state.main_window.active_playlist_index = Some(0);

        let (_sync_tx, sync_rx) = std::sync::mpsc::channel();
        owner.plex_sync_rx = Some(sync_rx);
        assert_eq!(
            owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state),
            SelectedPlaylistMediaSyncOutcome::NoChange,
            "{case} fixture should first queue Plex resolution"
        );
        let trigger_key = owner
            .plex_stream_resolve_trigger_key
            .take()
            .expect("automatic resolution should retain its Plex trigger");
        let operation_context = owner
            .plex_stream_resolve_context
            .take()
            .expect("automatic resolution should retain its Plex operation context");
        owner.plex_sync_rx = None;
        owner.plex_stream_resolve_result = Some(GuiPlexStreamResolveWorkerResult {
            operation_context,
            trigger_key,
            result: terminal_result,
            staged_cache_write: None,
        });
        owner.last_attached_media_resolution_trigger = None;
        assert!(owner.plex_stream_resolution_owns_cache_snapshot());

        assert_eq!(
            owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state),
            SelectedPlaylistMediaSyncOutcome::NoChange,
            "{case} has no playable fallback"
        );
        assert!(
            !owner.plex_stream_resolution_owns_cache_snapshot(),
            "{case} must be consumed instead of blocking Plex watch sync indefinitely"
        );
    }
}

#[test]
fn gui_persisted_config_runtime_owner_retries_plex_miss_and_activates_later_match() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.active_shared_playlist_index = Some(0);
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        plex_plugin_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("machine-1".to_owned()),
        plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(vec!["episode.mkv".to_owned()], Some(0), false);
    state.main_window.active_playlist_index = Some(0);

    let (_first_sync_tx, first_sync_rx) = std::sync::mpsc::channel();
    owner.plex_sync_rx = Some(first_sync_rx);
    assert_eq!(
        owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state),
        SelectedPlaylistMediaSyncOutcome::NoChange
    );
    let (_, resolving_source) = owner
        .playlist_resolution_source_state_for_projection(&state)
        .expect("the active automatic attempt should project while Plex is resolving");
    assert_eq!(resolving_source.policy, GuiPlaylistSourcePolicy::Automatic);
    assert_eq!(resolving_source.resolved_provider_id, None);
    assert_eq!(resolving_source.current_label, "Automatic");
    assert_eq!(resolving_source.status, GuiPlaylistSourceStatus::Resolving);
    assert!(
        resolving_source
            .options
            .iter()
            .all(|option| !option.selected)
    );
    let first_trigger = owner.plex_stream_resolve_trigger_key.take().unwrap();
    let first_context = owner.plex_stream_resolve_context.take().unwrap();
    owner.plex_sync_rx = None;
    owner.plex_stream_resolve_result = Some(GuiPlexStreamResolveWorkerResult {
        operation_context: first_context,
        trigger_key: first_trigger,
        result: Ok(GuiPlexStreamResolveOutcome {
            stream_target: Ok(None),
            cache: sorotte_plex::PlexMatchCache::default(),
        }),
        staged_cache_write: None,
    });
    owner.last_attached_media_resolution_trigger = None;
    assert_eq!(
        owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state),
        SelectedPlaylistMediaSyncOutcome::NoChange
    );
    let (_, missing_source) = owner
        .playlist_resolution_source_state_for_projection(&state)
        .expect("the active automatic miss should remain visible during Plex backoff");
    assert_eq!(missing_source.resolved_provider_id, None);
    assert_eq!(missing_source.current_label, "Automatic");
    assert_eq!(missing_source.status, GuiPlaylistSourceStatus::Missing);
    assert!(
        missing_source
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("Plex will retry automatically"))
    );
    assert!(
        !missing_source
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("episode.mkv"),
        "the lifecycle detail must not expose the media target"
    );
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let applied_missing_source = &state.main_window.playlist[0].source_state;
    assert_eq!(applied_missing_source.current_label, "Automatic");
    assert_eq!(
        applied_missing_source.status,
        GuiPlaylistSourceStatus::Missing,
        "snapshot reduction must preserve the provider-less Automatic lifecycle state"
    );
    assert_eq!(applied_missing_source.resolved_provider_id, None);
    assert!(
        applied_missing_source
            .options
            .iter()
            .all(|option| !option.selected)
    );
    let miss = owner
        .plex_miss_state
        .as_mut()
        .expect("the initial active Plex miss should schedule an independent retry");
    assert_eq!(miss.attempt_count, 1);
    miss.next_retry_at = std::time::Instant::now();
    assert!(owner.active_plex_miss_retry_due(&state));
    // The runtime pump invalidates the cached automatic trigger when this
    // independent deadline becomes due before asking the coordinator to retry.
    owner.last_attached_media_resolution_trigger = None;

    let (_second_sync_tx, second_sync_rx) = std::sync::mpsc::channel();
    owner.plex_sync_rx = Some(second_sync_rx);
    assert_eq!(
        owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state),
        SelectedPlaylistMediaSyncOutcome::NoChange
    );
    assert!(owner.plex_miss_state.as_ref().unwrap().retry_in_flight);
    let (_, retrying_source) = owner
        .playlist_resolution_source_state_for_projection(&state)
        .expect("the independent Plex retry should project as resolving");
    assert_eq!(retrying_source.current_label, "Automatic");
    assert_eq!(retrying_source.status, GuiPlaylistSourceStatus::Resolving);
    let second_trigger = owner.plex_stream_resolve_trigger_key.take().unwrap();
    let second_context = owner.plex_stream_resolve_context.take().unwrap();
    owner.plex_sync_rx = None;
    let (stream_target, _) = test_plex_stream_target("episode.mkv", "later-indexed");
    owner.plex_stream_resolve_result = Some(GuiPlexStreamResolveWorkerResult {
        operation_context: second_context,
        trigger_key: second_trigger,
        result: Ok(GuiPlexStreamResolveOutcome {
            stream_target: Ok(Some(stream_target)),
            cache: sorotte_plex::PlexMatchCache::default(),
        }),
        staged_cache_write: None,
    });
    owner.last_attached_media_resolution_trigger = None;

    assert_eq!(
        owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state),
        SelectedPlaylistMediaSyncOutcome::StartedLoading
    );
    assert!(
        owner.plex_miss_state.is_none(),
        "a later indexed Plex match must reset the miss backoff"
    );
    let (_, loading_source) = owner
        .playlist_resolution_source_state_for_projection(&state)
        .expect("the later Plex candidate should project until player confirmation");
    assert_eq!(
        loading_source.resolved_provider_id,
        Some(GuiMediaSourceProviderId::plex_stream())
    );
    assert_eq!(loading_source.current_label, "Plex Stream");
    assert_eq!(loading_source.status, GuiPlaylistSourceStatus::Loading);
    owner.refresh_player_state_impl();
    assert_eq!(
        owner.playlist_resolution_attempt.as_ref().unwrap().state,
        PlaylistResolutionAttemptState::Active
    );
    let (_, active_source) = owner
        .playlist_resolution_source_state_for_projection(&state)
        .expect("the confirmed Plex candidate should project as active");
    assert_eq!(active_source.status, GuiPlaylistSourceStatus::Active);
}

#[test]
fn gui_runtime_owner_reruns_active_automatic_miss_when_plex_server_context_changes() {
    let root = test_temp_root("automatic-plex-context-reresolution");
    let config_path = root.join("sorotte.ini");
    let old_settings = StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        plex_plugin_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("old-machine".to_owned()),
        plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        plex_selected_server_token: Some("old-server-token".into()),
        ..StoredClientSettingsMvp::default()
    };
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(&config_path, &old_settings)
        .expect("the initial Plex settings should persist for the integration fixture");
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path));
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.active_shared_playlist_index = Some(0);
    let mut state = SorotteGuiShellAppState::from_stored_settings(&old_settings);
    state.apply_shared_playlist_entries(vec!["episode.mkv".to_owned()], Some(0), false);
    state.main_window.active_playlist_index = Some(0);
    let active_entry_id = state.main_window.playlist[0].entry_id;

    let (_first_sync_tx, first_sync_rx) = std::sync::mpsc::channel();
    owner.plex_sync_rx = Some(first_sync_rx);
    assert_eq!(
        owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state),
        SelectedPlaylistMediaSyncOutcome::NoChange
    );
    let old_stream_trigger = owner
        .plex_stream_resolve_trigger_key
        .take()
        .expect("the active Automatic row should queue its initial Plex resolution");
    let old_stream_context = owner
        .plex_stream_resolve_context
        .take()
        .expect("the initial Plex resolution should retain its operation context");
    owner.plex_sync_rx = None;
    owner.plex_stream_resolve_result = Some(GuiPlexStreamResolveWorkerResult {
        operation_context: old_stream_context,
        trigger_key: old_stream_trigger.clone(),
        result: Ok(GuiPlexStreamResolveOutcome {
            stream_target: Ok(None),
            cache: sorotte_plex::PlexMatchCache::default(),
        }),
        staged_cache_write: None,
    });
    owner.last_attached_media_resolution_trigger = None;
    assert_eq!(
        owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state),
        SelectedPlaylistMediaSyncOutcome::NoChange
    );
    let old_automatic_trigger = owner
        .last_attached_media_resolution_trigger
        .clone()
        .expect("the active Automatic miss should retain its outer trigger");
    let old_index_revision = owner.attached_media_search_index_revision;
    let old_player_path = owner
        .player_local_file
        .as_ref()
        .and_then(|file| file.path.clone());
    let miss = owner
        .plex_miss_state
        .as_ref()
        .expect("the initial Plex miss should enter independent backoff");
    assert_eq!(miss.attempt_count, 1);
    assert!(miss.next_retry_at > std::time::Instant::now());
    assert!(
        !owner.active_plex_miss_retry_due(&state),
        "the ordinary miss deadline must still be in the future"
    );

    owner.plex_servers.push(PlexServerConnection {
        name: "New server".to_owned(),
        machine_identifier: "new-machine".to_owned(),
        uri: "http://127.0.0.1:32401".to_owned(),
        access_token: "new-server-token".into(),
        owned: true,
        has_local_connection: true,
        connection_kind: PlexServerConnectionKind::Local,
    });
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    assert!(owner.handle_select_plex_server_request(
        &handle,
        &mut state,
        "new-machine".to_owned(),
        "http://127.0.0.1:32401".to_owned(),
    ));
    assert_eq!(
        state.saved_configuration.plex_selected_server_id.as_deref(),
        Some("new-machine")
    );
    assert_eq!(
        state
            .saved_configuration
            .plex_selected_server_url
            .as_deref(),
        Some("http://127.0.0.1:32401")
    );
    assert_eq!(state.main_window.active_playlist_index, Some(0));
    assert_eq!(owner.active_shared_playlist_index, Some(0));
    assert_eq!(state.main_window.playlist[0].entry_id, active_entry_id);
    assert_eq!(
        state.main_window.playlist[0].source_state.policy,
        GuiPlaylistSourcePolicy::Automatic
    );
    assert_eq!(
        owner.last_attached_media_resolution_trigger.as_ref(),
        Some(&old_automatic_trigger),
        "the test must not manually reactivate the row or clear its cached trigger"
    );
    assert!(owner.plex_context_media_resolution_pending);

    let (_new_sync_tx, new_sync_rx) = std::sync::mpsc::channel();
    owner.plex_sync_rx = Some(new_sync_rx);
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let new_stream_trigger = owner
        .plex_stream_resolve_trigger_key
        .as_ref()
        .expect("the runtime pump should immediately queue Plex against the new server context");
    assert_ne!(
        new_stream_trigger, &old_stream_trigger,
        "the queued Plex trigger must change with the selected server and token"
    );
    let new_stream_context = owner
        .plex_stream_resolve_context
        .as_ref()
        .expect("the new-context Plex resolution should remain pending");
    assert_ne!(
        Some(new_stream_context),
        old_automatic_trigger.plex_operation_context.as_ref(),
        "server selection must change the privacy-safe operation context"
    );
    assert_eq!(
        owner
            .last_attached_media_resolution_trigger
            .as_ref()
            .and_then(|trigger| trigger.plex_operation_context.as_ref()),
        Some(new_stream_context),
        "the outer Automatic trigger must track the context used by the queued Plex worker"
    );
    assert!(
        owner.plex_miss_state.is_none(),
        "the old server's miss backoff must not suppress the new-context resolution"
    );
    assert!(!owner.plex_context_media_resolution_pending);
    assert_eq!(state.main_window.active_playlist_index, Some(0));
    assert_eq!(state.main_window.playlist[0].entry_id, active_entry_id);
    assert_eq!(
        owner.attached_media_search_index_revision, old_index_revision,
        "the retry must not depend on a local-index change"
    );
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.clone()),
        old_player_path,
        "the retry must not depend on a player-path observation"
    );
    let new_automatic_trigger = owner
        .last_attached_media_resolution_trigger
        .as_ref()
        .expect("the immediate retry should retain its new outer trigger");
    assert_eq!(new_automatic_trigger.target, old_automatic_trigger.target);
    assert_eq!(
        new_automatic_trigger.playlist_entry_id,
        old_automatic_trigger.playlist_entry_id
    );
    assert_eq!(
        new_automatic_trigger.playlist_generation,
        old_automatic_trigger.playlist_generation
    );
    assert_eq!(new_automatic_trigger.roots, old_automatic_trigger.roots);
    assert_eq!(
        new_automatic_trigger.current_player_path,
        old_automatic_trigger.current_player_path
    );
    assert_eq!(
        new_automatic_trigger.index_revision,
        old_automatic_trigger.index_revision
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_retries_selected_plex_source_when_worker_finishes() {
    let local_entry = "Episode 1.mkv";
    let playlist_uri = sorotte_plex::PlexPlaylistUri {
        machine_identifier: "machine-1".to_owned(),
        rating_key: "123".to_owned(),
        title: Some("Episode 1".to_owned()),
        file_name: Some(local_entry.to_owned()),
        duration_millis: Some(90_000),
        size_bytes: Some(123_456),
        media_type: Some(sorotte_plex::PlexMediaType::Episode),
    };
    let logical_uri = sorotte_plex::format_plex_playlist_uri(&playlist_uri);
    let logical_file = sorotte_player_api::LocalFileUpdate::new(local_entry)
        .with_path(logical_uri.clone())
        .with_duration_seconds(90.0)
        .with_size_bytes(123_456);
    let stream_target = sorotte_plex::PlexStreamTarget {
        playlist_uri,
        matched_item: sorotte_plex::PlexMatchedItem {
            rating_key: "123".to_owned(),
            title: "Episode 1".to_owned(),
            media_type: sorotte_plex::PlexMediaType::Episode,
            duration_millis: Some(90_000),
        },
        logical_file: logical_file.clone(),
        playback_url: sorotte_plex::SecretPlexPlaybackUrl::new(
            "http://127.0.0.1:32400/library/parts/1/file.mkv?X-Plex-Token=secret-token",
        ),
    };

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.active_shared_playlist_index = Some(0);
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        plex_plugin_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("machine-1".to_owned()),
        plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(vec![local_entry.to_owned()], Some(0), false);
    state.main_window.active_playlist_index = Some(0);
    let plex_settings = state.configuration.to_stored_settings();
    assert!(
        state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::Plex)
    );
    assert_eq!(plex_settings.plex_streaming_enabled, Some(true));
    assert_eq!(
        plex_settings.plex_selected_server_url.as_deref(),
        Some("http://127.0.0.1:32400")
    );
    assert_eq!(
        plex_settings
            .plex_selected_server_token
            .as_ref()
            .map(|token| token.expose_secret()),
        Some("server-token")
    );
    let handle = GuiQueuedRuntimeBridgeHandle::default();

    assert!(owner.handle_resolve_playlist_source_request(
        &handle,
        &mut state,
        0,
        GuiMediaSourceProviderId::plex_stream(),
    ));
    for action in handle.drain_actions() {
        assert!(state.apply(action));
    }
    assert_eq!(
        state.main_window.playlist[0].source_state.status,
        GuiPlaylistSourceStatus::Pending
    );
    assert_eq!(
        state.main_window.playlist[0]
            .source_state
            .current_provider_id,
        GuiMediaSourceProviderId::plex_stream()
    );
    assert!(owner.pending_playlist_source_resolution.is_some());

    let trigger_key = owner
        .plex_stream_resolve_trigger_key
        .clone()
        .expect("selected Plex source should have queued a stream worker");
    let operation_context = owner
        .plex_stream_resolve_context
        .clone()
        .expect("selected Plex source should capture its operation context");
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    result_tx
        .send(GuiPlexStreamResolveWorkerResult {
            operation_context,
            trigger_key,
            result: Ok(GuiPlexStreamResolveOutcome {
                stream_target: Ok(Some(stream_target)),
                cache: sorotte_plex::PlexMatchCache::default(),
            }),
            staged_cache_write: None,
        })
        .expect("fake Plex stream result should queue");
    drop(result_tx);
    owner.plex_stream_resolve_rx = Some(result_rx);
    owner.plex_stream_resolve_result = None;

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert_eq!(owner.pending_playlist_source_resolution, None);
    assert_eq!(owner.player_local_file, Some(logical_file));
    assert_eq!(
        state.main_window.playlist[0].source_state.status,
        GuiPlaylistSourceStatus::Loading,
        "accepting the open command must not publish Active before player completion"
    );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert_eq!(
        state.main_window.playlist[0].source_state.status,
        GuiPlaylistSourceStatus::Active
    );
    assert_eq!(
        state.main_window.playlist[0].source_state.detail.as_deref(),
        Some("The attached player confirmed the Plex Stream load.")
    );
}

#[test]
fn gui_persisted_config_runtime_owner_pending_duplicate_source_tracks_entry_id_across_reorders() {
    let duplicate_label = "Episode 1.mkv";
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        plex_plugin_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("machine-1".to_owned()),
        plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(
        vec![
            duplicate_label.to_owned(),
            "Interlude.mkv".to_owned(),
            duplicate_label.to_owned(),
            "Finale.mkv".to_owned(),
        ],
        Some(2),
        false,
    );
    state.main_window.playback.can_manage_playlist = true;
    let first_duplicate_id = state.main_window.playlist[0].entry_id;
    let target_entry_id = state.main_window.playlist[2].entry_id;
    assert_ne!(first_duplicate_id, target_entry_id);

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    assert!(owner.handle_resolve_playlist_source_request(
        &handle,
        &mut state,
        2,
        GuiMediaSourceProviderId::plex_stream(),
    ));
    for action in handle.drain_actions() {
        assert!(state.apply(action));
    }
    assert!(
        owner
            .pending_playlist_source_resolution
            .as_ref()
            .is_some_and(|pending| {
                pending.index == 2
                    && pending.entry_id == target_entry_id
                    && pending.target == duplicate_label
            })
    );

    assert!(state.apply(GuiShellAction::MoveMainWindowPlaylistRow {
        from_index: 2,
        to_index: 0,
    }));
    assert_eq!(state.main_window.playlist[0].entry_id, target_entry_id);
    assert_eq!(state.main_window.playlist[1].entry_id, first_duplicate_id);
    assert!(owner.retry_pending_playlist_source_resolution(&handle, &mut state));
    for action in handle.drain_actions() {
        assert!(state.apply(action));
    }
    assert!(
        owner
            .pending_playlist_source_resolution
            .as_ref()
            .is_some_and(|pending| pending.index == 0 && pending.entry_id == target_entry_id),
        "the pending source must follow the moved duplicate row"
    );

    state.apply_shared_playlist_entries(
        vec![
            "Interlude.mkv".to_owned(),
            duplicate_label.to_owned(),
            "Finale.mkv".to_owned(),
            duplicate_label.to_owned(),
        ],
        Some(3),
        false,
    );
    let first_matching_index = state
        .main_window
        .playlist
        .iter()
        .position(|row| row.label == duplicate_label)
        .expect("a duplicate row should remain");
    let target_index = state
        .main_window
        .playlist
        .iter()
        .position(|row| row.entry_id == target_entry_id)
        .expect("the pending row identity should survive projection");
    assert_eq!(first_matching_index, 1);
    assert_eq!(target_index, 3);
    assert_eq!(
        state.main_window.playlist[first_matching_index].entry_id,
        first_duplicate_id
    );

    assert!(owner.retry_pending_playlist_source_resolution(&handle, &mut state));
    for action in handle.drain_actions() {
        assert!(state.apply(action));
    }
    assert!(
        owner
            .pending_playlist_source_resolution
            .as_ref()
            .is_some_and(|pending| pending.index == 3 && pending.entry_id == target_entry_id),
        "retry must resolve by entry_id instead of choosing the first duplicate label"
    );
    assert_eq!(
        state.main_window.playlist[target_index]
            .source_state
            .current_provider_id,
        GuiMediaSourceProviderId::plex_stream()
    );
    assert_eq!(
        state.main_window.playlist[target_index].source_state.status,
        GuiPlaylistSourceStatus::Pending
    );
    assert_eq!(
        state.main_window.playlist[first_matching_index]
            .source_state
            .current_provider_id,
        GuiMediaSourceProviderId::local()
    );
    assert_ne!(
        state.main_window.playlist[first_matching_index]
            .source_state
            .status,
        GuiPlaylistSourceStatus::Pending
    );

    let order_before_shuffle = state
        .main_window
        .playlist
        .iter()
        .map(|row| row.entry_id)
        .collect::<Vec<_>>();
    for _ in 0..16 {
        assert!(state.apply(GuiShellAction::ShuffleEntireSharedPlaylist));
        if state
            .main_window
            .playlist
            .iter()
            .map(|row| row.entry_id)
            .collect::<Vec<_>>()
            != order_before_shuffle
        {
            break;
        }
    }
    let shuffled_target_index = state
        .main_window
        .playlist
        .iter()
        .position(|row| row.entry_id == target_entry_id)
        .expect("the pending duplicate identity should survive an actual shuffle");
    assert!(owner.retry_pending_playlist_source_resolution(&handle, &mut state));
    for action in handle.drain_actions() {
        assert!(state.apply(action));
    }
    assert!(
        owner
            .pending_playlist_source_resolution
            .as_ref()
            .is_some_and(|pending| {
                pending.index == shuffled_target_index && pending.entry_id == target_entry_id
            }),
        "an actual shuffle must keep pending resolution attached to the exact duplicate row"
    );
    assert_eq!(
        state.main_window.playlist[shuffled_target_index]
            .source_state
            .status,
        GuiPlaylistSourceStatus::Pending
    );
    assert!(state.main_window.playlist.iter().any(|row| {
        row.entry_id == first_duplicate_id
            && row.source_state.status != GuiPlaylistSourceStatus::Pending
    }));

    let source_states_before_stale_retry = state
        .main_window
        .playlist
        .iter()
        .map(|row| row.source_state.clone())
        .collect::<Vec<_>>();
    owner.playlist_resolution.generation = owner.playlist_resolution.generation.wrapping_add(1);
    assert!(
        !owner.retry_pending_playlist_source_resolution(&handle, &mut state),
        "a request from an earlier playlist generation must not be rewritten onto a surviving row ID"
    );
    assert!(owner.pending_playlist_source_resolution.is_none());
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.source_state.clone())
            .collect::<Vec<_>>(),
        source_states_before_stale_retry
    );
    assert!(handle.drain_actions().is_empty());
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

    assert_eq!(outcome, SelectedPlaylistMediaSyncOutcome::StartedLoading);
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(selected_media_path.as_str())
    );
    assert_eq!(
        owner
            .playlist_resolution_attempt
            .as_ref()
            .and_then(|attempt| attempt.candidate_provider.clone()),
        Some(GuiMediaSourceProviderId::media_matching()),
        "an exact inventory path must remain attributed to Media Matching"
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
    let indexed_relative_path = std::path::PathBuf::from("Season-1")
        .join("Episode2.mkv")
        .to_string_lossy()
        .into_owned();
    write_persisted_media_search_root_index(
        &root,
        &media_root,
        built_at,
        &[
            ("episode2.mkv", &[indexed_relative_path.as_str()]),
            ("Episode2.mkv", &[indexed_relative_path.as_str()]),
        ],
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

    assert_eq!(outcome, SelectedPlaylistMediaSyncOutcome::StartedLoading);
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
    let mut owner =
        GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path)).with_session_runtime(
            Box::new(MediaMatchPeerSessionRuntimeAdapter {
                peer_files: vec![sorotte_client_core::ClientMediaMatchPeerFileState {
                    username: "remote".to_owned(),
                    has_file: true,
                    file_name: Some(playlist_target.to_owned()),
                    file_size: None,
                    file_duration: None,
                    media_match_signature: Some(
                        sorotte_media_match::MediaMatchWireSignature::default(),
                    ),
                }],
            }),
        );
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
fn gui_persisted_config_runtime_owner_manual_media_match_replaces_stale_playlist_lookup() {
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

    let root = test_temp_root("media-match-manual-selected-target-replaces-stale");
    let config_path = root.join("sorotte.ini");
    let media_root = root.join("library");
    std::fs::create_dir_all(&media_root)
        .expect("Media Match manual selection fixture directory should be created");
    let item_a = "Item A.mkv";
    let item_b = "Item B.mkv";
    let mut owner =
        GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path)).with_session_runtime(
            Box::new(MediaMatchPeerSessionRuntimeAdapter {
                peer_files: vec![sorotte_client_core::ClientMediaMatchPeerFileState {
                    username: "remote".to_owned(),
                    has_file: true,
                    file_name: Some(item_a.to_owned()),
                    file_size: None,
                    file_duration: None,
                    media_match_signature: Some(
                        sorotte_media_match::MediaMatchWireSignature::default(),
                    ),
                }],
            }),
        );
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.active_shared_playlist_index = Some(1);
    let (_stale_tx, stale_rx) = mpsc::channel();
    owner.media_match_remote_lookup_rx = Some(stale_rx);
    owner.media_match_remote_lookup_trigger_key = Some(format!("target={item_a}"));

    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![media_root.to_string_lossy().into_owned()]),
        media_matching_plugin_enabled: Some(true),
        media_match_fingerprinting_enabled: Some(true),
        media_match_wire_sharing_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(vec![item_a.to_owned(), item_b.to_owned()], Some(1), false);
    state.main_window.active_playlist_index = Some(1);
    let token = owner.media_match_remote_resolution_token_for_state(&state);
    assert!(
        token.contains(item_b),
        "remote Media Match token should track the active shared playlist item: {token}"
    );
    assert!(
        !token.contains(item_a),
        "stale peer filename should not keep the remote Media Match token on item A: {token}"
    );
    let handle = GuiQueuedRuntimeBridgeHandle::default();

    assert!(owner.handle_resolve_playlist_source_request(
        &handle,
        &mut state,
        1,
        GuiMediaSourceProviderId::media_matching(),
    ));
    for action in handle.drain_actions() {
        assert!(state.apply(action));
    }

    let trigger_key = owner
        .media_match_remote_lookup_trigger_key
        .clone()
        .expect("manual Media Match selection should queue a lookup for item B");
    assert!(
        trigger_key.contains("target=Item B.mkv"),
        "queued lookup should target item B, got {trigger_key}"
    );
    assert!(
        !trigger_key.contains("target=Item A.mkv"),
        "queued lookup should not keep waiting on item A, got {trigger_key}"
    );
    assert_eq!(
        state.main_window.playlist[1].source_state.status,
        GuiPlaylistSourceStatus::Pending
    );
    assert_eq!(
        state.main_window.playlist[1]
            .source_state
            .current_provider_id,
        GuiMediaSourceProviderId::media_matching()
    );
    assert!(
        owner
            .pending_playlist_source_resolution
            .as_ref()
            .is_some_and(|pending| pending.index == 1 && pending.target == item_b)
    );

    wait_for_media_match_remote_lookup(&mut owner);
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

    let resolved = match owner.cached_missing_media_target_path(&index, "season-1/episode2.mkv") {
        Some(GuiUserMediaTargetResolution::Resolved { path, .. }) => Some(path.replace('\\', "/")),
        other => panic!("expected one exact relative-path match, got {other:?}"),
    };
    assert_eq!(
        resolved,
        Some(exact_path.to_string_lossy().replace('\\', "/"))
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_reports_cached_same_name_across_roots_as_ambiguous() {
    let root = test_temp_root("cross-root-cache-ambiguity");
    let first_root = root.join("library-a");
    let second_root = root.join("library-b");
    std::fs::create_dir_all(&first_root).expect("first cache root should exist");
    std::fs::create_dir_all(&second_root).expect("second cache root should exist");
    std::fs::write(first_root.join("cross-root-episode.mkv"), b"first")
        .expect("first cross-root fixture should be written");
    std::fs::write(second_root.join("cross-root-episode.mkv"), b"second")
        .expect("second cross-root fixture should be written");
    let first_key = crate::app::media_search_cache::normalized_media_search_root_key(&first_root);
    let second_key = crate::app::media_search_cache::normalized_media_search_root_key(&second_root);
    let candidates = std::collections::HashMap::from([(
        "cross-root-episode.mkv".to_owned(),
        vec!["cross-root-episode.mkv".to_owned()],
    )]);
    let index = GuiAttachedMediaSearchIndex {
        roots: vec![first_key.clone(), second_key.clone()],
        root_indexes_by_key: std::collections::HashMap::from([
            (
                first_key.clone(),
                GuiAttachedMediaSearchRootIndex {
                    root_key: first_key,
                    root_path: first_root,
                    built_at_unix_ms: 1234,
                    candidates_by_name: candidates.clone(),
                },
            ),
            (
                second_key.clone(),
                GuiAttachedMediaSearchRootIndex {
                    root_key: second_key,
                    root_path: second_root,
                    built_at_unix_ms: 1234,
                    candidates_by_name: candidates,
                },
            ),
        ]),
        roots_requiring_refresh: std::collections::BTreeSet::new(),
    };
    let owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);

    assert_eq!(
        owner.cached_missing_media_target_path(&index, "cross-root-episode.mkv"),
        Some(GuiUserMediaTargetResolution::Ambiguous { candidate_count: 2 })
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_reports_direct_child_same_name_across_roots_as_ambiguous() {
    let root = test_temp_root("cross-root-direct-ambiguity");
    let first_root = root.join("library-a");
    let second_root = root.join("library-b");
    std::fs::create_dir_all(&first_root).expect("first direct root should exist");
    std::fs::create_dir_all(&second_root).expect("second direct root should exist");
    let file_name = "cross-root-direct-only-episode.mkv";
    std::fs::write(first_root.join(file_name), b"first")
        .expect("first direct cross-root fixture should be written");
    std::fs::write(second_root.join(file_name), b"second")
        .expect("second direct cross-root fixture should be written");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![
            first_root.to_string_lossy().into_owned(),
            second_root.to_string_lossy().into_owned(),
        ]),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(vec![file_name.to_owned()], Some(0), false);
    let handle = GuiQueuedRuntimeBridgeHandle::default();

    assert!(owner.handle_resolve_playlist_source_request(
        &handle,
        &mut state,
        0,
        GuiMediaSourceProviderId::local(),
    ));
    let source_state = &state.main_window.playlist[0].source_state;
    assert_eq!(source_state.status, GuiPlaylistSourceStatus::Failed);
    let detail = source_state
        .detail
        .as_deref()
        .expect("direct cross-root ambiguity should be explained");
    assert!(detail.contains("2 equally credible files"));
    assert!(!detail.contains("library-a"));
    assert!(!detail.contains("library-b"));
    assert!(owner.player_local_file.is_none());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_reports_ambiguous_cached_names_and_allows_plex_fallback() {
    let root = test_temp_root("ambiguous-basename-cache-ranking");
    let first_directory = root.join("private-choice-a");
    let second_directory = root.join("private-choice-b");
    let first_path = first_directory.join("episode2.mkv");
    let second_path = second_directory.join("episode2.mkv");
    std::fs::create_dir_all(&first_directory)
        .expect("first ambiguous fixture directory should be created");
    std::fs::create_dir_all(&second_directory)
        .expect("second ambiguous fixture directory should be created");
    std::fs::write(&first_path, b"first").expect("first ambiguous fixture should be written");
    std::fs::write(&second_path, b"second").expect("second ambiguous fixture should be written");

    let root_key = crate::app::media_search_cache::normalized_media_search_root_key(&root);
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
                        "private-choice-a/episode2.mkv".to_owned(),
                        "private-choice-b/episode2.mkv".to_owned(),
                    ],
                )]),
            },
        )]),
        roots_requiring_refresh: std::collections::BTreeSet::new(),
    };
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);

    assert_eq!(
        owner.cached_missing_media_target_path(&index, "episode2.mkv"),
        Some(GuiUserMediaTargetResolution::Ambiguous { candidate_count: 2 })
    );

    owner.attached_media_search_index = Some(index);
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        plex_plugin_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("machine-1".to_owned()),
        plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(vec!["episode2.mkv".to_owned()], Some(0), false);
    let handle = GuiQueuedRuntimeBridgeHandle::default();

    assert!(owner.handle_resolve_playlist_source_request(
        &handle,
        &mut state,
        0,
        GuiMediaSourceProviderId::local(),
    ));

    let source_state = &state.main_window.playlist[0].source_state;
    assert_eq!(source_state.status, GuiPlaylistSourceStatus::Failed);
    let detail = source_state
        .detail
        .as_deref()
        .expect("forced Local ambiguity should explain why it did not open a file");
    assert!(detail.contains("2 equally credible files"));
    assert!(!detail.contains("private-choice-a"));
    assert!(!detail.contains("private-choice-b"));
    assert!(owner.player_local_file.is_none());

    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.active_shared_playlist_index = Some(0);
    let (stream_target, logical_file) = test_plex_stream_target("episode2.mkv", "456");
    let _ = owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);
    let trigger_key = owner
        .plex_stream_resolve_trigger_key
        .clone()
        .expect("Automatic should continue to Plex after an ambiguous local result");
    let operation_context = owner
        .plex_stream_resolve_context
        .clone()
        .expect("queued Plex fallback should retain its operation context");
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    result_tx
        .send(GuiPlexStreamResolveWorkerResult {
            operation_context,
            trigger_key,
            result: Ok(GuiPlexStreamResolveOutcome {
                stream_target: Ok(Some(stream_target)),
                cache: sorotte_plex::PlexMatchCache::default(),
            }),
            staged_cache_write: None,
        })
        .expect("ambiguous-local Plex fallback should be queued");
    owner.plex_stream_resolve_rx = Some(result_rx);
    owner.plex_stream_resolve_result = None;
    assert!(owner.pump_plex_stream_resolution_worker(&state));

    let _ = owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);
    assert_eq!(owner.player_local_file, Some(logical_file));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let source_state = &state.main_window.playlist[0].source_state;
    assert_eq!(source_state.policy, GuiPlaylistSourcePolicy::Automatic);
    assert_eq!(source_state.preferred_provider_id(), None);
    assert_eq!(
        source_state.resolved_provider_id.as_ref(),
        Some(&GuiMediaSourceProviderId::plex_stream()),
        "an ordinary playlist entry resolved by Plex must display Plex without changing Automatic policy"
    );
    assert_eq!(
        source_state.current_provider_id,
        GuiMediaSourceProviderId::plex_stream()
    );
    assert_eq!(source_state.status, GuiPlaylistSourceStatus::Active);

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
