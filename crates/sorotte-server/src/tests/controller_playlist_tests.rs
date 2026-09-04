use super::*;

fn controller_auth_payloads(
    directed_lines: &[DirectedOutboundLine],
) -> Vec<(String, String, bool)> {
    directed_lines
        .iter()
        .filter_map(|line| {
            let message = decode_message_line(&line.line).ok()?;
            let ProtocolMessage::Set(payload) = message else {
                return None;
            };
            let auth = payload.set.controller_auth?;
            Some((
                line.client_id.clone(),
                auth.room?,
                auth.success.unwrap_or(false),
            ))
        })
        .collect()
}

fn playlist_change_payloads(
    directed_lines: &[DirectedOutboundLine],
) -> Vec<(String, PlaylistChangePayload)> {
    directed_lines
        .iter()
        .filter_map(|line| {
            let message = decode_message_line(&line.line).ok()?;
            let ProtocolMessage::Set(payload) = message else {
                return None;
            };
            Some((line.client_id.clone(), payload.set.playlist_change?))
        })
        .collect()
}

fn playlist_index_payloads(
    directed_lines: &[DirectedOutboundLine],
) -> Vec<(String, PlaylistIndexPayload)> {
    directed_lines
        .iter()
        .filter_map(|line| {
            let message = decode_message_line(&line.line).ok()?;
            let ProtocolMessage::Set(payload) = message else {
                return None;
            };
            Some((line.client_id.clone(), payload.set.playlist_index?))
        })
        .collect()
}

fn playback_lifecycle_hello(username: &str) -> String {
    format!(
        r#"{{"Hello":{{"username":"{username}","room":{{"name":"room1"}},"version":"1.7.5","features":{{"sorottePlaybackBarrierV1":true}}}}}}"#
    )
}

#[test]
fn simultaneous_natural_eof_compare_and_set_commits_one_selection_generation() {
    let mut runtime = ServerRuntime::default();
    for (client_id, username) in [
        ("observer", "observer"),
        ("client-1", "alice"),
        ("client-2", "bob"),
        ("client-3", "carol"),
    ] {
        runtime
            .handle_line(
                client_id,
                &format!(
                    r#"{{"Hello":{{"username":"{username}","room":{{"name":"room1"}},"version":"1.7.5"}}}}"#
                ),
            )
            .expect("participant hello should establish a session");
    }
    runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"]}}}"#,
        )
        .expect("playlist should be accepted");
    runtime
        .handle_line_fanout("client-1", r#"{"Set":{"playlistIndex":{"index":0}}}"#)
        .expect("initial selection should be accepted");
    assert_eq!(runtime.room_playlist_state("room1").epoch, 2);

    let guarded_advance = r#"{"Set":{"playlistIndex":{"index":1,"sorotteExpectedPlaylistIndex":0,"sorotteExpectedPlaylistEpoch":2}}}"#;
    let winner = runtime
        .handle_line_fanout("client-1", guarded_advance)
        .expect("first EOF transition should be accepted");
    let loser_one = runtime
        .handle_line_fanout("client-2", guarded_advance)
        .expect("second EOF transition should be consumed");
    let loser_two = runtime
        .handle_line_fanout("client-3", guarded_advance)
        .expect("third EOF transition should be consumed");

    let winner_payloads = playlist_index_payloads(&winner);
    assert_eq!(
        winner_payloads.len(),
        4,
        "one accepted transition must fan out once"
    );
    assert!(winner_payloads.iter().all(|(_, payload)| {
        payload.index_value() == Some(1)
            && payload.playlist_epoch() == Some(3)
            && !payload.has_expected_playlist_state()
    }));
    assert!(
        loser_one.is_empty(),
        "a stale contender must not create a correction echo"
    );
    assert!(
        loser_two.is_empty(),
        "a stale contender must not create a correction echo"
    );
    assert_eq!(runtime.room_playlist_state("room1").index, Some(1));
    assert_eq!(runtime.room_playlist_state("room1").epoch, 3);

    let replay = runtime
        .handle_line_fanout("client-2", r#"{"Set":{"playlistIndex":{"index":1}}}"#)
        .expect("ordinary same-row replay should remain accepted");
    let replay_payloads = playlist_index_payloads(&replay);
    assert_eq!(replay_payloads.len(), 4);
    assert!(replay_payloads.iter().all(|(_, payload)| {
        payload.index_value() == Some(1) && payload.playlist_epoch() == Some(4)
    }));
    assert_eq!(runtime.room_playlist_state("room1").epoch, 4);
}

#[test]
fn direct_selection_atomically_retires_an_already_paused_predecessor_position() {
    let mut runtime = ServerRuntime::default();
    for (client_id, username) in [("client-1", "alice"), ("client-2", "bob")] {
        let hello_line = if client_id == "client-1" {
            playback_lifecycle_hello(username)
        } else {
            format!(
                r#"{{"Hello":{{"username":"{username}","room":{{"name":"room1"}},"version":"1.7.5"}}}}"#
            )
        };
        let hello = runtime
            .handle_line_fanout(client_id, &hello_line)
            .expect("participant hello should establish a session");
        acknowledge_directed_state_counters(&mut runtime, &decode_directed_lines(&hello));
    }
    runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"]}}}"#,
        )
        .expect("playlist should be accepted");
    let positioned = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"State":{"playstate":{"position":7.0,"paused":true,"doSeek":true}}}"#,
        )
        .expect("predecessor position should become canonical");
    acknowledge_directed_state_counters(&mut runtime, &decode_directed_lines(&positioned));
    assert_eq!(runtime.room_playback_state("room1").position, 7.0);
    assert!(runtime.room_playback_state("room1").paused);
    let predecessor_revision = runtime
        .transport_authority_revision_for_room("room1")
        .expect("the predecessor should have transport authority");

    let selection = runtime
        .handle_line_fanout("client-1", r#"{"Set":{"playlistIndex":{"index":0}}}"#)
        .expect("direct selection should be accepted");
    let selection_messages = decode_directed_lines(&selection);
    let successor_revision = runtime
        .transport_authority_revision_for_room("room1")
        .expect("the successor should have transport authority");
    assert!(successor_revision > predecessor_revision);
    assert_eq!(runtime.room_playback_state("room1").position, 0.0);
    assert!(runtime.room_playback_state("room1").paused);

    for client_id in ["client-1", "client-2"] {
        let recipient_messages: Vec<_> = selection_messages
            .iter()
            .filter(|(recipient, _)| recipient == client_id)
            .map(|(_, message)| message)
            .collect();
        let playlist_offset = recipient_messages
            .iter()
            .position(|message| {
                matches!(
                    message,
                    ProtocolMessage::Set(payload)
                        if payload.set.playlist_index.as_ref().is_some_and(|index| {
                            index.index_value() == Some(0)
                        })
                )
            })
            .expect("the successor selection should be fanned out");
        let state_offset = recipient_messages
            .iter()
            .position(|message| {
                matches!(
                    message,
                    ProtocolMessage::State(payload)
                        if payload.state.playstate.as_ref().is_some_and(|playstate| {
                            playstate.position == Some(0.0)
                                && playstate.paused == Some(true)
                        })
                )
            })
            .expect("the successor transport reset should be fanned out");
        assert!(playlist_offset < state_offset);
    }
}

