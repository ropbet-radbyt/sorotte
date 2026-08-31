use super::*;
use sorotte_client_core::ExternalPlayerAvailability;

use crate::app::runtime_bridge::{GuiSharedPlaylistOpenDispatch, GuiSharedPlaylistOpenItem};
use crate::app::runtime_owner::{
    GuiPendingPlaylistSourceResolution, GuiPendingSharedPlaylistOpen,
    player::SelectedPlaylistMediaSyncOutcome,
};
use crate::app::runtime_stack::{
    GuiClientCoreChatSessionRuntimeAdapter, GuiOutboundProtocolDeliveryResult,
    GuiPlaylistProtocolDeliveryFence, GuiQueuedSessionTransportHandle, GuiSessionRuntimeAdapter,
    GuiSessionTransportDriver,
};
use crate::app::{GuiMediaSourceProviderId, GuiPlaylistDefaultSourceId, GuiPlaylistSourceStatus};
use sorotte_plex::{
    PlexCachedMatch, PlexClientConfig, PlexMatchCache, PlexMediaType, parse_plex_playlist_uri,
    server_scoped_cache_key_for_file,
};

struct DelayedPlaylistReceiptDriver {
    release_one: std::sync::Arc<std::sync::atomic::AtomicBool>,
    writes: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    pending_token: Option<u64>,
    pending_line: Option<String>,
}

impl GuiSessionTransportDriver for DelayedPlaylistReceiptDriver {
    fn pump(&mut self, transport: &GuiQueuedSessionTransportHandle) -> Result<(), String> {
        if self.pending_token.is_none()
            && let Some(delivery) = transport.take_outbound_protocol_delivery_for_driver()
        {
            self.pending_line = Some(delivery.line().to_owned());
            self.pending_token = Some(delivery.token());
        }
        if self
            .release_one
            .swap(false, std::sync::atomic::Ordering::SeqCst)
            && let Some(token) = self.pending_token.take()
        {
            if let Some(line) = self.pending_line.take() {
                self.writes
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(line);
            }
            transport.publish_outbound_protocol_delivery_result(
                GuiOutboundProtocolDeliveryResult::FrameWritten { token },
            );
        }
        Ok(())
    }
}

struct OpenCountingPlayer {
    opens: std::sync::Arc<std::sync::Mutex<usize>>,
}

impl PlayerAdapter for OpenCountingPlayer {
    fn name(&self) -> &'static str {
        "open-counting"
    }

    fn open_file(&mut self, _path: &str) -> Result<(), sorotte_player_api::PlayerError> {
        *self
            .opens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) += 1;
        Ok(())
    }
}

fn seeded_loopback_shared_playlist_owner(
    active_index: usize,
) -> (
    GuiPersistedConfigRuntimeOwner,
    GuiQueuedRuntimeBridgeHandle,
    SorotteGuiShellAppState,
) {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode3.mkv")
            .with_path("C:/Media/episode3.mkv".to_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    handle.push_request(GuiRuntimeRequest::ReplacePlaylist {
        files: vec![
            "episode1.mkv".to_owned(),
            "episode2.mkv".to_owned(),
            "episode3.mkv".to_owned(),
        ],
        selected_index: Some(active_index),
    });
    let _ = pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| {
            state
                .main_window
                .playlist
                .iter()
                .map(|row| row.label.as_str())
                .eq(["episode1.mkv", "episode2.mkv", "episode3.mkv"])
                && state.main_window.active_playlist_index == Some(active_index)
        },
        "shared-playlist seed with active index",
    );

    (owner, handle, state)
}

#[test]
fn playlist_delivery_fence_replacement_discards_the_superseded_frontier() {
    let mut pending = GuiPendingSharedPlaylistOpen::AwaitingMutationDelivery {
        delivery_fence: GuiPlaylistProtocolDeliveryFence::new(["old-set".to_owned()]),
    };

    pending.replace_delivery_fence(GuiPlaylistProtocolDeliveryFence::new([
        "replacement-set".to_owned()
    ]));
    pending.note_frame_written("old-set");
    assert!(
        !pending.delivery_fence_reached(),
        "a receipt for the superseded mutation must not release the replacement fence"
    );
    pending.note_frame_written("replacement-set");
    assert!(pending.delivery_fence_reached());
}

#[test]
fn playlist_delivery_fence_replacement_discards_an_obsolete_open_continuation() {
    let mut pending = GuiPendingSharedPlaylistOpen::AfterMutation {
        dispatch: GuiSharedPlaylistOpenDispatch {
            items: vec![GuiSharedPlaylistOpenItem {
                published_entry: "episode1.mkv".to_owned(),
                local_origin: Some("C:/Media/episode1.mkv".to_owned()),
            }],
            imported_from_file: false,
        },
        opened_entry_count: 1,
        selected_playlist_index: Some(0),
        selected_media_source_path: Some("C:/Media/episode1.mkv".to_owned()),
        delivery_fence: GuiPlaylistProtocolDeliveryFence::new(["old-set".to_owned()]),
    };

    pending.replace_delivery_fence(GuiPlaylistProtocolDeliveryFence::new([
        "replacement-set".to_owned()
    ]));

    assert!(matches!(
        pending,
        GuiPendingSharedPlaylistOpen::AwaitingMutationDelivery { .. }
    ));
    pending.note_frame_written("replacement-set");
    assert!(pending.delivery_fence_reached());
}

#[test]
fn implicit_selected_media_changes_arm_exact_delivery_fences() {
    let release_one = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    let mut empty_owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    let empty_handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut empty_state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    pump_and_apply_runtime_owner_actions(&mut empty_owner, &empty_handle, &mut empty_state);
    empty_owner =
        empty_owner.with_session_transport_driver(Box::new(DelayedPlaylistReceiptDriver {
            release_one: release_one.clone(),
            writes: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            pending_token: None,
            pending_line: None,
        }));
    empty_handle.push_request(GuiRuntimeRequest::QueuePlaylistEntry {
        entry: "episode1.mkv".to_owned(),
        select_after_queue: false,
    });
    pump_and_apply_runtime_owner_actions(&mut empty_owner, &empty_handle, &mut empty_state);
    assert!(matches!(
        empty_owner.pending_shared_playlist_open,
        Some(GuiPendingSharedPlaylistOpen::AwaitingMutationDelivery { .. })
    ));

    let (mut replacement_owner, replacement_handle, mut replacement_state) =
        seeded_loopback_shared_playlist_owner(0);
    replacement_owner =
        replacement_owner.with_session_transport_driver(Box::new(DelayedPlaylistReceiptDriver {
            release_one,
            writes: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            pending_token: None,
            pending_line: None,
        }));
    replacement_handle.push_request(GuiRuntimeRequest::ReplacePlaylist {
        files: vec!["replacement.mkv".to_owned()],
        selected_index: None,
    });
    pump_and_apply_runtime_owner_actions(
        &mut replacement_owner,
        &replacement_handle,
        &mut replacement_state,
    );
    assert!(matches!(
        replacement_owner.pending_shared_playlist_open,
        Some(GuiPendingSharedPlaylistOpen::AwaitingMutationDelivery { .. })
    ));
}

#[test]
fn session_causal_player_effect_cleanup_cancels_a_pending_playlist_fence() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.pending_shared_playlist_open =
        Some(GuiPendingSharedPlaylistOpen::AwaitingMutationDelivery {
            delivery_fence: GuiPlaylistProtocolDeliveryFence::new(["pending-set".to_owned()]),
        });

    owner.clear_session_causal_player_effect_state();

    assert!(owner.pending_shared_playlist_open.is_none());
}

#[test]
fn client_core_session_bootstrap_preserves_username_and_room_in_the_hello() {
    let (mut owner, _) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core session runtime should bootstrap");
    let lines = owner
        .session
        .as_deref_mut()
        .expect("client-core runtime should be installed")
        .flush_outbound_protocol_lines()
        .expect("startup Hello should encode");
    let hello = lines
        .iter()
        .find_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .and_then(|value| value.get("Hello").cloned())
        .expect("startup output should contain Hello");

    assert_eq!(
        hello.get("username").and_then(|value| value.as_str()),
        Some("alice")
    );
    assert_eq!(
        hello
            .get("room")
            .and_then(|room| room.get("name"))
            .and_then(|value| value.as_str()),
        Some("room1")
    );
}

#[test]
fn every_selected_playlist_mutator_returns_its_real_protocol_delivery_fence() {
    type Mutation = fn(
        &mut (dyn GuiSessionRuntimeAdapter + Send),
    ) -> Result<GuiPlaylistProtocolDeliveryFence, String>;

    let cases: [(&str, Mutation); 5] = [
        ("advance", |session| {
            session.advance_playlist_index_with_delivery_fence()
        }),
        ("delete", |session| {
            session.delete_playlist_index_with_delivery_fence(1)
        }),
        ("undo", |session| {
            session.undo_playlist_change_with_delivery_fence()
        }),
        ("shuffle remaining", |session| {
            session.shuffle_remaining_playlist_with_delivery_fence()
        }),
        ("shuffle entire", |session| {
            session.shuffle_entire_playlist_with_delivery_fence()
        }),
    ];

    for (case, mutate) in cases {
        let mut session = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
            .expect("client-core session adapter should bootstrap");
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"sharedPlaylists":true}}}"#,
            )
            .expect("shared-playlist server Hello fixture should apply");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv","episode3.mkv"],"user":"alice"}}}"#,
            )
            .expect("authoritative playlist files should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
            .expect("authoritative playlist index should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"file":{"name":"episode1.mkv","duration":240.0}}}}}"#,
            )
            .expect("authoritative local file should apply");
        if case == "undo" {
            session
                .replace_playlist_with_delivery_fence(
                    vec!["episode3.mkv".to_owned(), "episode2.mkv".to_owned()],
                    Some(0),
                )
                .expect("undo case should first create local playlist history");
        }
        let fence = mutate(&mut session)
            .unwrap_or_else(|error| panic!("{case} should queue a playlist mutation: {error}"));
        assert!(
            fence.pending_frame_count() > 0,
            "{case} must return the exact nonempty protocol delivery frontier"
        );
    }
}

