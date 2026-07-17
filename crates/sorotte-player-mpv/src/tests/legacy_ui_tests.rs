use super::*;

fn settings_without_osd_move() -> LegacySyncplayUiSettings {
    LegacySyncplayUiSettings {
        chat_move_osd: false,
        ..LegacySyncplayUiSettings::default()
    }
}

fn parsed_writes(state: &FakeTransportStateHandle) -> Vec<Value> {
    state
        .writes()
        .iter()
        .map(|write| serde_json::from_str(write.trim_end()).expect("valid JSON IPC write"))
        .collect()
}

fn options_payload(write: &Value) -> Value {
    serde_json::from_str(
        write["command"][3]
            .as_str()
            .expect("bridge options should be a JSON string"),
    )
    .expect("bridge options should contain valid JSON")
}

fn is_command(write: &Value, command: &str) -> bool {
    write.pointer("/command/0").and_then(Value::as_str) == Some(command)
}

fn wait_for_write_count(state: &FakeTransportStateHandle, expected: usize) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
    while state.writes().len() < expected && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(state.writes().len(), expected);
}

#[test]
fn ready_bridge_can_be_marked_degraded_and_discards_pending_player_chat() {
    let (transport, state) = fake_transport_with_reads(&[
        FAKE_SYNCPLAYINTF_PONG_EVENT,
        r#"{"request_id":1,"error":"success"}"#,
        FAKE_SYNCPLAYINTF_ACK_EVENT,
        r#"{"request_id":2,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter
        .configure_legacy_syncplay_ui_settings(settings_without_osd_move())
        .expect("core mpv UI properties should configure independently");
    assert_eq!(
        adapter.configure_bundled_sorotte_bridge(),
        SorotteBridgeHealth::Ready
    );
    adapter.queue_test_pending_chat_request("stale player chat");

    let health = adapter.mark_sorotte_bridge_degraded(
        SorotteBridgeFailureKind::SettingsRejected,
        "the bridge rejected a runtime update",
    );

    assert_eq!(health, adapter.sorotte_bridge_health());
    let SorotteBridgeHealth::Degraded(failure) = health else {
        panic!("ready bridge should transition to degraded");
    };
    assert_eq!(failure.kind, SorotteBridgeFailureKind::SettingsRejected);
    assert_eq!(failure.reason, "the bridge rejected a runtime update");
    assert!(!adapter.legacy_syncplayintf_options_ready());
    assert_eq!(adapter.take_pending_chat_request(), None);
    assert_eq!(state.writes().len(), 2, "marking degraded must not retry");
    assert!(adapter.is_connected(), "core mpv IPC must remain healthy");
}

#[cfg(feature = "test-support")]
#[test]
fn undiscoverable_test_bridge_degrades_while_core_ipc_stays_connected() {
    let mut adapter =
        MpvAdapter::with_undiscoverable_sorotte_bridge_test_ipc(settings_without_osd_move());
    assert!(adapter.is_connected());

    let health = adapter.configure_bundled_sorotte_bridge();

    assert!(matches!(
        health,
        SorotteBridgeHealth::Degraded(failure)
            if failure.kind == SorotteBridgeFailureKind::Discovery
    ));
    assert!(adapter.is_connected());
}

#[cfg(feature = "test-support")]
#[test]
fn test_support_can_turn_an_initially_connected_ipc_unhealthy() {
    let mut adapter =
        MpvAdapter::with_unacknowledging_syncplayintf_test_ipc(settings_without_osd_move());
    assert!(adapter.is_connected());

    adapter.mark_test_ipc_unhealthy("simulated core IPC loss during bridge setup");

    assert!(!adapter.is_connected());
    let events = adapter.take_ipc_connection_events();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, crate::MpvIpcConnectionEvent::Disconnected { .. }))
    );
}

