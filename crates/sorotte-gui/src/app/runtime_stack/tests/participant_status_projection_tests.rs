use super::*;

use crate::app::support::system_time_seconds;
use sorotte_client_core::ExternalPlayerAvailability;
use sorotte_protocol::{ParticipantPlaybackPhase, ParticipantPlayerConnection};

#[test]
fn gui_adapter_forwards_external_player_availability_transitions() {
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core GUI adapter should bootstrap");
    let _ = adapter
        .flush_outbound_protocol_lines()
        .expect("startup Hello should flush");
    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sorotteParticipantStatusV1":true}}}"#,
        )
        .expect("participant-status-capable server Hello should apply");
    let _ = adapter
        .flush_outbound_protocol_lines()
        .expect("Hello transition should flush");
    let now_seconds = system_time_seconds();

    assert!(
        GuiSessionRuntimeAdapter::set_external_player_availability(
            &mut adapter,
            ExternalPlayerAvailability::Connecting,
            now_seconds,
        )
        .expect("attached player should become connecting")
    );
    let connecting = adapter
        .flush_outbound_protocol_lines()
        .expect("connecting report should flush");
    assert!(
        connecting
            .iter()
            .any(|line| line.contains(r#""playerConnection":"starting""#)),
        "GUI availability transition should reach the participant-status report: {connecting:?}"
    );

    assert!(
        GuiSessionRuntimeAdapter::set_external_player_availability(
            &mut adapter,
            ExternalPlayerAvailability::Unavailable,
            now_seconds + 1.0,
        )
        .expect("detached player should become unavailable")
    );
    let unavailable = adapter
        .flush_outbound_protocol_lines()
        .expect("unavailable report should flush");
    assert!(
        unavailable
            .iter()
            .any(|line| line.contains(r#""playerConnection":"unavailable""#)),
        "GUI detach transition should reach the participant-status report: {unavailable:?}"
    );
}

#[test]
fn gui_runtime_projects_negotiated_participant_status_and_authoritative_room_intent() {
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core GUI adapter should bootstrap");
    sync_adapter_to_saved_session_settings(&mut adapter, &state);

    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup Hello should encode");
    assert_eq!(startup_lines.len(), 1);
    assert!(
        startup_lines[0].contains(r#""sorotteParticipantStatusV1":true"#),
        "GUI Hello must negotiate room participant status: {}",
        startup_lines[0]
    );

    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sorotteParticipantStatusV1":true}}}"#,
        )
        .expect("participant-status-capable server Hello should apply");
    adapter
        .apply_message_json(
            r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"features":{"sorotteParticipantStatusV1":true}},"bob":{"room":{"name":"room1"},"features":{"sorotteParticipantStatusV1":true}},"carol":{"room":{"name":"room1"},"features":{"sorotteParticipantStatusV1":false}},"dave":{"room":{"name":"room1"},"features":{"sorotteParticipantStatusV1":true}},"erin":{"room":{"name":"room1"},"features":{"sorotteParticipantStatusV1":true}}}}}"#,
        )
        .expect("same-room peer capability rows should apply");
    adapter
        .apply_message_json(
            r#"{"State":{"playstate":{"position":755.03,"paused":true,"doSeek":false,"setBy":"server"},"sorotteParticipantStatusV1":{"scope":{"mediaGeneration":7,"stateRevision":19,"transportRevision":3},"snapshot":{"revision":1,"participants":{"alice":{"availability":"fresh","correlation":"exact","playbackScope":{"mediaGeneration":7,"stateRevision":19,"transportRevision":3},"playerConnection":"connected","phase":"playing","timelineKind":"vod","positionSeconds":754.82,"logicalPaused":false,"playbackRate":1.0,"pausedForCache":false,"bufferedAheadSeconds":14.3,"reportAgeMs":180,"sampleAgeMs":0,"positionSampleAgeMs":0,"roomOffsetSeconds":-0.21},"bob":{"availability":"fresh","correlation":"superseded","playbackScope":{"mediaGeneration":8,"stateRevision":20,"transportRevision":4},"playerConnection":"connected","phase":"rebuffering","timelineKind":"vod","positionSeconds":751.2,"logicalPaused":false,"playbackRate":1.0,"pausedForCache":true,"bufferedAheadSeconds":0.4,"cachePercent":20.0,"reportAgeMs":340,"sampleAgeMs":0,"positionSampleAgeMs":0,"roomOffsetSeconds":-3.83},"carol":{"availability":"unsupported"},"dave":{"availability":"awaitingReport"},"erin":{"availability":"fresh","correlation":"exact","playbackScope":{"mediaGeneration":7,"stateRevision":19,"transportRevision":3},"playerConnection":"disconnected","phase":"rebuffering","timelineKind":"vod","pausedForCache":true,"reportAgeMs":100,"sampleAgeMs":0}}}}}}"#,
        )
        .expect("room playstate and complete member snapshot should apply");

    let snapshot = adapter
        .main_window_runtime_snapshot(&state)
        .expect("new room/member state should project into a GUI snapshot");
    assert_eq!(snapshot.room_playback_intent.paused, Some(true));
    assert_eq!(snapshot.room_playback_intent.position_seconds, Some(755.03));
    assert_eq!(
        snapshot.room_playback_intent.set_by.as_deref(),
        Some("server")
    );
    assert!(
        snapshot
            .room_playback_intent
            .authority
            .as_deref()
            .is_some_and(|authority| authority.contains("legacy playstate"))
    );
    assert_eq!(snapshot.room_playback_intent.participant_count, 5);
    assert_eq!(
        snapshot.room_playback_intent.maximum_observed_drift_seconds,
        Some(0.21),
        "only fresh status on the current playback scope contributes to room drift"
    );
    assert!(
        snapshot
            .room_playback_intent
            .buffering_participants
            .is_empty(),
        "mismatched reports must not claim that the current room media is buffering"
    );

    let user = |username: &str| {
        snapshot
            .users
            .iter()
            .find(|user| user.username == username)
            .unwrap_or_else(|| panic!("missing projected user {username}"))
    };
    let MainWindowParticipantStatusPresentation::Report(alice) = &user("alice").participant_status
    else {
        panic!("capable local report should project");
    };
    assert_eq!(
        alice.status.player_connection,
        Some(ParticipantPlayerConnection::Connected)
    );
    assert_eq!(alice.status.phase, Some(ParticipantPlaybackPhase::Playing));
    assert_eq!(alice.freshness, MainWindowParticipantStatusFreshness::Fresh);
    assert!(!alice.timeline_mismatch);
    assert_eq!(alice.position_label(), "12:34.8");
    assert_eq!(alice.offset_label(), "0.2 s behind");

    let MainWindowParticipantStatusPresentation::Report(bob) = &user("bob").participant_status
    else {
        panic!("capable peer report should project");
    };
    assert_eq!(
        bob.status.phase,
        Some(ParticipantPlaybackPhase::Rebuffering)
    );
    assert!(bob.timeline_mismatch);
    assert_eq!(bob.position_label(), "Position unavailable");
    assert_eq!(bob.offset_label(), "Different playback scope");
    assert_eq!(bob.buffer_label(), "Buffer status unavailable");

    let MainWindowParticipantStatusPresentation::Report(erin) = &user("erin").participant_status
    else {
        panic!("disconnected capable peer report should project");
    };
    assert_eq!(
        erin.status.player_connection,
        Some(ParticipantPlayerConnection::Disconnected)
    );
    assert_eq!(
        erin.status.phase,
        Some(ParticipantPlaybackPhase::Rebuffering)
    );
    assert!(!erin.timeline_mismatch);

    assert!(matches!(
        user("carol").participant_status,
        MainWindowParticipantStatusPresentation::LegacyClient
    ));
    assert!(matches!(
        user("dave").participant_status,
        MainWindowParticipantStatusPresentation::WaitingForFirstReport
    ));
}

