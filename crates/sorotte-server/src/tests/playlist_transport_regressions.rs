use super::*;
use sorotte_client_core::{ClientRuntime, ClientSession, QueuedRuntimeControl};
use sorotte_player_api::DisconnectedPlayer;

fn exchange(runtime: &mut ServerRuntime, client: &str, line: &str) -> Vec<DirectedOutboundLine> {
    let replies = runtime.handle_line_fanout(client, line).unwrap();
    acknowledge_directed_state_counters(runtime, &decode_directed_lines(&replies));
    replies
}

fn playing_playlist(lifecycle: bool) -> ServerRuntime {
    playing_playlist_entries(lifecycle, &["a.mkv", "b.mkv", "c.mkv"], 1)
}

fn playing_playlist_entries(lifecycle: bool, files: &[&str], index: usize) -> ServerRuntime {
    let mut runtime = ServerRuntime::default();
    runtime.set_clock_overrides_seconds(Some(100.0), Some(0.0));
    for client in ["alice", "bob"] {
        exchange(
            &mut runtime,
            client,
            &json!({"Hello": {
                "username": client,
                "room": {"name": "room"},
                "version": "1.7.5",
                "features": {"sorottePlaybackBarrierV1": lifecycle}
            }})
            .to_string(),
        );
    }
    exchange(
        &mut runtime,
        "alice",
        &json!({"Set": {
            "playlistChange": {"files": files}, "playlistIndex": {"index": index}
        }})
        .to_string(),
    );
    for client in ["alice", "bob"] {
        exchange(
            &mut runtime,
            client,
            &json!({"Set": {"file": {"name": files[index], "duration": 600.0}}}).to_string(),
        );
    }
    exchange(
        &mut runtime,
        "alice",
        r#"{"State":{"playstate":{"position":120.0,"paused":false,"doSeek":true}}}"#,
    );
    assert_eq!(
        runtime.room_playlist_state("room").index,
        Some(index as i64)
    );
    assert_eq!(runtime.room_playback_state("room").position, 120.0);
    assert!(!runtime.room_playback_state("room").paused);
    runtime
}

type ProbeClient = ClientRuntime<DisconnectedPlayer, QueuedRuntimeControl>;

fn playlist_client(lifecycle: bool) -> ProbeClient {
    playlist_client_entries(lifecycle, &["a.mkv", "b.mkv", "c.mkv"], 1)
}

fn playlist_client_entries(lifecycle: bool, files: &[&str], index: usize) -> ProbeClient {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            &json!({"Hello": {
                "username": "alice", "room": {"name": "room"}, "version": "1.7.5",
                "features": {"sorottePlaybackBarrierV1": lifecycle}
            }})
            .to_string(),
        )
        .unwrap();
    for line in [
        json!({"Set": {"playlistChange": {"files": files, "user": "alice"}}}).to_string(),
        json!({"Set": {"playlistIndex": {"index": index, "user": "alice"}}}).to_string(),
    ] {
        session.apply_message_json(&line).unwrap();
    }
    ClientRuntime::new(session, DisconnectedPlayer, QueuedRuntimeControl::default())
}

fn client_playlist_edit(files: Vec<&str>, lifecycle: bool) -> Vec<String> {
    let mut client = playlist_client(lifecycle);
    assert!(
        client
            .run_replace_playlist(files.into_iter().map(str::to_owned).collect(), None)
            .unwrap()
    );
    queued_client_lines(&mut client)
}

fn queued_client_lines(client: &mut ProbeClient) -> Vec<String> {
    let mut lines = Vec::new();
    while let Some(pending) = client.pending_protocol_line().unwrap() {
        lines.push(pending.line().to_owned());
        client.acknowledge_protocol_line(pending.lease()).unwrap();
    }
    lines
}

fn edit_and_observe(runtime: &mut ServerRuntime, files: Vec<&str>) -> (f64, bool, usize) {
    let lifecycle = runtime.room_uses_playback_lifecycle_authority("room");
    edit_lines_and_observe(runtime, client_playlist_edit(files, lifecycle))
}

