use super::*;

#[test]
fn bootstrapped_room_exists() {
    let mut server = ServerApp::new();
    server.bootstrap_room("phase0");
    assert!(server.room_is_present("phase0"));
}

#[test]
fn default_motd_for_client_version_warns_on_outdated_semver() {
    assert_eq!(
        super::default_motd_for_client_version("1.2.255"),
        "You are using Syncplay 1.2.255 but a newer version is available from https://syncplay.pl"
    );
    assert!(super::default_motd_for_client_version("1.7.5").is_empty());
    assert!(super::default_motd_for_client_version("sorotte-dev").is_empty());
}

#[test]
fn motd_for_client_version_uses_template_override_placeholders() {
    assert_eq!(
        super::motd_for_client_version(
            "9.9.9",
            Some("Client={client_version}; Latest={latest_version}; Url={upgrade_url}"),
        ),
        "Client=9.9.9; Latest=1.7.5; Url=https://syncplay.pl"
    );
    assert_eq!(super::motd_for_client_version("1.2.255", Some("   ")), "");
}

#[test]
fn motd_for_client_version_prepends_upgrade_warning_for_outdated_client_with_custom_template() {
    assert_eq!(
        super::motd_for_client_version("1.2.255", Some("Custom latest={latest_version}")),
        "You are using Syncplay 1.2.255 but a newer version is available from https://syncplay.pl\nCustom latest=1.7.5"
    );
}

#[test]
fn motd_for_client_context_supports_python_template_variables() {
    assert_eq!(
        super::motd_for_client_context(
            "1.7.5",
            Some("Server=$version IP=$userIp User=$username Room=$room Cost=$$5"),
            "203.0.113.9",
            "alice",
            "room1",
        ),
        "Server=1.7.5 IP=203.0.113.9 User=alice Room=room1 Cost=$5"
    );
}

#[test]
fn motd_for_client_context_reports_python_template_errors() {
    assert_eq!(
        super::motd_for_client_context("1.7.5", Some("Bad $ placeholder"), "", "alice", "room1"),
        "Message of the Day has unescaped placeholders. All $ signs should be doubled ($$)."
    );
}

#[test]
fn motd_for_client_context_accepts_the_exact_limit_and_rejects_only_overlong_values() {
    for length in [9_999, 10_000] {
        let template = "x".repeat(length);
        assert_eq!(
            super::motd_for_client_context("1.7.5", Some(&template), "", "alice", "room1"),
            template,
            "a MOTD of {length} characters should be accepted"
        );
    }

    let template = "x".repeat(10_001);
    assert_eq!(
        super::motd_for_client_context("1.7.5", Some(&template), "", "alice", "room1"),
        "Message of the Day is too long - maximum of 10000 chars, 10001 given."
    );
}

#[test]
fn hello_line_registers_session_and_returns_server_hello() {
    let mut runtime = ServerRuntime::default();
    let outbound_lines = runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255","realversion":"1.7.5"}}"#,
        )
        .expect("hello line should be accepted");

    assert_eq!(outbound_lines.len(), 4);
    let response_message = outbound_lines
        .iter()
        .filter_map(|line| decode_message_line(line).ok())
        .find(|message| matches!(message, ProtocolMessage::Hello(_)))
        .expect("sender output should include a hello response");
    let hello = extract_hello_from_message(response_message).expect("hello should extract");
    assert_eq!(hello.username, "alice");
    assert_eq!(hello.room.name, "room1");
    assert_eq!(hello.version, "1.7.5");
    assert_eq!(
        hello.realversion.as_deref(),
        Some(super::SERVER_REAL_VERSION)
    );
}

#[test]
fn hello_line_includes_upgrade_motd_for_outdated_client_version() {
    let mut runtime = ServerRuntime::default();
    let outbound_lines = runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello line should be accepted");

    let response_message = outbound_lines
        .iter()
        .filter_map(|line| decode_message_line(line).ok())
        .find(|message| matches!(message, ProtocolMessage::Hello(_)))
        .expect("sender output should include a hello response");
    let hello = extract_hello_from_message(response_message).expect("hello should extract");

    assert_eq!(
        hello.extra.get("motd"),
        Some(&Value::String(
            "You are using Syncplay 1.2.255 but a newer version is available from https://syncplay.pl"
                .to_owned(),
        ))
    );
}

#[test]
fn hello_line_uses_custom_motd_template_when_configured() {
    let mut runtime =
        ServerRuntime::with_motd_template("Client {client_version} / Latest {latest_version}");
    let outbound_lines = runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"9.9.9"}}"#,
        )
        .expect("hello line should be accepted");

    let response_message = outbound_lines
        .iter()
        .filter_map(|line| decode_message_line(line).ok())
        .find(|message| matches!(message, ProtocolMessage::Hello(_)))
        .expect("sender output should include a hello response");
    let hello = extract_hello_from_message(response_message).expect("hello should extract");

    assert_eq!(
        hello.extra.get("motd"),
        Some(&Value::String("Client 9.9.9 / Latest 1.7.5".to_owned()))
    );
}

