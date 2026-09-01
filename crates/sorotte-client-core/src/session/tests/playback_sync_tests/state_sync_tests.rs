use super::*;
use crate::{LogicalMediaId, MediaTransportKind};

#[test]
fn transport_revision_fence_rejects_backwards_zero_and_untagged_downgrades() {
    let mut session = ClientSession::default();
    session
        .apply_message_json_at(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            1.0,
        )
        .expect("hello should apply");
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"bob","sorotteTransportRevision":12}}}"#,
            2.0,
        )
        .expect("tagged authority should apply");

    for stale in [
        r#"{"State":{"playstate":{"position":1.0,"paused":false,"doSeek":true,"setBy":"mallory","sorotteTransportRevision":11}}}"#,
        r#"{"State":{"playstate":{"position":2.0,"paused":false,"doSeek":true,"setBy":"mallory","sorotteTransportRevision":0}}}"#,
        r#"{"State":{"playstate":{"position":2.5,"paused":false,"doSeek":true,"setBy":"mallory","sorotteTransportRevision":"invalid"}}}"#,
        r#"{"State":{"playstate":{"position":3.0,"paused":false,"doSeek":true,"setBy":"mallory"}}}"#,
    ] {
        session
            .apply_message_json_at(stale, 3.0)
            .expect("invalid authority should be ignored without rejecting the State frame");
    }

    assert_eq!(session.current_room_transport_revision(), Some(12));
    assert_eq!(
        session.current_room_playstate(),
        Some(&RoomPlaystateView {
            position: Some(10.0),
            paused: Some(true),
            do_seek: Some(false),
            set_by: Some("bob".to_owned()),
        }),
        "retired or downgraded authority must not be laundered into the current revision"
    );
}

#[test]
fn room_membership_change_resets_transport_revision_ordering() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"sorotteTransportRevision":12}}}"#,
        )
        .expect("first membership authority should apply");
    session
        .apply_message_json(r#"{"Set":{"room":{"name":"room2"}}}"#)
        .expect("room2 switch should apply");
    session
        .apply_message_json(r#"{"Set":{"room":{"name":"room1"}}}"#)
        .expect("room1 rejoin should apply");
    session
        .apply_message_json(
            r#"{"State":{"playstate":{"position":1.0,"paused":true,"sorotteTransportRevision":1}}}"#,
        )
        .expect("successor membership authority should apply");

    assert_eq!(session.current_room_transport_revision(), Some(1));
    assert_eq!(
        session
            .current_room_playstate()
            .and_then(|playstate| playstate.position),
        Some(1.0),
        "a recreated room's first revision must not be compared with a retired membership"
    );
}

#[test]
fn first_remote_room_revision_waits_for_physical_convergence_before_echo() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");

    let player = RecordingPlayer {
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(12.5)
                .with_paused(true),
        ),
        ..RecordingPlayer::default()
    };
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    let sent = runtime.run_state_sync_reconcile_with_inbound_state(
        StatePayload::new()
            .with_playstate(
                PlaystatePayload::new()
                    .with_position(10.0)
                    .with_paused(false)
                    .with_set_by("bob")
                    .with_transport_revision(12),
            )
            .with_ping(PingPayload::new().with_latency_calculation(42.0)),
        100.0,
        0.25,
        false,
    );

    assert!(sent, "state sync should emit after telemetry is available");
    assert_eq!(
        runtime.control().outbound_messages().len(),
        1,
        "state sync should queue one outbound state message"
    );
    let ProtocolMessage::State(state_message) = &runtime.control().outbound_messages()[0] else {
        panic!("queued message should be State");
    };
    assert!(
        state_message.state.playstate.is_none(),
        "a late joiner must not pair revision 12 with its pre-membership player sample"
    );
    assert_eq!(
        runtime.session().local_position_seconds(),
        Some(12.5),
        "the local model must retain the sampled player position until correction is applied"
    );
    assert_eq!(
        runtime.session().local_paused(),
        Some(true),
        "echoing canonical authority must not pretend the physical player has already played"
    );
    assert_eq!(
        state_message.state.ignoring_on_the_fly, None,
        "baseline initialization is not a local transport mutation"
    );
    assert_eq!(
        state_message
            .state
            .ping
            .as_ref()
            .and_then(|ping| ping.latency_calculation),
        Some(42.0),
        "outbound ping should echo inbound latencyCalculation when present"
    );
    assert_eq!(
        state_message
            .state
            .ping
            .as_ref()
            .and_then(|ping| ping.client_latency_calculation),
        Some(100.0),
        "outbound ping should include client latency calculation"
    );
    assert_eq!(
        state_message
            .state
            .ping
            .as_ref()
            .and_then(|ping| ping.client_rtt),
        Some(0.25),
        "outbound ping should include client RTT"
    );

    runtime.flush_queued_protocol_messages();
    runtime
        .player_mut_for_test()
        .pending_playback_telemetry_update = Some(
        PlayerPlaybackTelemetryUpdate::default()
            .with_position_seconds(12.6)
            .with_paused(false),
    );
    assert!(
        runtime.run_state_sync_reconcile_with_inbound_state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(10.0)
                    .with_paused(false)
                    .with_set_by("bob")
                    .with_transport_revision(12),
            ),
            100.0,
            0.25,
            false,
        )
    );
    let ProtocolMessage::State(converged_response) = &runtime.control().outbound_messages()[0]
    else {
        panic!("post-command evidence should receive State");
    };
    let converged_playstate = converged_response
        .state
        .playstate
        .as_ref()
        .expect("physical convergence should release revision 12");
    assert_eq!(converged_playstate.position, Some(12.6));
    assert_eq!(converged_playstate.paused, Some(false));
    assert_eq!(
        converged_playstate.transport_revision().unwrap(),
        Some(12),
        "the post-effect sample must remain causally bound to the server revision it observed"
    );
}