#[test]
fn selected_entry_replacement_at_the_same_index_starts_fresh_transport_authority() {
    let mut runtime = ServerRuntime::default();
    for (client_id, username) in [("client-1", "alice"), ("client-2", "bob")] {
        let hello = runtime
            .handle_line_fanout(client_id, &playback_lifecycle_hello(username))
            .expect("participant hello should establish a session");
        acknowledge_directed_state_counters(&mut runtime, &decode_directed_lines(&hello));
    }
    runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"]}}}"#,
        )
        .expect("playlist should be accepted");
    let initial_selection = runtime
        .handle_line_fanout("client-1", r#"{"Set":{"playlistIndex":{"index":0}}}"#)
        .expect("initial selection should be accepted");
    acknowledge_directed_state_counters(&mut runtime, &decode_directed_lines(&initial_selection));
    let positioned = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"State":{"playstate":{"position":7.0,"paused":true,"doSeek":true}}}"#,
        )
        .expect("predecessor position should become canonical");
    acknowledge_directed_state_counters(&mut runtime, &decode_directed_lines(&positioned));
    let predecessor_revision = runtime
        .transport_authority_revision_for_room("room1")
        .expect("the predecessor should have transport authority");

    let replacement = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"playlistChange":{"files":["replacement.mkv","episode2.mkv"]}}}"#,
        )
        .expect("selected entry replacement should be accepted");
    let replacement_messages = decode_directed_lines(&replacement);
    assert_eq!(runtime.room_playlist_state("room1").index, Some(0));
    assert_eq!(runtime.room_playback_state("room1").position, 0.0);
    assert!(runtime.room_playback_state("room1").paused);
    assert!(
        runtime
            .transport_authority_revision_for_room("room1")
            .expect("the replacement should have transport authority")
            > predecessor_revision
    );

    for client_id in ["client-1", "client-2"] {
        let recipient_messages: Vec<_> = replacement_messages
            .iter()
            .filter(|(recipient, _)| recipient == client_id)
            .map(|(_, message)| message)
            .collect();
        let playlist_offset = recipient_messages
            .iter()
            .position(|message| {
                matches!(message, ProtocolMessage::Set(payload) if payload.set.playlist_change.is_some())
            })
            .expect("replacement contents should be fanned out");
        let index_offset = recipient_messages
            .iter()
            .position(|message| {
                matches!(
                    message,
                    ProtocolMessage::Set(payload)
                        if payload.set.playlist_index.as_ref().is_some_and(|index| {
                            index.index_value() == Some(0)
                        })
                )
            })
            .expect("the stable numeric row should be announced as a fresh selection");
        let state_offset = recipient_messages
            .iter()
            .position(|message| {
                matches!(
                    message,
                    ProtocolMessage::State(payload)
                        if payload.state.playstate.as_ref().is_some_and(|playstate| {
                            playstate.position == Some(0.0)
                                && playstate.paused == Some(true)
                                && playstate.set_by.as_deref() == Some("alice")
                        })
                )
            })
            .expect("fresh zero-position transport should follow the replacement selection");
        assert!(playlist_offset < index_offset);
        assert!(index_offset < state_offset);
    }
}

