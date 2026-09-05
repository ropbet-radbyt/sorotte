use rustls_pki_types::pem::PemObject;
use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
    process,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rusqlite::Connection;
use rustls::{ClientConfig, RootCertStore, pki_types::ServerName};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::{Mutex, mpsc, watch},
    time::timeout,
};
use tokio_rustls::TlsConnector;

use super::{
    DEFAULT_CONTROLLED_ROOM_HASH_SALT, DEFAULT_MAX_FILENAME_LENGTH, DEFAULT_MAX_ROOM_NAME_LENGTH,
    DEFAULT_PLAYLIST_MAX_ITEMS, DirectedOutboundLine, DirectedTransportAction,
    INITIAL_SERVER_STATE_DELAY_SECONDS, LEGACY_PERSISTENT_ROOMS_NOTICE,
    LEGACY_SERVER_LINE_DECODE_ERROR, LEGACY_SERVER_PASSWORD_REQUIRED_ERROR,
    LEGACY_SERVER_WRONG_PASSWORD_ERROR, LEGACY_UI_MODE_UNKNOWN, PersistedRoomState,
    RoomPasswordProvider, RoomPlaylistState, SERVER_REAL_VERSION, SERVER_STATE_INTERVAL_SECONDS,
    ServerActorError, ServerActorHandle, ServerApp, ServerInboundCommand, ServerLifecycleError,
    ServerNetworkError, ServerOutboundDelivery, ServerPersistenceEffect, ServerRuntime,
    ServerRuntimeDispatch, ServerRuntimeError, ServerSetCommand, ServerSharedFile,
    ServerTransportAction, TLS_CERT_ROTATION_MAX_RETRIES, TlsCertificateBundleMetadataClock,
    default_motd_for_client_version, load_tls_server_config_from_snapshot, motd_for_client_context,
    motd_for_client_version, read_network_line_from_stream, read_tls_certificate_bundle_snapshot,
    read_tls_certificate_bundle_snapshot_with_test_hook,
    read_tls_certificate_bundle_snapshot_with_test_reader, run_server_network_loop_until_shutdown,
    run_server_network_loops_and_shutdown_actor, tls_certificate_bundle_fingerprint,
};
use sorotte_protocol::{
    ChatPayload, ListPayload, PlaylistChangePayload, PlaylistIndexPayload, ProtocolMessage,
    SetPayload, decode_message_line, extract_hello_from_message,
};

mod frame_capacity_tests;
mod ping_timing_tests;

// Projection unit tests install a previously issued challenge as a fixture.
// Actual issuance, wire roundtrips, replay and reconnect are tested separately
// in ping_timing_tests; these tests focus on consuming the resulting estimate.
fn fixture_issued_ping_echo(
    runtime: &mut ServerRuntime,
    client: &str,
    timestamp: f64,
    sender_rtt: f64,
) {
    let rtt = runtime.current_time_seconds() - timestamp;
    let sent_at = runtime.local_time_seconds() - rtt;
    runtime
        .client_state_counters
        .get_mut(client)
        .unwrap()
        .outstanding_ping_challenges
        .push_back((timestamp, sent_at));
    runtime.ingest_client_ping_metrics(client, Some(timestamp), Some(sender_rtt));
}

const TEST_TLS_CERT_PEM: &str = include_str!("../../../fixtures/tls/test_cert.pem");
const TEST_TLS_CHAIN_PEM: &str = include_str!("../../../fixtures/tls/test_chain.pem");
const TEST_TLS_PRIVATE_KEY_PEM: &str = include_str!("../../../fixtures/tls/test_privkey.pem");

