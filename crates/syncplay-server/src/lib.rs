use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
    time::{SystemTime, UNIX_EPOCH},
};

use md5::Md5;
use regex::Regex;
use rusqlite::{Connection, params};
use rustls::{
    ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer},
};
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use sha2::Sha256;
use syncplay_core::{DomainError, SyncDomain};
use syncplay_protocol::{
    ChatPayload, ControllerAuthPayload, HelloPayload, IgnoringOnTheFlyPayload, ListPayload,
    ListUserEntry, NewControlledRoomPayload, PingPayload, PlaylistChangePayload,
    PlaylistIndexPayload, PlaystatePayload, ProtocolError, ProtocolMessage, ReadyPayload, RoomRef,
    SetPayload, StatePayload, TlsPayload, UserSetPayload, decode_message_line, encode_message_line,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{
        Mutex,
        mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
        watch,
    },
    task::JoinHandle,
    time,
};
use tokio_rustls::{TlsAcceptor, server::TlsStream};

const SERVER_REAL_VERSION: &str = "syncplay-rs-dev-server";
const LEGACY_COMPAT_SERVER_VERSION: &str = "1.7.5";
const LEGACY_COMPAT_UPGRADE_URL: &str = "https://syncplay.pl";
const DEFAULT_OUTDATED_MOTD_TEMPLATE: &str =
    "You are using Syncplay {client_version} but a newer version is available from {upgrade_url}";
const LEGACY_PERSISTENT_ROOMS_NOTICE: &str = "NOTICE: This server uses persistent rooms, which means that the playlist information is stored between playback sessions. If you want to create a room where information is not saved then put -temp at the end of the room name.";
const LEGACY_SERVER_PASSWORD_REQUIRED_ERROR: &str = "Password required";
const LEGACY_SERVER_WRONG_PASSWORD_ERROR: &str = "Wrong password supplied";
const LEGACY_UI_MODE_GRAPHICAL: &str = "GUI";
const LEGACY_UI_MODE_UNKNOWN: &str = "Unknown";
const DEFAULT_CONTROLLED_ROOM_HASH_SALT: &str = "syncplay-rs-controlled-room-v1";
const DEFAULT_MAX_CHAT_MESSAGE_LENGTH: usize = 150;
const DEFAULT_MAX_USERNAME_LENGTH: usize = 16;
const SERVER_STATE_INTERVAL_SECONDS: f64 = 1.0;
const PROTOCOL_TIMEOUT_SECONDS: f64 = 12.5;
const PING_MOVING_AVERAGE_WEIGHT: f64 = 0.85;
const SERVER_STATS_SNAPSHOT_INTERVAL_SECONDS: f64 = 3600.0;
const SERVER_STATS_DELAY_STEP_SECONDS: f64 = 5.0;
const SERVER_NETWORK_TICK_INTERVAL_SECONDS: f64 = 0.25;
const TLS_CERT_FILENAME: &str = "cert.pem";
const TLS_REQUIRED_CERT_FILENAMES: [&str; 3] = ["privkey.pem", "cert.pem", "chain.pem"];
const TLS_CERT_ROTATION_MAX_RETRIES: u32 = 10;

mod app;
mod auth;
mod compat;
mod messages;
mod network;
mod persistence;
mod runtime_api;
mod runtime_handlers;
mod runtime_maintenance;
mod tls;

pub use app::ServerApp;
pub use network::run_server_network_loop_until_shutdown;
pub use persistence::{RoomPersistenceError, StatsPersistenceError};

pub(crate) use auth::{RoomPasswordCheckError, RoomPasswordProvider};
pub(crate) use compat::*;
pub(crate) use messages::*;
#[cfg(test)]
pub(crate) use network::read_network_line_from_stream;
pub(crate) use persistence::{PersistedRoomState, RoomPersistenceStore, StatsPersistenceStore};
pub(crate) use tls::{
    load_tls_server_config, tls_certificate_bundle_is_available, tls_certificate_file_modified_time,
};

#[derive(Debug, Clone, PartialEq)]
pub struct ServerSession {
    pub username: String,
    pub room: String,
    pub version: String,
    pub features: Option<Value>,
}

