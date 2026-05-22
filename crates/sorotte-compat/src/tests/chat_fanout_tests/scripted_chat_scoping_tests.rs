use super::*;

#[test]
fn scripted_server_runtime_chat_room_scoping_scenario_validates_room_scoped_chat_fanout() {
    let events = replay_server_runtime_scenario_fixture(CHAT_ROOM_SCOPING_SCENARIO)
        .expect("chat room-scoping scenario fixture should replay through server runtime");
    assert_eq!(events.len(), 8);

    let step4 = events
        .get(3)
        .expect("step 4 room1 chat event should be present");
    assert_eq!(step4.client_id, "client-1");
    let step4_chat_recipients: Vec<_> = step4
        .outbound_lines
        .iter()
        .filter_map(|outbound| {
            let ProtocolMessage::Chat(chat_message) = decode_message_line(&outbound.line).ok()?
            else {
                return None;
            };
            match chat_message.chat {
                ChatPayload::Message(message)
                    if message.username == "alice" && message.message == "hello room1" =>
                {
                    Some(outbound.client_id.clone())
                }
                _ => None,
            }
        })
        .collect();
    assert_eq!(
        step4_chat_recipients,
        vec!["client-1".to_owned(), "client-3".to_owned()],
        "room1 chat should fan out only to room1 members (including sender)"
    );

    let step5 = events
        .get(4)
        .expect("step 5 room2 chat event should be present");
    assert_eq!(step5.client_id, "client-2");
    let step5_chat_recipients: Vec<_> = step5
        .outbound_lines
        .iter()
        .filter_map(|outbound| {
            let ProtocolMessage::Chat(chat_message) = decode_message_line(&outbound.line).ok()?
            else {
                return None;
            };
            match chat_message.chat {
                ChatPayload::Message(message)
                    if message.username == "bob" && message.message == "hello room2" =>
                {
                    Some(outbound.client_id.clone())
                }
                _ => None,
            }
        })
        .collect();
    assert_eq!(
        step5_chat_recipients,
        vec!["client-2".to_owned()],
        "room2 chat should initially fan out only to bob before carol moves"
    );

    let step7 = events
        .get(6)
        .expect("step 7 post-move room1 chat event should be present");
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
                    if message.username == "alice" && message.message == "still room1" =>
                {
                    Some(outbound.client_id.clone())
                }
                _ => None,
            }
        })
        .collect();
    assert_eq!(
        step7_chat_recipients,
        vec!["client-1".to_owned()],
        "after carol moves, room1 chat should stay scoped to alice only"
    );

    let step8 = events
        .get(7)
        .expect("step 8 post-move room2 chat event should be present");
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
                    if message.username == "bob" && message.message == "room2 after move" =>
                {
                    Some(outbound.client_id.clone())
                }
                _ => None,
            }
        })
        .collect();
    assert_eq!(
        step8_chat_recipients,
        vec!["client-2".to_owned(), "client-3".to_owned()],
        "after carol moves, room2 chat should fan out to bob and carol only"
    );
}

