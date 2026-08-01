use std::{
    collections::BTreeMap,
    fs,
    io::{Cursor, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{self, Command, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned, pki_types::ServerName};
use serde_json::{Value, json};

use super::legacy_server::run_legacy_server_fanout_roundtrip_with_full_overrides;
use super::scenario_replay::{
    replay_server_runtime_scenario_steps_with_full_overrides,
    run_python_fanout_roundtrip_with_full_overrides,
};
#[cfg(feature = "trace-capture")]
use super::trace_capture::{
    capture_legacy_server_trace_fixture_with_full_overrides,
    capture_python_trace_fixture_with_full_overrides,
};
use super::{
    DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT, InteropError, LEGACY_SERVER_STEP_IDLE_WAIT,
    LEGACY_SERVER_STEP_MAX_WAIT, LEGACY_SERVER_STEP_MIN_WAIT, LegacyClientChatSendContractCase,
    LegacyServerClientConnection, ServerRuntimeScenarioEvent, ServerRuntimeScenarioStep,
    all_protocol_fixture_names, collect_legacy_server_step_outputs, connect_legacy_client_stream,
    decode_fixture, decode_protocol_file, default_rust_client_hello_for_interop,
    default_rust_client_hello_for_legacy_live_tls, ensure_legacy_server_is_running,
    ensure_legacy_syncplay_checkout_available, ensure_repo_local_legacy_syncplay_checkout_with,
    fixture_decodes, fixture_path, legacy_server_step_collection_is_complete,
    legacy_syncplay_checkout_dir, legacy_syncplay_server_entry_script_path,
    load_server_runtime_scenario_fixture, parse_server_runtime_scenario_steps,
    prepare_legacy_server_request_line, python_bin_from_env, python_live_peer_probe_script_path,
    replay_server_runtime_scenario_fixture, replay_server_runtime_scenario_steps,
    replay_server_runtime_scenario_steps_with_motd_template,
    replay_server_runtime_scenario_steps_with_overrides, required_live_interop_enabled,
    reserve_legacy_server_port, reserve_legacy_server_port_with_lock,
    run_legacy_server_fanout_roundtrip, run_python_fanout_roundtrip,
    run_python_fanout_roundtrip_with_tls_available, run_python_handshake_roundtrip,
    run_python_legacy_client_chat_send_contract_batch,
    run_python_legacy_client_set_file_contract_probe,
    run_python_legacy_client_user_file_metadata_probe, run_python_privacy_file_payload_batch,
    run_python_protocol_roundtrip, run_python_same_fileduration_batch,
    run_python_same_fileduration_batch_with_overrides, run_python_same_filename_batch,
    run_python_same_filesize_batch, scenario_fixture_path, terminate_legacy_server_process,
    wait_for_legacy_server_startup,
};
#[cfg(feature = "trace-capture")]
use super::{
    capture_legacy_server_trace_fixture, capture_legacy_server_trace_fixture_with_overrides,
    capture_legacy_server_trace_fixture_with_salt_and_motd_template, capture_python_trace_fixture,
    capture_python_trace_fixture_with_motd_template, capture_python_trace_fixture_with_overrides,
};
use sorotte_client_core::{ClientRuntimeAction, ClientSession, PrivacyMode};
use sorotte_protocol::{
    ChatPayload, ListPayload, PlaystatePayload, ProtocolMessage, ReadyPayload, RoomRef, SetPayload,
    StatePayload, decode_message_line, encode_message_line, extract_hello_from_message,
};
use sorotte_server::ServerRuntime;

mod normalization_support;
mod scenario_constants;
mod tls_fixture_support;
use self::normalization_support::*;
use self::scenario_constants::*;
use self::tls_fixture_support::*;
mod assertions;
use self::assertions::*;

#[test]
fn legacy_server_port_lease_serializes_startup_allocation() {
    const ROLE_ENV: &str = "SOROTTE_COMPAT_SERVER_PORT_LOCK_ROLE";
    const ROOT_ENV: &str = "SOROTTE_COMPAT_SERVER_PORT_LOCK_ROOT";
    const TEST_NAME: &str = "tests::legacy_server_port_lease_serializes_startup_allocation";
    const FIXTURE_TIMEOUT: Duration = Duration::from_secs(15);

    if let Some(role) = std::env::var_os(ROLE_ENV) {
        let root = PathBuf::from(
            std::env::var_os(ROOT_ENV).expect("port-lock fixture child must receive its root"),
        );
        let lock_path = root.join("legacy-server-startup.lock");
        match role.to_string_lossy().as_ref() {
            "holder" => {
                let lease =
                    reserve_legacy_server_port_with_lock(&lock_path, FIXTURE_TIMEOUT, || {})
                        .expect("holder should acquire the startup lease");
                fs::write(root.join("holder-entered"), b"held")
                    .expect("holder should publish lock acquisition");
                assert!(
                    wait_for_compat_lock_fixture_marker(
                        &root.join("release-holder"),
                        FIXTURE_TIMEOUT,
                    ),
                    "holder timed out waiting for release"
                );
                drop(lease);
                fs::write(root.join("holder-released"), b"released")
                    .expect("holder should publish release");
            }
            "contender" => {
                let contention_marker = root.join("contender-contended");
                let lease =
                    reserve_legacy_server_port_with_lock(&lock_path, FIXTURE_TIMEOUT, || {
                        fs::write(&contention_marker, b"contended")
                            .expect("contender should publish actual process-lock contention");
                    })
                    .expect("contender should acquire after holder release");
                fs::write(root.join("contender-acquired"), lease.port().to_string())
                    .expect("contender should publish acquisition");
            }
            unexpected => panic!("unknown port-lock fixture role {unexpected:?}"),
        }
        return;
    }

    let first = reserve_legacy_server_port().expect("first startup lease should be available");
    assert!(
        TcpListener::bind(("127.0.0.1", first.port())).is_err(),
        "the lease must retain the socket reservation until child spawn"
    );

    let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
    let contender = thread::spawn(move || {
        let second =
            reserve_legacy_server_port().expect("contending startup lease should become available");
        acquired_tx
            .send(second.port())
            .expect("contending lease result should be observed");
    });
    assert!(
        acquired_rx
            .recv_timeout(Duration::from_millis(100))
            .is_err(),
        "a second startup allocator must remain blocked while the first lease is held"
    );

    drop(first);
    acquired_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("contending startup lease should acquire after release");
    contender.join().expect("contending allocator should exit");

    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "sorotte-compat-server-port-lock-{}-{suffix}",
        process::id()
    ));
    fs::create_dir(&root).expect("port-lock fixture root should be created");
    let executable = std::env::current_exe().expect("compatibility test image should resolve");
    let spawn_child = |role: &str| {
        Command::new(&executable)
            .args(["--exact", TEST_NAME, "--nocapture", "--test-threads=1"])
            .env(ROLE_ENV, role)
            .env(ROOT_ENV, &root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("port-lock fixture child should spawn")
    };

    let holder = spawn_child("holder");
    let holder_entered =
        wait_for_compat_lock_fixture_marker(&root.join("holder-entered"), FIXTURE_TIMEOUT);
    let contender = spawn_child("contender");
    let contender_contended =
        wait_for_compat_lock_fixture_marker(&root.join("contender-contended"), FIXTURE_TIMEOUT);
    let contender_acquired_before_release = root.join("contender-acquired").exists();
    fs::write(root.join("release-holder"), b"release").expect("holder release barrier should open");
    let holder_released =
        wait_for_compat_lock_fixture_marker(&root.join("holder-released"), FIXTURE_TIMEOUT);
    let contender_acquired =
        wait_for_compat_lock_fixture_marker(&root.join("contender-acquired"), FIXTURE_TIMEOUT);
    let (holder_bounded, holder_output) =
        wait_for_compat_lock_fixture_child(holder, FIXTURE_TIMEOUT);
    let (contender_bounded, contender_output) =
        wait_for_compat_lock_fixture_child(contender, FIXTURE_TIMEOUT);
    fs::remove_dir_all(&root).expect("port-lock fixture root should be removable");

    assert!(holder_entered, "holder never acquired the process lock");
    assert!(
        contender_contended,
        "contender never observed the holder's process lock"
    );
    assert!(
        !contender_acquired_before_release,
        "contender acquired before the holder released the process lock"
    );
    assert!(holder_released, "holder did not publish lock release");
    assert!(
        contender_acquired,
        "contender did not acquire after holder release"
    );
    for (role, bounded, output) in [
        ("holder", holder_bounded, holder_output),
        ("contender", contender_bounded, contender_output),
    ] {
        assert!(
            bounded && output.status.success(),
            "{role} child failed or exceeded its bound\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn step_collector_waits_for_a_delayed_required_first_frame() {
    assert!(
        !legacy_server_step_collection_is_complete(
            true,
            false,
            LEGACY_SERVER_STEP_IDLE_WAIT + Duration::from_millis(1),
            LEGACY_SERVER_STEP_IDLE_WAIT + Duration::from_millis(1),
        ),
        "required first output must not be declared idle before any frame"
    );
    assert!(
        legacy_server_step_collection_is_complete(
            false,
            false,
            LEGACY_SERVER_STEP_MIN_WAIT + LEGACY_SERVER_STEP_IDLE_WAIT,
            LEGACY_SERVER_STEP_IDLE_WAIT,
        ),
        "an intentionally silent step must retain its short quiescence boundary"
    );
    assert!(
        legacy_server_step_collection_is_complete(
            true,
            false,
            LEGACY_SERVER_STEP_MAX_WAIT,
            LEGACY_SERVER_STEP_MAX_WAIT,
        ),
        "missing required output must remain bounded by the hard deadline"
    );

    let connect_pair = || {
        let listener =
            TcpListener::bind(("127.0.0.1", 0)).expect("loopback listener should be available");
        let address = listener
            .local_addr()
            .expect("loopback listener address should resolve");
        let writer = TcpStream::connect(address).expect("loopback writer should connect");
        let (reader, _) = listener
            .accept()
            .expect("loopback collector stream should connect");
        reader
            .set_nonblocking(true)
            .expect("collector stream should become nonblocking");
        (reader, writer)
    };
    let (required_reader, mut required_writer) = connect_pair();
    let (unrelated_reader, mut unrelated_writer) = connect_pair();
    let mut clients = BTreeMap::from([
        (
            "late-client".to_owned(),
            LegacyServerClientConnection {
                stream: required_reader,
                pending_bytes: Vec::new(),
            },
        ),
        (
            "other-client".to_owned(),
            LegacyServerClientConnection {
                stream: unrelated_reader,
                pending_bytes: Vec::new(),
            },
        ),
    ]);
    unrelated_writer
        .write_all(b"{\"List\":null}\n")
        .expect("unrelated immediate framed output should be written");

    let delayed_writer = thread::spawn(move || {
        thread::sleep(LEGACY_SERVER_STEP_IDLE_WAIT + Duration::from_millis(40));
        required_writer
            .write_all(b"{\"List\":null}\n")
            .expect("delayed framed output should be written");
    });
    let outputs = collect_legacy_server_step_outputs(&mut clients, Some("late-client"))
        .expect("the delayed first frame should be collected");
    delayed_writer
        .join()
        .expect("the delayed writer should complete");

    assert_eq!(outputs.len(), 2);
    assert!(
        outputs.iter().any(|output| {
            output.client_id == "other-client" && output.line == r#"{"List":null}"#
        })
    );
    assert!(
        outputs.iter().any(|output| {
            output.client_id == "late-client" && output.line == r#"{"List":null}"#
        })
    );
}

fn wait_for_compat_lock_fixture_marker(path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while !path.is_file() {
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
    true
}

fn wait_for_compat_lock_fixture_child(
    mut child: std::process::Child,
    timeout: Duration,
) -> (bool, std::process::Output) {
    let deadline = Instant::now() + timeout;
    loop {
        match child
            .try_wait()
            .expect("lock fixture child status should be readable")
        {
            Some(_) => {
                return (
                    true,
                    child
                        .wait_with_output()
                        .expect("completed lock fixture output should be collected"),
                );
            }
            None if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            None => {
                let _ = child.kill();
                return (
                    false,
                    child
                        .wait_with_output()
                        .expect("timed-out lock fixture child should be reaped"),
                );
            }
        }
    }
}

#[test]
fn legacy_checkout_bootstrap_lock_serializes_processes() {
    const ROLE_ENV: &str = "SOROTTE_COMPAT_BOOTSTRAP_LOCK_ROLE";
    const ROOT_ENV: &str = "SOROTTE_COMPAT_BOOTSTRAP_LOCK_ROOT";
    const TEST_NAME: &str = "tests::legacy_checkout_bootstrap_lock_serializes_processes";
    const FIXTURE_TIMEOUT: Duration = Duration::from_secs(15);

    if let Some(role) = std::env::var_os(ROLE_ENV) {
        let root = PathBuf::from(
            std::env::var_os(ROOT_ENV).expect("lock fixture child must receive its root"),
        );
        let checkout = root.join("checkout");
        match role.to_string_lossy().as_ref() {
            "holder" => {
                let resolved = ensure_repo_local_legacy_syncplay_checkout_with(
                    &checkout,
                    FIXTURE_TIMEOUT,
                    || {},
                    |path| {
                        fs::write(root.join("holder-entered"), b"held")?;
                        if !wait_for_compat_lock_fixture_marker(
                            &root.join("release-holder"),
                            FIXTURE_TIMEOUT,
                        ) {
                            return Err(InteropError::Io(std::io::Error::new(
                                std::io::ErrorKind::TimedOut,
                                "holder timed out waiting for release",
                            )));
                        }
                        fs::create_dir_all(path)?;
                        fs::write(path.join("syncplayServer.py"), b"# ready\n")?;
                        Ok(())
                    },
                )
                .expect("holder should publish the ready checkout");
                assert_eq!(resolved, checkout);
                fs::write(root.join("holder-ready"), b"ready")
                    .expect("holder should publish completion");
            }
            "contender" => {
                let contention_marker = root.join("contender-contended");
                let duplicate_marker = root.join("duplicate-bootstrap");
                let resolved = ensure_repo_local_legacy_syncplay_checkout_with(
                    &checkout,
                    FIXTURE_TIMEOUT,
                    || {
                        fs::write(&contention_marker, b"contended")
                            .expect("contender should publish actual lock contention");
                    },
                    |_| {
                        fs::write(&duplicate_marker, b"duplicate")?;
                        Err(InteropError::Io(std::io::Error::other(
                            "contender must not bootstrap after the holder publishes readiness",
                        )))
                    },
                )
                .expect("contender should observe the checkout published by the holder");
                assert_eq!(resolved, checkout);
                fs::write(root.join("contender-ready"), b"ready")
                    .expect("contender should publish completion");
            }
            unexpected => panic!("unknown lock fixture role {unexpected:?}"),
        }
        return;
    }

    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "sorotte-compat-bootstrap-lock-{}-{suffix}",
        process::id()
    ));
    fs::create_dir(&root).expect("lock fixture root should be created");
    let executable = std::env::current_exe().expect("compatibility test image should resolve");
    let spawn_child = |role: &str| {
        Command::new(&executable)
            .args(["--exact", TEST_NAME, "--nocapture", "--test-threads=1"])
            .env(ROLE_ENV, role)
            .env(ROOT_ENV, &root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("lock fixture child should spawn")
    };

    let holder = spawn_child("holder");
    let holder_entered =
        wait_for_compat_lock_fixture_marker(&root.join("holder-entered"), FIXTURE_TIMEOUT);
    let contender = spawn_child("contender");
    let contender_contended =
        wait_for_compat_lock_fixture_marker(&root.join("contender-contended"), FIXTURE_TIMEOUT);
    let duplicate_before_release = root.join("duplicate-bootstrap").exists();
    fs::write(root.join("release-holder"), b"release").expect("holder release barrier should open");
    let holder_ready =
        wait_for_compat_lock_fixture_marker(&root.join("holder-ready"), FIXTURE_TIMEOUT);
    let contender_ready =
        wait_for_compat_lock_fixture_marker(&root.join("contender-ready"), FIXTURE_TIMEOUT);
    let (holder_bounded, holder_output) =
        wait_for_compat_lock_fixture_child(holder, FIXTURE_TIMEOUT);
    let (contender_bounded, contender_output) =
        wait_for_compat_lock_fixture_child(contender, FIXTURE_TIMEOUT);
    let duplicate_after_release = root.join("duplicate-bootstrap").exists();
    fs::remove_dir_all(&root).expect("lock fixture root should be removable");

    assert!(holder_entered, "holder never entered the bootstrap seam");
    assert!(
        contender_contended,
        "contender never observed the holder's process lock"
    );
    assert!(
        !duplicate_before_release && !duplicate_after_release,
        "contender executed a duplicate bootstrap action"
    );
    for (role, bounded, output) in [
        ("holder", holder_bounded, holder_output),
        ("contender", contender_bounded, contender_output),
    ] {
        assert!(
            bounded && output.status.success(),
            "{role} child failed or exceeded its bound\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(
        holder_ready,
        "holder did not observe its published checkout"
    );
    assert!(
        contender_ready,
        "contender did not observe readiness after acquiring the released lock"
    );
}

#[test]
fn compatibility_scenario_debug_does_not_print_raw_protocol_lines() {
    let secret = "compat-wire-password-canary";
    let step = ServerRuntimeScenarioStep {
        client_id: "client-1".to_owned(),
        request_line: format!(r#"{{\"Hello\":{{\"password\":\"{secret}\"}}}}"#),
        advance_seconds: 0.0,
        legacy_advance_seconds: None,
    };

    let debug = format!("{step:?}");
    assert!(debug.contains("request_line_bytes"));
    assert!(!debug.contains(secret));
}

#[test]
fn compatibility_scenario_parses_distinct_runtime_and_legacy_time_advances() {
    let steps = parse_server_runtime_scenario_steps(
        r#"{"client":"client-1","advanceSeconds":88.0,"legacyAdvanceSeconds":10.0,"message":{"List":null}}"#,
    )
    .expect("dual-clock compatibility step should parse");
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].advance_seconds, 88.0);
    assert_eq!(steps[0].legacy_advance_seconds, Some(10.0));

    let invalid = parse_server_runtime_scenario_steps(
        r#"{"client":"client-1","advanceSeconds":88.0,"legacyAdvanceSeconds":-1.0,"message":{"List":null}}"#,
    );
    assert!(matches!(invalid, Err(InteropError::InvalidScenarioStep(_))));
}

#[test]
fn legacy_server_request_shim_synthesizes_python_version_defaults_for_omitted_features() {
    let prepared = prepare_legacy_server_request_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255","realversion":"1.5.0"}}"#,
    )
    .expect("omitted-feature Hello should be prepared");
    let prepared: Value =
        serde_json::from_str(&prepared).expect("prepared Hello should remain valid JSON");

    assert_eq!(
        prepared.pointer("/Hello/features"),
        Some(&json!({
            "sharedPlaylists": true,
            "chat": true,
            "featureList": false,
            "readiness": true,
            "managedRooms": true,
            "persistentRooms": false,
            "uiMode": "Unknown",
        }))
    );
}

#[test]
fn legacy_server_request_shim_preserves_explicit_features() {
    let request = r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"9.9.9","features":{"uiMode":"CLI","chat":false}}}"#;
    let prepared =
        prepare_legacy_server_request_line(request).expect("explicit-feature Hello should prepare");

    assert_eq!(
        serde_json::from_str::<Value>(&prepared).expect("prepared Hello should decode"),
        serde_json::from_str::<Value>(request).expect("original Hello should decode")
    );
}

mod chat_fanout_tests;
mod controlled_room_fanout_tests;
mod fixture_tests;
mod legacy_client_contract_tests;
mod legacy_tls_tests;
mod normalization_tests;
mod playlist_fanout_tests;
mod python_protocol_tests;
mod rooms_motd_fanout_tests;
mod scenario_replay_tests;
mod state_fanout_tests;
mod trace_shape_tests;
