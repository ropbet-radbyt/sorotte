use super::*;
use sorotte_client_core::{ClientRuntime, ClientSession, QueuedRuntimeControl};
use sorotte_player_api::DisconnectedPlayer;
use sorotte_protocol::{HelloPayload, SOROTTE_READINESS_RECONNECT_TOKEN, UserReadinessIntent};

type TestClient = ClientRuntime<DisconnectedPlayer, QueuedRuntimeControl>;

fn hello_for_current_session(client: Option<&TestClient>, room: &str) -> String {
    let mut hello = HelloPayload::new("alice", room, "1.7.5").with_features(
        serde_json::from_value(json!({
            "sorottePlaybackBarrierV1": true,
            "sorotteReadinessV2": true,
        }))
        .unwrap(),
    );
    if let Some(token) =
        client.and_then(|client| client.session().readiness_reconnect_token_for_room(room))
    {
        hello.extra.insert(
            SOROTTE_READINESS_RECONNECT_TOKEN.to_owned(),
            Value::String(token.to_owned()),
        );
    }
    sorotte_protocol::encode_message_line(&ProtocolMessage::hello(hello)).unwrap()
}

fn deliver(client: &mut TestClient, client_id: &str, lines: &[DirectedOutboundLine], now: f64) {
    for (recipient, message) in decode_directed_lines(lines) {
        if recipient != client_id {
            continue;
        }
        match message {
            ProtocolMessage::State(state) => {
                client.run_state_sync_reconcile_with_inbound_state_with_ping_at_clocks(
                    state.state,
                    false,
                    now,
                    now,
                    now,
                );
            }
            other => client
                .session_mut()
                .apply_protocol_message_at(other, now)
                .unwrap(),
        }
    }
}

fn flush(server: &mut ServerRuntime, client: &mut TestClient, client_id: &str, now: f64) {
    let mut delivered = 0;
    while let Some(pending) = client.pending_protocol_line().unwrap() {
        delivered += 1;
        assert!(delivered < 100, "bounded in-process protocol exchange");
        let line = pending.line().to_owned();
        client.acknowledge_protocol_line(pending.lease()).unwrap();
        let replies = server.handle_line_fanout(client_id, &line).unwrap();
        deliver(client, client_id, &replies, now);
    }
}

