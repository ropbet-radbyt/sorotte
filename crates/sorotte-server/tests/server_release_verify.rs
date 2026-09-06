mod support;

use std::{fs, thread, time::Duration};

use serde_json::{Value, json};
use sorotte_protocol::{PingPayload, ProtocolMessage, StatePayload, TlsPayload};

use support::*;

// Each case owns its child processes, unique paths and ports. There is no process-wide
// environment mutation, so a shared mutex only coupled unrelated failures (and did
// not serialize nextest's separate processes).
#[test]
fn fixture_timeout_preserves_primary_failure_and_next_case_runs_after_cleanup() {
    let port = reserve_ipv4_port();
    let failure = std::panic::catch_unwind(|| {
        let mut server = ServerProcess::spawn(&server_args(
            port,
            &["--ipv4-only", "--interface-ipv4", "127.0.0.1"],
        ));
        let mut client = server.wait_for_ipv4(port);
        client.read_until_expected(
            "injected absent Hello",
            Duration::from_millis(40),
            1,
            |message| message.kind() == "Hello",
        );
    })
    .expect_err("injected absent exchange must fail");
    let detail = failure
        .downcast_ref::<String>()
        .expect("fixture panic should explain failure");
    assert!(detail.contains("absent-response") && detail.contains("injected absent Hello"));
    assert!(detail.contains("replay_args") && detail.contains("recent="));
    assert!(
        detail.contains("server_release_verify.rs:"),
        "failure must name the owning expectation: {detail}"
    );
    assert!(
        std::net::TcpStream::connect(("127.0.0.1", port)).is_err(),
        "unwinding must terminate the owned server before another case runs"
    );
    let mut next = ServerProcess::spawn(&server_args(
        port,
        &["--ipv4-only", "--interface-ipv4", "127.0.0.1"],
    ));
    let mut client = next.wait_for_ipv4(port);
    client.hello("independent-case", "fixture-cleanup");
}

fn server_args(port: u16, extra: &[&str]) -> Vec<String> {
    let mut args = vec!["--port".to_owned(), port.to_string()];
    args.extend(extra.iter().map(|arg| (*arg).to_owned()));
    args
}

fn message_value(message: &ProtocolMessage) -> Value {
    serde_json::to_value(message).expect("protocol message should serialize to JSON")
}

fn message_pointer_eq(message: &ProtocolMessage, pointer: &str, expected: Value) -> bool {
    message_value(message).pointer(pointer) == Some(&expected)
}

fn message_pointer_contains(message: &ProtocolMessage, pointer: &str, expected: &str) -> bool {
    message_value(message)
        .pointer(pointer)
        .and_then(Value::as_str)
        .is_some_and(|actual| actual.contains(expected))
}

#[test]
fn release_verify_listener_startup_modes_and_partial_bind() {
    let ipv4_port = reserve_ipv4_port();
    let mut ipv4_server = ServerProcess::spawn(&server_args(
        ipv4_port,
        &["--ipv4-only", "--interface-ipv4", "127.0.0.1"],
    ));
    let _ipv4_client = ipv4_server.wait_for_ipv4(ipv4_port);

    let Some(ipv6_port) = reserve_ipv6_port_or_skip() else {
        return;
    };
    let mut ipv6_server = ServerProcess::spawn(&server_args(
        ipv6_port,
        &["--ipv6-only", "--interface-ipv6", "::1"],
    ));
    let _ipv6_client = ipv6_server.wait_for_ipv6(ipv6_port);

    let Some(dual_port) = reserve_ipv6_port_or_skip() else {
        return;
    };
    let mut dual_server = ServerProcess::spawn(&server_args(
        dual_port,
        &["--interface-ipv4", "127.0.0.1", "--interface-ipv6", "::1"],
    ));
    let _dual_ipv6 = dual_server.wait_for_ipv6(dual_port);
    let _dual_ipv4 = dual_server.connect_ipv4(dual_port);

    let partial_port = reserve_ipv4_port();
    let occupied_ipv4 =
        std::net::TcpListener::bind(("127.0.0.1", partial_port)).expect("IPv4 guard should bind");
    let mut partial_server = ServerProcess::spawn(&server_args(
        partial_port,
        &["--interface-ipv4", "127.0.0.1", "--interface-ipv6", "::1"],
    ));
    let _partial_ipv6 = partial_server.wait_for_ipv6(partial_port);
    partial_server.wait_for_stderr_contains("listener bind failed");
    drop(occupied_ipv4);
}