#[test]
fn observed_explicit_pause_intent_can_mutate_the_first_remote_room_baseline() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");

    let player = RecordingPlayer {
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(12.5)
                .with_paused(false),
        ),
        ..RecordingPlayer::default()
    };
    let mut runtime = ClientRuntime::new(session, player, QueuedRuntimeControl::default());
    runtime.prepare_playback_media(
        LogicalMediaId::new("pre-baseline-explicit-play").expect("logical ID should be valid"),
        MediaTransportKind::LocalFile,
        0.0,
    );
    runtime.stage_external_player_pause_intent(false, 0.01);

    assert!(
        runtime.run_state_sync_reconcile_with_inbound_state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(10.0)
                    .with_paused(true)
                    .with_set_by("bob"),
            ),
            0.0,
            0.0,
            false,
        )
    );

    let ProtocolMessage::State(state_message) = &runtime.control().outbound_messages()[0] else {
        panic!("queued message should be State");
    };
    let playstate = state_message
        .state
        .playstate
        .as_ref()
        .expect("explicit transport intent should produce playstate");
    assert_eq!(playstate.position, Some(12.5));
    assert_eq!(playstate.paused, Some(false));
    assert_eq!(
        runtime
            .playback_coordination_snapshot()
            .pending_local_pause_intent,
        Some(false),
        "the intent remains fenced until a canonical acknowledgement"
    );
}

#[test]
fn staged_pause_intent_overrides_stale_playing_telemetry_in_state_response() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");

    let player = RecordingPlayer {
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(12.5)
                .with_paused(false),
        ),
        ..RecordingPlayer::default()
    };
    let mut runtime = ClientRuntime::new(session, player, QueuedRuntimeControl::default());
    runtime.prepare_playback_media(
        LogicalMediaId::new("pause-command-before-player-edge")
            .expect("logical ID should be valid"),
        MediaTransportKind::LocalFile,
        0.0,
    );
    runtime.stage_external_player_pause_intent(true, 0.01);

    assert!(
        runtime.run_state_sync_reconcile_with_inbound_state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(10.0)
                    .with_paused(false)
                    .with_set_by("bob"),
            ),
            0.0,
            0.0,
            false,
        )
    );

    let ProtocolMessage::State(state_message) = &runtime.control().outbound_messages()[0] else {
        panic!("queued message should be State");
    };
    let playstate = state_message
        .state
        .playstate
        .as_ref()
        .expect("the explicit Pause should produce playstate");
    assert_eq!(playstate.position, Some(12.5));
    assert_eq!(
        playstate.paused,
        Some(true),
        "the staged semantic command, not the preceding mpv sample, owns canonical mutation"
    );
    assert_eq!(
        runtime.session().local_paused(),
        Some(false),
        "publishing the command must not falsify the still-playing physical observation"
    );
    assert_eq!(
        runtime
            .playback_coordination_snapshot()
            .pending_local_pause_intent,
        Some(true),
        "the command remains fenced until both server and player confirm it"
    );
}