fn seed_cached_plex_match_for_local_path(
    root: &std::path::Path,
    media_path: &std::path::Path,
    rating_key: &str,
    title: &str,
) {
    let plex_config = PlexClientConfig {
        enabled: true,
        streaming_enabled: true,
        user_token: Some("user-token".into()),
        selected_server_id: Some("machine-1".to_owned()),
        selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        selected_server_token: Some("server-token".into()),
    };
    let metadata = std::fs::metadata(media_path)
        .expect("cached Plex local-media fixture metadata should be readable");
    let file_name = media_path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("cached Plex local-media fixture should have a UTF-8 file name");
    let local_file = sorotte_player_api::LocalFileUpdate::new(file_name)
        .with_path(media_path.to_string_lossy().into_owned())
        .with_size_bytes(metadata.len());
    let cache_key = server_scoped_cache_key_for_file(&plex_config, &local_file)
        .expect("server-scoped cache key should be available");
    let mut cache = PlexMatchCache::default();
    cache.entries.insert(
        cache_key,
        PlexCachedMatch {
            rating_key: rating_key.to_owned(),
            title: title.to_owned(),
            media_type: PlexMediaType::Episode,
            duration_millis: Some(90_000),
        },
    );
    cache
        .save_to_path(&root.join("cache").join("plex-watch-cache.json"))
        .expect("Plex cache should be written");
}