#[test]
fn lifecycle_same_row_replay_starts_fresh_transport_authority() {
    let mut runtime = ServerRuntime::default();
    for (client_id, username) in [("client-1", "alice"), ("client-2", "bob")] {
        let hello = runtime
            .handle_line_fanout(client_id, &playback_lifecycle_hello(username))
            .expect("participant hello should establish a lifecycle session");
        acknowledge_directed_state_counters(&mut runtime, &decode_directed_lines(&hello));
    }
    runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"]}}}"#,
        )
        .expect("playlist should be accepted");
    let initial_selection = runtime
        .handle_line_fanout("client-1", r#"{"Set":{"playlistIndex":{"index":0}}}"#)
        .expect("initial selection should be accepted");
    acknowledge_directed_state_counters(&mut runtime, &decode_directed_lines(&initial_selection));

    let initial_revision = runtime
        .transport_authority_revision_for_room("room1")
        .expect("initial selection should establish transport authority");
    let positioned = runtime
        .handle_line_fanout(
            "client-1",
            &format!(
                r#"{{"State":{{"playstate":{{"position":7.0,"paused":true,"doSeek":true,"sorotteTransportRevision":{initial_revision}}}}}}}"#
            ),
        )
        .expect("predecessor position should become canonical");
    acknowledge_directed_state_counters(&mut runtime, &decode_directed_lines(&positioned));
    let predecessor_revision = runtime
        .transport_authority_revision_for_room("room1")
        .expect("the predecessor should have transport authority");

    let replay = runtime
        .handle_line_fanout("client-2", r#"{"Set":{"playlistIndex":{"index":0}}}"#)
        .expect("same-row lifecycle replay should be accepted");
    let replay_messages = decode_directed_lines(&replay);
    let successor_revision = runtime
        .transport_authority_revision_for_room("room1")
        .expect("the replay should establish successor transport authority");
    assert!(successor_revision > predecessor_revision);
    assert_eq!(runtime.room_playback_state("room1").position, 0.0);
    assert!(runtime.room_playback_state("room1").paused);

    for client_id in ["client-1", "client-2"] {
        let recipient_messages: Vec<_> = replay_messages
            .iter()
            .filter(|(recipient, _)| recipient == client_id)
            .map(|(_, message)| message)
            .collect();
        let playlist_offset = recipient_messages
            .iter()
            .position(|message| {
                matches!(
                    message,
                    ProtocolMessage::Set(payload)
                        if payload.set.playlist_index.as_ref().is_some_and(|index| {
                            index.index_value() == Some(0)
                        })
                )
            })
            .expect("the replayed selection should be fanned out");
        let state_offset = recipient_messages
            .iter()
            .position(|message| {
                matches!(
                    message,
                    ProtocolMessage::State(payload)
                        if payload.state.playstate.as_ref().is_some_and(|playstate| {
                            playstate.position == Some(0.0)
                                && playstate.paused == Some(true)
                                && playstate.do_seek == Some(false)
                                && playstate.set_by.as_deref() == Some("bob")
                                && playstate.transport_revision().ok().flatten()
                                    == Some(successor_revision)
                        })
                )
            })
            .expect("successor transport must follow the replayed selection");
        assert!(playlist_offset < state_offset);
        let seeded = &runtime.client_playback_states[client_id];
        assert_eq!(seeded.position, Some(0.0));
        assert_eq!(seeded.transport_revision, Some(successor_revision));
    }
}

#[test]
fn guarded_natural_advance_retires_completed_media_transport_authority() {
    let mut runtime = ServerRuntime::default();
    for (client_id, username) in [("client-1", "alice"), ("client-2", "bob")] {
        let hello = runtime
            .handle_line_fanout(
                client_id,
                &format!(
                    r#"{{"Hello":{{"username":"{username}","room":{{"name":"room1"}},"version":"1.7.5"}}}}"#
                ),
            )
            .expect("participant hello should establish a session");
        acknowledge_directed_state_counters(&mut runtime, &decode_directed_lines(&hello));
    }
    runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"]}}}"#,
        )
        .expect("playlist should be accepted");
    let initial_selection = runtime
        .handle_line_fanout("client-1", r#"{"Set":{"playlistIndex":{"index":0}}}"#)
        .expect("initial selection should be accepted");
    acknowledge_directed_state_counters(&mut runtime, &decode_directed_lines(&initial_selection));

    let initial_revision = runtime
        .transport_authority_revision_for_room("room1")
        .expect("the room should have transport authority");
    let playing = runtime
        .handle_line_fanout(
            "client-1",
            &format!(
                r#"{{"State":{{"playstate":{{"position":9.75,"paused":false,"doSeek":true,"sorotteTransportRevision":{initial_revision}}}}}}}"#
            ),
        )
        .expect("the first item should become canonical playing state");
    acknowledge_directed_state_counters(&mut runtime, &decode_directed_lines(&playing));
    let completed_media_revision = runtime
        .transport_authority_revision_for_room("room1")
        .expect("the playing authority should be revisioned");
    assert!(!runtime.room_playback_state("room1").paused);
    assert_eq!(runtime.room_playback_state("room1").position, 9.75);

    let advance = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"playlistIndex":{"index":1,"sorotteExpectedPlaylistIndex":0,"sorotteExpectedPlaylistEpoch":2}}}"#,
        )
        .expect("the guarded EOF transition should be accepted");
    let advance_messages = decode_directed_lines(&advance);
    let successor_revision = runtime
        .transport_authority_revision_for_room("room1")
        .expect("the successor item should have transport authority");
    assert!(successor_revision > completed_media_revision);
    let successor = runtime.room_playback_state("room1");
    assert_eq!(successor.position, 0.0);
    assert!(successor.paused);

    for client_id in ["client-1", "client-2"] {
        let recipient_messages: Vec<_> = advance_messages
            .iter()
            .filter(|(recipient, _)| recipient == client_id)
            .map(|(_, message)| message)
            .collect();
        let playlist_offset = recipient_messages
            .iter()
            .position(|message| {
                matches!(
                    message,
                    ProtocolMessage::Set(payload)
                        if payload.set.playlist_index.as_ref().is_some_and(|index| {
                            index.index_value() == Some(1)
                        })
                )
            })
            .expect("the successor selection should be fanned out");
        let state_offset = recipient_messages
            .iter()
            .position(|message| {
                matches!(
                    message,
                    ProtocolMessage::State(payload)
                        if payload.state.playstate.as_ref().is_some_and(|playstate| {
                            playstate.position == Some(0.0)
                                && playstate.paused == Some(true)
                                && playstate.do_seek == Some(false)
                                && playstate.transport_revision().ok().flatten()
                                    == Some(successor_revision)
                        })
                )
            })
            .expect("the successor transport reset should be fanned out");
        assert!(
            playlist_offset < state_offset,
            "each client must select the successor before applying its transport reset"
        );
        let seeded = &runtime.client_playback_states[client_id];
        assert_eq!(seeded.position, Some(0.0));
        assert_eq!(seeded.transport_revision, Some(successor_revision));
    }
    acknowledge_directed_state_counters(&mut runtime, &advance_messages);

    let delayed = runtime
        .handle_line_fanout(
            "client-2",
            &format!(
                r#"{{"State":{{"playstate":{{"position":9.9,"paused":false,"doSeek":false,"sorotteTransportRevision":{completed_media_revision}}}}}}}"#
            ),
        )
        .expect("a retired completed-media sample should receive correction");
    assert_eq!(runtime.room_playback_state("room1"), successor);
    assert!(
        decode_directed_lines(&delayed)
            .iter()
            .any(|(recipient, message)| {
                recipient == "client-2"
                    && matches!(
                        message,
                        ProtocolMessage::State(payload)
                            if payload.state.playstate.as_ref().is_some_and(|playstate| {
                                playstate.position == Some(0.0)
                                    && playstate.paused == Some(true)
                                    && playstate.transport_revision().ok().flatten()
                                        == Some(successor_revision)
                            })
                    )
            })
    );
}

