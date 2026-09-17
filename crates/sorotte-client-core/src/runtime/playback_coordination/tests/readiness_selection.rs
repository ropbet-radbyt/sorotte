use super::*;

fn apply_ready_membership(runtime: &mut ClientRuntime<DisconnectedPlayer, QueuedRuntimeControl>) {
    let source = readiness_v2_session(7, 41, 0);
    runtime.session_mut().apply_message_json(&serde_json::json!({
        "Set": { "sorotteReadinessV2": { "snapshot": source.readiness_snapshot().unwrap() } },
    }).to_string()).unwrap();
}

fn apply_initial_hello(
    runtime: &mut ClientRuntime<DisconnectedPlayer, QueuedRuntimeControl>,
    room: &str,
    with_clock: bool,
) {
    let hello = sorotte_protocol::decode_message_line(&format!(
        r#"{{"Hello":{{"username":"alice","room":{{"name":"{room}"}},"version":"1.7.5","features":{{"readiness":true,"sorotteReadinessV2":true,"sorottePlaybackBarrierV1":true}}}}}}"#,
    )).unwrap();
    if with_clock {
        runtime
            .session_mut()
            .apply_protocol_message_at(hello, 0.2)
            .unwrap();
    } else {
        runtime.session_mut().apply_protocol_message(hello).unwrap();
    }
    apply_ready_membership(runtime);
}

#[test]
fn prehello_media_binds_only_when_hello_assigns_the_initial_room() {
    for (with_clock, initialize_before_prepare) in
        [(false, false), (true, false), (false, true), (true, true)]
    {
        let mut runtime = ClientRuntime::new(
            ClientSession::default(),
            DisconnectedPlayer,
            QueuedRuntimeControl::default(),
        );
        runtime.begin_protocol_connection_generation();
        if initialize_before_prepare {
            runtime
                .session_mut()
                .initialize_local_identity("alice".into(), "requested-room".into());
        }
        runtime.prepare_playback_media(
            LogicalMediaId::new("startup.mkv").unwrap(),
            MediaTransportKind::LocalFile,
            0.0,
        );
        if !initialize_before_prepare {
            runtime
                .session_mut()
                .initialize_local_identity("alice".into(), "requested-room".into());
        }
        assert!(
            runtime
                .playback_coordination
                .prepared_playlist_selection
                .as_ref()
                .unwrap()
                .room
                .is_none(),
            "provisional connection settings must not own the prepared selection"
        );

        apply_initial_hello(&mut runtime, "assigned-room", with_clock);
        runtime.playback_coordination.observe_transport(
            paused_transport(1, 0.3, PlayerTransportPhase::ReadyPaused, 0.0),
            0.3,
        );
        assert_eq!(
            runtime
                .playback_coordination
                .prepared_playlist_selection
                .as_ref()
                .unwrap()
                .room
                .as_deref(),
            Some("assigned-room")
        );
        assert_eq!(
            runtime
                .playback_coordination
                .next_technical_readiness_report(&runtime.session)
                .unwrap()
                .phase,
            TechnicalPlayabilityPhase::Playable
        );
        assert!(
            runtime
                .playback_coordination
                .next_player_command_failure_readiness_report(&runtime.session, 0.4)
                .is_some()
        );
    }
}

