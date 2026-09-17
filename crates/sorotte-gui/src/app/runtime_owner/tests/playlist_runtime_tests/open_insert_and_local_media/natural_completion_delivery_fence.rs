use super::*;
use serde_json::{Value, json};
use sorotte_player_api::{PlayerError, PlayerEventAcknowledgementToken, PlayerEventBatch};
use sorotte_player_mpv::{LifecycleVerificationPlaylistEntry, MpvLifecycleVerificationHarness};
use std::sync::{Arc, Mutex, atomic::Ordering};

#[derive(Default)]
struct CompletionFencePlayerState {
    lifecycle: MpvLifecycleVerificationHarness,
    opened_paths: Vec<String>,
}

struct CompletionFencePlayer(Arc<Mutex<CompletionFencePlayerState>>);

impl PlayerAdapter for CompletionFencePlayer {
    fn name(&self) -> &'static str {
        "completion-fence"
    }

    fn open_file(&mut self, path: &str) -> Result<(), PlayerError> {
        self.0.lock().unwrap().opened_paths.push(path.to_owned());
        Ok(())
    }

    fn take_player_event_batch(&mut self) -> Option<PlayerEventBatch> {
        self.0.lock().unwrap().lifecycle.take_event_batch()
    }

    fn acknowledge_player_event_batch(
        &mut self,
        token: PlayerEventAcknowledgementToken,
    ) -> Result<(), PlayerError> {
        self.0.lock().unwrap().lifecycle.acknowledge(token)
    }
}

