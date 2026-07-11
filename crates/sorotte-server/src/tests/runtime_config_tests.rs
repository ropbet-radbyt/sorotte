use super::*;

#[test]
fn room_password_provider_matches_legacy_sha_hash_output() {
    let provider = RoomPasswordProvider::default();
    let controlled_room_name = provider.controlled_room_name_for("room1", "AB-123-456");
    assert_eq!(controlled_room_name, "+room1:CB39A19549E8");
    assert_eq!(
        provider.check(&controlled_room_name, "AB-123-456"),
        Ok(true)
    );
    assert_eq!(
        provider.check(&controlled_room_name, "AB-123-457"),
        Ok(false)
    );
}

#[test]
fn controlled_room_password_accepts_exact_legacy_format() {
    let provider = RoomPasswordProvider::default();
    assert!(provider.is_valid_room_password("AB-123-456"));
    assert_eq!(
        provider.check("+room1:CB39A19549E8", "AB-123-456"),
        Ok(true)
    );
}

#[test]
fn controlled_room_password_rejects_trailing_characters() {
    let provider = RoomPasswordProvider::default();
    assert!(!provider.is_valid_room_password("AB-123-4567"));
    assert!(!provider.is_valid_room_password("AB-123-456-extra"));
    assert!(!provider.is_valid_room_password("ab-123-456"));
    assert_eq!(
        provider.check("+room1:CB39A19549E8", "AB-123-4567"),
        Err(RoomPasswordCheckError::InvalidPassword)
    );
    assert_eq!(
        provider.check("+room1:CB39A19549E8", "bad-password"),
        Err(RoomPasswordCheckError::InvalidPassword)
    );
}

#[test]
fn room_password_provider_salt_changes_controlled_room_hashes() {
    let default_provider = RoomPasswordProvider::default();
    let custom_provider = RoomPasswordProvider::new("custom-salt");
    let password = "AB-123-456";
    let default_room_name = default_provider.controlled_room_name_for("room1", password);
    let custom_room_name = custom_provider.controlled_room_name_for("room1", password);
    assert_ne!(custom_room_name, default_room_name);
    assert_eq!(custom_provider.check(&custom_room_name, password), Ok(true));
    assert_eq!(
        default_provider.check(&custom_room_name, password),
        Ok(false)
    );
}

#[test]
fn generated_server_salt_matches_legacy_shape() {
    let salt = generate_server_salt_legacy_compatible();

    assert_eq!(salt.len(), 10);
    assert!(salt.chars().all(|character| character.is_ascii_uppercase()));
}

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
fn motd_for_client_context_reports_overlong_rendered_template() {
    let template = "x".repeat(10_000);
    assert_eq!(
        super::motd_for_client_context("1.7.5", Some(&template), "", "alice", "room1"),
        "Message of the Day is too long - maximum of 10000 chars, 10000 given."
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

    let mut runtime = ServerRuntime::new();
    runtime.set_tls_cert_path(Some(cert_path.clone()));
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

    let mut runtime = ServerRuntime::new();
    runtime.set_tls_cert_path(Some(cert_path.clone()));
    let initial_outbound = runtime
        .handle_line("client-1", r#"{"TLS":{"startTLS":"send"}}"#)
        .expect("initial tls request should be handled");
    assert_eq!(
        tls_start_response(&initial_outbound).as_deref(),
        Some("true")
    );

    fs::remove_file(cert_path.join("chain.pem")).expect("chain file should be removable");
    overwrite_file_until_modified_time_changes(&cert_path.join("cert.pem"), "rotated-cert");

    let rotated_outbound = runtime
        .handle_line("client-1", r#"{"TLS":{"startTLS":"send"}}"#)
        .expect("rotated tls request should be handled");
    assert_eq!(
        tls_start_response(&rotated_outbound).as_deref(),
        Some("false")
    );

    fs::remove_dir_all(&cert_path).expect("tls cert temp directory should be removable");
}

fn assert_tls_rotation_detects_file_change(filename: &str) {
    let cert_path = temporary_directory_path(&format!("tls-rotation-{filename}"));
    let _ = fs::remove_dir_all(&cert_path);
    fs::create_dir_all(&cert_path).expect("tls cert temp directory should be creatable");
    write_valid_tls_bundle(&cert_path);

    let mut runtime = ServerRuntime::new();
    runtime.set_tls_cert_path(Some(cert_path.clone()));
    let initial_outbound = runtime
        .handle_line("client-1", r#"{"TLS":{"startTLS":"send"}}"#)
        .expect("initial tls request should be handled");
    assert_eq!(
        tls_start_response(&initial_outbound).as_deref(),
        Some("true")
    );

    overwrite_file_until_modified_time_changes(
        &cert_path.join(filename),
        &format!("rotated-{filename}"),
    );

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

    let mut runtime = ServerRuntime::new();
    runtime.set_tls_cert_path(Some(cert_path.clone()));
    fs::remove_file(cert_path.join("chain.pem")).expect("chain file should be removable");

    for attempt in 0..super::TLS_CERT_ROTATION_MAX_RETRIES {
        overwrite_file_until_modified_time_changes(
            &cert_path.join("cert.pem"),
            &format!("rotated-cert-{attempt}"),
        );
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
    overwrite_file_until_modified_time_changes(&cert_path.join("cert.pem"), "restored-cert");
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
