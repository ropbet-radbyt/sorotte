use super::*;

#[test]
fn scripted_server_runtime_chat_room_switch_object_payload_scoping_normalizes_and_rescopes_during_transition()
 {
    let events = replay_server_runtime_scenario_fixture(
            CHAT_ROOM_SWITCH_OBJECT_PAYLOAD_SCOPING_SCENARIO,
        )
        .expect(
            "chat room-switch object-payload scoping scenario fixture should replay through server runtime",
        );
    assert_eq!(events.len(), 7);

    let step4 = events
        .get(3)
        .expect("step 4 room switch event should be present");
    let step4_contains_chat = step4.outbound_lines.iter().any(|outbound| {
        matches!(
            decode_message_line(&outbound.line),
            Ok(ProtocolMessage::Chat(_))
        )
    });
    assert!(
        !step4_contains_chat,
        "room switch event should not emit chat fanout payloads"
    );

    let step5_outbound_chats: Vec<_> = events
        .get(4)
        .expect("step 5 old-room object chat event should be present")
        .outbound_lines
        .iter()
        .filter_map(|outbound| {
            let ProtocolMessage::Chat(chat_message) = decode_message_line(&outbound.line).ok()?
            else {
                return None;
            };
            Some((outbound.client_id.clone(), chat_message.chat))
        })
        .collect();
    assert_eq!(step5_outbound_chats.len(), 1);
    assert_eq!(
        step5_outbound_chats[0].0, "client-1",
        "old-room peer object chat immediately after bob switches should not leak to bob"
    );
    match &step5_outbound_chats[0].1 {
        ChatPayload::Message(message) => {
            assert_eq!(message.username, "alice");
            assert_eq!(message.message, "old room object after bob switch");
            assert!(
                message.extra.is_empty(),
                "outbound normalized chat payload should drop inbound extra fields for old-room peer chat"
            );
        }
        other => panic!("expected normalized outbound chat message payload, got {other:?}"),
    }

    let step6_outbound_chats: Vec<_> = events
        .get(5)
        .expect("step 6 new-room object chat event should be present")
        .outbound_lines
        .iter()
        .filter_map(|outbound| {
            let ProtocolMessage::Chat(chat_message) = decode_message_line(&outbound.line).ok()?
            else {
                return None;
            };
            Some((outbound.client_id.clone(), chat_message.chat))
        })
        .collect();
    let step6_recipients: Vec<_> = step6_outbound_chats
        .iter()
        .map(|(client_id, _)| client_id.clone())
        .collect();
    assert_eq!(
        step6_recipients,
        vec!["client-2".to_owned(), "client-3".to_owned()],
        "new-room peer object chat immediately after bob switches should include bob and carol only"
    );
    for (_, chat_payload) in step6_outbound_chats {
        match chat_payload {
            ChatPayload::Message(message) => {
                assert_eq!(message.username, "carol");
                assert_eq!(message.message, "new room object after bob switch");
                assert!(
                    message.extra.is_empty(),
                    "outbound normalized chat payload should drop inbound extra fields for new-room peer chat"
                );
            }
            other => panic!("expected normalized outbound chat message payload, got {other:?}"),
        }
    }

    let step7_outbound_chats: Vec<_> = events
        .get(6)
        .expect("step 7 moved-sender object chat event should be present")
        .outbound_lines
        .iter()
        .filter_map(|outbound| {
            let ProtocolMessage::Chat(chat_message) = decode_message_line(&outbound.line).ok()?
            else {
                return None;
            };
            Some((outbound.client_id.clone(), chat_message.chat))
        })
        .collect();
    let step7_recipients: Vec<_> = step7_outbound_chats
        .iter()
        .map(|(client_id, _)| client_id.clone())
        .collect();
    assert_eq!(
        step7_recipients,
        vec!["client-2".to_owned(), "client-3".to_owned()],
        "moved sender object chat should use destination-room membership immediately"
    );
    for (_, chat_payload) in step7_outbound_chats {
        match chat_payload {
            ChatPayload::Message(message) => {
                assert_eq!(
                    message.username, "bob",
                    "spoofed inbound username should be replaced with authenticated moved sender username"
                );
                assert_eq!(message.message, "bob object after move");
                assert!(
                    message.extra.is_empty(),
                    "outbound normalized chat payload should drop inbound extra fields for moved-sender object chat"
                );
            }
            other => panic!("expected normalized outbound chat message payload, got {other:?}"),
        }
    }
}