#[test]
fn malformed_playlist_precondition_fails_closed_as_a_correction() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should establish a session");
    runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"playlistChange":{"files":["a.mkv","b.mkv"]}}}"#,
        )
        .expect("playlist should be accepted");
    runtime
        .handle_line_fanout("client-1", r#"{"Set":{"playlistIndex":{"index":0}}}"#)
        .expect("initial selection should be accepted");

    for malformed in [
        r#"{"Set":{"playlistIndex":{"index":1,"sorotteExpectedPlaylistIndex":0}}}"#,
        r#"{"Set":{"playlistIndex":{"index":1,"sorotteExpectedPlaylistIndex":"zero","sorotteExpectedPlaylistEpoch":2}}}"#,
    ] {
        let correction = runtime
            .handle_line_fanout("client-1", malformed)
            .expect("malformed guard should receive canonical correction");
        let payloads = playlist_index_payloads(&correction);
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0].0, "client-1");
        assert_eq!(payloads[0].1.index_value(), Some(0));
        assert_eq!(payloads[0].1.playlist_epoch(), Some(2));
        assert_eq!(runtime.room_playlist_state("room1").index, Some(0));
        assert_eq!(runtime.room_playlist_state("room1").epoch, 2);
    }
}

#[test]
fn guarded_playlist_advance_rejects_aba_and_playlist_replacement() {
    let mut runtime = ServerRuntime::default();
    for (client_id, username) in [("client-1", "alice"), ("client-2", "bob")] {
        runtime
            .handle_line(
                client_id,
                &format!(
                    r#"{{"Hello":{{"username":"{username}","room":{{"name":"room1"}},"version":"1.7.5"}}}}"#
                ),
            )
            .expect("participant hello should establish a session");
    }
    runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"playlistChange":{"files":["a.mkv","b.mkv"]}}}"#,
        )
        .expect("playlist should be accepted");
    runtime
        .handle_line_fanout("client-1", r#"{"Set":{"playlistIndex":{"index":0}}}"#)
        .expect("initial selection should be accepted");
    let stale_guard = r#"{"Set":{"playlistIndex":{"index":1,"sorotteExpectedPlaylistIndex":0,"sorotteExpectedPlaylistEpoch":2}}}"#;

    runtime
        .handle_line_fanout("client-2", r#"{"Set":{"playlistIndex":{"index":1}}}"#)
        .expect("peer should advance selection");
    runtime
        .handle_line_fanout("client-2", r#"{"Set":{"playlistIndex":{"index":0}}}"#)
        .expect("peer should return to the original numeric index");
    assert_eq!(runtime.room_playlist_state("room1").index, Some(0));
    assert_eq!(runtime.room_playlist_state("room1").epoch, 4);
    assert!(
        runtime
            .handle_line_fanout("client-1", stale_guard)
            .expect("ABA-stale request should be consumed")
            .is_empty()
    );
    assert_eq!(runtime.room_playlist_state("room1").index, Some(0));
    assert_eq!(runtime.room_playlist_state("room1").epoch, 4);

    runtime
        .handle_line_fanout(
            "client-2",
            r#"{"Set":{"playlistChange":{"files":["replacement-a.mkv","replacement-b.mkv"]}}}"#,
        )
        .expect("replacement playlist should be accepted");
    assert_eq!(runtime.room_playlist_state("room1").epoch, 5);
    assert!(
        runtime
            .handle_line_fanout(
                "client-1",
                r#"{"Set":{"playlistIndex":{"index":1,"sorotteExpectedPlaylistIndex":0,"sorotteExpectedPlaylistEpoch":4}}}"#,
            )
            .expect("playlist-replacement-stale request should be consumed")
            .is_empty()
    );
    assert_eq!(runtime.room_playlist_state("room1").index, Some(0));
    assert_eq!(runtime.room_playlist_state("room1").epoch, 5);
}

#[test]
fn playlist_join_snapshot_carries_one_coherent_canonical_epoch() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("alice hello should establish a session");
    runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"playlistChange":{"files":["a.mkv","b.mkv"]}}}"#,
        )
        .expect("playlist should be accepted");
    runtime
        .handle_line_fanout("client-1", r#"{"Set":{"playlistIndex":{"index":0}}}"#)
        .expect("initial selection should be accepted");

    let join = runtime
        .handle_line_fanout(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("bob hello should receive a snapshot");
    let changes = playlist_change_payloads(&join);
    let indices = playlist_index_payloads(&join);
    assert!(changes.iter().any(|(client_id, payload)| {
        client_id == "client-2" && payload.playlist_epoch() == Some(2)
    }));
    assert!(indices.iter().any(|(client_id, payload)| {
        client_id == "client-2"
            && payload.index_value() == Some(0)
            && payload.playlist_epoch() == Some(2)
    }));
}

#[test]
fn empty_playlist_change_retires_selection_without_autoselecting_restore() {
    let mut runtime = ServerRuntime::default();
    for (client_id, username) in [("client-1", "alice"), ("client-2", "bob")] {
        runtime
            .handle_line(client_id, &playback_lifecycle_hello(username))
            .expect("participant hello should establish a session");
    }
    runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"playlistChange":{"files":["a.mkv","b.mkv"]}}}"#,
        )
        .expect("playlist should be accepted");
    runtime
        .handle_line_fanout("client-1", r#"{"Set":{"playlistIndex":{"index":1}}}"#)
        .expect("initial selection should be accepted");

    let cleared = runtime
        .handle_line_fanout("client-1", r#"{"Set":{"playlistChange":{"files":[]}}}"#)
        .expect("empty playlist should be accepted");
    let decoded = decode_directed_lines(&cleared);
    assert_eq!(
        runtime.room_playlist_state("room1").files,
        Vec::<String>::new()
    );
    assert_eq!(runtime.room_playlist_state("room1").index, None);
    assert_eq!(runtime.room_playlist_state("room1").epoch, 3);
    assert_eq!(runtime.room_playback_state("room1").position, 0.0);
    assert!(runtime.room_playback_state("room1").paused);

    let changes = playlist_change_payloads(&cleared);
    let indices = playlist_index_payloads(&cleared);
    assert_eq!(changes.len(), 2);
    assert_eq!(indices.len(), 2);
    assert!(changes.iter().all(|(_, payload)| {
        payload.files.is_empty()
            && payload.user.as_deref() == Some("alice")
            && payload.playlist_epoch() == Some(3)
    }));
    assert!(indices.iter().all(|(_, payload)| {
        payload.index_value().is_none()
            && payload.user.as_deref() == Some("alice")
            && payload.playlist_epoch() == Some(3)
    }));
    for client_id in ["client-1", "client-2"] {
        let recipient: Vec<_> = decoded
            .iter()
            .filter(|(recipient, _)| recipient == client_id)
            .map(|(_, message)| message)
            .collect();
        assert!(matches!(
            recipient.as_slice(),
            [ProtocolMessage::Set(change), ProtocolMessage::Set(index), ProtocolMessage::State(state)]
                if change.set.playlist_change.is_some()
                    && index.set.playlist_index.as_ref().is_some_and(|payload| {
                        payload.index_value().is_none()
                    })
                    && state.state.playstate.as_ref().is_some_and(|playstate| {
                        playstate.position == Some(0.0) && playstate.paused == Some(true)
                    })
        ));
    }

    let restored = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"playlistChange":{"files":["replacement.mkv"]}}}"#,
        )
        .expect("replacement contents should be accepted");
    assert_eq!(runtime.room_playlist_state("room1").index, None);
    assert_eq!(runtime.room_playlist_state("room1").epoch, 4);
    assert!(
        playlist_index_payloads(&restored).is_empty(),
        "non-empty contents must not silently select a row"
    );
}

