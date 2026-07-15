use super::*;
use serde_json::Number;
use sorotte_media_match::{MediaMatchWireSignature, media_match_wire_signature_from_value};
use sorotte_protocol::{
    PlaybackBarrierSetExtension, ReadinessSetExtension, SOROTTE_PLAYBACK_BARRIER_V1,
    SOROTTE_READINESS_V2,
};

#[derive(Debug, Clone, PartialEq)]
pub enum ClientCompatibilityFallback {
    IgnoredSetCommand { command: String },
    UsedLegacyFeatureDefaults { context: String },
    IgnoredInvalidFileSize { context: String },
    IgnoredInvalidMediaMatch { context: String, reason: String },
    IgnoredInvalidFeatures { context: String },
    IgnoredInvalidPlaybackBarrier { context: String, reason: String },
    IgnoredInvalidReadinessV2 { context: String, reason: String },
    IgnoredUnexpectedListRequest,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FileSize {
    Number(Number),
    Text(String),
}

impl FileSize {
    pub fn to_json_value(&self) -> Value {
        match self {
            Self::Number(number) => Value::Number(number.clone()),
            Self::Text(text) => Value::String(text.clone()),
        }
    }

    pub fn display_value(&self) -> String {
        match self {
            Self::Number(number) => number.to_string(),
            Self::Text(text) => text.clone(),
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Number(number) => number.as_f64(),
            Self::Text(_) => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Number(number) => number.as_u64(),
            Self::Text(_) => None,
        }
    }
}

impl PartialEq<Value> for FileSize {
    fn eq(&self, other: &Value) -> bool {
        self.to_json_value() == *other
    }
}

impl PartialEq<FileSize> for Value {
    fn eq(&self, other: &FileSize) -> bool {
        *self == other.to_json_value()
    }
}

#[derive(Clone, Copy, Debug)]
pub enum FileDuration {
    Signed(i64),
    Unsigned(u64),
    Float(f64),
}

impl FileDuration {
    fn from_number(number: &serde_json::Number) -> Option<Self> {
        if number.is_i64() {
            number.as_i64().map(Self::Signed)
        } else if number.is_u64() {
            number.as_u64().map(Self::Unsigned)
        } else {
            number
                .as_f64()
                .filter(|value| value.is_finite())
                .map(Self::Float)
        }
    }

    pub fn as_seconds(self) -> f64 {
        match self {
            Self::Signed(value) => value as f64,
            Self::Unsigned(value) => value as f64,
            Self::Float(value) => value,
        }
    }

    pub fn to_json_value(self) -> Value {
        match self {
            Self::Signed(value) => Value::from(value),
            Self::Unsigned(value) => Value::from(value),
            Self::Float(value) => Value::from(value),
        }
    }
}

impl PartialEq for FileDuration {
    fn eq(&self, other: &Self) -> bool {
        self.to_json_value() == other.to_json_value()
    }
}

impl PartialEq<f64> for FileDuration {
    fn eq(&self, other: &f64) -> bool {
        self.as_seconds() == *other
    }
}

#[derive(Clone, Default, PartialEq)]
pub struct SharedFile {
    pub name: Option<String>,
    pub duration: Option<FileDuration>,
    pub size: Option<FileSize>,
    pub media_match: Option<MediaMatchWireSignature>,
    /// Wire fields unknown to this version of the client.
    ///
    /// Retaining these fields keeps a non-empty forward-compatible payload
    /// distinct from the legacy no-file values (`null` and `{}`).
    pub extra: BTreeMap<String, Value>,
}

impl std::fmt::Debug for SharedFile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SharedFile")
            .field(
                "name",
                &self.name.as_ref().map(|_| sorotte_secret::REDACTED_SECRET),
            )
            .field("duration", &self.duration)
            .field("size", &self.size)
            .field("media_match", &self.media_match)
            .field("extra_fields_count", &self.extra.len())
            .finish()
    }
}

