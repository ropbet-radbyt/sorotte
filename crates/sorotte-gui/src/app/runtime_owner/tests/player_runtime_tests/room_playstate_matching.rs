use super::*;
use crate::app::GuiClientSession;
use crate::app::runtime_owner::GuiAttachedSystemSeekSource;
use crate::app::testing::support::runtime_state_for_shell;
use sorotte_client_app::app_boundary::application::ClientCommand;

mod shared_playlist_policy;

#[derive(Debug, Default)]
struct CoordinatorAuthorityPlayerState {
    paused: Vec<bool>,
    positions: Vec<f64>,
    playback_rates: Vec<f64>,
}

struct CoordinatorAuthorityPlayer {
    state: std::sync::Arc<std::sync::Mutex<CoordinatorAuthorityPlayerState>>,
}

impl PlayerAdapter for CoordinatorAuthorityPlayer {
    fn name(&self) -> &'static str {
        "coordinator-authority"
    }

    fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .paused
            .push(paused);
        Ok(())
    }

    fn set_position(
        &mut self,
        position_seconds: f64,
    ) -> Result<(), sorotte_player_api::PlayerError> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .positions
            .push(position_seconds);
        Ok(())
    }

    fn set_playback_rate(
        &mut self,
        playback_rate: f64,
    ) -> Result<(), sorotte_player_api::PlayerError> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .playback_rates
            .push(playback_rate);
        Ok(())
    }
}

fn prepared_barrier_owner(
    policy: sorotte_protocol::PlaybackBarrierPolicy,
) -> (
    GuiPersistedConfigRuntimeOwner,
    std::sync::Arc<std::sync::Mutex<CoordinatorAuthorityPlayerState>>,
    SorotteGuiShellAppState,
) {
    use crate::app::runtime_stack::test_support::barrier::*;
    use sorotte_client_core::{LogicalMediaId, MediaLoadIntent, MediaTransportKind};
    use sorotte_protocol::*;
    let mut session = barrier_aware_controller(policy);
    let now = crate::app::support::system_time_seconds();
    session
        .prepare_attached_playback_media(
            LogicalMediaId::new(LOGICAL_MEDIA_ID).unwrap(),
            MediaTransportKind::NetworkVod,
            MediaLoadIntent::NewPlayback,
            now,
        )
        .unwrap();
    let request = barrier_request(&mut session);
    apply_protocol_message(
        &mut session,
        ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(12.0)
                    .with_paused(true)
                    .with_do_seek(true)
                    .with_set_by("alice"),
            ),
        ),
    );
    apply_protocol_message(
        &mut session,
        ProtocolMessage::set(
            SetPayload::new().with_playback_barrier_v1(
                PlaybackBarrierSetExtension::new()
                    .with_prepare(
                        PrepareMediaPayload::new(
                            ROOM_MEDIA_GENERATION,
                            LOGICAL_MEDIA_ID,
                            12.0,
                            policy,
                        )
                        .with_request_nonce(request.request_nonce),
                    )
                    .with_status(barrier_status(
                        policy,
                        PlaybackBarrierPhase::Preparing,
                        None,
                    )),
            ),
        ),
    );
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
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode.mkv")
            .with_path("C:/Media/episode.mkv".to_owned()),
    );
    let shell = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..Default::default()
    });
    owner.sync_session_playstate_to_attached_player_impl(&runtime_state_for_shell(&shell), false);
    let actions = owner
        .session
        .as_mut()
        .unwrap()
        .sync_attached_player_transport_telemetry(
            transport(
                1.0,
                sorotte_player_api::PlayerTransportPhase::ReadyPaused,
                0.0,
                true,
                0,
            ),
            now,
        )
        .unwrap();
    owner.apply_attached_player_runtime_actions_impl(actions, "test ready observation");
    owner.sync_session_playstate_to_attached_player_impl(&runtime_state_for_shell(&shell), false);
    (owner, recorded, shell)
}