#[test]
fn room_change_fanout_emits_global_room_update_and_playlist_snapshot() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");
    runtime
        .handle_line(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("bob hello should establish session");
    runtime
        .handle_line(
            "client-3",
            r#"{"Hello":{"username":"carol","room":{"name":"room2"},"version":"1.2.255"}}"#,
        )
        .expect("carol hello should establish session");

    let directed_lines = runtime
        .handle_line_fanout("client-1", r#"{"Set":{"room":{"name":"room2"}}}"#)
        .expect("room change should fan out");
    let directed_messages = decode_directed_lines(&directed_lines);

    assert!(
        has_user_room_update(&directed_messages, "client-1", "alice", "room2"),
        "sender should receive global user room update"
    );
    assert!(
        has_user_room_update(&directed_messages, "client-2", "alice", "room2"),
        "old-room peer should receive global user room update"
    );
    assert!(
        has_user_room_update(&directed_messages, "client-3", "alice", "room2"),
        "new-room peer should receive global user room update"
    );
    assert!(
        has_playlist_snapshot(&directed_messages, "client-1", &[]),
        "moved user should receive playlist snapshot after room switch"
    );
    assert!(
        !has_playlist_snapshot(&directed_messages, "client-3", &[]),
        "destination room peers should not receive direct playlist snapshot for mover"
    );
    assert!(
        has_room_sync_state_update(&directed_messages, "client-1", true),
        "moved user should receive seek room sync state update"
    );
}

#[test]
fn controller_auth_grants_requested_room_when_current_room_differs() {
    let controlled_room_name = controlled_room_name_for_test("target", "AB-123-456");
    let mut runtime = server_runtime_with_default_controlled_room_salt_for_test();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"lobby"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");
    runtime
        .handle_line(
            "client-2",
            &format!(
                r#"{{"Hello":{{"username":"bob","room":{{"name":"{controlled_room_name}"}},"version":"1.2.255"}}}}"#
            ),
        )
        .expect("bob hello should establish session");

    let directed_lines = runtime
        .handle_line_fanout(
            "client-1",
            &format!(
                r#"{{"Set":{{"controllerAuth":{{"room":"{controlled_room_name}","password":"AB-123-456"}}}}}}"#
            ),
        )
        .expect("alice auth should succeed for requested room");

    assert!(runtime.user_is_room_controller("alice", &controlled_room_name));
    assert!(
        !runtime.user_is_room_controller("alice", "lobby"),
        "auth should not be granted for the sender's current room"
    );
    assert_eq!(
        controller_auth_payloads(&directed_lines),
        vec![("client-2".to_owned(), controlled_room_name, true)]
    );
}

#[test]
fn controller_auth_omitted_room_uses_current_room() {
    let controlled_room_name = controlled_room_name_for_test("room1", "AB-123-456");
    let mut runtime = server_runtime_with_default_controlled_room_salt_for_test();
    for (client_id, username) in [("client-1", "alice"), ("client-2", "bob")] {
        runtime
            .handle_line(
                client_id,
                &format!(
                    r#"{{"Hello":{{"username":"{username}","room":{{"name":"{controlled_room_name}"}},"version":"1.2.255"}}}}"#
                ),
            )
            .expect("hello should establish session");
    }

    let directed_lines = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"controllerAuth":{"password":"AB-123-456"}}}"#,
        )
        .expect("alice auth should use current room");

    assert!(runtime.user_is_room_controller("alice", &controlled_room_name));
    assert_eq!(
        controller_auth_payloads(&directed_lines),
        vec![
            ("client-1".to_owned(), controlled_room_name.clone(), true),
            ("client-2".to_owned(), controlled_room_name, true),
        ]
    );
}