#[test]
fn gui_persisted_config_runtime_owner_publishes_cached_plex_uri_for_shared_local_media_open() {
    let root = test_temp_root("shared-playlist-local-plex-uri");
    let config_path = root.join("sorotte.ini");
    let media_dir = root.join("Media");
    std::fs::create_dir_all(&media_dir)
        .expect("shared-playlist Plex fixture directory should be created");
    let media_path = media_dir.join("episode1.mkv");
    std::fs::write(&media_path, b"test").expect("shared-playlist Plex fixture should be written");
    let media_path_text = media_path.to_string_lossy().into_owned();

    let plex_config = PlexClientConfig {
        enabled: true,
        streaming_enabled: true,
        user_token: Some("user-token".into()),
        selected_server_id: Some("machine-1".to_owned()),
        selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        selected_server_token: Some("server-token".into()),
    };
    let local_file = sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
        .with_path(media_path_text.clone())
        .with_size_bytes(4);
    let cache_key = server_scoped_cache_key_for_file(&plex_config, &local_file)
        .expect("server-scoped cache key should be available");
    let mut cache = PlexMatchCache::default();
    cache.entries.insert(
        cache_key,
        PlexCachedMatch {
            rating_key: "123".to_owned(),
            title: "Episode 1".to_owned(),
            media_type: PlexMediaType::Episode,
            duration_millis: Some(90_000),
        },
    );
    cache
        .save_to_path(&root.join("cache").join("plex-watch-cache.json"))
        .expect("Plex cache should be written");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path))
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        plex_sync_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("machine-1".to_owned()),
        plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![media_path_text.clone()],
        load_into_shared_playlist: true,
        playlist_insert_slot: None,
    });
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| state.current_shared_playlist_entries().len() == 1,
        "shared-playlist local media open should publish Plex URI",
    );

    let entries = state.current_shared_playlist_entries();
    let uri =
        parse_plex_playlist_uri(&entries[0]).expect("shared playlist entry should be a Plex URI");
    assert_eq!(uri.machine_identifier, "machine-1");
    assert_eq!(uri.rating_key, "123");
    assert_eq!(uri.file_name.as_deref(), Some("episode1.mkv"));
    assert_eq!(uri.title.as_deref(), Some("Episode 1"));
    assert_eq!(uri.duration_millis, Some(90_000));
    assert_eq!(uri.size_bytes, Some(4));
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(media_path_text.as_str())
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_automatic_cached_plex_drop_prefers_exact_local_media() {
    let root = test_temp_root("shared-playlist-automatic-cached-plex-local");
    let config_path = root.join("sorotte.ini");
    let drop_dir = root.join("Dropped");
    std::fs::create_dir_all(&drop_dir)
        .expect("cached Plex local-drop fixture directory should be created");
    let media_path = drop_dir.join("episode1.mkv");
    std::fs::write(&media_path, b"test").expect("cached Plex local-drop fixture should be written");
    let media_path_text = media_path.to_string_lossy().into_owned();
    seed_cached_plex_match_for_local_path(&root, &media_path, "123", "Episode 1");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path))
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        plex_plugin_enabled: Some(true),
        plex_sync_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("machine-1".to_owned()),
        plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert_eq!(
        state.main_window.playlist_default_source.current_source_id,
        GuiPlaylistDefaultSourceId::automatic()
    );

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![media_path_text.clone()],
        load_into_shared_playlist: true,
        playlist_insert_slot: None,
    });
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| state.current_shared_playlist_entries().len() == 1,
        "Automatic cached-Plex local drop should project into the shared playlist",
    );

    let entries = state.current_shared_playlist_entries();
    let uri = parse_plex_playlist_uri(&entries[0])
        .expect("peer-facing shared playlist entry should remain a Plex URI");
    assert_eq!(uri.machine_identifier, "machine-1");
    assert_eq!(uri.rating_key, "123");
    assert_eq!(
        state.main_window.playlist[0]
            .source_state
            .current_provider_id,
        GuiMediaSourceProviderId::local(),
        "Automatic should project a known local drop as Local even when its peer-facing entry is a Plex URI"
    );
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(media_path_text.as_str()),
        "the selected drop should open the exact local filesystem path"
    );
    assert!(
        owner.plex_stream_resolve_rx.is_none(),
        "Automatic local precedence must not queue Plex stream resolution"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_automatic_cached_plex_duplicate_drop_retains_exact_local_media()
 {
    let root = test_temp_root("shared-playlist-automatic-cached-plex-duplicate-local");
    let config_path = root.join("sorotte.ini");
    let drop_dir = root.join("DroppedOutsideSearchRoots");
    std::fs::create_dir_all(&drop_dir)
        .expect("cached Plex duplicate-drop fixture directory should be created");
    let current_media_path = drop_dir.join("current.mkv");
    let media_path = drop_dir.join("episode1.mkv");
    std::fs::write(&current_media_path, b"current")
        .expect("current local-media fixture should be written");
    std::fs::write(&media_path, b"test")
        .expect("cached Plex duplicate-drop fixture should be written");
    let current_media_path_text = current_media_path.to_string_lossy().into_owned();
    let media_path_text = media_path.to_string_lossy().into_owned();
    seed_cached_plex_match_for_local_path(&root, &media_path, "123", "Episode 1");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path))
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("current.mkv")
            .with_path(current_media_path_text.clone()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        plex_plugin_enabled: Some(true),
        plex_sync_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("machine-1".to_owned()),
        plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let dispatch = owner
        .shared_playlist_open_dispatch_for_selected_paths_impl(
            &state,
            vec![media_path_text.clone()],
        )
        .expect("cached local file should produce a shared-playlist dispatch");
    let plex_uri = dispatch
        .playlist_entries()
        .first()
        .cloned()
        .expect("cached local file should produce a Plex playlist URI");
    parse_plex_playlist_uri(&plex_uri)
        .expect("cached local file should publish a peer-facing Plex URI");

    handle.push_request(GuiRuntimeRequest::ReplacePlaylist {
        files: vec!["current.mkv".to_owned(), plex_uri.clone()],
        selected_index: Some(0),
    });
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| {
            state.current_shared_playlist_entries() == ["current.mkv".to_owned(), plex_uri.clone()]
                && state.main_window.active_playlist_index == Some(0)
        },
        "Automatic Plex row should be seeded behind the active local row",
    );
    assert_eq!(
        state.main_window.playlist[1]
            .source_state
            .current_provider_id,
        GuiMediaSourceProviderId::plex_stream()
    );
    assert_eq!(
        state.main_window.playlist[1].source_state.policy,
        GuiPlaylistSourcePolicy::Automatic
    );
    assert_eq!(
        state.main_window.playlist[1].source_state.selection_origin,
        GuiPlaylistSourceSelectionOrigin::Inferred,
        "the seeded Plex source should be inferred by Automatic"
    );

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![media_path_text.clone()],
        load_into_shared_playlist: true,
        playlist_insert_slot: Some(2),
    });
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| {
            state.main_window.playlist[1]
                .source_state
                .current_provider_id
                == GuiMediaSourceProviderId::local()
        },
        "duplicate cached-Plex drop should update the existing Automatic row to Local",
    );

    assert_eq!(
        state.current_shared_playlist_entries(),
        vec!["current.mkv".to_owned(), plex_uri],
        "duplicate append should remain a playlist no-op"
    );
    assert_eq!(
        state.main_window.playlist[1].source_state.policy,
        GuiPlaylistSourcePolicy::Automatic,
        "Automatic local precedence should remain policy-driven"
    );
    assert_eq!(
        state.main_window.playlist[1].source_state.selection_origin,
        GuiPlaylistSourceSelectionOrigin::Inferred,
        "Automatic local precedence should remain inferred rather than becoming a manual override"
    );
    assert_eq!(state.main_window.active_playlist_index, Some(0));
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(current_media_path_text.as_str()),
        "the duplicate append should not interrupt the active local row"
    );
    assert!(owner.plex_stream_resolve_rx.is_none());

    handle.push_request(GuiRuntimeRequest::SetPlaylistIndex(1));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while std::time::Instant::now() < deadline {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        if owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref())
            == Some(media_path_text.as_str())
            || owner.plex_stream_resolve_rx.is_some()
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    assert_eq!(state.main_window.active_playlist_index, Some(1));
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(media_path_text.as_str()),
        "activating the deduplicated row should use the exact dropped filesystem path"
    );
    assert!(
        owner.plex_stream_resolve_rx.is_none(),
        "activating a deduplicated local drop must not queue Plex stream resolution"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_deduplicated_local_drop_retains_first_exact_path() {
    let root = test_temp_root("shared-playlist-deduplicated-local-path-order");
    let first_dir = root.join("First");
    let second_dir = root.join("Second");
    std::fs::create_dir_all(&first_dir).expect("first duplicate-path directory should be created");
    std::fs::create_dir_all(&second_dir)
        .expect("second duplicate-path directory should be created");
    let first_path = first_dir.join("episode.mkv");
    let second_path = second_dir.join("episode.mkv");
    std::fs::write(&first_path, b"first").expect("first duplicate-path file should be written");
    std::fs::write(&second_path, b"second").expect("second duplicate-path file should be written");
    let first_path_text = first_path.to_string_lossy().into_owned();
    let second_path_text = second_path.to_string_lossy().into_owned();

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    owner.open_media_files_through_shared_playlist_runtime_impl(
        &handle,
        &mut state,
        vec![first_path_text.clone(), second_path_text],
        Some(0),
    );

    assert_eq!(
        state
            .current_shared_playlist_entries()
            .iter()
            .filter(|entry| entry.as_str() == "episode.mkv")
            .count(),
        1,
        "duplicate local targets should collapse to one accepted playlist entry"
    );
    assert_eq!(owner.playlist_resolution.local_origins_by_row.len(), 1);
    assert_eq!(
        owner
            .playlist_resolution
            .local_origins_by_row
            .values()
            .next(),
        Some(&std::path::PathBuf::from(&first_path_text)),
        "deduplication must retain the exact path paired with the first accepted entry"
    );
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(first_path_text.as_str())
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn deduplicated_local_origin_updates_publish_snapshot_for_bound_and_missing_rows() {
    let root = test_temp_root("shared-playlist-deduplicated-local-origin-truth-table");
    let bound_path = root.join("bound.mkv");
    let missing_path = root.join("missing.mkv");
    std::fs::write(&bound_path, b"bound").expect("bound local-origin fixture should exist");
    let bound_path_text = bound_path.to_string_lossy().into_owned();
    let missing_path_text = missing_path.to_string_lossy().into_owned();

    for (case, include_bound_row, include_missing_row) in [
        ("bound only", true, false),
        ("missing only", false, true),
        ("bound and missing", true, true),
    ] {
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        let handle = GuiQueuedRuntimeBridgeHandle::default();
        let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            username: Some("alice".to_owned()),
            room: Some("room1".to_owned()),
            shared_playlist_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        });
        let mut entries = Vec::new();
        if include_bound_row {
            entries.push("bound.mkv".to_owned());
        }
        if include_missing_row {
            entries.push("missing.mkv".to_owned());
        }
        state.apply_shared_playlist_entries(entries.clone(), Some(0), false);
        assert!(
            state
                .main_window
                .playlist
                .iter()
                .all(|row| row.source_state.status == GuiPlaylistSourceStatus::Available),
            "{case}: every existing row must begin available"
        );

        let mut items = Vec::new();
        if include_bound_row {
            items.push(GuiSharedPlaylistOpenItem {
                published_entry: "bound.mkv".to_owned(),
                local_origin: Some(bound_path_text.clone()),
            });
        }
        if include_missing_row {
            items.push(GuiSharedPlaylistOpenItem {
                published_entry: "missing.mkv".to_owned(),
                local_origin: Some(missing_path_text.clone()),
            });
        }
        owner.open_shared_playlist_dispatch_runtime_impl(
            &handle,
            &mut state,
            items
                .iter()
                .filter_map(|item| item.local_origin.clone())
                .collect(),
            GuiSharedPlaylistOpenDispatch {
                items,
                imported_from_file: false,
            },
            Some(entries.len()),
        );

        let snapshots = handle
            .drain_actions()
            .into_iter()
            .filter(|action| matches!(action, GuiShellAction::ApplyMainWindowRuntimeSnapshot(_)))
            .count();
        assert_eq!(
            snapshots, 1,
            "{case}: every nonempty local-origin outcome must publish its source-state changes"
        );
        if include_bound_row {
            let bound_row = state
                .main_window
                .playlist
                .iter()
                .find(|row| row.label == "bound.mkv")
                .expect("bound row should remain projected");
            assert_eq!(
                bound_row.source_state.current_provider_id,
                GuiMediaSourceProviderId::local()
            );
            assert_eq!(
                bound_row.source_state.detail.as_deref(),
                Some("Added from the local filesystem.")
            );
        }
        if include_missing_row {
            let missing_row = state
                .main_window
                .playlist
                .iter()
                .find(|row| row.label == "missing.mkv")
                .expect("missing row should remain projected");
            assert_eq!(
                missing_row.source_state.status,
                GuiPlaylistSourceStatus::Missing
            );
            assert_eq!(
                missing_row.source_state.detail.as_deref(),
                Some("The selected local file is no longer available.")
            );
        }
        assert_eq!(
            state.current_shared_playlist_entries(),
            entries,
            "{case}: deduplicated existing-row bindings must not manufacture a playlist mutation"
        );
    }

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn successful_insert_preserves_pending_source_resolution_while_replacement_supersedes_it() {
    struct OpenRecordingPlayer {
        opened_paths: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl PlayerAdapter for OpenRecordingPlayer {
        fn name(&self) -> &'static str {
            "open-recording"
        }

        fn open_file(&mut self, path: &str) -> Result<(), sorotte_player_api::PlayerError> {
            self.opened_paths
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(path.to_owned());
            Ok(())
        }
    }

    let root = test_temp_root("shared-playlist-insert-replacement-source-resolution-truth-table");
    let current_path = root.join("current.mkv");
    let next_path = root.join("next.mkv");
    std::fs::write(&current_path, b"current").expect("current media fixture should exist");
    std::fs::write(&next_path, b"next").expect("next media fixture should exist");
    let next_path_text = next_path.to_string_lossy().into_owned();

    for (case, insert_slot, expected_generation_delta, expected_opens) in [
        ("insert", Some(1), 0, Vec::<String>::new()),
        ("replacement", None, 1, vec![next_path_text.clone()]),
    ] {
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
            .with_client_core_chat_loopback_session_runtime("alice", "room1")
            .expect("client-core loopback runtime owner should bootstrap");
        let opened_paths = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        owner.player = Some(GuiOwnedPlayer::Custom(Box::new(OpenRecordingPlayer {
            opened_paths: opened_paths.clone(),
        })));
        let handle = GuiQueuedRuntimeBridgeHandle::default();
        let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            username: Some("alice".to_owned()),
            room: Some("room1".to_owned()),
            player_path: Some("mpv".to_owned()),
            shared_playlist_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        });
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        owner.session_transport = None;
        owner.session_transport_driver = None;
        state.apply_shared_playlist_entries(vec!["current.mkv".to_owned()], Some(0), false);
        owner.active_shared_playlist_index = Some(0);
        owner.reconcile_local_shared_playlist_media_paths(&state);
        let current_entry_id = state.main_window.playlist[0].entry_id;
        owner
            .playlist_resolution
            .local_origins_by_row
            .insert(current_entry_id, current_path.clone());
        owner.pending_playlist_source_resolution = Some(GuiPendingPlaylistSourceResolution {
            index: 0,
            entry_id: current_entry_id,
            generation: owner.playlist_resolution.generation,
            target: "current.mkv".to_owned(),
            provider_id: GuiMediaSourceProviderId::media_matching(),
        });
        let generation_before = owner.playlist_resolution.generation;

        owner.open_shared_playlist_dispatch_runtime_impl(
            &handle,
            &mut state,
            vec![next_path_text.clone()],
            GuiSharedPlaylistOpenDispatch {
                items: vec![GuiSharedPlaylistOpenItem {
                    published_entry: "next.mkv".to_owned(),
                    local_origin: Some(next_path_text.clone()),
                }],
                imported_from_file: false,
            },
            insert_slot,
        );

        assert_eq!(
            owner.playlist_resolution.generation,
            generation_before + expected_generation_delta,
            "{case} must obey the replacement-scope truth table"
        );
        assert_eq!(
            owner.pending_playlist_source_resolution.is_some(),
            insert_slot.is_some(),
            "{case} must preserve a still-relevant source resolution only for insertion"
        );
        assert_eq!(
            *opened_paths
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            expected_opens,
            "{case} must not bypass a pending source-resolution handoff"
        );
        assert_eq!(
            state.current_shared_playlist_entries(),
            if insert_slot.is_some() {
                vec!["current.mkv".to_owned(), "next.mkv".to_owned()]
            } else {
                vec!["next.mkv".to_owned()]
            }
        );
        if insert_slot.is_some() {
            assert_eq!(state.main_window.active_playlist_index, Some(0));
            assert_eq!(
                owner
                    .playlist_resolution
                    .local_origins_by_row
                    .get(&current_entry_id),
                Some(&current_path)
            );
        }
    }

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_reactivates_cached_plex_append_from_exact_dropped_path() {
    let root = test_temp_root("shared-playlist-cached-plex-append-local-hint");
    let config_path = root.join("sorotte.ini");
    let search_dir = root.join("ConfiguredMedia");
    let drop_dir = root.join("DroppedOutsideSearchRoots");
    std::fs::create_dir_all(&search_dir)
        .expect("configured media-search fixture directory should be created");
    std::fs::create_dir_all(&drop_dir)
        .expect("outside-root local-drop fixture directory should be created");
    let current_media_path = search_dir.join("episode1.mkv");
    let dropped_media_path = drop_dir.join("episode2.mkv");
    std::fs::write(&current_media_path, b"first")
        .expect("active local-media fixture should be written");
    std::fs::write(&dropped_media_path, b"second")
        .expect("appended local-media fixture should be written");
    let current_media_path_text = current_media_path.to_string_lossy().into_owned();
    let dropped_media_path_text = dropped_media_path.to_string_lossy().into_owned();
    seed_cached_plex_match_for_local_path(&root, &dropped_media_path, "456", "Episode 2");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path))
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![search_dir.to_string_lossy().into_owned()]),
        plex_plugin_enabled: Some(true),
        plex_sync_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("machine-1".to_owned()),
        plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![current_media_path_text.clone()],
        load_into_shared_playlist: true,
        playlist_insert_slot: None,
    });
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| {
            state.current_shared_playlist_entries().len() == 1
                && state.main_window.active_playlist_index == Some(0)
        },
        "initial local row should remain active before the cached-Plex append",
    );
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(current_media_path_text.as_str())
    );

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![dropped_media_path_text.clone()],
        load_into_shared_playlist: true,
        playlist_insert_slot: Some(1),
    });
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| state.current_shared_playlist_entries().len() == 2,
        "cached-Plex local drop should append without switching the active row",
    );

    assert_eq!(state.main_window.active_playlist_index, Some(0));
    assert!(
        parse_plex_playlist_uri(&state.current_shared_playlist_entries()[1]).is_ok(),
        "peer-facing appended entry should remain a Plex URI"
    );
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(current_media_path_text.as_str()),
        "appending should not interrupt the currently active local row"
    );
    assert!(owner.plex_stream_resolve_rx.is_none());

    handle.push_request(GuiRuntimeRequest::SetPlaylistIndex(1));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while std::time::Instant::now() < deadline {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        if owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref())
            == Some(dropped_media_path_text.as_str())
            || owner.plex_stream_resolve_rx.is_some()
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    assert_eq!(state.main_window.active_playlist_index, Some(1));
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(dropped_media_path_text.as_str()),
        "later activation should reopen the exact appended path even though it is outside configured search roots"
    );
    assert!(
        owner.plex_stream_resolve_rx.is_none(),
        "later activation of a retained local drop must not queue Plex stream resolution"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_keeps_uncached_plex_local_add_on_fast_path() {
    let root = test_temp_root("shared-playlist-local-plex-cache-miss");
    let config_path = root.join("sorotte.ini");
    let media_dir = root.join("Media");
    std::fs::create_dir_all(&media_dir)
        .expect("shared-playlist Plex miss fixture directory should be created");
    let media_path = media_dir.join("episode1.mkv");
    std::fs::write(&media_path, b"test").expect("shared-playlist Plex miss fixture should exist");
    let media_path_text = media_path.to_string_lossy().into_owned();

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path));
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        plex_sync_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("machine-1".to_owned()),
        plex_selected_server_url: Some("not-a-valid-url".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    });

    let dispatch = owner
        .shared_playlist_open_dispatch_for_selected_paths_impl(
            &state,
            vec![media_path_text.clone()],
        )
        .expect("uncached Plex local add should still produce a playlist entry");

    assert_eq!(dispatch.playlist_entries(), vec!["episode1.mkv".to_owned()]);
    assert_eq!(
        dispatch.items[0].local_origin.as_deref(),
        Some(media_path_text.as_str())
    );
    assert!(
        owner.pending_stream_feedback.is_empty(),
        "playlist publication must not run uncached Plex stream resolution before the row is projected"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_routes_shared_playlist_open_through_client_core_session_and_player()
 {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let media_root = test_temp_root("shared-playlist-loopback-player");
    let episode1_path = media_root.join("episode1.mkv");
    let episode2_path = media_root.join("episode2.mkv");
    std::fs::write(&episode1_path, b"one").expect("first media fixture should be written");
    std::fs::write(&episode2_path, b"two").expect("second media fixture should be written");
    let episode1_path_text = episode1_path.to_string_lossy().into_owned();

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![
            episode1_path_text.clone(),
            episode2_path.to_string_lossy().into_owned(),
        ],
        load_into_shared_playlist: true,
        playlist_insert_slot: None,
    });
    let actions = pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| state.main_window.playlist.len() == 2,
        "shared-playlist open through loopback session and player",
    );

    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Success
                    && message == "Loaded 2 selected media entries into the shared playlist."
        )),
        "shared-playlist open should report playlist-backed success"
    );
    assert_eq!(state.active_view, GuiShellView::Room);
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.clone())
            .collect::<Vec<_>>(),
        vec!["episode1.mkv".to_owned(), "episode2.mkv".to_owned()]
    );
    assert_eq!(state.selection.selected_main_window_playlist, Some(0));
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .map(|file| file.name.as_str()),
        Some("episode1.mkv")
    );
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(episode1_path_text.as_str())
    );
    let _ = std::fs::remove_dir_all(media_root);
}

