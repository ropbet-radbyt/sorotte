use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Arc, LazyLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
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
    ChatPayload, CommitStartPayload, ControllerAuthPayload, DEFAULT_MAX_PROTOCOL_LINE_BYTES,
    DirectReadinessSurface, FilePayload, HelloPayload, IgnoringOnTheFlyPayload, ListPayload,
    ListUserEntry, MediaLoadIntent, MediaReadyPayload, MixedReadinessPolicy,
    NewControlledRoomPayload, ParticipantReadiness, ParticipantReadinessUpdate, PingPayload,
    PlaybackBarrierDegradedReason, PlaybackBarrierParticipantPhase,
    PlaybackBarrierParticipantStatus, PlaybackBarrierPhase, PlaybackBarrierPolicy,
    PlaybackBarrierRecoveryDisposition, PlaybackBarrierRecoveryPayload,
    PlaybackBarrierRequestResultPayload, PlaybackBarrierSetExtension,
    PlaybackBarrierStateExtension, PlaybackBarrierStatusPayload, PlaybackBarrierTimeoutAction,
    PlayerReadinessAction, PlaylistIndexPayload, PlaystatePayload, PrepareMediaPayload,
    ProtocolError, ProtocolMessage, ReadinessIntentRequest, ReadinessMutationMetadata,
    ReadinessMutationSource, ReadinessRequestResultPayload, ReadinessRequestResultStatus,
    ReadinessSetExtension, ReadinessStateExtension, ReadyPayload, RecoveryStage,
    RoomBufferingPhase, RoomBufferingPolicy, RoomBufferingPolicyPayload,
    RoomBufferingStatusPayload, RoomPauseOwner, RoomReadinessSnapshot, RoomRef, RoomStartGatePhase,
    SOROTTE_PLAYBACK_BARRIER_V1, SOROTTE_PLEX_PLAYLIST_URIS_FEATURE,
    SOROTTE_READINESS_RECONNECT_TOKEN, SOROTTE_READINESS_V2, SetPayload, StartGateDegradedReason,
    StartParticipationRole, StartedAckPayload, StatePayload, TechnicalBlockCause,
    TechnicalPlayability, TechnicalPlayabilityPhase, TechnicalReadinessBlock,
    TechnicalReadinessReport, TransportBufferingReportPayload, UserReadinessIntent,
    UserReadinessMutationSource, UserSetPayload, canonical_playlist_files_from_change, decode_line,
    decode_message_line_items, encode_message_line, playlist_change_with_plex_sidecar,
};
use sorotte_secret::SecretValue;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{
        Mutex, broadcast,
        mpsc::{Receiver, Sender, UnboundedSender, channel},
        watch,
    },
    task::{JoinHandle, JoinSet},
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
const PLAYBACK_BARRIER_DEFAULT_TIMEOUT_SECONDS: f64 = 20.0;
const PLAYBACK_BARRIER_MIN_TIMEOUT_SECONDS: f64 = 1.0;
const PLAYBACK_BARRIER_MAX_TIMEOUT_SECONDS: f64 = 30.0;
const PLAYBACK_BARRIER_STARTED_TIMEOUT_SECONDS: f64 = 10.0;
const PLAYBACK_BARRIER_MAX_LOGICAL_MEDIA_ID_CHARS: usize = 2048;
const PLAYBACK_BARRIER_MAX_REQUEST_ID_BYTES: usize = 128;
const READINESS_MAX_OPERATION_ID_BYTES: usize = 128;
const READINESS_MAX_RETAINED_OPERATIONS_PER_MEMBERSHIP: usize = 256;
const READINESS_RECONNECT_TTL_SECONDS: f64 = PROTOCOL_TIMEOUT_SECONDS * 2.0;
const READINESS_USER_TRANSPORT_GRACE_SECONDS: f64 = 5.0;
// Superseded transports are forcibly closed and ordinary live connections
// time out after PROTOCOL_TIMEOUT_SECONDS. Retain displaced request identities
// across that lifetime plus two IO deadlines and shutdown scheduling margin.
const PLAYBACK_BARRIER_REQUEST_TOMBSTONE_TTL_SECONDS: f64 =
    PROTOCOL_TIMEOUT_SECONDS + (2.0 * IO_TIMEOUT_SECONDS) + SERVER_NETWORK_SHUTDOWN_GRACE_SECONDS;