#[test]
fn initial_hello_does_not_adopt_a_successor_playlist_for_preloaded_media() {
    let mut runtime = ClientRuntime::new(
        ClientSession::default(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    runtime.prepare_playback_media(
        LogicalMediaId::new("a.mkv").unwrap(),
        MediaTransportKind::LocalFile,
        0.0,
    );
    apply_initial_hello(&mut runtime, "room1", true);
    runtime.session_mut().apply_message_json(r#"{"Set":{"playlistChange":{"files":["b.mkv"],"user":"bob"},"playlistIndex":{"index":0,"user":"bob"}}}"#).unwrap();
    runtime.playback_coordination.observe_transport(
        paused_transport(1, 0.3, PlayerTransportPhase::ReadyPaused, 0.0),
        0.3,
    );
    assert_eq!(
        runtime
            .playback_coordination
            .next_technical_readiness_report(&runtime.session),
        None
    );
    assert_eq!(
        runtime
            .playback_coordination
            .next_player_command_failure_readiness_report(&runtime.session, 0.4),
        None
    );

    runtime.prepare_playback_media_for_room_participation(
        LogicalMediaId::new("b.mkv").unwrap(),
        MediaTransportKind::LocalFile,
        0.5,
    );
    runtime.playback_coordination.observe_transport(
        paused_transport(2, 0.6, PlayerTransportPhase::ReadyPaused, 0.0),
        0.6,
    );
    assert!(
        runtime
            .playback_coordination
            .next_technical_readiness_report(&runtime.session)
            .is_some()
    );
}

#[test]
fn a_later_hello_cannot_transfer_prepared_media_to_another_room() {
    let mut runtime = selection_runtime(true);
    apply_initial_hello(&mut runtime, "room2", true);
    runtime.session_mut().apply_message_json(r#"{"Set":{"playlistChange":{"files":["a.mkv","b.mkv"],"user":"bob"},"playlistIndex":{"index":0,"user":"bob"}}}"#).unwrap();
    assert_eq!(
        runtime
            .playback_coordination
            .prepared_playlist_selection
            .as_ref()
            .unwrap()
            .room
            .as_deref(),
        Some("room1")
    );
    assert_eq!(
        runtime
            .playback_coordination
            .next_technical_readiness_report(&runtime.session),
        None
    );
    assert_eq!(
        runtime
            .playback_coordination
            .next_player_command_failure_readiness_report(&runtime.session, 0.4),
        None
    );

    runtime.prepare_playback_media_for_room_participation(
        LogicalMediaId::new("a.mkv").unwrap(),
        MediaTransportKind::LocalFile,
        0.5,
    );
    runtime.playback_coordination.observe_transport(
        paused_transport(2, 0.6, PlayerTransportPhase::ReadyPaused, 0.0),
        0.6,
    );
    assert!(
        runtime
            .playback_coordination
            .next_technical_readiness_report(&runtime.session)
            .is_some()
    );
}

fn selection_runtime(
    with_playlist: bool,
) -> ClientRuntime<DisconnectedPlayer, QueuedRuntimeControl> {
    let mut session = readiness_v2_session(1, 41, 0);
    if with_playlist {
        session.apply_message_json(r#"{"Set":{"playlistChange":{"files":["a.mkv","b.mkv"],"user":"bob"},"playlistIndex":{"index":0,"user":"bob"}}}"#).unwrap();
    }
    let mut runtime =
        ClientRuntime::new(session, DisconnectedPlayer, QueuedRuntimeControl::default());
    runtime.prepare_playback_media(
        LogicalMediaId::new("a.mkv").unwrap(),
        MediaTransportKind::LocalFile,
        0.0,
    );
    runtime
        .playback_coordination
        .observe_transport(transport(1, 0.1, PlayerTransportPhase::Playing, 1.0), 0.1);
    runtime
}

#[test]
fn direct_playback_without_a_selected_playlist_keeps_technical_reports() {
    let mut runtime = selection_runtime(false);
    let report = runtime
        .playback_coordination
        .next_technical_readiness_report(&runtime.session)
        .unwrap();
    assert_eq!(report.phase, TechnicalPlayabilityPhase::Playable);
    let failure = runtime
        .playback_coordination
        .next_player_command_failure_readiness_report(&runtime.session, 0.2)
        .unwrap();
    assert_eq!(failure.reason, Some(TechnicalBlockCause::PlayerFailure));
}

#[test]
fn predecessor_command_failure_does_not_block_the_selected_successor() {
    let mut runtime = selection_runtime(true);
    runtime
        .session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"bob"}}}"#)
        .unwrap();
    assert!(runtime.session.has_pending_playlist_index_reset_intent());
    assert_eq!(
        runtime
            .playback_coordination
            .next_player_command_failure_readiness_report(&runtime.session, 0.2),
        None
    );

    runtime.prepare_playback_media_for_room_participation(
        LogicalMediaId::new("b.mkv").unwrap(),
        MediaTransportKind::LocalFile,
        0.3,
    );
    assert!(
        runtime
            .playback_coordination
            .next_player_command_failure_readiness_report(&runtime.session, 0.4)
            .is_some(),
        "a command failure belonging to the prepared successor must remain reportable"
    );
}

#[test]
fn republishing_the_predecessor_file_does_not_rebind_its_carried_observations() {
    let mut runtime = selection_runtime(true);
    runtime
        .session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"bob"}}}"#)
        .unwrap();
    let refreshed = runtime
        .prepare_playback_media_for_current_file_publication(
            LogicalMediaId::new("a.mkv").unwrap(),
            MediaTransportKind::LocalFile,
            0.2,
        )
        .unwrap();
    assert!(!refreshed.logical_media_changed);
    assert!(!refreshed.playback_episode_changed);
    assert!(
        runtime.playback_coordination.latest_observation.is_some(),
        "the publication retained physical evidence, so its selection binding must also survive"
    );
    assert_eq!(
        runtime
            .playback_coordination
            .next_technical_readiness_report(&runtime.session),
        None
    );
}