#[test]
fn compact_and_stale_exact_statuses_do_not_invent_scope_mismatches() {
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core GUI adapter should bootstrap");
    sync_adapter_to_saved_session_settings(&mut adapter, &state);
    let _ = adapter
        .flush_outbound_protocol_lines()
        .expect("startup Hello should flush");
    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sorotteParticipantStatusV1":true}}}"#,
        )
        .unwrap();
    adapter
        .apply_message_json(
            r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"features":{"sorotteParticipantStatusV1":true}},"carol":{"room":{"name":"room1"},"features":{"sorotteParticipantStatusV1":true}}}}}"#,
        )
        .unwrap();
    adapter
        .apply_message_json(
            r#"{"State":{"sorotteParticipantStatusV1":{"scope":{"mediaGeneration":7,"stateRevision":19,"transportRevision":3},"snapshot":{"revision":1,"mode":"compact","participants":{"bob":{"availability":"fresh","correlation":"exact","playerConnection":"connected","phase":"playing","reportAgeMs":100,"sampleAgeMs":0},"carol":{"availability":"stale","correlation":"exact","playerConnection":"connected","phase":"playing","reportAgeMs":12000}}}}}}"#,
        )
        .unwrap();

    let snapshot = adapter
        .main_window_runtime_snapshot(&state)
        .expect("compact status rows should project");
    let user = |username: &str| {
        snapshot
            .users
            .iter()
            .find(|user| user.username == username)
            .unwrap_or_else(|| panic!("missing projected user {username}"))
    };
    let MainWindowParticipantStatusPresentation::Report(bob) = &user("bob").participant_status
    else {
        panic!("compact exact report should project");
    };
    assert!(!bob.timeline_mismatch);
    assert_eq!(bob.status.phase, Some(ParticipantPlaybackPhase::Playing));
    assert_eq!(bob.position_label(), "Position unavailable");
    assert_eq!(bob.offset_label(), "Offset unavailable");

    let MainWindowParticipantStatusPresentation::Report(carol) = &user("carol").participant_status
    else {
        panic!("stale exact report should project");
    };
    assert!(!carol.timeline_mismatch);
    assert_eq!(carol.freshness, MainWindowParticipantStatusFreshness::Stale);
    assert_eq!(carol.offset_label(), "Offset unavailable");
    assert!(carol.headline_label().starts_with("Status stale"));
}