fn commit_barrier(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    shell: &SorotteGuiShellAppState,
    policy: sorotte_protocol::PlaybackBarrierPolicy,
) {
    use crate::app::runtime_stack::test_support::barrier::*;
    use sorotte_protocol::*;
    let now = crate::app::support::system_time_seconds();
    let actions = owner
        .session
        .as_mut()
        .unwrap()
        .sync_attached_player_transport_telemetry(
            transport(
                2.0,
                sorotte_player_api::PlayerTransportPhase::ReadyPaused,
                12.0,
                true,
                0,
            ),
            now,
        )
        .unwrap();
    owner.apply_attached_player_runtime_actions_impl(actions, "test seek completion");
    let session = owner.session.as_mut().unwrap();
    apply_protocol_message(
        session,
        ProtocolMessage::set(
            SetPayload::new().with_playback_barrier_v1(
                PlaybackBarrierSetExtension::new()
                    .with_commit(CommitStartPayload::new(
                        ROOM_MEDIA_GENERATION,
                        ROOM_STATE_REVISION,
                        12.0,
                        now,
                        now + 10.0,
                    ))
                    .with_status(barrier_status(
                        policy,
                        PlaybackBarrierPhase::Committed,
                        Some(ROOM_STATE_REVISION),
                    )),
            ),
        ),
    );
    apply_protocol_message(
        session,
        ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(12.0)
                    .with_paused(false)
                    .with_do_seek(true)
                    .with_set_by("alice"),
            ),
        ),
    );
    owner.sync_session_playstate_to_attached_player_impl(&runtime_state_for_shell(shell), false);
}

#[test]
fn gui_controlled_reconnect_toggle_stays_dormant_before_transport_telemetry() {
    const ROOM: &str = "+room:ABCDEF123456";
    let mut adapter = GuiClientSession::new("alice", ROOM);
    let startup = adapter
        .deliver_outbound_protocol_lines()
        .expect("startup Hello should encode");
    assert_eq!(startup.len(), 1);
    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5","features":{"managedRooms":true,"sorottePlaybackBarrierV1":true}}}"#,
        )
        .expect("initial controlled-room Hello should apply");
    adapter
        .apply_message_json(
            r#"{"Set":{"user":{"alice":{"room":{"name":"+room:ABCDEF123456"},"controller":true}}}}"#,
        )
        .expect("initial controller authority should apply");
    adapter
        .prepare_attached_playback_media(
            sorotte_client_core::LogicalMediaId::new("gui-controlled-reconnect-toggle")
                .expect("logical media ID should be valid"),
            sorotte_client_core::MediaTransportKind::LocalFile,
            sorotte_client_core::MediaLoadIntent::NewPlayback,
            0.0,
        )
        .expect("GUI media preparation should succeed")
        .expect("client-core adapter should prepare coordinator media");
    adapter
        .apply_message_json(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
        )
        .expect("initial canonical pause should apply");
    assert!(
        !adapter
            .playback_coordination_snapshot()
            .expect("client-core snapshot should exist")
            .transport_telemetry_observed
    );

    let _ = adapter
        .runtime
        .dispatch(ClientCommand::Reconnect { attempt: 0 });
    adapter
        .runtime
        .session_mut()
        .reset_sync_state_for_reconnect();
    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5","features":{"managedRooms":true,"sorottePlaybackBarrierV1":true}}}"#,
        )
        .expect("replacement controlled-room Hello should apply");
    assert_eq!(
        adapter.runtime.session().local_can_control(),
        Some(true),
        "the cached controller projection is intentionally restored before fresh authority"
    );
    adapter
        .apply_message_json(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
        )
        .expect("replacement canonical pause should apply");
    assert!(
        !adapter
            .playback_coordination_snapshot()
            .expect("client-core snapshot should exist")
            .transport_telemetry_observed,
        "the regression must exercise the GUI before replacement-connection player telemetry"
    );

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.session = Some(Box::new(adapter));
    owner
        .stage_attached_player_pause_intent(false)
        .expect("GUI pause intent staging should succeed");

    assert_eq!(
        owner.pending_local_attached_pause_override, None,
        "the GUI must not mirror a dormant controlled-room command as active player authority"
    );
    let dormant = owner
        .session
        .as_ref()
        .and_then(|session| session.playback_coordination_snapshot())
        .expect("client-core snapshot should remain available");
    assert_eq!(dormant.pending_local_pause_intent, None);
    assert!(dormant.pending_local_pause_intent_dormant);
}