#[test]
fn scripted_server_runtime_chat_double_room_switch_scoping_uses_latest_room_membership_for_fanout()
{
    let events = replay_server_runtime_scenario_fixture(CHAT_DOUBLE_ROOM_SWITCH_SCOPING_SCENARIO)
        .expect(
            "chat double-room-switch scoping scenario fixture should replay through server runtime",
        );
    assert_eq!(events.len(), 8);

    for (step_index, expected_client_id) in [("step 4", "client-2"), ("step 5", "client-2")] {
        let event = match step_index {
            "step 4" => events
                .get(3)
                .expect("step 4 first room switch should be present"),
            _ => events
                .get(4)
                .expect("step 5 second room switch should be present"),
        };
        assert_eq!(event.client_id, expected_client_id);
        let contains_chat = event.outbound_lines.iter().any(|outbound| {
            matches!(
                decode_message_line(&outbound.line),
                Ok(ProtocolMessage::Chat(_))
            )
        });
        assert!(
            !contains_chat,
            "{step_index} room switch event should not emit chat fanout payloads"
        );
    }

    let step6 = events
        .get(5)
        .expect("step 6 room2 chat after bounce should be present");
    let step6_chat_recipients: Vec<_> = step6
        .outbound_lines
        .iter()
        .filter_map(|outbound| {
            let ProtocolMessage::Chat(chat_message) = decode_message_line(&outbound.line).ok()?
            else {
                return None;
            };
            match chat_message.chat {
                ChatPayload::Message(message)
                    if message.username == "carol"
                        && message.message == "room2 no bob after bounce" =>
                {
                    Some(outbound.client_id.clone())
                }
                _ => None,
            }
        })
        .collect();
    assert_eq!(
        step6_chat_recipients,
        vec!["client-3".to_owned()],
        "after bob bounces back to room1, room2 chat should not leak to bob"
    );

    let step7 = events
        .get(6)
        .expect("step 7 room1 peer chat after bounce should be present");
    let step7_chat_recipients: Vec<_> = step7
        .outbound_lines
        .iter()
        .filter_map(|outbound| {
            let ProtocolMessage::Chat(chat_message) = decode_message_line(&outbound.line).ok()?
            else {
                return None;
            };
            match chat_message.chat {
                ChatPayload::Message(message)
                    if message.username == "alice" && message.message == "room1 still has bob" =>
                {
                    Some(outbound.client_id.clone())
                }
                _ => None,
            }
        })
        .collect();
    assert_eq!(
        step7_chat_recipients,
        vec!["client-1".to_owned(), "client-2".to_owned()],
        "post-bounce room1 peer chat should include bob again using final room membership"
    );

    let step8 = events
        .get(7)
        .expect("step 8 bob chat after bounce should be present");
    let step8_chat_recipients: Vec<_> = step8
        .outbound_lines
        .iter()
        .filter_map(|outbound| {
            let ProtocolMessage::Chat(chat_message) = decode_message_line(&outbound.line).ok()?
            else {
                return None;
            };
            match chat_message.chat {
                ChatPayload::Message(message)
                    if message.username == "bob" && message.message == "bob back in room1" =>
                {
                    Some(outbound.client_id.clone())
                }
                _ => None,
            }
        })
        .collect();
    assert_eq!(
        step8_chat_recipients,
        vec!["client-1".to_owned(), "client-2".to_owned()],
        "sender chat after double room switch should use the latest room membership (room1)"
    );
}

