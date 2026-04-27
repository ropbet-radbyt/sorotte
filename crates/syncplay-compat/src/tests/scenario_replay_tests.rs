use super::*;

#[test]
fn scripted_server_runtime_scenario_replays_and_fanout_decodes() {
    let events = replay_server_runtime_scenario_fixture("server_runtime_fanout.jsonl")
        .expect("scenario fixture should replay through server runtime");
    assert_eq!(events.len(), 7);

    let mut saw_bob_join_to_alice = false;
    let mut saw_bob_room2_update_to_alice = false;
    let mut saw_ready_broadcast_to_bob = false;
    let mut saw_state_echo_to_bob = false;

    for event in &events {
        for outbound in &event.outbound_lines {
            let message = decode_message_line(&outbound.line)
                .expect("fanout output line should decode as protocol message");
            match message {
                ProtocolMessage::Set(payload) => {
                    if let Some(user_map) = payload.set.user.as_ref() {
                        if outbound.client_id == "client-1"
                            && user_map
                                .get("bob")
                                .and_then(|u| u.event.as_ref())
                                .and_then(|event| event.get("joined"))
                                == Some(&json!(true))
                        {
                            saw_bob_join_to_alice = true;
                        }
                        if outbound.client_id == "client-1"
                            && user_map
                                .get("bob")
                                .and_then(|u| u.room.as_ref())
                                .map(|room| room.name.as_str())
                                == Some("room2")
                        {
                            saw_bob_room2_update_to_alice = true;
                        }
                    }
                    if let Some(ready) = payload.set.ready.as_ref()
                        && outbound.client_id == "client-2"
                        && ready.username.as_deref() == Some("alice")
                        && ready.is_ready == Some(true)
                    {
                        saw_ready_broadcast_to_bob = true;
                    }
                }
                ProtocolMessage::State(payload) => {
                    if outbound.client_id == "client-2"
                        && payload.state.playstate.as_ref().is_some_and(|playstate| {
                            playstate.set_by.as_deref() == Some("bob")
                                && playstate.position == Some(10.0)
                                && playstate.paused == Some(false)
                                && playstate.do_seek == Some(false)
                        })
                    {
                        saw_state_echo_to_bob = true;
                    }
                }
                ProtocolMessage::Hello(_)
                | ProtocolMessage::List(_)
                | ProtocolMessage::Chat(_)
                | ProtocolMessage::Error(_)
                | ProtocolMessage::Tls(_) => {}
            }
        }
    }

    assert!(
        saw_bob_join_to_alice,
        "scenario should include bob join fanout to alice"
    );
    assert!(
        saw_bob_room2_update_to_alice,
        "scenario should include bob room2 user-update fanout to alice"
    );
    assert!(
        saw_ready_broadcast_to_bob,
        "scenario should include alice ready-state fanout to bob"
    );
    assert!(
        saw_state_echo_to_bob,
        "scenario should include bob state reflection after moving rooms"
    );
}

#[test]
fn python_fanout_roundtrip_matches_server_runtime_on_cross_room_playlist_scoping_scenario() {
    match assert_python_fanout_matches_server_runtime_for_scenario(
        CROSS_ROOM_PLAYLIST_SCOPING_SCENARIO,
    ) {
        Ok(()) => {}
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python fanout interop test skipped due to missing local prerequisites");
        }
        Err(err) => panic!(
            "python fanout interop for cross-room playlist scoping scenario should succeed, got: {err}"
        ),
    }
}
