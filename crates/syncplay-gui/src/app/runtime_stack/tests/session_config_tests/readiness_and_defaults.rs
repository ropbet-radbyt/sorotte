use super::*;

#[test]
fn gui_client_core_chat_session_runtime_adapter_preserves_ready_at_start_across_reconnect_before_first_hello()
 {
    let runtime_settings =
        stored_client_settings_runtime_snapshot_legacy_compatible(&StoredClientSettingsMvp {
            username: Some("alice".to_owned()),
            room: Some("room1".to_owned()),
            ready_at_start: Some(true),
            ..StoredClientSettingsMvp::default()
        });
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    GuiSessionRuntimeAdapter::sync_runtime_settings(&mut adapter, &runtime_settings)
        .expect("runtime settings should sync into the session");
    let _ = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");

    adapter.prepare_transport_reconnect();
    let reconnect_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("reconnect protocol lines should encode");
    assert_eq!(reconnect_lines.len(), 1);

    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#,
        )
        .expect("reconnect hello should apply");
    let outbound_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("ready-at-start lines should encode after reconnect hello");
    assert!(
        outbound_lines
            .iter()
            .any(|line| line.contains(r#""Set":{"ready":{"isReady":true"#)),
        "pre-Hello reconnects should preserve the ready-at-start dispatch after the eventual server hello"
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_preserves_ready_at_start_across_reconnect_after_hello_before_ready_echo()
 {
    let runtime_settings =
        stored_client_settings_runtime_snapshot_legacy_compatible(&StoredClientSettingsMvp {
            username: Some("alice".to_owned()),
            room: Some("room1".to_owned()),
            ready_at_start: Some(true),
            ..StoredClientSettingsMvp::default()
        });
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    GuiSessionRuntimeAdapter::sync_runtime_settings(&mut adapter, &runtime_settings)
        .expect("runtime settings should sync into the session");
    let _ = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");

    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#,
        )
        .expect("server hello should apply");
    let outbound_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("ready-at-start lines should encode after the first hello");
    assert!(
        outbound_lines
            .iter()
            .any(|line| line.contains(r#""Set":{"ready":{"isReady":true"#)),
        "the first hello should queue ready-at-start"
    );

    adapter.prepare_transport_reconnect();
    let reconnect_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("reconnect protocol lines should encode");
    assert_eq!(reconnect_lines.len(), 1);

    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#,
        )
        .expect("reconnect hello should apply");
    let reconnect_outbound_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("ready-at-start lines should encode after reconnect hello");
    assert!(
        reconnect_outbound_lines
            .iter()
            .any(|line| line.contains(r#""Set":{"ready":{"isReady":true"#)),
        "post-Hello reconnects should preserve ready-at-start until the ready update round-trips"
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_reconnect_hello_preserves_whitespace_room_names() {
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let _ = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"   "},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("inbound server hello should apply");

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
    assert_eq!(hello.hello.room.name, "   ");
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_clears_text_backed_runtime_settings_to_defaults() {
    let configured_settings =
        stored_client_settings_runtime_snapshot_legacy_compatible(&StoredClientSettingsMvp {
            username: Some("alice".to_owned()),
            room: Some("room1".to_owned()),
            trusted_domains: Some(vec!["*.example.com/videos".to_owned()]),
            rewind_threshold_seconds: Some(1.5),
            fastforward_threshold_seconds: Some(4.0),
            slowdown_threshold_seconds: Some(0.75),
            unpause_action: Some(UnpauseActionMode::IfMinUsersReady),
            autoplay_min_users: Some(AutoplayThresholdOverride::Set(3)),
            show_duration_notification: Some(false),
            ..StoredClientSettingsMvp::default()
        });
    let cleared_settings =
        stored_client_settings_runtime_snapshot_legacy_compatible(&StoredClientSettingsMvp {
            username: Some("alice".to_owned()),
            room: Some("room1".to_owned()),
            ..StoredClientSettingsMvp::default()
        });
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    GuiSessionRuntimeAdapter::sync_runtime_settings(&mut adapter, &configured_settings)
        .expect("configured runtime settings should sync");
    GuiSessionRuntimeAdapter::sync_runtime_settings(&mut adapter, &cleared_settings)
        .expect("cleared runtime settings should sync");

    assert_eq!(
        adapter.runtime.session().behavior_config().trusted_domains,
        SessionBehaviorConfig::default().trusted_domains
    );
    assert_eq!(
        adapter
            .runtime
            .session()
            .desync_config()
            .rewind_threshold_seconds,
        DesyncCorrectionConfig::default().rewind_threshold_seconds
    );
    assert_eq!(
        adapter
            .runtime
            .session()
            .desync_config()
            .fastforward_threshold_seconds,
        DesyncCorrectionConfig::default().fastforward_threshold_seconds
    );
    assert_eq!(
        adapter
            .runtime
            .session()
            .desync_config()
            .slowdown_threshold_seconds,
        DesyncCorrectionConfig::default().slowdown_threshold_seconds
    );
    assert_eq!(
        adapter
            .runtime
            .session()
            .readiness_autoplay_config()
            .unpause_action,
        ReadinessAutoplayConfig::default().unpause_action
    );
    assert_eq!(
        adapter
            .runtime
            .session()
            .readiness_autoplay_config()
            .auto_play_threshold,
        ReadinessAutoplayConfig::default().auto_play_threshold
    );
    assert_eq!(
        adapter
            .runtime
            .session()
            .readiness_autoplay_config()
            .show_duration_notification,
        ReadinessAutoplayConfig::default().show_duration_notification
    );
}