#[test]
fn heartbeat_publishes_pending_pause_instead_of_pre_command_player_sample() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(
            r#"{"State":{"playstate":{"position":10.0,"paused":false,"setBy":"bob"}}}"#,
        )
        .expect("canonical playing state should apply");

    let player = RecordingPlayer {
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(12.5)
                .with_paused(false),
        ),
        ..RecordingPlayer::default()
    };
    let mut runtime = ClientRuntime::new(session, player, QueuedRuntimeControl::default());
    runtime.prepare_playback_media(
        LogicalMediaId::new("pause-heartbeat-before-player-edge")
            .expect("logical ID should be valid"),
        MediaTransportKind::LocalFile,
        0.0,
    );
    runtime.stage_external_player_pause_intent(true, 0.01);

    assert!(runtime.run_state_sync_heartbeat_legacy_ping_compatible(false));
    let ProtocolMessage::State(state_message) = &runtime.control().outbound_messages()[0] else {
        panic!("queued heartbeat should be State");
    };
    assert_eq!(
        state_message
            .state
            .playstate
            .as_ref()
            .and_then(|playstate| playstate.paused),
        Some(true),
        "a heartbeat in the command/player race must carry the pending Pause"
    );
    assert_eq!(
        runtime.session().local_paused(),
        Some(false),
        "heartbeat publication must retain the physical player observation separately"
    );
}

#[test]
fn physical_pause_lag_without_explicit_intent_cannot_echo_over_room_authority() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");

    let player = RecordingPlayer {
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(10.0)
                .with_paused(true),
        ),
        ..RecordingPlayer::default()
    };
    let mut runtime = ClientRuntime::new(session, player, QueuedRuntimeControl::default());
    let canonical_playing = || {
        StatePayload::new().with_playstate(
            PlaystatePayload::new()
                .with_position(10.0)
                .with_paused(false)
                .with_set_by("bob"),
        )
    };

    assert!(runtime.run_state_sync_reconcile_with_inbound_state(
        canonical_playing(),
        0.0,
        0.0,
        false,
    ));
    let _ = runtime.flush_queued_protocol_messages();

    // The room baseline has arrived, but the external player has not applied
    // its correction yet. This physical lag is observation, not user intent.
    runtime
        .player_mut_for_test()
        .pending_playback_telemetry_update = Some(
        PlayerPlaybackTelemetryUpdate::default()
            .with_position_seconds(10.0)
            .with_paused(true),
    );
    assert!(runtime.run_state_sync_reconcile_with_inbound_state(
        canonical_playing(),
        0.0,
        0.0,
        false,
    ));

    let ProtocolMessage::State(response) = &runtime.control().outbound_messages()[0] else {
        panic!("canonical acknowledgement should remain State");
    };
    let playstate = response
        .state
        .playstate
        .as_ref()
        .expect("available telemetry should retain a playstate sample");
    assert_eq!(
        playstate.paused,
        Some(false),
        "a lagging player must echo canonical pause until an explicit local intent is staged"
    );
    assert_eq!(playstate.do_seek, None);
    assert_eq!(response.state.ignoring_on_the_fly, None);
}

#[test]
fn client_runtime_state_sync_reconcile_emits_ping_only_without_local_playback_state() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    let sent = runtime.run_state_sync_reconcile_with_inbound_state(
        StatePayload::new()
            .with_playstate(
                PlaystatePayload::new()
                    .with_position(10.0)
                    .with_paused(false)
                    .with_set_by("bob"),
            )
            .with_ping(PingPayload::new().with_latency_calculation(42.0)),
        100.0,
        0.25,
        false,
    );

    assert!(
        sent,
        "state sync should echo ping even without local playback telemetry"
    );
    assert_eq!(
        runtime.control().outbound_messages().len(),
        1,
        "state sync should queue one outbound state message"
    );
    let ProtocolMessage::State(state_message) = &runtime.control().outbound_messages()[0] else {
        panic!("queued message should be State");
    };
    assert_eq!(
        state_message.state.playstate, None,
        "ping-only response should omit playstate without local telemetry"
    );
    let ping = state_message
        .state
        .ping
        .as_ref()
        .expect("ping-only response should include ping metadata");
    assert_eq!(
        ping.latency_calculation,
        Some(42.0),
        "ping-only response should echo inbound latencyCalculation"
    );
    assert_eq!(
        ping.client_latency_calculation,
        Some(100.0),
        "ping-only response should include client latency calculation"
    );
    assert_eq!(
        ping.client_rtt,
        Some(0.25),
        "ping-only response should include client RTT"
    );
}