const PLAYBACK_BARRIER_MAX_REQUEST_TOMBSTONES_PER_ROOM: usize = 4096;
const PLAYBACK_BARRIER_MAX_REQUEST_TOMBSTONES_GLOBAL: usize = 16_384;
const PLAYBACK_BARRIER_REQUEST_RETRY_MIN_MILLIS: u64 = 250;
const PLAYBACK_BARRIER_REQUEST_RETRY_MAX_MILLIS: u64 = 30_000;
// New identities are deliberately burst-tolerant for normal playlist skips,
// while remaining far below the rate needed to keep the 120-second replay
// window continuously saturated. Exact retries use the same identity and do
// not consume this budget.
const PLAYBACK_BARRIER_NEW_IDENTITY_RATE_WINDOW_SECONDS: f64 = 10.0;
const PLAYBACK_BARRIER_MAX_NEW_IDENTITIES_PER_CLIENT_WINDOW: usize = 16;
const PLAYBACK_BARRIER_MAX_NEW_IDENTITIES_PER_ROOM_WINDOW: usize = 64;
const ROOM_BUFFERING_DEFAULT_QUORUM_PERCENT: u32 = 75;
const ROOM_BUFFERING_DEFAULT_DEBOUNCE_SECONDS: f64 = 0.75;
const ROOM_BUFFERING_MAX_DEBOUNCE_SECONDS: f64 = 10.0;
const ROOM_BUFFERING_DEFAULT_RESUME_HYSTERESIS_SECONDS: f64 = 1.5;
const ROOM_BUFFERING_MAX_RESUME_HYSTERESIS_SECONDS: f64 = 15.0;
const ROOM_BUFFERING_DEFAULT_MAX_PAUSE_SECONDS: f64 = 30.0;
const ROOM_BUFFERING_MIN_MAX_PAUSE_SECONDS: f64 = 1.0;
const ROOM_BUFFERING_MAX_MAX_PAUSE_SECONDS: f64 = 60.0;
// Media-match signatures can push otherwise valid Set/List lines above the
// base Syncplay line size, especially when multiple users publish signatures.
const MAX_PROTOCOL_LINE_BYTES: usize = DEFAULT_MAX_PROTOCOL_LINE_BYTES * 8;
const PROTOCOL_LINE_TOO_LONG_ERROR: &str = "Protocol line too long";
const CLIENT_OUTBOUND_QUEUE_CAPACITY: usize = 256;
// A reliable line gets a short chance to enter a temporarily full queue. If
// capacity does not recover, the client is explicitly closed and cleaned up.
const CLIENT_OUTBOUND_OVERLOAD_GRACE_MILLIS: u64 = 100;
const ACCEPTED_CLIENT_QUEUE_CAPACITY: usize = 1024;
const SERVER_NETWORK_SHUTDOWN_GRACE_SECONDS: f64 = 5.0;
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
mod inbound;
mod messages;
mod network;
mod persistence;
mod persistence_actor;
mod runtime_api;
mod runtime_handlers;
mod runtime_maintenance;
mod runtime_playback_barrier;
mod runtime_readiness;
mod tls;