#[test]
fn hello_line_with_custom_motd_template_prepends_warning_for_outdated_client() {
    let mut runtime = ServerRuntime::with_motd_template("Template latest={latest_version}");
    let outbound_lines = runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello line should be accepted");

    let response_message = outbound_lines
        .iter()
        .filter_map(|line| decode_message_line(line).ok())
        .find(|message| matches!(message, ProtocolMessage::Hello(_)))
        .expect("sender output should include a hello response");
    let hello = extract_hello_from_message(response_message).expect("hello should extract");

    assert_eq!(
        hello.extra.get("motd"),
        Some(&Value::String(
            "You are using Syncplay 1.2.255 but a newer version is available from https://syncplay.pl\nTemplate latest=1.7.5"
                .to_owned(),
        ))
    );
}

#[test]
fn server_app_with_motd_template_wires_runtime_override() {
    let mut app = ServerApp::with_motd_template("App MOTD for {client_version}");
    let outbound_lines = app
        .runtime_mut()
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"2.0.0"}}"#,
        )
        .expect("hello line should be accepted");

    let response_message = outbound_lines
        .iter()
        .filter_map(|line| decode_message_line(line).ok())
        .find(|message| matches!(message, ProtocolMessage::Hello(_)))
        .expect("sender output should include a hello response");
    let hello = extract_hello_from_message(response_message).expect("hello should extract");

    assert_eq!(
        hello.extra.get("motd"),
        Some(&Value::String("App MOTD for 2.0.0".to_owned()))
    );
}

#[test]
fn hello_line_reports_persistent_rooms_feature_when_enabled() {
    let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
    let outbound_lines = runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"9.9.9"}}"#,
        )
        .expect("hello line should be accepted");

    let response_message = outbound_lines
        .iter()
        .filter_map(|line| decode_message_line(line).ok())
        .find(|message| matches!(message, ProtocolMessage::Hello(_)))
        .expect("sender output should include a hello response");
    let hello = extract_hello_from_message(response_message).expect("hello should extract");
    let persistent_rooms = hello
        .features
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|features| features.get("persistentRooms"))
        .and_then(Value::as_bool);
    assert_eq!(persistent_rooms, Some(true));
}

#[test]
fn hello_line_persistent_rooms_notice_is_added_for_legacy_clients() {
    let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
    let outbound_lines = runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"9.9.9"}}"#,
        )
        .expect("hello line should be accepted");

    let response_message = outbound_lines
        .iter()
        .filter_map(|line| decode_message_line(line).ok())
        .find(|message| matches!(message, ProtocolMessage::Hello(_)))
        .expect("sender output should include a hello response");
    let hello = extract_hello_from_message(response_message).expect("hello should extract");

    assert_eq!(
        hello.extra.get("motd"),
        Some(&Value::String(
            super::LEGACY_PERSISTENT_ROOMS_NOTICE.to_owned(),
        ))
    );
}

#[test]
fn hello_line_persistent_rooms_notice_is_omitted_for_persistent_capable_clients() {
    let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
    let outbound_lines = runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"9.9.9","features":{"persistentRooms":true}}}"#,
        )
        .expect("hello line should be accepted");

    let response_message = outbound_lines
        .iter()
        .filter_map(|line| decode_message_line(line).ok())
        .find(|message| matches!(message, ProtocolMessage::Hello(_)))
        .expect("sender output should include a hello response");
    let hello = extract_hello_from_message(response_message).expect("hello should extract");

    assert_eq!(hello.extra.get("motd"), Some(&Value::String(String::new())));
}

#[test]
fn hello_line_persistent_rooms_notice_combines_with_existing_motd_with_blank_line() {
    let mut runtime = ServerRuntime::with_motd_template("Template latest={latest_version}");
    runtime.set_persistent_rooms_enabled(true);
    let outbound_lines = runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello line should be accepted");

    let response_message = outbound_lines
        .iter()
        .filter_map(|line| decode_message_line(line).ok())
        .find(|message| matches!(message, ProtocolMessage::Hello(_)))
        .expect("sender output should include a hello response");
    let hello = extract_hello_from_message(response_message).expect("hello should extract");

    assert_eq!(
        hello.extra.get("motd"),
        Some(&Value::String(format!(
            "{}\n\nYou are using Syncplay 1.2.255 but a newer version is available from https://syncplay.pl\nTemplate latest=1.7.5",
            super::LEGACY_PERSISTENT_ROOMS_NOTICE
        ),))
    );
}

