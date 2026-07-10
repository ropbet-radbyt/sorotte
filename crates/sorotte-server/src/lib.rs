use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Arc, LazyLock,
        atomic::{AtomicUsize, Ordering},
    },
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
use sorotte_core::{DomainError, SyncDomain};
use sorotte_protocol::{
    ChatPayload, ControllerAuthPayload, DEFAULT_MAX_PROTOCOL_LINE_BYTES, HelloPayload,
    IgnoringOnTheFlyPayload, ListPayload, ListUserEntry, NewControlledRoomPayload, PingPayload,
    PlaylistIndexPayload, PlaystatePayload, ProtocolError, ProtocolMessage, ReadyPayload, RoomRef,
    SOROTTE_PLEX_PLAYLIST_URIS_FEATURE, SetPayload, StatePayload, TlsPayload, UserSetPayload,
    canonical_playlist_files_from_change, decode_line, decode_message_line_items,
    encode_message_line, playlist_change_with_plex_sidecar,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{
        Mutex, broadcast,
        mpsc::{Receiver, Sender, UnboundedSender, channel},
        watch,
    },
    task::JoinHandle,
    time,
};
use tokio_rustls::{TlsAcceptor, server::TlsStream};

const LEGACY_COMPAT_SERVER_VERSION: &str = "1.7.5";
const SERVER_REAL_VERSION: &str = LEGACY_COMPAT_SERVER_VERSION;
const LEGACY_COMPAT_UPGRADE_URL: &str = "https://syncplay.pl";
const DEFAULT_OUTDATED_MOTD_TEMPLATE: &str =
    "You are using Syncplay {client_version} but a newer version is available from {upgrade_url}";
const LEGACY_SERVER_MOTD_UNESCAPED_PLACEHOLDERS: &str =
    "Message of the Day has unescaped placeholders. All $ signs should be doubled ($$).";