#[test]
fn release_verify_direct_protocol_room_state_chat_playlist_and_fanout() {
    let port = reserve_ipv4_port();
    let mut server = ServerProcess::spawn(&server_args(
        port,
        &["--ipv4-only", "--interface-ipv4", "127.0.0.1"],
    ));
    let mut alice = server.wait_for_ipv4(port);
    alice.hello("alice", "room-a");
    let mut bob = server.connect_ipv4(port);
    bob.hello("bob", "room-a");

    alice.read_until(|message| {
        message_pointer_eq(message, "/Set/user/bob/event/joined", json!(true))
    });

    alice.write_message(&chat_message("hello from alice"));
    bob.read_until(|message| {
        message_pointer_eq(message, "/Chat/username", json!("alice"))
            && message_pointer_eq(message, "/Chat/message", json!("hello from alice"))
    });

    alice.write_message(&set_ready_message(true));
    bob.read_until(|message| {
        message_pointer_eq(message, "/Set/ready/username", json!("alice"))
            && message_pointer_eq(message, "/Set/ready/isReady", json!(true))
    });

    alice.write_message(&set_file_message("episode-01.mkv"));
    bob.read_until(|message| {
        message_pointer_eq(
            message,
            "/Set/user/alice/file/name",
            json!("episode-01.mkv"),
        )
    });

    alice.write_message(&set_playlist_message(&["episode-01.mkv", "episode-02.mkv"]));
    bob.read_until(|message| {
        message_pointer_eq(
            message,
            "/Set/playlistChange/files",
            json!(["episode-01.mkv", "episode-02.mkv"]),
        )
    });

    alice.write_message(&set_playlist_index_message(1));
    bob.read_until(|message| message_pointer_eq(message, "/Set/playlistIndex/index", json!(1)));

    alice.write_message(&state_message(42.0, false));
    bob.read_until(|message| {
        message_pointer_eq(message, "/State/playstate/position", json!(42.0))
            && message_pointer_eq(message, "/State/playstate/paused", json!(false))
    });

    bob.write_message(&ProtocolMessage::list_request());
    let rooms = expect_list_rooms(bob.read_until_kind("List"));
    assert!(
        rooms
            .get("room-a")
            .is_some_and(|room| { room.contains_key("alice") && room.contains_key("bob") })
    );
}