#[test]
fn same_selected_shared_media_reopen_after_player_replacement_does_not_rearm_a_canonical_reset() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    let transport = owner
        .session_transport
        .as_ref()
        .expect("loopback owner should expose its transport")
        .clone();

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let media_root = test_temp_root("same-selection-player-replacement");
    let media_path = media_root.join("episode1.mkv");
    std::fs::write(&media_path, b"test").expect("media fixture should be written");
    let media_path_text = media_path.to_string_lossy().into_owned();
    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![media_path_text.clone()],
        load_into_shared_playlist: true,
        playlist_insert_slot: None,
    });
    let _ = pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| state.main_window.active_playlist_index == Some(0),
        "initial selected shared-media open",
    );
    transport.push_inbound_protocol_line(
        r#"{"State":{"playstate":{"position":3.0,"paused":true,"doSeek":false,"setBy":"alice"}}}"#
            .to_owned(),
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while std::time::Instant::now() < deadline
        && owner
            .session
            .as_ref()
            .expect("session should remain installed")
            .has_pending_playlist_index_reset_intent()
    {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        !owner
            .session
            .as_ref()
            .expect("session should remain installed")
            .has_pending_playlist_index_reset_intent(),
        "physical media confirmation plus post-selection State should complete the initial reset"
    );

    // Model a fresh physical transport which has not yet reported its file,
    // while preserving the already-selected canonical playlist row.
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.player_local_file = None;
    owner.player_local_file_placeholder = false;
    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![media_path_text.clone()],
        load_into_shared_playlist: true,
        playlist_insert_slot: None,
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while std::time::Instant::now() < deadline
        && owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref())
            != Some(media_path_text.as_str())
    {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(media_path_text.as_str()),
        "the replacement physical transport should load the already-selected row"
    );

    assert!(
        !owner
            .session
            .as_ref()
            .expect("session should remain installed")
            .has_pending_playlist_index_reset_intent(),
        "a physical reload of the selected row must not manufacture a canonical selection fence"
    );

    let _ = std::fs::remove_dir_all(media_root);
}

#[test]
fn gui_persisted_config_runtime_owner_flushes_shared_playlist_before_player_open() {
    struct RecordingTransportDriver {
        writes: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl GuiSessionTransportDriver for RecordingTransportDriver {
        fn pump(&mut self, transport: &GuiQueuedSessionTransportHandle) -> Result<(), String> {
            let Some(delivery) = transport.take_outbound_protocol_delivery_for_driver() else {
                return Ok(());
            };
            self.writes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(delivery.line().to_owned());
            transport.publish_outbound_protocol_delivery_result(
                GuiOutboundProtocolDeliveryResult::FrameWritten {
                    token: delivery.token(),
                },
            );
            Ok(())
        }
    }

    struct OutboundObservingPlayer {
        transport: crate::app::runtime_stack::GuiQueuedSessionTransportHandle,
        driver_writes: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        observed_outbound: std::sync::Arc<std::sync::Mutex<Vec<Vec<String>>>>,
    }

    impl PlayerAdapter for OutboundObservingPlayer {
        fn name(&self) -> &'static str {
            "outbound-observing"
        }

        fn open_file(&mut self, _path: &str) -> Result<(), sorotte_player_api::PlayerError> {
            let mut outbound = self
                .driver_writes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            outbound.extend(self.transport.drain_outbound_protocol_lines());
            self.observed_outbound
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(outbound);
            Ok(())
        }
    }

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    let transport = owner
        .session_transport
        .as_ref()
        .expect("loopback owner should expose a session transport")
        .clone();
    let observed_outbound = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let preopen_writes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(OutboundObservingPlayer {
        transport: transport.clone(),
        driver_writes: preopen_writes.clone(),
        observed_outbound: observed_outbound.clone(),
    })));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let _ = transport.drain_outbound_protocol_lines();
    let _ = owner
        .session
        .as_mut()
        .expect("loopback session should remain installed")
        .flush_outbound_protocol_lines()
        .expect("startup participant status should be acknowledged before the ordering fixture");
    owner = owner.with_session_transport_driver(Box::new(RecordingTransportDriver {
        writes: preopen_writes.clone(),
    }));

    // Reproduce the ordering pressure introduced by participant status: the
    // first-State user-list refresh queues a reliable List, then the player
    // transition queues a coalescible status State behind it. The media open
    // must still observe the later playlist Set before touching the player.
    assert!(
        owner
            .session
            .as_mut()
            .expect("loopback session should remain installed")
            .request_user_list()
            .expect("first-State user-list refresh should queue"),
        "first-State user-list refresh should produce a reliable List"
    );
    for backlog_index in 0..200 {
        owner
            .session
            .as_mut()
            .expect("loopback session should remain installed")
            .send_chat_message(format!("reliable-backlog-{backlog_index}"))
            .expect("reliable backlog chat should queue");
    }
    owner.report_external_player_availability(ExternalPlayerAvailability::Connecting);
    let pending_list = owner
        .session
        .as_mut()
        .expect("loopback session should remain installed")
        .begin_outbound_protocol_delivery()
        .expect("pending first-State List should stage")
        .expect("first-State List should remain pending");
    assert!(
        pending_list.line().contains("\"List\""),
        "participant-status transition should remain behind the first-State List: {}",
        pending_list.line()
    );
    owner
        .session
        .as_mut()
        .expect("loopback session should remain installed")
        .fail_outbound_protocol_delivery(pending_list.token())
        .expect("precondition inspection should restore the pending List");

    let media_root = test_temp_root("shared-playlist-flush-before-open");
    let media_path = media_root.join("episode1.mkv");
    std::fs::write(&media_path, b"test").expect("media fixture should be written");

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![media_path.to_string_lossy().into_owned()],
        load_into_shared_playlist: true,
        playlist_insert_slot: None,
    });
    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let observed = observed_outbound
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let outbound_at_open = observed
        .first()
        .expect("player open should observe outbound transport state");
    assert!(
        preopen_writes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
            > 128,
        "the production driver path should drain beyond both ordinary capped pumps before opening"
    );
    assert!(
        outbound_at_open
            .iter()
            .any(|line| line.contains("episode1.mkv")),
        "shared playlist transport update must be flushed before player open; outbound_at_open={outbound_at_open:?}"
    );
    let _ = std::fs::remove_dir_all(media_root);
}