#[test]
fn gui_controller_barrier_reconciles_self_attributed_seek_through_the_player() {
    let (_owner, recorded, _shell) =
        prepared_barrier_owner(sorotte_protocol::PlaybackBarrierPolicy::Controller);
    assert!(recorded.lock().unwrap().positions.contains(&12.0));
}

#[test]
fn gui_all_eligible_controller_participation_obeys_server_commit() {
    for policy in [
        sorotte_protocol::PlaybackBarrierPolicy::Controller,
        sorotte_protocol::PlaybackBarrierPolicy::AllEligible,
    ] {
        let (mut owner, recorded, shell) = prepared_barrier_owner(policy);
        recorded.lock().unwrap().paused.clear();
        commit_barrier(&mut owner, &shell, policy);
        assert_eq!(recorded.lock().unwrap().paused, vec![false]);
    }
}

#[test]
fn gui_controller_obeys_server_owned_room_buffering_pause_and_resume() {
    use crate::app::runtime_stack::test_support::barrier::*;
    use sorotte_protocol::*;
    let (mut owner, recorded, shell) = prepared_barrier_owner(PlaybackBarrierPolicy::Controller);
    commit_barrier(&mut owner, &shell, PlaybackBarrierPolicy::Controller);
    let now = crate::app::support::system_time_seconds();
    let actions = owner
        .session
        .as_mut()
        .unwrap()
        .sync_attached_player_transport_telemetry(
            transport(
                3.0,
                sorotte_player_api::PlayerTransportPhase::Playing,
                12.25,
                false,
                1,
            ),
            now,
        )
        .unwrap();
    owner.apply_attached_player_runtime_actions_impl(actions, "test playing observation");
    let actions = owner
        .session
        .as_mut()
        .unwrap()
        .sync_attached_player_transport_telemetry(
            transport(
                3.5,
                sorotte_player_api::PlayerTransportPhase::Playing,
                12.5,
                false,
                1,
            ),
            now,
        )
        .unwrap();
    owner.apply_attached_player_runtime_actions_impl(actions, "test advancing observation");
    assert!(
        drain_barrier_state_extensions(owner.session.as_mut().unwrap())
            .iter()
            .any(|extension| extension.started.is_some()),
        "the client must acknowledge actual playback before the server completes the barrier"
    );
    apply_protocol_message(
        owner.session.as_mut().unwrap(),
        ProtocolMessage::set(SetPayload::new().with_playback_barrier_v1(
            PlaybackBarrierSetExtension::new().with_status(barrier_status(
                PlaybackBarrierPolicy::Controller,
                PlaybackBarrierPhase::Complete,
                Some(ROOM_STATE_REVISION),
            )),
        )),
    );
    recorded.lock().unwrap().paused.clear();
    let policy = RoomBufferingPolicyPayload::new(
        ROOM_MEDIA_GENERATION,
        RoomBufferingPolicy::PauseAnyEligible,
    )
    .with_debounce_ms(1)
    .with_resume_hysteresis_ms(1)
    .with_max_pause_ms(30_000);
    apply_protocol_message(
        owner.session.as_mut().unwrap(),
        ProtocolMessage::set(
            SetPayload::new().with_playback_barrier_v1(
                PlaybackBarrierSetExtension::new()
                    .with_buffering_policy(policy.clone())
                    .with_buffering_status(RoomBufferingStatusPayload {
                        config: policy,
                        phase: RoomBufferingPhase::Paused,
                        eligible_clients: 1,
                        required_buffering_clients: 1,
                        buffering_clients: ["alice".to_owned()].into(),
                        pause_deadline: None,
                    }),
            ),
        ),
    );
    for (i, paused) in [true, false].into_iter().enumerate() {
        apply_protocol_message(
            owner.session.as_mut().unwrap(),
            ProtocolMessage::state(
                StatePayload::new().with_playstate(
                    PlaystatePayload::new()
                        .with_position(12.25)
                        .with_paused(paused)
                        .with_do_seek(false)
                        .with_set_by("alice"),
                ),
            ),
        );
        owner.sync_session_playstate_to_attached_player_impl(
            &runtime_state_for_shell(&shell),
            false,
        );
        let phase = if paused {
            sorotte_player_api::PlayerTransportPhase::ReadyPaused
        } else {
            sorotte_player_api::PlayerTransportPhase::Playing
        };
        let actions = owner
            .session
            .as_mut()
            .unwrap()
            .sync_attached_player_transport_telemetry(
                transport(4.0 + i as f64, phase, 12.25, paused, 1),
                crate::app::support::system_time_seconds(),
            )
            .unwrap();
        owner.apply_attached_player_runtime_actions_impl(actions, "test buffering observation");
    }
    assert_eq!(recorded.lock().unwrap().paused, vec![true, false]);
}