pub use actor::{ServerActorError, ServerActorHandle};
pub use app::ServerApp;
pub use backpressure::ServerOutboundBackpressureSnapshot;
pub use inbound::{ServerClientCapabilities, ServerCompatibilityFallback};
pub use network::{
    run_server_network_loop_until_shutdown, run_server_network_loops_and_shutdown_actor,
    run_server_network_loops_until_shutdown,
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
pub(crate) use inbound::{
    ServerHelloCommand, ServerInboundCommand, ServerSetCommand, ServerSharedFile,
    ServerStateCommand, normalize_server_protocol_message,
};
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
    pub capabilities: ServerClientCapabilities,
    pub(crate) file: Option<ServerSharedFile>,
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
    #[error(
        "server network shutdown exceeded its {timeout_millis} ms grace period with {acceptor_tasks} acceptor task(s) and {session_tasks} client session task(s) still active"
    )]
    ShutdownTimeout {
        timeout_millis: u64,
        acceptor_tasks: usize,
        session_tasks: usize,
    },
}

/// Errors from the production lifecycle boundary. Network teardown and the
/// actor durability barrier are both attempted, and a dual failure preserves
/// both causes for diagnostics.
#[derive(Debug, thiserror::Error)]
pub enum ServerLifecycleError {
    #[error("server network failed: {0}")]
    Network(ServerNetworkError),
    #[error("server actor shutdown failed: {0}")]
    Shutdown(ServerActorError),
    #[error("server network failed: {network}; server actor shutdown also failed: {shutdown}")]
    NetworkAndShutdown {
        network: ServerNetworkError,
        shutdown: ServerActorError,
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

#[derive(Clone, PartialEq, Eq)]
pub struct DirectedOutboundLine {
    pub client_id: String,
    pub line: String,
    pub delivery: ServerOutboundDelivery,
}

impl std::fmt::Debug for DirectedOutboundLine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DirectedOutboundLine")
            .field("client_id", &self.client_id)
            .field("line_bytes", &self.line.len())
            .field("delivery", &self.delivery)
            .finish()
    }
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
    room_playback_barriers: BTreeMap<String, RoomPlaybackBarrier>,
    room_buffering_controls: BTreeMap<String, RoomBufferingControl>,
    room_readiness: BTreeMap<String, RoomReadinessCoordinator>,
    readiness_reconnect_cache: BTreeMap<[u8; 32], DetachedReadinessMembership>,
    readiness_reconnect_identity_by_client: BTreeMap<String, SecretValue>,
    mixed_readiness_policy: MixedReadinessPolicy,
    pending_user_transport_by_client: BTreeMap<String, PendingUserTransportTransition>,
    next_readiness_membership_epoch: u64,
    /// Superseded transport identities awaiting network teardown after a
    /// newer connection recovered their playback lifecycle. Fenced clients
    /// cannot dispatch any protocol command, and a later disconnect therefore
    /// cannot degrade the replacement owner.
    playback_barrier_fenced_clients: BTreeSet<String>,
    /// Recently displaced application operations. Current identity remains
    /// canonical in the barrier or buffering control; only retired identities
    /// consume this time-bounded, per-room and globally bounded replay cache.
    playback_barrier_request_tombstones:
        BTreeMap<(String, PlaybackBarrierRequestId), PlaybackBarrierRequestTombstone>,
    playback_barrier_request_tombstone_policy: PlaybackBarrierRequestTombstonePolicy,
    playback_barrier_request_clock_started_at: Instant,
    #[cfg(test)]
    playback_barrier_request_clock_override_seconds: Option<f64>,
    playback_barrier_new_identity_rate_policy: PlaybackBarrierNewIdentityRatePolicy,
    playback_barrier_new_identity_rate_by_client:
        BTreeMap<String, VecDeque<PlaybackBarrierNewIdentityRateEvent>>,
    playback_barrier_new_identity_rate_by_room:
        BTreeMap<String, VecDeque<PlaybackBarrierNewIdentityRateEvent>>,
    /// Highest accepted/consumed request nonce for each live connection.
    /// This bounds duplicate suppression to live sessions and prevents
    /// delayed requests from older room generations replaying as fresh user
    /// intent, including after a room switch.
    playback_barrier_request_nonces: BTreeMap<String, u64>,
    next_playback_barrier_generation: u64,
    next_playback_barrier_revision: u64,
    client_playback_states: BTreeMap<String, ClientPlaybackState>,
    client_room_join_sequence: BTreeMap<String, u64>,
    next_room_join_sequence: u64,
    client_state_counters: BTreeMap<String, ClientStateCounters>,
    client_last_state_update_at: BTreeMap<String, f64>,
    client_next_periodic_state_at: BTreeMap<String, f64>,
    time_now_override_seconds: Option<f64>,
    room_password_provider: RoomPasswordProvider,
    server_password_token: Option<SecretValue>,
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
    pending_compatibility_fallbacks: Vec<ServerCompatibilityFallback>,
}