#[test]
fn controller_auth_status_reports_requested_room() {
    let controlled_room_name = controlled_room_name_for_test("target", "AB-123-456");
    let mut runtime = server_runtime_with_default_controlled_room_salt_for_test();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"lobby"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");
    runtime
        .handle_line(
            "client-2",
            &format!(
                r#"{{"Hello":{{"username":"bob","room":{{"name":"{controlled_room_name}"}},"version":"1.2.255"}}}}"#
            ),
        )
        .expect("bob hello should establish session");

    let directed_lines = runtime
        .handle_line_fanout(
            "client-1",
            &format!(
                r#"{{"Set":{{"controllerAuth":{{"room":"{controlled_room_name}","password":"AB-123-456"}}}}}}"#
            ),
        )
        .expect("alice auth should succeed for requested room");

    let auth_payloads = controller_auth_payloads(&directed_lines);
    assert!(
        !auth_payloads.is_empty(),
        "controllerAuth status should fan out to requested room peers"
    );
    assert!(
        auth_payloads
            .iter()
            .all(|(_, room, success)| room == &controlled_room_name && *success),
        "controllerAuth status should report the requested room"
    );
}

#[test]
fn controller_auth_on_uncontrolled_room_returns_new_controlled_room_to_sender() {
    let mut runtime = server_runtime_with_default_controlled_room_salt_for_test();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");

    let directed_lines = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"controllerAuth":{"room":"room1","password":"AB-123-456"}}}"#,
        )
        .expect("controller auth on uncontrolled room should respond");
    assert_eq!(directed_lines.len(), 1);
    assert_eq!(directed_lines[0].client_id, "client-1");

    let message = decode_message_line(&directed_lines[0].line)
        .expect("new controlled room line should decode");
    let expected_room_name = controlled_room_name_for_test("room1", "AB-123-456");
    match message {
        ProtocolMessage::Set(payload) => {
            let new_room = payload
                .set
                .new_controlled_room
                .as_ref()
                .expect("newControlledRoom payload should be present");
            assert_eq!(
                new_room
                    .password
                    .as_ref()
                    .map(|password| password.expose_secret()),
                Some("AB-123-456")
            );
            assert_eq!(
                new_room.room_name.as_deref(),
                Some(expected_room_name.as_str())
            );
        }
        other => panic!("expected set response, got {}", other.kind()),
    }
}

#[test]
fn controller_auth_respects_runtime_configured_room_password_salt() {
    let mut runtime = ServerRuntime::with_room_password_salt("custom-salt");
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");

    let directed_lines = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"controllerAuth":{"room":"room1","password":"AB-123-456"}}}"#,
        )
        .expect("controller auth on uncontrolled room should respond");
    assert_eq!(directed_lines.len(), 1);
    assert_eq!(directed_lines[0].client_id, "client-1");

    let message = decode_message_line(&directed_lines[0].line)
        .expect("new controlled room line should decode");
    let expected_room_name =
        controlled_room_name_for_salt_test("room1", "AB-123-456", "custom-salt");
    let default_room_name = controlled_room_name_for_test("room1", "AB-123-456");
    match message {
        ProtocolMessage::Set(payload) => {
            let new_room = payload
                .set
                .new_controlled_room
                .as_ref()
                .expect("newControlledRoom payload should be present");
            assert_eq!(
                new_room.room_name.as_deref(),
                Some(expected_room_name.as_str())
            );
            assert_ne!(
                new_room.room_name.as_deref(),
                Some(default_room_name.as_str())
            );
        }
        other => panic!("expected set response, got {}", other.kind()),
    }
}

