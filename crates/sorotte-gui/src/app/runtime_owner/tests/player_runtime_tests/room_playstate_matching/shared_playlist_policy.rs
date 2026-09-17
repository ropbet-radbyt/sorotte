use super::*;
use crate::app::runtime_stack::test_support::barrier::transport;
use sorotte_client_core::{LogicalMediaId, MediaLoadIntent, MediaTransportKind};
use sorotte_player_api::PlayerTransportPhase;

struct AttachedPolicyFixture {
    owner: GuiPersistedConfigRuntimeOwner,
    recorded: std::sync::Arc<std::sync::Mutex<CoordinatorAuthorityPlayerState>>,
    now: f64,
}

impl AttachedPolicyFixture {
    fn new(shared_playlist_enabled: Option<bool>) -> Self {
        let mut session = GuiClientSession::new("alice", "room1");
        if let Some(enabled) = shared_playlist_enabled {
            session
                .apply_runtime_settings_snapshot(&stored_client_settings_runtime_snapshot(
                    &StoredClientSettings {
                        username: Some("alice".to_owned()),
                        room: Some("room1".to_owned()),
                        shared_playlist_enabled: Some(enabled),
                        ..Default::default()
                    },
                ))
                .unwrap();
        }
        session.apply_message_json(r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sharedPlaylists":true}}}"#).unwrap();
        let now = crate::app::support::system_time_seconds();
        session.apply_message_json_at(r#"{"Set":{"playlistChange":{"files":["local-predecessor.mkv","remote-successor.mkv"],"user":"bob"},"playlistIndex":{"index":0,"user":"bob"}}}"#, now).unwrap();
        assert!(!session.has_pending_playlist_index_reset_intent());
        session.apply_message_json_at(r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#, now).unwrap();
        session
            .prepare_attached_playback_media(
                LogicalMediaId::new("local-predecessor.mkv").unwrap(),
                MediaTransportKind::LocalFile,
                MediaLoadIntent::NewPlayback,
                now,
            )
            .unwrap();
        let recorded = std::sync::Arc::new(std::sync::Mutex::new(
            CoordinatorAuthorityPlayerState::default(),
        ));
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.session = Some(Box::new(session));
        owner.player = Some(GuiOwnedPlayer::Custom(Box::new(
            CoordinatorAuthorityPlayer {
                state: recorded.clone(),
            },
        )));
        owner.player_local_file = Some(sorotte_player_api::LocalFileUpdate::new(
            "local-predecessor.mkv",
        ));
        let mut fixture = Self {
            owner,
            recorded,
            now,
        };
        fixture.reconcile(0.0);
        fixture.observe(0.0, 10.0, true);
        fixture.reconcile(0.1);
        fixture.observe(0.2, 10.0, true);
        fixture.clear_recorded();
        fixture
    }

    fn session(&mut self) -> &mut GuiClientSession {
        self.owner.session.as_mut().unwrap()
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.session()
            .apply_runtime_settings_snapshot(&stored_client_settings_runtime_snapshot(
                &StoredClientSettings {
                    username: Some("alice".to_owned()),
                    room: Some("room1".to_owned()),
                    shared_playlist_enabled: Some(enabled),
                    ..Default::default()
                },
            ))
            .unwrap();
    }

    fn receive_selection(&mut self, index: usize) {
        let now = self.now + 1.0;
        self.session()
            .apply_message_json_at(
                &serde_json::json!({"Set":{"playlistIndex":{"index":index,"user":"bob"}}})
                    .to_string(),
                now,
            )
            .unwrap();
        assert!(self.session().has_pending_playlist_index_reset_intent());
    }

    fn room_state(&mut self, paused: bool, seconds: f64) {
        let now = self.now + seconds;
        self.session()
            .apply_message_json_at(
                &serde_json::json!({"State":{"playstate":{
                    "position":10.0,"paused":paused,"doSeek":false,"setBy":"bob"
                }}})
                .to_string(),
                now,
            )
            .unwrap();
    }

    fn observe(&mut self, seconds: f64, position: f64, paused: bool) {
        self.observe_at(seconds, seconds, position, paused);
    }

    fn observe_at(
        &mut self,
        observed_seconds: f64,
        received_seconds: f64,
        position: f64,
        paused: bool,
    ) {
        let now = self.now + received_seconds;
        let phase = if paused {
            PlayerTransportPhase::ReadyPaused
        } else {
            PlayerTransportPhase::Playing
        };
        let actions = self
            .session()
            .sync_attached_player_transport_telemetry(
                transport(observed_seconds, phase, position, paused, 1),
                now,
            )
            .unwrap();
        self.owner
            .apply_attached_player_runtime_actions_impl(actions, "shared-playlist observation");
    }

    fn reconcile(&mut self, seconds: f64) {
        let now = self.now + seconds;
        let actions = self.session().attached_player_runtime_actions(now).unwrap();
        self.owner
            .apply_attached_player_runtime_actions_impl(actions, "shared-playlist reconciliation");
    }

    fn clear_recorded(&mut self) {
        *self.recorded.lock().unwrap() = CoordinatorAuthorityPlayerState::default();
    }
}

#[test]
fn default_shared_playlist_owner_fences_queued_predecessor_transport_actions() {
    for index in [0, 1] {
        let mut fixture = AttachedPolicyFixture::new(None);
        fixture.receive_selection(index);
        // The old physical file's already queued Playing observation crosses
        // the GUI external-observation seam after a new selection or replay.
        // It must not send the old planner's pause/seek work to the player.
        fixture.observe_at(0.5, 1.1, 10.0, false);
        let recorded = fixture.recorded.lock().unwrap();
        assert!(recorded.paused.is_empty(), "index={index}: {recorded:?}");
        assert!(recorded.positions.is_empty(), "index={index}: {recorded:?}");
        assert!(
            recorded.playback_rates.is_empty(),
            "index={index}: {recorded:?}"
        );
    }
}

#[test]
fn saved_shared_playlist_opt_out_preserves_attached_room_pause_and_play() {
    let mut fixture = AttachedPolicyFixture::new(Some(false));
    fixture.receive_selection(1);
    fixture.observe_at(0.5, 1.1, 10.0, false);
    assert_eq!(fixture.recorded.lock().unwrap().paused, [true]);
    fixture.observe(1.1, 10.0, true);
    fixture.room_state(false, 1.2);
    fixture.reconcile(1.2);
    fixture.observe(1.3, 10.0, true);
    assert_eq!(fixture.recorded.lock().unwrap().paused, [true, false]);
    assert!(fixture.session().has_pending_playlist_index_reset_intent());
}

#[test]
fn live_shared_playlist_opt_out_update_reaches_attached_coordinator() {
    let mut fixture = AttachedPolicyFixture::new(None);
    fixture.receive_selection(1);
    fixture.set_enabled(false);
    fixture.observe_at(0.5, 1.1, 10.0, false);
    assert_eq!(fixture.recorded.lock().unwrap().paused, [true]);
    fixture.observe(1.1, 10.0, true);
    fixture.clear_recorded();
    fixture.set_enabled(true);
    fixture.observe(1.2, 10.0, false);
    assert!(fixture.recorded.lock().unwrap().paused.is_empty());
    assert!(fixture.session().has_pending_playlist_index_reset_intent());
}