fn ready_member(switch_rooms: bool) -> (ServerRuntime, TestClient, u64) {
    let mut server = ServerRuntime::default();
    server.set_clock_overrides_seconds(Some(100.0), Some(0.0));
    let initial_room = if switch_rooms { "room-a" } else { "room-b" };
    let hello = server
        .handle_line_fanout("alice-old", &hello_for_current_session(None, initial_room))
        .unwrap();
    let mut client = ClientRuntime::new(
        ClientSession::default(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    deliver(&mut client, "alice-old", &hello, 100.0);
    flush(&mut server, &mut client, "alice-old", 100.0);
    assert!(
        client
            .session()
            .readiness_reconnect_token_for_room(initial_room)
            .is_some()
    );
    if switch_rooms {
        let previous_token = client
            .session()
            .readiness_reconnect_token_for_room(initial_room)
            .unwrap()
            .to_owned();
        let previous_epoch = server.room_readiness[initial_room].participants["alice"]
            .record
            .membership_epoch;
        assert!(client.run_set_room("room-b").unwrap());
        flush(&mut server, &mut client, "alice-old", 100.0);
        assert_eq!(client.session().room(), Some("room-b"));
        assert!(
            client
                .session()
                .readiness_reconnect_token_for_room(initial_room)
                .is_none()
        );
        assert_ne!(
            client
                .session()
                .readiness_reconnect_token_for_room("room-b"),
            Some(previous_token.as_str())
        );
        let new_membership = &server.room_readiness["room-b"].participants["alice"].record;
        assert_ne!(new_membership.membership_epoch, previous_epoch);
        assert_eq!(new_membership.user_intent, UserReadinessIntent::NotReady);
    }
    assert!(client.run_toggle_ready(true).unwrap());
    flush(&mut server, &mut client, "alice-old", 100.0);
    assert!(client.session().pending_readiness_intent().is_none());
    let member = &server.room_readiness["room-b"].participants["alice"].record;
    assert_eq!(member.user_intent, UserReadinessIntent::Ready);
    assert!(member.room_ready);
    let epoch = member.membership_epoch;
    (server, client, epoch)
}

fn reconnect(
    server: &mut ServerRuntime,
    client: &mut TestClient,
    previous_client_id: &str,
    next_client_id: &str,
    elapsed: f64,
) {
    assert!(
        client
            .session()
            .readiness_reconnect_token_for_room("room-b")
            .is_some()
    );
    server
        .handle_transport_disconnect_fanout(previous_client_id)
        .unwrap();
    client.begin_protocol_connection_generation();
    client.session_mut().mark_reconnecting(1);
    client.session_mut().reset_sync_state_for_reconnect();
    server.set_clock_overrides_seconds(Some(100.0 + elapsed), Some(elapsed));
    let hello = hello_for_current_session(Some(client), "room-b");
    let reply = server.handle_line_fanout(next_client_id, &hello).unwrap();
    deliver(client, next_client_id, &reply, 100.0 + elapsed);
    flush(server, client, next_client_id, 100.0 + elapsed);
}

#[test]
fn room_switch_ready_survives_same_room_reconnect() {
    let (mut server, mut client, epoch) = ready_member(true);
    reconnect(&mut server, &mut client, "alice-old", "alice-new", 1.0);
    let record = &server.room_readiness["room-b"].participants["alice"].record;
    assert_eq!(
        record.user_intent,
        UserReadinessIntent::Ready,
        "Ready was acknowledged in room-b before disconnect; a same-room reconnect must preserve that intent after an ordinary room switch"
    );
    assert_eq!(record.membership_epoch, epoch);
}

#[test]
fn direct_room_join_ready_survives_reconnect_control() {
    let (mut server, mut client, epoch) = ready_member(false);
    reconnect(&mut server, &mut client, "alice-old", "alice-new", 1.0);
    let record = &server.room_readiness["room-b"].participants["alice"].record;
    assert_eq!(record.user_intent, UserReadinessIntent::Ready);
    assert_eq!(record.membership_epoch, epoch);
}

#[test]
fn room_switch_and_repeated_reconnect_preserve_subsequent_intent() {
    let (mut server, mut client, epoch) = ready_member(true);
    reconnect(&mut server, &mut client, "alice-old", "alice-new", 1.0);
    assert_eq!(
        server.room_readiness["room-b"].participants["alice"]
            .record
            .user_intent,
        UserReadinessIntent::Ready
    );
    // A later deliberate Not Ready must survive another reconnect as well.
    assert!(client.run_toggle_ready(true).unwrap());
    flush(&mut server, &mut client, "alice-new", 101.0);
    assert_eq!(
        server.room_readiness["room-b"].participants["alice"]
            .record
            .user_intent,
        UserReadinessIntent::NotReady,
    );
    reconnect(&mut server, &mut client, "alice-new", "alice-newer", 2.0);
    let record = &server.room_readiness["room-b"].participants["alice"].record;
    assert_eq!(record.user_intent, UserReadinessIntent::NotReady);
    assert_eq!(record.membership_epoch, epoch);
}

fn assert_join_playlist(author_moves: bool) {
    let mut server = ServerRuntime::default();
    server.set_clock_overrides_seconds(Some(100.0), Some(0.0));
    let bob_hello = server.handle_line_fanout(
        "bob-client",
        r#"{"Hello":{"username":"bob","room":{"name":"room-c"},"version":"1.7.5","features":{"sorottePlaybackBarrierV1":true,"sorotteReadinessV2":true}}}"#,
    ).unwrap();
    let mut bob = ClientRuntime::new(
        ClientSession::default(),
        DisconnectedPlayer,
        QueuedRuntimeControl::default(),
    );
    deliver(&mut bob, "bob-client", &bob_hello, 100.0);
    flush(&mut server, &mut bob, "bob-client", 100.0);
    for (client_id, username) in [("alice-client", "alice"), ("keeper-client", "keeper")] {
        let joined = server.handle_line_fanout(
            client_id,
            &format!(r#"{{"Hello":{{"username":"{username}","room":{{"name":"room-a"}},"version":"1.7.5","features":{{"sorottePlaybackBarrierV1":true,"sorotteReadinessV2":true}}}}}}"#),
        ).unwrap();
        deliver(&mut bob, "bob-client", &joined, 100.0);
    }
    server
        .handle_line_fanout(
            "alice-client",
            r#"{"Set":{"playlistChange":{"files":["episode-1.mkv","episode-2.mkv"]}}}"#,
        )
        .unwrap();
    server
        .handle_line_fanout("alice-client", r#"{"Set":{"playlistIndex":{"index":0}}}"#)
        .unwrap();
    assert_eq!(
        server.room_playback_state("room-a").set_by.as_deref(),
        Some("alice")
    );
    if author_moves {
        let moved = server
            .handle_line_fanout("alice-client", r#"{"Set":{"room":{"name":"room-b"}}}"#)
            .unwrap();
        deliver(&mut bob, "bob-client", &moved, 100.0);
        assert_eq!(bob.session().user_room("alice"), Some("room-b"));
    }
    // The remaining room member has no player loaded. Several periodic ticks
    // must not be mistaken for a missing delivery or an immediate-switch race.
    for second in 1..=5 {
        server.set_clock_overrides_seconds(Some(100.0 + second as f64), Some(second as f64));
        let tick = server.collect_dispatch_at(100.0 + second as f64).unwrap();
        deliver(
            &mut bob,
            "bob-client",
            &tick.outbound_lines,
            100.0 + second as f64,
        );
        flush(&mut server, &mut bob, "bob-client", 100.0 + second as f64);
    }
    assert!(bob.run_set_room("room-a").unwrap());
    flush(&mut server, &mut bob, "bob-client", 105.0);
    assert_eq!(bob.session().room(), Some("room-a"));
    assert_eq!(
        server.room_playlist_state("room-a").files,
        ["episode-1.mkv", "episode-2.mkv"]
    );
    for second in 6..=10 {
        let now = 100.0 + second as f64;
        server.set_clock_overrides_seconds(Some(now), Some(second as f64));
        let tick = server.collect_dispatch_at(now).unwrap();
        deliver(&mut bob, "bob-client", &tick.outbound_lines, now);
        flush(&mut server, &mut bob, "bob-client", now);
    }
    assert!(bob.run_request_user_list().unwrap());
    flush(&mut server, &mut bob, "bob-client", 110.0);
    assert_eq!(
        bob.session()
            .current_room_playlist()
            .map(|playlist| playlist.files.as_slice()),
        Some(["episode-1.mkv".to_owned(), "episode-2.mkv".to_owned()].as_slice()),
        "joining room-a must install room-a's canonical playlist even after its previous author moved to room-b"
    );
    assert_eq!(
        bob.session().current_room_playlist().unwrap().index,
        Some(0)
    );
    assert!(bob.run_set_playlist_index(1).unwrap());
    flush(&mut server, &mut bob, "bob-client", 110.0);
    assert_eq!(
        bob.session().current_room_playlist().unwrap().index,
        Some(1)
    );
    assert_eq!(server.room_playlist_state("room-a").index, Some(1));
}

#[test]
fn new_room_membership_identity_is_sent_only_to_the_joining_client() {
    let mut server = ServerRuntime::default();
    server
        .handle_line_fanout("alice", &hello_for_current_session(None, "room-a"))
        .unwrap();
    server.handle_line_fanout("bob", r#"{"Hello":{"username":"bob","room":{"name":"room-b"},"version":"1.7.5","features":{"sorotteReadinessV2":true,"sorottePlaybackBarrierV1":true}}}"#).unwrap();
    let moved = server
        .handle_line_fanout("alice", r#"{"Set":{"room":{"name":"room-b"}}}"#)
        .unwrap();
    let identities: Vec<_> = decode_directed_lines(&moved)
        .into_iter()
        .filter_map(|(recipient, message)| {
            let ProtocolMessage::Set(set) = message else {
                return None;
            };
            let extension = set.set.readiness_v2().unwrap()?;
            extension
                .membership
                .map(|membership| (recipient, membership))
        })
        .collect();
    assert_eq!(identities.len(), 1);
    assert_eq!(identities[0].0, "alice");
    assert_eq!(identities[0].1.room, "room-b");
    assert_eq!(
        identities[0].1.membership_epoch,
        server.room_readiness["room-b"].participants["alice"]
            .record
            .membership_epoch
    );
}

#[test]
fn join_installs_playlist_after_its_author_changes_rooms() {
    assert_join_playlist(true);
}

#[test]
fn join_installs_playlist_when_author_stays_control() {
    assert_join_playlist(false);
}