#[test]
fn ping_only_state_response_echoes_forced_state_ack_without_local_telemetry() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    let sent = runtime.run_state_sync_reconcile_with_inbound_state(
        StatePayload::new()
            .with_playstate(
                PlaystatePayload::new()
                    .with_position(10.0)
                    .with_paused(false)
                    .with_set_by("bob"),
            )
            .with_ping(PingPayload::new().with_latency_calculation(42.0))
            .with_ignoring_on_the_fly(IgnoringOnTheFlyPayload::new().with_server(7)),
        100.0,
        0.25,
        false,
    );

    assert!(sent, "forced state should still get a ping-only response");
    let ProtocolMessage::State(state_message) = &runtime.control().outbound_messages()[0] else {
        panic!("queued message should be State");
    };
    assert_eq!(
        state_message.state.playstate, None,
        "ping-only response should omit playstate without local telemetry"
    );
    let ignoring = state_message
        .state
        .ignoring_on_the_fly
        .as_ref()
        .expect("ping-only response should acknowledge forced state");
    assert_eq!(
        ignoring.server,
        Some(7),
        "ping-only response should echo the inbound server ignore counter"
    );
    assert_eq!(
        runtime.session().server_ignoring_on_the_fly(),
        0,
        "server ignore counter should be cleared after it is echoed"
    );
    let room_playstate = runtime
        .session()
        .current_room_playstate()
        .expect("inbound playstate should still update the room state");
    assert_eq!(room_playstate.position, Some(10.0));
    assert_eq!(room_playstate.paused, Some(false));
}

#[test]
fn client_runtime_state_sync_heartbeat_emits_when_active_even_if_chat_is_disabled() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255","features":{"chat":false}}}"#,
        )
        .expect("hello should apply");

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    assert!(
        runtime.run_state_sync_heartbeat_legacy_ping_compatible(false),
        "an active session should heartbeat independently of the chat capability"
    );
    assert_eq!(
        runtime.control().outbound_messages().len(),
        1,
        "heartbeat should queue one outbound state message"
    );
    let ProtocolMessage::State(state_message) = &runtime.control().outbound_messages()[0] else {
        panic!("queued heartbeat should be State");
    };
    assert_eq!(
        state_message.state.playstate, None,
        "heartbeat without local playback telemetry should omit playstate"
    );
    let ping = state_message
        .state
        .ping
        .as_ref()
        .expect("heartbeat should include ping metadata");
    assert!(
        ping.client_latency_calculation.unwrap_or(0.0) > 0.0,
        "heartbeat should include non-zero clientLatencyCalculation"
    );
    assert!(
        ping.client_rtt.is_some(),
        "heartbeat should include clientRtt even without local playback telemetry"
    );
}

#[test]
fn blocked_state_write_coalesces_repeated_heartbeats_to_the_latest_pending_state() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(
            r#"{"State":{"playstate":{"position":0.0,"paused":false,"setBy":"bob"}}}"#,
        )
        .expect("room playback state should apply");
    let player = RecordingPlayer {
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(10.0)
                .with_paused(false),
        ),
        ..RecordingPlayer::default()
    };
    let mut runtime = ClientRuntime::new(session, player, QueuedRuntimeControl::default());

    assert!(runtime.run_state_sync_heartbeat_legacy_ping_compatible(false));
    runtime
        .flush_queued_protocol_lines_to_transport(|_| {
            Err(ProtocolError::ServerError {
                message: "blocked writer".to_owned(),
            })
        })
        .expect_err("blocked writer should leave State pending");

    for position in [20.0, 30.0, 40.0] {
        runtime
            .player_mut_for_test()
            .pending_playback_telemetry_update = Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(position)
                .with_paused(false),
        );
        assert!(runtime.run_state_sync_heartbeat_legacy_ping_compatible(false));
    }

    assert_eq!(
        runtime.control().outbound_messages().len(),
        1,
        "blocked heartbeats should keep only one coalesced pending State"
    );
    let ProtocolMessage::State(state) = &runtime.control().outbound_messages()[0] else {
        panic!("coalesced protocol message should be State");
    };
    assert_eq!(
        state
            .state
            .playstate
            .as_ref()
            .and_then(|playstate| playstate.position),
        Some(40.0),
        "the coalesced State should contain the newest playback telemetry"
    );
}