#[test]
fn gui_recovery_interrupt_resets_rate_on_the_real_attached_player() {
    use crate::app::runtime_stack::test_support::barrier::{
        accept_coordinator_commands, transport,
    };
    use sorotte_player_api::PlayerTransportPhase::*;
    let mut session = crate::app::runtime_stack::test_support::active_session();
    session
        .prepare_attached_playback_media(
            sorotte_client_core::LogicalMediaId::new("catchup-media").unwrap(),
            sorotte_client_core::MediaTransportKind::NetworkVod,
            sorotte_client_core::MediaLoadIntent::NewPlayback,
            0.0,
        )
        .unwrap();
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":0.0,"paused":false,"setBy":"bob"}}}"#,
            0.0,
        )
        .unwrap();
    let actions = session.attached_player_runtime_actions(0.0).unwrap();
    accept_coordinator_commands(&mut session, &actions, 0.0);
    for (time, phase, position, rate) in [
        (0.0, Playing, 0.0, 1.0),
        (0.2, Playing, 0.2, 1.0),
        (10.0, Rebuffering, 8.0, 1.0),
        (11.0, Playing, 9.0, 1.0),
        (12.0, Playing, 10.0, 1.0),
        (13.0, Playing, 11.0, 1.03),
    ] {
        let mut update = transport(time, phase, position, false, 1);
        update.playback_rate = Some(rate);
        let actions = session
            .sync_attached_player_transport_telemetry(update, time)
            .unwrap();
        accept_coordinator_commands(&mut session, &actions, time);
    }
    assert!(
        session
            .playback_coordination_snapshot()
            .unwrap()
            .recovery_episode
            .is_some()
    );
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
    assert!(owner.interrupt_attached_playback_recovery_impl("test interruption"));
    assert_eq!(recorded.lock().unwrap().playback_rates, vec![1.0]);
    assert!(
        owner
            .session
            .as_ref()
            .unwrap()
            .playback_coordination_snapshot()
            .unwrap()
            .recovery_episode
            .is_none()
    );
}

#[test]
fn gui_persisted_config_runtime_owner_skips_self_origin_room_position_sync_for_attached_player() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_paused_values: Vec<bool>,
        set_positions: Vec<f64>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_recording_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );
    owner.player_position_seconds = Some(41.0);

    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettings::default()
    });

    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("hello should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"State":{"playstate":{"position":42.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
        )
        .expect("self-origin room playstate should apply");

    owner.sync_session_playstate_to_attached_player_impl(&runtime_state_for_shell(&state), true);

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded.set_positions.is_empty(),
        "force-sync should not replay the local user's own room position back into the attached player"
    );
    assert!(
        recorded.set_paused_values.is_empty(),
        "force-sync should not replay the local user's own room pause state back into the attached player"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_ignores_unattributed_room_playstate_when_no_remote_users_are_known()
 {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_positions: Vec<f64>,
        set_paused_values: Vec<bool>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_recording_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );
    owner.player_position_seconds = Some(41.0);
    owner.player_paused = Some(false);

    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettings::default()
    });

    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("hello should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(r#"{"State":{"playstate":{"position":0.0,"paused":true}}}"#)
        .expect("unattributed room playstate should apply");

    owner.sync_session_playstate_to_attached_player_impl(&runtime_state_for_shell(&state), false);

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded.set_positions.is_empty(),
        "room playstate without remote authority should not rewind the attached player while alone"
    );
    assert!(
        recorded.set_paused_values.is_empty(),
        "room playstate without remote authority should not pause the attached player while alone"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_waits_for_local_file_before_applying_room_playstate() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_positions: Vec<f64>,
        set_paused_values: Vec<bool>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_recording_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));

    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettings::default()
    });

    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("hello should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":true,"setBy":"bob"}}}"#,
        )
        .expect("room playstate should apply");

    owner.sync_session_playstate_to_attached_player_impl(&runtime_state_for_shell(&state), false);
    {
        let recorded = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(recorded.set_positions.is_empty());
        assert!(recorded.set_paused_values.is_empty());
    }
    assert_eq!(owner.last_applied_attached_room_playstate, None);

    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );
    owner.sync_session_playstate_to_attached_player_impl(&runtime_state_for_shell(&state), false);

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded
            .set_positions
            .iter()
            .any(|position| (*position - 10.0).abs() < 1.0),
        "room playstate should seek once the attached player reports a local file"
    );
    assert_eq!(recorded.set_paused_values, vec![true]);
}

