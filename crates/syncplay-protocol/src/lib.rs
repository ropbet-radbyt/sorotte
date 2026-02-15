use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomRef {
    pub name: String,
}

impl RoomRef {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HelloPayload {
    pub username: String,
    pub room: RoomRef,
    pub version: String,
    #[serde(default)]
    pub realversion: Option<String>,
    #[serde(default)]
    pub features: Option<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl HelloPayload {
    pub fn new(
        username: impl Into<String>,
        room_name: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        Self {
            username: username.into(),
            room: RoomRef::new(room_name),
            version: version.into(),
            realversion: None,
            features: None,
            extra: BTreeMap::new(),
        }
    }

    pub fn with_realversion(mut self, realversion: impl Into<String>) -> Self {
        self.realversion = Some(realversion.into());
        self
    }

    pub fn with_features(mut self, features: Value) -> Self {
        self.features = Some(features);
        self
    }

    pub fn effective_version(&self) -> &str {
        self.realversion.as_deref().unwrap_or(self.version.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SetPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<RoomRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<FilePayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<BTreeMap<String, UserSetPayload>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "controllerAuth"
    )]
    pub controller_auth: Option<ControllerAuthPayload>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "newControlledRoom"
    )]
    pub new_controlled_room: Option<NewControlledRoomPayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready: Option<ReadyPayload>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "playlistChange"
    )]
    pub playlist_change: Option<PlaylistChangePayload>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "playlistIndex"
    )]
    pub playlist_index: Option<PlaylistIndexPayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl SetPayload {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_room(mut self, room: RoomRef) -> Self {
        self.room = Some(room);
        self
    }

    pub fn with_file(mut self, file: FilePayload) -> Self {
        self.file = Some(file);
        self
    }

    pub fn with_user(mut self, user: BTreeMap<String, UserSetPayload>) -> Self {
        self.user = Some(user);
        self
    }

    pub fn with_controller_auth(mut self, controller_auth: ControllerAuthPayload) -> Self {
        self.controller_auth = Some(controller_auth);
        self
    }

    pub fn with_new_controlled_room(
        mut self,
        new_controlled_room: NewControlledRoomPayload,
    ) -> Self {
        self.new_controlled_room = Some(new_controlled_room);
        self
    }

    pub fn with_ready(mut self, ready: ReadyPayload) -> Self {
        self.ready = Some(ready);
        self
    }

    pub fn with_playlist_change(mut self, playlist_change: PlaylistChangePayload) -> Self {
        self.playlist_change = Some(playlist_change);
        self
    }

    pub fn with_playlist_index(mut self, playlist_index: PlaylistIndexPayload) -> Self {
        self.playlist_index = Some(playlist_index);
        self
    }

    pub fn with_features(mut self, features: Value) -> Self {
        self.features = Some(features);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct FilePayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl FilePayload {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_duration(mut self, duration: f64) -> Self {
        self.duration = Some(duration);
        self
    }

    pub fn with_size(mut self, size: Value) -> Self {
        self.size = Some(size);
        self
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct UserSetPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<RoomRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "isReady")]
    pub is_ready: Option<bool>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl UserSetPayload {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_room(mut self, room: RoomRef) -> Self {
        self.room = Some(room);
        self
    }

    pub fn with_file(mut self, file: Value) -> Self {
        self.file = Some(file);
        self
    }

    pub fn with_event(mut self, event: Value) -> Self {
        self.event = Some(event);
        self
    }

    pub fn with_features(mut self, features: Value) -> Self {
        self.features = Some(features);
        self
    }

    pub fn with_controller(mut self, controller: bool) -> Self {
        self.controller = Some(controller);
        self
    }

    pub fn with_is_ready(mut self, is_ready: bool) -> Self {
        self.is_ready = Some(is_ready);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ControllerAuthPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl ControllerAuthPayload {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_room(mut self, room: impl Into<String>) -> Self {
        self.room = Some(room.into());
        self
    }

    pub fn with_password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
        self
    }

    pub fn with_user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self
    }

    pub fn with_success(mut self, success: bool) -> Self {
        self.success = Some(success);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct NewControlledRoomPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "roomName")]
    pub room_name: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl NewControlledRoomPayload {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
        self
    }

    pub fn with_room_name(mut self, room_name: impl Into<String>) -> Self {
        self.room_name = Some(room_name.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadyPayload {
    #[serde(rename = "isReady")]
    pub is_ready: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "manuallyInitiated"
    )]
    pub manually_initiated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "setBy")]
    pub set_by: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl ReadyPayload {
    pub fn new(is_ready: bool) -> Self {
        Self {
            is_ready,
            manually_initiated: None,
            username: None,
            set_by: None,
            extra: BTreeMap::new(),
        }
    }

    pub fn with_manually_initiated(mut self, manually_initiated: bool) -> Self {
        self.manually_initiated = Some(manually_initiated);
        self
    }

    pub fn with_username(mut self, username: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self
    }

    pub fn with_set_by(mut self, set_by: impl Into<String>) -> Self {
        self.set_by = Some(set_by.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaylistChangePayload {
    pub files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl PlaylistChangePayload {
    pub fn new(files: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            files: files.into_iter().map(Into::into).collect(),
            user: None,
            extra: BTreeMap::new(),
        }
    }

    pub fn with_user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaylistIndexPayload {
    pub index: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl PlaylistIndexPayload {
    pub fn new(index: i64) -> Self {
        Self {
            index,
            user: None,
            extra: BTreeMap::new(),
        }
    }

    pub fn with_user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ListPayload {
    Request(Option<()>),
    Rooms(BTreeMap<String, BTreeMap<String, ListUserEntry>>),
}

impl ListPayload {
    pub fn request() -> Self {
        Self::Request(None)
    }

    pub fn rooms(rooms: BTreeMap<String, BTreeMap<String, ListUserEntry>>) -> Self {
        Self::Rooms(rooms)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ListUserEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "isReady")]
    pub is_ready: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl ListUserEntry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_position(mut self, position: f64) -> Self {
        self.position = Some(position);
        self
    }

    pub fn with_file(mut self, file: Value) -> Self {
        self.file = Some(file);
        self
    }

    pub fn with_controller(mut self, controller: bool) -> Self {
        self.controller = Some(controller);
        self
    }

    pub fn with_is_ready(mut self, is_ready: bool) -> Self {
        self.is_ready = Some(is_ready);
        self
    }

    pub fn with_features(mut self, features: Value) -> Self {
        self.features = Some(features);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct StatePayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playstate: Option<PlaystatePayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ping: Option<PingPayload>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "ignoringOnTheFly"
    )]
    pub ignoring_on_the_fly: Option<IgnoringOnTheFlyPayload>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl StatePayload {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_playstate(mut self, playstate: PlaystatePayload) -> Self {
        self.playstate = Some(playstate);
        self
    }

    pub fn with_ping(mut self, ping: PingPayload) -> Self {
        self.ping = Some(ping);
        self
    }

    pub fn with_ignoring_on_the_fly(
        mut self,
        ignoring_on_the_fly: IgnoringOnTheFlyPayload,
    ) -> Self {
        self.ignoring_on_the_fly = Some(ignoring_on_the_fly);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PlaystatePayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "doSeek")]
    pub do_seek: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "setBy")]
    pub set_by: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl PlaystatePayload {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_position(mut self, position: f64) -> Self {
        self.position = Some(position);
        self
    }

    pub fn with_paused(mut self, paused: bool) -> Self {
        self.paused = Some(paused);
        self
    }

    pub fn with_do_seek(mut self, do_seek: bool) -> Self {
        self.do_seek = Some(do_seek);
        self
    }

    pub fn with_set_by(mut self, set_by: impl Into<String>) -> Self {
        self.set_by = Some(set_by.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PingPayload {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "latencyCalculation"
    )]
    pub latency_calculation: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "clientLatencyCalculation"
    )]
    pub client_latency_calculation: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "clientRtt")]
    pub client_rtt: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "serverRtt")]
    pub server_rtt: Option<f64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl PingPayload {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_latency_calculation(mut self, latency_calculation: f64) -> Self {
        self.latency_calculation = Some(latency_calculation);
        self
    }

    pub fn with_client_latency_calculation(mut self, client_latency_calculation: f64) -> Self {
        self.client_latency_calculation = Some(client_latency_calculation);
        self
    }

    pub fn with_client_rtt(mut self, client_rtt: f64) -> Self {
        self.client_rtt = Some(client_rtt);
        self
    }

    pub fn with_server_rtt(mut self, server_rtt: f64) -> Self {
        self.server_rtt = Some(server_rtt);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct IgnoringOnTheFlyPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<u32>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl IgnoringOnTheFlyPayload {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_server(mut self, server: u32) -> Self {
        self.server = Some(server);
        self
    }

    pub fn with_client(mut self, client: u32) -> Self {
        self.client = Some(client);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChatPayload {
    Text(String),
    Message(ChatMessagePayload),
}

impl ChatPayload {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(text.into())
    }

    pub fn message(username: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Message(ChatMessagePayload::new(username, message))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessagePayload {
    pub username: String,
    pub message: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl ChatMessagePayload {
    pub fn new(username: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            message: message.into(),
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorPayload {
    pub message: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl ErrorPayload {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TlsPayload {
    #[serde(rename = "startTLS")]
    pub start_tls: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl TlsPayload {
    pub fn new(start_tls: impl Into<String>) -> Self {
        Self {
            start_tls: start_tls.into(),
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HelloMessage {
    #[serde(rename = "Hello")]
    pub hello: HelloPayload,
}

impl HelloMessage {
    pub fn new(hello: HelloPayload) -> Self {
        Self { hello }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetMessage {
    #[serde(rename = "Set")]
    pub set: SetPayload,
}

impl SetMessage {
    pub fn new(set: SetPayload) -> Self {
        Self { set }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListMessage {
    #[serde(rename = "List")]
    pub list: ListPayload,
}

impl ListMessage {
    pub fn new(list: ListPayload) -> Self {
        Self { list }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateMessage {
    #[serde(rename = "State")]
    pub state: StatePayload,
}

impl StateMessage {
    pub fn new(state: StatePayload) -> Self {
        Self { state }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    #[serde(rename = "Chat")]
    pub chat: ChatPayload,
}

impl ChatMessage {
    pub fn new(chat: ChatPayload) -> Self {
        Self { chat }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorMessage {
    #[serde(rename = "Error")]
    pub error: ErrorPayload,
}

impl ErrorMessage {
    pub fn new(error: ErrorPayload) -> Self {
        Self { error }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TlsMessage {
    #[serde(rename = "TLS")]
    pub tls: TlsPayload,
}

impl TlsMessage {
    pub fn new(tls: TlsPayload) -> Self {
        Self { tls }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ProtocolMessage {
    Hello(HelloMessage),
    Set(Box<SetMessage>),
    List(Box<ListMessage>),
    State(Box<StateMessage>),
    Chat(ChatMessage),
    Error(ErrorMessage),
    Tls(TlsMessage),
}

impl ProtocolMessage {
    pub fn hello(hello: HelloPayload) -> Self {
        Self::Hello(HelloMessage::new(hello))
    }

    pub fn hello_basic(
        username: impl Into<String>,
        room_name: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        Self::hello(HelloPayload::new(username, room_name, version))
    }

    pub fn set(set: SetPayload) -> Self {
        Self::Set(Box::new(SetMessage::new(set)))
    }

    pub fn list(list: ListPayload) -> Self {
        Self::List(Box::new(ListMessage::new(list)))
    }

    pub fn list_request() -> Self {
        Self::list(ListPayload::request())
    }

    pub fn state(state: StatePayload) -> Self {
        Self::State(Box::new(StateMessage::new(state)))
    }

    pub fn chat(chat: ChatPayload) -> Self {
        Self::Chat(ChatMessage::new(chat))
    }

    pub fn chat_text(text: impl Into<String>) -> Self {
        Self::chat(ChatPayload::text(text))
    }

    pub fn chat_message(username: impl Into<String>, message: impl Into<String>) -> Self {
        Self::chat(ChatPayload::message(username, message))
    }

    pub fn error(error: ErrorPayload) -> Self {
        Self::Error(ErrorMessage::new(error))
    }

    pub fn error_message(message: impl Into<String>) -> Self {
        Self::error(ErrorPayload::new(message))
    }

    pub fn tls(tls: TlsPayload) -> Self {
        Self::Tls(TlsMessage::new(tls))
    }

    pub fn start_tls(mode: impl Into<String>) -> Self {
        Self::tls(TlsPayload::new(mode))
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Hello(_) => "Hello",
            Self::Set(_) => "Set",
            Self::List(_) => "List",
            Self::State(_) => "State",
            Self::Chat(_) => "Chat",
            Self::Error(_) => "Error",
            Self::Tls(_) => "TLS",
        }
    }
}

impl From<HelloPayload> for ProtocolMessage {
    fn from(value: HelloPayload) -> Self {
        Self::hello(value)
    }
}

impl From<SetPayload> for ProtocolMessage {
    fn from(value: SetPayload) -> Self {
        Self::set(value)
    }
}

impl From<ListPayload> for ProtocolMessage {
    fn from(value: ListPayload) -> Self {
        Self::list(value)
    }
}

impl From<StatePayload> for ProtocolMessage {
    fn from(value: StatePayload) -> Self {
        Self::state(value)
    }
}

impl From<ChatPayload> for ProtocolMessage {
    fn from(value: ChatPayload) -> Self {
        Self::chat(value)
    }
}

impl From<ErrorPayload> for ProtocolMessage {
    fn from(value: ErrorPayload) -> Self {
        Self::error(value)
    }
}

impl From<TlsPayload> for ProtocolMessage {
    fn from(value: TlsPayload) -> Self {
        Self::tls(value)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("invalid JSON payload: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("unexpected message kind: expected '{expected}', found '{found}'")]
    UnexpectedMessageKind {
        expected: &'static str,
        found: &'static str,
    },
}

pub fn decode_line(line: &str) -> Result<Value, ProtocolError> {
    serde_json::from_str(line).map_err(ProtocolError::from)
}

pub fn encode_line(value: &Value) -> Result<String, ProtocolError> {
    serde_json::to_string(value).map_err(ProtocolError::from)
}

pub fn decode_message_line(line: &str) -> Result<ProtocolMessage, ProtocolError> {
    serde_json::from_str(line).map_err(ProtocolError::from)
}

pub fn encode_message_line(message: &ProtocolMessage) -> Result<String, ProtocolError> {
    serde_json::to_string(message).map_err(ProtocolError::from)
}

pub fn extract_hello(value: &Value) -> Result<HelloPayload, ProtocolError> {
    let message: ProtocolMessage = serde_json::from_value(value.clone())?;
    match message {
        ProtocolMessage::Hello(hello) => Ok(hello.hello),
        other => Err(ProtocolError::UnexpectedMessageKind {
            expected: "Hello",
            found: other.kind(),
        }),
    }
}

pub fn extract_hello_from_message(message: ProtocolMessage) -> Result<HelloPayload, ProtocolError> {
    match message {
        ProtocolMessage::Hello(hello) => Ok(hello.hello),
        other => Err(ProtocolError::UnexpectedMessageKind {
            expected: "Hello",
            found: other.kind(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use serde_json::json;

    use super::{
        ChatPayload, HelloPayload, ListPayload, PingPayload, PlaystatePayload, ProtocolMessage,
        ReadyPayload, RoomRef, SetPayload, StatePayload, decode_line, decode_message_line,
        encode_line, encode_message_line, extract_hello, extract_hello_from_message,
    };

    fn fixture_dir() -> PathBuf {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("..");
        path.push("..");
        path.push("fixtures");
        path.push("protocol");
        path
    }

    fn fixture_path(name: &str) -> PathBuf {
        fixture_dir().join(name)
    }

    fn read_fixture(name: &str) -> String {
        fs::read_to_string(fixture_path(name)).expect("fixture file should be readable")
    }

    #[test]
    fn decode_hello_fixture() {
        let fixture = read_fixture("hello_minimal.json");
        let value = decode_line(&fixture).expect("fixture JSON should decode");
        let hello = extract_hello(&value).expect("hello payload should parse");

        assert_eq!(hello.username, "alice");
        assert_eq!(hello.room.name, "room1");
        assert_eq!(hello.version, "1.2.255");
        assert_eq!(hello.realversion.as_deref(), Some("1.7.5"));
        assert_eq!(hello.effective_version(), "1.7.5");
    }

    #[test]
    fn decode_message_hello_fixture() {
        let fixture = read_fixture("hello_minimal.json");
        let message =
            decode_message_line(&fixture).expect("fixture should decode as protocol message");
        let hello = extract_hello_from_message(message).expect("hello message should be extracted");
        assert_eq!(hello.username, "alice");
    }

    #[test]
    fn decode_all_fixtures_as_protocol_messages() {
        let fixture_paths = fs::read_dir(fixture_dir()).expect("fixture directory should exist");
        for entry in fixture_paths {
            let entry = entry.expect("fixture entry should be readable");
            if !entry
                .file_type()
                .expect("file type should be readable")
                .is_file()
            {
                continue;
            }
            let fixture =
                fs::read_to_string(entry.path()).expect("fixture file should be readable");
            let message = decode_message_line(&fixture)
                .expect("each fixture should decode as protocol message");
            assert!(!message.kind().is_empty());
        }
    }

    #[test]
    fn roundtrip_message_fixture() {
        let fixture = read_fixture("state_ping.json");
        let message = decode_message_line(&fixture).expect("state fixture should decode");
        let encoded = encode_message_line(&message).expect("message should encode");
        let decoded = decode_message_line(&encoded).expect("encoded message should decode");
        assert_eq!(message, decoded);
    }

    #[test]
    fn roundtrip_raw_json_value_fixture() {
        let fixture = read_fixture("state_ping.json");
        let value = decode_line(&fixture).expect("fixture JSON should decode");
        let encoded = encode_line(&value).expect("value should encode");
        let decoded = decode_line(&encoded).expect("encoded JSON should decode");
        assert_eq!(value, decoded);
    }

    #[test]
    fn list_request_fixture_decodes_as_request_variant() {
        let fixture = read_fixture("list_request.json");
        let message = decode_message_line(&fixture).expect("list request should decode");
        match message {
            ProtocolMessage::List(payload) => {
                assert!(matches!(payload.list, ListPayload::Request(_)));
            }
            other => panic!("expected List message, found {}", other.kind()),
        }
    }

    #[test]
    fn chat_fixture_supports_text_and_object_variants() {
        let text_message =
            decode_message_line(&read_fixture("chat_text.json")).expect("text chat should decode");
        match text_message {
            ProtocolMessage::Chat(chat) => assert!(matches!(chat.chat, ChatPayload::Text(_))),
            other => panic!("expected Chat message, found {}", other.kind()),
        }

        let object_message = decode_message_line(&read_fixture("chat_message.json"))
            .expect("object chat should decode");
        match object_message {
            ProtocolMessage::Chat(chat) => assert!(matches!(chat.chat, ChatPayload::Message(_))),
            other => panic!("expected Chat message, found {}", other.kind()),
        }
    }

    #[test]
    fn set_fixtures_decode_user_event_variants() {
        let joined_message = decode_message_line(&read_fixture("set_user_joined.json"))
            .expect("set joined fixture should decode");
        match joined_message {
            ProtocolMessage::Set(payload) => {
                let users = payload.set.user.expect("user payload should be present");
                let alice = users.get("alice").expect("alice user entry should exist");
                assert_eq!(
                    alice.room.as_ref().map(|room| room.name.as_str()),
                    Some("room1")
                );
                assert_eq!(alice.event.as_ref(), Some(&json!({"joined": true})));
                assert_eq!(alice.features.as_ref(), Some(&json!({"uiMode": "GUI"})));
                assert_eq!(alice.controller, Some(false));
                assert_eq!(alice.is_ready, Some(true));
            }
            other => panic!("expected Set message, found {}", other.kind()),
        }

        let left_message = decode_message_line(&read_fixture("set_user_left.json"))
            .expect("set left fixture should decode");
        match left_message {
            ProtocolMessage::Set(payload) => {
                let users = payload.set.user.expect("user payload should be present");
                let alice = users.get("alice").expect("alice user entry should exist");
                assert_eq!(alice.event.as_ref(), Some(&json!({"left": true})));
            }
            other => panic!("expected Set message, found {}", other.kind()),
        }
    }

    #[test]
    fn set_fixtures_decode_controller_playlist_and_file_variants() {
        let controller_auth_message =
            decode_message_line(&read_fixture("set_controller_auth_success.json"))
                .expect("controller auth fixture should decode");
        match controller_auth_message {
            ProtocolMessage::Set(payload) => {
                let controller_auth = payload
                    .set
                    .controller_auth
                    .expect("controllerAuth payload should be present");
                assert_eq!(controller_auth.room.as_deref(), Some("room1"));
                assert_eq!(controller_auth.password.as_deref(), Some("secret"));
                assert_eq!(controller_auth.user.as_deref(), Some("alice"));
                assert_eq!(controller_auth.success, Some(true));
            }
            other => panic!("expected Set message, found {}", other.kind()),
        }

        let controlled_room_message =
            decode_message_line(&read_fixture("set_new_controlled_room.json"))
                .expect("new controlled room fixture should decode");
        match controlled_room_message {
            ProtocolMessage::Set(payload) => {
                let room = payload
                    .set
                    .new_controlled_room
                    .expect("newControlledRoom payload should be present");
                assert_eq!(room.room_name.as_deref(), Some("managed-room"));
                assert_eq!(room.password.as_deref(), Some("roompass"));
            }
            other => panic!("expected Set message, found {}", other.kind()),
        }

        let playlist_change_message =
            decode_message_line(&read_fixture("set_playlist_change.json"))
                .expect("playlist change fixture should decode");
        match playlist_change_message {
            ProtocolMessage::Set(payload) => {
                let playlist_change = payload
                    .set
                    .playlist_change
                    .expect("playlistChange payload should be present");
                assert_eq!(
                    playlist_change.files,
                    vec!["episode1.mkv".to_owned(), "episode2.mkv".to_owned()]
                );
                assert_eq!(playlist_change.user.as_deref(), Some("alice"));
            }
            other => panic!("expected Set message, found {}", other.kind()),
        }

        let playlist_index_message = decode_message_line(&read_fixture("set_playlist_index.json"))
            .expect("playlist index fixture should decode");
        match playlist_index_message {
            ProtocolMessage::Set(payload) => {
                let playlist_index = payload
                    .set
                    .playlist_index
                    .expect("playlistIndex payload should be present");
                assert_eq!(playlist_index.index, 1);
                assert_eq!(playlist_index.user.as_deref(), Some("alice"));
            }
            other => panic!("expected Set message, found {}", other.kind()),
        }

        let file_message = decode_message_line(&read_fixture("set_file_full.json"))
            .expect("set file fixture should decode");
        match file_message {
            ProtocolMessage::Set(payload) => {
                let file = payload.set.file.expect("file payload should be present");
                assert_eq!(file.name.as_deref(), Some("movie.mkv"));
                assert_eq!(file.duration, Some(95.5));
                assert_eq!(file.size.as_ref(), Some(&json!(123456789)));
                assert_eq!(file.path.as_deref(), Some("/media/movie.mkv"));
            }
            other => panic!("expected Set message, found {}", other.kind()),
        }

        let features_message = decode_message_line(&read_fixture("set_features_update.json"))
            .expect("set features fixture should decode");
        match features_message {
            ProtocolMessage::Set(payload) => {
                assert_eq!(
                    payload.set.features.as_ref(),
                    Some(&json!({"username":"alice","features":{"chat":true,"readiness":true}}))
                );
            }
            other => panic!("expected Set message, found {}", other.kind()),
        }
    }

    #[test]
    fn state_fixtures_decode_playstate_ping_and_ignore_variants() {
        let playstate_message = decode_message_line(&read_fixture("state_playstate_setby.json"))
            .expect("state playstate fixture should decode");
        match playstate_message {
            ProtocolMessage::State(payload) => {
                let playstate = payload
                    .state
                    .playstate
                    .expect("playstate payload should be present");
                assert_eq!(playstate.position, Some(42.0));
                assert_eq!(playstate.paused, Some(true));
                assert_eq!(playstate.do_seek, Some(true));
                assert_eq!(playstate.set_by.as_deref(), Some("alice"));
            }
            other => panic!("expected State message, found {}", other.kind()),
        }

        let ping_message = decode_message_line(&read_fixture("state_ping_full.json"))
            .expect("state ping full fixture should decode");
        match ping_message {
            ProtocolMessage::State(payload) => {
                let ping = payload.state.ping.expect("ping payload should be present");
                assert_eq!(ping.latency_calculation, Some(173.4));
                assert_eq!(ping.client_latency_calculation, Some(174.1));
                assert_eq!(ping.client_rtt, Some(0.12));
                assert_eq!(ping.server_rtt, Some(0.09));
            }
            other => panic!("expected State message, found {}", other.kind()),
        }

        let ignore_server_message =
            decode_message_line(&read_fixture("state_ignoring_server.json"))
                .expect("state ignoring server fixture should decode");
        match ignore_server_message {
            ProtocolMessage::State(payload) => {
                let ignore = payload
                    .state
                    .ignoring_on_the_fly
                    .expect("ignoringOnTheFly payload should be present");
                assert_eq!(ignore.server, Some(2));
                assert_eq!(ignore.client, None);
            }
            other => panic!("expected State message, found {}", other.kind()),
        }

        let ignore_client_message =
            decode_message_line(&read_fixture("state_ignoring_client.json"))
                .expect("state ignoring client fixture should decode");
        match ignore_client_message {
            ProtocolMessage::State(payload) => {
                let ignore = payload
                    .state
                    .ignoring_on_the_fly
                    .expect("ignoringOnTheFly payload should be present");
                assert_eq!(ignore.server, None);
                assert_eq!(ignore.client, Some(1));
            }
            other => panic!("expected State message, found {}", other.kind()),
        }
    }

    #[test]
    fn hello_constructor_matches_expected_wire_shape() {
        let message = ProtocolMessage::hello(
            HelloPayload::new("alice", "room1", "1.2.255")
                .with_realversion("1.7.5")
                .with_features(json!({"featureList": true})),
        );

        let encoded =
            encode_message_line(&message).expect("constructor-built message should encode");
        let value = decode_line(&encoded).expect("encoded message should be valid JSON");
        assert_eq!(
            value,
            json!({
                "Hello": {
                    "username": "alice",
                    "room": { "name": "room1" },
                    "version": "1.2.255",
                    "realversion": "1.7.5",
                    "features": { "featureList": true }
                }
            })
        );
    }

    #[test]
    fn convenience_constructors_match_common_wire_shapes() {
        let list_value = decode_line(
            &encode_message_line(&ProtocolMessage::list_request())
                .expect("list request message should encode"),
        )
        .expect("list request JSON should decode");
        assert_eq!(list_value, json!({"List": null}));

        let chat_value = decode_line(
            &encode_message_line(&ProtocolMessage::chat_message("alice", "hello everyone"))
                .expect("chat message should encode"),
        )
        .expect("chat JSON should decode");
        assert_eq!(
            chat_value,
            json!({"Chat": {"username": "alice", "message": "hello everyone"}})
        );
    }

    #[test]
    fn set_and_state_builder_messages_roundtrip() {
        let set_message = ProtocolMessage::set(
            SetPayload::new()
                .with_room(RoomRef::new("room1"))
                .with_ready(
                    ReadyPayload::new(true)
                        .with_manually_initiated(true)
                        .with_username("alice"),
                ),
        );
        let set_encoded = encode_message_line(&set_message).expect("set message should encode");
        let set_decoded = decode_message_line(&set_encoded).expect("set message should decode");
        assert_eq!(set_message, set_decoded);

        let state_message = ProtocolMessage::state(
            StatePayload::new()
                .with_ping(
                    PingPayload::new()
                        .with_latency_calculation(1.0)
                        .with_client_latency_calculation(2.0)
                        .with_client_rtt(0.01),
                )
                .with_playstate(
                    PlaystatePayload::new()
                        .with_position(12.5)
                        .with_paused(false)
                        .with_do_seek(false),
                ),
        );
        let state_encoded =
            encode_message_line(&state_message).expect("state message should encode");
        let state_decoded =
            decode_message_line(&state_encoded).expect("state message should decode");
        assert_eq!(state_message, state_decoded);
    }
}