#[test]
fn controlled_room_playlist_updates_require_controller_auth() {
    let controlled_room_name = controlled_room_name_for_test("room1", "AB-123-456");
    let mut runtime = server_runtime_with_default_controlled_room_salt_for_test();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");
    runtime
        .handle_line(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("bob hello should establish session");
    runtime
        .handle_line_fanout(
            "client-1",
            &format!(r#"{{"Set":{{"room":{{"name":"{controlled_room_name}"}}}}}}"#),
        )
        .expect("alice room switch should succeed");
    runtime
        .handle_line_fanout(
            "client-2",
            &format!(r#"{{"Set":{{"room":{{"name":"{controlled_room_name}"}}}}}}"#),
        )
        .expect("bob room switch should succeed");

    let bob_change_attempt = runtime
        .handle_line_fanout(
            "client-2",
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"]}}}"#,
        )
        .expect("bob playlist change attempt should respond");
    assert_eq!(bob_change_attempt.len(), 2);
    assert!(
        bob_change_attempt
            .iter()
            .all(|line| line.client_id == "client-2"),
        "non-controller correction should be directed only to sender"
    );
    let bob_messages: Vec<_> = bob_change_attempt
        .iter()
        .map(|line| decode_message_line(&line.line).expect("line should decode"))
        .collect();
    assert!(
        bob_messages.iter().any(|message| match message {
            ProtocolMessage::Set(payload) =>
                payload
                    .set
                    .playlist_change
                    .as_ref()
                    .is_some_and(|playlist| {
                        playlist.files.is_empty()
                            && playlist.user.as_deref() == Some(controlled_room_name.as_str())
                    },),
            _ => false,
        }),
        "non-controller should receive playlistChange correction for room state"
    );
    assert!(
        bob_messages.iter().any(|message| match message {
            ProtocolMessage::Set(payload) =>
                payload
                    .set
                    .playlist_index
                    .as_ref()
                    .is_some_and(|playlist_index| {
                        playlist_index.index_value().is_none()
                            && playlist_index.user.as_deref() == Some(controlled_room_name.as_str())
                    }),
            _ => false,
        }),
        "non-controller should receive playlistIndex correction for room state"
    );
    let alice_auth = runtime
        .handle_line_fanout(
            "client-1",
            &format!(
                r#"{{"Set":{{"controllerAuth":{{"room":"{controlled_room_name}","password":"AB-123-456"}}}}}}"#
            ),
        )
        .expect("alice auth should succeed");
    assert!(
        alice_auth.iter().any(|line| {
            decode_message_line(&line.line)
                .ok()
                .is_some_and(|message| match message {
                    ProtocolMessage::Set(payload) => payload
                        .set
                        .controller_auth
                        .as_ref()
                        .is_some_and(|auth| auth.success == Some(true)),
                    _ => false,
                })
        }),
        "controller auth success should be broadcast"
    );

    let alice_change = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"]}}}"#,
        )
        .expect("alice playlist change should succeed as controller");
    assert!(
        alice_change.iter().any(|line| line.client_id == "client-1")
            && alice_change.iter().any(|line| line.client_id == "client-2"),
        "controller playlist change should fan out to room peers"
    );
}

#[test]
fn plex_playlist_sidecar_is_sent_only_to_opted_in_sorotte_clients() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sorottePlexPlaylistUris":true}}}"#,
        )
        .expect("alice hello should establish session");
    runtime
        .handle_line(
            "client-2",
            r#"{"Hello":{"username":"python","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("python hello should establish session");

    let plex_uri =
        "plex://server/metadata/14452?title=Episode%2011&file=Episode%2011%20%5B1080p%5D.mkv";
    let directed_lines = runtime
        .handle_line_fanout(
            "client-1",
            &format!(
                r#"{{"Set":{{"playlistChange":{{"files":["Episode 11 [1080p].mkv"],"sorottePlexPlaylistUris":["{plex_uri}"]}}}}}}"#
            ),
        )
        .expect("playlist sidecar update should fan out");

    let payloads = playlist_change_payloads(&directed_lines);
    let alice_payload = payloads
        .iter()
        .find(|(client_id, _)| client_id == "client-1")
        .map(|(_, payload)| payload)
        .expect("alice should receive playlist update");
    let python_payload = payloads
        .iter()
        .find(|(client_id, _)| client_id == "client-2")
        .map(|(_, payload)| payload)
        .expect("python client should receive playlist update");

    assert_eq!(
        alice_payload.files,
        vec!["Episode 11 [1080p].mkv".to_owned()]
    );
    assert_eq!(
        alice_payload.extra.get("sorottePlexPlaylistUris"),
        Some(&json!([plex_uri]))
    );
    assert_eq!(
        python_payload.files,
        vec!["Episode 11 [1080p].mkv".to_owned()]
    );
    assert!(!python_payload.extra.contains_key("sorottePlexPlaylistUris"));

    let late_join_lines = runtime
        .handle_line_fanout(
            "client-3",
            r#"{"Hello":{"username":"carol","room":{"name":"room1"},"version":"1.7.5","features":{"sorottePlexPlaylistUris":true}}}"#,
        )
        .expect("late Sorotte hello should receive room snapshot");
    let late_payload = playlist_change_payloads(&late_join_lines)
        .into_iter()
        .find(|(client_id, _)| client_id == "client-3")
        .map(|(_, payload)| payload)
        .expect("late Sorotte client should receive playlist snapshot");
    assert_eq!(
        late_payload.files,
        vec!["Episode 11 [1080p].mkv".to_owned()]
    );
    assert_eq!(
        late_payload.extra.get("sorottePlexPlaylistUris"),
        Some(&json!([plex_uri]))
    );
}

#[test]
fn invalid_playlist_change_is_rejected_with_current_room_playlist_correction() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("alice hello should establish session");

    let files: Vec<String> = (0..=DEFAULT_PLAYLIST_MAX_ITEMS)
        .map(|index| format!("episode-{index}.mkv"))
        .collect();
    let messages = runtime
        .handle_protocol_message_fanout(
            "client-1",
            ProtocolMessage::set(
                SetPayload::new().with_playlist_change(PlaylistChangePayload::new(files)),
            ),
        )
        .expect("invalid playlist should be rejected with correction");

    assert_eq!(
        runtime.room_playlist_state("room1").files,
        Vec::<String>::new(),
        "invalid playlist should not replace room playlist state"
    );
    assert!(
        messages.iter().any(|message| {
            message.client_id == "client-1"
                && matches!(
                    &message.message,
                    ProtocolMessage::Set(payload)
                        if payload.set.playlist_change.as_ref().is_some_and(|playlist| {
                            playlist.files.is_empty()
                                && playlist.user.as_deref() == Some("room1")
                        })
                )
        }),
        "sender should receive current playlist correction"
    );
}