#[test]
fn gui_persisted_config_runtime_owner_resumes_open_after_delayed_playlist_delivery() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let release_one = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let writes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    owner = owner.with_session_transport_driver(Box::new(DelayedPlaylistReceiptDriver {
        release_one: release_one.clone(),
        writes: writes.clone(),
        pending_token: None,
        pending_line: None,
    }));
    owner
        .session
        .as_mut()
        .expect("loopback session should remain installed")
        .request_user_list()
        .expect("reliable List should queue");
    owner.report_external_player_availability(ExternalPlayerAvailability::Connecting);
    let opens = std::sync::Arc::new(std::sync::Mutex::new(0));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(OpenCountingPlayer {
        opens: opens.clone(),
    })));

    let media_root = test_temp_root("shared-playlist-backpressured-before-open");
    let media_path = media_root.join("episode1.mkv");
    std::fs::write(&media_path, b"test").expect("media fixture should be written");
    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![media_path.to_string_lossy().into_owned()],
        load_into_shared_playlist: true,
        playlist_insert_slot: None,
    });
    let waiting_actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert_eq!(
        *opens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        0,
        "an asynchronous transport receipt must fence the local player open"
    );
    assert!(owner.pending_shared_playlist_open.is_some());
    assert!(!waiting_actions.iter().any(|action| {
        matches!(
            action,
            GuiShellAction::PushTransientNotification { message, .. }
                if message.contains("backpressured")
        )
    }));

    let pending_playlist_frames = match owner.pending_shared_playlist_open.as_ref() {
        Some(GuiPendingSharedPlaylistOpen::AfterMutation { delivery_fence, .. }) => {
            delivery_fence.pending_frame_count()
        }
        _ => panic!("the open should wait on the exact playlist delivery frontier"),
    };
    assert!(pending_playlist_frames > 0);
    assert_eq!(
        *opens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        0,
        "the selected-media reconciler must not bypass the post-mutation delivery fence"
    );

    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert_eq!(
        *opens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        0,
        "another runtime pump must keep the local open fenced until its receipt"
    );
    assert!(matches!(
        owner.pending_shared_playlist_open,
        Some(GuiPendingSharedPlaylistOpen::AfterMutation { .. })
    ));
    owner.detach_player();
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(OpenCountingPlayer {
        opens: opens.clone(),
    })));
    owner.sync_active_shared_playlist_media_and_playstate_impl(&state);
    assert!(matches!(
        owner.pending_shared_playlist_open,
        Some(GuiPendingSharedPlaylistOpen::AfterMutation { .. })
    ));
    assert_eq!(
        *opens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        0,
        "a player attachment transition must retain an unsatisfied playlist delivery fence"
    );
    assert_eq!(
        owner.open_selected_playlist_media_path_through_attached_player_impl(
            &state,
            &[media_path.to_string_lossy().into_owned()],
        ),
        SelectedPlaylistMediaSyncOutcome::NoChange,
        "all source-resolution paths must converge on the player-effect fence"
    );
    assert_eq!(
        *opens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        0,
        "the centralized source-resolution guard must contain the player side effect"
    );

    // Withdraw participant-status capability while its unleased State is
    // behind the staged List. The core cancels that coalescible tail, which
    // must not strand a playlist fence captured before the cancellation.
    owner
        .session
        .as_mut()
        .expect("loopback session should remain installed")
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{}}}"#,
        )
        .expect("capability withdrawal should apply");

    assert!(owner.handle_resolve_playlist_source_request(
        &handle,
        &mut state,
        0,
        GuiMediaSourceProviderId::local(),
    ));
    assert!(owner.pending_playlist_source_resolution.is_some());
    assert_eq!(
        *opens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        0,
        "an explicit source request must join the same delivery fence"
    );

    owner
        .session
        .as_mut()
        .expect("loopback session should remain installed")
        .send_chat_message("post-playlist-fence-traffic".to_owned())
        .expect("unrelated later chat should queue behind the playlist");

    let mut completed_actions = Vec::new();
    release_one.store(true, std::sync::atomic::Ordering::SeqCst);
    completed_actions.extend(pump_and_apply_runtime_owner_actions(
        &mut owner, &handle, &mut state,
    ));
    let pending_after_list_receipt = match owner.pending_shared_playlist_open.as_ref() {
        Some(GuiPendingSharedPlaylistOpen::AfterMutation { delivery_fence, .. }) => {
            delivery_fence.pending_frame_count()
        }
        _ => panic!("an unrelated List receipt must not satisfy the playlist frontier"),
    };
    assert_eq!(pending_after_list_receipt, pending_playlist_frames);
    assert_eq!(
        *opens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        0,
        "the unrelated List receipt must not release the local player open"
    );

    for _ in 0..16 {
        release_one.store(true, std::sync::atomic::Ordering::SeqCst);
        completed_actions.extend(pump_and_apply_runtime_owner_actions(
            &mut owner, &handle, &mut state,
        ));
        if owner.pending_shared_playlist_open.is_none() {
            break;
        }
    }
    assert_eq!(
        *opens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        1,
        "the retained open must resume exactly once after the playlist receipt"
    );
    assert!(owner.pending_shared_playlist_open.is_none());
    let writes = writes
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(writes.iter().any(|line| line.contains("episode1.mkv")));
    assert!(
        writes
            .iter()
            .all(|line| !line.contains("post-playlist-fence-traffic")),
        "traffic queued after the playlist fence must not delay the local open"
    );
    drop(writes);
    assert!(completed_actions.iter().any(|action| {
        matches!(
            action,
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Success,
                message,
            } if message.contains("shared playlist")
        )
    }));
    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert_eq!(
        *opens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        1,
        "later pumps must not replay the completed continuation"
    );
    let _ = std::fs::remove_dir_all(media_root);
}

#[test]
fn ordinary_playlist_selection_mutations_wait_for_their_exact_delivery_receipts() {
    let media_root = test_temp_root("ordinary-playlist-player-effect-fence");
    std::fs::create_dir_all(&media_root).expect("media fixture directory should be created");
    let first_path = media_root.join("episode1.mkv");
    let second_path = media_root.join("episode2.mkv");
    std::fs::write(&first_path, b"test").expect("first media fixture should be written");
    std::fs::write(&second_path, b"test").expect("second media fixture should be written");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    let opens = std::sync::Arc::new(std::sync::Mutex::new(0));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(OpenCountingPlayer {
        opens: opens.clone(),
    })));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path(first_path.to_string_lossy().into_owned()),
    );
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![media_root.to_string_lossy().into_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    owner
        .session
        .as_mut()
        .expect("loopback session should remain installed")
        .apply_message_json(r#"{"Set":{"playlistChange":{"files":["episode1.mkv"],"user":"bob"}}}"#)
        .expect("initial playlist should apply");
    owner
        .session
        .as_mut()
        .expect("loopback session should remain installed")
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"bob"}}}"#)
        .expect("initial playlist index should apply");
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    *opens
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = 0;

    let release_one = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    owner = owner.with_session_transport_driver(Box::new(DelayedPlaylistReceiptDriver {
        release_one: release_one.clone(),
        writes: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        pending_token: None,
        pending_line: None,
    }));

    handle.push_request(GuiRuntimeRequest::QueuePlaylistEntry {
        entry: "episode2.mkv".to_owned(),
        select_after_queue: true,
    });
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(matches!(
        owner.pending_shared_playlist_open,
        Some(GuiPendingSharedPlaylistOpen::AwaitingMutationDelivery { .. })
    ));
    assert_eq!(
        *opens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        0,
        "queue-and-select must not open optimistically projected media before its receipt"
    );
    for _ in 0..16 {
        release_one.store(true, std::sync::atomic::Ordering::SeqCst);
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        if owner.pending_shared_playlist_open.is_none() {
            break;
        }
    }
    assert!(owner.pending_shared_playlist_open.is_none());
    assert_eq!(
        *opens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        1,
        "queue-and-select should open once after its exact playlist receipt"
    );

    for _ in 0..8 {
        release_one.store(true, std::sync::atomic::Ordering::SeqCst);
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    }
    *opens
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = 0;
    handle.push_request(GuiRuntimeRequest::SetPlaylistIndex(0));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(matches!(
        owner.pending_shared_playlist_open,
        Some(GuiPendingSharedPlaylistOpen::AwaitingMutationDelivery { .. })
    ));
    assert_eq!(
        *opens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        0,
        "an explicit selected-index mutation must wait for its exact receipt"
    );
    for _ in 0..16 {
        release_one.store(true, std::sync::atomic::Ordering::SeqCst);
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        if owner.pending_shared_playlist_open.is_none() {
            break;
        }
    }
    assert!(owner.pending_shared_playlist_open.is_none());
    assert_eq!(
        *opens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        1,
        "selected-index activation should open once after its exact receipt"
    );

    let _ = std::fs::remove_dir_all(media_root);
}

