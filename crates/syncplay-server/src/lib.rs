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

fn legacy_stats_snapshot_start_delay_seconds_for_port(port: u16) -> f64 {
    SERVER_STATS_DELAY_STEP_SECONDS * (f64::from(port % 10) + 1.0)
}

fn tls_certificate_bundle_is_available(path: &Path) -> bool {
    TLS_REQUIRED_CERT_FILENAMES
        .iter()
        .all(|filename| path.join(filename).is_file())
}

fn tls_certificate_file_modified_time(path: &Path) -> Option<SystemTime> {
    fs::metadata(path.join(TLS_CERT_FILENAME))
        .ok()
        .and_then(|metadata| metadata.modified().ok())
}

fn tls_certificates_from_pem(path: &Path) -> io::Result<Vec<CertificateDer<'static>>> {
    let file = fs::File::open(path)?;
    let mut reader = io::BufReader::new(file);
    let certificates = rustls_pemfile::certs(&mut reader).collect::<Result<Vec<_>, _>>()?;
    if certificates.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("tls certificate file '{}' is empty", path.display()),
        ));
    }
    Ok(certificates)
}

fn tls_private_key_from_pem(path: &Path) -> io::Result<PrivateKeyDer<'static>> {
    let file = fs::File::open(path)?;
    let mut reader = io::BufReader::new(file);
    rustls_pemfile::private_key(&mut reader)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("tls private key file '{}' has no key", path.display()),
        )
    })
}

fn load_tls_server_config(path: &Path) -> io::Result<Arc<ServerConfig>> {
    let mut certificate_chain = tls_certificates_from_pem(&path.join("cert.pem"))?;
    certificate_chain.extend(tls_certificates_from_pem(&path.join("chain.pem"))?);
    let private_key = tls_private_key_from_pem(&path.join("privkey.pem"))?;
    let server_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certificate_chain, private_key)
        .map_err(|source| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("tls certificate bundle is invalid: {source}"),
            )
        })?;
    Ok(Arc::new(server_config))
}

static CONTROLLED_ROOM_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\+(.*):(\w{12})$").expect("controlled room regex is valid"));
static PASSWORD_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Z]{2}-\d{3}-\d{3}").expect("password regex is valid"));

fn parse_numeric_version_components(version: &str) -> Option<Vec<u32>> {
    let trimmed = version.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut components = Vec::new();
    for part in trimmed.split('.') {
        if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        components.push(part.parse().ok()?);
    }

    Some(components)
}

fn is_client_version_outdated(client_version: &str, server_version: &str) -> bool {
    let Some(mut client_components) = parse_numeric_version_components(client_version) else {
        return false;
    };
    let Some(mut server_components) = parse_numeric_version_components(server_version) else {
        return false;
    };

    let width = client_components.len().max(server_components.len());
    client_components.resize(width, 0);
    server_components.resize(width, 0);
    client_components < server_components
}

fn render_motd_template(template: &str, client_version: &str) -> String {
    template
        .replace("{client_version}", client_version)
        .replace("{latest_version}", LEGACY_COMPAT_SERVER_VERSION)
        .replace("{upgrade_url}", LEGACY_COMPAT_UPGRADE_URL)
}

fn default_motd_for_client_version(client_version: &str) -> String {
    if is_client_version_outdated(client_version, LEGACY_COMPAT_SERVER_VERSION) {
        return render_motd_template(DEFAULT_OUTDATED_MOTD_TEMPLATE, client_version);
    }
    String::new()
}

fn motd_for_client_version(client_version: &str, motd_template_override: Option<&str>) -> String {
    let is_outdated = is_client_version_outdated(client_version, LEGACY_COMPAT_SERVER_VERSION);
    if let Some(template) = motd_template_override.map(str::trim) {
        if template.is_empty() {
            return String::new();
        }
        let custom_motd = render_motd_template(template, client_version);
        if is_outdated {
            let warning_motd = render_motd_template(DEFAULT_OUTDATED_MOTD_TEMPLATE, client_version);
            return format!("{warning_motd}\n{custom_motd}");
        }
        return custom_motd;
    }
    default_motd_for_client_version(client_version)
}

fn truncate_text_to_max_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn hello_server_password_token(hello: &HelloPayload) -> Option<&str> {
    hello.extra.get("password").and_then(Value::as_str)
}

fn legacy_server_password_token_md5_hex(token: &str) -> String {
    format!("{:x}", Md5::digest(token.as_bytes()))
}

fn server_password_token_matches_legacy_compatible(
    presented_token: &str,
    configured_token: &str,
) -> bool {
    // Accept raw tokens for Rust-Rust interoperability and legacy-Python MD5 tokens for parity.
    presented_token == configured_token
        || presented_token == legacy_server_password_token_md5_hex(configured_token)
}