#[test]
fn release_verify_password_motd_and_protocol_errors() {
    let motd_file = temporary_path("release-motd", "txt");
    fs::write(
        &motd_file,
        "\u{feff}Server=$version IP=$userIp User=$username Room=$room",
    )
    .expect("MOTD file should write");

    let port = reserve_ipv4_port();
    let mut server = ServerProcess::spawn(&server_args(
        port,
        &[
            "--ipv4-only",
            "--interface-ipv4",
            "127.0.0.1",
            "--password",
            "secret",
            "--motd-file",
            motd_file.to_str().expect("MOTD path should be UTF-8"),
        ],
    ));

    let mut missing_password = server.wait_for_ipv4(port);
    missing_password.write_json_line(
        r#"{"Hello":{"username":"missing","room":{"name":"room"},"version":"1.7.5"}}"#,
    );
    assert!(message_pointer_eq(
        &missing_password.read_until_kind("Error"),
        "/Error/message",
        json!("Password required"),
    ));

    let mut wrong_password = server.connect_ipv4(port);
    wrong_password.write_json_line(
        r#"{"Hello":{"username":"wrong","room":{"name":"room"},"version":"1.7.5","password":"bad"}}"#,
    );
    assert!(message_pointer_eq(
        &wrong_password.read_until_kind("Error"),
        "/Error/message",
        json!("Wrong password supplied"),
    ));

    let mut good_password = server.connect_ipv4(port);
    good_password.write_json_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room"},"version":"1.7.5","password":"5ebe2294ecd0e0f08eab7690d2a6ee69"}}"#,
    );
    let hello = good_password.read_until_kind("Hello");
    assert!(message_pointer_eq(
        &hello,
        "/Hello/username",
        json!("alice")
    ));
    assert!(message_pointer_eq(
        &hello,
        "/Hello/motd",
        json!("Server=1.7.5 IP=127.0.0.1 User=alice Room=room"),
    ));

    let invalid_motd_file = temporary_path("release-invalid-motd", "txt");
    fs::write(&invalid_motd_file, "Bad $ placeholder").expect("invalid MOTD should write");
    let invalid_port = reserve_ipv4_port();
    let mut invalid_server = ServerProcess::spawn(&server_args(
        invalid_port,
        &[
            "--ipv4-only",
            "--interface-ipv4",
            "127.0.0.1",
            "--motd-file",
            invalid_motd_file
                .to_str()
                .expect("invalid MOTD path should be UTF-8"),
        ],
    ));
    let mut invalid_client = invalid_server.wait_for_ipv4(invalid_port);
    invalid_client.write_message(&hello_message("badmotd", "room"));
    let invalid_hello = invalid_client.read_until_kind("Hello");
    assert!(message_pointer_contains(
        &invalid_hello,
        "/Hello/motd",
        "unescaped placeholders"
    ));

    let overlong_motd_file = temporary_path("release-overlong-motd", "txt");
    fs::write(&overlong_motd_file, "x".repeat(10_001)).expect("overlong MOTD should write");
    let overlong_port = reserve_ipv4_port();
    let mut overlong_server = ServerProcess::spawn(&server_args(
        overlong_port,
        &[
            "--ipv4-only",
            "--interface-ipv4",
            "127.0.0.1",
            "--motd-file",
            overlong_motd_file
                .to_str()
                .expect("overlong MOTD path should be UTF-8"),
        ],
    ));
    let mut overlong_client = overlong_server.wait_for_ipv4(overlong_port);
    overlong_client.write_message(&hello_message("longmotd", "room"));
    let overlong_hello = overlong_client.read_until_kind("Hello");
    assert!(message_pointer_contains(
        &overlong_hello,
        "/Hello/motd",
        "Message of the Day is too long"
    ));

    let mut invalid_json = server.connect_ipv4(port);
    invalid_json.write_json_line("{not-json");
    assert_eq!(
        invalid_json.read_until_kind("Error").kind(),
        "Error",
        "invalid JSON should return protocol error"
    );

    let mut invalid_utf8 = server.connect_ipv4(port);
    invalid_utf8.write_raw_line(&[0xff, 0xfe, b'\n']);
    assert_eq!(
        invalid_utf8.read_until_kind("Error").kind(),
        "Error",
        "invalid UTF-8 should return protocol error"
    );

    let _ = fs::remove_file(motd_file);
    let _ = fs::remove_file(invalid_motd_file);
    let _ = fs::remove_file(overlong_motd_file);
}