#[test]
fn tokenized_media_debug_canary_is_redacted_across_server_domain_carriers() {
    const MARKER: &str = "server-media-secret-canary-817c25";
    let target = format!("https://media.example/video?X-Plex-Token={MARKER}");
    let mut runtime = ServerRuntime::default();
    runtime.room_playlists.insert(
        "room".to_owned(),
        RoomPlaylistState {
            files: vec![target.clone()],
            index: Some(0),
            epoch: 0,
        },
    );
    let shared_file = ServerSharedFile {
        name: Some(target.clone()),
        ..ServerSharedFile::default()
    };
    let debug_values = [
        format!("{runtime:?}"),
        format!("{shared_file:?}"),
        format!(
            "{:?}",
            ServerInboundCommand::Set(vec![ServerSetCommand::PlaylistChange(vec![target.clone()])])
        ),
        format!(
            "{:?}",
            ServerInboundCommand::Set(vec![ServerSetCommand::File(Some(shared_file))])
        ),
        format!(
            "{:?}",
            ServerPersistenceEffect::SaveRoom {
                room_name: "room".to_owned(),
                files: vec![target.clone()],
                playlist_index: Some(0),
                position: 0.0,
                last_activity_at_seconds: 1.0,
                owner_bucket: None,
                created_at_seconds: 0.0,
                version: 1,
            }
        ),
        format!(
            "{:?}",
            PersistedRoomState {
                files: vec![target],
                index: Some(0),
                position: 0.0,
                last_activity_at_seconds: 1.0,
                version: 0,
                owner_bucket: None,
                created_at_seconds: 0.0,
            }
        ),
    ];

    for debug in debug_values {
        assert!(!debug.contains(MARKER), "leaky Debug output: {debug}");
    }
}

fn decode_directed_lines(lines: &[DirectedOutboundLine]) -> Vec<(String, ProtocolMessage)> {
    lines
        .iter()
        .map(|line| {
            let message = decode_message_line(&line.line)
                .expect("directed outbound line should decode as protocol message");
            (line.client_id.clone(), message)
        })
        .collect()
}

