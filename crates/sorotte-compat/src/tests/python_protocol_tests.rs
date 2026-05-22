use super::*;

#[test]
fn python_interop_roundtrip_returns_server_hello() {
    let transcript = match run_python_handshake_roundtrip() {
        Ok(transcript) => transcript,
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python interop handshake test skipped due to missing local prerequisites");
            return;
        }
        Err(err) => panic!("python interop handshake should succeed, got: {err}"),
    };

    assert_eq!(transcript.response_hello.username, "interop-client");
    assert_eq!(transcript.response_hello.room.name, "interop-room");
    assert_eq!(transcript.response_hello.version, "sorotte-dev");
    assert_eq!(
        transcript.response_hello.realversion.as_deref(),
        Some("1.7.5")
    );
}

#[test]
fn python_interop_sequence_supports_list_set_and_state() {
    let requests = vec![
        ProtocolMessage::hello(default_rust_client_hello_for_interop()),
        ProtocolMessage::list_request(),
        ProtocolMessage::set(SetPayload::new().with_room(RoomRef::new("interop-room-2"))),
        ProtocolMessage::list_request(),
        ProtocolMessage::set(
            SetPayload::new().with_ready(
                ReadyPayload::new(true)
                    .with_manually_initiated(true)
                    .with_username("interop-client"),
            ),
        ),
        ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(42.0)
                    .with_paused(false)
                    .with_do_seek(false),
            ),
        ),
    ];

    let transcript = match run_python_protocol_roundtrip(&requests) {
        Ok(transcript) => transcript,
        Err(InteropError::LegacySyncplayCheckoutMissing(_))
        | Err(InteropError::PythonSpawn { .. }) => {
            eprintln!("python interop sequence test skipped due to missing local prerequisites");
            return;
        }
        Err(err) => panic!("python interop sequence should succeed, got: {err}"),
    };

    assert_eq!(transcript.steps.len(), requests.len());

    let hello = extract_hello_from_message(
        transcript.steps[0]
            .response_messages
            .first()
            .expect("hello step should return one message")
            .clone(),
    )
    .expect("first response should be hello");
    assert_eq!(hello.room.name, "interop-room");

    match transcript.steps[1]
        .response_messages
        .first()
        .expect("list response should be present")
    {
        ProtocolMessage::List(payload) => match &payload.list {
            ListPayload::Rooms(rooms) => {
                assert!(rooms.contains_key("interop-room"));
                let room = rooms.get("interop-room").expect("room should exist");
                assert!(room.contains_key("interop-client"));
            }
            other => panic!("expected list room snapshot, got {other:?}"),
        },
        other => panic!("expected list response, got {}", other.kind()),
    }

    match transcript.steps[2]
        .response_messages
        .first()
        .expect("set room response should be present")
    {
        ProtocolMessage::Set(payload) => {
            let room = payload
                .set
                .room
                .as_ref()
                .expect("set room payload should exist");
            assert_eq!(room.name, "interop-room-2");
        }
        other => panic!("expected set response, got {}", other.kind()),
    }

    match transcript.steps[3]
        .response_messages
        .first()
        .expect("second list response should be present")
    {
        ProtocolMessage::List(payload) => match &payload.list {
            ListPayload::Rooms(rooms) => {
                assert!(rooms.contains_key("interop-room-2"));
                let room = rooms.get("interop-room-2").expect("room should exist");
                assert!(room.contains_key("interop-client"));
            }
            other => panic!("expected list room snapshot, got {other:?}"),
        },
        other => panic!("expected list response, got {}", other.kind()),
    }

    match transcript.steps[4]
        .response_messages
        .first()
        .expect("set ready response should be present")
    {
        ProtocolMessage::Set(payload) => {
            let ready = payload
                .set
                .ready
                .as_ref()
                .expect("ready payload should be present");
            assert_eq!(ready.username.as_deref(), Some("interop-client"));
            assert_eq!(ready.is_ready, Some(true));
        }
        other => panic!("expected set response, got {}", other.kind()),
    }

    assert!(
        transcript.steps[5].response_messages.is_empty(),
        "state message should be accepted without an immediate response"
    );
}