const LEGACY_SERVER_MOTD_TOO_LONG_PREFIX: &str = "Message of the Day is too long - maximum of";
const LEGACY_SERVER_MAX_TEMPLATE_LENGTH: usize = 10_000;
const LEGACY_PERSISTENT_ROOMS_NOTICE: &str = "NOTICE: This server uses persistent rooms, which means that the playlist information is stored between playback sessions. If you want to create a room where information is not saved then put -temp at the end of the room name.";
const LEGACY_SERVER_PASSWORD_REQUIRED_ERROR: &str = "Password required";
const LEGACY_SERVER_WRONG_PASSWORD_ERROR: &str = "Wrong password supplied";
const LEGACY_CONTROLLED_ROOMS_MIN_VERSION: &str = "1.3.0";
const LEGACY_USER_READY_MIN_VERSION: &str = "1.3.0";
const LEGACY_SHARED_PLAYLIST_MIN_VERSION: &str = "1.4.0";
const LEGACY_CHAT_MIN_VERSION: &str = "1.5.0";
const LEGACY_UI_MODE_GRAPHICAL: &str = "GUI";
const LEGACY_UI_MODE_UNKNOWN: &str = "Unknown";
// This value is part of controlled-room hash compatibility; keep it byte-stable.
const DEFAULT_CONTROLLED_ROOM_HASH_SALT: &str = "syncplay-rs-controlled-room-v1";
const DEFAULT_MAX_CHAT_MESSAGE_LENGTH: usize = 150;
const DEFAULT_MAX_USERNAME_LENGTH: usize = 16;
const DEFAULT_MAX_ROOM_NAME_LENGTH: usize = 35;
const DEFAULT_MAX_FILENAME_LENGTH: usize = 250;
const DEFAULT_PLAYLIST_MAX_ITEMS: usize = 250;
const DEFAULT_PLAYLIST_MAX_CHARACTERS: usize = 10_000;
const SERVER_STATE_INTERVAL_SECONDS: f64 = 1.0;
const INITIAL_SERVER_STATE_DELAY_SECONDS: f64 = 0.1;
// GUI clients can spend tens of seconds doing local media-match fingerprinting
// or media-root scans. Keep room liveness tolerant of those local stalls while
// retaining shorter IO-specific timeouts below for handshakes and writes.
const PROTOCOL_TIMEOUT_SECONDS: f64 = 90.0;
const IO_TIMEOUT_SECONDS: f64 = 12.5;
const PING_MOVING_AVERAGE_WEIGHT: f64 = 0.85;
const SERVER_STATS_SNAPSHOT_INTERVAL_SECONDS: f64 = 3600.0;
const SERVER_STATS_DELAY_STEP_SECONDS: f64 = 5.0;
const SERVER_NETWORK_TICK_INTERVAL_SECONDS: f64 = 0.25;
// Media-match signatures can push otherwise valid Set/List lines above the
// base Syncplay line size, especially when multiple users publish signatures.
const MAX_PROTOCOL_LINE_BYTES: usize = DEFAULT_MAX_PROTOCOL_LINE_BYTES * 8;
const PROTOCOL_LINE_TOO_LONG_ERROR: &str = "Protocol line too long";
const CLIENT_OUTBOUND_QUEUE_CAPACITY: usize = 256;
// A reliable line gets a short chance to enter a temporarily full queue. If
// capacity does not recover, the client is explicitly closed and cleaned up.
const CLIENT_OUTBOUND_OVERLOAD_GRACE_MILLIS: u64 = 100;
const ACCEPTED_CLIENT_QUEUE_CAPACITY: usize = 1024;
const SERVER_PERSISTENCE_EVENT_CAPACITY: usize = 256;
const TLS_HANDSHAKE_TIMEOUT_SECONDS: f64 = IO_TIMEOUT_SECONDS;
const SERVER_WRITE_TIMEOUT_SECONDS: f64 = IO_TIMEOUT_SECONDS;
const TLS_REQUIRED_CERT_FILENAMES: [&str; 3] = ["privkey.pem", "cert.pem", "chain.pem"];
const TLS_CERT_ROTATION_MAX_RETRIES: u32 = 10;
const LEGACY_SERVER_UNKNOWN_COMMAND_ERROR_PREFIX: &str = "Unknown command";
const LEGACY_SERVER_NOT_JSON_ERROR_PREFIX: &str = "Not a json encoded string";
const LEGACY_SERVER_LINE_DECODE_ERROR: &str = "Not a utf-8 string";
const LEGACY_SERVER_NOT_KNOWN_ERROR: &str =
    "You must be known to server before sending this command";
const LEGACY_SERVER_HELLO_ERROR: &str = "Not enough Hello arguments";

mod actor;
mod app;
mod auth;
mod backpressure;
mod compat;
mod messages;
mod network;
mod persistence;
mod persistence_actor;
mod runtime_api;
mod runtime_handlers;
mod runtime_maintenance;
mod tls;

pub use actor::{ServerActorError, ServerActorHandle};
pub use app::ServerApp;
pub use backpressure::ServerOutboundBackpressureSnapshot;
pub use network::{
    run_server_network_loop_until_shutdown, run_server_network_loops_until_shutdown,
};
pub use persistence::{RoomPersistenceError, StatsPersistenceError};
pub use persistence_actor::{
    ServerPersistenceEffect, ServerPersistenceEvent, ServerPersistenceWorkerKind,
};

pub(crate) use auth::{
    RoomPasswordCheckError, RoomPasswordProvider, generate_server_salt_legacy_compatible,
};
pub(crate) use backpressure::ServerOutboundBackpressureMetrics;
pub(crate) use compat::*;
pub(crate) use messages::*;
#[cfg(test)]
pub(crate) use network::read_network_line_from_stream;
pub(crate) use persistence::{PersistedRoomState, RoomPersistenceStore, StatsPersistenceStore};
pub(crate) use persistence_actor::{RoomPersistenceService, StatsPersistenceService};
pub(crate) use tls::{
    load_tls_server_config, tls_certificate_bundle_is_available,
    tls_certificate_bundle_modified_time,
};