#[test]
fn gui_persisted_config_runtime_owner_prioritizes_player_setup_before_stream_helper_modal_for_playlist_urls()
 {
    let root = test_temp_root("shared-playlist-youtube-no-player");
    let config_path = root.join("sorotte.ini");
    let helper_bin_dir = root.join("tools").join("stream-helper").join("bin");
    std::fs::create_dir_all(&helper_bin_dir)
        .expect("managed helper bin dir should be created for playlist-url regression");
    std::fs::write(
        helper_bin_dir.join(if cfg!(windows) {
            "yt-dlp.exe"
        } else {
            "yt-dlp"
        }),
        b"not an executable",
    )
    .expect("invalid yt-dlp fixture should be written");
    std::fs::write(
        helper_bin_dir.join(if cfg!(windows) { "deno.exe" } else { "deno" }),
        b"not an executable",
    )
    .expect("invalid deno fixture should be written");

    let mut owner =
        GuiPersistedConfigRuntimeOwner::with_config_path_and_startup_player(Some(config_path));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec!["https://www.youtube.com/watch?v=qDVPFAuBSXw".to_owned()],
        load_into_shared_playlist: true,
        playlist_insert_slot: None,
    });
    let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(
        owner
            .player_unavailability_reason
            .as_deref()
            .is_some_and(|message| message.contains("Set playerPath to mpv")),
        "playlist URL opens without a player should keep the player setup blocker visible"
    );
    assert_eq!(
        owner.stream_helper_runtime_snapshot.target, None,
        "stream-helper preflight should not run before player attachment is available"
    );
    assert!(
        !actions.iter().any(|action| matches!(
            action,
            GuiShellAction::OpenModal(GuiShellModal::StreamSupport)
        )),
        "playlist URL opens without a player should not open the stream-helper modal first"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_opens_playlist_url_without_a_local_origin() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        trusted_domains: Some(vec!["media.example.test".to_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    let stream_url = "https://media.example.test/video.mp4";

    owner.open_media_files_through_shared_playlist_runtime_impl(
        &handle,
        &mut state,
        vec![stream_url.to_owned()],
        None,
    );

    assert_eq!(
        state.current_shared_playlist_entries(),
        vec![stream_url.to_owned()]
    );
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(stream_url),
        "Automatic should continue through ordinary URL resolution when no local origin exists"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_automatic_session_open_loads_direct_web_url() {
    for stream_url in [
        "http://127.0.0.1:43210/generated-fault.wav",
        "https://media.example.test/video.mp4",
    ] {
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
            .with_client_core_chat_loopback_session_runtime("alice", "room1")
            .expect("client-core loopback runtime owner should bootstrap");
        owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
        let handle = GuiQueuedRuntimeBridgeHandle::default();
        let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            username: Some("alice".to_owned()),
            room: Some("room1".to_owned()),
            player_path: Some("mpv".to_owned()),
            shared_playlist_enabled: Some(true),
            only_switch_to_trusted_domains: Some(false),
            trusted_domains: Some(Vec::new()),
            ..StoredClientSettingsMvp::default()
        });
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

        owner.open_media_files_through_shared_playlist_runtime_impl(
            &handle,
            &mut state,
            vec![stream_url.to_owned()],
            None,
        );

        assert_eq!(
            state.current_shared_playlist_entries(),
            vec![stream_url.to_owned()]
        );
        assert_eq!(
            owner
                .player_local_file
                .as_ref()
                .and_then(|file| file.path.as_deref()),
            Some(stream_url),
            "Automatic session-backed media opens should load an initial direct HTTP(S) URL"
        );
    }
}

#[test]
fn gui_persisted_config_runtime_owner_deduplicates_accepted_direct_url_until_media_confirmation() {
    struct TrackedRecordingPlayer {
        opened_paths: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        command_progress: std::sync::Arc<
            std::sync::Mutex<std::collections::VecDeque<sorotte_player_api::PlayerCommandProgress>>,
        >,
        media_load_outcomes: std::sync::Arc<
            std::sync::Mutex<
                std::collections::VecDeque<sorotte_player_api::PlayerMediaLoadOutcome>,
            >,
        >,
        local_file_updates: std::sync::Arc<
            std::sync::Mutex<std::collections::VecDeque<sorotte_player_api::LocalFileUpdate>>,
        >,
        next_command_id: u64,
    }

    impl PlayerAdapter for TrackedRecordingPlayer {
        fn name(&self) -> &'static str {
            "tracked-recording"
        }

        fn execute_tracked(
            &mut self,
            command: sorotte_player_api::PlayerCommand,
        ) -> Result<sorotte_player_api::PlayerCommandId, sorotte_player_api::PlayerError> {
            let sorotte_player_api::PlayerCommand::OpenFile(path) = command else {
                return Err(sorotte_player_api::PlayerError::Unsupported(
                    "execute_tracked",
                ));
            };
            self.opened_paths
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(path);
            let command_id = sorotte_player_api::PlayerCommandId::new(self.next_command_id);
            self.next_command_id = self.next_command_id.wrapping_add(1);
            Ok(command_id)
        }

        fn take_command_progress(&mut self) -> Option<sorotte_player_api::PlayerCommandProgress> {
            self.command_progress
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .pop_front()
        }

        fn take_media_load_outcome(
            &mut self,
        ) -> Option<sorotte_player_api::PlayerMediaLoadOutcome> {
            self.media_load_outcomes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .pop_front()
        }

        fn take_local_file_update(&mut self) -> Option<sorotte_player_api::LocalFileUpdate> {
            self.local_file_updates
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .pop_front()
        }
    }

    let initial_url = "http://127.0.0.1:43210/generated-fault.wav";
    let replacement_url = "https://media.example.test/replacement.mp4";
    let opened_paths = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let command_progress =
        std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));
    let media_load_outcomes =
        std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));
    let local_file_updates =
        std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(TrackedRecordingPlayer {
        opened_paths: opened_paths.clone(),
        command_progress: command_progress.clone(),
        media_load_outcomes: media_load_outcomes.clone(),
        local_file_updates: local_file_updates.clone(),
        next_command_id: 1,
    })));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        only_switch_to_trusted_domains: Some(false),
        trusted_domains: Some(Vec::new()),
        ..StoredClientSettingsMvp::default()
    });
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    owner.open_media_files_through_shared_playlist_runtime_impl(
        &handle,
        &mut state,
        vec![initial_url.to_owned()],
        None,
    );
    assert_eq!(
        *opened_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        vec![initial_url.to_owned()]
    );
    let playlist_generation = owner.playlist_resolution.generation;
    let remote_revision = owner
        .session
        .as_ref()
        .expect("session should remain attached")
        .current_room_playlist_remote_revision();
    let initial_command_id = owner
        .playlist_resolution_attempt
        .as_ref()
        .and_then(|attempt| attempt.player_command_id)
        .expect("initial direct URL load should be tracked");
    assert!(owner.player_local_file_placeholder);

    command_progress
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .extend([
            sorotte_player_api::PlayerCommandProgress::accepted(
                initial_command_id,
                Some(sorotte_player_api::PlayerMediaGeneration::new(1)),
                None,
            ),
            sorotte_player_api::PlayerCommandProgress::finished(
                initial_command_id,
                Some(sorotte_player_api::PlayerMediaGeneration::new(1)),
                None,
                None,
                sorotte_player_api::PlayerCommandResult::Completed,
            ),
        ]);
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let attempt = owner
        .playlist_resolution_attempt
        .as_ref()
        .expect("accepted direct URL load should remain coordinated");
    assert_eq!(
        attempt.state,
        crate::app::runtime_owner::player::PlaylistResolutionAttemptState::Loading
    );
    assert_eq!(attempt.player_command_id, None);
    assert!(
        owner.last_attached_media_resolution_trigger.is_some(),
        "an IPC reply must not clear the dedupe trigger before media confirmation"
    );
    owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);
    for _ in 0..4 {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    }
    assert_eq!(
        owner.playlist_resolution.generation, playlist_generation,
        "matching playlistChange and playlistIndex echoes must preserve the optimistic scope"
    );
    assert_eq!(
        owner
            .session
            .as_ref()
            .expect("session should remain attached")
            .current_room_playlist_remote_revision(),
        remote_revision,
        "matching self echoes must not manufacture a remote replacement"
    );
    assert_eq!(
        *opened_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        vec![initial_url.to_owned()],
        "the accepted loadfile reply and matching playlist echoes must not resubmit the URL"
    );

    let mut projected_row_ids = vec![state.main_window.playlist[0].entry_id];
    for _ in 0..3 {
        state.main_window.playlist[0].entry_id = crate::app::GuiPlaylistEntryId::next();
        projected_row_ids.push(state.main_window.playlist[0].entry_id);
        owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);
    }
    assert_eq!(
        projected_row_ids
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        4,
        "the regression must exercise three distinct same-target row-scope projections"
    );
    assert_eq!(owner.playlist_resolution.generation, playlist_generation);
    assert_eq!(
        *opened_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        vec![initial_url.to_owned()],
        "same-target row identity churn must adopt the physical in-flight load instead of resubmitting it"
    );

    let successful_load = sorotte_player_api::PlayerMediaLoadOutcome::success(
        initial_url,
        Some(initial_url.to_owned()),
    );
    media_load_outcomes
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push_back(successful_load);
    local_file_updates
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push_back(
            sorotte_player_api::LocalFileUpdate::new("generated-fault.wav").with_path(initial_url),
        );
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        !owner.player_local_file_placeholder,
        "matching media evidence should clear the physical loading placeholder"
    );
    assert!(
        owner.current_player_matches_media_target(initial_url),
        "matching media evidence should confirm the direct URL identity"
    );
    owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);
    assert_eq!(
        owner
            .playlist_resolution_attempt
            .as_ref()
            .expect("confirmed direct URL load should remain coordinated")
            .state,
        crate::app::runtime_owner::player::PlaylistResolutionAttemptState::Active
    );

    let terminal_failure = sorotte_player_api::PlayerMediaLoadOutcome::failure(
        initial_url,
        Some(initial_url.to_owned()),
        sorotte_player_api::PlayerMediaLoadFailureKind::Network,
        "fixture connection ended",
    );
    media_load_outcomes
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push_back(terminal_failure);
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);
    assert_eq!(
        *opened_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        vec![initial_url.to_owned(), initial_url.to_owned()],
        "an actual terminal failure after activation must still retry the selected URL"
    );

    owner
        .session
        .as_mut()
        .expect("session should remain attached")
        .apply_message_json(
            &serde_json::json!({
                "Set": {
                    "playlistChange": {
                        "files": [replacement_url],
                        "user": "bob",
                    }
                }
            })
            .to_string(),
        )
        .expect("different remote playlist replacement should apply");
    let actions = owner
        .session
        .as_mut()
        .expect("session should remain attached")
        .drain_gui_actions(&state);
    for action in actions {
        assert!(
            state.apply(action),
            "remote replacement projection should apply cleanly"
        );
    }
    owner.reconcile_playlist_resolution_scope(&handle, &mut state);
    assert!(
        owner.playlist_resolution.generation > playlist_generation,
        "a genuinely different remote replacement must advance the playlist scope"
    );
    owner.sync_selected_shared_playlist_media_to_attached_player_impl(&state);
    assert_eq!(
        *opened_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        vec![
            initial_url.to_owned(),
            initial_url.to_owned(),
            replacement_url.to_owned(),
        ],
        "a genuinely different remote replacement must supersede the in-flight retry"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_automatic_session_open_keeps_unsupported_urls_unresolved() {
    for unsupported_url in [
        "ftp://media.example.test/video.mp4",
        "custom://media.example.test/video.mp4",
    ] {
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
            .with_client_core_chat_loopback_session_runtime("alice", "room1")
            .expect("client-core loopback runtime owner should bootstrap");
        owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
        let handle = GuiQueuedRuntimeBridgeHandle::default();
        let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            username: Some("alice".to_owned()),
            room: Some("room1".to_owned()),
            player_path: Some("mpv".to_owned()),
            shared_playlist_enabled: Some(true),
            only_switch_to_trusted_domains: Some(true),
            trusted_domains: Some(vec!["media.example.test".to_owned()]),
            ..StoredClientSettingsMvp::default()
        });
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

        owner.open_media_files_through_shared_playlist_runtime_impl(
            &handle,
            &mut state,
            vec![unsupported_url.to_owned()],
            None,
        );

        assert_eq!(
            state.current_shared_playlist_entries(),
            vec![unsupported_url.to_owned()]
        );
        assert!(
            owner.player_local_file.is_none(),
            "Automatic session-backed media opens must not load unsupported URL schemes"
        );
    }
}