fn client_supports_persistent_rooms(advertised_features: Option<&Value>) -> bool {
    advertised_features
        .and_then(Value::as_object)
        .and_then(|features| features.get("persistentRooms"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn persistent_rooms_notice_motd(
    base_motd: String,
    persistent_rooms_enabled: bool,
    advertised_features: Option<&Value>,
) -> String {
    if !persistent_rooms_enabled || client_supports_persistent_rooms(advertised_features) {
        return base_motd;
    }
    if base_motd.is_empty() {
        return LEGACY_PERSISTENT_ROOMS_NOTICE.to_owned();
    }
    format!("{LEGACY_PERSISTENT_ROOMS_NOTICE}\n\n{base_motd}")
}

fn room_name_is_marked_temporary(room_name: &str) -> bool {
    let room_name = room_name.to_ascii_lowercase();
    room_name.ends_with("-temp") || room_name.contains("-temp:")
}

fn playlist_as_multiline(files: &[String]) -> String {
    files.join("\n")
}

fn multiline_as_playlist(multiline: &str) -> Vec<String> {
    if multiline.is_empty() {
        return Vec::new();
    }
    multiline.split('\n').map(str::to_owned).collect()
}

fn parse_permanent_rooms_file(contents: &str) -> BTreeSet<String> {
    contents.lines().map(str::to_owned).collect()
}

fn feature_ui_mode(features: Option<&Value>) -> Option<&str> {
    features
        .and_then(Value::as_object)
        .and_then(|features| features.get("uiMode"))
        .and_then(Value::as_str)
}

fn client_is_gui_user(features: Option<&Value>) -> bool {
    let mut ui_mode = feature_ui_mode(features).unwrap_or(LEGACY_UI_MODE_UNKNOWN);
    if ui_mode == LEGACY_UI_MODE_UNKNOWN {
        ui_mode = LEGACY_UI_MODE_GRAPHICAL;
    }
    ui_mode == LEGACY_UI_MODE_GRAPHICAL
}

fn features_include_ui_mode(features: Option<&Value>) -> bool {
    features
        .and_then(Value::as_object)
        .is_some_and(|features| features.contains_key("uiMode"))
}

fn legacy_dummy_list_entry() -> ListUserEntry {
    ListUserEntry::new()
        .with_position(0.0)
        .with_file(json!({}))
        .with_controller(false)
        .with_is_ready(true)
        .with_features(json!([]))
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServerSession {
    pub username: String,
    pub room: String,
    pub version: String,
    pub features: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
struct PersistedRoomState {
    files: Vec<String>,
    index: Option<i64>,
    position: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum StatsPersistenceError {
    #[error("stats persistence '{action}' failed for '{path}': {source}")]
    Sqlite {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StatsPersistenceStore {
    db_path: PathBuf,
}

impl StatsPersistenceStore {
    fn open(db_path: impl AsRef<Path>) -> Result<Self, StatsPersistenceError> {
        let store = Self {
            db_path: db_path.as_ref().to_path_buf(),
        };
        store.initialize_schema()?;
        Ok(store)
    }

    fn add_version_log(
        &self,
        snapshot_time: i64,
        version: &str,
    ) -> Result<(), StatsPersistenceError> {
        let connection = self.connection("connect")?;
        connection
            .execute(
                "INSERT INTO clients_snapshots (snapshot_time, version) VALUES (?1, ?2)",
                params![snapshot_time, version],
            )
            .map_err(|source| self.sqlite_error("insert clients snapshot row", source))?;
        Ok(())
    }

    fn initialize_schema(&self) -> Result<(), StatsPersistenceError> {
        let connection = self.connection("connect")?;
        connection
            .execute(
                "CREATE TABLE IF NOT EXISTS clients_snapshots (\
                 snapshot_time INTEGER, \
                 version STRING\
                 )",
                [],
            )
            .map_err(|source| self.sqlite_error("initialize schema", source))?;
        Ok(())
    }

    fn connection(&self, action: &'static str) -> Result<Connection, StatsPersistenceError> {
        Connection::open(&self.db_path).map_err(|source| self.sqlite_error(action, source))
    }

    fn sqlite_error(&self, action: &'static str, source: rusqlite::Error) -> StatsPersistenceError {
        StatsPersistenceError::Sqlite {
            action,
            path: self.db_path.clone(),
            source,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RoomPersistenceError {
    #[error("room persistence '{action}' failed for '{path}': {source}")]
    Sqlite {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RoomPersistenceStore {
    db_path: PathBuf,
}

impl RoomPersistenceStore {
    fn open(db_path: impl AsRef<Path>) -> Result<Self, RoomPersistenceError> {
        let store = Self {
            db_path: db_path.as_ref().to_path_buf(),
        };
        store.initialize_schema()?;
        Ok(store)
    }

    fn load_rooms(&self) -> Result<BTreeMap<String, PersistedRoomState>, RoomPersistenceError> {
        let connection = self.connection("connect")?;
        let mut statement = connection
            .prepare(
                "SELECT name, playlist, playlistIndex, position \
                 FROM persistent_rooms",
            )
            .map_err(|source| self.sqlite_error("prepare load query", source))?;
        let rows = statement
            .query_map([], |row| {
                let room_name: String = row.get(0)?;
                let playlist_multiline: Option<String> = row.get(1)?;
                let playlist_index: Option<i64> = row.get(2)?;
                let position: Option<f64> = row.get(3)?;
                Ok((
                    room_name,
                    PersistedRoomState {
                        files: multiline_as_playlist(&playlist_multiline.unwrap_or_default()),
                        index: playlist_index,
                        position: position.unwrap_or(0.0),
                    },
                ))
            })
            .map_err(|source| self.sqlite_error("query persisted rooms", source))?;

        let mut rooms = BTreeMap::new();
        for row in rows {
            let (room_name, room_state) =
                row.map_err(|source| self.sqlite_error("decode persisted room row", source))?;
            rooms.insert(room_name, room_state);
        }
        Ok(rooms)
    }

    fn save_room(
        &self,
        room_name: &str,
        files: &[String],
        playlist_index: Option<i64>,
        position: f64,
    ) -> Result<(), RoomPersistenceError> {
        let connection = self.connection("connect")?;
        connection
            .execute(
                "INSERT OR REPLACE INTO persistent_rooms \
                 (name, playlist, playlistIndex, position, lastSavedUpdate) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    room_name,
                    playlist_as_multiline(files),
                    playlist_index,
                    position,
                    0_i64
                ],
            )
            .map_err(|source| self.sqlite_error("save persisted room", source))?;
        Ok(())
    }

    fn delete_room(&self, room_name: &str) -> Result<(), RoomPersistenceError> {
        let connection = self.connection("connect")?;
        connection
            .execute(
                "DELETE FROM persistent_rooms WHERE name = ?1",
                params![room_name],
            )
            .map_err(|source| self.sqlite_error("delete persisted room", source))?;
        Ok(())
    }

    fn initialize_schema(&self) -> Result<(), RoomPersistenceError> {
        let connection = self.connection("connect")?;
        connection
            .execute(
                "CREATE TABLE IF NOT EXISTS persistent_rooms (\
                 name STRING PRIMARY KEY, \
                 playlist STRING, \
                 playlistIndex INTEGER, \
                 position REAL, \
                 lastSavedUpdate INTEGER\
                 )",
                [],
            )
            .map_err(|source| self.sqlite_error("initialize schema", source))?;
        Ok(())
    }

    fn connection(&self, action: &'static str) -> Result<Connection, RoomPersistenceError> {
        Connection::open(&self.db_path).map_err(|source| self.sqlite_error(action, source))
    }

    fn sqlite_error(&self, action: &'static str, source: rusqlite::Error) -> RoomPersistenceError {
        RoomPersistenceError::Sqlite {
            action,
            path: self.db_path.clone(),
            source,
        }
    }
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum RoomPasswordCheckError {
    InvalidPassword,
    NotControlledRoom,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RoomPasswordProvider {
    salt: String,
}

impl Default for RoomPasswordProvider {
    fn default() -> Self {
        Self::new(DEFAULT_CONTROLLED_ROOM_HASH_SALT)
    }
}

impl RoomPasswordProvider {
    fn new(salt: impl Into<String>) -> Self {
        Self { salt: salt.into() }
    }

    fn is_controlled_room_name(&self, room_name: &str) -> bool {
        CONTROLLED_ROOM_REGEX.is_match(room_name)
    }

    fn is_valid_room_password(&self, password: &str) -> bool {
        if password.is_empty() {
            return false;
        }
        PASSWORD_REGEX
            .find(password)
            .is_some_and(|matched| matched.start() == 0)
    }

    fn check(&self, room_name: &str, password: &str) -> Result<bool, RoomPasswordCheckError> {
        if !self.is_valid_room_password(password) {
            return Err(RoomPasswordCheckError::InvalidPassword);
        }

        let captures = CONTROLLED_ROOM_REGEX
            .captures(room_name)
            .ok_or(RoomPasswordCheckError::NotControlledRoom)?;
        let base_room = captures
            .get(1)
            .expect("controlled room regex always includes base room capture")
            .as_str();
        let expected_hash = captures
            .get(2)
            .expect("controlled room regex always includes hash capture")
            .as_str();
        let computed_hash = self.compute_room_hash(base_room, password);
        Ok(computed_hash == expected_hash)
    }

    fn controlled_room_name_for(&self, room_name: &str, password: &str) -> String {
        format!(
            "+{room_name}:{}",
            self.compute_room_hash(room_name, password)
        )
    }

    fn compute_room_hash(&self, room_name: &str, password: &str) -> String {
        let salt_hash = format!("{:x}", Sha256::digest(self.salt.as_bytes()));
        let provisional_input = format!("{room_name}{salt_hash}");
        let provisional_hash = format!("{:x}", Sha256::digest(provisional_input.as_bytes()));
        let room_hash_input = format!("{provisional_hash}{salt_hash}{password}");
        let room_hash = format!("{:x}", Sha1::digest(room_hash_input.as_bytes()));
        room_hash[..12].to_ascii_uppercase()
    }
}

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

impl Default for ServerRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerRuntime {
    pub fn new() -> Self {
        Self::with_room_password_salt(DEFAULT_CONTROLLED_ROOM_HASH_SALT)
    }

    pub fn with_room_password_salt(salt: impl Into<String>) -> Self {
        Self {
            domain: SyncDomain::default(),
            sessions: BTreeMap::new(),
            room_controllers: BTreeMap::new(),
            room_playlists: BTreeMap::new(),
            room_playback_states: BTreeMap::new(),
            client_state_counters: BTreeMap::new(),
            client_last_state_update_at: BTreeMap::new(),
            client_next_periodic_state_at: BTreeMap::new(),
            time_now_override_seconds: None,
            room_password_provider: RoomPasswordProvider::new(salt),
            server_password_token: None,
            motd_template: None,
            stats_persistence: None,
            stats_snapshot_start_delay_seconds: legacy_stats_snapshot_start_delay_seconds_for_port(
                0,
            ),
            stats_snapshot_interval_seconds: SERVER_STATS_SNAPSHOT_INTERVAL_SECONDS,
            stats_next_snapshot_at_seconds: None,
            tls_cert_path: None,
            tls_server_config: None,
            tls_context_available: false,
            server_accepts_tls: false,
            tls_last_edit_cert_time: None,
            tls_rotation_attempts: 0,
            pending_transport_actions: Vec::new(),
            persistent_rooms_enabled: false,
            isolate_rooms: false,
            chat_enabled: true,
            readiness_enabled: true,
            max_chat_message_length: DEFAULT_MAX_CHAT_MESSAGE_LENGTH,
            max_username_length: DEFAULT_MAX_USERNAME_LENGTH,
            room_persistence: None,
            permanent_rooms: BTreeSet::new(),
        }
    }

    pub fn with_motd_template(template: impl Into<String>) -> Self {
        let mut runtime = Self::new();
        runtime.set_motd_template(Some(template.into()));
        runtime
    }

    pub fn with_persistent_rooms_enabled(enabled: bool) -> Self {
        let mut runtime = Self::new();
        runtime.set_persistent_rooms_enabled(enabled);
        runtime
    }

    pub fn set_motd_template(&mut self, template: Option<String>) {
        self.motd_template = template.and_then(|template| {
            let trimmed = template.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            }
        });
    }

    pub fn set_server_password_token(&mut self, token: Option<String>) {
        self.server_password_token = token.filter(|token| !token.is_empty());
    }

    pub fn with_stats_db_path(db_path: impl Into<PathBuf>) -> Result<Self, ServerRuntimeError> {
        let mut runtime = Self::new();
        runtime.set_stats_db_path(Some(db_path.into()))?;
        Ok(runtime)
    }

    pub fn with_tls_cert_path(path: impl Into<PathBuf>) -> Self {
        let mut runtime = Self::new();
        runtime.set_tls_cert_path(Some(path.into()));
        runtime
    }

    pub fn set_stats_snapshot_start_delay_seconds(&mut self, delay_seconds: f64) {
        self.stats_snapshot_start_delay_seconds =
            if delay_seconds.is_finite() && delay_seconds >= 0.0 {
                delay_seconds
            } else {
                0.0
            };
        if self.stats_persistence.is_some() {
            self.initialize_stats_snapshot_schedule();
        }
    }

    pub fn set_stats_snapshot_start_delay_for_port(&mut self, port: u16) {
        self.set_stats_snapshot_start_delay_seconds(
            legacy_stats_snapshot_start_delay_seconds_for_port(port),
        );
    }

    pub fn set_stats_snapshot_interval_seconds(&mut self, interval_seconds: f64) {
        self.stats_snapshot_interval_seconds =
            if interval_seconds.is_finite() && interval_seconds > 0.0 {
                interval_seconds
            } else {
                SERVER_STATS_SNAPSHOT_INTERVAL_SECONDS
            };
        if self.stats_persistence.is_some() {
            self.initialize_stats_snapshot_schedule();
        }
    }

    pub fn set_stats_db_path(
        &mut self,
        db_path: Option<PathBuf>,
    ) -> Result<(), ServerRuntimeError> {
        let Some(db_path) = db_path else {
            self.stats_persistence = None;
            self.stats_next_snapshot_at_seconds = None;
            return Ok(());
        };
        let stats_persistence = StatsPersistenceStore::open(&db_path)?;
        self.stats_persistence = Some(stats_persistence);
        self.initialize_stats_snapshot_schedule();
        Ok(())
    }

    pub fn set_tls_cert_path(&mut self, path: Option<PathBuf>) {
        self.tls_cert_path = path;
        self.tls_rotation_attempts = 0;
        self.refresh_tls_context_from_cert_path();
    }

    pub fn set_persistent_rooms_enabled(&mut self, enabled: bool) {
        self.persistent_rooms_enabled = enabled;
    }

    pub fn set_isolate_rooms(&mut self, enabled: bool) {
        self.isolate_rooms = enabled;
    }

    pub fn set_chat_enabled(&mut self, enabled: bool) {
        self.chat_enabled = enabled;
    }

    pub fn set_readiness_enabled(&mut self, enabled: bool) {
        self.readiness_enabled = enabled;
    }

    pub fn set_max_chat_message_length(&mut self, max_chars: usize) {
        self.max_chat_message_length = max_chars;
    }

    pub fn set_max_username_length(&mut self, max_chars: usize) {
        self.max_username_length = max_chars;
    }

    pub fn with_persistent_rooms_db_path(
        db_path: impl Into<PathBuf>,
    ) -> Result<Self, ServerRuntimeError> {
        let mut runtime = Self::new();
        runtime.set_persistent_rooms_db_path(Some(db_path.into()))?;
        runtime.set_persistent_rooms_enabled(true);
        Ok(runtime)
    }

    pub fn with_permanent_rooms_file_path(
        permanent_rooms_file_path: impl Into<PathBuf>,
    ) -> Result<Self, ServerRuntimeError> {
        let mut runtime = Self::new();
        runtime.set_permanent_rooms_file_path(Some(permanent_rooms_file_path.into()))?;
        Ok(runtime)
    }

    pub fn set_persistent_rooms_db_path(
        &mut self,
        db_path: Option<PathBuf>,
    ) -> Result<(), ServerRuntimeError> {
        let Some(db_path) = db_path else {
            self.room_persistence = None;
            return Ok(());
        };
        let persistence = RoomPersistenceStore::open(&db_path)?;
        let persisted_rooms = persistence.load_rooms()?;
        self.room_persistence = Some(persistence);
        self.apply_persisted_rooms_snapshot(persisted_rooms);
        self.apply_permanent_rooms_snapshot();
        Ok(())
    }

    pub fn set_permanent_rooms<I, S>(&mut self, permanent_rooms: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.permanent_rooms = permanent_rooms.into_iter().map(Into::into).collect();
        self.apply_permanent_rooms_snapshot();
    }

    pub fn set_permanent_rooms_file_path(
        &mut self,
        permanent_rooms_file_path: Option<PathBuf>,
    ) -> Result<(), ServerRuntimeError> {
        let Some(path) = permanent_rooms_file_path else {
            self.set_permanent_rooms(Vec::<String>::new());
            return Ok(());
        };
        if !path.is_file() {
            self.set_permanent_rooms(Vec::<String>::new());
            return Ok(());
        }
        let file_contents = fs::read_to_string(&path).map_err(|source| {
            ServerRuntimeError::PermanentRoomsFileRead {
                path: path.clone(),
                source,
            }
        })?;
        self.set_permanent_rooms(parse_permanent_rooms_file(&file_contents));
        Ok(())
    }

    pub fn bootstrap_room(&mut self, room_name: &str) {
        self.domain.join_room("bootstrap", room_name);
    }

    pub fn room_is_present(&self, room_name: &str) -> bool {
        self.domain.users_in_room(room_name).is_some()
    }

    pub fn session(&self, client_id: &str) -> Option<&ServerSession> {
        self.sessions.get(client_id)
    }

    pub fn tls_cert_path(&self) -> Option<PathBuf> {
        self.tls_cert_path.clone()
    }

    fn tls_server_config(&self) -> Option<Arc<ServerConfig>> {
        self.tls_server_config.clone()
    }

    pub fn set_time_now_override_seconds(&mut self, seconds: Option<f64>) {
        self.time_now_override_seconds = seconds;
    }

    pub fn drain_transport_actions(&mut self) -> Vec<DirectedTransportAction> {
        std::mem::take(&mut self.pending_transport_actions)
    }

    pub fn advance_time_and_collect_fanout(
        &mut self,
        delta_seconds: f64,
    ) -> Result<Vec<DirectedOutboundLine>, ServerRuntimeError> {
        let base_now = self.current_time_seconds();
        let advanced_now = if delta_seconds.is_finite() && delta_seconds > 0.0 {
            base_now + delta_seconds
        } else {
            base_now
        };
        self.time_now_override_seconds = Some(advanced_now);
        let outbound_messages = self.collect_due_periodic_updates()?;
        self.collect_due_stats_snapshots()?;
        outbound_messages
            .into_iter()
            .map(|message| {
                Ok(DirectedOutboundLine {
                    client_id: message.client_id,
                    line: encode_message_line(&message.message)?,
                })
            })
            .collect()
    }

    pub fn handle_line(
        &mut self,
        client_id: &str,
        json_line: &str,
    ) -> Result<Vec<String>, ServerRuntimeError> {
        let outbound_lines = self.handle_line_fanout(client_id, json_line)?;
        Ok(outbound_lines
            .into_iter()
            .filter(|line| line.client_id == client_id)
            .map(|line| line.line)
            .collect())
    }

    pub fn handle_line_fanout(
        &mut self,
        client_id: &str,
        json_line: &str,
    ) -> Result<Vec<DirectedOutboundLine>, ServerRuntimeError> {
        let message = decode_message_line(json_line)?;
        let outbound_messages = self.handle_protocol_message_fanout(client_id, message)?;
        outbound_messages
            .into_iter()
            .map(|message| {
                Ok(DirectedOutboundLine {
                    client_id: message.client_id,
                    line: encode_message_line(&message.message)?,
                })
            })
            .collect()
    }

    pub fn handle_transport_disconnect_fanout(
        &mut self,
        client_id: &str,
    ) -> Result<Vec<DirectedOutboundLine>, ServerRuntimeError> {
        let outbound_messages = self.timeout_disconnect_messages(client_id)?;
        outbound_messages
            .into_iter()
            .map(|message| {
                Ok(DirectedOutboundLine {
                    client_id: message.client_id,
                    line: encode_message_line(&message.message)?,
                })
            })
            .collect()
    }

    pub fn handle_line_fanout_with_transport_actions(
        &mut self,
        client_id: &str,
        json_line: &str,
    ) -> Result<ServerRuntimeDispatch, ServerRuntimeError> {
        let outbound_lines = self.handle_line_fanout(client_id, json_line)?;
        let transport_actions = self.drain_transport_actions();
        Ok(ServerRuntimeDispatch {
            outbound_lines,
            transport_actions,
        })
    }

    pub fn handle_protocol_message(
        &mut self,
        client_id: &str,
        message: ProtocolMessage,
    ) -> Result<Vec<ProtocolMessage>, ServerRuntimeError> {
        let outbound_messages = self.handle_protocol_message_fanout(client_id, message)?;
        Ok(outbound_messages
            .into_iter()
            .filter(|message| message.client_id == client_id)
            .map(|message| message.message)
            .collect())
    }

    pub fn handle_protocol_message_fanout(
        &mut self,
        client_id: &str,
        message: ProtocolMessage,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        match message {
            ProtocolMessage::Hello(payload) => self.handle_hello(client_id, payload.hello),
            ProtocolMessage::Set(payload) => self.handle_set(client_id, payload.set),
            ProtocolMessage::List(payload) => self.handle_list(client_id, payload.list),
            ProtocolMessage::State(payload) => self.handle_state(client_id, payload.state),
            ProtocolMessage::Tls(payload) => self.handle_tls(client_id, payload.tls),
            ProtocolMessage::Chat(payload) => self.handle_chat(client_id, payload.chat),
            ProtocolMessage::Error(_) => Ok(Vec::new()),
        }
    }

    fn handle_chat(
        &mut self,
        client_id: &str,
        chat: ChatPayload,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        if !self.chat_enabled {
            return Ok(Vec::new());
        }
        let session = self
            .sessions
            .get(client_id)
            .ok_or_else(|| ServerRuntimeError::MissingSession(client_id.to_owned()))?;
        let message = match chat {
            ChatPayload::Text(message) => message,
            ChatPayload::Message(message_payload) => message_payload.message,
        };
        let message = truncate_text_to_max_chars(&message, self.max_chat_message_length);
        let outbound_message = ProtocolMessage::chat_message(session.username.clone(), message);
        Ok(self
            .clients_in_room(&session.room)
            .into_iter()
            .map(|peer_client| DirectedProtocolMessage::new(&peer_client, outbound_message.clone()))
            .collect())
    }

    fn handle_tls(
        &mut self,
        client_id: &str,
        tls: TlsPayload,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        if !tls.start_tls.contains("send") {
            return Ok(Vec::new());
        }
        let should_start_tls = if !self.sessions.contains_key(client_id) && self.server_accepts_tls
        {
            self.refresh_tls_context_after_cert_rotation_if_needed();
            self.tls_context_available
        } else {
            false
        };
        if should_start_tls {
            self.pending_transport_actions
                .push(DirectedTransportAction::new(
                    client_id,
                    ServerTransportAction::StartTls,
                ));
        }
        let start_tls = if should_start_tls { "true" } else { "false" };
        Ok(vec![DirectedProtocolMessage::new(
            client_id,
            ProtocolMessage::start_tls(start_tls),
        )])
    }

    fn handle_hello(
        &mut self,
        client_id: &str,
        hello: HelloPayload,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let requested_username =
            truncate_text_to_max_chars(hello.username.trim(), self.max_username_length);
        let room_name = hello.room.name.trim();
        let version = hello.effective_version().trim();
        if requested_username.is_empty() || room_name.is_empty() || version.is_empty() {
            return Err(ServerRuntimeError::InvalidHello);
        }
        if let Some(required_password_token) = self.server_password_token.as_deref() {
            let Some(server_password_token) = hello_server_password_token(&hello) else {
                return Ok(vec![DirectedProtocolMessage::new(
                    client_id,
                    ProtocolMessage::error_message(LEGACY_SERVER_PASSWORD_REQUIRED_ERROR),
                )]);
            };
            if !server_password_token_matches_legacy_compatible(
                server_password_token,
                required_password_token,
            ) {
                return Ok(vec![DirectedProtocolMessage::new(
                    client_id,
                    ProtocolMessage::error_message(LEGACY_SERVER_WRONG_PASSWORD_ERROR),
                )]);
            }
        }

        let advertised_features = hello.features.clone();
        if let Some(previous_session) = self.remove_session_tracking(client_id) {
            self.cleanup_room_if_empty(&previous_session.room)?;
        }
        let username = self.find_free_username(&requested_username, Some(client_id));
        self.domain.join_room(&username, room_name);
        self.ensure_room_state(room_name);
        self.sessions.insert(
            client_id.to_owned(),
            ServerSession {
                username: username.to_owned(),
                room: room_name.to_owned(),
                version: version.to_owned(),
                features: advertised_features.clone(),
            },
        );
        self.client_state_counters
            .insert(client_id.to_owned(), ClientStateCounters::default());
        let now = self.current_time_seconds();
        self.client_last_state_update_at
            .insert(client_id.to_owned(), now);
        self.client_next_periodic_state_at
            .insert(client_id.to_owned(), now + SERVER_STATE_INTERVAL_SECONDS);

        let mut outbound = Vec::new();
        let joined_message = user_joined_message_with_metadata(
            &username,
            room_name,
            version,
            advertised_features.clone(),
        );
        for existing_client in self.clients_visible_on_join(room_name, client_id) {
            outbound.push(DirectedProtocolMessage::new(
                existing_client,
                joined_message.clone(),
            ));
        }

        let room_playlist = self.room_playlist_state(room_name);
        let room_playback = self.room_playback_state(room_name);
        let playlist_snapshot_message = playlist_snapshot_change_message(
            room_playlist.files.clone(),
            room_playback.set_by.as_deref(),
        );
        outbound.push(DirectedProtocolMessage::new(
            client_id,
            playlist_snapshot_message,
        ));
        if let Some(index) = room_playlist.index {
            outbound.push(DirectedProtocolMessage::new(
                client_id,
                playlist_snapshot_index_message(index, room_playback.set_by.as_deref()),
            ));
        }

        let base_motd = motd_for_client_version(version, self.motd_template.as_deref());
        let motd = persistent_rooms_notice_motd(
            base_motd,
            self.persistent_rooms_enabled,
            advertised_features.as_ref(),
        );
        let mut response = HelloPayload::new(username.clone(), room_name, version)
            .with_realversion(SERVER_REAL_VERSION)
            .with_features(server_feature_list(
                self.persistent_rooms_enabled,
                self.isolate_rooms,
                self.chat_enabled,
                self.readiness_enabled,
                self.max_chat_message_length,
                self.max_username_length,
            ));
        response
            .extra
            .insert("motd".to_owned(), Value::String(motd));
        outbound.push(DirectedProtocolMessage::new(
            client_id,
            ProtocolMessage::hello(response),
        ));

        if self.persistent_rooms_enabled {
            self.enqueue_list_snapshots_for_clients(
                &mut outbound,
                self.clients_receiving_to_gui_only_list_updates(),
            );
        }

        if self.clients_in_room(room_name).len() > 1 {
            let idle_state_message = room_idle_state_message(self.current_time_seconds());
            for room_client in self.clients_in_room(room_name) {
                outbound.push(DirectedProtocolMessage::new(
                    room_client,
                    idle_state_message.clone(),
                ));
            }
        }

        Ok(outbound)
    }

    fn handle_list(
        &self,
        client_id: &str,
        list: ListPayload,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        if !self.sessions.contains_key(client_id) {
            return Err(ServerRuntimeError::MissingSession(client_id.to_owned()));
        }
        match list {
            ListPayload::Request(_) => {
                let rooms = self.list_rooms_snapshot_for_client(client_id);
                Ok(vec![DirectedProtocolMessage::new(
                    client_id,
                    ProtocolMessage::list(ListPayload::rooms(rooms)),
                )])
            }
            ListPayload::Rooms(_) => Ok(Vec::new()),
        }
    }

    fn handle_set(
        &mut self,
        client_id: &str,
        set: SetPayload,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let mut session = self
            .sessions
            .get(client_id)
            .cloned()
            .ok_or_else(|| ServerRuntimeError::MissingSession(client_id.to_owned()))?;

        let mut outbound_messages = Vec::new();

        if let Some(room_ref) = set.room
            && session.room != room_ref.name
        {
            let previous_room = session.room.clone();
            self.domain.leave_room(&session.username, &previous_room)?;
            self.remove_room_controller(&session.username, &previous_room);
            self.domain.join_room(&session.username, &room_ref.name);
            self.ensure_room_state(&room_ref.name);
            session.room = room_ref.name.clone();
            self.sessions.insert(client_id.to_owned(), session.clone());
            self.client_next_periodic_state_at.insert(
                client_id.to_owned(),
                self.current_time_seconds() + SERVER_STATE_INTERVAL_SECONDS,
            );
            self.cleanup_room_if_empty(&previous_room)?;

            outbound_messages.push(DirectedProtocolMessage::new(
                client_id,
                room_idle_state_message(self.current_time_seconds()),
            ));
            outbound_messages.push(DirectedProtocolMessage::new(
                client_id,
                self.forced_state_sync_message_for_client(client_id, 0.0, true, true, None),
            ));

            let room_update_message = user_room_update_message(&session.username, &session.room);
            for peer_client in
                self.room_switch_visibility_recipients(client_id, &previous_room, &session.room)
            {
                outbound_messages.push(DirectedProtocolMessage::new(
                    peer_client,
                    room_update_message.clone(),
                ));
            }
            if self.isolate_rooms {
                let left_message = user_event_message(
                    &session.username,
                    &previous_room,
                    json!({
                        "left": true,
                    }),
                );
                for peer_client in self.clients_in_room(&previous_room) {
                    outbound_messages.push(DirectedProtocolMessage::new(
                        peer_client,
                        left_message.clone(),
                    ));
                }
            }

            let room_playlist = self.room_playlist_state(&session.room);
            let room_playback = self.room_playback_state(&session.room);
            let playlist_snapshot_message = playlist_snapshot_change_message(
                room_playlist.files.clone(),
                room_playback.set_by.as_deref(),
            );
            outbound_messages.push(DirectedProtocolMessage::new(
                client_id,
                playlist_snapshot_message,
            ));
            if let Some(index) = room_playlist.index {
                outbound_messages.push(DirectedProtocolMessage::new(
                    client_id,
                    playlist_snapshot_index_message(index, room_playback.set_by.as_deref()),
                ));
            }

            if self.persistent_rooms_enabled {
                self.enqueue_list_snapshots_for_clients(
                    &mut outbound_messages,
                    self.clients_receiving_to_gui_only_list_updates(),
                );
            }
        }

        if let Some(mut playlist_change) = set.playlist_change {
            self.ensure_room_state(&session.room);
            if self.user_can_control_playlist(&session.username, &session.room) {
                let new_files = playlist_change.files.clone();
                self.room_playlist_state_mut(&session.room).files = new_files;
                self.persist_room_if_needed(&session.room)?;
                playlist_change.user = Some(session.username.clone());
                let playlist_message =
                    ProtocolMessage::set(SetPayload::new().with_playlist_change(playlist_change));
                for peer_client in self.clients_in_room(&session.room) {
                    outbound_messages.push(DirectedProtocolMessage::new(
                        peer_client,
                        playlist_message.clone(),
                    ));
                }
            } else {
                let room_state = self.room_playlist_state(&session.room);
                let correction_change = ProtocolMessage::set(
                    SetPayload::new().with_playlist_change(
                        PlaylistChangePayload::new(room_state.files.iter().cloned())
                            .with_user(session.room.clone()),
                    ),
                );
                outbound_messages.push(DirectedProtocolMessage::new(client_id, correction_change));
            }
        }

        if let Some(mut playlist_index) = set.playlist_index {
            self.ensure_room_state(&session.room);
            if self.user_can_control_playlist(&session.username, &session.room) {
                self.room_playlist_state_mut(&session.room).index = Some(playlist_index.index);
                self.persist_room_if_needed(&session.room)?;
                playlist_index.user = Some(session.username.clone());
                let playlist_message =
                    ProtocolMessage::set(SetPayload::new().with_playlist_index(playlist_index));
                for peer_client in self.clients_in_room(&session.room) {
                    outbound_messages.push(DirectedProtocolMessage::new(
                        peer_client,
                        playlist_message.clone(),
                    ));
                }
            }
        }

        if let Some(controller_auth) = set.controller_auth {
            let room_to_check = controller_auth.room.unwrap_or_else(|| session.room.clone());
            let auth_password = controller_auth.password.unwrap_or_default();
            match self
                .room_password_provider
                .check(&room_to_check, &auth_password)
            {
                Ok(success) => {
                    if success {
                        self.add_room_controller(&session.username, &session.room);
                    }
                    let auth_message =
                        controller_auth_status_message(&session.username, &session.room, success);
                    let recipients = if self.isolate_rooms {
                        self.clients_in_room(&session.room)
                    } else {
                        self.clients_all()
                    };
                    for peer_client in recipients {
                        outbound_messages.push(DirectedProtocolMessage::new(
                            peer_client,
                            auth_message.clone(),
                        ));
                    }
                }
                Err(RoomPasswordCheckError::NotControlledRoom) => {
                    let new_room_name = self
                        .room_password_provider
                        .controlled_room_name_for(&room_to_check, &auth_password);
                    let new_room_message =
                        new_controlled_room_message(&new_room_name, &auth_password);
                    outbound_messages
                        .push(DirectedProtocolMessage::new(client_id, new_room_message));
                }
                Err(RoomPasswordCheckError::InvalidPassword) => {
                    let auth_message =
                        controller_auth_status_message(&session.username, &session.room, false);
                    for peer_client in self.clients_in_room(&session.room) {
                        outbound_messages.push(DirectedProtocolMessage::new(
                            peer_client,
                            auth_message.clone(),
                        ));
                    }
                }
            }
        }

        if self.readiness_enabled
            && let Some(ready) = set.ready
        {
            self.domain
                .set_ready(&session.username, &session.room, ready.is_ready)?;
            let ready_message = ready_update_message(
                &session.username,
                ready.is_ready,
                ready.manually_initiated.unwrap_or(true),
                ready.set_by.as_deref(),
            );
            for peer_client in self.clients_in_room(&session.room) {
                outbound_messages.push(DirectedProtocolMessage::new(
                    peer_client,
                    ready_message.clone(),
                ));
            }
        }

        Ok(outbound_messages)
    }

    fn handle_state(
        &mut self,
        client_id: &str,
        state: StatePayload,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let session = self
            .sessions
            .get(client_id)
            .cloned()
            .ok_or_else(|| ServerRuntimeError::MissingSession(client_id.to_owned()))?;

        if let Some(ignore) = state.ignoring_on_the_fly.as_ref() {
            if let Some(server_ignoring_counter) = ignore.server {
                self.acknowledge_server_ignoring_counter(client_id, server_ignoring_counter);
            }
            if let Some(client_ignoring_counter) = ignore.client {
                self.queue_client_ignoring_counter(client_id, client_ignoring_counter);
            }
        }
        if let Some(ping) = state.ping.as_ref() {
            if let Some(client_latency_calculation) = ping.client_latency_calculation {
                self.queue_client_latency_calculation(client_id, client_latency_calculation);
            }
            self.ingest_client_ping_metrics(client_id, ping.latency_calculation, ping.client_rtt);
        }
        if self.server_ignoring_counter(client_id) > 0 {
            return Ok(Vec::new());
        }
        self.record_client_state_update_now(client_id);

        let Some(playstate) = state.playstate else {
            return Ok(Vec::new());
        };

        self.ensure_room_state(&session.room);
        let room_state_before = self.room_playback_state(&session.room);
        let can_control_room = self.user_can_control_playlist(&session.username, &session.room);
        let do_seek = playstate.do_seek.unwrap_or(false);
        let forward_delay_seconds = self.forward_delay_seconds(client_id);
        let pause_changed = playstate
            .paused
            .is_some_and(|paused| paused != room_state_before.paused);

        if can_control_room {
            let room_state = self.room_playback_state_mut(&session.room);
            if let Some(paused) = playstate.paused {
                room_state.paused = paused;
            }
            if let Some(mut position) = playstate.position {
                if !playstate.paused.unwrap_or(false) {
                    position += forward_delay_seconds;
                }
                room_state.position = position;
            }
            room_state.set_by = Some(session.username.clone());
            self.persist_room_if_needed(&session.room)?;
        }

        if !do_seek && !pause_changed {
            return Ok(Vec::new());
        }

        if can_control_room {
            let room_state = self.room_playback_state(&session.room);
            let mut outbound_messages = Vec::new();
            for peer_client in self.clients_in_room(&session.room) {
                let state_message = self.forced_state_sync_message_for_client(
                    &peer_client,
                    room_state.position,
                    room_state.paused,
                    do_seek,
                    Some(&session.username),
                );
                outbound_messages.push(DirectedProtocolMessage::new(peer_client, state_message));
            }
            return Ok(outbound_messages);
        }

        let watcher_pause_state = playstate.paused.unwrap_or(room_state_before.paused);
        Ok(vec![
            DirectedProtocolMessage::new(
                client_id,
                self.forced_state_sync_message_for_client(
                    client_id,
                    room_state_before.position,
                    watcher_pause_state,
                    false,
                    Some(&session.username),
                ),
            ),
            DirectedProtocolMessage::new(
                client_id,
                self.forced_state_sync_message_for_client(
                    client_id,
                    room_state_before.position,
                    room_state_before.paused,
                    true,
                    room_state_before.set_by.as_deref(),
                ),
            ),
        ])
    }

    fn current_time_seconds(&self) -> f64 {
        self.time_now_override_seconds
            .unwrap_or_else(current_unix_timestamp_seconds)
    }

    fn record_client_state_update_now(&mut self, client_id: &str) {
        self.client_last_state_update_at
            .insert(client_id.to_owned(), self.current_time_seconds());
    }

    fn initialize_stats_snapshot_schedule(&mut self) {
        if self.stats_persistence.is_none() {
            self.stats_next_snapshot_at_seconds = None;
            return;
        }
        self.stats_next_snapshot_at_seconds = Some(
            self.current_time_seconds()
                + self.stats_snapshot_start_delay_seconds
                + self.stats_snapshot_interval_seconds,
        );
    }

    fn refresh_tls_context_from_cert_path(&mut self) {
        let Some(path) = self.tls_cert_path.as_ref() else {
            self.tls_server_config = None;
            self.tls_context_available = false;
            self.server_accepts_tls = false;
            self.tls_last_edit_cert_time = None;
            return;
        };
        if !tls_certificate_bundle_is_available(path) {
            self.tls_server_config = None;
            self.tls_context_available = false;
            self.server_accepts_tls = false;
            self.tls_last_edit_cert_time = None;
            return;
        }
        self.tls_last_edit_cert_time = tls_certificate_file_modified_time(path);
        match load_tls_server_config(path) {
            Ok(server_config) => {
                self.tls_server_config = Some(server_config);
                self.tls_context_available = true;
                self.server_accepts_tls = true;
            }
            Err(_) => {
                self.tls_server_config = None;
                self.tls_context_available = false;
                self.server_accepts_tls = false;
            }
        }
    }

    fn refresh_tls_context_after_cert_rotation_if_needed(&mut self) {
        let Some(path) = self.tls_cert_path.as_ref() else {
            return;
        };
        let Some(current_edit_time) = tls_certificate_file_modified_time(path) else {
            return;
        };
        if Some(current_edit_time) == self.tls_last_edit_cert_time {
            return;
        }
        self.refresh_tls_context_after_rotation_attempt();
    }

    fn refresh_tls_context_after_rotation_attempt(&mut self) {
        self.refresh_tls_context_from_cert_path();
        self.tls_rotation_attempts = self.tls_rotation_attempts.saturating_add(1);
        if self.tls_rotation_attempts < TLS_CERT_ROTATION_MAX_RETRIES {
            self.server_accepts_tls = true;
        }
    }

    fn collect_due_stats_snapshots(&mut self) -> Result<(), ServerRuntimeError> {
        let Some(stats_persistence) = self.stats_persistence.clone() else {
            self.stats_next_snapshot_at_seconds = None;
            return Ok(());
        };
        if self.stats_next_snapshot_at_seconds.is_none() {
            self.initialize_stats_snapshot_schedule();
        }
        let Some(mut next_snapshot_at_seconds) = self.stats_next_snapshot_at_seconds else {
            return Ok(());
        };
        let now_seconds = self.current_time_seconds();
        while next_snapshot_at_seconds <= now_seconds {
            self.record_stats_snapshot_at(&stats_persistence, next_snapshot_at_seconds)?;
            next_snapshot_at_seconds += self.stats_snapshot_interval_seconds;
        }
        self.stats_next_snapshot_at_seconds = Some(next_snapshot_at_seconds);
        Ok(())
    }

    fn record_stats_snapshot_at(
        &self,
        stats_persistence: &StatsPersistenceStore,
        snapshot_at_seconds: f64,
    ) -> Result<(), ServerRuntimeError> {
        let snapshot_time = snapshot_at_seconds.floor() as i64;
        let mut versions: Vec<String> = self
            .sessions
            .values()
            .map(|session| session.version.clone())
            .collect();
        versions.sort();
        for version in versions {
            stats_persistence.add_version_log(snapshot_time, &version)?;
        }
        Ok(())
    }

    fn collect_due_periodic_updates(
        &mut self,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let now = self.current_time_seconds();
        let mut due_clients: Vec<String> = self
            .client_next_periodic_state_at
            .iter()
            .filter(|(_, next_state_at)| **next_state_at <= now)
            .map(|(client_id, _)| client_id.clone())
            .collect();
        due_clients.sort();

        let mut outbound = Vec::new();
        for client_id in due_clients {
            let Some(mut next_state_at) =
                self.client_next_periodic_state_at.get(&client_id).copied()
            else {
                continue;
            };
            while next_state_at <= now {
                self.client_next_periodic_state_at.insert(
                    client_id.clone(),
                    next_state_at + SERVER_STATE_INTERVAL_SECONDS,
                );
                outbound.extend(self.collect_periodic_tick_for_client(&client_id, next_state_at)?);
                if !self.sessions.contains_key(&client_id) {
                    break;
                }
                next_state_at += SERVER_STATE_INTERVAL_SECONDS;
            }
        }

        Ok(outbound)
    }

    fn collect_periodic_tick_for_client(
        &mut self,
        client_id: &str,
        ticked_at: f64,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let Some(session) = self.sessions.get(client_id).cloned() else {
            return Ok(Vec::new());
        };
        self.ensure_room_state(&session.room);
        if self.room_playback_state(&session.room).set_by.is_none()
            && let Some(set_by_username) = self.fallback_room_set_by_username(&session.room)
        {
            self.room_playback_state_mut(&session.room).set_by = Some(set_by_username);
        }
        let room_state = self.room_playback_state(&session.room);

        let mut outbound = Vec::new();
        if let Some(state_message) = self.periodic_state_sync_message_for_client(
            client_id,
            room_state.position,
            room_state.paused,
            room_state.set_by.as_deref(),
        ) {
            outbound.push(DirectedProtocolMessage::new(client_id, state_message));
        }

        if self.client_timed_out(client_id, ticked_at) {
            outbound.extend(self.timeout_disconnect_messages(client_id)?);
        }

        Ok(outbound)
    }

    fn fallback_room_set_by_username(&self, room_name: &str) -> Option<String> {
        let mut usernames: Vec<String> = self
            .sessions
            .values()
            .filter(|session| session.room == room_name)
            .map(|session| session.username.clone())
            .collect();
        usernames.sort();
        usernames.into_iter().next()
    }

    fn periodic_state_sync_message_for_client(
        &mut self,
        client_id: &str,
        position: f64,
        paused: bool,
        set_by: Option<&str>,
    ) -> Option<ProtocolMessage> {
        let server_ignoring_counter = self.server_ignoring_counter(client_id);
        let server_rtt_seconds = self.server_rtt_seconds(client_id);
        let (pending_client_latency, pending_client_ignoring) =
            self.take_client_passthrough_state_metadata(client_id);
        if server_ignoring_counter > 0 {
            return None;
        }

        Some(state_sync_message(
            position,
            paused,
            false,
            StateSyncOptions {
                set_by,
                client_latency_calculation: pending_client_latency,
                client_ignoring_counter: pending_client_ignoring,
                server_rtt_seconds,
                latency_calculation_seconds: Some(self.current_time_seconds()),
                ..StateSyncOptions::default()
            },
        ))
    }

    fn client_timed_out(&self, client_id: &str, now_seconds: f64) -> bool {
        self.client_last_state_update_at
            .get(client_id)
            .is_some_and(|updated_at| now_seconds - updated_at > PROTOCOL_TIMEOUT_SECONDS)
    }

    fn timeout_disconnect_messages(
        &mut self,
        client_id: &str,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let Some(session) = self.remove_session_tracking(client_id) else {
            return Ok(Vec::new());
        };
        self.cleanup_room_if_empty(&session.room)?;
        let left_message = user_event_message(
            &session.username,
            &session.room,
            json!({
                "left": true,
            }),
        );
        let mut recipients = if self.isolate_rooms {
            self.clients_in_room(&session.room)
        } else {
            self.clients_all()
        };
        recipients.push(client_id.to_owned());
        let mut outbound_messages: Vec<_> = recipients
            .into_iter()
            .map(|peer_client| DirectedProtocolMessage::new(peer_client, left_message.clone()))
            .collect();
        if self.persistent_rooms_enabled {
            self.enqueue_list_snapshots_for_clients(
                &mut outbound_messages,
                self.clients_receiving_to_gui_only_list_updates(),
            );
        }
        Ok(outbound_messages)
    }

    fn remove_session_tracking(&mut self, client_id: &str) -> Option<ServerSession> {
        let session = self.sessions.remove(client_id)?;
        let _ = self.domain.leave_room(&session.username, &session.room);
        self.remove_room_controller(&session.username, &session.room);
        self.client_state_counters.remove(client_id);
        self.client_last_state_update_at.remove(client_id);
        self.client_next_periodic_state_at.remove(client_id);
        Some(session)
    }

    fn apply_persisted_rooms_snapshot(
        &mut self,
        persisted_rooms: BTreeMap<String, PersistedRoomState>,
    ) {
        for (room_name, persisted_room) in persisted_rooms {
            self.room_playlists.insert(
                room_name.clone(),
                RoomPlaylistState {
                    files: persisted_room.files,
                    index: persisted_room.index,
                },
            );
            let room_playback = self.room_playback_states.entry(room_name).or_default();
            room_playback.position = persisted_room.position;
        }
    }

    fn apply_permanent_rooms_snapshot(&mut self) {
        if self.room_persistence.is_none() {
            return;
        }
        for room_name in self.permanent_rooms.clone() {
            self.room_playlists
                .entry(room_name.clone())
                .or_insert_with(|| RoomPlaylistState {
                    files: Vec::new(),
                    index: Some(0),
                });
            self.room_controllers.entry(room_name.clone()).or_default();
            self.room_playback_states.entry(room_name).or_default();
        }
    }

    fn room_is_persistent(&self, room_name: &str) -> bool {
        self.persistent_rooms_enabled && !room_name_is_marked_temporary(room_name)
    }

    fn room_is_permanent(&self, room_name: &str) -> bool {
        self.room_persistence.is_some() && self.permanent_rooms.contains(room_name)
    }

    fn room_should_be_retained_when_empty(&self, room_name: &str) -> bool {
        self.room_is_persistent(room_name) && !self.room_playlist_state(room_name).files.is_empty()
    }

    fn persist_room_if_needed(&self, room_name: &str) -> Result<(), ServerRuntimeError> {
        if !self.room_is_persistent(room_name) {
            return Ok(());
        }
        let Some(room_persistence) = self.room_persistence.as_ref() else {
            return Ok(());
        };
        let playlist = self.room_playlist_state(room_name);
        let playback = self.room_playback_state(room_name);
        room_persistence.save_room(
            room_name,
            &playlist.files,
            playlist.index,
            playback.position,
        )?;
        Ok(())
    }

    fn delete_persisted_room_if_needed(&self, room_name: &str) -> Result<(), ServerRuntimeError> {
        let Some(room_persistence) = self.room_persistence.as_ref() else {
            return Ok(());
        };
        room_persistence.delete_room(room_name)?;
        Ok(())
    }

    fn cleanup_room_if_empty(&mut self, room_name: &str) -> Result<(), ServerRuntimeError> {
        if !self.clients_in_room(room_name).is_empty() {
            return Ok(());
        }
        if self.room_is_permanent(room_name) {
            self.persist_room_if_needed(room_name)?;
            return Ok(());
        }
        if self.room_should_be_retained_when_empty(room_name) {
            self.persist_room_if_needed(room_name)?;
            return Ok(());
        }
        self.room_controllers.remove(room_name);
        self.room_playlists.remove(room_name);
        self.room_playback_states.remove(room_name);
        self.delete_persisted_room_if_needed(room_name)?;
        Ok(())
    }

    fn ensure_room_state(&mut self, room_name: &str) {
        self.room_playlists.entry(room_name.to_owned()).or_default();
        self.room_controllers
            .entry(room_name.to_owned())
            .or_default();
        self.room_playback_states
            .entry(room_name.to_owned())
            .or_default();
    }

    fn room_playlist_state_mut(&mut self, room_name: &str) -> &mut RoomPlaylistState {
        self.room_playlists.entry(room_name.to_owned()).or_default()
    }

    fn room_playlist_state(&self, room_name: &str) -> RoomPlaylistState {
        self.room_playlists
            .get(room_name)
            .cloned()
            .unwrap_or_default()
    }

    fn room_playback_state_mut(&mut self, room_name: &str) -> &mut RoomPlaybackState {
        self.room_playback_states
            .entry(room_name.to_owned())
            .or_default()
    }

    fn room_playback_state(&self, room_name: &str) -> RoomPlaybackState {
        self.room_playback_states
            .get(room_name)
            .cloned()
            .unwrap_or_default()
    }

    fn acknowledge_server_ignoring_counter(&mut self, client_id: &str, server_counter: u32) {
        let Some(state_counters) = self.client_state_counters.get_mut(client_id) else {
            return;
        };
        if state_counters.server_ignoring_on_the_fly == server_counter {
            state_counters.server_ignoring_on_the_fly = 0;
        }
    }

    fn server_ignoring_counter(&self, client_id: &str) -> u32 {
        self.client_state_counters
            .get(client_id)
            .map(|state_counters| state_counters.server_ignoring_on_the_fly)
            .unwrap_or_default()
    }

    fn next_server_ignoring_counter(&mut self, client_id: &str) -> u32 {
        let state_counters = self
            .client_state_counters
            .entry(client_id.to_owned())
            .or_default();
        state_counters.server_ignoring_on_the_fly =
            state_counters.server_ignoring_on_the_fly.saturating_add(1);
        state_counters.server_ignoring_on_the_fly
    }

    fn queue_client_ignoring_counter(&mut self, client_id: &str, client_ignoring_counter: u32) {
        let state_counters = self
            .client_state_counters
            .entry(client_id.to_owned())
            .or_default();
        state_counters.pending_client_ignoring_on_the_fly = Some(client_ignoring_counter);
    }

    fn queue_client_latency_calculation(&mut self, client_id: &str, client_latency: f64) {
        let state_counters = self
            .client_state_counters
            .entry(client_id.to_owned())
            .or_default();
        state_counters.pending_client_latency_calculation = Some(client_latency);
    }

    fn ingest_client_ping_metrics(
        &mut self,
        client_id: &str,
        latency_calculation: Option<f64>,
        client_rtt: Option<f64>,
    ) {
        let Some(latency_calculation) = latency_calculation else {
            return;
        };
        let sender_rtt = client_rtt.unwrap_or(0.0);
        if !latency_calculation.is_finite() || !sender_rtt.is_finite() || sender_rtt < 0.0 {
            return;
        }

        let current_rtt_seconds = self.current_time_seconds() - latency_calculation;
        if !current_rtt_seconds.is_finite() || current_rtt_seconds < 0.0 {
            return;
        }

        let state_counters = self
            .client_state_counters
            .entry(client_id.to_owned())
            .or_default();
        state_counters.ping_rtt_seconds = current_rtt_seconds;
        if state_counters.ping_average_rtt_seconds == 0.0 {
            state_counters.ping_average_rtt_seconds = current_rtt_seconds;
        }
        state_counters.ping_average_rtt_seconds = state_counters.ping_average_rtt_seconds
            * PING_MOVING_AVERAGE_WEIGHT
            + current_rtt_seconds * (1.0 - PING_MOVING_AVERAGE_WEIGHT);
        if sender_rtt < current_rtt_seconds {
            state_counters.ping_forward_delay_seconds =
                state_counters.ping_average_rtt_seconds / 2.0 + (current_rtt_seconds - sender_rtt);
        } else {
            state_counters.ping_forward_delay_seconds =
                state_counters.ping_average_rtt_seconds / 2.0;
        }
    }

    fn server_rtt_seconds(&self, client_id: &str) -> f64 {
        self.client_state_counters
            .get(client_id)
            .map(|state_counters| state_counters.ping_rtt_seconds)
            .unwrap_or_default()
    }

    fn forward_delay_seconds(&self, client_id: &str) -> f64 {
        self.client_state_counters
            .get(client_id)
            .map(|state_counters| state_counters.ping_forward_delay_seconds)
            .unwrap_or_default()
    }

    fn take_client_passthrough_state_metadata(
        &mut self,
        client_id: &str,
    ) -> (Option<f64>, Option<u32>) {
        let state_counters = self
            .client_state_counters
            .entry(client_id.to_owned())
            .or_default();
        let pending_client_latency = state_counters.pending_client_latency_calculation.take();
        let pending_client_ignoring = state_counters.pending_client_ignoring_on_the_fly.take();
        (pending_client_latency, pending_client_ignoring)
    }

    fn forced_state_sync_message_for_client(
        &mut self,
        client_id: &str,
        position: f64,
        paused: bool,
        do_seek: bool,
        set_by: Option<&str>,
    ) -> ProtocolMessage {
        let server_ignoring_counter = self.next_server_ignoring_counter(client_id);
        let server_rtt_seconds = self.server_rtt_seconds(client_id);
        let (pending_client_latency, pending_client_ignoring) =
            self.take_client_passthrough_state_metadata(client_id);
        state_sync_message(
            position,
            paused,
            do_seek,
            StateSyncOptions {
                set_by,
                server_ignoring_counter: Some(server_ignoring_counter),
                client_latency_calculation: pending_client_latency,
                client_ignoring_counter: pending_client_ignoring,
                server_rtt_seconds,
                latency_calculation_seconds: Some(self.current_time_seconds()),
            },
        )
    }

    fn add_room_controller(&mut self, username: &str, room_name: &str) {
        self.ensure_room_state(room_name);
        if let Some(room_controllers) = self.room_controllers.get_mut(room_name) {
            room_controllers.insert(username.to_owned());
        }
    }

    fn remove_room_controller(&mut self, username: &str, room_name: &str) {
        if let Some(room_controllers) = self.room_controllers.get_mut(room_name) {
            room_controllers.remove(username);
        }
    }

    fn user_is_room_controller(&self, username: &str, room_name: &str) -> bool {
        self.room_controllers
            .get(room_name)
            .is_some_and(|controllers| controllers.contains(username))
    }

    fn user_can_control_playlist(&self, username: &str, room_name: &str) -> bool {
        !self
            .room_password_provider
            .is_controlled_room_name(room_name)
            || self.user_is_room_controller(username, room_name)
    }

    fn clients_in_room(&self, room_name: &str) -> Vec<String> {
        self.sessions
            .iter()
            .filter(|(_, session)| session.room == room_name)
            .map(|(client_id, _)| client_id.clone())
            .collect()
    }

    fn clients_all(&self) -> Vec<String> {
        self.sessions.keys().cloned().collect()
    }

    fn clients_receiving_to_gui_only_list_updates(&self) -> Vec<String> {
        self.sessions
            .iter()
            .filter(|(_, session)| features_include_ui_mode(session.features.as_ref()))
            .map(|(client_id, _)| client_id.clone())
            .collect()
    }

    fn clients_all_excluding(&self, excluded_client_id: &str) -> Vec<String> {
        self.sessions
            .keys()
            .filter(|client_id| client_id.as_str() != excluded_client_id)
            .cloned()
            .collect()
    }

    fn clients_visible_on_join(&self, room_name: &str, joining_client_id: &str) -> Vec<String> {
        if self.isolate_rooms {
            self.clients_in_room(room_name)
                .into_iter()
                .filter(|client_id| client_id != joining_client_id)
                .collect()
        } else {
            self.clients_all_excluding(joining_client_id)
        }
    }

    fn room_switch_visibility_recipients(
        &self,
        moving_client_id: &str,
        _previous_room: &str,
        current_room: &str,
    ) -> Vec<String> {
        if !self.isolate_rooms {
            return self.clients_all();
        }
        let mut recipients = BTreeSet::new();
        recipients.insert(moving_client_id.to_owned());
        for client_id in self.clients_in_room(current_room) {
            recipients.insert(client_id);
        }
        recipients.into_iter().collect()
    }

    fn user_ready(&self, username: &str, room_name: &str) -> Option<bool> {
        if !self.readiness_enabled {
            return None;
        }
        self.domain.users_in_room(room_name).and_then(|users| {
            users
                .into_iter()
                .find(|user| user.username == username)
                .map(|user| user.ready)
        })
    }

    fn list_rooms_snapshot_for_client(
        &self,
        client_id: &str,
    ) -> BTreeMap<String, BTreeMap<String, ListUserEntry>> {
        if self.isolate_rooms {
            let Some(session) = self.sessions.get(client_id) else {
                return BTreeMap::new();
            };
            let mut all_rooms = self.list_rooms_snapshot();
            let mut rooms = BTreeMap::new();
            if let Some(room_entries) = all_rooms.remove(&session.room) {
                rooms.insert(session.room.clone(), room_entries);
            }
            return rooms;
        }
        let mut rooms = self.list_rooms_snapshot();
        if client_is_gui_user(
            self.sessions
                .get(client_id)
                .and_then(|session| session.features.as_ref()),
        ) {
            self.add_empty_room_dummy_entries(&mut rooms);
        }
        rooms
    }

    fn list_rooms_snapshot(&self) -> BTreeMap<String, BTreeMap<String, ListUserEntry>> {
        let mut rooms = BTreeMap::new();
        for session in self.sessions.values() {
            let ready = self.user_ready(&session.username, &session.room);
            let mut entry = ListUserEntry::new()
                .with_position(0.0)
                .with_file(json!({}))
                .with_controller(self.user_is_room_controller(&session.username, &session.room));
            if let Some(ready) = ready {
                entry = entry.with_is_ready(ready);
            }
            if let Some(features) = &session.features {
                entry = entry.with_features(features.clone());
            }
            rooms
                .entry(session.room.clone())
                .or_insert_with(BTreeMap::new)
                .insert(session.username.clone(), entry);
        }
        rooms
    }

    fn add_empty_room_dummy_entries(
        &self,
        rooms: &mut BTreeMap<String, BTreeMap<String, ListUserEntry>>,
    ) {
        let mut known_rooms = BTreeSet::new();
        known_rooms.extend(self.room_controllers.keys().cloned());
        known_rooms.extend(self.room_playlists.keys().cloned());
        known_rooms.extend(self.room_playback_states.keys().cloned());

        let mut dummy_count = 0usize;
        for room_name in known_rooms {
            if !self.clients_in_room(&room_name).is_empty() {
                continue;
            }
            dummy_count = dummy_count.saturating_add(1);
            rooms
                .entry(room_name)
                .or_default()
                .insert(" ".repeat(dummy_count), legacy_dummy_list_entry());
        }
    }

    fn enqueue_list_snapshots_for_clients(
        &self,
        outbound_messages: &mut Vec<DirectedProtocolMessage>,
        recipients: Vec<String>,
    ) {
        for client_id in recipients {
            let rooms = self.list_rooms_snapshot_for_client(&client_id);
            outbound_messages.push(DirectedProtocolMessage::new(
                client_id,
                ProtocolMessage::list(ListPayload::rooms(rooms)),
            ));
        }
    }

    fn find_free_username(&self, username: &str, excluded_client_id: Option<&str>) -> String {
        let mut chosen_username = username.to_owned();
        let all_names: BTreeSet<String> = self
            .sessions
            .iter()
            .filter(|(client_id, _)| {
                excluded_client_id.is_none_or(|excluded| *client_id != excluded)
            })
            .map(|(_, session)| session.username.to_ascii_lowercase())
            .collect();

        if all_names.contains(&chosen_username.to_ascii_lowercase())
            && chosen_username.ends_with('_')
        {
            chosen_username = chosen_username.trim_end_matches('_').to_owned();
            if chosen_username.is_empty() {
                chosen_username = "_".to_owned();
            }
        }

        while all_names.contains(&chosen_username.to_ascii_lowercase()) {
            chosen_username.push('_');
        }
        chosen_username
    }
}

fn user_joined_message_with_metadata(
    username: &str,
    room_name: &str,
    version: &str,
    features: Option<Value>,
) -> ProtocolMessage {
    let event = json!({
        "joined": true,
        "version": version,
        "features": features.unwrap_or(Value::Null),
    });
    user_event_message(username, room_name, event)
}

fn user_room_update_message(username: &str, room_name: &str) -> ProtocolMessage {
    user_setting_message(username, room_name, None)
}

fn user_event_message(username: &str, room_name: &str, event: Value) -> ProtocolMessage {
    user_setting_message(username, room_name, Some(event))
}

fn user_setting_message(username: &str, room_name: &str, event: Option<Value>) -> ProtocolMessage {
    let mut user_setting = UserSetPayload::new().with_room(RoomRef::new(room_name));
    if let Some(event) = event {
        user_setting = user_setting.with_event(event);
    }
    let mut users = BTreeMap::new();
    users.insert(username.to_owned(), user_setting);
    ProtocolMessage::set(SetPayload::new().with_user(users))
}

fn ready_update_message(
    username: &str,
    is_ready: bool,
    manually_initiated: bool,
    set_by_username: Option<&str>,
) -> ProtocolMessage {
    let mut payload = ReadyPayload::new(is_ready)
        .with_manually_initiated(manually_initiated)
        .with_username(username);
    if let Some(set_by) = set_by_username {
        payload = payload.with_set_by(set_by);
    }
    ProtocolMessage::set(SetPayload::new().with_ready(payload))
}

fn room_idle_state_message(latency_calculation_seconds: f64) -> ProtocolMessage {
    state_sync_message(
        0.0,
        true,
        false,
        StateSyncOptions {
            latency_calculation_seconds: Some(latency_calculation_seconds),
            ..StateSyncOptions::default()
        },
    )
}

#[derive(Debug, Clone, Default)]
struct StateSyncOptions<'a> {
    set_by: Option<&'a str>,
    server_ignoring_counter: Option<u32>,
    client_latency_calculation: Option<f64>,
    client_ignoring_counter: Option<u32>,
    server_rtt_seconds: f64,
    latency_calculation_seconds: Option<f64>,
}

fn state_sync_message(
    position: f64,
    paused: bool,
    do_seek: bool,
    options: StateSyncOptions<'_>,
) -> ProtocolMessage {
    let mut playstate = PlaystatePayload::new()
        .with_position(position)
        .with_paused(paused)
        .with_do_seek(do_seek);
    if let Some(set_by) = options.set_by {
        playstate = playstate.with_set_by(set_by);
    }

    let mut ping = PingPayload::new()
        .with_latency_calculation(
            options
                .latency_calculation_seconds
                .unwrap_or_else(current_unix_timestamp_seconds),
        )
        .with_server_rtt(options.server_rtt_seconds);
    if let Some(client_latency_calculation) = options.client_latency_calculation {
        ping = ping.with_client_latency_calculation(client_latency_calculation);
    }
    let mut state = StatePayload::new()
        .with_playstate(playstate)
        .with_ping(ping);
    if options.server_ignoring_counter.is_some() || options.client_ignoring_counter.is_some() {
        let mut ignoring = IgnoringOnTheFlyPayload::new();
        if let Some(server_ignoring_counter) = options.server_ignoring_counter {
            ignoring = ignoring.with_server(server_ignoring_counter);
        }
        if let Some(client_ignoring_counter) = options.client_ignoring_counter {
            ignoring = ignoring.with_client(client_ignoring_counter);
        }
        state = state.with_ignoring_on_the_fly(ignoring);
    }
    ProtocolMessage::state(state)
}

fn current_unix_timestamp_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn controller_auth_status_message(
    username: &str,
    room_name: &str,
    success: bool,
) -> ProtocolMessage {
    let auth_status = ControllerAuthPayload::new()
        .with_user(username)
        .with_room(room_name)
        .with_success(success);
    ProtocolMessage::set(SetPayload::new().with_controller_auth(auth_status))
}

fn new_controlled_room_message(room_name: &str, password: &str) -> ProtocolMessage {
    let payload = NewControlledRoomPayload::new()
        .with_room_name(room_name)
        .with_password(password);
    ProtocolMessage::set(SetPayload::new().with_new_controlled_room(payload))
}

fn playlist_snapshot_change_message(files: Vec<String>, set_by: Option<&str>) -> ProtocolMessage {
    let mut playlist_change = PlaylistChangePayload::new(files);
    if let Some(set_by) = set_by {
        playlist_change = playlist_change.with_user(set_by);
    }
    ProtocolMessage::set(SetPayload::new().with_playlist_change(playlist_change))
}

fn playlist_snapshot_index_message(index: i64, set_by: Option<&str>) -> ProtocolMessage {
    let mut playlist_index = PlaylistIndexPayload::new(index);
    if let Some(set_by) = set_by {
        playlist_index = playlist_index.with_user(set_by);
    }
    ProtocolMessage::set(SetPayload::new().with_playlist_index(playlist_index))
}

#[cfg(test)]
fn controlled_room_name_for(room_name: &str, password: &str) -> String {
    RoomPasswordProvider::default().controlled_room_name_for(room_name, password)
}

fn server_feature_list(
    persistent_rooms_enabled: bool,
    isolate_rooms: bool,
    chat_enabled: bool,
    readiness_enabled: bool,
    max_chat_message_length: usize,
    max_username_length: usize,
) -> Value {
    json!({
        "isolateRooms": isolate_rooms,
        "readiness": readiness_enabled,
        "managedRooms": true,
        "persistentRooms": persistent_rooms_enabled,
        "chat": chat_enabled,
        "maxChatMessageLength": max_chat_message_length,
        "maxUsernameLength": max_username_length,
        "featureList": true,
        "setOthersReadiness": true,
        "uiMode": "UNKNOWN",
    })
}

async fn dispatch_outbound_lines_to_clients(
    client_line_senders: &SharedClientLineSenders,
    outbound_lines: Vec<DirectedOutboundLine>,
) {
    for line in outbound_lines {
        let line_sender = {
            let senders = client_line_senders.lock().await;
            senders.get(&line.client_id).cloned()
        };
        if let Some(line_sender) = line_sender {
            let _ = line_sender.send(line.line);
        }
    }
}

fn dispatch_transport_actions_to_sink(
    transport_action_sink: Option<&UnboundedSender<DirectedTransportAction>>,
    transport_actions: &[DirectedTransportAction],
) {
    if let Some(transport_action_sink) = transport_action_sink {
        for action in transport_actions {
            let _ = transport_action_sink.send(action.clone());
        }
    }
}

async fn write_network_line_to_stream<S>(stream: &mut S, line: &str) -> io::Result<()>
where
    S: AsyncWrite + Unpin,
{
    stream.write_all(line.as_bytes()).await?;
    stream.write_all(b"\n").await?;
    stream.flush().await?;
    Ok(())
}

async fn read_network_line_from_stream<S>(stream: &mut S) -> io::Result<Option<String>>
where
    S: AsyncRead + Unpin,
{
    let mut bytes = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        let bytes_read = stream.read(&mut byte).await?;
        if bytes_read == 0 {
            if bytes.is_empty() {
                return Ok(None);
            }
            break;
        }
        if byte[0] == b'\n' {
            break;
        }
        bytes.push(byte[0]);
    }

    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }

    String::from_utf8(bytes).map(Some).map_err(|source| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("inbound protocol line is not valid utf-8: {source}"),
        )
    })
}

#[derive(Debug)]
enum ServerNetworkTransport {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
    Closed,
}

impl ServerNetworkTransport {
    fn is_tls(&self) -> bool {
        matches!(self, Self::Tls(_))
    }

    async fn read_line(&mut self) -> io::Result<Option<String>> {
        match self {
            Self::Plain(stream) => read_network_line_from_stream(stream).await,
            Self::Tls(stream) => read_network_line_from_stream(stream.as_mut()).await,
            Self::Closed => Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "transport is closed",
            )),
        }
    }

    async fn write_line(&mut self, line: &str) -> io::Result<()> {
        match self {
            Self::Plain(stream) => write_network_line_to_stream(stream, line).await,
            Self::Tls(stream) => write_network_line_to_stream(stream.as_mut(), line).await,
            Self::Closed => Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "transport is closed",
            )),
        }
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(stream) => stream.shutdown().await,
            Self::Tls(stream) => stream.shutdown().await,
            Self::Closed => Ok(()),
        }
    }

    async fn upgrade_to_tls(self, acceptor: TlsAcceptor) -> io::Result<Self> {
        match self {
            Self::Plain(stream) => {
                let tls_stream = acceptor.accept(stream).await?;
                Ok(Self::Tls(Box::new(tls_stream)))
            }
            Self::Tls(stream) => Ok(Self::Tls(stream)),
            Self::Closed => Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "transport is closed",
            )),
        }
    }
}

async fn route_outbound_lines_for_client_session(
    transport: &mut ServerNetworkTransport,
    client_id: &str,
    client_line_senders: &SharedClientLineSenders,
    outbound_lines: Vec<DirectedOutboundLine>,
) -> io::Result<()> {
    let mut peer_outbound_lines = Vec::new();
    for line in outbound_lines {
        if line.client_id == client_id {
            transport.write_line(&line.line).await?;
        } else {
            peer_outbound_lines.push(line);
        }
    }
    dispatch_outbound_lines_to_clients(client_line_senders, peer_outbound_lines).await;
    Ok(())
}

async fn tls_acceptor_from_runtime(runtime: &Arc<Mutex<ServerRuntime>>) -> io::Result<TlsAcceptor> {
    let tls_server_config = {
        let runtime_guard = runtime.lock().await;
        runtime_guard.tls_server_config()
    };
    let Some(tls_server_config) = tls_server_config else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "tls server config is not available",
        ));
    };
    Ok(TlsAcceptor::from(tls_server_config))
}