#[test]
fn active_target_preserving_reorder_keeps_technical_reports() {
    let mut runtime = selection_runtime(true);
    let prior_revision = runtime.session.current_room_playlist_selection_revision();
    runtime.session.apply_message_json(r#"{"Set":{"playlistChange":{"files":["b.mkv","a.mkv"],"user":"bob"},"playlistIndex":{"index":1,"user":"bob"}}}"#).unwrap();
    assert_ne!(
        runtime.session.current_room_playlist_selection_revision(),
        prior_revision
    );
    assert!(!runtime.session.has_pending_playlist_index_reset_intent());
    assert_eq!(
        runtime
            .playback_coordination
            .next_technical_readiness_report(&runtime.session)
            .unwrap()
            .phase,
        TechnicalPlayabilityPhase::Playable
    );
}

#[test]
fn same_row_replay_allows_reports_after_the_physical_reset_completes() {
    let mut runtime = selection_runtime(true);
    runtime
        .session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"bob"}}}"#)
        .unwrap();
    assert!(runtime.session.has_pending_playlist_index_reset_intent());
    assert_eq!(
        runtime
            .playback_coordination
            .next_technical_readiness_report(&runtime.session),
        None
    );
    assert!(
        runtime
            .session
            .mark_pending_playlist_index_reset_physical_effect_applied(1)
    );
    runtime
        .session
        .apply_protocol_message_at(
            sorotte_protocol::decode_message_line(
                r#"{"State":{"playstate":{"position":0,"paused":true,"doSeek":true}}}"#,
            )
            .unwrap(),
            0.2,
        )
        .unwrap();
    assert!(
        runtime
            .session
            .complete_pending_playlist_index_reset_for_attachment(1)
            .is_some()
    );
    runtime.playback_coordination.observe_transport(
        paused_transport(1, 0.3, PlayerTransportPhase::ReadyPaused, 0.0),
        0.3,
    );
    assert_eq!(
        runtime
            .playback_coordination
            .next_technical_readiness_report(&runtime.session)
            .unwrap()
            .phase,
        TechnicalPlayabilityPhase::Playable
    );
}

#[test]
fn predecessor_buffering_does_not_pause_a_policy_only_successor() {
    let mut runtime = selection_runtime(true);
    runtime
        .session
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"bob"}}}"#)
        .unwrap();
    apply_barrier_extension(
        &mut runtime.session,
        PlaybackBarrierSetExtension::new().with_buffering_policy(RoomBufferingPolicyPayload::new(
            2,
            RoomBufferingPolicy::PauseAnyEligible,
        )),
    );
    let mut stale = transport(1, 0.2, PlayerTransportPhase::Rebuffering, 1.0);
    stale.paused_for_cache = Some(true);
    runtime.playback_coordination.observe_transport(stale, 0.2);
    assert!(
        runtime
            .playback_coordination
            .room_buffering_observation(&runtime.session)
            .is_none()
    );

    runtime.prepare_playback_media_for_room_participation(
        LogicalMediaId::new("b.mkv").unwrap(),
        MediaTransportKind::LocalFile,
        0.3,
    );
    let mut current = transport(2, 0.4, PlayerTransportPhase::Rebuffering, 0.0);
    current.paused_for_cache = Some(true);
    runtime
        .playback_coordination
        .observe_transport(current, 0.4);
    let report = runtime
        .playback_coordination
        .room_buffering_observation(&runtime.session)
        .unwrap();
    assert_eq!(report.media_generation, 2);
    assert!(report.buffering);
}