fn acknowledge_server_state_counter(
    runtime: &mut ServerRuntime,
    client_id: &str,
    server_counter: u32,
) {
    let ack = format!(r#"{{"State":{{"ignoringOnTheFly":{{"server":{server_counter}}}}}}}"#);
    runtime
        .handle_line_fanout(client_id, &ack)
        .expect("server state counter ack should be accepted");
}

fn acknowledge_directed_state_counters(
    runtime: &mut ServerRuntime,
    directed_messages: &[(String, ProtocolMessage)],
) {
    let counters: Vec<_> = directed_messages
        .iter()
        .filter_map(|(client_id, message)| {
            let ProtocolMessage::State(payload) = message else {
                return None;
            };
            let server_counter = payload
                .state
                .ignoring_on_the_fly
                .as_ref()
                .and_then(|ignore| ignore.server)?;
            Some((client_id.clone(), server_counter))
        })
        .collect();
    for (client_id, server_counter) in counters {
        acknowledge_server_state_counter(runtime, &client_id, server_counter);
    }
}

fn acknowledge_outbound_state_counters(
    runtime: &mut ServerRuntime,
    client_id: &str,
    outbound_lines: &[String],
) {
    let directed_messages: Vec<_> = outbound_lines
        .iter()
        .filter_map(|line| decode_message_line(line).ok())
        .map(|message| (client_id.to_owned(), message))
        .collect();
    acknowledge_directed_state_counters(runtime, &directed_messages);
}

fn has_user_event(
    directed_messages: &[(String, ProtocolMessage)],
    recipient: &str,
    username: &str,
    event: &str,
) -> bool {
    directed_messages.iter().any(|(client_id, message)| {
        if client_id != recipient {
            return false;
        }
        match message {
            ProtocolMessage::Set(payload) => {
                payload
                    .set
                    .user
                    .as_ref()
                    .and_then(|users| users.get(username))
                    .and_then(|user| user.event.as_ref())
                    .and_then(|event_value| event_value.get(event))
                    .and_then(Value::as_bool)
                    == Some(true)
            }
            _ => false,
        }
    })
}

fn has_user_room_update(
    directed_messages: &[(String, ProtocolMessage)],
    recipient: &str,
    username: &str,
    room: &str,
) -> bool {
    directed_messages.iter().any(|(client_id, message)| {
        if client_id != recipient {
            return false;
        }
        match message {
            ProtocolMessage::Set(payload) => payload
                .set
                .user
                .as_ref()
                .and_then(|users| users.get(username))
                .and_then(|user| user.room.as_ref())
                .is_some_and(|room_ref| room_ref.name == room),
            _ => false,
        }
    })
}

fn has_user_file_update(
    directed_messages: &[(String, ProtocolMessage)],
    recipient: &str,
    username: &str,
    filename: &str,
) -> bool {
    directed_messages.iter().any(|(client_id, message)| {
        if client_id != recipient {
            return false;
        }
        match message {
            ProtocolMessage::Set(payload) => {
                payload
                    .set
                    .user
                    .as_ref()
                    .and_then(|users| users.get(username))
                    .and_then(|user| user.file.as_ref())
                    .and_then(|file| file.get("name"))
                    .and_then(Value::as_str)
                    == Some(filename)
            }
            _ => false,
        }
    })
}

fn has_ready_update(
    directed_messages: &[(String, ProtocolMessage)],
    recipient: &str,
    username: &str,
    is_ready: bool,
) -> bool {
    has_ready_update_state(directed_messages, recipient, username, Some(is_ready))
}

fn has_ready_update_state(
    directed_messages: &[(String, ProtocolMessage)],
    recipient: &str,
    username: &str,
    is_ready: Option<bool>,
) -> bool {
    directed_messages.iter().any(|(client_id, message)| {
        if client_id != recipient {
            return false;
        }
        match message {
            ProtocolMessage::Set(payload) => payload.set.ready.as_ref().is_some_and(|ready| {
                ready.username.as_deref() == Some(username) && ready.is_ready == is_ready
            }),
            _ => false,
        }
    })
}

fn has_state_update(
    directed_messages: &[(String, ProtocolMessage)],
    recipient: &str,
    set_by_username: &str,
    position: f64,
    paused: bool,
    do_seek: bool,
) -> bool {
    directed_messages.iter().any(|(client_id, message)| {
        if client_id != recipient {
            return false;
        }
        match message {
            ProtocolMessage::State(payload) => {
                payload.state.playstate.as_ref().is_some_and(|playstate| {
                    playstate.set_by.as_deref() == Some(set_by_username)
                        && playstate
                            .position
                            .is_some_and(|actual| (actual - position).abs() <= 0.000_001)
                        && playstate.paused == Some(paused)
                        && playstate.do_seek == Some(do_seek)
                }) && payload.state.ping.as_ref().is_some_and(|ping| {
                    ping.latency_calculation.is_some() && ping.server_rtt == Some(0.0)
                }) && payload
                    .state
                    .ignoring_on_the_fly
                    .as_ref()
                    .is_some_and(|ignore| ignore.server.is_some())
            }
            _ => false,
        }
    })
}

fn has_room_sync_state_update(
    directed_messages: &[(String, ProtocolMessage)],
    recipient: &str,
    do_seek: bool,
) -> bool {
    directed_messages.iter().any(|(client_id, message)| {
        if client_id != recipient {
            return false;
        }
        match message {
            ProtocolMessage::State(payload) => {
                payload.state.playstate.as_ref().is_some_and(|playstate| {
                    playstate.set_by.is_none()
                        && playstate.position == Some(0.0)
                        && playstate.paused == Some(true)
                        && playstate.do_seek == Some(do_seek)
                }) && payload.state.ping.as_ref().is_some_and(|ping| {
                    ping.latency_calculation.is_some() && ping.server_rtt == Some(0.0)
                }) && if do_seek {
                    payload
                        .state
                        .ignoring_on_the_fly
                        .as_ref()
                        .is_some_and(|ignore| ignore.server.is_some())
                } else {
                    payload.state.ignoring_on_the_fly.is_none()
                }
            }
            _ => false,
        }
    })
}

fn room_seek_sync_server_counters(
    directed_messages: &[(String, ProtocolMessage)],
    recipient: &str,
) -> Vec<u32> {
    directed_messages
        .iter()
        .filter_map(|(client_id, message)| {
            if client_id != recipient {
                return None;
            }
            let ProtocolMessage::State(payload) = message else {
                return None;
            };
            let playstate = payload.state.playstate.as_ref()?;
            if playstate.set_by.is_some()
                || playstate.position != Some(0.0)
                || playstate.paused != Some(true)
                || playstate.do_seek != Some(true)
            {
                return None;
            }
            payload
                .state
                .ignoring_on_the_fly
                .as_ref()
                .and_then(|ignore| ignore.server)
        })
        .collect()
}

fn has_playlist_snapshot(
    directed_messages: &[(String, ProtocolMessage)],
    recipient: &str,
    files: &[&str],
) -> bool {
    directed_messages.iter().any(|(client_id, message)| {
        if client_id != recipient {
            return false;
        }
        match message {
            ProtocolMessage::Set(payload) => {
                payload
                    .set
                    .playlist_change
                    .as_ref()
                    .is_some_and(|playlist| {
                        playlist
                            .files
                            .iter()
                            .map(String::as_str)
                            .eq(files.iter().copied())
                            && playlist.user.is_none()
                            && playlist.user_is_null
                    })
            }
            _ => false,
        }
    })
}

fn has_playlist_snapshot_with_user(
    directed_messages: &[(String, ProtocolMessage)],
    recipient: &str,
    files: &[&str],
    user: &str,
) -> bool {
    directed_messages.iter().any(|(client_id, message)| {
        if client_id != recipient {
            return false;
        }
        match message {
            ProtocolMessage::Set(payload) => {
                payload
                    .set
                    .playlist_change
                    .as_ref()
                    .is_some_and(|playlist| {
                        playlist
                            .files
                            .iter()
                            .map(String::as_str)
                            .eq(files.iter().copied())
                            && playlist.user.as_deref() == Some(user)
                    })
            }
            _ => false,
        }
    })
}

fn has_playlist_index_snapshot(
    directed_messages: &[(String, ProtocolMessage)],
    recipient: &str,
    index: i64,
) -> bool {
    directed_messages.iter().any(|(client_id, message)| {
        if client_id != recipient {
            return false;
        }
        match message {
            ProtocolMessage::Set(payload) => payload
                .set
                .playlist_index
                .as_ref()
                .is_some_and(|playlist_index| playlist_index.index == index),
            _ => false,
        }
    })
}

fn has_null_playlist_index_snapshot(
    directed_messages: &[(String, ProtocolMessage)],
    recipient: &str,
) -> bool {
    directed_messages.iter().any(|(client_id, message)| {
        if client_id != recipient {
            return false;
        }
        match message {
            ProtocolMessage::Set(payload) => {
                payload
                    .set
                    .playlist_index
                    .as_ref()
                    .is_some_and(|playlist_index| {
                        playlist_index.index_value().is_none()
                            && playlist_index.user.is_none()
                            && playlist_index.user_is_null
                    })
            }
            _ => false,
        }
    })
}

fn controlled_room_name_for_test(base_room: &str, password: &str) -> String {
    super::controlled_room_name_for(base_room, password)
}

fn controlled_room_name_for_salt_test(base_room: &str, password: &str, salt: &str) -> String {
    RoomPasswordProvider::new(salt).controlled_room_name_for(base_room, password)
}

fn server_runtime_with_default_controlled_room_salt_for_test() -> ServerRuntime {
    ServerRuntime::with_room_password_salt(DEFAULT_CONTROLLED_ROOM_HASH_SALT)
}

fn temporary_sqlite_path(label: &str) -> PathBuf {
    let now_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "sorotte-{label}-{}-{now_nanos}.sqlite3",
        process::id()
    ))
}