#[test]
fn gui_persisted_config_runtime_owner_waits_for_advancement_without_seeking_on_cache_release() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        events: Option<ScriptedPlayerEvents>,
        set_positions: Vec<f64>,
        set_paused_values: Vec<bool>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn take_player_event_batch(&mut self) -> Option<sorotte_player_api::PlayerEventBatch> {
            self.state
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .events
                .as_ref()
                .and_then(ScriptedPlayerEvents::peek)
        }
        fn acknowledge_player_event_batch(
            &mut self,
            token: sorotte_player_api::PlayerEventAcknowledgementToken,
        ) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .events
                .as_mut()
                .expect("scripted ingress")
                .acknowledge(token)
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState {
        events: Some(active_player_events(1)),
        ..Default::default()
    }));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_recording_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );
    owner.player_position_seconds = Some(3.0);
    owner.player_paused = Some(false);
    owner.player_paused_for_cache = Some(true);

    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettings::default()
    });

    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#,
        )
        .expect("hello should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
        .expect("local ready should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"State":{"playstate":{"position":30.0,"paused":false,"doSeek":true,"setBy":"bob"}}}"#,
        )
        .expect("room seek should apply");

    owner.sync_session_playstate_to_attached_player_impl(&runtime_state_for_shell(&state), false);
    {
        let mut recorded = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(
            !recorded.set_positions.is_empty(),
            "room seek should still be applied while cache pause defers unpause"
        );
        assert!(
            recorded.set_paused_values.is_empty(),
            "cache pause should defer room unpause"
        );
        recorded.set_positions.clear();
    }
    assert_eq!(
        owner
            .attached_system_seek_ownership
            .back()
            .map(|ownership| ownership.source),
        Some(GuiAttachedSystemSeekSource::RuntimeAction),
        "legacy room-state correction should retain ownership of its physical player seek"
    );
    assert!(
        owner
            .attached_system_seek_ownership
            .back()
            .is_some_and(|ownership| (ownership.target_position_seconds - 30.0).abs() < 0.01),
        "legacy room-state ownership should retain the aged room target"
    );
    assert!(owner.pending_attached_room_unpause_observation.is_some());
    assert_eq!(owner.player_paused, Some(false));
    assert_eq!(owner.last_applied_attached_room_playstate, None);

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .events
        .as_mut()
        .expect("scripted physical observations")
        .push_event(playback_event(
            1,
            sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
                .with_paused_for_cache(false),
        ));
    owner.refresh_player_state_impl();
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"State":{"playstate":{"position":34.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
        )
        .expect("post-cache room playstate should apply");

    owner.sync_session_playstate_to_attached_player_impl(&runtime_state_for_shell(&state), false);

    {
        let recorded = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(
            recorded.set_positions.is_empty(),
            "cache release alone must not seek to the room's newer moving position"
        );
        assert!(
            recorded.set_paused_values.is_empty(),
            "cache release alone must not replay the room unpause"
        );
    }
    assert!(
        owner.pending_attached_room_unpause_observation.is_some(),
        "cache release is not evidence that playback has resumed"
    );
    assert_eq!(owner.last_applied_attached_room_playstate, None);

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .events
        .as_mut()
        .expect("scripted physical observations")
        .push_event(playback_event(
            1,
            sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(30.0)
                .with_paused(false),
        ));
    owner.refresh_player_state_impl();
    owner.sync_session_playstate_to_attached_player_impl(&runtime_state_for_shell(&state), false);
    assert!(
        owner.pending_attached_room_unpause_observation.is_some(),
        "one stationary post-cache sample must keep desired play pending"
    );
    assert_eq!(owner.last_applied_attached_room_playstate, None);

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .events
        .as_mut()
        .expect("scripted physical observations")
        .push_event(playback_event(
            1,
            sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(30.25)
                .with_paused(false),
        ));
    owner.refresh_player_state_impl();
    owner.sync_session_playstate_to_attached_player_impl(&runtime_state_for_shell(&state), false);
    assert!(
        owner.pending_attached_room_unpause_observation.is_none(),
        "fresh forward position advancement should acknowledge desired play"
    );
    assert!(
        owner.last_applied_attached_room_playstate.is_some(),
        "the room playstate may be marked applied only after advancement is observed"
    );
    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(recorded.set_positions.is_empty());
    assert!(recorded.set_paused_values.is_empty());
    assert_ne!(
        owner
            .session
            .as_ref()
            .and_then(|session| session.local_pause_state()),
        Some(true),
        "observed post-cache recovery should not become a manual pause"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_retains_room_play_until_advancement_after_ipc_acceptance() {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        events: Option<ScriptedPlayerEvents>,
        set_positions: Vec<f64>,
        set_paused_values: Vec<bool>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn take_player_event_batch(&mut self) -> Option<sorotte_player_api::PlayerEventBatch> {
            self.state
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .events
                .as_ref()
                .and_then(ScriptedPlayerEvents::peek)
        }
        fn acknowledge_player_event_batch(
            &mut self,
            token: sorotte_player_api::PlayerEventAcknowledgementToken,
        ) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .events
                .as_mut()
                .expect("scripted ingress")
                .acknowledge(token)
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState {
        events: Some(active_player_events(1)),
        ..Default::default()
    }));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_recording_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path("C:/Media/episode1.mkv".to_owned()),
    );
    owner.player_position_seconds = Some(10.0);
    owner.player_paused = Some(true);

    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettings::default()
    });
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("hello should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
        )
        .expect("room play should apply");

    owner.sync_session_playstate_to_attached_player_impl(&runtime_state_for_shell(&state), false);
    let baseline_position_seconds = owner
        .player_position_seconds
        .expect("room sync should retain an observation baseline");
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values,
        vec![false]
    );
    assert_eq!(
        owner.player_paused,
        Some(true),
        "IPC acceptance must not overwrite the last observed pause property"
    );
    assert!(owner.pending_attached_room_unpause_observation.is_some());
    assert_eq!(
        owner.last_applied_attached_room_playstate, None,
        "IPC acceptance alone must not mark desired play as applied"
    );

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .events
        .as_mut()
        .expect("scripted physical observations")
        .push_event(playback_event(
            1,
            sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(baseline_position_seconds)
                .with_paused(false),
        ));
    owner.refresh_player_state_impl();
    owner.sync_session_playstate_to_attached_player_impl(&runtime_state_for_shell(&state), false);
    assert!(
        owner.pending_attached_room_unpause_observation.is_some(),
        "a pause=false property without forward motion is not observed playback"
    );
    assert_eq!(owner.last_applied_attached_room_playstate, None);

    player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .events
        .as_mut()
        .expect("scripted physical observations")
        .push_event(playback_event(
            1,
            sorotte_player_api::PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(baseline_position_seconds + 0.25)
                .with_paused(false),
        ));
    owner.refresh_player_state_impl();
    owner.sync_session_playstate_to_attached_player_impl(&runtime_state_for_shell(&state), false);

    assert!(owner.pending_attached_room_unpause_observation.is_none());
    assert!(owner.last_applied_attached_room_playstate.is_some());
    assert_eq!(
        player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_paused_values,
        vec![false],
        "retained desired play should not busy-loop unpause commands while awaiting observations"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_does_not_force_room_sync_for_matched_playlist_target_without_reset_intent()
 {
    #[derive(Debug, Default)]
    struct RecordingPlayerState {
        set_positions: Vec<f64>,
        set_paused_values: Vec<bool>,
    }

    struct RecordingPlayerAdapter {
        state: std::sync::Arc<std::sync::Mutex<RecordingPlayerState>>,
    }

    impl PlayerAdapter for RecordingPlayerAdapter {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn set_position(
            &mut self,
            position_seconds: f64,
        ) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_positions
                .push(position_seconds);
            Ok(())
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), sorotte_player_api::PlayerError> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_paused_values
                .push(paused);
            Ok(())
        }
    }

    let root = test_temp_root("matched-playlist-target-no-reset");
    let media_path = root.join("episode1.mkv");
    std::fs::write(&media_path, b"test").expect("playlist target fixture should be written");

    let player_state = std::sync::Arc::new(std::sync::Mutex::new(RecordingPlayerState::default()));
    let (mut owner, _session_transport) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_recording_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(RecordingPlayerAdapter {
        state: player_state.clone(),
    })));
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new("episode1.mkv")
            .with_path(media_path.to_string_lossy().into_owned()),
    );
    owner.player_position_seconds = Some(0.0);
    owner.player_paused = Some(false);

    let stored_settings = StoredClientSettings {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(true),
        media_search_directories: Some(vec![root.to_string_lossy().into_owned()]),
        rewind_on_desync: Some(false),
        fastforward_on_desync: Some(false),
        slow_on_desync: Some(false),
        ..StoredClientSettings::default()
    };
    let mut state = SorotteGuiShellAppState::from_stored_settings(&stored_settings);
    state.apply_shared_playlist_entries(vec!["episode1.mkv".to_owned()], Some(0), false);
    owner.active_shared_playlist_index = Some(0);
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .sync_runtime_settings(&stored_client_settings_runtime_snapshot(&stored_settings))
        .expect("runtime settings should sync");

    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("hello should apply");
    owner
        .session
        .as_mut()
        .expect("session should exist")
        .apply_message_json(
            r#"{"State":{"playstate":{"position":41.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
        )
        .expect("room playstate should apply");

    owner.sync_session_playstate_to_attached_player_impl(&runtime_state_for_shell(&state), false);
    {
        let mut recorded = player_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        recorded.set_positions.clear();
        recorded.set_paused_values.clear();
    }
    owner.player_position_seconds = Some(42.0);

    let selected_media_sync = owner.sync_selected_shared_playlist_media_to_attached_player_impl(
        &runtime_state_for_shell(&state),
    );
    assert_eq!(
        selected_media_sync,
        SelectedPlaylistMediaSyncOutcome::MatchedCurrentTarget
    );

    let selection_handoff_ready = selected_media_sync.selection_handoff_ready(
        owner
            .session
            .as_ref()
            .expect("session should exist")
            .has_pending_playlist_index_reset_intent(),
    );
    assert!(
        !selection_handoff_ready,
        "matched playlist targets without a pending reset should not force a room playstate handoff"
    );

    owner.apply_pending_playlist_index_reset_to_attached_player_impl(
        &runtime_state_for_shell(&state),
        selection_handoff_ready,
    );
    owner.sync_session_playstate_to_attached_player_impl(
        &runtime_state_for_shell(&state),
        selection_handoff_ready,
    );

    let recorded = player_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        recorded.set_positions.is_empty(),
        "playlist updates that keep the current target selected should not rewind the attached player; recorded={recorded:?}"
    );
    assert!(
        recorded.set_paused_values.is_empty(),
        "playlist updates that keep the current target selected should not toggle pause state; recorded={recorded:?}"
    );
    assert_eq!(owner.player_position_seconds, Some(42.0));

    let _ = std::fs::remove_dir_all(&root);
}