async fn apply_local_transport_actions(
    transport: &mut ServerNetworkTransport,
    client_id: &str,
    runtime: &Arc<Mutex<ServerRuntime>>,
    transport_actions: &[DirectedTransportAction],
) -> io::Result<()> {
    let should_start_tls = transport_actions.iter().any(|action| {
        action.client_id == client_id && action.action == ServerTransportAction::StartTls
    });
    if !should_start_tls || transport.is_tls() {
        return Ok(());
    }
    let tls_acceptor = tls_acceptor_from_runtime(runtime).await?;
    let current_transport = std::mem::replace(transport, ServerNetworkTransport::Closed);
    *transport = current_transport.upgrade_to_tls(tls_acceptor).await?;
    Ok(())
}

async fn run_server_network_client_session(
    stream: TcpStream,
    client_id: String,
    runtime: Arc<Mutex<ServerRuntime>>,
    client_line_senders: SharedClientLineSenders,
    transport_action_sink: Option<UnboundedSender<DirectedTransportAction>>,
) -> Result<(), ServerNetworkError> {
    let (line_tx, mut line_rx): (UnboundedSender<String>, UnboundedReceiver<String>) =
        unbounded_channel();
    {
        let mut senders = client_line_senders.lock().await;
        senders.insert(client_id.clone(), line_tx);
    }

    let mut transport = ServerNetworkTransport::Plain(stream);
    let mut session_error: Option<ServerNetworkError> = None;
    loop {
        tokio::select! {
            inbound_line_result = transport.read_line() => {
                let inbound_line = match inbound_line_result {
                    Ok(Some(line)) => line,
                    Ok(None) => break,
                    Err(source) => {
                        session_error = Some(ServerNetworkError::Io(source));
                        break;
                    }
                };
                if inbound_line.is_empty() {
                    continue;
                }
                let dispatch = {
                    let mut runtime_guard = runtime.lock().await;
                    runtime_guard.handle_line_fanout_with_transport_actions(&client_id, &inbound_line)
                };
                let dispatch = match dispatch {
                    Ok(dispatch) => dispatch,
                    Err(source) => {
                        session_error = Some(ServerNetworkError::Runtime(source));
                        break;
                    }
                };
                if let Err(source) = route_outbound_lines_for_client_session(
                    &mut transport,
                    &client_id,
                    &client_line_senders,
                    dispatch.outbound_lines,
                )
                .await
                {
                    session_error = Some(ServerNetworkError::Io(source));
                    break;
                }
                dispatch_transport_actions_to_sink(
                    transport_action_sink.as_ref(),
                    &dispatch.transport_actions,
                );
                if let Err(source) = apply_local_transport_actions(
                    &mut transport,
                    &client_id,
                    &runtime,
                    &dispatch.transport_actions,
                )
                .await
                {
                    session_error = Some(ServerNetworkError::Io(source));
                    break;
                }
            }
            outbound_line = line_rx.recv() => {
                let Some(outbound_line) = outbound_line else {
                    break;
                };
                if let Err(source) = transport.write_line(&outbound_line).await {
                    session_error = Some(ServerNetworkError::Io(source));
                    break;
                }
            }
        }
    }

    {
        let mut senders = client_line_senders.lock().await;
        senders.remove(&client_id);
    }
    if let Err(source) = transport.shutdown().await
        && session_error.is_none()
    {
        session_error = Some(ServerNetworkError::Io(source));
    }

    let disconnect_fanout = {
        let mut runtime_guard = runtime.lock().await;
        runtime_guard.handle_transport_disconnect_fanout(&client_id)
    };
    match disconnect_fanout {
        Ok(outbound_lines) => {
            dispatch_outbound_lines_to_clients(&client_line_senders, outbound_lines).await;
        }
        Err(source) => {
            if session_error.is_none() {
                session_error = Some(ServerNetworkError::Runtime(source));
            }
        }
    }

    if let Some(session_error) = session_error {
        return Err(session_error);
    }
    Ok(())
}