#[test]
fn server_app_with_persistent_rooms_enabled_wires_runtime_override() {
    let mut app = ServerApp::with_persistent_rooms_enabled(true);
    let outbound_lines = app
        .runtime_mut()
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"9.9.9"}}"#,
        )
        .expect("hello line should be accepted");

    let response_message = outbound_lines
        .iter()
        .filter_map(|line| decode_message_line(line).ok())
        .find(|message| matches!(message, ProtocolMessage::Hello(_)))
        .expect("sender output should include a hello response");
    let hello = extract_hello_from_message(response_message).expect("hello should extract");
    let persistent_rooms = hello
        .features
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|features| features.get("persistentRooms"))
        .and_then(Value::as_bool);
    assert_eq!(persistent_rooms, Some(true));
}

#[test]
fn stats_snapshot_start_delay_for_port_matches_legacy_formula() {
    let db_path = temporary_sqlite_path("stats-delay-formula");
    let _ = fs::remove_file(&db_path);

    let mut runtime = ServerRuntime::new();
    runtime.set_time_now_override_seconds(Some(100.0));
    runtime.set_stats_snapshot_start_delay_for_port(8999);
    runtime.set_stats_snapshot_interval_seconds(1.0);
    runtime
        .set_stats_db_path(Some(db_path.clone()))
        .expect("runtime should initialize stats persistence");

    assert_eq!(runtime.stats_next_snapshot_at_seconds, Some(151.0));

    drop(runtime);
    fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
}

#[test]
fn stats_snapshot_records_connected_client_versions() {
    let db_path = temporary_sqlite_path("stats-snapshots");
    let _ = fs::remove_file(&db_path);

    let mut runtime = ServerRuntime::new();
    runtime.set_time_now_override_seconds(Some(0.0));
    runtime.set_stats_snapshot_start_delay_seconds(0.0);
    runtime.set_stats_snapshot_interval_seconds(1.0);
    runtime
        .set_stats_db_path(Some(db_path.clone()))
        .expect("runtime should initialize stats persistence");
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.0"}}"#,
        )
        .expect("first hello should establish stats-tracked session");
    runtime
        .handle_line(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.6.0"}}"#,
        )
        .expect("second hello should establish stats-tracked session");

    runtime
        .advance_time_and_collect_fanout(1.1)
        .expect("time advance should trigger first stats snapshot");
    runtime
        .flush_persistence()
        .expect("stats persistence worker should acknowledge the snapshot");
    assert_eq!(
        load_stats_snapshot_rows(&db_path),
        vec![(1, "1.6.0".to_owned()), (1, "1.7.0".to_owned())]
    );

    drop(runtime);
    fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
}

#[test]
fn server_app_with_stats_db_path_wires_runtime_override() {
    let db_path = temporary_sqlite_path("server-app-stats");
    let _ = fs::remove_file(&db_path);

    let mut app = ServerApp::with_stats_db_path(db_path.clone())
        .expect("server app should initialize stats persistence");
    app.runtime_mut().set_time_now_override_seconds(Some(0.0));
    app.runtime_mut()
        .set_stats_snapshot_start_delay_seconds(0.0);
    app.runtime_mut().set_stats_snapshot_interval_seconds(1.0);
    app.runtime_mut()
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"2.0.0"}}"#,
        )
        .expect("hello should establish stats-tracked session");

    app.runtime_mut()
        .advance_time_and_collect_fanout(1.1)
        .expect("time advance should trigger stats snapshot");
    app.runtime_mut()
        .flush_persistence()
        .expect("stats persistence worker should acknowledge the snapshot");
    assert_eq!(
        load_stats_snapshot_rows(&db_path),
        vec![(1, "2.0.0".to_owned())]
    );

    drop(app);
    fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
}