#[derive(Clone, PartialEq, Eq, Default)]
struct RoomPlaylistState {
    files: Vec<String>,
    index: Option<i64>,
}

impl std::fmt::Debug for RoomPlaylistState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RoomPlaylistState")
            .field("files_count", &self.files.len())
            .field("index", &self.index)
            .finish()
    }
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

#[derive(Debug, Clone, PartialEq)]
struct RoomPlaybackBarrierParticipant {
    username: String,
    status: PlaybackBarrierParticipantStatus,
}

#[derive(Debug, Clone, PartialEq)]
struct RoomPlaybackBarrier {
    prepare: PrepareMediaPayload,
    /// Retained after the barrier becomes terminal so an idempotent retry can
    /// replay the canonical lifecycle without making the commit active again.
    commit: Option<CommitStartPayload>,
    initiator_client_id: String,
    initiator_session_sequence: u64,
    initiator_username: String,
    participants: BTreeMap<String, RoomPlaybackBarrierParticipant>,
    excluded_legacy_clients: BTreeSet<String>,
    phase: PlaybackBarrierPhase,
    state_revision: Option<u64>,
    readiness_revision: Option<u64>,
    deadline: f64,
    started_deadline: Option<f64>,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PlaybackBarrierRequestId(String);

impl PlaybackBarrierRequestId {
    fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl std::fmt::Debug for PlaybackBarrierRequestId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("<redacted-playback-request-id>")
    }
}

#[derive(Clone, PartialEq)]
struct PlaybackBarrierRequestTombstone {
    request_nonce: u64,
    logical_media_id_digest: Option<[u8; 32]>,
    media_generation: u64,
    retain_until_seconds: f64,
}

impl std::fmt::Debug for PlaybackBarrierRequestTombstone {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlaybackBarrierRequestTombstone")
            .field("request_nonce", &self.request_nonce)
            .field(
                "logical_media_id_digest",
                &self.logical_media_id_digest.as_ref().map(|_| "<redacted>"),
            )
            .field("media_generation", &self.media_generation)
            .field("retain_until_seconds", &self.retain_until_seconds)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PlaybackBarrierRequestTombstonePolicy {
    ttl_seconds: f64,
    max_per_room: usize,
    max_global: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PlaybackBarrierNewIdentityRatePolicy {
    window_seconds: f64,
    max_per_client: usize,
    max_per_room: usize,
}

impl Default for PlaybackBarrierNewIdentityRatePolicy {
    fn default() -> Self {
        Self {
            window_seconds: PLAYBACK_BARRIER_NEW_IDENTITY_RATE_WINDOW_SECONDS,
            max_per_client: PLAYBACK_BARRIER_MAX_NEW_IDENTITIES_PER_CLIENT_WINDOW,
            max_per_room: PLAYBACK_BARRIER_MAX_NEW_IDENTITIES_PER_ROOM_WINDOW,
        }
    }
}

#[derive(Clone, PartialEq)]
struct PlaybackBarrierNewIdentityRateEvent {
    username: String,
    room_name: String,
    request_id: Option<PlaybackBarrierRequestId>,
    request_nonce: u64,
    observed_at_seconds: f64,
}

impl PlaybackBarrierNewIdentityRateEvent {
    fn matches_operation(
        &self,
        username: &str,
        room_name: &str,
        request_id: Option<&str>,
        request_nonce: u64,
    ) -> bool {
        self.username == username
            && self.room_name == room_name
            && self
                .request_id
                .as_ref()
                .map(|request_id| request_id.0.as_str())
                == request_id
            && self.request_nonce == request_nonce
    }
}

impl std::fmt::Debug for PlaybackBarrierNewIdentityRateEvent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlaybackBarrierNewIdentityRateEvent")
            .field("username", &self.username)
            .field("room_name", &self.room_name)
            .field(
                "request_id",
                &self.request_id.as_ref().map(|_| "<redacted>"),
            )
            .field("request_nonce", &self.request_nonce)
            .field("observed_at_seconds", &self.observed_at_seconds)
            .finish()
    }
}

