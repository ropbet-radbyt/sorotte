use super::*;
use sorotte_media_match::MediaMatchWireSignature;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerCompatibilityFallback {
    IgnoredSetCommand { command: String },
    UsedLegacyFeatureDefaults { context: String },
    IgnoredInvalidFileSize { context: String },
    IgnoredInvalidMediaMatch { context: String, reason: String },
    IgnoredInvalidFeatures { context: String },
    IgnoredInvalidPlaybackBarrier { context: String, reason: String },
    IgnoredInvalidReadiness { context: String, reason: String },
    IgnoredUnexpectedMessage { command: &'static str },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ServerFileSize {
    Number(serde_json::Number),
    Text(String),
}

impl ServerFileSize {
    fn to_value(&self) -> Value {
        match self {
            Self::Number(number) => Value::Number(number.clone()),
            Self::Text(text) => Value::String(text.clone()),
        }
    }
}

#[derive(Clone, PartialEq, Default)]
pub(crate) struct ServerSharedFile {
    pub(crate) name: Option<String>,
    pub(crate) duration: Option<f64>,
    pub(crate) size: Option<ServerFileSize>,
    pub(crate) media_match: Option<MediaMatchWireSignature>,
    pub(crate) extra: BTreeMap<String, Value>,
}

impl std::fmt::Debug for ServerSharedFile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServerSharedFile")
            .field(
                "name",
                &self.name.as_ref().map(|_| sorotte_secret::REDACTED_SECRET),
            )
            .field("duration", &self.duration)
            .field("has_size", &self.size.is_some())
            .field("media_match", &self.media_match)
            .field("extra_fields_count", &self.extra.len())
            .finish()
    }
}

impl ServerSharedFile {
    pub(crate) fn to_wire_value(&self, include_media_match: bool) -> Value {
        let mut fields: serde_json::Map<String, Value> = self.extra.clone().into_iter().collect();
        if let Some(name) = &self.name {
            fields.insert("name".to_owned(), Value::String(name.clone()));
        }
        if let Some(duration) = self.duration {
            fields.insert("duration".to_owned(), Value::from(duration));
        }
        if let Some(size) = &self.size {
            fields.insert("size".to_owned(), size.to_value());
        }
        if include_media_match
            && let Some(signature) = &self.media_match
            && let Ok(value) = serde_json::to_value(signature)
        {
            fields.insert("mediaMatch".to_owned(), value);
        }
        Value::Object(fields)
    }

    fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.duration.is_none()
            && self.size.is_none()
            && self.media_match.is_none()
            && self.extra.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerClientCapabilities {
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
    pub ui_mode_advertised: bool,
    pub(crate) advertised_fields: BTreeSet<&'static str>,
}

impl ServerClientCapabilities {
    pub(crate) fn to_wire_value(&self) -> Value {
        let mut features = serde_json::Map::new();
        for (name, enabled) in [
            ("sharedPlaylists", self.shared_playlists),
            ("chat", self.chat),
            ("featureList", self.feature_list),
            ("readiness", self.readiness),
            ("managedRooms", self.managed_rooms),
            ("persistentRooms", self.persistent_rooms),
            ("mediaMatch", self.media_match),
            (SOROTTE_PLEX_PLAYLIST_URIS_FEATURE, self.plex_playlist_uris),
            ("setOthersReadiness", self.remote_readiness),
            (SOROTTE_PLAYBACK_BARRIER_V1, self.playback_barrier_v1),
            (SOROTTE_READINESS_V2, self.readiness_v2),
        ] {
            if self.advertised_fields.contains(name) {
                features.insert(name.to_owned(), Value::Bool(enabled));
            }
        }
        if self.ui_mode_advertised
            && let Some(ui_mode) = &self.ui_mode
        {
            features.insert("uiMode".to_owned(), Value::String(ui_mode.clone()));
        }
        Value::Object(features)
    }