#[test]
fn natural_completion_holds_successor_open_until_its_exact_protocol_write_receipt() {
    for keep_open in [false, true] {
        let media_root = test_temp_root(if keep_open {
            "retained-natural-completion-delivery-fence"
        } else {
            "end-file-natural-completion-delivery-fence"
        });
        let first_path = media_root.join("episode1.mkv");
        let second_path = media_root.join("episode2.mkv");
        std::fs::write(&first_path, b"first").unwrap();
        std::fs::write(&second_path, b"second").unwrap();
        let first_target = first_path.to_string_lossy().into_owned();
        let player = Arc::new(Mutex::new(CompletionFencePlayerState::default()));
        {
            let mut player = player.lock().unwrap();
            player.lifecycle.accept_tracked_load(&first_target, []);
            player.lifecycle.apply_authoritative_snapshot(
                [LifecycleVerificationPlaylistEntry::new(
                    77,
                    Some(first_target.clone()),
                    true,
                )],
                Some(first_target.clone()),
            );
            for event in [
                json!({"event":"start-file","playlist_entry_id":77}),
                json!({"event":"property-change","name":"path","data":first_target}),
                json!({"event":"property-change","name":"duration","data":240.0}),
                json!({"event":"file-loaded"}),
                json!({"event":"playback-restart"}),
                json!({"event":"property-change","name":"pause","data":false}),
                json!({"event":"property-change","name":"paused-for-cache","data":false}),
                json!({"event":"property-change","name":"eof-reached","data":false}),
                json!({"event":"property-change","name":"core-idle","data":false}),
                json!({"event":"property-change","name":"time-pos","data":30.0}),
            ] {
                player.lifecycle.ingest_decoded_mpv_json(event);
            }
        }

        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
            .with_client_core_chat_loopback_session_runtime("alice", "room1")
            .unwrap();
        owner.player = Some(GuiOwnedPlayer::Custom(Box::new(CompletionFencePlayer(
            player.clone(),
        ))));
        let handle = GuiQueuedRuntimeBridgeHandle::default();
        let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
            username: Some("alice".to_owned()),
            room: Some("room1".to_owned()),
            shared_playlist_enabled: Some(true),
            media_search_directories: Some(vec![media_root.to_string_lossy().into_owned()]),
            ..StoredClientSettings::default()
        });
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        let session = owner.session.as_mut().unwrap();
        session.apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"bob","sorottePlaylistEpoch":1}}}"#,
        ).unwrap();
        session
            .apply_message_json(
                r#"{"Set":{"playlistIndex":{"index":0,"user":"bob","sorottePlaylistEpoch":2}}}"#,
            )
            .unwrap();
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        assert_eq!(state.main_window.active_playlist_index, Some(0));
        assert!(owner.pending_shared_playlist_open.is_none());
        assert!(player.lock().unwrap().opened_paths.is_empty());

        let release_one = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let writes = Arc::new(Mutex::new(Vec::new()));
        owner = owner.with_session_transport_driver(Box::new(DelayedPlaylistReceiptDriver {
            release_one: release_one.clone(),
            writes: writes.clone(),
            pending_token: None,
            pending_line: None,
        }));
        owner
            .session
            .as_mut()
            .unwrap()
            .queue_user_list_for_test()
            .unwrap();
        {
            let mut player = player.lock().unwrap();
            player.lifecycle.ingest_decoded_mpv_json(
                json!({"event":"property-change","name":"time-pos","data":240.0}),
            );
            if keep_open {
                for event in [
                    json!({"event":"property-change","name":"eof-reached","data":true}),
                    json!({"event":"property-change","name":"core-idle","data":true}),
                    json!({"event":"property-change","name":"pause","data":true}),
                ] {
                    player.lifecycle.ingest_decoded_mpv_json(event);
                }
            } else {
                player.lifecycle.ingest_decoded_mpv_json(
                    json!({"event":"end-file","playlist_entry_id":77,"reason":"eof"}),
                );
            }
        }
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        assert_eq!(
            state.main_window.active_playlist_index,
            Some(1),
            "owned natural completion must select its successor"
        );
        let Some(GuiPendingSharedPlaylistOpen::AwaitingMutationDelivery { delivery_fence }) =
            owner.pending_shared_playlist_open.as_ref()
        else {
            panic!("natural completion must retain its pending protocol write fence");
        };
        assert_eq!(delivery_fence.pending_frame_count(), 1);
        assert!(
            player.lock().unwrap().opened_paths.is_empty(),
            "projecting the successor must not open it before the playlist write receipt"
        );
        owner
            .session
            .as_mut()
            .unwrap()
            .send_chat_message("after-natural-completion-fence".to_owned())
            .unwrap();

        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        assert!(player.lock().unwrap().opened_paths.is_empty());
        let mut list_written = false;
        for _ in 0..16 {
            release_one.store(true, Ordering::SeqCst);
            pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
            assert!(
                owner.pending_shared_playlist_open.is_some(),
                "unrelated writes through the preceding List must not release completion"
            );
            assert!(player.lock().unwrap().opened_paths.is_empty());
            list_written = writes.lock().unwrap().iter().any(|line| {
                serde_json::from_str::<Value>(line)
                    .unwrap()
                    .get("List")
                    .is_some()
            });
            if list_written {
                break;
            }
        }
        assert!(
            list_written,
            "the reliable List queued before completion must be acknowledged first"
        );

        for _ in 0..16 {
            release_one.store(true, Ordering::SeqCst);
            pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
            if owner.pending_shared_playlist_open.is_none() {
                break;
            }
        }
        assert!(owner.pending_shared_playlist_open.is_none());
        let opened_paths = player.lock().unwrap().opened_paths.clone();
        assert_eq!(
            opened_paths.len(),
            1,
            "the successor must open exactly once after its playlist receipt"
        );
        assert_eq!(
            std::fs::canonicalize(&opened_paths[0]).unwrap(),
            second_path.canonicalize().unwrap()
        );
        let written = writes.lock().unwrap().clone();
        let indices = written
            .iter()
            .filter_map(|line| {
                serde_json::from_str::<Value>(line)
                    .unwrap()
                    .get("Set")?
                    .get("playlistIndex")
                    .cloned()
            })
            .collect::<Vec<_>>();
        assert_eq!(indices.len(), 1, "one EOF must publish one advancement");
        assert_eq!(indices[0]["index"], 1);
        assert_eq!(indices[0]["sorotteExpectedPlaylistIndex"], 0);
        assert_eq!(indices[0]["sorotteExpectedPlaylistEpoch"], 2);
        assert!(
            written
                .iter()
                .all(|line| !line.contains("after-natural-completion-fence")),
            "later traffic must not extend the player-effect fence"
        );
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        assert_eq!(
            player.lock().unwrap().opened_paths.len(),
            1,
            "subsequent polls must not replay completion"
        );
    }
}