#[test]
fn gui_persisted_config_runtime_owner_automatic_session_open_blocks_untrusted_direct_web_url() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        only_switch_to_trusted_domains: Some(true),
        trusted_domains: Some(vec!["trusted.example.test".to_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let untrusted_url = "https://untrusted.example.test/video.mp4";

    owner.open_media_files_through_shared_playlist_runtime_impl(
        &handle,
        &mut state,
        vec![untrusted_url.to_owned()],
        None,
    );

    assert_eq!(
        state.current_shared_playlist_entries(),
        vec![untrusted_url.to_owned()]
    );
    assert!(
        owner.player_local_file.is_none(),
        "the direct-media candidate must remain behind room URL trust preflight"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_inserts_shared_playlist_media_at_requested_slot() {
    let media_root = test_temp_root("shared-playlist-requested-slot");
    let inserted_path = media_root.join("episode2.mkv");
    std::fs::write(&inserted_path, b"test").expect("inserted media fixture should be written");
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    handle.push_request(GuiRuntimeRequest::ReplacePlaylist {
        files: vec!["episode1.mkv".to_owned(), "episode3.mkv".to_owned()],
        selected_index: Some(0),
    });
    let _ = pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| {
            state
                .main_window
                .playlist
                .iter()
                .map(|row| row.label.as_str())
                .eq(["episode1.mkv", "episode3.mkv"])
                && state.selection.selected_main_window_playlist == Some(0)
        },
        "shared-playlist seed before slot insert",
    );

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![inserted_path.to_string_lossy().into_owned()],
        load_into_shared_playlist: true,
        playlist_insert_slot: Some(1),
    });
    let actions = pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| {
            state
                .main_window
                .playlist
                .iter()
                .map(|row| row.label.as_str())
                .eq(["episode1.mkv", "episode2.mkv", "episode3.mkv"])
                && state.selection.selected_main_window_playlist == Some(0)
        },
        "shared-playlist insert at requested slot",
    );

    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Success
                    && message == "Loaded 1 selected media entry into the shared playlist."
        )),
        "shared-playlist insert should report a runtime-backed success"
    );
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.clone())
            .collect::<Vec<_>>(),
        vec![
            "episode1.mkv".to_owned(),
            "episode2.mkv".to_owned(),
            "episode3.mkv".to_owned(),
        ]
    );
    assert_eq!(state.selection.selected_main_window_playlist, Some(0));
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some("C:/Media/episode1.mkv")
    );
    let _ = std::fs::remove_dir_all(media_root);
}

#[test]
fn gui_persisted_config_runtime_owner_applies_playlist_default_source_to_local_media_insert() {
    let media_root = test_temp_root("shared-playlist-default-source-insert");
    let media_path = media_root.join("episode1.mkv");
    std::fs::write(&media_path, b"test").expect("default-source media fixture should be written");
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        plex_plugin_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("machine-1".to_owned()),
        plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    });
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylistDefaultSource {
        source_id: GuiPlaylistDefaultSourceId::provider(GuiMediaSourceProviderId::plex_stream()),
    }));

    owner.open_media_files_through_shared_playlist_runtime_impl(
        &handle,
        &mut state,
        vec![media_path.to_string_lossy().into_owned()],
        Some(0),
    );

    assert_eq!(
        state
            .main_window
            .playlist
            .first()
            .map(|row| row.label.as_str()),
        Some("episode1.mkv")
    );
    assert_eq!(
        state.main_window.playlist[0]
            .source_state
            .current_provider_id,
        GuiMediaSourceProviderId::plex_stream(),
        "local drag/drop additions should use the selected playlist default source"
    );
    assert!(
        owner.plex_stream_resolve_rx.is_some(),
        "a newly selected Plex-default row should queue Plex stream resolution instead of opening local"
    );
    assert!(
        owner.player_local_file.is_none(),
        "the selected local path must not be opened directly when the row source is Plex Stream"
    );
    let _ = std::fs::remove_dir_all(media_root);
}

#[test]
fn gui_persisted_config_runtime_owner_explicit_plex_default_wins_for_cached_plex_local_media_insert()
 {
    let root = test_temp_root("shared-playlist-explicit-plex-cached-local");
    let config_path = root.join("sorotte.ini");
    let media_dir = root.join("Dropped");
    std::fs::create_dir_all(&media_dir)
        .expect("explicit Plex cached-local fixture directory should be created");
    let media_path = media_dir.join("episode1.mkv");
    std::fs::write(&media_path, b"test")
        .expect("explicit Plex cached-local fixture should be written");
    let media_path_text = media_path.to_string_lossy().into_owned();
    seed_cached_plex_match_for_local_path(&root, &media_path, "123", "Episode 1");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path))
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        plex_plugin_enabled: Some(true),
        plex_sync_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("machine-1".to_owned()),
        plex_selected_server_url: Some("http://127.0.0.1:32400".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    });
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylistDefaultSource {
        source_id: GuiPlaylistDefaultSourceId::provider(GuiMediaSourceProviderId::plex_stream()),
    }));

    owner.open_media_files_through_shared_playlist_runtime_impl(
        &handle,
        &mut state,
        vec![media_path_text.clone()],
        None,
    );

    let entries = state.current_shared_playlist_entries();
    parse_plex_playlist_uri(
        entries
            .first()
            .expect("cached local drop should project a shared-playlist row"),
    )
    .expect("cached local drop should retain its peer-facing Plex URI");
    assert_eq!(
        state.main_window.playlist[0]
            .source_state
            .current_provider_id,
        GuiMediaSourceProviderId::plex_stream(),
        "an explicit Plex default must override local precedence"
    );
    assert_eq!(
        state.main_window.playlist[0].source_state.policy,
        GuiPlaylistSourcePolicy::ForcePlex
    );
    assert_eq!(
        state.main_window.playlist[0].source_state.selection_origin,
        GuiPlaylistSourceSelectionOrigin::PlaylistDefault,
        "the Plex default should be recorded distinctly from a per-row override"
    );
    assert!(
        owner.player_local_file.is_none(),
        "the exact local path must not open directly when Plex was explicitly selected"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_uses_local_for_media_match_default_local_media_insert() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        media_matching_plugin_enabled: Some(true),
        media_match_fingerprinting_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        state.apply(GuiShellAction::SelectMainWindowPlaylistDefaultSource {
            source_id: GuiPlaylistDefaultSourceId::provider(
                GuiMediaSourceProviderId::media_matching()
            ),
        })
    );
    let media_root = test_temp_root("media-match-default-local-drop");
    let media_path = media_root.join("episode1.mkv");
    std::fs::write(&media_path, b"test").expect("media fixture should be written");
    let media_path_text = media_path.to_string_lossy().into_owned();

    owner.open_media_files_through_shared_playlist_runtime_impl(
        &handle,
        &mut state,
        vec![media_path_text.clone()],
        Some(0),
    );

    assert_eq!(
        state
            .main_window
            .playlist
            .first()
            .map(|row| row.label.as_str()),
        Some("episode1.mkv")
    );
    assert_eq!(
        state.main_window.playlist[0]
            .source_state
            .current_provider_id,
        GuiMediaSourceProviderId::local(),
        "a local drag/drop should stay local even when the playlist default is Media Matching"
    );
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(media_path_text.as_str())
    );
    assert!(
        owner.media_match_remote_lookup_rx.is_none(),
        "Media Matching should not run when the local file path is already available"
    );
    let _ = std::fs::remove_dir_all(media_root);
}

#[test]
fn gui_persisted_config_runtime_owner_keeps_manual_media_match_after_same_row_local_drop() {
    let media_root = test_temp_root("manual-media-match-then-local-drop");
    let media_path = media_root.join("episode1.mkv");
    std::fs::write(&media_path, b"test")
        .expect("manual Media Match drop fixture should be written");
    let media_path_text = media_path.to_string_lossy().into_owned();

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.active_shared_playlist_index = Some(0);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        media_matching_plugin_enabled: Some(true),
        media_match_fingerprinting_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.apply_shared_playlist_entries(vec!["episode1.mkv".to_owned()], Some(0), false);
    state.main_window.active_playlist_index = Some(0);
    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylistSource {
        index: 0,
        provider_id: GuiMediaSourceProviderId::media_matching(),
    }));
    let entry_id = state.main_window.playlist[0].entry_id;

    owner.open_media_files_through_shared_playlist_runtime_impl(
        &handle,
        &mut state,
        vec![media_path_text],
        Some(0),
    );

    let row = &state.main_window.playlist[0];
    assert_eq!(
        row.entry_id, entry_id,
        "the deduplicated drop should target the existing row"
    );
    assert_eq!(
        row.source_state.policy,
        GuiPlaylistSourcePolicy::ForceMediaMatching
    );
    assert_eq!(
        row.source_state.selection_origin,
        GuiPlaylistSourceSelectionOrigin::UserOverride
    );
    assert_ne!(
        row.source_state.current_provider_id,
        GuiMediaSourceProviderId::local(),
        "a local drop must not erase a per-row Media Matching override"
    );
    assert!(
        owner.player_local_file.is_none(),
        "the retained exact local origin must remain ineligible under ForceMediaMatching"
    );
    assert!(
        owner.pending_attached_media_resolution.is_none(),
        "strict Media Matching must not start ordinary local indexing after the drop"
    );

    let _ = std::fs::remove_dir_all(media_root);
}

