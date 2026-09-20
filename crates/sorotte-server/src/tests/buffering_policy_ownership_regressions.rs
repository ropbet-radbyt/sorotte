use super::*;
use sorotte_protocol::{
    PlayerInteractionSurface, PlayerReadinessAction, ReadinessIntentRequest, ReadinessSetExtension,
    RoomPauseOwner, UserReadinessIntent, UserReadinessMutationSource, encode_message_line,
};

fn exchange(server: &mut ServerRuntime, client: &str, line: &str) -> Vec<DirectedOutboundLine> {
    let replies = server.handle_line_fanout(client, line).unwrap();
    acknowledge_directed_state_counters(server, &decode_directed_lines(&replies));
    replies
}

fn transport(server: &mut ServerRuntime, position: f64, paused: bool) {
    exchange(
        server,
        "alice",
        &json!({"State": {"playstate": {
            "position": position, "paused": paused, "doSeek": true
        }}})
        .to_string(),
    );
}

fn player_intent(server: &mut ServerRuntime, nonce: u64, paused: bool) {
    let room = server.sessions["alice"].room.clone();
    let epoch = server.room_readiness[&room].participants["alice"]
        .record
        .membership_epoch;
    let request = ReadinessIntentRequest::new(
        format!("player-intent-{nonce}"),
        nonce,
        epoch,
        if paused {
            UserReadinessIntent::NotReady
        } else {
            UserReadinessIntent::Ready
        },
        UserReadinessMutationSource::IndirectPlayer {
            action: if paused {
                PlayerReadinessAction::Pause
            } else {
                PlayerReadinessAction::Play
            },
            surface: PlayerInteractionSurface::NativePlayerControl,
        },
    );
    exchange(
        server,
        "alice",
        &encode_message_line(&ProtocolMessage::set(
            SetPayload::new().with_readiness_v2(ReadinessSetExtension::new().with_intent(request)),
        ))
        .unwrap(),
    );
}

fn replace_buffering_policy(user_paused: bool) {
    let room = controlled_room_name_for_test("buffering-owner", "AB-123-456");
    let mut server = ServerRuntime::with_room_password_salt(DEFAULT_CONTROLLED_ROOM_HASH_SALT);
    server.set_clock_overrides_seconds(Some(100.0), Some(0.0));
    for client in ["alice", "bob"] {
        exchange(
            &mut server,
            client,
            &json!({"Hello": {
                "username": client, "room": {"name": room}, "version": "1.7.5",
                "features": {"sorottePlaybackBarrierV1": true, "sorotteReadinessV2": true}
            }})
            .to_string(),
        );
    }
    exchange(
        &mut server,
        "alice",
        r#"{"Set":{"controllerAuth":{"password":"AB-123-456"}}}"#,
    );
    player_intent(&mut server, 1, false);
    transport(&mut server, 40.0, false);
    assert!(!server.room_playback_state(&room).paused);
    if user_paused {
        player_intent(&mut server, 2, true);
        transport(&mut server, 40.0, true);
    }
    exchange(
        &mut server,
        "alice",
        r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{
        "mediaGeneration":0,"requestNonce":1,"requestId":"policy-1","loadIntent":"newPlayback",
        "policy":"pauseAnyEligible","debounceMs":0,"resumeHysteresisMs":0,"maxPauseMs":30000
    }}}}"#,
    );
    exchange(
        &mut server,
        "bob",
        r#"{"State":{"sorottePlaybackBarrierV1":{"transport":{
        "mediaGeneration":1,"buffering":true,"bufferedSeconds":0.0
    }}}}"#,
    );
    assert!(server.room_playback_state(&room).paused);
    exchange(
        &mut server,
        "alice",
        r#"{"Set":{"sorottePlaybackBarrierV1":{"bufferingPolicy":{
        "mediaGeneration":0,"requestNonce":2,"requestId":"policy-2","loadIntent":"transportRefresh",
        "policy":"independent","debounceMs":0,"resumeHysteresisMs":0,"maxPauseMs":30000
    }}}}"#,
    );
    let expected_owner = if user_paused {
        RoomPauseOwner::User {
            actor: "alice".to_owned(),
        }
    } else {
        RoomPauseOwner::None
    };
    assert_eq!(server.room_playback_state(&room).paused, user_paused);
    // A later member receives the canonical public snapshot after the policy
    // refresh, including ownership retained from the completed transition.
    let joined = exchange(
        &mut server,
        "charlie",
        &json!({"Hello": {
            "username": "charlie", "room": {"name": room}, "version": "1.7.5",
            "features": {"sorottePlaybackBarrierV1": true, "sorotteReadinessV2": true}
        }})
        .to_string(),
    );
    let snapshot = decode_directed_lines(&joined)
        .into_iter()
        .filter(|(recipient, _)| recipient == "bob")
        .filter_map(|(_, message)| match message {
            ProtocolMessage::Set(set) => set.set.readiness_v2().unwrap()?.snapshot,
            _ => None,
        })
        .next_back()
        .expect("joining publishes the current room readiness snapshot");
    assert_eq!(
        snapshot.pause_owner, expected_owner,
        "public pause ownership must agree with the completed policy transition"
    );
    assert_eq!(server.room_readiness[&room].pause_owner, expected_owner);

    // The next ordinary explicit gesture owns its pause, and later Play/seek
    // remains usable after replacing the policy.
    player_intent(&mut server, 3, true);
    transport(&mut server, 40.0, true);
    assert_eq!(
        server.room_readiness[&room].pause_owner,
        RoomPauseOwner::User {
            actor: "alice".to_owned()
        }
    );
    player_intent(&mut server, 4, false);
    transport(&mut server, 40.0, false);
    assert_eq!(
        server.room_readiness[&room].pause_owner,
        RoomPauseOwner::None
    );
    transport(&mut server, 75.0, false);
    assert!(!server.room_playback_state(&room).paused);
    assert_eq!(server.room_playback_state(&room).position, 75.0);
}

#[test]
fn replacing_buffering_policy_releases_its_public_pause_owner() {
    replace_buffering_policy(false);
}

#[test]
fn replacing_buffering_policy_preserves_an_explicit_user_pause() {
    replace_buffering_policy(true);
}