#[test]
fn release_verify_persistence_permanent_rooms_and_isolation() {
    let rooms_db = temporary_path("release-rooms", "sqlite3");
    let permanent_rooms = temporary_path("release-permanent-rooms", "txt");
    fs::write(&permanent_rooms, "permanent-room\n").expect("permanent rooms file should write");

    let port = reserve_ipv4_port();
    {
        let mut server = ServerProcess::spawn(&server_args(
            port,
            &[
                "--ipv4-only",
                "--interface-ipv4",
                "127.0.0.1",
                "--rooms-db-file",
                rooms_db.to_str().expect("rooms db path should be UTF-8"),
                "--permanent-rooms-file",
                permanent_rooms
                    .to_str()
                    .expect("permanent rooms path should be UTF-8"),
            ],
        ));
        let mut alice = server.wait_for_ipv4(port);
        alice.hello("alice", "persisted-room");
        let mut watcher = server.connect_ipv4(port);
        watcher.hello("watcher", "persisted-room");
        alice.read_until(|message| {
            message_pointer_eq(message, "/Set/user/watcher/event/joined", json!(true))
        });
        alice.write_message(&set_playlist_message(&["persisted.mkv"]));
        watcher.read_until(|message| {
            message_pointer_eq(
                message,
                "/Set/playlistChange/files",
                json!(["persisted.mkv"]),
            )
        });
        alice.write_message(&set_playlist_index_message(0));
        watcher.read_until(|message| {
            message_pointer_eq(message, "/Set/playlistIndex/index", json!(0))
        });
        alice.write_message(&state_message(33.0, false));
        watcher.read_until(|message| {
            message_pointer_eq(message, "/State/playstate/position", json!(33.0))
        });
        drop(alice);
        drop(watcher);
        thread::sleep(Duration::from_millis(500));
    }

    {
        let mut server = ServerProcess::spawn(&server_args(
            port,
            &[
                "--ipv4-only",
                "--interface-ipv4",
                "127.0.0.1",
                "--rooms-db-file",
                rooms_db.to_str().expect("rooms db path should be UTF-8"),
                "--permanent-rooms-file",
                permanent_rooms
                    .to_str()
                    .expect("permanent rooms path should be UTF-8"),
            ],
        ));
        let mut bob = server.wait_for_ipv4(port);
        bob.write_json_line(
            r#"{"Hello":{"username":"bob","room":{"name":"persisted-room"},"version":"1.7.5","features":{"featureList":true,"uiMode":"GUI","persistentRooms":true}}}"#,
        );
        let mut saw_playlist = false;
        let mut saw_hello = false;
        for _ in 0..8 {
            let message = bob
                .read_message()
                .expect("persistent server should respond after restart");
            saw_playlist |= message_pointer_eq(
                &message,
                "/Set/playlistChange/files",
                json!(["persisted.mkv"]),
            );
            saw_hello |= message.kind() == "Hello";
            if saw_playlist && saw_hello {
                break;
            }
        }
        assert!(
            saw_playlist,
            "persisted room should restore playlist after server restart"
        );
        bob.write_message(&ProtocolMessage::list_request());
        let rooms = expect_list_rooms(bob.read_until_kind("List"));
        assert!(rooms.contains_key("persisted-room"));
        assert!(rooms.contains_key("permanent-room"));
    }

    let isolate_port = reserve_ipv4_port();
    let mut isolate_server = ServerProcess::spawn(&server_args(
        isolate_port,
        &[
            "--ipv4-only",
            "--interface-ipv4",
            "127.0.0.1",
            "--isolate-rooms",
        ],
    ));
    let mut alice = isolate_server.wait_for_ipv4(isolate_port);
    alice.hello("alice", "room-a");
    let mut bob = isolate_server.connect_ipv4(isolate_port);
    bob.hello("bob", "room-b");
    alice.write_message(&ProtocolMessage::list_request());
    let rooms = expect_list_rooms(alice.read_until_kind("List"));
    assert!(rooms.contains_key("room-a"));
    assert!(
        !rooms.contains_key("room-b"),
        "isolate-room mode must not leak other room snapshots"
    );

    let _ = fs::remove_file(rooms_db);
    let _ = fs::remove_file(permanent_rooms);
}

#[test]
fn release_verify_tls_and_idle_timeout_behavior() {
    let cert_path = temporary_directory_path("release-tls");
    write_valid_tls_bundle(&cert_path);
    let port = reserve_ipv4_port();
    let mut server = ServerProcess::spawn(&server_args(
        port,
        &[
            "--ipv4-only",
            "--interface-ipv4",
            "127.0.0.1",
            "--tls",
            cert_path.to_str().expect("TLS cert path should be UTF-8"),
        ],
    ));
    let client = server.wait_for_ipv4(port);
    let mut tls_client = client.upgrade_to_tls();
    tls_client.write_message(&hello_message("tls-client", "tls-room"));
    assert_eq!(tls_client.read_until_kind("Hello").kind(), "Hello");

    let tls_ca_file = cert_path.join("cert.pem");
    if let Some(mut python_tls_peer) =
        PythonPeer::spawn_tls_or_skip("127.0.0.1", port, "py-tls", "tls-room", &tls_ca_file)
    {
        let python_tls_snapshot = python_tls_peer.snapshot();
        assert_eq!(
            python_tls_snapshot.get("room").and_then(Value::as_str),
            Some("tls-room")
        );
    }

    let mut logged_client = server.connect_ipv4(port);
    logged_client.hello("plain-client", "tls-room");
    logged_client.write_message(&ProtocolMessage::tls(TlsPayload::new("send")));
    let tls_denied = logged_client.read_until_kind("TLS");
    assert!(message_pointer_eq(
        &tls_denied,
        "/TLS/startTLS",
        json!("false")
    ));

    let timeout_port = reserve_ipv4_port();
    let mut timeout_server = ServerProcess::spawn(&server_args(
        timeout_port,
        &["--ipv4-only", "--interface-ipv4", "127.0.0.1"],
    ));
    let mut stale = timeout_server.wait_for_ipv4(timeout_port);
    stale.hello("stale", "timeout-room");
    let mut watcher = timeout_server.connect_ipv4(timeout_port);
    watcher.hello("watcher", "timeout-room");
    for _ in 0..23 {
        thread::sleep(Duration::from_secs(4));
        watcher.write_message(&ProtocolMessage::state(
            StatePayload::new().with_ping(
                PingPayload::new()
                    .with_client_latency_calculation(1.0)
                    .with_client_rtt(0.0),
            ),
        ));
    }
    watcher.read_until_with_limit(160, |message| {
        message_pointer_eq(message, "/Set/user/stale/event/left", json!(true))
    });

    let _ = fs::remove_dir_all(cert_path);
}