#[test]
fn non_controller_playlist_index_update_receives_current_index_correction() {
    let controlled_room_name = controlled_room_name_for_test("room1", "AB-123-456");
    let mut runtime = server_runtime_with_default_controlled_room_salt_for_test();
    for (client_id, username) in [("client-1", "alice"), ("client-2", "bob")] {
        let hello = format!(
            r#"{{"Hello":{{"username":"{username}","room":{{"name":"{controlled_room_name}"}},"version":"1.2.255"}}}}"#
        );
        runtime
            .handle_line(client_id, &hello)
            .expect("hello should establish session");
    }
    runtime
        .handle_line_fanout(
            "client-1",
            &format!(
                r#"{{"Set":{{"controllerAuth":{{"room":"{controlled_room_name}","password":"AB-123-456"}}}}}}"#
            ),
        )
        .expect("alice auth should succeed");
    runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"]}}}"#,
        )
        .expect("controller playlist change should succeed");
    runtime
        .handle_line_fanout("client-1", r#"{"Set":{"playlistIndex":{"index":1}}}"#)
        .expect("controller playlist index should succeed");

    let bob_index_attempt = runtime
        .handle_line_fanout("client-2", r#"{"Set":{"playlistIndex":{"index":0}}}"#)
        .expect("non-controller playlist index attempt should respond");
    let bob_messages = decode_directed_lines(&bob_index_attempt);

    assert!(
        bob_messages.iter().any(|(client_id, message)| {
            client_id == "client-2"
                && matches!(
                    message,
                    ProtocolMessage::Set(payload)
                        if payload.set.playlist_index.as_ref().is_some_and(|playlist_index| {
                            playlist_index.index == 1
                                && playlist_index.user.as_deref()
                                    == Some(controlled_room_name.as_str())
                        })
                )
        }),
        "non-controller should receive current playlistIndex correction"
    );
}

#[test]
fn playlist_index_rejects_negative_and_out_of_range_values() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should establish session");
    runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"playlistChange":{"files":["a.mkv","b.mkv"]}}}"#,
        )
        .expect("playlist should be accepted");
    assert_eq!(
        runtime.room_playlist_state("room1").index,
        None,
        "playlistChange must not synthesize the separate playlistIndex operation"
    );
    runtime
        .handle_line_fanout("client-1", r#"{"Set":{"playlistIndex":{"index":1}}}"#)
        .expect("valid index should be accepted");

    for invalid_index in [-1_i64, 2, i64::MAX] {
        let correction = runtime
            .handle_line_fanout(
                "client-1",
                &format!(r#"{{"Set":{{"playlistIndex":{{"index":{invalid_index}}}}}}}"#),
            )
            .expect("invalid index should receive a correction");
        assert_eq!(runtime.room_playlist_state("room1").index, Some(1));
        assert!(has_playlist_index_snapshot(
            &decode_directed_lines(&correction),
            "client-1",
            1
        ));
    }

    let null_correction = runtime
        .handle_line_fanout("client-1", r#"{"Set":{"playlistIndex":{"index":null}}}"#)
        .expect("a null index for a nonempty playlist should receive a correction");
    assert_eq!(runtime.room_playlist_state("room1").index, Some(1));
    assert!(has_playlist_index_snapshot(
        &decode_directed_lines(&null_correction),
        "client-1",
        1
    ));
}

#[test]
fn playlist_replacement_retires_an_explicit_index_that_no_longer_names_an_entry() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line("client-1", &playback_lifecycle_hello("alice"))
        .expect("hello should establish session");
    runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"playlistChange":{"files":["a.mkv","b.mkv","c.mkv"]}}}"#,
        )
        .expect("playlist should be accepted");
    runtime
        .handle_line_fanout("client-1", r#"{"Set":{"playlistIndex":{"index":2}}}"#)
        .expect("valid index should be accepted");

    let shortened = runtime
        .handle_line_fanout(
            "client-1",
            r#"{"Set":{"playlistChange":{"files":["replacement.mkv"]}}}"#,
        )
        .expect("shorter replacement should be accepted");
    assert_eq!(runtime.room_playlist_state("room1").index, None);
    assert!(
        decode_directed_lines(&shortened)
            .iter()
            .any(|(_, message)| matches!(
                message,
                ProtocolMessage::Set(payload)
                    if payload.set.playlist_index.as_ref().is_some_and(|index| {
                        index.index_value().is_none()
                    })
            )),
        "playlist replacement must retire the now-invalid selection"
    );

    let cleared = runtime
        .handle_line_fanout("client-1", r#"{"Set":{"playlistChange":{"files":[]}}}"#)
        .expect("playlist clear should be accepted");
    assert_eq!(runtime.room_playlist_state("room1").index, None);
    assert!(
        decode_directed_lines(&cleared)
            .iter()
            .all(|(_, message)| !matches!(
                message,
                ProtocolMessage::Set(payload) if payload.set.playlist_index.is_some()
            )),
        "an already-retired selection must not synthesize another playlistIndex(null)"
    );
}