pub async fn run_server_network_loop_until_shutdown(
    listener: TcpListener,
    runtime: Arc<Mutex<ServerRuntime>>,
    transport_action_sink: Option<UnboundedSender<DirectedTransportAction>>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<(), ServerNetworkError> {
    let client_line_senders: SharedClientLineSenders = Arc::new(Mutex::new(BTreeMap::new()));
    let mut session_tasks: Vec<JoinHandle<()>> = Vec::new();
    let mut next_client_number: u64 = 1;
    let mut tick = time::interval(std::time::Duration::from_secs_f64(
        SERVER_NETWORK_TICK_INTERVAL_SECONDS,
    ));

    loop {
        tokio::select! {
            _ = tick.tick() => {
                let outbound_lines = {
                    let mut runtime_guard = runtime.lock().await;
                    runtime_guard.advance_time_and_collect_fanout(SERVER_NETWORK_TICK_INTERVAL_SECONDS)?
                };
                dispatch_outbound_lines_to_clients(&client_line_senders, outbound_lines).await;
            }
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    break;
                }
            }
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let client_id = format!("client-{next_client_number}");
                next_client_number = next_client_number.saturating_add(1);
                let runtime = runtime.clone();
                let client_line_senders = client_line_senders.clone();
                let transport_action_sink = transport_action_sink.clone();
                session_tasks.push(tokio::spawn(async move {
                    let _ = run_server_network_client_session(
                        stream,
                        client_id,
                        runtime,
                        client_line_senders,
                        transport_action_sink,
                    )
                    .await;
                }));
            }
        }
    }

    for task in session_tasks {
        task.abort();
    }

    Ok(())
}