#[test]
fn release_verify_real_python_clients_against_rust_binary() {
    if !strict_release_required() {
        eprintln!("legacy Python client release verification skipped outside strict release runs");
        return;
    }

    let port = reserve_ipv4_port();
    let mut server = ServerProcess::spawn(&server_args(
        port,
        &["--ipv4-only", "--interface-ipv4", "127.0.0.1"],
    ));
    let _probe = server.wait_for_ipv4(port);

    let Some(mut alice) = PythonPeer::spawn_or_skip("127.0.0.1", port, "py-alice", "py-room", None)
    else {
        return;
    };
    let mut bob = PythonPeer::spawn_or_skip("127.0.0.1", port, "py-bob", "py-room", None)
        .expect("Python peer prerequisites were available for alice");

    bob.wait_for_user_room("py-alice", "py-room");
    alice.set_ready(true);
    bob.wait_for_user_ready("py-alice", true);
    alice.send_chat_message("python hello");
    bob.wait_for_chat_message("py-alice", "python hello");
    alice.set_file("python-file.mkv");
    bob.wait_for_user_file_name("py-alice", "python-file.mkv");
    alice.set_playlist(&["python-file.mkv", "python-next.mkv"]);
    bob.wait_for_playlist(&["python-file.mkv", "python-next.mkv"]);
    alice.set_playlist_index(1);
    bob.wait_for_playlist_index(1);
    alice.set_room("py-room-2");
    bob.wait_for_user_room("py-alice", "py-room-2");
    alice.request_controlled_room("controlled", "AB-123-456");
    alice.wait_for_local_controller(true);

    let password_port = reserve_ipv4_port();
    let mut password_server = ServerProcess::spawn(&server_args(
        password_port,
        &[
            "--ipv4-only",
            "--interface-ipv4",
            "127.0.0.1",
            "--password",
            "secret",
        ],
    ));
    let _password_probe = password_server.wait_for_ipv4(password_port);
    let mut password_peer = PythonPeer::spawn_or_skip(
        "127.0.0.1",
        password_port,
        "py-pass",
        "pass-room",
        Some("secret"),
    )
    .expect("Python peer prerequisites were available for alice");
    let password_snapshot = password_peer.snapshot();
    assert_eq!(
        password_snapshot.get("room").and_then(Value::as_str),
        Some("pass-room")
    );

    let rooms_db = temporary_path("release-python-rooms", "sqlite3");
    let persistent_port = reserve_ipv4_port();
    {
        let mut persistent_server = ServerProcess::spawn(&server_args(
            persistent_port,
            &[
                "--ipv4-only",
                "--interface-ipv4",
                "127.0.0.1",
                "--rooms-db-file",
                rooms_db.to_str().expect("rooms db path should be UTF-8"),
            ],
        ));
        let mut direct = persistent_server.wait_for_ipv4(persistent_port);
        direct.hello("seed", "python-persisted");
        direct.write_message(&set_playlist_message(&["python-persisted.mkv"]));
        direct.write_message(&set_playlist_index_message(0));
        drop(direct);
        thread::sleep(Duration::from_millis(500));
    }
    {
        let mut persistent_server = ServerProcess::spawn(&server_args(
            persistent_port,
            &[
                "--ipv4-only",
                "--interface-ipv4",
                "127.0.0.1",
                "--rooms-db-file",
                rooms_db.to_str().expect("rooms db path should be UTF-8"),
            ],
        ));
        let _persistent_probe = persistent_server.wait_for_ipv4(persistent_port);
        let mut peer = PythonPeer::spawn_or_skip(
            "127.0.0.1",
            persistent_port,
            "py-persist",
            "python-persisted",
            None,
        )
        .expect("Python peer prerequisites were available for alice");
        peer.wait_for_playlist(&["python-persisted.mkv"]);
    }

    let _ = fs::remove_file(rooms_db);
}