impl Default for PlaybackBarrierRequestTombstonePolicy {
    fn default() -> Self {
        Self {
            ttl_seconds: PLAYBACK_BARRIER_REQUEST_TOMBSTONE_TTL_SECONDS,
            max_per_room: PLAYBACK_BARRIER_MAX_REQUEST_TOMBSTONES_PER_ROOM,
            max_global: PLAYBACK_BARRIER_MAX_REQUEST_TOMBSTONES_GLOBAL,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DisplacedPlaybackBarrierRequest {
    request_nonce: u64,
    logical_media_id_digest: Option<[u8; 32]>,
    media_generation: u64,
}

#[derive(Debug, Clone, PartialEq)]
struct RoomBufferingParticipantReport {
    username: String,
    buffering: bool,
    buffered_seconds: Option<f64>,
    reported_at_seconds: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct RoomBufferingControl {
    config: RoomBufferingPolicyPayload,
    requested_config: RoomBufferingPolicyPayload,
    configured_by_client_id: String,
    configured_by_username: String,
    reports: BTreeMap<String, RoomBufferingParticipantReport>,
    condition_active_since: Option<f64>,
    condition_clear_since: Option<f64>,
    paused_by_policy: bool,
    pause_deadline: Option<f64>,
    fail_open_latched: bool,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ReadinessOperationId(String);

impl ReadinessOperationId {
    fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl std::fmt::Debug for ReadinessOperationId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("<redacted-readiness-operation-id>")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AcceptedReadinessOperation {
    membership_epoch: u64,
    desired: UserReadinessIntent,
    source: UserReadinessMutationSource,
    target_username: Option<String>,
    accepted_revision: u64,
    accepted_user_intent_revision: u64,
}

#[derive(Debug, Clone, PartialEq)]
struct ServerReadinessParticipant {
    client_id: String,
    record: ParticipantReadiness,
    initialization_open: bool,
    highest_request_nonce: u64,
    accepted_operations: BTreeMap<ReadinessOperationId, AcceptedReadinessOperation>,
    pending_automatic_pause_owner: Option<RoomPauseOwner>,
    last_technical_observed_at: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
struct RoomReadinessCoordinator {
    revision: u64,
    media_generation: Option<u64>,
    start_gate_phase: RoomStartGatePhase,
    pause_owner: RoomPauseOwner,
    participants: BTreeMap<String, ServerReadinessParticipant>,
}

impl Default for RoomReadinessCoordinator {
    fn default() -> Self {
        Self {
            revision: 0,
            media_generation: None,
            start_gate_phase: RoomStartGatePhase::Inactive,
            pause_owner: RoomPauseOwner::None,
            participants: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct DetachedReadinessMembership {
    room_name: String,
    username: String,
    membership_epoch: u64,
    user_intent: UserReadinessIntent,
    user_intent_revision: u64,
    last_user_mutation: Option<ReadinessMutationMetadata>,
    last_technical_report_sequence: u64,
    initialization_open: bool,
    accepted_operations: BTreeMap<ReadinessOperationId, AcceptedReadinessOperation>,
    room_readiness_revision: u64,
    detached_at_seconds: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct PendingUserTransportTransition {
    room_name: String,
    actor: String,
    desired_paused: bool,
    evidence: PendingUserTransportEvidence,
    expires_at_seconds: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingUserTransportEvidence {
    AcceptedIndirectAction,
    UnclassifiedObservation,
}
#[cfg(test)]
mod tests;