#[derive(Debug, Default)]
pub struct ServerApp {
    runtime: ServerRuntime,
}

impl ServerApp {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_motd_template(template: impl Into<String>) -> Self {
        Self {
            runtime: ServerRuntime::with_motd_template(template),
        }
    }

    pub fn with_room_password_salt(salt: impl Into<String>) -> Self {
        Self {
            runtime: ServerRuntime::with_room_password_salt(salt),
        }
    }

    pub fn with_persistent_rooms_enabled(enabled: bool) -> Self {
        Self {
            runtime: ServerRuntime::with_persistent_rooms_enabled(enabled),
        }
    }

    pub fn with_stats_db_path(db_path: impl Into<PathBuf>) -> Result<Self, ServerRuntimeError> {
        Ok(Self {
            runtime: ServerRuntime::with_stats_db_path(db_path)?,
        })
    }

    pub fn with_tls_cert_path(path: impl Into<PathBuf>) -> Self {
        Self {
            runtime: ServerRuntime::with_tls_cert_path(path),
        }
    }

    pub fn with_persistent_rooms_db_path(
        db_path: impl Into<PathBuf>,
    ) -> Result<Self, ServerRuntimeError> {
        Ok(Self {
            runtime: ServerRuntime::with_persistent_rooms_db_path(db_path)?,
        })
    }

    pub fn with_permanent_rooms_file_path(
        permanent_rooms_file_path: impl Into<PathBuf>,
    ) -> Result<Self, ServerRuntimeError> {
        Ok(Self {
            runtime: ServerRuntime::with_permanent_rooms_file_path(permanent_rooms_file_path)?,
        })
    }

    pub fn runtime_mut(&mut self) -> &mut ServerRuntime {
        &mut self.runtime
    }

    pub fn bootstrap_room(&mut self, room_name: &str) {
        self.runtime.bootstrap_room(room_name);
    }

    pub fn room_is_present(&self, room_name: &str) -> bool {
        self.runtime.room_is_present(room_name)
    }
}