impl SharedFile {
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.duration.is_none()
            && self.size.is_none()
            && self.media_match.is_none()
            && self.extra.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PeerCapabilities {
    pub shared_playlists: bool,
    pub chat: bool,
    pub feature_list: bool,
    pub readiness: bool,
    pub managed_rooms: bool,
    pub persistent_rooms: bool,
    pub media_match: bool,
    pub plex_playlist_uris: bool,
    pub remote_readiness: bool,
    pub playback_barrier_v1: bool,
    pub readiness_v2: bool,
    pub ui_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClientHello {
    pub(crate) username: String,
    pub(crate) room: String,
    pub(crate) capabilities: ServerCapabilities,
    pub(crate) max_chat_message_length: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClientUserUpdate {
    pub(crate) username: String,
    pub(crate) room: Option<String>,
    pub(crate) file: Option<SharedFile>,
    pub(crate) left: bool,
    pub(crate) capabilities: Option<PeerCapabilities>,
    pub(crate) controller: Option<bool>,
    pub(crate) ready: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClientListUser {
    pub(crate) file: Option<SharedFile>,
    pub(crate) capabilities: Option<PeerCapabilities>,
    pub(crate) controller: bool,
    pub(crate) ready: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClientControllerAuth {
    pub(crate) room: Option<String>,
    pub(crate) user: Option<String>,
    pub(crate) success: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClientNewControlledRoom {
    pub(crate) room_name: Option<String>,
    pub(crate) password: Option<SecretValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClientReadyUpdate {
    pub(crate) ready: Option<bool>,
    pub(crate) username: Option<String>,
}

#[derive(Clone, PartialEq)]
pub(crate) enum ClientSetCommand {
    Room(String),
    Users(Vec<ClientUserUpdate>),
    ControllerAuth(ClientControllerAuth),
    NewControlledRoom(ClientNewControlledRoom),
    Ready(ClientReadyUpdate),
    PlaylistChange {
        files: Vec<String>,
        user: Option<String>,
    },
    PlaylistIndex {
        index: Option<i64>,
        user: Option<String>,
    },
    Features {
        username: Option<String>,
        capabilities: PeerCapabilities,
    },
    PlaybackBarrier(Box<PlaybackBarrierSetExtension>),
    ReadinessV2(Box<ReadinessSetExtension>),
}

impl std::fmt::Debug for ClientSetCommand {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Room(_) => formatter.write_str("Room(<redacted>)"),
            Self::Users(users) => formatter
                .debug_struct("Users")
                .field("users_count", &users.len())
                .finish(),
            Self::ControllerAuth(command) => formatter
                .debug_tuple("ControllerAuth")
                .field(command)
                .finish(),
            Self::NewControlledRoom(command) => formatter
                .debug_tuple("NewControlledRoom")
                .field(command)
                .finish(),
            Self::Ready(command) => formatter.debug_tuple("Ready").field(command).finish(),
            Self::PlaylistChange { files, user } => formatter
                .debug_struct("PlaylistChange")
                .field("files_count", &files.len())
                .field("has_user", &user.is_some())
                .finish(),
            Self::PlaylistIndex { index, user } => formatter
                .debug_struct("PlaylistIndex")
                .field("index", index)
                .field("has_user", &user.is_some())
                .finish(),
            Self::Features {
                username,
                capabilities,
            } => formatter
                .debug_struct("Features")
                .field("has_username", &username.is_some())
                .field("capabilities", capabilities)
                .finish(),
            Self::PlaybackBarrier(extension) => formatter
                .debug_tuple("PlaybackBarrier")
                .field(extension)
                .finish(),
            Self::ReadinessV2(extension) => formatter
                .debug_tuple("ReadinessV2")
                .field(extension)
                .finish(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClientPlaystate {
    pub(crate) position: Option<f64>,
    pub(crate) paused: Option<bool>,
    pub(crate) do_seek: Option<bool>,
    pub(crate) set_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct ClientPing {
    pub(crate) latency_calculation: Option<f64>,
    pub(crate) client_latency_calculation: Option<f64>,
    pub(crate) client_rtt: Option<f64>,
    pub(crate) server_rtt: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct ClientIgnoringOnTheFly {
    pub(crate) server: Option<u32>,
    pub(crate) client: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct ClientStateUpdate {
    pub(crate) playstate: Option<ClientPlaystate>,
    pub(crate) ping: Option<ClientPing>,
    pub(crate) ignoring_on_the_fly: Option<ClientIgnoringOnTheFly>,
}

#[derive(Clone, PartialEq)]
pub(crate) enum ClientInboundCommand {
    Hello(ClientHello),
    Set(Vec<ClientSetCommand>),
    List(BTreeMap<String, BTreeMap<String, ClientListUser>>),
    State(ClientStateUpdate),
    Chat(ChatNotification),
    ServerError(String),
    UnexpectedTls(String),
    Ignore,
}

impl std::fmt::Debug for ClientInboundCommand {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Hello(value) => formatter.debug_tuple("Hello").field(value).finish(),
            Self::Set(value) => formatter.debug_tuple("Set").field(value).finish(),
            Self::List(value) => formatter.debug_tuple("List").field(value).finish(),
            Self::State(value) => formatter.debug_tuple("State").field(value).finish(),
            Self::Chat(value) => formatter.debug_tuple("Chat").field(value).finish(),
            Self::ServerError(_) => formatter
                .debug_tuple("ServerError")
                .field(&sorotte_secret::REDACTED_SECRET)
                .finish(),
            Self::UnexpectedTls(value) => {
                formatter.debug_tuple("UnexpectedTls").field(value).finish()
            }
            Self::Ignore => formatter.write_str("Ignore"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NormalizedClientInbound {
    pub(crate) command: ClientInboundCommand,
    pub(crate) fallbacks: Vec<ClientCompatibilityFallback>,
}

fn feature_bool(features: Option<&Value>, name: &str) -> Option<bool> {
    features
        .and_then(Value::as_object)
        .and_then(|features| features.get(name))
        .and_then(Value::as_bool)
}

fn feature_usize(features: Option<&Value>, name: &str) -> Option<usize> {
    features
        .and_then(Value::as_object)
        .and_then(|features| features.get(name))
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn peer_capabilities(value: &Value) -> Option<PeerCapabilities> {
    let features = value.as_object()?;
    Some(PeerCapabilities {
        shared_playlists: features
            .get("sharedPlaylists")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        chat: features
            .get("chat")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        feature_list: features
            .get("featureList")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        readiness: features
            .get("readiness")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        managed_rooms: features
            .get("managedRooms")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        persistent_rooms: features
            .get("persistentRooms")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        media_match: features
            .get("mediaMatch")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        plex_playlist_uris: features
            .get(SOROTTE_PLEX_PLAYLIST_URIS_FEATURE)
            .and_then(Value::as_bool)
            .unwrap_or(false),
        remote_readiness: features
            .get("setOthersReadiness")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        playback_barrier_v1: features
            .get(SOROTTE_PLAYBACK_BARRIER_V1)
            .and_then(Value::as_bool)
            .unwrap_or(false),
        readiness_v2: features
            .get(SOROTTE_READINESS_V2)
            .and_then(Value::as_bool)
            .unwrap_or(false),
        ui_mode: features
            .get("uiMode")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

fn normalize_size(
    value: Option<Value>,
    context: &str,
    fallbacks: &mut Vec<ClientCompatibilityFallback>,
) -> Option<FileSize> {
    match value {
        Some(Value::Number(number)) => Some(FileSize::Number(number)),
        Some(Value::String(text)) => Some(FileSize::Text(text)),
        Some(Value::Null) | None => None,
        Some(_) => {
            fallbacks.push(ClientCompatibilityFallback::IgnoredInvalidFileSize {
                context: context.to_owned(),
            });
            None
        }
    }
}

fn normalize_media_match(
    value: Option<Value>,
    context: &str,
    fallbacks: &mut Vec<ClientCompatibilityFallback>,
) -> Option<MediaMatchWireSignature> {
    let value = value?;
    match media_match_wire_signature_from_value(&value) {
        Ok(signature) => Some(signature),
        Err(reason) => {
            fallbacks.push(ClientCompatibilityFallback::IgnoredInvalidMediaMatch {
                context: context.to_owned(),
                reason,
            });
            None
        }
    }
}

fn normalize_file_value(
    value: Value,
    context: &str,
    fallbacks: &mut Vec<ClientCompatibilityFallback>,
) -> Option<SharedFile> {
    match value {
        Value::Null => None,
        Value::String(name) if !name.is_empty() => Some(SharedFile {
            name: Some(name),
            ..SharedFile::default()
        }),
        Value::Object(mut fields) => {
            let was_nonempty = !fields.is_empty();
            let file = SharedFile {
                name: fields
                    .remove("name")
                    .and_then(|value| value.as_str().map(str::to_owned)),
                duration: fields.remove("duration").and_then(|value| match value {
                    Value::Number(number) => FileDuration::from_number(&number),
                    _ => None,
                }),
                size: normalize_size(fields.remove("size"), context, fallbacks),
                media_match: normalize_media_match(
                    fields.remove(MEDIA_MATCH_FILE_PAYLOAD_KEY),
                    context,
                    fallbacks,
                ),
                extra: fields.into_iter().collect(),
            };
            // Legacy clients use the raw object's truthiness for presence.
            // Keep that decision separate from whether this version could
            // normalize any of the object's metadata.
            was_nonempty.then_some(file)
        }
        _ => None,
    }
}

fn ordered_set_commands(set: SetPayload) -> Vec<(String, SetPayload)> {
    let mut order = set.command_order.clone();
    for command in [
        "room",
        "file",
        "user",
        "controllerAuth",
        "newControlledRoom",
        "ready",
        "playlistChange",
        "playlistIndex",
        "features",
        SOROTTE_PLAYBACK_BARRIER_V1,
        SOROTTE_READINESS_V2,
    ] {
        if !order.iter().any(|candidate| candidate == command) {
            order.push(command.to_owned());
        }
    }
    order
        .into_iter()
        .map(|command| (command, set.clone()))
        .collect()
}

pub(crate) fn normalize_client_protocol_message(
    message: ProtocolMessage,
) -> NormalizedClientInbound {
    let mut fallbacks = Vec::new();
    let command = match message {
        ProtocolMessage::Hello(message) => {
            let hello = message.hello;
            let server_version = hello.effective_version().to_owned();
            if !hello.features.as_ref().is_some_and(Value::is_object) {
                fallbacks.push(ClientCompatibilityFallback::UsedLegacyFeatureDefaults {
                    context: "Hello.features".to_owned(),
                });
            }
            let capabilities = ServerCapabilities {
                readiness: feature_bool(hello.features.as_ref(), "readiness").unwrap_or_else(
                    || {
                        ClientSession::meets_min_version_legacy_compatible(
                            &server_version,
                            LEGACY_USER_READY_MIN_VERSION,
                        )
                    },
                ),
                remote_readiness: feature_bool(hello.features.as_ref(), "setOthersReadiness")
                    .unwrap_or_else(|| {
                        ClientSession::meets_min_version_legacy_compatible(
                            &server_version,
                            LEGACY_SET_OTHERS_READINESS_MIN_VERSION,
                        )
                    }),
                managed_rooms: feature_bool(hello.features.as_ref(), "managedRooms")
                    .unwrap_or_else(|| {
                        ClientSession::meets_min_version_legacy_compatible(
                            &server_version,
                            LEGACY_MANAGED_ROOMS_MIN_VERSION,
                        )
                    }),
                shared_playlists: feature_bool(hello.features.as_ref(), "sharedPlaylists")
                    .unwrap_or_else(|| {
                        ClientSession::meets_min_version_legacy_compatible(
                            &server_version,
                            LEGACY_SHARED_PLAYLIST_MIN_VERSION,
                        )
                    }),
                media_match: feature_bool(hello.features.as_ref(), "mediaMatch").unwrap_or(false),
                chat: feature_bool(hello.features.as_ref(), "chat").unwrap_or_else(|| {
                    ClientSession::meets_min_version_legacy_compatible(
                        &server_version,
                        LEGACY_CHAT_MIN_VERSION,
                    )
                }),
                plex_playlist_uris: feature_bool(
                    hello.features.as_ref(),
                    SOROTTE_PLEX_PLAYLIST_URIS_FEATURE,
                )
                .unwrap_or(false),
                playback_barrier_v1: feature_bool(
                    hello.features.as_ref(),
                    SOROTTE_PLAYBACK_BARRIER_V1,
                )
                .unwrap_or(false),
                readiness_v2: feature_bool(hello.features.as_ref(), SOROTTE_READINESS_V2)
                    .unwrap_or(false),
                persistent_rooms: feature_bool(hello.features.as_ref(), "persistentRooms")
                    .unwrap_or(false),
                max_username_length: feature_usize(hello.features.as_ref(), "maxUsernameLength")
                    .unwrap_or(LEGACY_FALLBACK_MAX_USERNAME_LENGTH),
                max_room_name_length: feature_usize(hello.features.as_ref(), "maxRoomNameLength")
                    .unwrap_or(LEGACY_FALLBACK_MAX_ROOM_NAME_LENGTH),
                max_filename_length: feature_usize(hello.features.as_ref(), "maxFilenameLength")
                    .unwrap_or(LEGACY_FALLBACK_MAX_FILENAME_LENGTH),
            };
            ClientInboundCommand::Hello(ClientHello {
                username: hello.username,
                room: hello.room.name,
                capabilities,
                max_chat_message_length: feature_usize(
                    hello.features.as_ref(),
                    "maxChatMessageLength",
                )
                .unwrap_or(LEGACY_FALLBACK_MAX_CHAT_MESSAGE_LENGTH),
            })
        }
        ProtocolMessage::Set(message) => {
            let mut playback_barrier = match message.set.playback_barrier_v1() {
                Ok(extension) => extension,
                Err(reason) => {
                    fallbacks.push(ClientCompatibilityFallback::IgnoredInvalidPlaybackBarrier {
                        context: "Set.sorottePlaybackBarrierV1".to_owned(),
                        reason: reason.to_string(),
                    });
                    None
                }
            };
            let mut readiness_v2 = match message.set.readiness_v2() {
                Ok(extension) => extension,
                Err(reason) => {
                    fallbacks.push(ClientCompatibilityFallback::IgnoredInvalidReadinessV2 {
                        context: "Set.sorotteReadinessV2".to_owned(),
                        reason: reason.to_string(),
                    });
                    None
                }
            };
            let mut commands = Vec::new();
            for (name, mut set) in ordered_set_commands(message.set) {
                let command =
                    match name.as_str() {
                        "room" => set
                            .room
                            .take()
                            .map(|room| ClientSetCommand::Room(room.name)),
                        "file" => set.file.take().and_then(|_| {
                            fallbacks.push(ClientCompatibilityFallback::IgnoredSetCommand {
                                command: "file".to_owned(),
                            });
                            None
                        }),
                        "user" => set.user.take().map(|users| {
                            let updates = users
                                .into_iter()
                                .map(|(username, user)| {
                                    let context = format!("Set.user.{username}.file");
                                    let file = user.file.and_then(|file| {
                                        normalize_file_value(file, &context, &mut fallbacks)
                                    });
                                    let capabilities = user.features.and_then(|features| {
                                        peer_capabilities(&features).or_else(|| {
                                            fallbacks.push(
                                            ClientCompatibilityFallback::IgnoredInvalidFeatures {
                                                context: format!("Set.user.{username}.features"),
                                            },
                                        );
                                            None
                                        })
                                    });
                                    ClientUserUpdate {
                                        username,
                                        room: user.room.map(|room| room.name),
                                        file,
                                        left: user
                                            .event
                                            .as_ref()
                                            .and_then(|event| event.get("left"))
                                            .and_then(Value::as_bool)
                                            == Some(true),
                                        capabilities,
                                        controller: user.controller,
                                        ready: user.is_ready,
                                    }
                                })
                                .collect();
                            ClientSetCommand::Users(updates)
                        }),
                        "controllerAuth" => set.controller_auth.take().map(|auth| {
                            ClientSetCommand::ControllerAuth(ClientControllerAuth {
                                room: auth.room,
                                user: auth.user,
                                success: auth.success,
                            })
                        }),
                        "newControlledRoom" => set.new_controlled_room.take().map(|room| {
                            ClientSetCommand::NewControlledRoom(ClientNewControlledRoom {
                                room_name: room.room_name,
                                password: room.password,
                            })
                        }),
                        "ready" => set.ready.take().map(|ready| {
                            ClientSetCommand::Ready(ClientReadyUpdate {
                                ready: ready.is_ready,
                                username: ready.username,
                            })
                        }),
                        "playlistChange" => set.playlist_change.take().map(|playlist| {
                            ClientSetCommand::PlaylistChange {
                                files: canonical_playlist_files_from_change(&playlist),
                                user: playlist.user,
                            }
                        }),
                        "playlistIndex" => set.playlist_index.take().map(|playlist| {
                            ClientSetCommand::PlaylistIndex {
                                index: playlist.index_value(),
                                user: playlist.user,
                            }
                        }),
                        "features" => set.features.take().and_then(|features| {
                            let username = features
                                .get("username")
                                .and_then(Value::as_str)
                                .map(str::to_owned);
                            let feature_value = features.get("features").unwrap_or(&features);
                            peer_capabilities(feature_value)
                                .map(|capabilities| ClientSetCommand::Features {
                                    username,
                                    capabilities,
                                })
                                .or_else(|| {
                                    fallbacks.push(
                                        ClientCompatibilityFallback::IgnoredInvalidFeatures {
                                            context: "Set.features".to_owned(),
                                        },
                                    );
                                    None
                                })
                        }),
                        SOROTTE_PLAYBACK_BARRIER_V1 => playback_barrier
                            .take()
                            .map(Box::new)
                            .map(ClientSetCommand::PlaybackBarrier),
                        SOROTTE_READINESS_V2 => readiness_v2
                            .take()
                            .map(Box::new)
                            .map(ClientSetCommand::ReadinessV2),
                        _ => {
                            if set.extra.contains_key(&name) {
                                fallbacks.push(ClientCompatibilityFallback::IgnoredSetCommand {
                                    command: name,
                                });
                            }
                            None
                        }
                    };
                if let Some(command) = command {
                    commands.push(command);
                }
            }
            ClientInboundCommand::Set(commands)
        }
        ProtocolMessage::List(message) => match message.list {
            ListPayload::Request(_) => {
                fallbacks.push(ClientCompatibilityFallback::IgnoredUnexpectedListRequest);
                ClientInboundCommand::Ignore
            }
            ListPayload::Rooms(rooms) => {
                let rooms = rooms
                    .into_iter()
                    .map(|(room, users)| {
                        let users = users
                            .into_iter()
                            .map(|(username, user)| {
                                let context = format!("List.{room}.{username}.file");
                                let file = user.file.and_then(|file| {
                                    normalize_file_value(file, &context, &mut fallbacks)
                                });
                                let capabilities = user.features.and_then(|features| {
                                    peer_capabilities(&features).or_else(|| {
                                        fallbacks.push(
                                            ClientCompatibilityFallback::IgnoredInvalidFeatures {
                                                context: format!("List.{room}.{username}.features"),
                                            },
                                        );
                                        None
                                    })
                                });
                                (
                                    username,
                                    ClientListUser {
                                        file,
                                        capabilities,
                                        controller: user.controller.unwrap_or(false),
                                        ready: user.is_ready,
                                    },
                                )
                            })
                            .collect();
                        (room, users)
                    })
                    .collect();
                ClientInboundCommand::List(rooms)
            }
        },
        ProtocolMessage::State(message) => {
            ClientInboundCommand::State(normalize_client_state_payload(message.state))
        }
        ProtocolMessage::Chat(message) => ClientInboundCommand::Chat(match message.chat {
            ChatPayload::Text(message) => ChatNotification::Message {
                username: None,
                message,
            },
            ChatPayload::Message(message) => ChatNotification::Message {
                username: Some(message.username),
                message: message.message,
            },
        }),
        ProtocolMessage::Error(message) => ClientInboundCommand::ServerError(message.error.message),
        ProtocolMessage::Tls(message) => ClientInboundCommand::UnexpectedTls(message.tls.start_tls),
    };
    NormalizedClientInbound { command, fallbacks }
}

pub(crate) fn normalize_client_state_payload(state: StatePayload) -> ClientStateUpdate {
    ClientStateUpdate {
        playstate: state.playstate.map(|playstate| ClientPlaystate {
            position: playstate.position,
            paused: playstate.paused,
            do_seek: playstate.do_seek,
            set_by: playstate.set_by,
        }),
        ping: state.ping.map(|ping| ClientPing {
            latency_calculation: ping.latency_calculation,
            client_latency_calculation: ping.client_latency_calculation,
            client_rtt: ping.client_rtt,
            server_rtt: ping.server_rtt,
        }),
        ignoring_on_the_fly: state
            .ignoring_on_the_fly
            .map(|ignoring| ClientIgnoringOnTheFly {
                server: ignoring.server,
                client: ignoring.client,
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_new_controlled_room_debug_redacts_password() {
        let message = ProtocolMessage::set(
            SetPayload::new().with_new_controlled_room(
                sorotte_protocol::NewControlledRoomPayload::new()
                    .with_room_name("+room:ABCDEF123456")
                    .with_password("normalized-client-secret"),
            ),
        );

        let debug = format!("{:?}", normalize_client_protocol_message(message));

        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("normalized-client-secret"));
    }

    #[test]
    fn normalized_server_error_debug_redacts_whitespace_reflected_password() {
        const MARKER: &str = "normalized-reflected-password-canary";
        let message =
            ProtocolMessage::error_message(format!(r#"Not JSON: {{"password" : "{MARKER}"}}"#));

        let debug = format!("{:?}", normalize_client_protocol_message(message));

        assert!(debug.contains(sorotte_secret::REDACTED_SECRET));
        assert!(!debug.contains(MARKER));
    }
}