#[derive(Debug, Clone, PartialEq)]
pub struct ServerSession {
    pub username: String,
    pub room: String,
    pub version: String,
    pub features: Option<Value>,
    pub file: Option<Value>,
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
    #[error("{0} persistence worker is unavailable")]
    PersistenceWorkerUnavailable(&'static str),
}

#[derive(Debug, thiserror::Error)]
pub enum ServerNetworkError {
    #[error(transparent)]
    Actor(#[from] ServerActorError),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(
        "client '{client_id}' disconnected after sustained outbound overload at queue depth {queue_depth}"
    )]
    OutboundOverload {
        client_id: String,
        queue_depth: usize,
    },
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
    pub delivery: ServerOutboundDelivery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerOutboundDelivery {
    Reliable,
    CoalesciblePeriodicState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerRuntimeDispatch {
    pub outbound_lines: Vec<DirectedOutboundLine>,
    pub transport_actions: Vec<DirectedTransportAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerTransportAction {
    StartTls,
    Close,
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

#[derive(Debug)]
pub struct ServerRuntime {
    domain: SyncDomain,
    sessions: BTreeMap<String, ServerSession>,
    room_controllers: BTreeMap<String, BTreeSet<String>>,
    room_playlists: BTreeMap<String, RoomPlaylistState>,
    room_playback_states: BTreeMap<String, RoomPlaybackState>,
    client_playback_states: BTreeMap<String, ClientPlaybackState>,
    client_room_join_sequence: BTreeMap<String, u64>,
    next_room_join_sequence: u64,
    client_state_counters: BTreeMap<String, ClientStateCounters>,
    client_last_state_update_at: BTreeMap<String, f64>,
    client_next_periodic_state_at: BTreeMap<String, f64>,
    time_now_override_seconds: Option<f64>,
    room_password_provider: RoomPasswordProvider,
    server_password_token: Option<String>,
    motd_template: Option<String>,
    stats_persistence: Option<StatsPersistenceService>,
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
    room_persistence: Option<RoomPersistenceService>,
    room_persistence_versions: BTreeMap<String, u64>,
    persistence_events: broadcast::Sender<ServerPersistenceEvent>,
    persistence_degraded_worker_count: Arc<AtomicUsize>,
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
    updated_at_seconds: f64,
}

impl Default for RoomPlaybackState {
    fn default() -> Self {
        Self::new_at(0.0)
    }
}

impl RoomPlaybackState {
    fn new_at(updated_at_seconds: f64) -> Self {
        Self {
            position: 0.0,
            paused: true,
            set_by: None,
            updated_at_seconds,
        }
    }

    fn position_at(&self, now_seconds: f64) -> f64 {
        if self.paused {
            return self.position;
        }
        let elapsed_seconds = now_seconds - self.updated_at_seconds;
        if elapsed_seconds.is_finite() && elapsed_seconds > 0.0 {
            self.position + elapsed_seconds
        } else {
            self.position
        }
    }

    fn aged_at(&self, now_seconds: f64) -> Self {
        let mut aged = self.clone();
        aged.position = self.position_at(now_seconds);
        aged.updated_at_seconds = now_seconds;
        aged
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ClientPlaybackState {
    position: Option<f64>,
    updated_at_seconds: f64,
}

impl ClientPlaybackState {
    fn new(position: Option<f64>, updated_at_seconds: f64) -> Self {
        Self {
            position,
            updated_at_seconds,
        }
    }

    fn position_at(&self, room_paused: bool, now_seconds: f64) -> Option<f64> {
        let position = self.position?;
        if room_paused {
            return Some(position);
        }
        let elapsed_seconds = now_seconds - self.updated_at_seconds;
        if elapsed_seconds.is_finite() && elapsed_seconds > 0.0 {
            Some(position + elapsed_seconds)
        } else {
            Some(position)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
struct ClientStateCounters {
    server_ignoring_on_the_fly: u32,
    pending_client_ignoring_on_the_fly: Option<u32>,
    pending_client_latency_calculation: Option<f64>,
    pending_client_latency_calculation_arrival_time: Option<f64>,
    ping_rtt_seconds: f64,
    ping_average_rtt_seconds: f64,
    ping_forward_delay_seconds: f64,
}
#[cfg(test)]
mod tests;
