use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
    process,
    sync::Arc,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rusqlite::Connection;
use rustls::{ClientConfig, RootCertStore, pki_types::ServerName};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::{Mutex, mpsc, watch},
    time::timeout,
};
use tokio_rustls::TlsConnector;

use super::{
    DEFAULT_MAX_FILENAME_LENGTH, DEFAULT_MAX_ROOM_NAME_LENGTH, DEFAULT_PLAYLIST_MAX_ITEMS,
    DirectedOutboundLine, DirectedTransportAction, LEGACY_PERSISTENT_ROOMS_NOTICE,
    LEGACY_SERVER_LINE_DECODE_ERROR, LEGACY_SERVER_PASSWORD_REQUIRED_ERROR,
    LEGACY_SERVER_WRONG_PASSWORD_ERROR, LEGACY_UI_MODE_UNKNOWN, RoomPasswordCheckError,
    RoomPasswordProvider, SERVER_REAL_VERSION, SERVER_STATE_INTERVAL_SECONDS, ServerApp,
    ServerRuntime, ServerRuntimeDispatch, ServerRuntimeError, ServerTransportAction,
    TLS_CERT_ROTATION_MAX_RETRIES, default_motd_for_client_version, motd_for_client_version,
    read_network_line_from_stream, run_server_network_loop_until_shutdown,
};
use syncplay_protocol::{
    ChatPayload, ListPayload, PlaylistChangePayload, ProtocolMessage, SetPayload,
    decode_message_line, extract_hello_from_message,
};

const TEST_TLS_CERT_PEM: &str = include_str!("../../../fixtures/tls/test_cert.pem");
const TEST_TLS_CHAIN_PEM: &str = include_str!("../../../fixtures/tls/test_chain.pem");
const TEST_TLS_PRIVATE_KEY_PEM: &str = include_str!("../../../fixtures/tls/test_privkey.pem");

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
                    .is_some_and(|ignore| ignore.server == Some(1))
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
                        .is_some_and(|ignore| ignore.server == Some(1))
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

fn controlled_room_name_for_test(base_room: &str, password: &str) -> String {
    super::controlled_room_name_for(base_room, password)
}

fn controlled_room_name_for_salt_test(base_room: &str, password: &str, salt: &str) -> String {
    RoomPasswordProvider::new(salt).controlled_room_name_for(base_room, password)
}

fn temporary_sqlite_path(label: &str) -> PathBuf {
    let now_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "syncplay-rs-{label}-{}-{now_nanos}.sqlite3",
        process::id()
    ))
}

fn temporary_text_path(label: &str) -> PathBuf {
    let now_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "syncplay-rs-{label}-{}-{now_nanos}.txt",
        process::id()
    ))
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
    std::env::temp_dir().join(format!("syncplay-rs-{label}-{}-{now_nanos}", process::id()))
}

fn write_valid_tls_bundle(path: &Path) {
    fs::write(path.join("privkey.pem"), TEST_TLS_PRIVATE_KEY_PEM)
        .expect("valid private key fixture should write");
    fs::write(path.join("cert.pem"), TEST_TLS_CERT_PEM)
        .expect("valid certificate fixture should write");
    fs::write(path.join("chain.pem"), TEST_TLS_CHAIN_PEM)
        .expect("valid chain fixture should write");
}

fn tls_client_connector_for_test_fixture() -> TlsConnector {
    let mut cert_reader = io::BufReader::new(TEST_TLS_CERT_PEM.as_bytes());
    let certs = rustls_pemfile::certs(&mut cert_reader)
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

fn overwrite_file_until_modified_time_changes(path: &Path, contents: &str) {
    let original_modified_time = fs::metadata(path)
        .expect("file should be readable before overwrite")
        .modified()
        .expect("file should expose modification time");
    for attempt in 0..8 {
        fs::write(path, format!("{contents}-{attempt}"))
            .expect("file overwrite should succeed while testing rotation");
        let updated_modified_time = fs::metadata(path)
            .expect("overwritten file should be readable")
            .modified()
            .expect("overwritten file should expose modification time");
        if updated_modified_time != original_modified_time {
            return;
        }
        thread::sleep(Duration::from_millis(250));
    }
    panic!("file modification time did not change after repeated overwrite attempts");
}

fn rewrite_file_until_modified_time_changes(path: &Path, contents: &str) {
    let original_modified_time = fs::metadata(path)
        .expect("file should be readable before overwrite")
        .modified()
        .expect("file should expose modification time");
    for _ in 0..8 {
        fs::write(path, contents)
            .expect("file rewrite should succeed while testing rotation recovery");
        let updated_modified_time = fs::metadata(path)
            .expect("rewritten file should be readable")
            .modified()
            .expect("rewritten file should expose modification time");
        if updated_modified_time != original_modified_time {
            return;
        }
        thread::sleep(Duration::from_millis(250));
    }
    panic!("file modification time did not change after repeated rewrite attempts");
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
mod persistence_tests;
mod runtime_config_tests;
mod session_tests;
mod state_tests;
