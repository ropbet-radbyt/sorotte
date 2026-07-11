use super::*;

#[test]
fn gui_client_core_outbound_delivery_retains_front_until_matching_write_receipt() {
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let startup = adapter
        .begin_outbound_protocol_delivery()
        .expect("startup delivery should stage")
        .expect("startup Hello should be pending");
    assert!(startup.line().contains("\"Hello\""));
    assert!(
        adapter
            .begin_outbound_protocol_delivery()
            .expect("duplicate delivery probe should succeed")
            .is_none(),
        "one session line must remain in flight until a receipt arrives"
    );
    adapter
        .fail_outbound_protocol_delivery(startup.token())
        .expect("failed transport attempt should release only the staged clone");

    let startup_retry = adapter
        .begin_outbound_protocol_delivery()
        .expect("startup retry should stage")
        .expect("failed startup Hello should remain pending");
    assert_eq!(startup_retry.line(), startup.line());
    adapter
        .acknowledge_outbound_protocol_delivery(startup_retry.token())
        .expect("matching write receipt should acknowledge startup Hello");
    assert!(adapter.pending_startup_protocol_lines.is_empty());

    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("server Hello should activate the session");
    GuiSessionRuntimeAdapter::send_chat_message(&mut adapter, "receipt-canary".to_owned())
        .expect("chat should queue");
    assert_eq!(adapter.runtime.pending_protocol_message_count(), 1);

    let chat = adapter
        .begin_outbound_protocol_delivery()
        .expect("chat delivery should stage")
        .expect("chat should be pending");
    assert!(chat.line().contains("receipt-canary"));
    assert_eq!(
        adapter.runtime.pending_protocol_message_count(),
        1,
        "staging must not acknowledge the client-core outbox"
    );
    assert!(
        adapter
            .acknowledge_outbound_protocol_delivery(chat.token().wrapping_add(1))
            .is_err(),
        "a stale or mismatched receipt must not pop the outbox front"
    );
    assert_eq!(adapter.runtime.pending_protocol_message_count(), 1);
    adapter
        .acknowledge_outbound_protocol_delivery(chat.token())
        .expect("matching write receipt should acknowledge chat");
    assert_eq!(adapter.runtime.pending_protocol_message_count(), 0);
}

#[test]
fn failed_runtime_delivery_retries_only_after_reconnect_hello_completes() {
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");
    let startup = adapter
        .begin_outbound_protocol_delivery()
        .expect("startup delivery should stage")
        .expect("startup Hello should be pending");
    adapter
        .acknowledge_outbound_protocol_delivery(startup.token())
        .expect("startup Hello should acknowledge");
    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("server Hello should activate the session");
    GuiSessionRuntimeAdapter::send_chat_message(&mut adapter, "retry-canary".to_owned())
        .expect("chat should queue");

    let failed_chat = adapter
        .begin_outbound_protocol_delivery()
        .expect("chat delivery should stage")
        .expect("chat should be pending");
    adapter
        .fail_outbound_protocol_delivery(failed_chat.token())
        .expect("partial write failure should leave the core front unacknowledged");
    adapter.prepare_transport_reconnect();

    let reconnect_hello = adapter
        .begin_outbound_protocol_delivery()
        .expect("reconnect Hello should stage")
        .expect("reconnect Hello should take priority");
    assert!(reconnect_hello.line().contains("\"Hello\""));
    assert!(!reconnect_hello.line().contains("retry-canary"));
    adapter
        .acknowledge_outbound_protocol_delivery(reconnect_hello.token())
        .expect("reconnect Hello write should acknowledge");
    assert!(
        adapter
            .begin_outbound_protocol_delivery()
            .expect("inactive delivery probe should succeed")
            .is_none(),
        "retained commands must wait for the server Hello, not merely the Hello socket write"
    );
    assert_eq!(adapter.runtime.pending_protocol_message_count(), 1);

    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("replacement server Hello should reactivate the session");
    let retried_chat = adapter
        .begin_outbound_protocol_delivery()
        .expect("retained chat retry should stage")
        .expect("retained chat should retry after activation");
    assert!(retried_chat.line().contains("retry-canary"));
    assert_ne!(retried_chat.token(), failed_chat.token());
}