fn temporary_text_path(label: &str) -> PathBuf {
    let now_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("sorotte-{label}-{}-{now_nanos}.txt", process::id()))
}

fn load_stats_snapshot_rows(path: &PathBuf) -> Vec<(i64, String)> {
    let connection = Connection::open(path).expect("stats sqlite db should be openable");
    let mut statement = connection
        .prepare(
            "SELECT snapshot_time, version \
             FROM clients_snapshots \
             ORDER BY snapshot_time, version, rowid",
        )
        .expect("stats snapshot query should prepare");
    let rows = statement
        .query_map([], |row| {
            let snapshot_time: i64 = row.get(0)?;
            let version: String = row.get(1)?;
            Ok((snapshot_time, version))
        })
        .expect("stats snapshot rows should query");
    rows.collect::<Result<Vec<_>, _>>()
        .expect("stats snapshot rows should decode")
}

fn temporary_directory_path(label: &str) -> PathBuf {
    let now_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("sorotte-{label}-{}-{now_nanos}", process::id()))
}

fn write_valid_tls_bundle(path: &Path) {
    fs::write(path.join("privkey.pem"), TEST_TLS_PRIVATE_KEY_PEM)
        .expect("valid private key fixture should write");
    fs::write(path.join("cert.pem"), TEST_TLS_CERT_PEM)
        .expect("valid certificate fixture should write");
    fs::write(path.join("chain.pem"), TEST_TLS_CHAIN_PEM)
        .expect("valid chain fixture should write");
}