fn edit_lines_and_observe(runtime: &mut ServerRuntime, lines: Vec<String>) -> (f64, bool, usize) {
    let lifecycle = runtime.room_uses_playback_lifecycle_authority("room");
    let mut observer = ClientSession::default();
    let joined = exchange(
        runtime,
        "observer",
        &json!({"Hello": {
            "username": "observer", "room": {"name": "room"}, "version": "1.7.5",
            "features": {"sorottePlaybackBarrierV1": lifecycle}
        }})
        .to_string(),
    );
    for (recipient, message) in decode_directed_lines(&joined) {
        if recipient == "observer" {
            observer.apply_protocol_message_at(message, 100.0).unwrap();
        }
    }
    assert!(!observer.has_pending_playlist_index_reset_intent());
    let mut bob_transport_resets = 0;
    for line in lines {
        println!("client edit: {}", line.trim());
        let replies = exchange(runtime, "alice", &line);
        bob_transport_resets += decode_directed_lines(&replies)
            .iter()
            .filter(|(recipient, message)| {
                recipient == "bob"
                    && matches!(message, ProtocolMessage::State(state)
                        if state.state.playstate.as_ref().is_some_and(|playstate|
                            playstate.position == Some(0.0) && playstate.paused == Some(true)))
            })
            .count();
        for (recipient, message) in decode_directed_lines(&replies) {
            if recipient == "observer" {
                observer.apply_protocol_message_at(message, 100.0).unwrap();
            }
        }
    }
    let playlist = runtime.room_playlist_state("room");
    let observed_playlist = observer.current_room_playlist().unwrap();
    assert_eq!(observed_playlist.files, playlist.files);
    assert_eq!(observed_playlist.index, playlist.index);
    if bob_transport_resets == 0 {
        assert!(
            !observer.has_pending_playlist_index_reset_intent(),
            "the actual edit fanout must preserve the remote client's player too"
        );
    }
    let state = runtime.room_playback_state("room");
    println!(
        "after edit: index={:?}, position={}, paused={}, bob paused-zero frames={}",
        runtime.room_playlist_state("room").index,
        state.position,
        state.paused,
        bob_transport_resets
    );
    (state.position, state.paused, bob_transport_resets)
}