#[cfg(test)]
mod tests {
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
        DirectedOutboundLine, DirectedTransportAction, LEGACY_SERVER_PASSWORD_REQUIRED_ERROR,
        LEGACY_SERVER_WRONG_PASSWORD_ERROR, RoomPasswordCheckError, RoomPasswordProvider,
        ServerApp, ServerRuntime, ServerRuntimeError, ServerTransportAction,
        run_server_network_loop_until_shutdown,
    };
    use syncplay_protocol::{
        ChatPayload, ListPayload, ProtocolMessage, decode_message_line, extract_hello_from_message,
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

    fn has_start_tls_transport_action(
        actions: &[DirectedTransportAction],
        recipient: &str,
    ) -> bool {
        actions.iter().any(|action| {
            action.client_id == recipient && action.action == ServerTransportAction::StartTls
        })
    }

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
    fn room_password_provider_uses_legacy_regex_matching_behavior() {
        let provider = RoomPasswordProvider::default();
        assert!(provider.is_valid_room_password("AB-123-4567"));
        assert!(!provider.is_valid_room_password("ab-123-456"));
        assert_eq!(
            provider.check("+room1:CB39A19549E8", "AB-123-4567"),
            Ok(false)
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
        assert!(super::default_motd_for_client_version("syncplay-rs-dev").is_empty());
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
    fn hello_line_registers_session_and_returns_server_hello() {
        let mut runtime = ServerRuntime::default();
        let outbound_lines = runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255","realversion":"1.7.5"}}"#,
            )
            .expect("hello line should be accepted");

        assert_eq!(outbound_lines.len(), 2);
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
        assert_eq!(
            load_stats_snapshot_rows(&db_path),
            vec![(1, "1.6.0".to_owned()), (1, "1.7.0".to_owned())]
        );

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
        assert_eq!(
            load_stats_snapshot_rows(&db_path),
            vec![(1, "2.0.0".to_owned())]
        );

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

    #[tokio::test]
    async fn server_network_loop_routes_hello_response_to_connected_client() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let address = listener
            .local_addr()
            .expect("listener should have local address");
        let runtime = Arc::new(Mutex::new(ServerRuntime::new()));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let server_task = tokio::spawn(run_server_network_loop_until_shutdown(
            listener,
            runtime,
            None,
            shutdown_rx,
        ));

        let stream = TcpStream::connect(address)
            .await
            .expect("client should connect");
        let (reader, mut writer) = stream.into_split();
        writer
            .write_all(
                br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.0"}}"#,
            )
            .await
            .expect("hello line should write");
        writer
            .write_all(b"\n")
            .await
            .expect("hello newline should write");
        writer.flush().await.expect("hello write should flush");

        let mut buffered_reader = BufReader::new(reader);
        let mut saw_hello = false;
        for _ in 0..4 {
            let mut line = String::new();
            let read_bytes = timeout(Duration::from_secs(2), buffered_reader.read_line(&mut line))
                .await
                .expect("server response should arrive before timeout")
                .expect("server response read should succeed");
            if read_bytes == 0 {
                break;
            }
            let message =
                decode_message_line(line.trim_end()).expect("response line should decode");
            if matches!(message, ProtocolMessage::Hello(_)) {
                saw_hello = true;
                break;
            }
        }
        assert!(
            saw_hello,
            "network loop should route runtime hello response to connected client"
        );

        shutdown_tx
            .send(true)
            .expect("shutdown signal should send successfully");
        server_task
            .await
            .expect("server task should join cleanly")
            .expect("server loop should exit without error");
    }

    #[tokio::test]
    async fn server_network_loop_forwards_tls_start_transport_action_to_sink() {
        let cert_path = temporary_directory_path("tls-network-loop");
        let _ = fs::remove_dir_all(&cert_path);
        fs::create_dir_all(&cert_path).expect("tls cert temp directory should be creatable");
        write_valid_tls_bundle(&cert_path);

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let address = listener
            .local_addr()
            .expect("listener should have local address");
        let runtime = Arc::new(Mutex::new(ServerRuntime::new()));
        {
            let mut runtime_guard = runtime.lock().await;
            runtime_guard.set_tls_cert_path(Some(cert_path.clone()));
        }
        let (transport_action_tx, mut transport_action_rx) = mpsc::unbounded_channel();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let server_task = tokio::spawn(run_server_network_loop_until_shutdown(
            listener,
            runtime,
            Some(transport_action_tx),
            shutdown_rx,
        ));

        let stream = TcpStream::connect(address)
            .await
            .expect("client should connect");
        let (reader, mut writer) = stream.into_split();
        writer
            .write_all(br#"{"TLS":{"startTLS":"send"}}"#)
            .await
            .expect("tls request line should write");
        writer
            .write_all(b"\n")
            .await
            .expect("tls request newline should write");
        writer.flush().await.expect("tls request should flush");

        let mut buffered_reader = BufReader::new(reader);
        let mut response_line = String::new();
        timeout(
            Duration::from_secs(2),
            buffered_reader.read_line(&mut response_line),
        )
        .await
        .expect("tls response should arrive before timeout")
        .expect("tls response read should succeed");
        let tls_response =
            decode_message_line(response_line.trim_end()).expect("tls response should decode");
        let ProtocolMessage::Tls(payload) = tls_response else {
            panic!("server should respond with TLS payload");
        };
        assert_eq!(payload.tls.start_tls, "true");

        let action = timeout(Duration::from_secs(2), transport_action_rx.recv())
            .await
            .expect("transport action should arrive before timeout")
            .expect("transport action channel should deliver StartTls");
        assert_eq!(action.client_id, "client-1");
        assert_eq!(action.action, ServerTransportAction::StartTls);

        shutdown_tx
            .send(true)
            .expect("shutdown signal should send successfully");
        server_task
            .await
            .expect("server task should join cleanly")
            .expect("server loop should exit without error");

        fs::remove_dir_all(&cert_path).expect("tls cert temp directory should be removable");
    }

    #[tokio::test]
    async fn server_network_loop_tls_upgrade_preserves_post_upgrade_protocol_flow() {
        let cert_path = temporary_directory_path("tls-network-upgrade-flow");
        let _ = fs::remove_dir_all(&cert_path);
        fs::create_dir_all(&cert_path).expect("tls cert temp directory should be creatable");
        write_valid_tls_bundle(&cert_path);

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let address = listener
            .local_addr()
            .expect("listener should have local address");
        let runtime = Arc::new(Mutex::new(ServerRuntime::new()));
        {
            let mut runtime_guard = runtime.lock().await;
            runtime_guard.set_tls_cert_path(Some(cert_path.clone()));
        }
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let server_task = tokio::spawn(run_server_network_loop_until_shutdown(
            listener,
            runtime,
            None,
            shutdown_rx,
        ));

        let mut stream = TcpStream::connect(address)
            .await
            .expect("client should connect");
        stream
            .write_all(br#"{"TLS":{"startTLS":"send"}}"#)
            .await
            .expect("tls request line should write");
        stream
            .write_all(b"\n")
            .await
            .expect("tls request newline should write");
        stream.flush().await.expect("tls request should flush");

        let tls_response_line = timeout(
            Duration::from_secs(2),
            super::read_network_line_from_stream(&mut stream),
        )
        .await
        .expect("tls response should arrive before timeout")
        .expect("tls response read should succeed")
        .expect("tls response line should be present");
        let tls_response =
            decode_message_line(&tls_response_line).expect("tls response line should decode");
        let ProtocolMessage::Tls(payload) = tls_response else {
            panic!("server should respond with TLS payload");
        };
        assert_eq!(payload.tls.start_tls, "true");

        let connector = tls_client_connector_for_test_fixture();
        let server_name = ServerName::try_from("localhost").expect("server name should parse");
        let mut tls_stream = timeout(
            Duration::from_secs(2),
            connector.connect(server_name, stream),
        )
        .await
        .expect("tls handshake should complete before timeout")
        .expect("tls handshake should succeed");

        tls_stream
            .write_all(
                br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.0"}}"#,
            )
            .await
            .expect("hello line should write over tls");
        tls_stream
            .write_all(b"\n")
            .await
            .expect("hello newline should write over tls");
        tls_stream
            .flush()
            .await
            .expect("hello line should flush over tls");

        let mut saw_hello = false;
        for _ in 0..4 {
            let maybe_line = timeout(
                Duration::from_secs(2),
                super::read_network_line_from_stream(&mut tls_stream),
            )
            .await
            .expect("post-upgrade response should arrive before timeout")
            .expect("post-upgrade response read should succeed");
            let Some(line) = maybe_line else {
                break;
            };
            if line.is_empty() {
                continue;
            }
            let message = decode_message_line(&line).expect("post-upgrade line should decode");
            if matches!(message, ProtocolMessage::Hello(_)) {
                saw_hello = true;
                break;
            }
        }
        assert!(
            saw_hello,
            "server should continue protocol flow over upgraded TLS transport"
        );

        shutdown_tx
            .send(true)
            .expect("shutdown signal should send successfully");
        server_task
            .await
            .expect("server task should join cleanly")
            .expect("server loop should exit without error");

        fs::remove_dir_all(&cert_path).expect("tls cert temp directory should be removable");
    }

    #[tokio::test]
    async fn server_network_loop_tls_upgrade_uses_cached_context_when_files_disappear() {
        let cert_path = temporary_directory_path("tls-network-upgrade-cached-context");
        let _ = fs::remove_dir_all(&cert_path);
        fs::create_dir_all(&cert_path).expect("tls cert temp directory should be creatable");
        write_valid_tls_bundle(&cert_path);

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let address = listener
            .local_addr()
            .expect("listener should have local address");
        let runtime = Arc::new(Mutex::new(ServerRuntime::new()));
        {
            let mut runtime_guard = runtime.lock().await;
            runtime_guard.set_tls_cert_path(Some(cert_path.clone()));
        }

        fs::remove_file(cert_path.join("privkey.pem")).expect("privkey file should be removable");
        fs::remove_file(cert_path.join("chain.pem")).expect("chain file should be removable");
        fs::remove_file(cert_path.join("cert.pem")).expect("cert file should be removable");

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let server_task = tokio::spawn(run_server_network_loop_until_shutdown(
            listener,
            runtime,
            None,
            shutdown_rx,
        ));

        let mut stream = TcpStream::connect(address)
            .await
            .expect("client should connect");
        stream
            .write_all(br#"{"TLS":{"startTLS":"send"}}"#)
            .await
            .expect("tls request line should write");
        stream
            .write_all(b"\n")
            .await
            .expect("tls request newline should write");
        stream.flush().await.expect("tls request should flush");

        let tls_response_line = timeout(
            Duration::from_secs(2),
            super::read_network_line_from_stream(&mut stream),
        )
        .await
        .expect("tls response should arrive before timeout")
        .expect("tls response read should succeed")
        .expect("tls response line should be present");
        let tls_response =
            decode_message_line(&tls_response_line).expect("tls response line should decode");
        let ProtocolMessage::Tls(payload) = tls_response else {
            panic!("server should respond with TLS payload");
        };
        assert_eq!(
            payload.tls.start_tls, "true",
            "server should still accept TLS using cached loaded context"
        );

        let connector = tls_client_connector_for_test_fixture();
        let server_name = ServerName::try_from("localhost").expect("server name should parse");
        let mut tls_stream = timeout(
            Duration::from_secs(2),
            connector.connect(server_name, stream),
        )
        .await
        .expect("tls handshake should complete before timeout")
        .expect("tls handshake should succeed with cached context");

        tls_stream
            .write_all(
                br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.0"}}"#,
            )
            .await
            .expect("hello line should write over tls");
        tls_stream
            .write_all(b"\n")
            .await
            .expect("hello newline should write over tls");
        tls_stream
            .flush()
            .await
            .expect("hello line should flush over tls");

        let mut saw_hello = false;
        for _ in 0..4 {
            let maybe_line = timeout(
                Duration::from_secs(2),
                super::read_network_line_from_stream(&mut tls_stream),
            )
            .await
            .expect("post-upgrade response should arrive before timeout")
            .expect("post-upgrade response read should succeed");
            let Some(line) = maybe_line else {
                break;
            };
            if line.is_empty() {
                continue;
            }
            let message = decode_message_line(&line).expect("post-upgrade line should decode");
            if matches!(message, ProtocolMessage::Hello(_)) {
                saw_hello = true;
                break;
            }
        }
        assert!(
            saw_hello,
            "server should continue protocol flow over cached-context upgraded tls transport"
        );

        shutdown_tx
            .send(true)
            .expect("shutdown signal should send successfully");
        server_task
            .await
            .expect("server task should join cleanly")
            .expect("server loop should exit without error");

        fs::remove_dir_all(&cert_path).expect("tls cert temp directory should be removable");
    }

    #[tokio::test]
    async fn server_network_loop_tls_upgrade_keeps_inflight_handshake_when_bundle_rotates_invalid_after_starttls_true()
     {
        let cert_path = temporary_directory_path("tls-network-upgrade-rotation-window");
        let _ = fs::remove_dir_all(&cert_path);
        fs::create_dir_all(&cert_path).expect("tls cert temp directory should be creatable");
        write_valid_tls_bundle(&cert_path);

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let address = listener
            .local_addr()
            .expect("listener should have local address");
        let runtime = Arc::new(Mutex::new(ServerRuntime::new()));
        {
            let mut runtime_guard = runtime.lock().await;
            runtime_guard.set_tls_cert_path(Some(cert_path.clone()));
        }

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let server_task = tokio::spawn(run_server_network_loop_until_shutdown(
            listener,
            runtime,
            None,
            shutdown_rx,
        ));

        let mut first_stream = TcpStream::connect(address)
            .await
            .expect("first client should connect");
        first_stream
            .write_all(br#"{"TLS":{"startTLS":"send"}}"#)
            .await
            .expect("first tls request line should write");
        first_stream
            .write_all(b"\n")
            .await
            .expect("first tls request newline should write");
        first_stream
            .flush()
            .await
            .expect("first tls request should flush");

        let first_tls_response_line = timeout(
            Duration::from_secs(2),
            super::read_network_line_from_stream(&mut first_stream),
        )
        .await
        .expect("first tls response should arrive before timeout")
        .expect("first tls response read should succeed")
        .expect("first tls response line should be present");
        let first_tls_response = decode_message_line(&first_tls_response_line)
            .expect("first tls response line should decode");
        let ProtocolMessage::Tls(payload) = first_tls_response else {
            panic!("server should respond with TLS payload for first client");
        };
        assert_eq!(payload.tls.start_tls, "true");

        fs::remove_file(cert_path.join("chain.pem")).expect("chain file should be removable");
        overwrite_file_until_modified_time_changes(
            &cert_path.join("cert.pem"),
            "rotated-after-starttls-true",
        );

        let connector = tls_client_connector_for_test_fixture();
        let server_name = ServerName::try_from("localhost").expect("server name should parse");
        let mut first_tls_stream = timeout(
            Duration::from_secs(2),
            connector.connect(server_name, first_stream),
        )
        .await
        .expect("first tls handshake should complete before timeout")
        .expect("first tls handshake should succeed with cached context");

        first_tls_stream
            .write_all(
                br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.0"}}"#,
            )
            .await
            .expect("hello line should write over first tls stream");
        first_tls_stream
            .write_all(b"\n")
            .await
            .expect("hello newline should write over first tls stream");
        first_tls_stream
            .flush()
            .await
            .expect("hello line should flush over first tls stream");

        let mut saw_hello = false;
        for _ in 0..4 {
            let maybe_line = timeout(
                Duration::from_secs(2),
                super::read_network_line_from_stream(&mut first_tls_stream),
            )
            .await
            .expect("first post-upgrade response should arrive before timeout")
            .expect("first post-upgrade response read should succeed");
            let Some(line) = maybe_line else {
                break;
            };
            if line.is_empty() {
                continue;
            }
            let message =
                decode_message_line(&line).expect("first post-upgrade line should decode");
            if matches!(message, ProtocolMessage::Hello(_)) {
                saw_hello = true;
                break;
            }
        }
        assert!(
            saw_hello,
            "first client should complete protocol flow over cached-context upgraded tls transport"
        );

        let mut second_stream = TcpStream::connect(address)
            .await
            .expect("second client should connect");
        second_stream
            .write_all(br#"{"TLS":{"startTLS":"send"}}"#)
            .await
            .expect("second tls request line should write");
        second_stream
            .write_all(b"\n")
            .await
            .expect("second tls request newline should write");
        second_stream
            .flush()
            .await
            .expect("second tls request should flush");

        let second_tls_response_line = timeout(
            Duration::from_secs(2),
            super::read_network_line_from_stream(&mut second_stream),
        )
        .await
        .expect("second tls response should arrive before timeout")
        .expect("second tls response read should succeed")
        .expect("second tls response line should be present");
        let second_tls_response = decode_message_line(&second_tls_response_line)
            .expect("second tls response line should decode");
        let ProtocolMessage::Tls(payload) = second_tls_response else {
            panic!("server should respond with TLS payload for second client");
        };
        assert_eq!(
            payload.tls.start_tls, "false",
            "second client should be denied TLS after cert rotation makes bundle invalid"
        );

        shutdown_tx
            .send(true)
            .expect("shutdown signal should send successfully");
        server_task
            .await
            .expect("server task should join cleanly")
            .expect("server loop should exit without error");

        fs::remove_dir_all(&cert_path).expect("tls cert temp directory should be removable");
    }

    #[tokio::test]
    async fn server_network_loop_tls_upgrade_recovers_after_invalid_rotation_bundle_is_restored() {
        let cert_path = temporary_directory_path("tls-network-upgrade-rotation-recovery");
        let _ = fs::remove_dir_all(&cert_path);
        fs::create_dir_all(&cert_path).expect("tls cert temp directory should be creatable");
        write_valid_tls_bundle(&cert_path);

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let address = listener
            .local_addr()
            .expect("listener should have local address");
        let runtime = Arc::new(Mutex::new(ServerRuntime::new()));
        {
            let mut runtime_guard = runtime.lock().await;
            runtime_guard.set_tls_cert_path(Some(cert_path.clone()));
        }

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let server_task = tokio::spawn(run_server_network_loop_until_shutdown(
            listener,
            runtime,
            None,
            shutdown_rx,
        ));

        let mut first_stream = TcpStream::connect(address)
            .await
            .expect("first client should connect");
        first_stream
            .write_all(br#"{"TLS":{"startTLS":"send"}}"#)
            .await
            .expect("first tls request line should write");
        first_stream
            .write_all(b"\n")
            .await
            .expect("first tls request newline should write");
        first_stream
            .flush()
            .await
            .expect("first tls request should flush");
        let first_tls_response_line = timeout(
            Duration::from_secs(2),
            super::read_network_line_from_stream(&mut first_stream),
        )
        .await
        .expect("first tls response should arrive before timeout")
        .expect("first tls response read should succeed")
        .expect("first tls response line should be present");
        let first_tls_response = decode_message_line(&first_tls_response_line)
            .expect("first tls response line should decode");
        let ProtocolMessage::Tls(first_tls_payload) = first_tls_response else {
            panic!("server should respond with TLS payload for first client");
        };
        assert_eq!(first_tls_payload.tls.start_tls, "true");

        fs::remove_file(cert_path.join("chain.pem")).expect("chain file should be removable");
        overwrite_file_until_modified_time_changes(
            &cert_path.join("cert.pem"),
            "rotated-invalid-before-second-client",
        );

        let mut second_stream = TcpStream::connect(address)
            .await
            .expect("second client should connect");
        second_stream
            .write_all(br#"{"TLS":{"startTLS":"send"}}"#)
            .await
            .expect("second tls request line should write");
        second_stream
            .write_all(b"\n")
            .await
            .expect("second tls request newline should write");
        second_stream
            .flush()
            .await
            .expect("second tls request should flush");
        let second_tls_response_line = timeout(
            Duration::from_secs(2),
            super::read_network_line_from_stream(&mut second_stream),
        )
        .await
        .expect("second tls response should arrive before timeout")
        .expect("second tls response read should succeed")
        .expect("second tls response line should be present");
        let second_tls_response = decode_message_line(&second_tls_response_line)
            .expect("second tls response line should decode");
        let ProtocolMessage::Tls(second_tls_payload) = second_tls_response else {
            panic!("server should respond with TLS payload for second client");
        };
        assert_eq!(
            second_tls_payload.tls.start_tls, "false",
            "second client should be denied TLS after invalid cert rotation"
        );

        write_valid_tls_bundle(&cert_path);
        rewrite_file_until_modified_time_changes(&cert_path.join("cert.pem"), TEST_TLS_CERT_PEM);

        let mut third_stream = TcpStream::connect(address)
            .await
            .expect("third client should connect");
        third_stream
            .write_all(br#"{"TLS":{"startTLS":"send"}}"#)
            .await
            .expect("third tls request line should write");
        third_stream
            .write_all(b"\n")
            .await
            .expect("third tls request newline should write");
        third_stream
            .flush()
            .await
            .expect("third tls request should flush");
        let third_tls_response_line = timeout(
            Duration::from_secs(2),
            super::read_network_line_from_stream(&mut third_stream),
        )
        .await
        .expect("third tls response should arrive before timeout")
        .expect("third tls response read should succeed")
        .expect("third tls response line should be present");
        let third_tls_response = decode_message_line(&third_tls_response_line)
            .expect("third tls response line should decode");
        let ProtocolMessage::Tls(third_tls_payload) = third_tls_response else {
            panic!("server should respond with TLS payload for third client");
        };
        assert_eq!(
            third_tls_payload.tls.start_tls, "true",
            "third client should be allowed TLS after valid bundle restoration"
        );

        let connector = tls_client_connector_for_test_fixture();
        let server_name = ServerName::try_from("localhost").expect("server name should parse");
        let mut third_tls_stream = timeout(
            Duration::from_secs(2),
            connector.connect(server_name, third_stream),
        )
        .await
        .expect("third tls handshake should complete before timeout")
        .expect("third tls handshake should succeed after bundle restoration");

        third_tls_stream
            .write_all(
                br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.0"}}"#,
            )
            .await
            .expect("hello line should write over third tls stream");
        third_tls_stream
            .write_all(b"\n")
            .await
            .expect("hello newline should write over third tls stream");
        third_tls_stream
            .flush()
            .await
            .expect("hello line should flush over third tls stream");

        let mut saw_hello = false;
        for _ in 0..4 {
            let maybe_line = timeout(
                Duration::from_secs(2),
                super::read_network_line_from_stream(&mut third_tls_stream),
            )
            .await
            .expect("third post-upgrade response should arrive before timeout")
            .expect("third post-upgrade response read should succeed");
            let Some(line) = maybe_line else {
                break;
            };
            if line.is_empty() {
                continue;
            }
            let message =
                decode_message_line(&line).expect("third post-upgrade line should decode");
            if matches!(message, ProtocolMessage::Hello(_)) {
                saw_hello = true;
                break;
            }
        }
        assert!(
            saw_hello,
            "third client should complete protocol flow over restored TLS context"
        );

        shutdown_tx
            .send(true)
            .expect("shutdown signal should send successfully");
        server_task
            .await
            .expect("server task should join cleanly")
            .expect("server loop should exit without error");

        fs::remove_dir_all(&cert_path).expect("tls cert temp directory should be removable");
    }

    #[test]
    fn persistent_room_retains_playlist_index_and_position_after_empty_transition() {
        let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
        runtime.set_time_now_override_seconds(Some(0.0));
        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"persistent-room"},"version":"9.9.9"}}"#,
            )
            .expect("initial hello should establish session");
        runtime
            .handle_line_fanout(
                "client-1",
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"]}}}"#,
            )
            .expect("playlist change should succeed");
        runtime
            .handle_line_fanout("client-1", r#"{"Set":{"playlistIndex":{"index":1}}}"#)
            .expect("playlist index change should succeed");
        runtime
            .handle_line_fanout(
                "client-1",
                r#"{"State":{"playstate":{"position":42.0,"paused":true,"doSeek":true}}}"#,
            )
            .expect("state update should succeed");
        runtime
            .handle_line_fanout("client-1", r#"{"Set":{"room":{"name":"lobby"}}}"#)
            .expect("room switch should succeed");

        let directed_lines = runtime
            .handle_line_fanout(
                "client-2",
                r#"{"Hello":{"username":"bob","room":{"name":"persistent-room"},"version":"9.9.9"}}"#,
            )
            .expect("rejoin to persistent room should succeed");
        let directed_messages = decode_directed_lines(&directed_lines);
        assert!(
            has_playlist_snapshot_with_user(
                &directed_messages,
                "client-2",
                &["episode1.mkv", "episode2.mkv"],
                "alice",
            ),
            "joining user should receive persisted playlist snapshot"
        );
        assert!(
            has_playlist_index_snapshot(&directed_messages, "client-2", 1),
            "joining user should receive persisted playlist index snapshot"
        );

        let periodic_lines = runtime
            .advance_time_and_collect_fanout(super::SERVER_STATE_INTERVAL_SECONDS)
            .expect("periodic tick should succeed");
        let periodic_messages = decode_directed_lines(&periodic_lines);
        let client_state = periodic_messages
            .iter()
            .find(|(recipient, message)| {
                recipient == "client-2" && matches!(message, ProtocolMessage::State(_))
            })
            .expect("rejoined client should receive periodic room state")
            .1
            .clone();
        let ProtocolMessage::State(state_payload) = client_state else {
            panic!("periodic room state should decode as state message");
        };
        let playstate = state_payload
            .state
            .playstate
            .expect("periodic room state should include playstate");
        assert_eq!(playstate.position, Some(42.0));
        assert_eq!(playstate.paused, Some(true));
    }

    #[test]
    fn temporary_room_does_not_retain_playlist_or_position_when_empty() {
        let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
        runtime.set_time_now_override_seconds(Some(0.0));
        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"session-temp"},"version":"9.9.9"}}"#,
            )
            .expect("initial hello should establish session");
        runtime
            .handle_line_fanout(
                "client-1",
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"]}}}"#,
            )
            .expect("playlist change should succeed");
        runtime
            .handle_line_fanout("client-1", r#"{"Set":{"playlistIndex":{"index":1}}}"#)
            .expect("playlist index change should succeed");
        runtime
            .handle_line_fanout(
                "client-1",
                r#"{"State":{"playstate":{"position":37.0,"paused":true,"doSeek":true}}}"#,
            )
            .expect("state update should succeed");
        runtime
            .handle_line_fanout("client-1", r#"{"Set":{"room":{"name":"lobby"}}}"#)
            .expect("room switch should succeed");

        let directed_lines = runtime
            .handle_line_fanout(
                "client-2",
                r#"{"Hello":{"username":"bob","room":{"name":"session-temp"},"version":"9.9.9"}}"#,
            )
            .expect("rejoin to temporary room should succeed");
        let directed_messages = decode_directed_lines(&directed_lines);
        assert!(
            has_playlist_snapshot(&directed_messages, "client-2", &[]),
            "temporary room should reset playlist state when emptied"
        );
        assert!(
            !has_playlist_index_snapshot(&directed_messages, "client-2", 1),
            "temporary room should not retain playlist index"
        );

        let periodic_lines = runtime
            .advance_time_and_collect_fanout(super::SERVER_STATE_INTERVAL_SECONDS)
            .expect("periodic tick should succeed");
        let periodic_messages = decode_directed_lines(&periodic_lines);
        let client_state = periodic_messages
            .iter()
            .find(|(recipient, message)| {
                recipient == "client-2" && matches!(message, ProtocolMessage::State(_))
            })
            .expect("rejoined client should receive periodic room state")
            .1
            .clone();
        let ProtocolMessage::State(state_payload) = client_state else {
            panic!("periodic room state should decode as state message");
        };
        let playstate = state_payload
            .state
            .playstate
            .expect("periodic room state should include playstate");
        assert_eq!(playstate.position, Some(0.0));
        assert_eq!(playstate.paused, Some(true));
    }

    #[test]
    fn persistent_room_sqlite_reload_restores_playlist_index_and_position() {
        let db_path = temporary_sqlite_path("persistent-rooms-reload");
        let _ = fs::remove_file(&db_path);
        {
            let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
            runtime
                .set_persistent_rooms_db_path(Some(db_path.clone()))
                .expect("runtime should initialize sqlite persistence");
            runtime.set_time_now_override_seconds(Some(0.0));
            runtime
                .handle_line(
                    "client-1",
                    r#"{"Hello":{"username":"alice","room":{"name":"persistent-room"},"version":"9.9.9"}}"#,
                )
                .expect("initial hello should establish session");
            runtime
                .handle_line_fanout(
                    "client-1",
                    r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"]}}}"#,
                )
                .expect("playlist change should succeed");
            runtime
                .handle_line_fanout("client-1", r#"{"Set":{"playlistIndex":{"index":1}}}"#)
                .expect("playlist index change should succeed");
            runtime
                .handle_line_fanout(
                    "client-1",
                    r#"{"State":{"playstate":{"position":24.0,"paused":true,"doSeek":true}}}"#,
                )
                .expect("state update should succeed");
            runtime
                .handle_line_fanout("client-1", r#"{"Set":{"room":{"name":"lobby"}}}"#)
                .expect("room switch should persist empty-room state");
        }

        let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
        runtime
            .set_persistent_rooms_db_path(Some(db_path.clone()))
            .expect("runtime should load sqlite persistence snapshot");
        runtime.set_time_now_override_seconds(Some(0.0));
        let directed_lines = runtime
            .handle_line_fanout(
                "client-2",
                r#"{"Hello":{"username":"bob","room":{"name":"persistent-room"},"version":"9.9.9"}}"#,
            )
            .expect("hello should restore persisted room snapshot");
        let directed_messages = decode_directed_lines(&directed_lines);
        assert!(
            has_playlist_snapshot(
                &directed_messages,
                "client-2",
                &["episode1.mkv", "episode2.mkv"]
            ),
            "sqlite-backed reload should restore playlist snapshot"
        );
        assert!(
            has_playlist_index_snapshot(&directed_messages, "client-2", 1),
            "sqlite-backed reload should restore playlist index"
        );

        let periodic_lines = runtime
            .advance_time_and_collect_fanout(super::SERVER_STATE_INTERVAL_SECONDS)
            .expect("periodic tick should succeed");
        let periodic_messages = decode_directed_lines(&periodic_lines);
        let client_state = periodic_messages
            .iter()
            .find(|(recipient, message)| {
                recipient == "client-2" && matches!(message, ProtocolMessage::State(_))
            })
            .expect("reloaded client should receive periodic room state")
            .1
            .clone();
        let ProtocolMessage::State(state_payload) = client_state else {
            panic!("periodic room state should decode as state message");
        };
        let playstate = state_payload
            .state
            .playstate
            .expect("periodic room state should include playstate");
        assert_eq!(playstate.position, Some(24.0));
        assert_eq!(playstate.paused, Some(true));

        fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
    }

    #[test]
    fn permanent_room_file_retains_empty_playlist_state_when_room_empties() {
        let db_path = temporary_sqlite_path("permanent-room-retention");
        let permanent_rooms_file = temporary_text_path("permanent-room-retention");
        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_file(&permanent_rooms_file);
        fs::write(&permanent_rooms_file, "permanent-room\n")
            .expect("permanent rooms file should be writable");

        let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
        runtime
            .set_persistent_rooms_db_path(Some(db_path.clone()))
            .expect("runtime should initialize sqlite persistence");
        runtime
            .set_permanent_rooms_file_path(Some(permanent_rooms_file.clone()))
            .expect("runtime should load permanent rooms file");
        runtime.set_time_now_override_seconds(Some(0.0));
        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"permanent-room"},"version":"9.9.9"}}"#,
            )
            .expect("alice hello should establish session");
        runtime
            .handle_line_fanout(
                "client-1",
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv"]}}}"#,
            )
            .expect("playlist change should succeed");
        runtime
            .handle_line_fanout("client-1", r#"{"Set":{"playlistIndex":{"index":0}}}"#)
            .expect("playlist index change should succeed");
        runtime
            .handle_line_fanout("client-1", r#"{"Set":{"playlistChange":{"files":[]}}}"#)
            .expect("playlist clear should succeed");
        runtime
            .handle_line_fanout("client-1", r#"{"Set":{"room":{"name":"lobby"}}}"#)
            .expect("room switch should succeed");

        let directed_lines = runtime
            .handle_line_fanout(
                "client-2",
                r#"{"Hello":{"username":"bob","room":{"name":"permanent-room"},"version":"9.9.9"}}"#,
            )
            .expect("bob hello should succeed");
        let directed_messages = decode_directed_lines(&directed_lines);
        assert!(
            has_playlist_snapshot(&directed_messages, "client-2", &[]),
            "permanent room should preserve empty playlist snapshot"
        );
        assert!(
            has_playlist_index_snapshot(&directed_messages, "client-2", 0),
            "permanent room should preserve playlist index even when playlist is empty"
        );

        fs::remove_file(&permanent_rooms_file).expect("temporary permanent rooms file cleanup");
        fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
    }

    #[test]
    fn gui_list_shows_dummy_entry_for_empty_permanent_room() {
        let db_path = temporary_sqlite_path("gui-dummy-room-list");
        let permanent_rooms_file = temporary_text_path("gui-dummy-room-list");
        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_file(&permanent_rooms_file);
        fs::write(&permanent_rooms_file, "permanent-room\n")
            .expect("permanent rooms file should be writable");

        let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
        runtime
            .set_persistent_rooms_db_path(Some(db_path.clone()))
            .expect("runtime should initialize sqlite persistence");
        runtime
            .set_permanent_rooms_file_path(Some(permanent_rooms_file.clone()))
            .expect("runtime should load permanent rooms file");
        runtime
            .handle_line(
                "gui-client",
                r#"{"Hello":{"username":"gui-user","room":{"name":"lobby"},"version":"9.9.9","features":{"uiMode":"GUI"}}}"#,
            )
            .expect("gui user hello should establish session");
        runtime
            .handle_line(
                "cli-client",
                r#"{"Hello":{"username":"cli-user","room":{"name":"lobby"},"version":"9.9.9","features":{"uiMode":"CLI"}}}"#,
            )
            .expect("cli user hello should establish session");

        let gui_list_lines = runtime
            .handle_line("gui-client", r#"{"List":null}"#)
            .expect("gui list request should succeed");
        assert_eq!(gui_list_lines.len(), 1);
        let gui_list_message =
            decode_message_line(&gui_list_lines[0]).expect("gui list output should decode");
        let gui_rooms = match gui_list_message {
            ProtocolMessage::List(payload) => match payload.list {
                ListPayload::Rooms(rooms) => rooms,
                other => panic!("expected gui room snapshot, got {other:?}"),
            },
            other => panic!(
                "expected list message for gui request, got {}",
                other.kind()
            ),
        };
        let permanent_room = gui_rooms
            .get("permanent-room")
            .expect("gui list should include empty permanent room");
        assert_eq!(permanent_room.len(), 1);
        let (dummy_username, dummy_entry) = permanent_room
            .iter()
            .next()
            .expect("dummy entry should be present");
        assert_eq!(dummy_username, " ");
        assert_eq!(dummy_entry.position, Some(0.0));
        assert_eq!(dummy_entry.file.as_ref(), Some(&json!({})));
        assert_eq!(dummy_entry.controller, Some(false));
        assert_eq!(dummy_entry.is_ready, Some(true));
        assert_eq!(dummy_entry.features.as_ref(), Some(&json!([])));

        let cli_list_lines = runtime
            .handle_line("cli-client", r#"{"List":null}"#)
            .expect("cli list request should succeed");
        assert_eq!(cli_list_lines.len(), 1);
        let cli_list_message =
            decode_message_line(&cli_list_lines[0]).expect("cli list output should decode");
        let cli_rooms = match cli_list_message {
            ProtocolMessage::List(payload) => match payload.list {
                ListPayload::Rooms(rooms) => rooms,
                other => panic!("expected cli room snapshot, got {other:?}"),
            },
            other => panic!(
                "expected list message for cli request, got {}",
                other.kind()
            ),
        };
        assert!(
            !cli_rooms.contains_key("permanent-room"),
            "cli list should not include dummy empty permanent room"
        );

        fs::remove_file(&permanent_rooms_file).expect("temporary permanent rooms file cleanup");
        fs::remove_file(&db_path).expect("temporary sqlite db should be removable");
    }

    #[test]
    fn persistent_list_updates_skip_clients_missing_ui_mode_feature() {
        let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"9.9.9","features":{"uiMode":"CLI"}}}"#,
            )
            .expect("client-1 hello should establish session");

        let directed_lines = runtime
            .handle_line_fanout(
                "client-2",
                r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"9.9.9"}}"#,
            )
            .expect("client-2 hello should establish session");
        let directed_messages = decode_directed_lines(&directed_lines);
        let list_recipients: BTreeSet<_> = directed_messages
            .iter()
            .filter_map(|(recipient, message)| {
                if matches!(message, ProtocolMessage::List(_)) {
                    Some(recipient.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert!(
            list_recipients.contains("client-1"),
            "persistent list updates should include clients that advertise uiMode"
        );
        assert!(
            !list_recipients.contains("client-2"),
            "persistent list updates should skip clients that omit uiMode"
        );
    }

    #[test]
    fn persistent_timeout_disconnect_emits_ui_mode_scoped_list_update() {
        let mut runtime = ServerRuntime::with_persistent_rooms_enabled(true);
        runtime.set_time_now_override_seconds(Some(0.0));
        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"9.9.9","features":{"uiMode":"CLI"}}}"#,
            )
            .expect("client-1 hello should establish session");
        runtime
            .handle_line(
                "client-2",
                r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"9.9.9","features":{"uiMode":"CLI"}}}"#,
            )
            .expect("client-2 hello should establish session");
        runtime
            .handle_line(
                "client-3",
                r#"{"Hello":{"username":"charlie","room":{"name":"room1"},"version":"9.9.9"}}"#,
            )
            .expect("client-3 hello should establish session");

        runtime
            .advance_time_and_collect_fanout(10.0)
            .expect("time advance should succeed");
        runtime
            .handle_line_fanout(
                "client-1",
                r#"{"State":{"ping":{"latencyCalculation":10.0}}}"#,
            )
            .expect("client-1 heartbeat state should succeed");
        runtime
            .handle_line_fanout(
                "client-3",
                r#"{"State":{"ping":{"latencyCalculation":10.0}}}"#,
            )
            .expect("client-3 heartbeat state should succeed");

        let timeout_lines = runtime
            .advance_time_and_collect_fanout(3.0)
            .expect("timeout-producing time advance should succeed");
        let timeout_messages = decode_directed_lines(&timeout_lines);
        let list_recipients: BTreeSet<_> = timeout_messages
            .iter()
            .filter_map(|(recipient, message)| {
                if matches!(message, ProtocolMessage::List(_)) {
                    Some(recipient.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert!(
            list_recipients.contains("client-1"),
            "timeout list update should target connected clients that advertise uiMode"
        );
        assert!(
            !list_recipients.contains("client-3"),
            "timeout list update should skip connected clients that omit uiMode"
        );
    }

    #[test]
    fn list_request_returns_room_snapshot_for_session() {
        let mut runtime = ServerRuntime::default();
        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should establish session");

        let outbound_lines = runtime
            .handle_line("client-1", r#"{"List":null}"#)
            .expect("list request should succeed");
        assert_eq!(outbound_lines.len(), 1);

        let response =
            decode_message_line(&outbound_lines[0]).expect("list response should decode");
        match response {
            ProtocolMessage::List(payload) => match payload.list {
                ListPayload::Rooms(rooms) => {
                    let room = rooms.get("room1").expect("room1 should be present");
                    let alice = room.get("alice").expect("alice should be listed");
                    assert_eq!(alice.is_ready, Some(false));
                }
                other => panic!("expected list room snapshot, got {other:?}"),
            },
            other => panic!("expected List message, got {}", other.kind()),
        }
    }

    #[test]
    fn set_room_moves_session_between_rooms() {
        let mut runtime = ServerRuntime::default();
        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should establish session");

        let outbound_lines = runtime
            .handle_line("client-1", r#"{"Set":{"room":{"name":"room2"}}}"#)
            .expect("set room should succeed");
        assert_eq!(outbound_lines.len(), 4);
        assert!(!runtime.room_is_present("room1"));
        assert!(runtime.room_is_present("room2"));
        let outbound_messages: Vec<_> = outbound_lines
            .iter()
            .map(|line| decode_message_line(line).expect("outbound line should decode"))
            .collect();
        assert!(
            outbound_messages.iter().any(|message| match message {
                ProtocolMessage::Set(payload) => payload
                    .set
                    .user
                    .as_ref()
                    .and_then(|users| users.get("alice"))
                    .and_then(|user| user.room.as_ref())
                    .is_some_and(|room| room.name == "room2"),
                _ => false,
            }),
            "sender should receive user room update"
        );
        assert!(
            outbound_messages.iter().any(|message| {
                matches!(
                    message,
                    ProtocolMessage::State(payload)
                    if payload.state.playstate.as_ref().is_some_and(|playstate| {
                        playstate.do_seek == Some(false) && playstate.paused == Some(true)
                    })
                )
            }),
            "sender should receive baseline room sync state update"
        );
        assert!(
            outbound_messages.iter().any(|message| {
                matches!(
                    message,
                    ProtocolMessage::State(payload)
                    if payload.state.playstate.as_ref().is_some_and(|playstate| {
                        playstate.do_seek == Some(true) && playstate.paused == Some(true)
                    })
                )
            }),
            "sender should receive seek room sync state update"
        );
        assert_eq!(
            runtime
                .session("client-1")
                .expect("session should exist")
                .room
                .as_str(),
            "room2"
        );
    }

    #[test]
    fn set_room_seek_sync_state_counter_increments_without_ack() {
        let mut runtime = ServerRuntime::default();
        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should establish session");

        let first_switch = runtime
            .handle_line_fanout("client-1", r#"{"Set":{"room":{"name":"room2"}}}"#)
            .expect("first room switch should succeed");
        let first_messages = decode_directed_lines(&first_switch);
        assert_eq!(
            room_seek_sync_server_counters(&first_messages, "client-1"),
            vec![1],
            "first room-switch seek sync should carry server counter 1"
        );

        let second_switch = runtime
            .handle_line_fanout("client-1", r#"{"Set":{"room":{"name":"room3"}}}"#)
            .expect("second room switch should succeed");
        let second_messages = decode_directed_lines(&second_switch);
        assert_eq!(
            room_seek_sync_server_counters(&second_messages, "client-1"),
            vec![2],
            "second room-switch seek sync should increment server counter without ack"
        );
    }

    #[test]
    fn list_requires_existing_session() {
        let mut runtime = ServerRuntime::default();
        let err = runtime
            .handle_line("unknown-client", r#"{"List":null}"#)
            .expect_err("list without hello should fail");
        assert!(matches!(err, ServerRuntimeError::MissingSession(_)));
    }

    #[test]
    fn hello_fanout_notifies_existing_room_members() {
        let mut runtime = ServerRuntime::default();
        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("first hello should establish room");

        let directed_lines = runtime
            .handle_line_fanout(
                "client-2",
                r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("second hello should fan out user events");
        let directed_messages = decode_directed_lines(&directed_lines);

        assert_eq!(directed_messages.len(), 5);

        assert!(
            directed_messages.iter().any(|(recipient, message)| {
                recipient == "client-2" && matches!(message, ProtocolMessage::Hello(_))
            }),
            "expected hello response to new client"
        );
        assert!(
            has_user_event(&directed_messages, "client-1", "bob", "joined"),
            "existing room member should receive joined event for bob"
        );
        assert!(
            has_playlist_snapshot(&directed_messages, "client-2", &[]),
            "new client should receive playlist snapshot before hello"
        );
        assert!(
            !has_user_event(&directed_messages, "client-2", "alice", "joined"),
            "new client should not receive synthetic joined snapshot for existing users"
        );
        assert!(
            has_room_sync_state_update(&directed_messages, "client-1", false),
            "existing room member should receive baseline room sync state update on peer join"
        );
        assert!(
            has_room_sync_state_update(&directed_messages, "client-2", false),
            "new room member should receive baseline room sync state update on join"
        );
    }

    #[test]
    fn hello_username_conflict_is_resolved_with_underscored_variant() {
        let mut runtime = ServerRuntime::default();
        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("first hello should establish session");

        let directed_lines = runtime
            .handle_line_fanout(
                "client-2",
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("second hello should resolve username conflict");
        let directed_messages = decode_directed_lines(&directed_lines);

        assert!(
            has_user_event(&directed_messages, "client-1", "alice_", "joined"),
            "existing user should observe conflict-resolved username"
        );
        let response_message = directed_messages
            .iter()
            .find(|(client_id, message)| {
                client_id == "client-2" && matches!(message, ProtocolMessage::Hello(_))
            })
            .expect("conflict-resolved hello response should be sent to joining client")
            .1
            .clone();
        let response_hello =
            extract_hello_from_message(response_message).expect("hello response should decode");
        assert_eq!(response_hello.username, "alice_");
        assert_eq!(
            runtime
                .session("client-2")
                .expect("session should be registered")
                .username,
            "alice_"
        );
    }

    #[test]
    fn hello_username_conflict_applies_legacy_trailing_underscore_rules() {
        let mut runtime = ServerRuntime::default();
        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice_","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("first hello should establish session");
        runtime
            .handle_line(
                "client-2",
                r#"{"Hello":{"username":"alice_","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("second hello should apply trailing-underscore conflict handling");
        assert_eq!(
            runtime
                .session("client-2")
                .expect("second session should exist")
                .username,
            "alice",
            "collision on name ending with underscore should first strip underscores"
        );

        runtime
            .handle_line(
                "client-3",
                r#"{"Hello":{"username":"alice_","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("third hello should append underscores until free");
        assert_eq!(
            runtime
                .session("client-3")
                .expect("third session should exist")
                .username,
            "alice__",
            "after stripping to a conflicting base username, underscores should be appended"
        );
    }

    #[test]
    fn hello_response_features_reflect_chat_readiness_and_length_limits() {
        let mut runtime = ServerRuntime::default();
        runtime.set_chat_enabled(false);
        runtime.set_readiness_enabled(false);
        runtime.set_max_chat_message_length(42);
        runtime.set_max_username_length(12);

        let directed_lines = runtime
            .handle_line_fanout(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should succeed");
        let directed_messages = decode_directed_lines(&directed_lines);

        let hello_message = directed_messages
            .into_iter()
            .find(|(recipient, message)| {
                recipient == "client-1" && matches!(message, ProtocolMessage::Hello(_))
            })
            .expect("hello response should be present")
            .1;
        let hello = extract_hello_from_message(hello_message).expect("hello payload should decode");
        let features = hello
            .features
            .expect("server hello should include features");
        assert_eq!(features.get("chat").and_then(Value::as_bool), Some(false));
        assert_eq!(
            features.get("readiness").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            features.get("maxChatMessageLength").and_then(Value::as_u64),
            Some(42)
        );
        assert_eq!(
            features.get("maxUsernameLength").and_then(Value::as_u64),
            Some(12)
        );
    }

    #[test]
    fn hello_response_features_reflect_isolate_rooms() {
        let mut runtime = ServerRuntime::default();
        runtime.set_isolate_rooms(true);

        let directed_lines = runtime
            .handle_line_fanout(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should succeed");
        let directed_messages = decode_directed_lines(&directed_lines);
        let hello_message = directed_messages
            .into_iter()
            .find(|(recipient, message)| {
                recipient == "client-1" && matches!(message, ProtocolMessage::Hello(_))
            })
            .expect("hello response should be present")
            .1;
        let hello = extract_hello_from_message(hello_message).expect("hello payload should decode");
        let features = hello
            .features
            .expect("server hello should include features");
        assert_eq!(
            features.get("isolateRooms").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn hello_requires_server_password_token_when_configured() {
        let mut runtime = ServerRuntime::default();
        runtime.set_server_password_token(Some("secret".to_owned()));

        let directed_lines = runtime
            .handle_line_fanout(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should return protocol error response");
        let directed_messages = decode_directed_lines(&directed_lines);

        assert!(
            directed_messages.iter().any(|(recipient, message)| {
                recipient == "client-1"
                    && matches!(
                        message,
                        ProtocolMessage::Error(payload)
                            if payload.error.message == LEGACY_SERVER_PASSWORD_REQUIRED_ERROR
                    )
            }),
            "hello without password should receive legacy password-required error"
        );
        assert!(
            runtime.session("client-1").is_none(),
            "session should not be created after password failure"
        );
    }

    #[test]
    fn hello_server_password_token_accepts_exact_match_and_username_is_truncated() {
        let mut runtime = ServerRuntime::default();
        runtime.set_server_password_token(Some("secret".to_owned()));
        runtime.set_max_username_length(4);

        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice-long","password":"secret","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello with matching password should succeed");
        assert_eq!(
            runtime
                .session("client-1")
                .expect("session should exist")
                .username,
            "alic"
        );
    }

    #[test]
    fn hello_server_password_token_accepts_legacy_python_md5_hash() {
        let mut runtime = ServerRuntime::default();
        runtime.set_server_password_token(Some("secret".to_owned()));

        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","password":"5ebe2294ecd0e0f08eab7690d2a6ee69","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello with Python-style MD5 password token should succeed");
        assert!(
            runtime.session("client-1").is_some(),
            "session should be created after MD5-compatible password match"
        );
    }

    #[test]
    fn hello_server_password_token_rejects_non_matching_token() {
        let mut runtime = ServerRuntime::default();
        runtime.set_server_password_token(Some("secret".to_owned()));

        let directed_lines = runtime
            .handle_line_fanout(
                "client-1",
                r#"{"Hello":{"username":"alice","password":"deadbeef","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should return protocol error response");
        let directed_messages = decode_directed_lines(&directed_lines);

        assert!(
            directed_messages.iter().any(|(recipient, message)| {
                recipient == "client-1"
                    && matches!(
                        message,
                        ProtocolMessage::Error(payload)
                            if payload.error.message == LEGACY_SERVER_WRONG_PASSWORD_ERROR
                    )
            }),
            "hello with wrong password token should receive legacy wrong-password error"
        );
        assert!(
            runtime.session("client-1").is_none(),
            "session should not be created after wrong password"
        );
    }

    #[test]
    fn chat_and_ready_updates_obey_runtime_disable_flags_and_chat_limit() {
        let mut runtime = ServerRuntime::default();
        runtime.set_chat_enabled(false);
        runtime.set_readiness_enabled(false);
        runtime.set_max_chat_message_length(4);
        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("alice hello should succeed");
        runtime
            .handle_line(
                "client-2",
                r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("bob hello should succeed");

        let chat_disabled = runtime
            .handle_line_fanout("client-1", r#"{"Chat":"hello world"}"#)
            .expect("chat while disabled should be ignored");
        assert!(
            chat_disabled.is_empty(),
            "chat should be ignored when disabled"
        );

        let ready_disabled = runtime
            .handle_line_fanout(
                "client-1",
                r#"{"Set":{"ready":{"isReady":true,"manuallyInitiated":true}}}"#,
            )
            .expect("ready while disabled should be ignored");
        assert!(
            ready_disabled.is_empty(),
            "ready update should be ignored when readiness is disabled"
        );

        runtime.set_chat_enabled(true);
        let chat_enabled = runtime
            .handle_line_fanout("client-1", r#"{"Chat":"hello world"}"#)
            .expect("chat after enabling should fan out");
        let directed_messages = decode_directed_lines(&chat_enabled);
        assert!(
            directed_messages.iter().any(|(recipient, message)| {
                recipient == "client-2"
                    && matches!(
                        message,
                        ProtocolMessage::Chat(payload)
                            if matches!(
                                &payload.chat,
                                ChatPayload::Message(chat) if chat.message == "hell"
                            ) || matches!(&payload.chat, ChatPayload::Text(text) if text == "hell")
                    )
            }),
            "chat message should be truncated to runtime max length"
        );
    }

    #[test]
    fn isolate_rooms_join_events_do_not_leak_to_other_rooms() {
        let mut runtime = ServerRuntime::default();
        runtime.set_isolate_rooms(true);
        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("alice hello should succeed");
        runtime
            .handle_line(
                "client-2",
                r#"{"Hello":{"username":"charlie","room":{"name":"room2"},"version":"1.2.255"}}"#,
            )
            .expect("charlie hello should succeed");

        let directed_lines = runtime
            .handle_line_fanout(
                "client-3",
                r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("bob hello should succeed");
        let directed_messages = decode_directed_lines(&directed_lines);
        assert!(
            has_user_event(&directed_messages, "client-1", "bob", "joined"),
            "same-room peer should receive join event"
        );
        assert!(
            !has_user_event(&directed_messages, "client-2", "bob", "joined"),
            "other-room peer should not receive join event when isolateRooms is enabled"
        );
    }

    #[test]
    fn isolate_rooms_list_request_is_scoped_to_requester_room() {
        let mut runtime = ServerRuntime::default();
        runtime.set_isolate_rooms(true);
        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("alice hello should succeed");
        runtime
            .handle_line(
                "client-2",
                r#"{"Hello":{"username":"charlie","room":{"name":"room2"},"version":"1.2.255"}}"#,
            )
            .expect("charlie hello should succeed");

        let outbound_lines = runtime
            .handle_line("client-1", r#"{"List":null}"#)
            .expect("list request should succeed");
        let response =
            decode_message_line(&outbound_lines[0]).expect("list response should decode");
        let ProtocolMessage::List(payload) = response else {
            panic!("expected list response");
        };
        let ListPayload::Rooms(rooms) = payload.list else {
            panic!("expected room snapshot list");
        };
        assert!(rooms.contains_key("room1"));
        assert!(
            !rooms.contains_key("room2"),
            "other rooms should be hidden in isolateRooms mode"
        );
    }

    #[test]
    fn isolate_rooms_room_switch_sends_left_to_old_room_without_destination_leak() {
        let mut runtime = ServerRuntime::default();
        runtime.set_isolate_rooms(true);
        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("alice hello should succeed");
        runtime
            .handle_line(
                "client-2",
                r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("bob hello should succeed");
        runtime
            .handle_line(
                "client-3",
                r#"{"Hello":{"username":"charlie","room":{"name":"room2"},"version":"1.2.255"}}"#,
            )
            .expect("charlie hello should succeed");

        let directed_lines = runtime
            .handle_line_fanout("client-1", r#"{"Set":{"room":{"name":"room2"}}}"#)
            .expect("room switch should succeed");
        let directed_messages = decode_directed_lines(&directed_lines);

        assert!(
            has_user_event(&directed_messages, "client-2", "alice", "left"),
            "old-room peer should receive left event"
        );
        assert!(
            !has_user_room_update(&directed_messages, "client-2", "alice", "room2"),
            "old-room peer should not receive destination room update in isolateRooms mode"
        );
        assert!(
            has_user_room_update(&directed_messages, "client-3", "alice", "room2"),
            "new-room peer should receive room update for moved user"
        );
    }

    #[test]
    fn ready_updates_are_broadcast_to_room_members() {
        let mut runtime = ServerRuntime::default();
        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("alice hello should establish session");
        runtime
            .handle_line(
                "client-2",
                r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("bob hello should establish session");

        let directed_lines = runtime
            .handle_line_fanout(
                "client-1",
                r#"{"Set":{"ready":{"isReady":true,"manuallyInitiated":true}}}"#,
            )
            .expect("ready update should fan out");
        let directed_messages = decode_directed_lines(&directed_lines);

        assert!(
            has_ready_update(&directed_messages, "client-1", "alice", true),
            "sender should receive echoed ready update"
        );
        assert!(
            has_ready_update(&directed_messages, "client-2", "alice", true),
            "peer should receive ready update"
        );
    }

    #[test]
    fn state_playstate_updates_are_broadcast_to_room_members_with_metadata() {
        let mut runtime = ServerRuntime::default();
        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("alice hello should establish session");
        runtime
            .handle_line(
                "client-2",
                r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("bob hello should establish session");

        let directed_lines = runtime
            .handle_line_fanout(
                "client-1",
                r#"{"State":{"playstate":{"position":12.5,"paused":false,"doSeek":false}}}"#,
            )
            .expect("state playstate update should fan out");
        let directed_messages = decode_directed_lines(&directed_lines);

        assert_eq!(directed_messages.len(), 2);
        assert!(
            has_state_update(&directed_messages, "client-1", "alice", 12.5, false, false),
            "sender should receive reflected state update with setBy and ping metadata"
        );
        assert!(
            has_state_update(&directed_messages, "client-2", "alice", 12.5, false, false),
            "room peer should receive reflected state update with setBy and ping metadata"
        );
    }

    #[test]
    fn state_playstate_without_seek_or_pause_change_produces_no_immediate_outbound_messages() {
        let mut runtime = ServerRuntime::default();
        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("alice hello should establish session");
        runtime
            .handle_line(
                "client-2",
                r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("bob hello should establish session");

        let first_update = runtime
            .handle_line_fanout(
                "client-1",
                r#"{"State":{"playstate":{"position":12.5,"paused":false,"doSeek":false}}}"#,
            )
            .expect("first state update should trigger forced room fanout");
        assert_eq!(
            first_update.len(),
            2,
            "pause transition should force state propagation to room members"
        );

        let second_update = runtime
            .handle_line_fanout(
                "client-1",
                r#"{"State":{"playstate":{"position":13.0,"paused":false,"doSeek":false}}}"#,
            )
            .expect("second state update should be accepted");
        assert!(
            second_update.is_empty(),
            "state updates without seek/pause transitions should not force immediate fanout"
        );
    }

    #[test]
    fn state_forced_update_forwards_sender_client_metadata_once() {
        let mut runtime = ServerRuntime::default();
        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("alice hello should establish session");
        runtime
            .handle_line(
                "client-2",
                r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("bob hello should establish session");

        let first_forced_lines = runtime
            .handle_line_fanout(
                "client-1",
                r#"{"State":{"playstate":{"position":12.5,"paused":false,"doSeek":false},"ping":{"clientLatencyCalculation":124.1,"clientRtt":0.12},"ignoringOnTheFly":{"client":7}}}"#,
            )
            .expect("first forced state update should fan out");
        let first_forced_messages = decode_directed_lines(&first_forced_lines);
        assert_eq!(first_forced_messages.len(), 2);

        for (recipient, message) in &first_forced_messages {
            let ProtocolMessage::State(payload) = message else {
                panic!("forced state update should produce state messages");
            };
            let ping = payload
                .state
                .ping
                .as_ref()
                .expect("forced state update should include ping");
            let ignore = payload
                .state
                .ignoring_on_the_fly
                .as_ref()
                .expect("forced state update should include ignore counters");
            if recipient == "client-1" {
                assert_eq!(ping.client_latency_calculation, Some(124.1));
                assert_eq!(ignore.client, Some(7));
            } else {
                assert_eq!(ping.client_latency_calculation, None);
                assert_eq!(ignore.client, None);
            }
        }

        let second_forced_lines = runtime
            .handle_line_fanout(
                "client-1",
                r#"{"State":{"ignoringOnTheFly":{"server":1},"playstate":{"position":13.0,"paused":false,"doSeek":true}}}"#,
            )
            .expect("second forced state update should fan out");
        let second_forced_messages = decode_directed_lines(&second_forced_lines);
        assert_eq!(second_forced_messages.len(), 2);
        for (recipient, message) in &second_forced_messages {
            let ProtocolMessage::State(payload) = message else {
                panic!("forced state update should produce state messages");
            };
            let ping = payload
                .state
                .ping
                .as_ref()
                .expect("forced state update should include ping");
            let ignore = payload
                .state
                .ignoring_on_the_fly
                .as_ref()
                .expect("forced state update should include ignore counters");
            assert_eq!(
                ping.client_latency_calculation, None,
                "client latency passthrough should be consumed after first forced send"
            );
            assert_eq!(
                ignore.client, None,
                "client ignore passthrough should be consumed after first forced send"
            );
            if recipient == "client-1" {
                assert_eq!(
                    ignore.server,
                    Some(1),
                    "sender counter should reset after ack and increment again"
                );
            } else if recipient == "client-2" {
                assert_eq!(
                    ignore.server,
                    Some(2),
                    "peer counter should continue incrementing without ack"
                );
            } else {
                panic!("unexpected recipient for forced state fanout");
            }
        }
    }

    #[test]
    fn state_ping_only_client_metadata_is_forwarded_on_next_forced_update() {
        let mut runtime = ServerRuntime::default();
        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("alice hello should establish session");
        runtime
            .handle_line(
                "client-2",
                r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("bob hello should establish session");

        let ping_only_lines = runtime
            .handle_line_fanout(
                "client-1",
                r#"{"State":{"ping":{"clientLatencyCalculation":222.2},"ignoringOnTheFly":{"client":5}}}"#,
            )
            .expect("ping-only update should be accepted");
        assert!(
            ping_only_lines.is_empty(),
            "ping-only updates should still emit no immediate fanout"
        );

        let forced_lines = runtime
            .handle_line_fanout(
                "client-1",
                r#"{"State":{"playstate":{"position":3.0,"paused":false,"doSeek":false}}}"#,
            )
            .expect("subsequent forced state update should fan out");
        let forced_messages = decode_directed_lines(&forced_lines);
        assert_eq!(forced_messages.len(), 2);
        let sender_message = forced_messages
            .iter()
            .find(|(recipient, _)| recipient == "client-1")
            .expect("sender should receive forced update")
            .1
            .clone();
        let ProtocolMessage::State(payload) = sender_message else {
            panic!("sender forced output should be state");
        };
        assert_eq!(
            payload
                .state
                .ping
                .as_ref()
                .and_then(|ping| ping.client_latency_calculation),
            Some(222.2)
        );
        assert_eq!(
            payload
                .state
                .ignoring_on_the_fly
                .as_ref()
                .and_then(|ignore| ignore.client),
            Some(5)
        );
    }

    #[test]
    fn state_ping_metrics_apply_forward_delay_and_non_zero_server_rtt_for_sender() {
        let mut runtime = ServerRuntime::default();
        runtime.set_time_now_override_seconds(Some(100.0));
        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("alice hello should establish session");
        runtime
            .handle_line(
                "client-2",
                r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("bob hello should establish session");

        let directed_lines = runtime
            .handle_line_fanout(
                "client-1",
                r#"{"State":{"playstate":{"position":5.0,"paused":false,"doSeek":true},"ping":{"latencyCalculation":90.0,"clientRtt":2.0}}}"#,
            )
            .expect("state update with ping metrics should be accepted");
        let directed_messages = decode_directed_lines(&directed_lines);
        assert_eq!(directed_messages.len(), 2);

        for (recipient, message) in directed_messages {
            let ProtocolMessage::State(payload) = message else {
                panic!("state update should fanout as state messages");
            };
            let playstate = payload
                .state
                .playstate
                .as_ref()
                .expect("fanout state should include playstate");
            assert!(
                playstate
                    .position
                    .is_some_and(|position| (position - 18.0).abs() <= 0.000_001),
                "forward delay should be applied to unpaused position updates"
            );
            assert_eq!(playstate.paused, Some(false));
            assert_eq!(playstate.do_seek, Some(true));
            let server_rtt = payload
                .state
                .ping
                .as_ref()
                .and_then(|ping| ping.server_rtt)
                .expect("fanout state should include ping.serverRtt");
            if recipient == "client-1" {
                assert!(
                    (server_rtt - 10.0).abs() <= 0.000_001,
                    "sender should receive updated non-zero serverRtt from ping metrics"
                );
            } else if recipient == "client-2" {
                assert_eq!(
                    server_rtt, 0.0,
                    "peer without inbound ping metrics should keep default zero serverRtt"
                );
            } else {
                panic!("unexpected recipient");
            }
        }
    }

    #[test]
    fn controlled_room_non_controller_state_update_gets_forced_corrections() {
        let controlled_room_name = controlled_room_name_for_test("room1", "AB-123-456");
        let mut runtime = ServerRuntime::default();
        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("alice hello should establish session");
        runtime
            .handle_line(
                "client-2",
                r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("bob hello should establish session");
        runtime
            .handle_line_fanout(
                "client-1",
                r#"{"Set":{"controllerAuth":{"room":"room1","password":"AB-123-456"}}}"#,
            )
            .expect("controller auth on uncontrolled room should respond");
        runtime
            .handle_line_fanout(
                "client-1",
                &format!(r#"{{"Set":{{"room":{{"name":"{controlled_room_name}"}}}}}}"#),
            )
            .expect("alice switch to controlled room should succeed");
        runtime
            .handle_line_fanout(
                "client-2",
                &format!(r#"{{"Set":{{"room":{{"name":"{controlled_room_name}"}}}}}}"#),
            )
            .expect("bob switch to controlled room should succeed");
        runtime
            .handle_line_fanout(
                "client-2",
                r#"{"State":{"ignoringOnTheFly":{"server":1},"ping":{"latencyCalculation":100.0}}}"#,
            )
            .expect("bob should ack room-switch forced state before sending updates");

        let directed_lines = runtime
            .handle_line_fanout(
                "client-2",
                r#"{"State":{"playstate":{"position":42.0,"paused":false,"doSeek":false}}}"#,
            )
            .expect("non-controller state update should receive correction pair");
        let directed_messages = decode_directed_lines(&directed_lines);

        assert_eq!(
            directed_messages.len(),
            2,
            "non-controller forced correction should emit exactly two directed state updates"
        );
        assert!(
            directed_messages
                .iter()
                .all(|(recipient, _)| recipient == "client-2"),
            "non-controller correction flow should be directed only to sender"
        );

        let ProtocolMessage::State(first_state) = &directed_messages[0].1 else {
            panic!("first correction should be a state message");
        };
        let first_playstate = first_state
            .state
            .playstate
            .as_ref()
            .expect("first correction should include playstate");
        assert_eq!(first_playstate.position, Some(0.0));
        assert_eq!(first_playstate.paused, Some(false));
        assert_eq!(first_playstate.do_seek, Some(false));
        assert_eq!(first_playstate.set_by.as_deref(), Some("bob"));
        assert_eq!(
            first_state
                .state
                .ignoring_on_the_fly
                .as_ref()
                .and_then(|ignore| ignore.server),
            Some(1)
        );

        let ProtocolMessage::State(second_state) = &directed_messages[1].1 else {
            panic!("second correction should be a state message");
        };
        let second_playstate = second_state
            .state
            .playstate
            .as_ref()
            .expect("second correction should include playstate");
        assert_eq!(second_playstate.position, Some(0.0));
        assert_eq!(second_playstate.paused, Some(true));
        assert_eq!(second_playstate.do_seek, Some(true));
        assert_eq!(second_playstate.set_by, None);
        assert_eq!(
            second_state
                .state
                .ignoring_on_the_fly
                .as_ref()
                .and_then(|ignore| ignore.server),
            Some(2)
        );
    }

    #[test]
    fn state_ping_only_update_produces_no_immediate_outbound_messages() {
        let mut runtime = ServerRuntime::default();
        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("alice hello should establish session");

        let directed_lines = runtime
            .handle_line_fanout(
                "client-1",
                r#"{"State":{"ping":{"latencyCalculation":123.4,"clientLatencyCalculation":124.1,"clientRtt":0.12}}}"#,
            )
            .expect("state ping-only update should be accepted");

        assert!(
            directed_lines.is_empty(),
            "ping-only update should not emit immediate state fanout"
        );
    }

    #[test]
    fn state_requires_existing_session() {
        let mut runtime = ServerRuntime::default();
        let err = runtime
            .handle_line(
                "unknown-client",
                r#"{"State":{"playstate":{"position":1.0,"paused":false,"doSeek":false}}}"#,
            )
            .expect_err("state without hello should fail");
        assert!(matches!(err, ServerRuntimeError::MissingSession(_)));
    }

    #[test]
    fn periodic_state_updates_emit_after_time_advance() {
        let mut runtime = ServerRuntime::default();
        runtime.set_time_now_override_seconds(Some(0.0));
        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("alice hello should establish session");
        runtime
            .handle_line(
                "client-2",
                r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("bob hello should establish session");

        let periodic_lines = runtime
            .advance_time_and_collect_fanout(super::SERVER_STATE_INTERVAL_SECONDS)
            .expect("periodic state tick should encode outbound fanout lines");
        let periodic_messages = decode_directed_lines(&periodic_lines);

        assert_eq!(
            periodic_messages.len(),
            2,
            "one periodic idle state update should be emitted per connected client"
        );
        let mut recipients = BTreeSet::new();
        for (recipient, message) in periodic_messages {
            recipients.insert(recipient);
            let ProtocolMessage::State(payload) = message else {
                panic!("periodic output should be state message");
            };
            let playstate = payload
                .state
                .playstate
                .as_ref()
                .expect("periodic state update should include playstate");
            assert_eq!(playstate.position, Some(0.0));
            assert_eq!(playstate.paused, Some(true));
            assert_eq!(playstate.do_seek, Some(false));
            assert_eq!(
                playstate.set_by.as_deref(),
                Some("alice"),
                "periodic idle updates should carry room setBy watcher identity"
            );
            assert!(
                payload.state.ignoring_on_the_fly.is_none(),
                "periodic idle updates should not include server ignore counters"
            );
        }
        assert_eq!(
            recipients,
            BTreeSet::from(["client-1".to_owned(), "client-2".to_owned()])
        );
    }

    #[test]
    fn periodic_timeout_disconnects_stale_client_and_broadcasts_left_event() {
        let mut runtime = ServerRuntime::default();
        runtime.set_time_now_override_seconds(Some(0.0));
        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("alice hello should establish session");
        runtime
            .handle_line(
                "client-2",
                r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("bob hello should establish session");

        let _ = runtime
            .advance_time_and_collect_fanout(10.0)
            .expect("periodic state ticks before timeout should encode");
        runtime
            .handle_line_fanout(
                "client-2",
                r#"{"State":{"ping":{"latencyCalculation":10.0}}}"#,
            )
            .expect("ping-only update should refresh client timeout timestamp");

        let timeout_lines = runtime
            .advance_time_and_collect_fanout(3.0)
            .expect("timeout tick should encode outbound fanout lines");
        let timeout_messages = decode_directed_lines(&timeout_lines);

        assert!(
            runtime.session("client-1").is_none(),
            "stale client should be dropped after protocol timeout"
        );
        assert!(
            runtime.session("client-2").is_some(),
            "recently updated peer should remain connected"
        );
        assert!(
            has_user_event(&timeout_messages, "client-2", "alice", "left"),
            "peer should receive left event when stale client is dropped"
        );
    }

    #[test]
    fn room_change_fanout_emits_global_room_update_and_playlist_snapshot() {
        let mut runtime = ServerRuntime::default();
        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("alice hello should establish session");
        runtime
            .handle_line(
                "client-2",
                r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("bob hello should establish session");
        runtime
            .handle_line(
                "client-3",
                r#"{"Hello":{"username":"carol","room":{"name":"room2"},"version":"1.2.255"}}"#,
            )
            .expect("carol hello should establish session");

        let directed_lines = runtime
            .handle_line_fanout("client-1", r#"{"Set":{"room":{"name":"room2"}}}"#)
            .expect("room change should fan out");
        let directed_messages = decode_directed_lines(&directed_lines);

        assert!(
            has_user_room_update(&directed_messages, "client-1", "alice", "room2"),
            "sender should receive global user room update"
        );
        assert!(
            has_user_room_update(&directed_messages, "client-2", "alice", "room2"),
            "old-room peer should receive global user room update"
        );
        assert!(
            has_user_room_update(&directed_messages, "client-3", "alice", "room2"),
            "new-room peer should receive global user room update"
        );
        assert!(
            has_playlist_snapshot(&directed_messages, "client-1", &[]),
            "moved user should receive playlist snapshot after room switch"
        );
        assert!(
            !has_playlist_snapshot(&directed_messages, "client-3", &[]),
            "destination room peers should not receive direct playlist snapshot for mover"
        );
        assert!(
            has_room_sync_state_update(&directed_messages, "client-1", false),
            "moved user should receive baseline room sync state update"
        );
        assert!(
            has_room_sync_state_update(&directed_messages, "client-1", true),
            "moved user should receive seek room sync state update"
        );
    }

    #[test]
    fn controller_auth_on_uncontrolled_room_returns_new_controlled_room_to_sender() {
        let mut runtime = ServerRuntime::default();
        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("alice hello should establish session");

        let directed_lines = runtime
            .handle_line_fanout(
                "client-1",
                r#"{"Set":{"controllerAuth":{"room":"room1","password":"AB-123-456"}}}"#,
            )
            .expect("controller auth on uncontrolled room should respond");
        assert_eq!(directed_lines.len(), 1);
        assert_eq!(directed_lines[0].client_id, "client-1");

        let message = decode_message_line(&directed_lines[0].line)
            .expect("new controlled room line should decode");
        let expected_room_name = controlled_room_name_for_test("room1", "AB-123-456");
        match message {
            ProtocolMessage::Set(payload) => {
                let new_room = payload
                    .set
                    .new_controlled_room
                    .as_ref()
                    .expect("newControlledRoom payload should be present");
                assert_eq!(new_room.password.as_deref(), Some("AB-123-456"));
                assert_eq!(
                    new_room.room_name.as_deref(),
                    Some(expected_room_name.as_str())
                );
            }
            other => panic!("expected set response, got {}", other.kind()),
        }
    }

    #[test]
    fn controller_auth_respects_runtime_configured_room_password_salt() {
        let mut runtime = ServerRuntime::with_room_password_salt("custom-salt");
        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("alice hello should establish session");

        let directed_lines = runtime
            .handle_line_fanout(
                "client-1",
                r#"{"Set":{"controllerAuth":{"room":"room1","password":"AB-123-456"}}}"#,
            )
            .expect("controller auth on uncontrolled room should respond");
        assert_eq!(directed_lines.len(), 1);
        assert_eq!(directed_lines[0].client_id, "client-1");

        let message = decode_message_line(&directed_lines[0].line)
            .expect("new controlled room line should decode");
        let expected_room_name =
            controlled_room_name_for_salt_test("room1", "AB-123-456", "custom-salt");
        let default_room_name = controlled_room_name_for_test("room1", "AB-123-456");
        match message {
            ProtocolMessage::Set(payload) => {
                let new_room = payload
                    .set
                    .new_controlled_room
                    .as_ref()
                    .expect("newControlledRoom payload should be present");
                assert_eq!(
                    new_room.room_name.as_deref(),
                    Some(expected_room_name.as_str())
                );
                assert_ne!(
                    new_room.room_name.as_deref(),
                    Some(default_room_name.as_str())
                );
            }
            other => panic!("expected set response, got {}", other.kind()),
        }
    }

    #[test]
    fn controlled_room_playlist_updates_require_controller_auth() {
        let controlled_room_name = controlled_room_name_for_test("room1", "AB-123-456");
        let mut runtime = ServerRuntime::default();
        runtime
            .handle_line(
                "client-1",
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("alice hello should establish session");
        runtime
            .handle_line(
                "client-2",
                r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("bob hello should establish session");
        runtime
            .handle_line_fanout(
                "client-1",
                &format!(r#"{{"Set":{{"room":{{"name":"{controlled_room_name}"}}}}}}"#),
            )
            .expect("alice room switch should succeed");
        runtime
            .handle_line_fanout(
                "client-2",
                &format!(r#"{{"Set":{{"room":{{"name":"{controlled_room_name}"}}}}}}"#),
            )
            .expect("bob room switch should succeed");

        let bob_change_attempt = runtime
            .handle_line_fanout(
                "client-2",
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"]}}}"#,
            )
            .expect("bob playlist change attempt should respond");
        assert_eq!(bob_change_attempt.len(), 1);
        assert!(
            bob_change_attempt
                .iter()
                .all(|line| line.client_id == "client-2"),
            "non-controller correction should be directed only to sender"
        );
        let bob_messages: Vec<_> = bob_change_attempt
            .iter()
            .map(|line| decode_message_line(&line.line).expect("line should decode"))
            .collect();
        assert!(
            bob_messages.iter().any(|message| match message {
                ProtocolMessage::Set(payload) =>
                    payload
                        .set
                        .playlist_change
                        .as_ref()
                        .is_some_and(|playlist| {
                            playlist.files.is_empty()
                                && playlist.user.as_deref() == Some(controlled_room_name.as_str())
                        },),
                _ => false,
            }),
            "non-controller should receive playlistChange correction for room state"
        );
        let alice_auth = runtime
            .handle_line_fanout(
                "client-1",
                &format!(
                    r#"{{"Set":{{"controllerAuth":{{"room":"{controlled_room_name}","password":"AB-123-456"}}}}}}"#
                ),
            )
            .expect("alice auth should succeed");
        assert!(
            alice_auth.iter().any(|line| {
                decode_message_line(&line.line)
                    .ok()
                    .is_some_and(|message| match message {
                        ProtocolMessage::Set(payload) => payload
                            .set
                            .controller_auth
                            .as_ref()
                            .is_some_and(|auth| auth.success == Some(true)),
                        _ => false,
                    })
            }),
            "controller auth success should be broadcast"
        );

        let alice_change = runtime
            .handle_line_fanout(
                "client-1",
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"]}}}"#,
            )
            .expect("alice playlist change should succeed as controller");
        assert!(
            alice_change.iter().any(|line| line.client_id == "client-1")
                && alice_change.iter().any(|line| line.client_id == "client-2"),
            "controller playlist change should fan out to room peers"
        );
    }
}