fn write_invalid_tls_bundle(path: &Path, label: &str) {
    for filename in super::TLS_REQUIRED_CERT_FILENAMES {
        fs::write(path.join(filename), format!("invalid-{label}-{filename}"))
            .expect("invalid TLS bundle fixture should write");
    }
}

fn set_file_modified_time_for_test(path: &Path, modified_time: SystemTime) {
    let file = fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("TLS bundle member should open for timestamp update");
    file.set_times(fs::FileTimes::new().set_modified(modified_time))
        .expect("TLS bundle member modification time should be settable");
    assert_eq!(
        fs::metadata(path)
            .expect("timestamped TLS bundle member should remain readable")
            .modified()
            .expect("TLS bundle member should expose modification time"),
        modified_time,
        "filesystem must preserve the explicit test timestamp"
    );
}

fn server_runtime_with_tls_metadata_clock(
    cert_path: &Path,
) -> (ServerRuntime, TlsCertificateBundleMetadataClock) {
    let metadata_clock = TlsCertificateBundleMetadataClock::new();
    let mut runtime = ServerRuntime::new();
    runtime.set_tls_certificate_bundle_metadata_clock_for_test(metadata_clock.clone());
    runtime.set_tls_cert_path(Some(cert_path.to_path_buf()));
    (runtime, metadata_clock)
}

fn tls_client_connector_for_test_fixture() -> TlsConnector {
    let mut cert_reader = io::BufReader::new(TEST_TLS_CERT_PEM.as_bytes());
    let certs = rustls_pki_types::CertificateDer::pem_reader_iter(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .expect("test certificate fixture should parse");
    let mut roots = RootCertStore::empty();
    for cert in certs {
        roots
            .add(cert)
            .expect("test certificate should be addable to root store");
    }
    let client_config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    TlsConnector::from(Arc::new(client_config))
}

fn tls_start_response(lines: &[String]) -> Option<String> {
    lines.iter().find_map(|line| {
        let message = decode_message_line(line).ok()?;
        let ProtocolMessage::Tls(payload) = message else {
            return None;
        };
        Some(payload.tls.start_tls)
    })
}

fn has_start_tls_transport_action(actions: &[DirectedTransportAction], recipient: &str) -> bool {
    actions.iter().any(|action| {
        action.client_id == recipient && action.action == ServerTransportAction::StartTls
    })
}

fn has_close_transport_action(actions: &[DirectedTransportAction], recipient: &str) -> bool {
    actions.iter().any(|action| {
        action.client_id == recipient && action.action == ServerTransportAction::Close
    })
}

fn dispatch_error_message(dispatch: &ServerRuntimeDispatch) -> Option<String> {
    dispatch.outbound_lines.iter().find_map(|line| {
        let message = decode_message_line(&line.line).ok()?;
        let ProtocolMessage::Error(payload) = message else {
            return None;
        };
        Some(payload.error.message)
    })
}

mod controller_playlist_tests;
mod network_tests;
mod participant_status_tests;
mod persistence_platform_syscall_fault_tests;
mod persistence_power_loss_harness_tests;
mod persistence_tests;
mod playback_barrier_tests;
mod raw_protocol_framing_tests;
mod readiness_v2_tests;
mod runtime_config_tests;
mod session_tests;
mod state_tests;
mod tls_snapshot_fault_tests;