#[test]
fn gui_persisted_config_runtime_owner_appends_shared_playlist_media_without_switching_selection() {
    let media_root = test_temp_root("shared-playlist-append-selection");
    let appended_path = media_root.join("episode3.mkv");
    std::fs::write(&appended_path, b"test").expect("appended media fixture should be written");
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_loopback_session_runtime("alice", "room1")
        .expect("client-core loopback runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    handle.push_request(GuiRuntimeRequest::ReplacePlaylist {
        files: vec!["episode1.mkv".to_owned(), "episode2.mkv".to_owned()],
        selected_index: Some(0),
    });
    let _ = pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| {
            state
                .main_window
                .playlist
                .iter()
                .map(|row| row.label.as_str())
                .eq(["episode1.mkv", "episode2.mkv"])
                && state.selection.selected_main_window_playlist == Some(0)
        },
        "shared-playlist seed before append",
    );

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![appended_path.to_string_lossy().into_owned()],
        load_into_shared_playlist: true,
        playlist_insert_slot: Some(2),
    });
    let actions = pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| {
            state
                .main_window
                .playlist
                .iter()
                .map(|row| row.label.as_str())
                .eq(["episode1.mkv", "episode2.mkv", "episode3.mkv"])
                && state.selection.selected_main_window_playlist == Some(0)
        },
        "shared-playlist append preserves selection",
    );

    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Success
                    && message == "Loaded 1 selected media entry into the shared playlist."
        )),
        "shared-playlist append should report a runtime-backed success"
    );
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.clone())
            .collect::<Vec<_>>(),
        vec![
            "episode1.mkv".to_owned(),
            "episode2.mkv".to_owned(),
            "episode3.mkv".to_owned(),
        ]
    );
    assert_eq!(state.selection.selected_main_window_playlist, Some(0));
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some("C:/Media/episode1.mkv")
    );
    let _ = std::fs::remove_dir_all(media_root);
}

#[test]
fn gui_persisted_config_runtime_owner_preserves_session_playlist_index_when_local_selection_is_stale_on_append()
 {
    let media_root = test_temp_root("shared-playlist-stale-selection-append");
    let appended_path = media_root.join("episode4.mkv");
    std::fs::write(&appended_path, b"test").expect("appended media fixture should be written");
    let (mut owner, handle, mut state) = seeded_loopback_shared_playlist_owner(2);

    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(1)));
    assert!(state.main_window_playlist_selection_is_local);

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![appended_path.to_string_lossy().into_owned()],
        load_into_shared_playlist: true,
        playlist_insert_slot: Some(3),
    });
    let _ = pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| {
            state
                .main_window
                .playlist
                .iter()
                .map(|row| row.label.as_str())
                .eq([
                    "episode1.mkv",
                    "episode2.mkv",
                    "episode3.mkv",
                    "episode4.mkv",
                ])
        },
        "shared-playlist append with stale local selection",
    );

    assert_eq!(state.main_window.active_playlist_index, Some(2));
    assert_eq!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.current_room_playlist_index()),
        Some(2)
    );
    assert_eq!(owner.active_shared_playlist_index, Some(2));
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some("C:/Media/episode3.mkv")
    );
    let _ = std::fs::remove_dir_all(media_root);
}

#[test]
fn gui_persisted_config_runtime_owner_remaps_active_playlist_index_when_inserting_before_active() {
    let media_root = test_temp_root("shared-playlist-insert-before-active");
    let inserted_path = media_root.join("episode1-5.mkv");
    std::fs::write(&inserted_path, b"test").expect("inserted media fixture should be written");
    let (mut owner, handle, mut state) = seeded_loopback_shared_playlist_owner(2);

    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(1)));
    assert!(state.main_window_playlist_selection_is_local);

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![inserted_path.to_string_lossy().into_owned()],
        load_into_shared_playlist: true,
        playlist_insert_slot: Some(1),
    });
    let _ = pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| {
            state
                .main_window
                .playlist
                .iter()
                .map(|row| row.label.as_str())
                .eq([
                    "episode1.mkv",
                    "episode1-5.mkv",
                    "episode2.mkv",
                    "episode3.mkv",
                ])
        },
        "shared-playlist insert before active entry",
    );

    assert_eq!(state.main_window.active_playlist_index, Some(3));
    assert_eq!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.current_room_playlist_index()),
        Some(3)
    );
    assert_eq!(owner.active_shared_playlist_index, Some(3));
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some("C:/Media/episode3.mkv")
    );
    let _ = std::fs::remove_dir_all(media_root);
}

#[test]
fn gui_persisted_config_runtime_owner_coerces_local_media_open_into_playlist_control_when_shared_playlist_is_enabled()
 {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let media_root = test_temp_root("coerced-shared-playlist-local-open");
    let media_path = media_root.join("local-only.mkv");
    std::fs::write(&media_path, b"test").expect("media fixture should be written");
    let media_path_text = media_path.to_string_lossy().into_owned();

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![media_path_text.clone()],
        load_into_shared_playlist: false,
        playlist_insert_slot: None,
    });
    let actions = pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| {
            state.main_window.playlist.len() == 1
                && state.selection.selected_main_window_playlist == Some(0)
                && state.main_window.playlist[0].label == "local-only.mkv"
        },
        "shared-playlist-enabled local media opens route through playlist control",
    );

    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Success
                    && message == "Loaded 1 selected media entry into the shared playlist."
        )),
        "shared-playlist-enabled media opens should still report playlist success",
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Warning
                    && message == "Shared playlist updates require a session runtime connection; the selected media was not added to the room playlist."
        )),
        "detached shared-playlist media opens should report that room sync is unavailable",
    );
    assert_eq!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.clone())
            .collect::<Vec<_>>(),
        vec!["local-only.mkv".to_owned()]
    );
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(media_path_text.as_str())
    );
    let _ = std::fs::remove_dir_all(media_root);
}

#[test]
fn gui_persisted_config_runtime_owner_coerces_local_media_open_into_playlist_control_even_when_legacy_toggle_is_disabled()
 {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("mpv".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let media_root = test_temp_root("coerced-legacy-toggle-local-open");
    let media_path = media_root.join("local-drop.mkv");
    std::fs::write(&media_path, b"test").expect("media fixture should be written");
    let media_path_text = media_path.to_string_lossy().into_owned();

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![media_path_text.clone()],
        load_into_shared_playlist: false,
        playlist_insert_slot: None,
    });
    let actions = pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(1),
        |state| {
            state.main_window.shared_playlist_enabled
                && state.main_window.playlist.len() == 1
                && state.selection.selected_main_window_playlist == Some(0)
                && state.main_window.playlist[0].label == "local-drop.mkv"
        },
        "playlist-backed local media opens remain active with the legacy toggle disabled",
    );

    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Success
                    && message == "Loaded 1 selected media entry into the shared playlist."
        )),
        "media opens should still report playlist success when the legacy toggle is off",
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Warning
                    && message == "Shared playlist updates require a session runtime connection; the selected media was not added to the room playlist."
        )),
        "detached playlist-backed media opens should still warn about missing room sync",
    );
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(media_path_text.as_str())
    );
    let _ = std::fs::remove_dir_all(media_root);
}

#[test]
fn gui_persisted_config_runtime_owner_blocks_local_media_open_when_room_playlist_control_is_unavailable()
 {
    let media_root = test_temp_root("shared-playlist-control-unavailable");
    let media_path = media_root.join("blocked-drop.mkv");
    std::fs::write(&media_path, b"test").expect("blocked-drop fixture should be written");
    #[derive(Debug, Default)]
    struct NoControlSessionState {
        replace_playlist_calls: usize,
    }

    struct NoControlSessionRuntimeAdapter {
        state: std::sync::Arc<std::sync::Mutex<NoControlSessionState>>,
    }

    impl GuiSessionRuntimeAdapter for NoControlSessionRuntimeAdapter {
        fn playlist_control_available(&self) -> bool {
            false
        }

        fn replace_playlist(
            &mut self,
            _files: Vec<String>,
            _selected_index: Option<usize>,
        ) -> Result<(), String> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .replace_playlist_calls += 1;
            Ok(())
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

    let session_state =
        std::sync::Arc::new(std::sync::Mutex::new(NoControlSessionState::default()));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None).with_session_runtime(
        Box::new(NoControlSessionRuntimeAdapter {
            state: session_state.clone(),
        }),
    );
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("+room1".to_owned()),
        player_path: Some("mpv".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let initial_playlist = state
        .main_window
        .playlist
        .iter()
        .map(|row| row.label.clone())
        .collect::<Vec<_>>();

    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![media_path.to_string_lossy().into_owned()],
        load_into_shared_playlist: false,
        playlist_insert_slot: None,
    });
    let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(
        actions.iter().any(|action| matches!(
            action,
            GuiShellAction::PushTransientNotification { level, message }
                if *level == GuiTransientNotificationLevel::Error
                    && message == "Shared playlist control is unavailable for the active room; the selected media was not added to the room playlist or opened in the attached player."
        )),
        "non-controller media drops should fail instead of opening directly in the attached player",
    );
    assert!(
        state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.clone())
            .collect::<Vec<_>>()
            == initial_playlist,
        "blocked non-controller media drops must not change the shared playlist locally",
    );
    assert!(
        owner.player_local_file.is_none(),
        "blocked non-controller media drops must not open a local file in the attached player",
    );
    assert!(
        session_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .replace_playlist_calls
            == 0,
        "blocked non-controller media drops must not attempt a session playlist mutation",
    );
    let _ = std::fs::remove_dir_all(media_root);
}