#[test]
fn scripted_server_runtime_chat_room_switch_sender_scoping_updates_sender_room_before_echo_fanout()
{
    let events = replay_server_runtime_scenario_fixture(CHAT_ROOM_SWITCH_SENDER_SCOPING_SCENARIO)
        .expect(
            "chat room-switch sender-scoping scenario fixture should replay through server runtime",
        );
    assert_eq!(events.len(), 8);

    let step4 = events
        .get(3)
        .expect("step 4 pre-move sender chat event should be present");
    assert_eq!(step4.client_id, "client-2");
    let step4_chat_recipients: Vec<_> = step4
        .outbound_lines
        .iter()
        .filter_map(|outbound| {
            let ProtocolMessage::Chat(chat_message) = decode_message_line(&outbound.line).ok()?
            else {
                return None;
            };
            match chat_message.chat {
                ChatPayload::Message(message)
                    if message.username == "bob" && message.message == "before move from bob" =>
                {
                    Some(outbound.client_id.clone())
                }
                _ => None,
            }
        })
        .collect();
    assert_eq!(
        step4_chat_recipients,
        vec!["client-1".to_owned(), "client-2".to_owned()],
        "pre-move sender chat should fan out to room1 members in room order"
    );

    let step5 = events
        .get(4)
        .expect("step 5 room switch event should be present");
    assert_eq!(step5.client_id, "client-2");
    let step5_contains_chat = step5.outbound_lines.iter().any(|outbound| {
        matches!(
            decode_message_line(&outbound.line),
            Ok(ProtocolMessage::Chat(_))
        )
    });
    assert!(
        !step5_contains_chat,
        "room switch event should not emit chat fanout payloads"
    );

    let step6 = events
        .get(5)
        .expect("step 6 post-move sender chat event should be present");
    assert_eq!(step6.client_id, "client-2");
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
                    if message.username == "bob" && message.message == "after move from bob" =>
                {
                    Some(outbound.client_id.clone())
                }
                _ => None,
            }
        })
        .collect();
    assert_eq!(
        step6_chat_recipients,
        vec!["client-2".to_owned(), "client-3".to_owned()],
        "post-move sender chat should fan out to destination room members only (no stale old-room echo)"
    );

    let step7 = events
        .get(6)
        .expect("step 7 old-room follow-up chat should be present");
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
                    if message.username == "alice" && message.message == "room1 after bob left" =>
                {
                    Some(outbound.client_id.clone())
                }
                _ => None,
            }
        })
        .collect();
    assert_eq!(
        step7_chat_recipients,
        vec!["client-1".to_owned()],
        "old-room follow-up chat should not leak back to moved sender"
    );

    let step8 = events
        .get(7)
        .expect("step 8 destination-room follow-up chat should be present");
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
                    if message.username == "carol" && message.message == "room2 sees bob" =>
                {
                    Some(outbound.client_id.clone())
                }
                _ => None,
            }
        })
        .collect();
    assert_eq!(
        step8_chat_recipients,
        vec!["client-2".to_owned(), "client-3".to_owned()],
        "destination-room follow-up chat should include moved sender and existing member in room order"
    );
}

#[test]
fn scripted_server_runtime_chat_room_switch_peer_transition_scoping_applies_room_change_before_peer_chat_fanout()
 {
    let events = replay_server_runtime_scenario_fixture(
            CHAT_ROOM_SWITCH_PEER_TRANSITION_SCOPING_SCENARIO,
        )
        .expect(
            "chat room-switch peer-transition scoping scenario fixture should replay through server runtime",
        );
    assert_eq!(events.len(), 7);

    let step4 = events
        .get(3)
        .expect("step 4 room switch event should be present");
    assert_eq!(step4.client_id, "client-2");
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

    let step5 = events
        .get(4)
        .expect("step 5 old-room peer chat event should be present");
    let step5_chat_recipients: Vec<_> = step5
        .outbound_lines
        .iter()
        .filter_map(|outbound| {
            let ProtocolMessage::Chat(chat_message) = decode_message_line(&outbound.line).ok()?
            else {
                return None;
            };
            match chat_message.chat {
                ChatPayload::Message(message)
                    if message.username == "alice"
                        && message.message == "room1 immediate after bob switch" =>
                {
                    Some(outbound.client_id.clone())
                }
                _ => None,
            }
        })
        .collect();
    assert_eq!(
        step5_chat_recipients,
        vec!["client-1".to_owned()],
        "old-room peer chat immediately after bob switches should not leak to bob"
    );

    let step6 = events
        .get(5)
        .expect("step 6 destination-room peer chat event should be present");
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
                        && message.message == "room2 immediate after bob switch" =>
                {
                    Some(outbound.client_id.clone())
                }
                _ => None,
            }
        })
        .collect();
    assert_eq!(
        step6_chat_recipients,
        vec!["client-2".to_owned(), "client-3".to_owned()],
        "destination-room peer chat immediately after bob switches should include bob and carol in stable room recipient order"
    );

    let step7 = events
        .get(6)
        .expect("step 7 moved-sender chat event should be present");
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
                    if message.username == "bob"
                        && message.message == "bob confirms room2 immediately" =>
                {
                    Some(outbound.client_id.clone())
                }
                _ => None,
            }
        })
        .collect();
    assert_eq!(
        step7_chat_recipients,
        vec!["client-2".to_owned(), "client-3".to_owned()],
        "moved sender chat should use destination-room membership immediately after peer transition-window chats"
    );
}