#[test]
fn gui_hello_shared_playlist_feature_preserves_default_and_explicit_values() {
    for (configured, expected) in [(None, true), (Some(true), true), (Some(false), false)] {
        let runtime_settings =
            stored_client_settings_runtime_snapshot_legacy_compatible(&StoredClientSettingsMvp {
                shared_playlist_enabled: configured,
                ..StoredClientSettingsMvp::default()
            });
        let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
            .expect("client-core chat adapter should bootstrap");
        GuiSessionRuntimeAdapter::sync_runtime_settings(&mut adapter, &runtime_settings)
            .expect("runtime settings should sync into the startup Hello");

        let startup_lines = adapter
            .flush_outbound_protocol_lines()
            .expect("startup protocol lines should encode");
        let ProtocolMessage::Hello(hello) =
            decode_message_line(&startup_lines[0]).expect("startup Hello should decode")
        else {
            panic!("startup protocol line should be a Hello message");
        };
        assert_eq!(
            hello
                .hello
                .features
                .as_ref()
                .and_then(serde_json::Value::as_object)
                .and_then(|features| features.get("sharedPlaylists"))
                .and_then(serde_json::Value::as_bool),
            Some(expected),
            "unexpected sharedPlaylists value for stored setting {configured:?}"
        );
    }
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_startup_hello_includes_hashed_password_and_full_features()
 {
    let runtime_settings =
        stored_client_settings_runtime_snapshot_legacy_compatible(&StoredClientSettingsMvp {
            username: Some("bob".to_owned()),
            room: Some("room2".to_owned()),
            server_password: Some("secret-pass".into()),
            shared_playlist_enabled: Some(false),
            ..StoredClientSettingsMvp::default()
        });
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    GuiSessionRuntimeAdapter::sync_runtime_settings(&mut adapter, &runtime_settings)
        .expect("runtime settings should sync into the startup hello");
    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    let ProtocolMessage::Hello(hello) =
        decode_message_line(&startup_lines[0]).expect("startup hello should decode")
    else {
        panic!("startup protocol line should be a Hello message");
    };
    assert_eq!(hello.hello.username, "bob");
    assert_eq!(hello.hello.room.name, "room2");
    assert_eq!(hello.hello.version, SYNCPLAY_WIRE_VERSION_LEGACY);
    assert_eq!(
        hello.hello.realversion.as_deref(),
        Some(SYNCPLAY_COMPAT_VERSION_LEGACY)
    );
    assert_eq!(
        hello
            .hello
            .extra
            .get("password")
            .and_then(serde_json::Value::as_str),
        Some("591fac3e56ffbdc6f310c1b646050c09")
    );

    let features = hello
        .hello
        .features
        .as_ref()
        .and_then(serde_json::Value::as_object)
        .expect("startup hello should advertise a feature map");
    assert_eq!(
        features
            .get("sharedPlaylists")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert_eq!(
        features.get("chat").and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        features.get("uiMode").and_then(serde_json::Value::as_str),
        Some("GUI")
    );
    assert_eq!(
        features
            .get("featureList")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        features
            .get("readiness")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        features
            .get("managedRooms")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        features
            .get("persistentRooms")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        features
            .get("setOthersReadiness")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        features
            .get("mediaMatch")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        features
            .get("sorottePlexPlaylistUris")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_reconnect_hello_uses_updated_runtime_identity() {
    let runtime_settings =
        stored_client_settings_runtime_snapshot_legacy_compatible(&StoredClientSettingsMvp {
            username: Some("bob".to_owned()),
            room: Some("room2".to_owned()),
            ..StoredClientSettingsMvp::default()
        });
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let _ = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");

    GuiSessionRuntimeAdapter::sync_runtime_settings(&mut adapter, &runtime_settings)
        .expect("runtime settings should sync into the reconnect hello");
    adapter.reset_session_for_reconnect();
    let reconnect_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("reconnect protocol lines should encode");
    assert_eq!(reconnect_lines.len(), 1);

    let ProtocolMessage::Hello(hello) =
        decode_message_line(&reconnect_lines[0]).expect("reconnect hello should decode")
    else {
        panic!("reconnect protocol line should be a Hello message");
    };
    assert_eq!(hello.hello.username, "bob");
    assert_eq!(hello.hello.room.name, "room2");
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_reconnect_hello_uses_server_assigned_username() {
    let runtime_settings =
        stored_client_settings_runtime_snapshot_legacy_compatible(&StoredClientSettingsMvp {
            username: Some("alice".to_owned()),
            room: Some("room1".to_owned()),
            ..StoredClientSettingsMvp::default()
        });
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let _ = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice_2","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("server hello should apply");

    GuiSessionRuntimeAdapter::sync_runtime_settings(&mut adapter, &runtime_settings)
        .expect("runtime settings sync should preserve the server-assigned username");
    adapter.prepare_transport_reconnect();
    let reconnect_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("reconnect protocol lines should encode");
    assert_eq!(reconnect_lines.len(), 1);

    let ProtocolMessage::Hello(hello) =
        decode_message_line(&reconnect_lines[0]).expect("reconnect hello should decode")
    else {
        panic!("reconnect protocol line should be a Hello message");
    };
    assert_eq!(hello.hello.username, "alice_2");
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_reconnect_hello_preserves_current_room_over_local_file_name()
 {
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let _ = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("inbound server hello should apply");
    adapter
        .apply_message_json(
            r#"{"Set":{"user":{"alice":{"room":{"name":"room2"},"file":{"name":"movie.mkv","size":123456789,"duration":95.5}}}}}"#,
        )
        .expect("local user update should apply");

    adapter.reset_session_for_reconnect();
    let reconnect_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("reconnect protocol lines should encode");
    assert_eq!(reconnect_lines.len(), 1);

    let ProtocolMessage::Hello(hello) =
        decode_message_line(&reconnect_lines[0]).expect("reconnect hello should decode")
    else {
        panic!("reconnect protocol line should be a Hello message");
    };
    assert_eq!(hello.hello.room.name, "room2");
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_reconnect_hello_preserves_pending_room_switch() {
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let _ = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("inbound server hello should apply");

    GuiSessionRuntimeAdapter::set_room(&mut adapter, "room2".to_owned())
        .expect("room change should queue");
    let outbound_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("queued room change should encode");
    let set_message = outbound_lines
        .iter()
        .find_map(|line| {
            match decode_message_line(line)
                .expect("queued room change protocol lines should decode")
            {
                ProtocolMessage::Set(set_message) => Some(set_message),
                _ => None,
            }
        })
        .expect("queued room change protocol lines should include a Set message");
    assert_eq!(
        set_message.set.room.as_ref().map(|room| room.name.as_str()),
        Some("room2")
    );

    adapter.prepare_transport_reconnect();
    let reconnect_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("reconnect protocol lines should encode");
    let hello = reconnect_lines
        .iter()
        .find_map(|line| {
            match decode_message_line(line).expect("reconnect protocol lines should decode") {
                ProtocolMessage::Hello(hello) => Some(hello),
                _ => None,
            }
        })
        .expect("reconnect protocol lines should include a Hello message");
    assert_eq!(hello.hello.room.name, "room2");
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_reconnect_hello_follows_server_authoritative_room_after_mismatched_room_response()
 {
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let _ = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("inbound server hello should apply");

    GuiSessionRuntimeAdapter::set_room(&mut adapter, "room2".to_owned())
        .expect("room change should queue");
    let _ = adapter
        .flush_outbound_protocol_lines()
        .expect("queued room change should encode");

    adapter
        .apply_message_json(r#"{"Set":{"room":{"name":"room3"}}}"#)
        .expect("authoritative room response should apply");

    adapter.prepare_transport_reconnect();
    let reconnect_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("reconnect protocol lines should encode");
    let hello = reconnect_lines
        .iter()
        .find_map(|line| {
            match decode_message_line(line).expect("reconnect protocol lines should decode") {
                ProtocolMessage::Hello(hello) => Some(hello),
                _ => None,
            }
        })
        .expect("reconnect protocol lines should include a Hello message");
    assert_eq!(
        hello.hello.room.name, "room3",
        "future reconnects should follow the server-authoritative room once a room response arrives"
    );
}