fn assert_valid_successor_selection(runtime: &mut ServerRuntime) {
    let current_index = runtime.room_playlist_state("room").index.unwrap();
    let revision = runtime.transport_authority_revision_for_room("room");
    exchange(
        runtime,
        "bob",
        &json!({"Set": {"playlistIndex": {"index": current_index}}}).to_string(),
    );
    assert_eq!(
        runtime.room_playlist_state("room").index,
        Some(current_index)
    );
    assert_eq!(runtime.room_playback_state("room").position, 0.0);
    assert!(runtime.room_playback_state("room").paused);
    assert!(runtime.transport_authority_revision_for_room("room") > revision);
    exchange(runtime, "bob", r#"{"Set":{"playlistIndex":{"index":0}}}"#);
    assert_eq!(runtime.room_playlist_state("room").index, Some(0));
    assert_eq!(runtime.room_playback_state("room").position, 0.0);
    assert!(runtime.room_playback_state("room").paused);
}

#[test]
fn reordering_the_active_row_preserves_room_transport() {
    for paused in [false, true] {
        let mut runtime = playing_playlist(true);
        if paused {
            exchange(
                &mut runtime,
                "alice",
                r#"{"State":{"playstate":{"position":120.0,"paused":true,"doSeek":true}}}"#,
            );
        }
        let observed = edit_and_observe(&mut runtime, vec!["c.mkv", "a.mkv", "b.mkv"]);
        assert_eq!(runtime.room_playlist_state("room").index, Some(2));
        assert_eq!(runtime.room_playlist_state("room").files[2], "b.mkv");
        assert_valid_successor_selection(&mut runtime);
        assert_eq!(
            observed,
            (120.0, paused, 0),
            "reordering the active row must preserve playback"
        );
    }
}

#[test]
fn deleting_before_the_active_row_preserves_room_transport() {
    let mut runtime = playing_playlist(true);
    let mut client = playlist_client(true);
    assert!(client.run_delete_playlist_index(0).unwrap());
    let observed = edit_lines_and_observe(&mut runtime, queued_client_lines(&mut client));
    assert_eq!(runtime.room_playlist_state("room").index, Some(0));
    assert_eq!(runtime.room_playlist_state("room").files[0], "b.mkv");
    assert_valid_successor_selection(&mut runtime);
    assert_eq!(
        observed,
        (120.0, false, 0),
        "removing a different row must preserve playback"
    );
}

#[test]
fn deleting_after_the_active_row_preserves_room_transport() {
    let mut runtime = playing_playlist(true);
    let mut client = playlist_client(true);
    assert!(client.run_delete_playlist_index(2).unwrap());
    let observed = edit_lines_and_observe(&mut runtime, queued_client_lines(&mut client));
    assert_eq!(runtime.room_playlist_state("room").index, Some(1));
    assert_eq!(runtime.room_playlist_state("room").files[1], "b.mkv");
    assert_valid_successor_selection(&mut runtime);
    assert_eq!(
        observed,
        (120.0, false, 0),
        "removing a later different row must preserve playback"
    );
}

#[test]
fn appending_an_unselected_row_preserves_room_transport() {
    let mut runtime = playing_playlist(true);
    let observed = edit_and_observe(&mut runtime, vec!["a.mkv", "b.mkv", "c.mkv", "d.mkv"]);
    assert_eq!(runtime.room_playlist_state("room").index, Some(1));
    assert_valid_successor_selection(&mut runtime);
    assert_eq!(
        observed,
        (120.0, false, 0),
        "appending must preserve playback"
    );
}

#[test]
fn queueing_without_selection_preserves_room_transport() {
    let mut runtime = playing_playlist(true);
    let mut client = playlist_client(true);
    assert!(client.run_queue_playlist_item("d.mkv", false).unwrap());
    let lines = queued_client_lines(&mut client);
    assert_eq!(lines.len(), 1, "the edit and selection share one receipt");
    let observed = edit_lines_and_observe(&mut runtime, lines);
    assert_eq!(observed, (120.0, false, 0));
    assert_valid_successor_selection(&mut runtime);
}

#[test]
fn deleting_the_active_row_resets_to_the_successor() {
    let mut runtime = playing_playlist(true);
    let mut client = playlist_client(true);
    assert!(client.run_delete_playlist_index(1).unwrap());
    let observed = edit_lines_and_observe(&mut runtime, queued_client_lines(&mut client));
    assert_eq!(runtime.room_playlist_state("room").index, Some(1));
    assert_eq!(runtime.room_playlist_state("room").files[1], "c.mkv");
    assert_eq!((observed.0, observed.1), (0.0, true));
    assert!(
        observed.2 > 0,
        "the successor receives reset transport authority"
    );
    assert_valid_successor_selection(&mut runtime);
}

#[test]
fn deleting_the_active_duplicate_resets_transport() {
    let files = ["episode.mkv", "episode.mkv", "episode.mkv"];
    for active_index in [0, 1, 2] {
        let mut runtime = playing_playlist_entries(true, &files, active_index);
        let mut client = playlist_client_entries(true, &files, active_index);
        assert!(
            client
                .run_delete_playlist_index(active_index as i64)
                .unwrap()
        );
        let observed = edit_lines_and_observe(&mut runtime, queued_client_lines(&mut client));
        assert_eq!(runtime.room_playlist_state("room").files.len(), 2);
        assert_eq!(
            runtime.room_playlist_state("room").index,
            Some(active_index.min(1) as i64)
        );
        assert_eq!((observed.0, observed.1), (0.0, true));
        assert!(observed.2 > 0, "the new row needs explicit reset authority");
        assert_valid_successor_selection(&mut runtime);
    }
}

#[test]
fn deleting_an_inactive_duplicate_preserves_transport() {
    let files = ["episode.mkv", "episode.mkv", "episode.mkv"];
    for (active_index, delete_index, expected_index) in [(1, 0, 0), (0, 1, 0), (2, 1, 1)] {
        let mut runtime = playing_playlist_entries(true, &files, active_index);
        let mut client = playlist_client_entries(true, &files, active_index);
        assert!(client.run_delete_playlist_index(delete_index).unwrap());
        let lines = queued_client_lines(&mut client);
        assert_eq!(
            lines.len(),
            1,
            "the surviving row keeps compound edit semantics"
        );
        let observed = edit_lines_and_observe(&mut runtime, lines);
        assert_eq!(
            runtime.room_playlist_state("room").index,
            Some(expected_index)
        );
        assert_eq!(observed, (120.0, false, 0));
        assert_valid_successor_selection(&mut runtime);
    }
}

#[test]
fn reordering_duplicate_rows_preserves_transport() {
    let files = ["episode.mkv", "middle.mkv", "episode.mkv", "last.mkv"];
    let mut runtime = playing_playlist_entries(true, &files, 2);
    let mut client = playlist_client_entries(true, &files, 2);
    assert!(
        client
            .run_replace_playlist(
                ["last.mkv", "episode.mkv", "middle.mkv", "episode.mkv"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                Some(3),
            )
            .unwrap()
    );
    let lines = queued_client_lines(&mut client);
    assert_eq!(lines.len(), 1);
    let observed = edit_lines_and_observe(&mut runtime, lines);
    assert_eq!(runtime.room_playlist_state("room").index, Some(3));
    assert_eq!(observed, (120.0, false, 0));
    assert_valid_successor_selection(&mut runtime);
}

#[test]
fn explicit_duplicate_selection_after_replacement_resets_transport() {
    let files = ["episode.mkv", "episode.mkv", "last.mkv"];
    let mut runtime = playing_playlist_entries(true, &files, 1);
    let mut client = playlist_client_entries(true, &files, 1);
    assert!(
        client
            .run_replace_playlist(
                ["episode.mkv", "episode.mkv", "replacement.mkv"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                Some(1),
            )
            .unwrap()
    );
    assert!(client.run_set_playlist_index(0).unwrap());
    let observed = edit_lines_and_observe(&mut runtime, queued_client_lines(&mut client));
    assert_eq!(runtime.room_playlist_state("room").index, Some(0));
    assert_eq!((observed.0, observed.1), (0.0, true));
    assert!(observed.2 > 0);
    assert_valid_successor_selection(&mut runtime);
}

#[test]
fn replacing_the_selected_file_at_the_same_index_resets_transport() {
    let mut runtime = playing_playlist(true);
    let mut client = playlist_client(true);
    assert!(
        client
            .run_replace_playlist(
                vec![
                    "a.mkv".to_owned(),
                    "replacement.mkv".to_owned(),
                    "c.mkv".to_owned()
                ],
                Some(1),
            )
            .unwrap()
    );
    let observed = edit_lines_and_observe(&mut runtime, queued_client_lines(&mut client));
    assert_eq!(runtime.room_playlist_state("room").index, Some(1));
    assert_eq!((observed.0, observed.1), (0.0, true));
    assert!(
        observed.2 > 0,
        "the replacement receives reset transport authority"
    );
    assert_valid_successor_selection(&mut runtime);
}

#[test]
fn syncplay_playlist_reorder_retains_independent_transport() {
    let mut runtime = playing_playlist(false);
    let lines = client_playlist_edit(vec!["c.mkv", "a.mkv", "b.mkv"], false);
    assert_eq!(
        lines.len(),
        2,
        "Syncplay keeps its existing separate frames"
    );
    let observed = edit_lines_and_observe(&mut runtime, lines);
    assert_eq!(runtime.room_playlist_state("room").index, Some(2));
    assert_eq!(observed, (120.0, false, 0));
}

#[test]
fn compound_edit_preserves_transport_for_a_syncplay_peer_in_a_mixed_room() {
    let mut runtime = playing_playlist(true);
    exchange(
        &mut runtime,
        "bob",
        r#"{"Set":{"features":{"sorottePlaybackBarrierV1":false}}}"#,
    );
    let observed = edit_and_observe(&mut runtime, vec!["c.mkv", "a.mkv", "b.mkv"]);
    assert_eq!(observed, (120.0, false, 0));
    assert_eq!(runtime.room_playlist_state("room").index, Some(2));
    // Even a Syncplay peer's subsequent standalone replay remains explicit
    // selection authority in a room with a lifecycle-capable member.
    assert_valid_successor_selection(&mut runtime);
}

fn deliver_to_bob(bob: &mut ClientSession, lines: &[DirectedOutboundLine], now: f64) {
    for (recipient, message) in decode_directed_lines(lines) {
        if recipient == "bob" {
            bob.apply_protocol_message_at(message, now).unwrap();
        }
    }
}

fn departed_playstate_author(author_moves: bool) -> Option<f64> {
    let mut runtime = ServerRuntime::default();
    runtime.set_clock_overrides_seconds(Some(100.0), Some(0.0));
    let mut bob = ClientSession::default();
    // Bob can see global room metadata before joining room-a. Keeper remains
    // in room-a without a loaded file, as while choosing/resolving media.
    for (client, room) in [("bob", "room-c"), ("alice", "room-a"), ("keeper", "room-a")] {
        let replies = exchange(
            &mut runtime,
            client,
            &json!({"Hello": {
                "username": client, "room": {"name": room}, "version": "1.7.5",
                "features": {"sorottePlaybackBarrierV1": true}
            }})
            .to_string(),
        );
        deliver_to_bob(&mut bob, &replies, 100.0);
    }
    exchange(
        &mut runtime,
        "alice",
        r#"{"Set":{"file":{"name":"movie.mkv","duration":600.0}}}"#,
    );
    exchange(
        &mut runtime,
        "alice",
        r#"{"State":{"playstate":{"position":120.0,"paused":true,"doSeek":true}}}"#,
    );
    if author_moves {
        let replies = exchange(
            &mut runtime,
            "alice",
            r#"{"Set":{"room":{"name":"room-b"}}}"#,
        );
        deliver_to_bob(&mut bob, &replies, 100.0);
        assert_eq!(bob.user_room("alice"), Some("room-b"));
    }
    let replies = exchange(&mut runtime, "bob", r#"{"Set":{"room":{"name":"room-a"}}}"#);
    deliver_to_bob(&mut bob, &replies, 100.0);
    assert_eq!(bob.room(), Some("room-a"));
    for second in 1..=5 {
        let now = 100.0 + f64::from(second);
        runtime.set_clock_overrides_seconds(Some(now), Some(f64::from(second)));
        let replies = runtime.collect_dispatch_at(now).unwrap().outbound_lines;
        acknowledge_directed_state_counters(&mut runtime, &decode_directed_lines(&replies));
        for (recipient, message) in decode_directed_lines(&replies) {
            if recipient == "bob"
                && let ProtocolMessage::State(state) = message
                && let Some(playstate) = state.state.playstate
            {
                println!("bob receives room-a tick {second}: {playstate:?}");
            }
        }
        deliver_to_bob(&mut bob, &replies, now);
    }
    assert_eq!(runtime.room_playback_state("room-a").position, 120.0);
    let observed = bob
        .current_room_playstate()
        .and_then(|state| state.position);
    assert!(
        bob.room_playstate("room-b").is_none(),
        "historical authorship must not mutate playback in another room"
    );
    println!(
        "after join: bob room={:?}, current={:?}, room-b={:?}",
        bob.room(),
        bob.current_room_playstate(),
        bob.room_playstate("room-b")
    );
    // A later valid transport command remains usable and repairs attribution.
    let replies = exchange(
        &mut runtime,
        "bob",
        r#"{"State":{"playstate":{"position":90.0,"paused":true,"doSeek":true}}}"#,
    );
    deliver_to_bob(&mut bob, &replies, 105.0);
    assert_eq!(runtime.room_playback_state("room-a").position, 90.0);
    assert_eq!(
        bob.current_room_playstate()
            .and_then(|state| state.position),
        Some(90.0)
    );
    observed
}

#[test]
fn joining_after_the_playstate_author_moves_uses_membership_scope() {
    assert_eq!(
        departed_playstate_author(true),
        Some(120.0),
        "a current room State must not be routed to its historical author's new room"
    );
}

#[test]
fn joining_with_the_playstate_author_present_uses_membership_scope() {
    assert_eq!(departed_playstate_author(false), Some(120.0));
}