#[test]
fn tls_send_returns_false_when_server_has_no_tls_bundle() {
    let mut runtime = ServerRuntime::new();
    let outbound_lines = runtime
        .handle_line("client-1", r#"{"TLS":{"startTLS":"send"}}"#)
        .expect("tls request should be handled");
    assert_eq!(
        tls_start_response(&outbound_lines).as_deref(),
        Some("false")
    );
}

#[test]
fn tls_send_returns_true_for_unlogged_client_when_tls_bundle_is_present() {
    let cert_path = temporary_directory_path("tls-cert-bundle");
    let _ = fs::remove_dir_all(&cert_path);
    fs::create_dir_all(&cert_path).expect("tls cert temp directory should be creatable");
    write_valid_tls_bundle(&cert_path);

    let mut runtime = ServerRuntime::new();
    runtime.set_tls_cert_path(Some(cert_path.clone()));
    let outbound_lines = runtime
        .handle_line("client-1", r#"{"TLS":{"startTLS":"send"}}"#)
        .expect("tls request should be handled");
    assert_eq!(tls_start_response(&outbound_lines).as_deref(), Some("true"));

    fs::remove_dir_all(&cert_path).expect("tls cert temp directory should be removable");
}

#[test]
fn tls_send_true_enqueues_start_tls_transport_action() {
    let cert_path = temporary_directory_path("tls-transport-action");
    let _ = fs::remove_dir_all(&cert_path);
    fs::create_dir_all(&cert_path).expect("tls cert temp directory should be creatable");
    write_valid_tls_bundle(&cert_path);

    let mut runtime = ServerRuntime::new();
    runtime.set_tls_cert_path(Some(cert_path.clone()));
    let outbound_lines = runtime
        .handle_line("client-1", r#"{"TLS":{"startTLS":"send"}}"#)
        .expect("tls request should be handled");
    assert_eq!(tls_start_response(&outbound_lines).as_deref(), Some("true"));
    let transport_actions = runtime.drain_transport_actions();
    assert!(
        has_start_tls_transport_action(&transport_actions, "client-1"),
        "startTLS=true should emit a transport StartTls action"
    );
    assert!(
        runtime.drain_transport_actions().is_empty(),
        "transport actions should drain once"
    );

    fs::remove_dir_all(&cert_path).expect("tls cert temp directory should be removable");
}

#[test]
fn tls_send_dispatch_includes_transport_action_bundle() {
    let cert_path = temporary_directory_path("tls-dispatch-action");
    let _ = fs::remove_dir_all(&cert_path);
    fs::create_dir_all(&cert_path).expect("tls cert temp directory should be creatable");
    write_valid_tls_bundle(&cert_path);

    let mut runtime = ServerRuntime::new();
    runtime.set_tls_cert_path(Some(cert_path.clone()));
    let dispatch = runtime
        .handle_line_fanout_with_transport_actions("client-1", r#"{"TLS":{"startTLS":"send"}}"#)
        .expect("tls dispatch should be handled");
    assert_eq!(
        tls_start_response(
            &dispatch
                .outbound_lines
                .iter()
                .map(|line| line.line.clone())
                .collect::<Vec<_>>(),
        )
        .as_deref(),
        Some("true")
    );
    assert!(
        has_start_tls_transport_action(&dispatch.transport_actions, "client-1"),
        "dispatch should contain start-tls transport action"
    );
    assert!(
        runtime.drain_transport_actions().is_empty(),
        "dispatch helper should drain transport action queue"
    );

    fs::remove_dir_all(&cert_path).expect("tls cert temp directory should be removable");
}

#[test]
fn start_tls_is_a_transport_boundary_for_a_bundled_hello() {
    let cert_path = temporary_directory_path("tls-bundled-hello-boundary");
    let _ = fs::remove_dir_all(&cert_path);
    fs::create_dir_all(&cert_path).expect("tls cert temp directory should be creatable");
    write_valid_tls_bundle(&cert_path);

    let mut runtime = ServerRuntime::new();
    runtime.set_tls_cert_path(Some(cert_path.clone()));
    let dispatch = runtime
        .handle_line_fanout_with_transport_actions(
            "client-1",
            r#"{"TLS":{"startTLS":"send"},"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("STARTTLS negotiation should be handled");

    assert!(
        has_start_tls_transport_action(&dispatch.transport_actions, "client-1"),
        "the server should still authorize the transport upgrade"
    );
    assert!(
        runtime.session("client-1").is_none(),
        "application commands bundled after STARTTLS must not execute on the plaintext side of the transport boundary"
    );
    assert_eq!(
        dispatch.outbound_lines.len(),
        1,
        "only the STARTTLS acknowledgement may be written before the connection is upgraded"
    );

    fs::remove_dir_all(&cert_path).expect("tls cert temp directory should be removable");
}

#[test]
fn protocol_error_dispatch_sends_not_json_error_and_close() {
    let mut runtime = ServerRuntime::new();

    let dispatch = runtime
        .handle_line_fanout_with_transport_actions("client-1", "not-json")
        .expect("malformed protocol line should produce protocol error dispatch");

    assert_eq!(
        dispatch_error_message(&dispatch).as_deref(),
        Some("Not a json encoded string not-json")
    );
    assert!(
        has_close_transport_action(&dispatch.transport_actions, "client-1"),
        "malformed protocol line should schedule connection close after Error"
    );
}

#[test]
fn reflected_malformed_line_debug_does_not_expose_credentials() {
    let marker = "server-reflected-line-password-canary";
    let malformed = format!("not-json password={marker}");
    let mut runtime = ServerRuntime::new();

    let dispatch = runtime
        .handle_line_fanout_with_transport_actions("client-1", &malformed)
        .expect("malformed protocol line should produce protocol error dispatch");

    assert!(
        dispatch_error_message(&dispatch)
            .as_deref()
            .is_some_and(|message| message.contains(marker)),
        "wire-compatible error should retain the reflected input"
    );
    assert!(!format!("{dispatch:?}").contains(marker));
}

#[test]
fn protocol_error_dispatch_sends_unknown_command_error_and_close() {
    let mut runtime = ServerRuntime::new();

    let dispatch = runtime
        .handle_line_fanout_with_transport_actions("client-1", r#"{"Bogus":{"x":1}}"#)
        .expect("unknown command should produce protocol error dispatch");

    assert!(
        dispatch_error_message(&dispatch)
            .as_deref()
            .is_some_and(|message| message.starts_with("Unknown command")),
        "unknown command should be serialized as protocol Error"
    );
    assert!(
        has_close_transport_action(&dispatch.transport_actions, "client-1"),
        "unknown command should schedule connection close after Error"
    );
}

#[test]
fn protocol_error_dispatch_flushes_valid_batched_commands_before_unknown_command() {
    let mut runtime = ServerRuntime::new();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should establish session");

    let dispatch = runtime
        .handle_line_fanout_with_transport_actions(
            "client-1",
            r#"{"Set":{"room":{"name":"room2"}},"Bogus":{"x":1}}"#,
        )
        .expect("mixed batched line should produce protocol error dispatch");
    let directed_messages = decode_directed_lines(&dispatch.outbound_lines);
    let error_position = directed_messages
        .iter()
        .position(|(_, message)| matches!(message, ProtocolMessage::Error(_)))
        .expect("unknown command should emit protocol Error");

    assert!(
        error_position > 0,
        "valid commands before the unknown command should be flushed first"
    );
    assert_eq!(
        runtime
            .session("client-1")
            .expect("session should remain inspectable until transport closes")
            .room,
        "room2"
    );
    assert!(
        dispatch_error_message(&dispatch)
            .as_deref()
            .is_some_and(|message| message == r#"Unknown command {"x":1}"#),
        "unknown command should use the offending command payload"
    );
    assert!(
        has_close_transport_action(&dispatch.transport_actions, "client-1"),
        "unknown command should schedule connection close after Error"
    );
}

#[test]
fn protocol_error_dispatch_sends_not_known_error_for_pre_hello_command_and_close() {
    let mut runtime = ServerRuntime::new();

    let dispatch = runtime
        .handle_line_fanout_with_transport_actions("client-1", r#"{"List":null}"#)
        .expect("pre-hello command should produce protocol error dispatch");

    assert_eq!(
        dispatch_error_message(&dispatch).as_deref(),
        Some("You must be known to server before sending this command")
    );
    assert!(
        has_close_transport_action(&dispatch.transport_actions, "client-1"),
        "pre-hello command should schedule connection close after Error"
    );
}

#[test]
fn protocol_error_dispatch_sends_hello_argument_error_and_close() {
    let mut runtime = ServerRuntime::new();

    let dispatch = runtime
        .handle_line_fanout_with_transport_actions("client-1", r#"{"Hello":{"username":"alice"}}"#)
        .expect("invalid hello should produce protocol error dispatch");

    assert_eq!(
        dispatch_error_message(&dispatch).as_deref(),
        Some("Not enough Hello arguments")
    );
    assert!(
        has_close_transport_action(&dispatch.transport_actions, "client-1"),
        "invalid hello should schedule connection close after Error"
    );
}

#[test]
fn tls_send_returns_false_for_logged_client_even_when_tls_bundle_is_present() {
    let cert_path = temporary_directory_path("tls-after-hello");
    let _ = fs::remove_dir_all(&cert_path);
    fs::create_dir_all(&cert_path).expect("tls cert temp directory should be creatable");
    write_valid_tls_bundle(&cert_path);

    let mut runtime = ServerRuntime::new();
    runtime.set_tls_cert_path(Some(cert_path.clone()));
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"9.9.9"}}"#,
        )
        .expect("hello should log in client");
    let outbound_lines = runtime
        .handle_line("client-1", r#"{"TLS":{"startTLS":"send"}}"#)
        .expect("tls request should be handled");
    assert_eq!(
        tls_start_response(&outbound_lines).as_deref(),
        Some("false")
    );

    fs::remove_dir_all(&cert_path).expect("tls cert temp directory should be removable");
}

#[test]
fn tls_send_false_does_not_enqueue_transport_action() {
    let mut runtime = ServerRuntime::new();
    let outbound_lines = runtime
        .handle_line("client-1", r#"{"TLS":{"startTLS":"send"}}"#)
        .expect("tls request should be handled");
    assert_eq!(
        tls_start_response(&outbound_lines).as_deref(),
        Some("false")
    );
    assert!(
        runtime.drain_transport_actions().is_empty(),
        "startTLS=false should not emit transport actions"
    );
}

#[test]
fn tls_non_send_inquiry_is_ignored() {
    let mut runtime = ServerRuntime::new();
    let outbound_lines = runtime
        .handle_line("client-1", r#"{"TLS":{"startTLS":"status"}}"#)
        .expect("tls request should be handled");
    assert!(outbound_lines.is_empty());
}

#[test]
fn server_app_with_tls_cert_path_wires_runtime_override() {
    let cert_path = temporary_directory_path("tls-server-app");
    let _ = fs::remove_dir_all(&cert_path);
    fs::create_dir_all(&cert_path).expect("tls cert temp directory should be creatable");
    write_valid_tls_bundle(&cert_path);

    let mut app = ServerApp::with_tls_cert_path(cert_path.clone());
    let outbound_lines = app
        .runtime_mut()
        .handle_line("client-1", r#"{"TLS":{"startTLS":"send"}}"#)
        .expect("tls request should be handled");
    assert_eq!(tls_start_response(&outbound_lines).as_deref(), Some("true"));

    fs::remove_dir_all(&cert_path).expect("tls cert temp directory should be removable");
}

#[test]
fn tls_send_keeps_loaded_context_when_cert_files_disappear_without_rotation_signal() {
    let cert_path = temporary_directory_path("tls-context-cache");
    let _ = fs::remove_dir_all(&cert_path);
    fs::create_dir_all(&cert_path).expect("tls cert temp directory should be creatable");
    write_valid_tls_bundle(&cert_path);

    let (mut runtime, _metadata_clock) = server_runtime_with_tls_metadata_clock(&cert_path);
    fs::remove_file(cert_path.join("privkey.pem")).expect("privkey file should be removable");
    fs::remove_file(cert_path.join("chain.pem")).expect("chain file should be removable");
    fs::remove_file(cert_path.join("cert.pem")).expect("cert file should be removable");

    let outbound_lines = runtime
        .handle_line("client-1", r#"{"TLS":{"startTLS":"send"}}"#)
        .expect("tls request should be handled");
    assert_eq!(tls_start_response(&outbound_lines).as_deref(), Some("true"));

    fs::remove_dir_all(&cert_path).expect("tls cert temp directory should be removable");
}

#[test]
fn tls_send_reloads_context_when_cert_edit_time_changes() {
    let cert_path = temporary_directory_path("tls-cert-rotation");
    let _ = fs::remove_dir_all(&cert_path);
    fs::create_dir_all(&cert_path).expect("tls cert temp directory should be creatable");
    write_valid_tls_bundle(&cert_path);

    let (mut runtime, metadata_clock) = server_runtime_with_tls_metadata_clock(&cert_path);
    let initial_outbound = runtime
        .handle_line("client-1", r#"{"TLS":{"startTLS":"send"}}"#)
        .expect("initial tls request should be handled");
    assert_eq!(
        tls_start_response(&initial_outbound).as_deref(),
        Some("true")
    );

    fs::remove_file(cert_path.join("chain.pem")).expect("chain file should be removable");
    fs::write(cert_path.join("cert.pem"), "rotated-cert")
        .expect("rotated certificate fixture should write");
    metadata_clock.advance();

    let rotated_outbound = runtime
        .handle_line("client-1", r#"{"TLS":{"startTLS":"send"}}"#)
        .expect("rotated tls request should be handled");
    assert_eq!(
        tls_start_response(&rotated_outbound).as_deref(),
        Some("false")
    );

    fs::remove_dir_all(&cert_path).expect("tls cert temp directory should be removable");
}

#[test]
fn tls_rotation_detects_non_max_member_edit_on_filesystem() {
    let cert_path = temporary_directory_path("tls-rotation-max-mtime-collision");
    let _ = fs::remove_dir_all(&cert_path);
    fs::create_dir_all(&cert_path).expect("tls cert temp directory should be creatable");
    write_valid_tls_bundle(&cert_path);

    let base_time = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    set_file_modified_time_for_test(
        &cert_path.join("privkey.pem"),
        base_time + Duration::from_secs(60),
    );
    set_file_modified_time_for_test(
        &cert_path.join("chain.pem"),
        base_time + Duration::from_secs(120),
    );
    set_file_modified_time_for_test(
        &cert_path.join("cert.pem"),
        base_time + Duration::from_secs(180),
    );

    let mut runtime = ServerRuntime::new();
    runtime.set_tls_cert_path(Some(cert_path.clone()));
    let token_before = tls_certificate_bundle_fingerprint(&cert_path);

    fs::write(cert_path.join("privkey.pem"), "rotated-invalid-private-key")
        .expect("rotated private-key fixture should write");
    set_file_modified_time_for_test(
        &cert_path.join("privkey.pem"),
        base_time + Duration::from_secs(90),
    );
    let token_after = tls_certificate_bundle_fingerprint(&cert_path);
    assert_ne!(
        token_after, token_before,
        "content replacement must change the bundle fingerprint even below the maximum mtime"
    );

    let outbound_lines = runtime
        .handle_line("client-1", r#"{"TLS":{"startTLS":"send"}}"#)
        .expect("TLS request should be handled");
    assert_eq!(
        tls_start_response(&outbound_lines).as_deref(),
        Some("false"),
        "rotating a required TLS bundle member must invalidate the cached context"
    );
    fs::remove_dir_all(&cert_path).expect("tls cert temp directory should be removable");
}

#[derive(Debug, Clone, Copy)]
enum TlsRotationHistoryStep {
    CorruptWithoutRevision,
    RotateInvalid,
    RotateValid,
}

impl TlsRotationHistoryStep {
    fn from_ternary_digit(digit: usize) -> Self {
        match digit {
            0 => Self::CorruptWithoutRevision,
            1 => Self::RotateInvalid,
            2 => Self::RotateValid,
            _ => unreachable!("a ternary digit must be in 0..=2"),
        }
    }

    fn changed_revision(self) -> bool {
        !matches!(self, Self::CorruptWithoutRevision)
    }

    fn bundle_is_valid(self) -> bool {
        matches!(self, Self::RotateValid)
    }
}

#[derive(Debug)]
struct TlsRotationReferenceState {
    context_available: bool,
    accepts_tls: bool,
    attempts: u32,
}

impl TlsRotationReferenceState {
    fn apply(&mut self, step: TlsRotationHistoryStep) -> bool {
        let accepted_before_step = self.accepts_tls;
        if step.changed_revision() && accepted_before_step {
            self.attempts += 1;
            self.context_available = step.bundle_is_valid();
            self.accepts_tls =
                self.context_available || self.attempts < super::TLS_CERT_ROTATION_MAX_RETRIES;
        }
        accepted_before_step && self.context_available
    }
}

#[test]
fn tls_rotation_revision_histories_match_reference_model_without_wall_clock_waits() {
    const HISTORY_LENGTH: usize = 5;
    const HISTORY_COUNT: usize = 3_usize.pow(HISTORY_LENGTH as u32);

    let cert_path = temporary_directory_path("tls-rotation-reference-model");
    let _ = fs::remove_dir_all(&cert_path);
    fs::create_dir_all(&cert_path).expect("tls cert temp directory should be creatable");

    for encoded_history in 0..HISTORY_COUNT {
        write_valid_tls_bundle(&cert_path);
        let (mut runtime, metadata_clock) = server_runtime_with_tls_metadata_clock(&cert_path);
        let mut reference = TlsRotationReferenceState {
            context_available: true,
            accepts_tls: true,
            attempts: 0,
        };
        let mut remaining_history = encoded_history;

        for step_index in 0..HISTORY_LENGTH {
            let step = TlsRotationHistoryStep::from_ternary_digit(remaining_history % 3);
            remaining_history /= 3;
            match step {
                TlsRotationHistoryStep::CorruptWithoutRevision => {
                    write_invalid_tls_bundle(
                        &cert_path,
                        &format!("{encoded_history}-{step_index}-cached"),
                    );
                }
                TlsRotationHistoryStep::RotateInvalid => {
                    write_invalid_tls_bundle(
                        &cert_path,
                        &format!("{encoded_history}-{step_index}-rotated"),
                    );
                    metadata_clock.advance();
                }
                TlsRotationHistoryStep::RotateValid => {
                    write_valid_tls_bundle(&cert_path);
                    metadata_clock.advance();
                }
            }

            let expected_start_tls = reference.apply(step);
            let client_id = format!("history-{encoded_history}-step-{step_index}");
            let outbound_lines = runtime
                .handle_line(&client_id, r#"{"TLS":{"startTLS":"send"}}"#)
                .expect("generated TLS request should be handled");
            assert_eq!(
                tls_start_response(&outbound_lines).as_deref(),
                Some(if expected_start_tls { "true" } else { "false" }),
                "STARTTLS response diverged for history {encoded_history}, step {step_index}: {step:?}"
            );
            assert_eq!(
                has_start_tls_transport_action(&runtime.drain_transport_actions(), &client_id,),
                expected_start_tls,
                "transport action diverged for history {encoded_history}, step {step_index}: {step:?}"
            );
            assert_eq!(
                runtime.tls_context_available, reference.context_available,
                "context availability diverged for history {encoded_history}, step {step_index}: {step:?}"
            );
            assert_eq!(
                runtime.server_accepts_tls, reference.accepts_tls,
                "acceptability diverged for history {encoded_history}, step {step_index}: {step:?}"
            );
            assert_eq!(
                runtime.tls_rotation_attempts, reference.attempts,
                "retry count diverged for history {encoded_history}, step {step_index}: {step:?}"
            );
        }
    }

    fs::remove_dir_all(&cert_path).expect("tls cert temp directory should be removable");
}

fn assert_tls_rotation_detects_file_change(filename: &str) {
    let cert_path = temporary_directory_path(&format!("tls-rotation-{filename}"));
    let _ = fs::remove_dir_all(&cert_path);
    fs::create_dir_all(&cert_path).expect("tls cert temp directory should be creatable");
    write_valid_tls_bundle(&cert_path);

    let (mut runtime, metadata_clock) = server_runtime_with_tls_metadata_clock(&cert_path);
    let initial_outbound = runtime
        .handle_line("client-1", r#"{"TLS":{"startTLS":"send"}}"#)
        .expect("initial tls request should be handled");
    assert_eq!(
        tls_start_response(&initial_outbound).as_deref(),
        Some("true")
    );

    fs::write(cert_path.join(filename), format!("rotated-{filename}"))
        .expect("rotated TLS bundle member should write");
    metadata_clock.advance();

    let rotated_outbound = runtime
        .handle_line("client-1", r#"{"TLS":{"startTLS":"send"}}"#)
        .expect("rotated tls request should be handled");
    assert_eq!(
        tls_start_response(&rotated_outbound).as_deref(),
        Some("false"),
        "editing {filename} should trigger TLS bundle reload"
    );

    fs::remove_dir_all(&cert_path).expect("tls cert temp directory should be removable");
}

#[test]
fn tls_rotation_detects_cert_change() {
    assert_tls_rotation_detects_file_change("cert.pem");
}

#[test]
fn tls_rotation_detects_chain_change() {
    assert_tls_rotation_detects_file_change("chain.pem");
}

#[test]
fn tls_rotation_detects_privkey_change() {
    assert_tls_rotation_detects_file_change("privkey.pem");
}

#[test]
fn tls_rotation_retry_cap_disables_acceptability_after_repeated_failed_reloads() {
    let cert_path = temporary_directory_path("tls-cert-rotation-retry-cap");
    let _ = fs::remove_dir_all(&cert_path);
    fs::create_dir_all(&cert_path).expect("tls cert temp directory should be creatable");
    write_valid_tls_bundle(&cert_path);

    let (mut runtime, metadata_clock) = server_runtime_with_tls_metadata_clock(&cert_path);
    fs::remove_file(cert_path.join("chain.pem")).expect("chain file should be removable");

    for attempt in 0..super::TLS_CERT_ROTATION_MAX_RETRIES {
        fs::write(
            cert_path.join("cert.pem"),
            format!("rotated-cert-{attempt}"),
        )
        .expect("invalid rotated certificate should write");
        metadata_clock.advance();
        let outbound_lines = runtime
            .handle_line("client-1", r#"{"TLS":{"startTLS":"send"}}"#)
            .expect("tls request should be handled");
        assert_eq!(
            tls_start_response(&outbound_lines).as_deref(),
            Some("false")
        );
    }
    assert!(
        !runtime.server_accepts_tls,
        "retry cap should eventually disable server_accepts_tls gate"
    );

    fs::write(cert_path.join("chain.pem"), "chain-restored")
        .expect("chain file restore should succeed");
    fs::write(cert_path.join("cert.pem"), "restored-cert")
        .expect("restored certificate fixture should write");
    metadata_clock.advance();
    let outbound_after_restore = runtime
        .handle_line("client-1", r#"{"TLS":{"startTLS":"send"}}"#)
        .expect("tls request after restore should be handled");
    assert_eq!(
        tls_start_response(&outbound_after_restore).as_deref(),
        Some("false"),
        "legacy retry-cap behavior should keep TLS disabled once acceptability gate is off"
    );

    fs::remove_dir_all(&cert_path).expect("tls cert temp directory should be removable");
}

#[test]
fn transport_disconnect_fanout_emits_left_event_and_removes_session() {
    let mut runtime = ServerRuntime::default();
    runtime
        .handle_line(
            "client-1",
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.0"}}"#,
        )
        .expect("first hello should establish session");
    runtime
        .handle_line(
            "client-2",
            r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.7.0"}}"#,
        )
        .expect("second hello should establish session");

    let disconnect_lines = runtime
        .handle_transport_disconnect_fanout("client-1")
        .expect("transport disconnect should generate fanout");
    let disconnect_messages = decode_directed_lines(&disconnect_lines);
    assert!(
        has_user_event(&disconnect_messages, "client-2", "alice", "left"),
        "remaining peer should receive left event on transport disconnect"
    );
    assert!(
        runtime.session("client-1").is_none(),
        "disconnected client session should be removed from runtime state"
    );
}