#[test]
fn legacy_uncorrelated_wire_rows_never_project_precise_room_offsets() {
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core GUI adapter should bootstrap");
    sync_adapter_to_saved_session_settings(&mut adapter, &state);
    let _ = adapter
        .flush_outbound_protocol_lines()
        .expect("startup Hello should flush");
    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sorotteParticipantStatusV1":true}}}"#,
        )
        .unwrap();
    adapter
        .apply_message_json(
            r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"features":{"sorotteParticipantStatusV1":true}}}}}"#,
        )
        .unwrap();
    adapter
        .apply_message_json(
            r#"{"State":{"sorotteParticipantStatusV1":{"snapshot":{"revision":1,"participants":{"bob":{"availability":"fresh","correlation":"uncorrelated","playerConnection":"connected","phase":"playing","positionSeconds":42.5,"reportAgeMs":0,"sampleAgeMs":0,"positionSampleAgeMs":0,"roomOffsetSeconds":-3.83}}}}}}"#,
        )
        .unwrap();

    let snapshot = adapter
        .main_window_runtime_snapshot(&state)
        .expect("legacy-compatible status should project");
    let bob = snapshot
        .users
        .iter()
        .find(|user| user.username == "bob")
        .expect("bob should project");
    let MainWindowParticipantStatusPresentation::Report(report) = &bob.participant_status else {
        panic!("bob should retain a coarse legacy report");
    };
    assert_eq!(report.position_label(), "00:42.5");
    assert_eq!(report.offset_label(), "Offset unavailable");
    assert_eq!(
        snapshot.room_playback_intent.maximum_observed_drift_seconds, None,
        "uncorrelated offsets must not influence the room summary"
    );
}