#[test]
fn bundled_bridge_degradation_preserves_core_playback_and_retry_after_simulated_release_recovers() {
    let (transport, state) = fake_transport_with_reads(&[
        FAKE_SYNCPLAYINTF_PONG_EVENT,
        r#"{"request_id":1,"error":"success"}"#,
        FAKE_SYNCPLAYINTF_REJECTED_ACK_EVENT,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"request_id":3,"error":"success","data":false}"#,
        r#"{"request_id":4,"error":"success"}"#,
        r#"{"request_id":5,"error":"success"}"#,
        r#"{"request_id":6,"error":"success"}"#,
        FAKE_SYNCPLAYINTF_ACK_EVENT,
        r#"{"request_id":7,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter
        .configure_legacy_syncplay_ui_settings(settings_without_osd_move())
        .expect("core mpv UI properties should configure independently");

    let degraded = adapter.configure_test_bundled_sorotte_bridge_without_retry();
    let SorotteBridgeHealth::Degraded(failure) = degraded else {
        panic!("a competing owner should degrade only the optional bridge");
    };
    assert_eq!(failure.kind, SorotteBridgeFailureKind::LeaseBusy);
    assert!(failure.retryable_in_place());
    assert!(adapter.is_connected());
    assert!(!adapter.legacy_syncplayintf_options_ready());

    let writes_before_polling = state.writes().len();
    assert_eq!(adapter.take_pending_chat_request(), None);
    adapter.force_test_legacy_syncplayintf_heartbeat_due();
    assert_eq!(
        state.writes().len(),
        writes_before_polling,
        "degraded chat polling and heartbeat maintenance must not opportunistically reacquire the lease"
    );

    adapter
        .show_syncplay_legacy_message("bridge degraded", LegacySyncplayOsdKind::Notification)
        .expect("OSD should fall back to core mpv show-text");
    adapter
        .set_paused(true)
        .expect("pause must remain available while the optional bridge is degraded");
    adapter
        .set_position(12.0)
        .unwrap_or_else(|error| {
            panic!(
                "seek must remain available while the optional bridge is degraded: {error}; writes={:?}",
                parsed_writes(&state)
            )
        });

    let health = adapter.retry_bundled_sorotte_bridge();
    assert_eq!(health, SorotteBridgeHealth::Ready);
    assert_eq!(adapter.sorotte_bridge_health(), SorotteBridgeHealth::Ready);
    assert!(adapter.legacy_syncplayintf_options_ready());

    let writes = parsed_writes(&state);
    assert_eq!(
        writes[3]["command"],
        json!(["show-text", "bridge degraded", 3_000, 1])
    );
    assert_eq!(writes[4]["command"], json!(["set_property", "pause", true]));
    assert_eq!(
        writes[5]["command"],
        json!(["set_property", "time-pos", 12.0])
    );
    assert_eq!(writes[6]["command"][2], "set_sorotte_syncplayintf_options");
    assert_eq!(options_payload(&writes[6])["generation"], 2);
}

#[test]
fn graceful_bridge_release_is_terminal_nonblocking_and_idempotent() {
    let (transport, state) = fake_transport_with_reads(&[
        FAKE_SYNCPLAYINTF_PONG_EVENT,
        r#"{"request_id":1,"error":"success"}"#,
        FAKE_SYNCPLAYINTF_ACK_EVENT,
        r#"{"request_id":2,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter
        .configure_legacy_syncplay_ui_settings(settings_without_osd_move())
        .expect("core mpv UI properties should configure independently");
    assert_eq!(
        adapter.configure_bundled_sorotte_bridge(),
        SorotteBridgeHealth::Ready
    );

    let started = std::time::Instant::now();
    adapter.release_sorotte_bridge_best_effort();
    assert!(started.elapsed() < std::time::Duration::from_millis(250));
    wait_for_write_count(&state, 3);
    assert_eq!(
        adapter.sorotte_bridge_health(),
        SorotteBridgeHealth::Disabled
    );
    assert!(!adapter.legacy_syncplayintf_options_ready());
    assert!(
        !adapter.is_connected(),
        "a final-write client must not be reused"
    );

    let writes = parsed_writes(&state);
    assert_eq!(writes[2]["command"][0], "script-message-to");
    assert_eq!(writes[2]["command"][1], LEGACY_SYNCPLAYINTF_SCRIPT_NAME);
    assert_eq!(writes[2]["command"][2], "sorotte_syncplayintf_release");
    let release_payload: Value = serde_json::from_str(writes[2]["command"][3].as_str().unwrap())
        .expect("release payload should be structured JSON");
    assert_eq!(release_payload["protocol"], LEGACY_SYNCPLAYINTF_PROTOCOL);
    assert_eq!(release_payload["bridgeInstanceId"], "test-bridge");

    adapter.release_sorotte_bridge_best_effort();
    assert_eq!(state.writes().len(), 3, "release should be one-shot");
}

#[test]
fn disabled_bridge_configuration_never_disturbs_core_ipc() {
    let (transport, state) = fake_transport_with_reads(&[]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    let settings = LegacySyncplayUiSettings {
        chat_input_enabled: false,
        chat_output_enabled: false,
        chat_move_osd: false,
        ..LegacySyncplayUiSettings::default()
    };
    adapter
        .configure_legacy_syncplay_ui_settings(settings)
        .expect("disabled optional integration should not affect core settings");

    assert_eq!(
        adapter.configure_bundled_sorotte_bridge(),
        SorotteBridgeHealth::Disabled
    );
    assert!(adapter.is_connected());
    assert!(state.writes().is_empty());
}

#[test]
fn missing_legacy_bridge_loads_stable_resource_then_discovers_and_configures_it() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"error running command"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        FAKE_SYNCPLAYINTF_PONG_EVENT,
        r#"{"request_id":3,"error":"success"}"#,
        FAKE_SYNCPLAYINTF_ACK_EVENT,
        r#"{"request_id":4,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter
        .configure_legacy_syncplay_ui_settings(settings_without_osd_move())
        .expect("settings should remain pending until discovery");

    let resource_path = std::path::Path::new("C:/sorotte/resources/sorotte_syncplayintf.lua");
    adapter
        .load_legacy_syncplayintf_script(resource_path)
        .expect("a missing stable target should be loaded and discovered");

    assert!(adapter.legacy_syncplayintf_script_loaded());
    assert!(adapter.legacy_syncplayintf_options_ready());
    let writes = parsed_writes(&state);
    assert_eq!(writes.len(), 4);
    assert_eq!(
        writes[0]["command"],
        json!([
            "script-message-to",
            LEGACY_SYNCPLAYINTF_SCRIPT_NAME,
            "sorotte_syncplayintf_ping",
            writes[0]["command"][3]
        ])
    );
    assert_eq!(
        writes[1],
        json!({
            "command": ["load-script", resource_path.to_string_lossy()],
            "request_id": 2,
        })
    );
    assert_eq!(writes[2]["command"][1], LEGACY_SYNCPLAYINTF_SCRIPT_NAME);
    assert_eq!(writes[2]["command"][2], "sorotte_syncplayintf_ping");
    assert_eq!(writes[3]["command"][1], LEGACY_SYNCPLAYINTF_SCRIPT_NAME);
    assert_eq!(writes[3]["command"][2], "set_sorotte_syncplayintf_options");
}

#[test]
fn newly_loaded_bridge_gets_a_bounded_registration_window_before_failure() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"error running command"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"request_id":3,"error":"error running command"}"#,
        r#"{"request_id":4,"error":"error running command"}"#,
        FAKE_SYNCPLAYINTF_PONG_EVENT,
        r#"{"request_id":5,"error":"success"}"#,
        FAKE_SYNCPLAYINTF_ACK_EVENT,
        r#"{"request_id":6,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter
        .configure_legacy_syncplay_ui_settings(settings_without_osd_move())
        .expect("settings should remain pending until discovery");

    adapter
        .load_legacy_syncplayintf_script(std::path::Path::new(
            "C:/sorotte/resources/sorotte_syncplayintf.lua",
        ))
        .expect("post-load discovery should tolerate delayed script-message registration");

    assert!(adapter.legacy_syncplayintf_options_ready());
    let writes = parsed_writes(&state);
    assert_eq!(writes.len(), 6);
    assert_eq!(
        writes
            .iter()
            .filter(|write| is_command(write, "load-script"))
            .count(),
        1
    );
    assert_eq!(
        writes
            .iter()
            .filter(|write| {
                write.pointer("/command/2").and_then(Value::as_str)
                    == Some("sorotte_syncplayintf_ping")
            })
            .count(),
        4
    );
}

#[test]
fn existing_legacy_bridge_is_pinged_and_reused_without_loading_a_duplicate() {
    let (transport, state) = fake_transport_with_reads(&[
        FAKE_SYNCPLAYINTF_PONG_EVENT,
        r#"{"request_id":1,"error":"success"}"#,
        FAKE_SYNCPLAYINTF_ACK_EVENT,
        r#"{"request_id":2,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter
        .configure_legacy_syncplay_ui_settings(settings_without_osd_move())
        .expect("settings should remain pending until discovery");

    adapter
        .load_legacy_syncplayintf_script(std::path::Path::new(
            "C:/sorotte/resources/sorotte_syncplayintf.lua",
        ))
        .expect("the existing stable bridge should be reused");

    assert!(adapter.legacy_syncplayintf_options_ready());
    let writes = parsed_writes(&state);
    assert_eq!(writes.len(), 2);
    assert!(writes.iter().all(|write| !is_command(write, "load-script")));
    assert_eq!(writes[0]["command"][1], LEGACY_SYNCPLAYINTF_SCRIPT_NAME);
    assert_eq!(writes[0]["command"][2], "sorotte_syncplayintf_ping");
    assert_eq!(writes[1]["command"][1], LEGACY_SYNCPLAYINTF_SCRIPT_NAME);
    assert_eq!(writes[1]["command"][2], "set_sorotte_syncplayintf_options");
}

#[test]
fn expired_input_lease_is_reacquired_with_a_fresh_acknowledged_generation() {
    let (transport, state) = fake_transport_with_reads(&[
        FAKE_SYNCPLAYINTF_PONG_EVENT,
        r#"{"request_id":1,"error":"success"}"#,
        FAKE_SYNCPLAYINTF_ACK_EVENT,
        r#"{"request_id":2,"error":"success"}"#,
        FAKE_SYNCPLAYINTF_LEASE_EXPIRED_EVENT,
        r#"{"request_id":3,"error":"success"}"#,
        FAKE_SYNCPLAYINTF_ACK_EVENT,
        r#"{"request_id":4,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter
        .configure_legacy_syncplay_ui_settings(settings_without_osd_move())
        .expect("settings should remain pending until discovery");
    adapter
        .load_legacy_syncplayintf_script(std::path::Path::new(
            "C:/sorotte/resources/sorotte_syncplayintf.lua",
        ))
        .expect("initial bridge configuration should succeed");
    assert!(adapter.legacy_syncplayintf_options_ready());

    adapter.force_test_legacy_syncplayintf_heartbeat_due();

    assert!(
        adapter.legacy_syncplayintf_options_ready(),
        "a lease-expired event observed during heartbeat must trigger a fresh exact options acknowledgement"
    );
    let writes = parsed_writes(&state);
    assert_eq!(writes.len(), 4);
    assert_eq!(writes[2]["command"][2], "sorotte_syncplayintf_heartbeat");
    let initial = options_payload(&writes[1]);
    let reacquired = options_payload(&writes[3]);
    assert_eq!(initial["generation"], 1);
    assert_eq!(reacquired["generation"], 2);
    assert_eq!(reacquired["ownerId"], initial["ownerId"]);
    assert_eq!(reacquired["attachmentId"], initial["attachmentId"]);
}

#[test]
fn busy_lease_reacquisition_is_not_retried_by_repeated_chat_polls() {
    let (transport, state) = fake_transport_with_reads(&[
        FAKE_SYNCPLAYINTF_PONG_EVENT,
        r#"{"request_id":1,"error":"success"}"#,
        FAKE_SYNCPLAYINTF_ACK_EVENT,
        r#"{"request_id":2,"error":"success"}"#,
        FAKE_SYNCPLAYINTF_LEASE_EXPIRED_EVENT,
        r#"{"request_id":3,"error":"success"}"#,
        FAKE_SYNCPLAYINTF_REJECTED_ACK_EVENT,
        r#"{"request_id":4,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter
        .configure_legacy_syncplay_ui_settings(settings_without_osd_move())
        .expect("settings should remain pending until discovery");
    adapter
        .load_legacy_syncplayintf_script(std::path::Path::new(
            "C:/sorotte/resources/sorotte_syncplayintf.lua",
        ))
        .expect("initial bridge configuration should succeed");
    assert!(adapter.legacy_syncplayintf_options_ready());

    adapter.force_test_legacy_syncplayintf_heartbeat_due();

    assert!(
        !adapter.legacy_syncplayintf_options_ready(),
        "a competing owner must keep the expired lease pending"
    );
    let option_write_count_before_polls = parsed_writes(&state)
        .iter()
        .filter(|write| {
            write.pointer("/command/2").and_then(Value::as_str)
                == Some("set_sorotte_syncplayintf_options")
        })
        .count();
    assert_eq!(option_write_count_before_polls, 2);

    for _ in 0..5 {
        assert_eq!(adapter.take_pending_chat_request(), None);
    }

    let option_write_count_after_polls = parsed_writes(&state)
        .iter()
        .filter(|write| {
            write.pointer("/command/2").and_then(Value::as_str)
                == Some("set_sorotte_syncplayintf_options")
        })
        .count();
    assert_eq!(
        option_write_count_after_polls, option_write_count_before_polls,
        "chat polling must leave busy lease retries to the throttled heartbeat maintainer"
    );
}

#[test]
fn output_only_busy_guard_survives_maintenance_and_repeated_chat_polls() {
    let (transport, state) = fake_transport_with_reads(&[
        FAKE_SYNCPLAYINTF_PONG_EVENT,
        r#"{"request_id":1,"error":"success"}"#,
        FAKE_SYNCPLAYINTF_REJECTED_ACK_EVENT,
        r#"{"request_id":2,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    let mut settings = settings_without_osd_move();
    settings.chat_input_enabled = false;
    settings.chat_output_enabled = true;
    adapter
        .configure_legacy_syncplay_ui_settings(settings)
        .expect("output-only settings should remain pending until discovery");
    adapter
        .load_legacy_syncplayintf_script(std::path::Path::new(
            "C:/sorotte/resources/sorotte_syncplayintf.lua",
        ))
        .expect("the existing bridge should be discovered");

    assert!(!adapter.legacy_syncplayintf_options_ready());
    let option_write_count_before_polls = parsed_writes(&state)
        .iter()
        .filter(|write| {
            write.pointer("/command/2").and_then(Value::as_str)
                == Some("set_sorotte_syncplayintf_options")
        })
        .count();
    assert_eq!(option_write_count_before_polls, 1);

    adapter.force_test_legacy_syncplayintf_heartbeat_due();
    for _ in 0..5 {
        assert_eq!(adapter.take_pending_chat_request(), None);
    }

    let option_write_count_after_polls = parsed_writes(&state)
        .iter()
        .filter(|write| {
            write.pointer("/command/2").and_then(Value::as_str)
                == Some("set_sorotte_syncplayintf_options")
        })
        .count();
    assert_eq!(
        option_write_count_after_polls, option_write_count_before_polls,
        "disabled input maintenance must retain the busy guard against opportunistic retries"
    );
}

#[test]
fn legacy_bridge_uses_typed_structured_settings_and_requires_an_exact_ack() {
    let (transport, state) = fake_transport_with_reads(&[
        FAKE_SYNCPLAYINTF_PONG_EVENT,
        r#"{"request_id":1,"error":"success"}"#,
        FAKE_SYNCPLAYINTF_ACK_EVENT,
        r#"{"request_id":2,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter
        .configure_legacy_syncplay_ui_settings(LegacySyncplayUiSettings {
            chat_input_enabled: true,
            chat_input_font_underline: true,
            chat_input_font_family: "serif".to_owned(),
            chat_input_relative_font_size: 18,
            chat_input_font_weight: 50,
            chat_input_font_color: "#abcdef".to_owned(),
            chat_input_position: "Bottom".to_owned(),
            chat_direct_input: true,
            chat_output_enabled: false,
            chat_output_font_underline: true,
            chat_output_font_family: "monospace".to_owned(),
            chat_output_relative_font_size: 30,
            chat_output_font_weight: 75,
            chat_output_mode: "Scrolling".to_owned(),
            chat_max_lines: 9,
            chat_top_margin: 40,
            chat_left_margin: 35,
            chat_bottom_margin: 45,
            chat_move_osd: false,
            notification_timeout_ms: 4_000,
            alert_timeout_ms: 6_000,
            chat_timeout_ms: 8_000,
            ..LegacySyncplayUiSettings::default()
        })
        .expect("settings should remain pending until discovery");

    adapter
        .load_legacy_syncplayintf_script(std::path::Path::new(
            "C:/sorotte/resources/sorotte_syncplayintf.lua",
        ))
        .expect("exact structured acknowledgement should complete configuration");

    assert!(adapter.legacy_syncplayintf_options_ready());
    let writes = parsed_writes(&state);
    let payload = options_payload(&writes[1]);
    assert_eq!(payload["protocol"], LEGACY_SYNCPLAYINTF_PROTOCOL);
    assert_eq!(payload["bridgeInstanceId"], "test-bridge");
    assert!(payload["ownerId"].is_string());
    assert!(payload["attachmentId"].is_string());
    assert_eq!(payload["generation"], 1);
    assert_eq!(payload["leaseMs"], 2_000);
    assert_eq!(
        payload["settings"],
        json!({
            "chatInputEnabled": true,
            "chatInputFontFamily": "serif",
            "chatInputRelativeFontSize": 18,
            "chatInputFontWeight": 50,
            "chatInputFontUnderline": true,
            "chatInputFontColor": "#abcdef",
            "chatInputPosition": "Bottom",
            "chatOutputFontFamily": "monospace",
            "chatOutputRelativeFontSize": 30,
            "chatOutputFontWeight": 75,
            "chatOutputFontUnderline": true,
            "chatOutputMode": "Scrolling",
            "chatMaxLines": 9,
            "chatTopMargin": 40,
            "chatLeftMargin": 35,
            "chatBottomMargin": 45,
            "chatDirectInput": true,
            "notificationTimeout": 4.0,
            "alertTimeout": 6.0,
            "chatTimeout": 8.0,
            "chatOutputEnabled": false,
        })
    );
}

#[test]
fn pending_legacy_settings_retry_reuses_the_same_generation_until_acknowledged() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"error running command"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        FAKE_SYNCPLAYINTF_PONG_EVENT,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"request_id":4,"error":"success"}"#,
        r#"{"request_id":5,"error":"success","data":false}"#,
        FAKE_SYNCPLAYINTF_ACK_EVENT,
        r#"{"request_id":6,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter
        .configure_legacy_syncplay_ui_settings(settings_without_osd_move())
        .expect("settings should remain pending until discovery");

    adapter
        .load_legacy_syncplayintf_script(std::path::Path::new(
            "C:/sorotte/resources/sorotte_syncplayintf.lua",
        ))
        .expect("loading should tolerate an acknowledgement timing race");
    assert!(!adapter.legacy_syncplayintf_options_ready());

    adapter
        .apply_pending_legacy_syncplayintf_options()
        .expect("retry should accept the exact acknowledgement");
    assert!(adapter.legacy_syncplayintf_options_ready());

    let writes = parsed_writes(&state);
    let option_writes: Vec<_> = writes
        .iter()
        .filter(|write| {
            write.pointer("/command/2").and_then(Value::as_str)
                == Some("set_sorotte_syncplayintf_options")
        })
        .collect();
    assert_eq!(option_writes.len(), 2);
    let first = options_payload(option_writes[0]);
    let second = options_payload(option_writes[1]);
    assert_eq!(first["generation"], 1);
    assert_eq!(second["generation"], first["generation"]);
    assert_eq!(second["ownerId"], first["ownerId"]);
    assert_eq!(second["attachmentId"], first["attachmentId"]);
}

#[test]
fn stale_malformed_future_and_rejected_legacy_acks_never_set_readiness() {
    for (label, ack_marker) in [
        ("stale", FAKE_SYNCPLAYINTF_STALE_ACK_EVENT),
        ("malformed", FAKE_SYNCPLAYINTF_MALFORMED_ACK_EVENT),
        ("future", FAKE_SYNCPLAYINTF_FUTURE_ACK_EVENT),
        ("rejected", FAKE_SYNCPLAYINTF_REJECTED_ACK_EVENT),
    ] {
        let (transport, state) = fake_transport_with_reads(&[
            FAKE_SYNCPLAYINTF_PONG_EVENT,
            r#"{"request_id":1,"error":"success"}"#,
            ack_marker,
            r#"{"request_id":2,"error":"success"}"#,
            r#"{"request_id":3,"error":"success","data":false}"#,
        ]);
        let mut adapter = MpvAdapter::with_test_transport(transport);
        adapter
            .configure_legacy_syncplay_ui_settings(settings_without_osd_move())
            .expect("settings should remain pending until discovery");

        adapter
            .load_legacy_syncplayintf_script(std::path::Path::new(
                "C:/sorotte/resources/sorotte_syncplayintf.lua",
            ))
            .unwrap_or_else(|error| {
                panic!("{label} acknowledgement case should remain retryable: {error}")
            });

        assert!(
            !adapter.legacy_syncplayintf_options_ready(),
            "{label} acknowledgement must not make the bridge ready"
        );
        assert!(
            parsed_writes(&state)
                .iter()
                .all(|write| !is_command(write, "load-script")),
            "{label} acknowledgement must not cause a duplicate load"
        );
    }
}

#[test]
fn stable_target_accepting_ping_without_valid_pong_refuses_duplicate_load() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success","data":false}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"request_id":4,"error":"success","data":false}"#,
        r#"{"request_id":5,"error":"success"}"#,
        r#"{"request_id":6,"error":"success","data":false}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    let error = adapter
        .load_legacy_syncplayintf_script(std::path::Path::new(
            "C:/sorotte/resources/sorotte_syncplayintf.lua",
        ))
        .expect_err("a target that never proves its identity must not be duplicated");

    assert!(error.to_string().contains("did not return a valid pong"));
    let writes = parsed_writes(&state);
    assert_eq!(writes.len(), 6);
    assert!(writes.iter().all(|write| !is_command(write, "load-script")));
    assert_eq!(
        writes
            .iter()
            .filter(|write| {
                write.pointer("/command/2").and_then(Value::as_str)
                    == Some("sorotte_syncplayintf_ping")
            })
            .count(),
        3
    );
}

#[test]
fn explicit_ipc_reattach_rediscovers_bridge_without_transferring_script_identity() {
    let (first_transport, first_state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"error running command"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        FAKE_SYNCPLAYINTF_PONG_EVENT,
        r#"{"request_id":3,"error":"success"}"#,
        FAKE_SYNCPLAYINTF_ACK_EVENT,
        r#"{"request_id":4,"error":"success"}"#,
    ]);
    let mut first_adapter = MpvAdapter::with_test_transport(first_transport);
    first_adapter
        .configure_legacy_syncplay_ui_settings(settings_without_osd_move())
        .expect("first attachment settings should remain pending");
    first_adapter
        .load_legacy_syncplayintf_script(std::path::Path::new(
            "C:/sorotte/resources/sorotte_syncplayintf.lua",
        ))
        .expect("first attachment should load the resource");
    let first_options = parsed_writes(&first_state);
    let first_options = options_payload(
        first_options
            .iter()
            .find(|write| {
                write.pointer("/command/2").and_then(Value::as_str)
                    == Some("set_sorotte_syncplayintf_options")
            })
            .expect("first attachment options"),
    );

    let (second_transport, second_state) = fake_transport_with_reads(&[
        FAKE_SYNCPLAYINTF_PONG_EVENT,
        r#"{"request_id":1,"error":"success"}"#,
        FAKE_SYNCPLAYINTF_ACK_EVENT,
        r#"{"request_id":2,"error":"success"}"#,
    ]);
    let mut second_adapter = MpvAdapter::with_test_transport(second_transport);
    second_adapter
        .configure_legacy_syncplay_ui_settings(settings_without_osd_move())
        .expect("reattached settings should remain pending");
    second_adapter
        .load_legacy_syncplayintf_script(std::path::Path::new(
            "C:/sorotte/resources/sorotte_syncplayintf.lua",
        ))
        .expect("reattach should rediscover the existing stable target");

    assert!(second_adapter.legacy_syncplayintf_options_ready());
    let second_writes = parsed_writes(&second_state);
    assert_eq!(second_writes.len(), 2);
    assert!(
        second_writes
            .iter()
            .all(|write| !is_command(write, "load-script"))
    );
    let second_options = options_payload(&second_writes[1]);
    assert_eq!(second_options["bridgeInstanceId"], "test-bridge");
    assert_eq!(second_options["ownerId"], first_options["ownerId"]);
    assert_ne!(
        second_options["attachmentId"], first_options["attachmentId"],
        "a fresh IPC attachment must get its own lease identity"
    );
}

#[test]
fn legacy_notification_and_chat_messages_target_the_stable_script_name() {
    let (transport, state) = fake_transport_with_reads(&[
        FAKE_SYNCPLAYINTF_PONG_EVENT,
        r#"{"request_id":1,"error":"success"}"#,
        FAKE_SYNCPLAYINTF_ACK_EVENT,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"request_id":4,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter
        .configure_legacy_syncplay_ui_settings(settings_without_osd_move())
        .expect("settings should remain pending until discovery");
    adapter
        .load_legacy_syncplayintf_script(std::path::Path::new(
            "C:/sorotte/resources/sorotte_syncplayintf.lua",
        ))
        .expect("existing bridge should be discovered and configured");

    adapter
        .show_syncplay_legacy_message("room updated", LegacySyncplayOsdKind::Notification)
        .expect("notification should use the bridge");
    adapter
        .show_syncplay_legacy_chat_message("<alice> hi")
        .expect("chat should use the bridge");

    let writes = parsed_writes(&state);
    assert_eq!(
        writes[2],
        json!({
            "command": [
                "script-message-to",
                LEGACY_SYNCPLAYINTF_SCRIPT_NAME,
                "notification-osd-neutral",
                "room updated"
            ],
            "request_id": 3,
        })
    );
    assert_eq!(
        writes[3],
        json!({
            "command": [
                "script-message-to",
                LEGACY_SYNCPLAYINTF_SCRIPT_NAME,
                "chat",
                "<alice> hi"
            ],
            "request_id": 4,
        })
    );
}

#[test]
fn legacy_osd_falls_back_to_show_text_while_bridge_ack_is_pending() {
    let (transport, state) = fake_transport_with_reads(&[
        FAKE_SYNCPLAYINTF_PONG_EVENT,
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"request_id":3,"error":"success","data":false}"#,
        r#"{"request_id":4,"error":"success"}"#,
        r#"{"request_id":5,"error":"success","data":false}"#,
        r#"{"request_id":6,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter
        .configure_legacy_syncplay_ui_settings(settings_without_osd_move())
        .expect("settings should remain pending until discovery");
    adapter
        .load_legacy_syncplayintf_script(std::path::Path::new(
            "C:/sorotte/resources/sorotte_syncplayintf.lua",
        ))
        .expect("bridge discovery should tolerate a pending acknowledgement");

    adapter
        .show_syncplay_legacy_message("room updated", LegacySyncplayOsdKind::Notification)
        .expect("pending bridge should fall back to show-text");

    let writes = parsed_writes(&state);
    assert_eq!(writes.len(), 6);
    assert_eq!(
        writes[5],
        json!({
            "command": ["show-text", "room updated", 3_000, 1],
            "request_id": 6,
        })
    );
}

#[test]
fn configure_legacy_syncplay_ui_settings_applies_osd_position_when_needed() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success","data":"top"}"#,
        r#"{"request_id":2,"error":"success","data":16}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"request_id":4,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .configure_legacy_syncplay_ui_settings(LegacySyncplayUiSettings::default())
        .expect("legacy settings application should succeed");

    let writes = parsed_writes(&state);
    assert_eq!(writes.len(), 4);
    assert_eq!(
        writes[0],
        json!({"command": ["get_property", "osd-align-y"], "request_id": 1})
    );
    assert_eq!(
        writes[1],
        json!({"command": ["get_property", "osd-margin-y"], "request_id": 2})
    );
    assert_eq!(
        writes[2],
        json!({"command": ["set_property", "osd-align-y", "bottom"], "request_id": 3})
    );
    assert_eq!(
        writes[3],
        json!({"command": ["set_property", "osd-margin-y", 110], "request_id": 4})
    );
}

#[test]
fn configure_legacy_syncplay_ui_settings_restores_osd_position_when_move_is_disabled() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success","data":"top"}"#,
        r#"{"request_id":2,"error":"success","data":16}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"request_id":4,"error":"success"}"#,
        r#"{"request_id":5,"error":"success"}"#,
        r#"{"request_id":6,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter
        .configure_legacy_syncplay_ui_settings(LegacySyncplayUiSettings::default())
        .expect("enabling OSD movement should succeed");
    adapter
        .configure_legacy_syncplay_ui_settings(settings_without_osd_move())
        .expect("disabling OSD movement should restore captured placement");

    let writes = parsed_writes(&state);
    assert_eq!(
        writes[4],
        json!({"command": ["set_property", "osd-align-y", "top"], "request_id": 5})
    );
    assert_eq!(
        writes[5],
        json!({"command": ["set_property", "osd-margin-y", 16], "request_id": 6})
    );
    assert!(adapter.legacy_syncplay_osd_placement_restore().is_none());
}

#[test]
fn transferred_osd_restore_state_survives_an_explicit_ipc_reattach() {
    let (first_transport, _) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success","data":"top"}"#,
        r#"{"request_id":2,"error":"success","data":16}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"request_id":4,"error":"success"}"#,
    ]);
    let mut first_adapter = MpvAdapter::with_test_transport(first_transport);
    first_adapter
        .configure_legacy_syncplay_ui_settings(LegacySyncplayUiSettings::default())
        .expect("the first adapter should capture and move OSD placement");
    let restore = first_adapter
        .legacy_syncplay_osd_placement_restore()
        .expect("pre-Sorotte OSD placement should remain transferable");

    let (reattached_transport, reattached_state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success"}"#,
    ]);
    let mut reattached_adapter = MpvAdapter::with_test_transport(reattached_transport);
    reattached_adapter.set_legacy_syncplay_osd_placement_restore(Some(restore));
    reattached_adapter
        .configure_legacy_syncplay_ui_settings(settings_without_osd_move())
        .expect("reattached adapter should restore pre-Sorotte placement");

    let writes = parsed_writes(&reattached_state);
    assert_eq!(writes.len(), 2);
    assert_eq!(
        writes[0]["command"],
        json!(["set_property", "osd-align-y", "top"])
    );
    assert_eq!(
        writes[1]["command"],
        json!(["set_property", "osd-margin-y", 16])
    );
}

#[test]
fn configure_legacy_syncplay_ui_settings_skips_osd_position_when_disabled() {
    let (transport, state) = fake_transport_with_reads(&[]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .configure_legacy_syncplay_ui_settings(settings_without_osd_move())
        .expect("legacy settings application should succeed");

    assert!(state.writes().is_empty());
}

#[test]
fn show_syncplay_legacy_message_uses_notification_timeout_for_show_text() {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter
        .configure_legacy_syncplay_ui_settings(LegacySyncplayUiSettings {
            chat_output_enabled: false,
            chat_move_osd: false,
            notification_timeout_ms: 4_500,
            ..LegacySyncplayUiSettings::default()
        })
        .expect("legacy settings application should succeed");

    adapter
        .show_syncplay_legacy_message("room updated", LegacySyncplayOsdKind::Notification)
        .expect("show-text notification should succeed");

    assert_eq!(
        parsed_writes(&state)[0],
        json!({"command": ["show-text", "room updated", 4_500, 1], "request_id": 1})
    );
}

#[test]
fn show_syncplay_legacy_message_uses_alert_timeout_for_show_text() {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter
        .configure_legacy_syncplay_ui_settings(LegacySyncplayUiSettings {
            chat_output_enabled: false,
            chat_move_osd: false,
            alert_timeout_ms: 6_000,
            ..LegacySyncplayUiSettings::default()
        })
        .expect("legacy settings application should succeed");

    adapter
        .show_syncplay_legacy_message("autoplay", LegacySyncplayOsdKind::Alert)
        .expect("show-text alert should succeed");

    assert_eq!(
        parsed_writes(&state)[0],
        json!({"command": ["show-text", "autoplay", 6_000, 1], "request_id": 1})
    );
}

#[test]
fn show_syncplay_legacy_chat_message_uses_chat_timeout_when_bridge_is_unavailable() {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter
        .configure_legacy_syncplay_ui_settings(LegacySyncplayUiSettings {
            show_osd: false,
            chat_move_osd: false,
            chat_timeout_ms: 8_000,
            ..LegacySyncplayUiSettings::default()
        })
        .expect("legacy settings application should succeed");

    adapter
        .show_syncplay_legacy_chat_message("<alice> hi")
        .expect("chat show-text should succeed");

    assert_eq!(
        parsed_writes(&state)[0],
        json!({"command": ["show-text", "<alice> hi", 8_000, 1], "request_id": 1})
    );
}

#[test]
fn show_syncplay_legacy_chat_message_uses_notification_timeout_when_chat_output_is_disabled() {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter
        .configure_legacy_syncplay_ui_settings(LegacySyncplayUiSettings {
            chat_output_enabled: false,
            chat_move_osd: false,
            notification_timeout_ms: 2_500,
            ..LegacySyncplayUiSettings::default()
        })
        .expect("legacy settings application should succeed");

    adapter
        .show_syncplay_legacy_chat_message("<alice> hi")
        .expect("chat fallback show-text should succeed");

    assert_eq!(
        parsed_writes(&state)[0],
        json!({"command": ["show-text", "<alice> hi", 2_500, 1], "request_id": 1})
    );
}