    pub(crate) fn is_gui_user(&self) -> bool {
        matches!(
            self.ui_mode.as_deref(),
            None | Some(LEGACY_UI_MODE_UNKNOWN) | Some(LEGACY_UI_MODE_GRAPHICAL)
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ServerHelloCommand {
    pub(crate) username: String,
    pub(crate) room: String,
    pub(crate) version: String,
    pub(crate) capabilities: ServerClientCapabilities,
    pub(crate) password_token: Option<SecretValue>,
}

#[derive(Clone, PartialEq)]
pub(crate) enum ServerSetCommand {
    Room(String),
    File(Option<ServerSharedFile>),
    ControllerAuth {
        room: Option<String>,
        password: SecretValue,
    },
    Ready {
        ready: bool,
        manually_initiated: bool,
        username: Option<String>,
        set_by: Option<String>,
    },
    PlaylistChange(Vec<String>),
    PlaylistIndex(Option<i64>),
    Features(ServerClientCapabilities),
    PlaybackBarrier(Box<PlaybackBarrierSetExtension>),
    Readiness(Box<ReadinessSetExtension>),
}

impl std::fmt::Debug for ServerSetCommand {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Room(_) => formatter.write_str("Room(<redacted>)"),
            Self::File(file) => formatter.debug_tuple("File").field(file).finish(),
            Self::ControllerAuth { password, .. } => formatter
                .debug_struct("ControllerAuth")
                .field("room", &sorotte_secret::REDACTED_SECRET)
                .field("password", password)
                .finish(),
            Self::Ready {
                ready,
                manually_initiated,
                username,
                set_by,
            } => formatter
                .debug_struct("Ready")
                .field("ready", ready)
                .field("manually_initiated", manually_initiated)
                .field("has_username", &username.is_some())
                .field("has_set_by", &set_by.is_some())
                .finish(),
            Self::PlaylistChange(files) => formatter
                .debug_struct("PlaylistChange")
                .field("files_count", &files.len())
                .finish(),
            Self::PlaylistIndex(index) => {
                formatter.debug_tuple("PlaylistIndex").field(index).finish()
            }
            Self::Features(capabilities) => formatter
                .debug_tuple("Features")
                .field(capabilities)
                .finish(),
            Self::PlaybackBarrier(extension) => formatter
                .debug_tuple("PlaybackBarrier")
                .field(extension)
                .finish(),
            Self::Readiness(extension) => {
                formatter.debug_tuple("Readiness").field(extension).finish()
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct ServerPlaystateCommand {
    pub(crate) position: Option<f64>,
    pub(crate) paused: Option<bool>,
    pub(crate) do_seek: Option<bool>,
    pub(crate) set_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct ServerPingCommand {
    pub(crate) latency_calculation: Option<f64>,
    pub(crate) client_latency_calculation: Option<f64>,
    pub(crate) client_rtt: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct ServerStateCommand {
    pub(crate) playstate: Option<ServerPlaystateCommand>,
    pub(crate) ping: Option<ServerPingCommand>,
    pub(crate) server_ignoring: Option<u32>,
    pub(crate) client_ignoring: Option<u32>,
    pub(crate) playback_barrier: Option<PlaybackBarrierStateExtension>,
    pub(crate) readiness: Option<ReadinessStateExtension>,
}

#[derive(Clone, PartialEq)]
pub(crate) enum ServerInboundCommand {
    Hello(ServerHelloCommand),
    Set(Vec<ServerSetCommand>),
    ListRequest,
    State(ServerStateCommand),
    Tls(String),
    Chat(String),
    Ignore,
}

impl std::fmt::Debug for ServerInboundCommand {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Hello(command) => formatter.debug_tuple("Hello").field(command).finish(),
            Self::Set(commands) => formatter.debug_tuple("Set").field(commands).finish(),
            Self::ListRequest => formatter.write_str("ListRequest"),
            Self::State(command) => formatter.debug_tuple("State").field(command).finish(),
            Self::Tls(_) => formatter.write_str("Tls(<redacted>)"),
            Self::Chat(message) => formatter
                .debug_struct("Chat")
                .field("message_bytes", &message.len())
                .finish(),
            Self::Ignore => formatter.write_str("Ignore"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NormalizedServerInbound {
    pub(crate) command: ServerInboundCommand,
    pub(crate) fallbacks: Vec<ServerCompatibilityFallback>,
}

fn bool_feature(features: &serde_json::Map<String, Value>, name: &str) -> bool {
    features.get(name).and_then(Value::as_bool).unwrap_or(false)
}

fn legacy_capabilities(version: &str) -> ServerClientCapabilities {
    ServerClientCapabilities {
        shared_playlists: client_version_meets_minimum(version, LEGACY_SHARED_PLAYLIST_MIN_VERSION),
        chat: client_version_meets_minimum(version, LEGACY_CHAT_MIN_VERSION),
        feature_list: false,
        readiness: client_version_meets_minimum(version, LEGACY_USER_READY_MIN_VERSION),
        managed_rooms: client_version_meets_minimum(version, LEGACY_CONTROLLED_ROOMS_MIN_VERSION),
        persistent_rooms: false,
        media_match: false,
        plex_playlist_uris: false,
        remote_readiness: false,
        playback_barrier_v1: false,
        readiness_v2: false,
        ui_mode: Some(LEGACY_UI_MODE_UNKNOWN.to_owned()),
        ui_mode_advertised: true,
        advertised_fields: BTreeSet::from([
            "sharedPlaylists",
            "chat",
            "featureList",
            "readiness",
            "managedRooms",
            "persistentRooms",
        ]),
    }
}

fn normalize_capabilities(
    value: Option<Value>,
    version: &str,
    context: &str,
    fallbacks: &mut Vec<ServerCompatibilityFallback>,
) -> ServerClientCapabilities {
    let Some(Value::Object(features)) = value else {
        fallbacks.push(ServerCompatibilityFallback::UsedLegacyFeatureDefaults {
            context: context.to_owned(),
        });
        return legacy_capabilities(version);
    };
    if features.is_empty() {
        fallbacks.push(ServerCompatibilityFallback::UsedLegacyFeatureDefaults {
            context: context.to_owned(),
        });
        return legacy_capabilities(version);
    }
    capabilities_from_object(features)
}

fn capabilities_from_object(features: serde_json::Map<String, Value>) -> ServerClientCapabilities {
    let ui_mode_advertised = features.contains_key("uiMode");
    let advertised_fields = [
        "sharedPlaylists",
        "chat",
        "featureList",
        "readiness",
        "managedRooms",
        "persistentRooms",
        "mediaMatch",
        SOROTTE_PLEX_PLAYLIST_URIS_FEATURE,
        "setOthersReadiness",
        SOROTTE_PLAYBACK_BARRIER_V1,
        SOROTTE_READINESS_V2,
    ]
    .into_iter()
    .filter(|name| features.contains_key(*name))
    .collect();
    ServerClientCapabilities {
        shared_playlists: bool_feature(&features, "sharedPlaylists"),
        chat: bool_feature(&features, "chat"),
        feature_list: bool_feature(&features, "featureList"),
        readiness: bool_feature(&features, "readiness"),
        managed_rooms: bool_feature(&features, "managedRooms"),
        persistent_rooms: bool_feature(&features, "persistentRooms"),
        media_match: bool_feature(&features, "mediaMatch"),
        plex_playlist_uris: bool_feature(&features, SOROTTE_PLEX_PLAYLIST_URIS_FEATURE),
        remote_readiness: bool_feature(&features, "setOthersReadiness"),
        playback_barrier_v1: bool_feature(&features, SOROTTE_PLAYBACK_BARRIER_V1),
        readiness_v2: bool_feature(&features, SOROTTE_READINESS_V2),
        ui_mode: features
            .get("uiMode")
            .and_then(Value::as_str)
            .map(str::to_owned),
        ui_mode_advertised,
        advertised_fields,
    }
}

fn normalize_file(
    file: FilePayload,
    context: &str,
    fallbacks: &mut Vec<ServerCompatibilityFallback>,
) -> Option<ServerSharedFile> {
    let size = match file.size {
        Some(Value::Number(number)) => Some(ServerFileSize::Number(number)),
        Some(Value::String(text)) => Some(ServerFileSize::Text(text)),
        Some(Value::Null) | None => None,
        Some(_) => {
            fallbacks.push(ServerCompatibilityFallback::IgnoredInvalidFileSize {
                context: context.to_owned(),
            });
            None
        }
    };
    let mut extra = file.extra;
    let media_match = extra.remove("mediaMatch").and_then(|value| {
        match serde_json::from_value::<MediaMatchWireSignature>(value) {
            Ok(signature) => Some(signature),
            Err(reason) => {
                fallbacks.push(ServerCompatibilityFallback::IgnoredInvalidMediaMatch {
                    context: context.to_owned(),
                    reason: reason.to_string(),
                });
                None
            }
        }
    });
    let file = ServerSharedFile {
        name: file
            .name
            .map(|name| truncate_text_to_max_chars(&name, DEFAULT_MAX_FILENAME_LENGTH)),
        duration: file.duration.filter(|value| value.is_finite()),
        size,
        media_match,
        extra,
    };
    (!file.is_empty()).then_some(file)
}

pub(crate) fn normalize_server_protocol_message(
    message: ProtocolMessage,
) -> NormalizedServerInbound {
    let mut fallbacks = Vec::new();
    let command = match message {
        ProtocolMessage::Hello(message) => {
            let hello = message.hello;
            let version = hello.effective_version().to_owned();
            let capabilities =
                normalize_capabilities(hello.features, &version, "Hello.features", &mut fallbacks);
            let password_token = hello
                .extra
                .get("password")
                .and_then(Value::as_str)
                .map(SecretValue::from);
            ServerInboundCommand::Hello(ServerHelloCommand {
                username: hello.username,
                room: hello.room.name,
                version,
                capabilities,
                password_token,
            })
        }
        ProtocolMessage::Set(message) => {
            let mut set = message.set;
            let mut playback_barrier = match set.playback_barrier_v1() {
                Ok(extension) => extension,
                Err(reason) => {
                    fallbacks.push(ServerCompatibilityFallback::IgnoredInvalidPlaybackBarrier {
                        context: "Set.sorottePlaybackBarrierV1".to_owned(),
                        reason: reason.to_string(),
                    });
                    None
                }
            };
            let mut readiness = match set.readiness_v2() {
                Ok(extension) => extension,
                Err(reason) => {
                    fallbacks.push(ServerCompatibilityFallback::IgnoredInvalidReadiness {
                        context: "Set.sorotteReadinessV2".to_owned(),
                        reason: reason.to_string(),
                    });
                    None
                }
            };
            let mut order = set.command_order.clone();
            for command in [
                "room",
                "file",
                "controllerAuth",
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
            let mut commands = Vec::new();
            for name in order {
                let command = match name.as_str() {
                    "room" => set
                        .room
                        .take()
                        .map(|room| ServerSetCommand::Room(room.name)),
                    "file" => set.file.take().map(|file| {
                        ServerSetCommand::File(normalize_file(file, "Set.file", &mut fallbacks))
                    }),
                    "controllerAuth" => {
                        set.controller_auth
                            .take()
                            .map(|auth| ServerSetCommand::ControllerAuth {
                                room: auth.room,
                                password: auth.password.unwrap_or_default(),
                            })
                    }
                    "ready" => set.ready.take().and_then(|ready| {
                        ready.is_ready.map(|is_ready| ServerSetCommand::Ready {
                            ready: is_ready,
                            manually_initiated: ready.manually_initiated.unwrap_or(false),
                            username: ready.username,
                            set_by: ready.set_by,
                        })
                    }),
                    "playlistChange" => set.playlist_change.take().map(|playlist| {
                        ServerSetCommand::PlaylistChange(canonical_playlist_files_from_change(
                            &playlist,
                        ))
                    }),
                    "playlistIndex" => set
                        .playlist_index
                        .take()
                        .map(|playlist| ServerSetCommand::PlaylistIndex(playlist.index_value())),
                    "features" => set.features.take().and_then(|features| match features {
                        Value::Object(features) => Some(ServerSetCommand::Features(
                            capabilities_from_object(features),
                        )),
                        _ => {
                            fallbacks.push(ServerCompatibilityFallback::IgnoredInvalidFeatures {
                                context: "Set.features".to_owned(),
                            });
                            None
                        }
                    }),
                    SOROTTE_PLAYBACK_BARRIER_V1 => playback_barrier
                        .take()
                        .map(Box::new)
                        .map(ServerSetCommand::PlaybackBarrier),
                    SOROTTE_READINESS_V2 => readiness
                        .take()
                        .map(Box::new)
                        .map(ServerSetCommand::Readiness),
                    _ => {
                        if set.extra.contains_key(&name)
                            || matches!(name.as_str(), "user" | "newControlledRoom")
                        {
                            fallbacks.push(ServerCompatibilityFallback::IgnoredSetCommand {
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
            ServerInboundCommand::Set(commands)
        }
        ProtocolMessage::List(message) => match message.list {
            ListPayload::Request(_) => ServerInboundCommand::ListRequest,
            ListPayload::Rooms(_) => {
                fallbacks.push(ServerCompatibilityFallback::IgnoredUnexpectedMessage {
                    command: "List snapshot",
                });
                ServerInboundCommand::Ignore
            }
        },
        ProtocolMessage::State(message) => {
            let state = message.state;
            let playback_barrier = match state.playback_barrier_v1() {
                Ok(extension) => extension,
                Err(reason) => {
                    fallbacks.push(ServerCompatibilityFallback::IgnoredInvalidPlaybackBarrier {
                        context: "State.sorottePlaybackBarrierV1".to_owned(),
                        reason: reason.to_string(),
                    });
                    None
                }
            };
            let readiness = match state.readiness_v2() {
                Ok(extension) => extension,
                Err(reason) => {
                    fallbacks.push(ServerCompatibilityFallback::IgnoredInvalidReadiness {
                        context: "State.sorotteReadinessV2".to_owned(),
                        reason: reason.to_string(),
                    });
                    None
                }
            };
            let ignoring = state.ignoring_on_the_fly.unwrap_or_default();
            ServerInboundCommand::State(ServerStateCommand {
                playstate: state.playstate.map(|playstate| ServerPlaystateCommand {
                    position: playstate.position,
                    paused: playstate.paused,
                    do_seek: playstate.do_seek,
                    set_by: playstate.set_by,
                }),
                ping: state.ping.map(|ping| ServerPingCommand {
                    latency_calculation: ping.latency_calculation,
                    client_latency_calculation: ping.client_latency_calculation,
                    client_rtt: ping.client_rtt,
                }),
                server_ignoring: ignoring.server,
                client_ignoring: ignoring.client,
                playback_barrier,
                readiness,
            })
        }
        ProtocolMessage::Tls(message) => ServerInboundCommand::Tls(message.tls.start_tls),
        ProtocolMessage::Chat(message) => ServerInboundCommand::Chat(match message.chat {
            ChatPayload::Text(message) => message,
            ChatPayload::Message(message) => message.message,
        }),
        ProtocolMessage::Error(_) => {
            fallbacks
                .push(ServerCompatibilityFallback::IgnoredUnexpectedMessage { command: "Error" });
            ServerInboundCommand::Ignore
        }
    };
    NormalizedServerInbound { command, fallbacks }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_credentials_remain_redacted_in_debug_output() {
        let hello = sorotte_protocol::decode_message_line(
            r#"{"Hello":{"username":"alice","room":{"name":"room"},"version":"1.7.5","password":"normalized-token-secret"}}"#,
        )
        .expect("hello should decode");
        let controller_auth = sorotte_protocol::decode_message_line(
            r#"{"Set":{"controllerAuth":{"room":"room","password":"normalized-controller-secret"}}}"#,
        )
        .expect("controller auth should decode");

        let hello_debug = format!("{:?}", normalize_server_protocol_message(hello));
        let controller_debug = format!("{:?}", normalize_server_protocol_message(controller_auth));

        assert!(hello_debug.contains("<redacted>"));
        assert!(controller_debug.contains("<redacted>"));
        assert!(!hello_debug.contains("normalized-token-secret"));
        assert!(!controller_debug.contains("normalized-controller-secret"));
    }
}