#[test]
fn scripted_server_runtime_chat_username_normalization_scenario_uses_session_username() {
    let events = replay_server_runtime_scenario_fixture(CHAT_USERNAME_NORMALIZATION_SCENARIO)
        .expect(
            "chat username-normalization scenario fixture should replay through server runtime",
        );
    assert_eq!(events.len(), 3);

    let chat_event = events
        .get(2)
        .expect("step 3 spoofed chat event should be present");
    assert_eq!(chat_event.client_id, "client-1");

    let matching_chat_messages: Vec<_> = chat_event
        .outbound_lines
        .iter()
        .filter_map(|outbound| {
            let ProtocolMessage::Chat(chat_message) = decode_message_line(&outbound.line).ok()?
            else {
                return None;
            };
            match chat_message.chat {
                ChatPayload::Message(message)
                    if message.message == "spoofed username should be ignored" =>
                {
                    Some((outbound.client_id.clone(), message.username))
                }
                _ => None,
            }
        })
        .collect();

    let recipients: Vec<_> = matching_chat_messages
        .iter()
        .map(|(client_id, _)| client_id.clone())
        .collect();
    assert_eq!(
        recipients,
        vec!["client-1".to_owned(), "client-2".to_owned()],
        "spoofed chat should fan out to both room members"
    );

    let observed_usernames: Vec<_> = matching_chat_messages
        .into_iter()
        .map(|(_, username)| username)
        .collect();
    assert_eq!(
        observed_usernames,
        vec!["alice".to_owned(), "alice".to_owned()],
        "server should normalize outbound chat sender username to authenticated session username"
    );
}

#[test]
fn scripted_server_runtime_chat_payload_normalization_scenario_normalizes_format_and_order() {
    let events = replay_server_runtime_scenario_fixture(CHAT_PAYLOAD_NORMALIZATION_SCENARIO)
        .expect("chat payload-normalization scenario fixture should replay through server runtime");
    assert_eq!(events.len(), 4);

    let step3 = events
        .get(2)
        .expect("step 3 text chat event should be present");
    let step3_outbound_chats: Vec<_> = step3
        .outbound_lines
        .iter()
        .filter_map(|outbound| {
            let ProtocolMessage::Chat(chat_message) = decode_message_line(&outbound.line).ok()?
            else {
                return None;
            };
            Some((outbound.client_id.clone(), chat_message.chat))
        })
        .collect();
    assert_eq!(step3_outbound_chats.len(), 2);
    let step3_recipients: Vec<_> = step3_outbound_chats
        .iter()
        .map(|(client_id, _)| client_id.clone())
        .collect();
    assert_eq!(
        step3_recipients,
        vec!["client-1".to_owned(), "client-2".to_owned()],
        "text chat should fan out to sender and peer in room order"
    );
    for (_, chat_payload) in step3_outbound_chats {
        match chat_payload {
            ChatPayload::Message(message) => {
                assert_eq!(message.username, "alice");
                assert_eq!(message.message, "plain text first");
                assert!(
                    message.extra.is_empty(),
                    "outbound normalized chat payload should not preserve extra fields"
                );
            }
            other => panic!("expected normalized outbound chat message payload, got {other:?}"),
        }
    }

    let step4 = events
        .get(3)
        .expect("step 4 object chat event should be present");
    let step4_outbound_chats: Vec<_> = step4
        .outbound_lines
        .iter()
        .filter_map(|outbound| {
            let ProtocolMessage::Chat(chat_message) = decode_message_line(&outbound.line).ok()?
            else {
                return None;
            };
            Some((outbound.client_id.clone(), chat_message.chat))
        })
        .collect();
    assert_eq!(step4_outbound_chats.len(), 2);
    let step4_recipients: Vec<_> = step4_outbound_chats
        .iter()
        .map(|(client_id, _)| client_id.clone())
        .collect();
    assert_eq!(
        step4_recipients,
        vec!["client-1".to_owned(), "client-2".to_owned()],
        "object chat should fan out to sender and peer in room order"
    );
    for (_, chat_payload) in step4_outbound_chats {
        match chat_payload {
            ChatPayload::Message(message) => {
                assert_eq!(
                    message.username, "bob",
                    "spoofed inbound username should be replaced with authenticated username"
                );
                assert_eq!(message.message, "object payload second");
                assert!(
                    message.extra.is_empty(),
                    "outbound normalized chat payload should drop inbound extra fields"
                );
            }
            other => panic!("expected normalized outbound chat message payload, got {other:?}"),
        }
    }
}