#[test]
fn leased_state_keeps_staged_bytes_stable_while_newer_heartbeats_coalesce() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(
            r#"{"State":{"playstate":{"position":0.0,"paused":false,"setBy":"bob"}}}"#,
        )
        .expect("room playback state should apply");
    let player = RecordingPlayer {
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(10.0)
                .with_paused(false),
        ),
        ..RecordingPlayer::default()
    };
    let mut runtime = ClientRuntime::new(session, player, QueuedRuntimeControl::default());

    assert!(runtime.run_state_sync_heartbeat_legacy_ping_compatible(false));
    let staged_line = runtime
        .pending_protocol_line()
        .expect("State should encode")
        .expect("State should be pending");

    for position in [20.0, 30.0] {
        runtime
            .player_mut_for_test()
            .pending_playback_telemetry_update = Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(position)
                .with_paused(false),
        );
        assert!(runtime.run_state_sync_heartbeat_legacy_ping_compatible(false));
    }

    assert_eq!(
        runtime
            .pending_protocol_line()
            .expect("leased State should still encode"),
        Some(staged_line.clone()),
        "new heartbeats must not mutate bytes already staged with a transport"
    );
    assert!(matches!(
        runtime.acknowledge_protocol_line(staged_line.lease()),
        Some(ProtocolMessage::State(_))
    ));
    let ProtocolMessage::State(latest) = &runtime.control().outbound_messages()[0] else {
        panic!("the message after the leased State should be the latest State");
    };
    assert_eq!(
        latest
            .state
            .playstate
            .as_ref()
            .and_then(|playstate| playstate.position),
        Some(30.0)
    );
}

#[test]
fn client_runtime_state_sync_heartbeat_reports_room_position_when_dont_slow_down_with_me_is_enabled()
 {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session
        .apply_message_json(
            r#"{"State":{"playstate":{"position":10.0,"paused":false,"setBy":"bob"}}}"#,
        )
        .expect("room state should apply");
    let now_seconds = unix_wall_clock_time_seconds_legacy_compatible();
    session
        .model
        .room
        .playstate_updated_at_seconds
        .insert("room1".to_owned(), now_seconds - 2.0);

    let player = RecordingPlayer {
        pending_playback_telemetry_update: Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(15.0)
                .with_paused(false),
        ),
        ..RecordingPlayer::default()
    };
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);

    assert!(
        runtime.run_state_sync_heartbeat_legacy_ping_compatible(true),
        "heartbeat should emit state while the session is active"
    );

    let ProtocolMessage::State(state_message) = &runtime.control().outbound_messages()[0] else {
        panic!("queued heartbeat should be State");
    };
    let outbound_position = state_message
        .state
        .playstate
        .as_ref()
        .and_then(|playstate| playstate.position)
        .expect("heartbeat should include playstate position");
    assert!(
        (11.0..13.5).contains(&outbound_position),
        "dontSlowDownWithMe should report the room position instead of the locally fast-forwarded player time, got {outbound_position}"
    );
    assert_eq!(
        state_message
            .state
            .playstate
            .as_ref()
            .and_then(|playstate| playstate.paused),
        Some(false)
    );
}

#[test]
fn client_runtime_desync_correction_legacy_ping_forward_delay_compensates_borderline_fastforward_threshold()
 {
    fn local_unpaused_telemetry(position_seconds: f64) -> PlayerPlaybackTelemetryUpdate {
        PlayerPlaybackTelemetryUpdate::default()
            .with_position_seconds(position_seconds)
            .with_paused(false)
    }

    fn runtime_fixture() -> ClientRuntime<RecordingPlayer, QueuedRuntimeControl> {
        let session = desync_session_with_remote_state(5.0, false, false, "bob");
        let player = RecordingPlayer {
            pending_playback_telemetry_update: Some(local_unpaused_telemetry(0.2)),
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        ClientRuntime::new(session, player, control)
    }

    let mut baseline_runtime = runtime_fixture();
    baseline_runtime
        .run_desync_correction_if_needed(0.0, false, false, false)
        .expect("initial behind detection should not fail");
    baseline_runtime
        .player_mut_for_test()
        .pending_playback_telemetry_update = Some(local_unpaused_telemetry(0.2));
    baseline_runtime
        .run_desync_correction_if_needed(4.0, false, false, false)
        .expect("borderline fastforward check should not fail");
    assert_eq!(
        baseline_runtime.player().position,
        None,
        "without forward-delay compensation, local position should stay just above fastforward threshold"
    );

    let mut compensated_runtime = runtime_fixture();
    compensated_runtime
        .ping_metrics_legacy_compatible
        .forward_delay_seconds = 0.35;
    compensated_runtime
        .run_desync_correction_if_needed(0.0, false, false, false)
        .expect("initial behind detection with forward delay should not fail");
    compensated_runtime
        .player_mut_for_test()
        .pending_playback_telemetry_update = Some(local_unpaused_telemetry(0.2));
    compensated_runtime
        .run_desync_correction_if_needed(4.0, false, false, false)
        .expect("compensated fastforward check should not fail");
    assert_eq!(
        compensated_runtime.player().position,
        Some(5.25),
        "forward-delay compensation should push a borderline behind client over the fastforward threshold"
    );
}