#[derive(Debug, thiserror::Error)]
pub enum ServerRuntimeError {
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error(transparent)]
    Domain(#[from] DomainError),
    #[error(transparent)]
    StatsPersistence(#[from] StatsPersistenceError),
    #[error(transparent)]
    RoomPersistence(#[from] RoomPersistenceError),
    #[error("failed to read permanent rooms file '{path}': {source}")]
    PermanentRoomsFileRead {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("missing session for client id '{0}'")]
    MissingSession(String),
    #[error("hello payload is missing required username, room, or version")]
    InvalidHello,
}

#[derive(Debug, thiserror::Error)]
pub enum ServerNetworkError {
    #[error(transparent)]
    Runtime(#[from] ServerRuntimeError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

#[derive(Debug, Clone, PartialEq)]
pub struct DirectedProtocolMessage {
    pub client_id: String,
    pub message: ProtocolMessage,
}

impl DirectedProtocolMessage {
    fn new(client_id: impl Into<String>, message: ProtocolMessage) -> Self {
        Self {
            client_id: client_id.into(),
            message,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectedOutboundLine {
    pub client_id: String,
    pub line: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerRuntimeDispatch {
    pub outbound_lines: Vec<DirectedOutboundLine>,
    pub transport_actions: Vec<DirectedTransportAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerTransportAction {
    StartTls,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectedTransportAction {
    pub client_id: String,
    pub action: ServerTransportAction,
}

impl DirectedTransportAction {
    fn new(client_id: impl Into<String>, action: ServerTransportAction) -> Self {
        Self {
            client_id: client_id.into(),
            action,
        }
    }
}

type ClientLineSender = UnboundedSender<String>;
type SharedClientLineSenders = Arc<Mutex<BTreeMap<String, ClientLineSender>>>;

#[derive(Debug)]
pub struct ServerRuntime {
    domain: SyncDomain,
    sessions: BTreeMap<String, ServerSession>,
    room_controllers: BTreeMap<String, BTreeSet<String>>,
    room_playlists: BTreeMap<String, RoomPlaylistState>,
    room_playback_states: BTreeMap<String, RoomPlaybackState>,
    client_state_counters: BTreeMap<String, ClientStateCounters>,
    client_last_state_update_at: BTreeMap<String, f64>,
    client_next_periodic_state_at: BTreeMap<String, f64>,
    time_now_override_seconds: Option<f64>,
    room_password_provider: RoomPasswordProvider,
    server_password_token: Option<String>,
    motd_template: Option<String>,
    stats_persistence: Option<StatsPersistenceStore>,
    stats_snapshot_start_delay_seconds: f64,
    stats_snapshot_interval_seconds: f64,
    stats_next_snapshot_at_seconds: Option<f64>,
    tls_cert_path: Option<PathBuf>,
    tls_server_config: Option<Arc<ServerConfig>>,
    tls_context_available: bool,
    server_accepts_tls: bool,
    tls_last_edit_cert_time: Option<SystemTime>,
    tls_rotation_attempts: u32,
    pending_transport_actions: Vec<DirectedTransportAction>,
    persistent_rooms_enabled: bool,
    isolate_rooms: bool,
    chat_enabled: bool,
    readiness_enabled: bool,
    max_chat_message_length: usize,
    max_username_length: usize,
    room_persistence: Option<RoomPersistenceStore>,
    permanent_rooms: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct RoomPlaylistState {
    files: Vec<String>,
    index: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
struct RoomPlaybackState {
    position: f64,
    paused: bool,
    set_by: Option<String>,
}

impl Default for RoomPlaybackState {
    fn default() -> Self {
        Self {
            position: 0.0,
            paused: true,
            set_by: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
struct ClientStateCounters {
    server_ignoring_on_the_fly: u32,
    pending_client_ignoring_on_the_fly: Option<u32>,
    pending_client_latency_calculation: Option<f64>,
    ping_rtt_seconds: f64,
    ping_average_rtt_seconds: f64,
    ping_forward_delay_seconds: f64,
}
#[cfg(test)]
mod tests;
