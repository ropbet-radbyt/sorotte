use super::*;
use std::collections::VecDeque;

use crate::outbox::{EffectOutbox, ProtocolLineLease, ProtocolOutbox};
use sorotte_protocol::PlaybackBarrierSetExtension;

macro_rules! notification_outbox_methods {
    ($front:ident, $acknowledge:ident, $flush:ident, $field:ident, $notification:ty) => {
        pub(crate) fn $front(&self) -> Option<&$notification> {
            self.$field.front()
        }

        pub(crate) fn $acknowledge(&mut self) -> Option<$notification> {
            self.$field.acknowledge_front()
        }

        pub(crate) fn $flush<E>(
            &mut self,
            notify: impl FnMut(&$notification) -> Result<(), E>,
        ) -> Result<(), E> {
            self.$field.try_flush(notify)
        }
    };
}

#[derive(Clone, PartialEq)]
pub enum ClientRuntimeAction {
    SetPaused(bool),
    RequestUserList,
    SetRoom {
        room: String,
    },
    SetReady {
        ready: bool,
        manually_initiated: bool,
    },
    SetReadyForUser {
        ready: bool,
        manually_initiated: bool,
        username: String,
    },
    SetFile {
        file: FilePayload,
    },
    SetPlaylist {
        files: Vec<String>,
    },
    SetPlaylistIndex {
        index: i64,
    },
    RequestControllerAuth {
        room: String,
        password: SecretValue,
    },
    SendChat {
        message: String,
    },
    NotifyChat(ChatNotification),
    NotifyControlledRoomCreation(ControlledRoomCreationNotification),
    NotifyControllerAuthTransition(ControllerAuthTransitionNotification),
    NotifyUserChange(UserChangeNotification),
    NotifyReconnectTransition(ReconnectTransitionNotification),
    NotifyAutoplayCountdown(AutoplayCountdownNotification),
    SetPosition(f64),
    SetPlaybackRate(f64),
    ScheduleReconnect {
        delay_seconds: f64,
    },
    StopReconnect,
}

impl std::fmt::Debug for ClientRuntimeAction {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SetPaused(value) => formatter.debug_tuple("SetPaused").field(value).finish(),
            Self::RequestUserList => formatter.write_str("RequestUserList"),
            Self::SetRoom { .. } => formatter.write_str("SetRoom(<redacted>)"),
            Self::SetReady {
                ready,
                manually_initiated,
            } => formatter
                .debug_struct("SetReady")
                .field("ready", ready)
                .field("manually_initiated", manually_initiated)
                .finish(),
            Self::SetReadyForUser {
                ready,
                manually_initiated,
                ..
            } => formatter
                .debug_struct("SetReadyForUser")
                .field("ready", ready)
                .field("manually_initiated", manually_initiated)
                .field("username", &sorotte_secret::REDACTED_SECRET)
                .finish(),
            Self::SetFile { file } => formatter.debug_tuple("SetFile").field(file).finish(),
            Self::SetPlaylist { files } => formatter
                .debug_struct("SetPlaylist")
                .field("files_count", &files.len())
                .finish(),
            Self::SetPlaylistIndex { index } => formatter
                .debug_struct("SetPlaylistIndex")
                .field("index", index)
                .finish(),
            Self::RequestControllerAuth { password, .. } => formatter
                .debug_struct("RequestControllerAuth")
                .field("room", &sorotte_secret::REDACTED_SECRET)
                .field("password", password)
                .finish(),
            Self::SendChat { .. } => formatter.write_str("SendChat(<redacted>)"),
            Self::NotifyChat(_) => formatter.write_str("NotifyChat(<redacted>)"),
            Self::NotifyControlledRoomCreation(notification) => formatter
                .debug_tuple("NotifyControlledRoomCreation")
                .field(notification)
                .finish(),
            Self::NotifyControllerAuthTransition(_) => {
                formatter.write_str("NotifyControllerAuthTransition(<redacted>)")
            }
            Self::NotifyUserChange(_) => formatter.write_str("NotifyUserChange(<redacted>)"),
            Self::NotifyReconnectTransition(notification) => formatter
                .debug_tuple("NotifyReconnectTransition")
                .field(notification)
                .finish(),
            Self::NotifyAutoplayCountdown(notification) => formatter
                .debug_tuple("NotifyAutoplayCountdown")
                .field(notification)
                .finish(),
            Self::SetPosition(value) => formatter.debug_tuple("SetPosition").field(value).finish(),
            Self::SetPlaybackRate(value) => formatter
                .debug_tuple("SetPlaybackRate")
                .field(value)
                .finish(),
            Self::ScheduleReconnect { delay_seconds } => formatter
                .debug_struct("ScheduleReconnect")
                .field("delay_seconds", delay_seconds)
                .finish(),
            Self::StopReconnect => formatter.write_str("StopReconnect"),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct PlaybackBarrierRequestScope {
    room: String,
    local_media_generation: u64,
    request_nonce: u64,
}

impl PlaybackBarrierRequestScope {
    pub fn new(room: impl Into<String>, local_media_generation: u64, request_nonce: u64) -> Self {
        Self {
            room: room.into(),
            local_media_generation,
            request_nonce,
        }
    }

    fn matches(&self, extension: &PlaybackBarrierSetExtension) -> bool {
        if self.room.trim().is_empty()
            || self.local_media_generation == 0
            || self.request_nonce == 0
        {
            return false;
        }

        let prepare_nonce = extension
            .prepare
            .as_ref()
            .map(|prepare| prepare.request_nonce);
        let buffering_nonce = extension
            .buffering_policy
            .as_ref()
            .map(|policy| policy.request_nonce);
        (prepare_nonce.is_some() || buffering_nonce.is_some())
            && prepare_nonce
                .into_iter()
                .chain(buffering_nonce)
                .all(|request_nonce| request_nonce == self.request_nonce)
    }
}

impl std::fmt::Debug for PlaybackBarrierRequestScope {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlaybackBarrierRequestScope")
            .field("room", &sorotte_secret::REDACTED_SECRET)
            .field("local_media_generation", &self.local_media_generation)
            .field("request_nonce", &self.request_nonce)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingProtocolLine {
    lease: ProtocolLineLease,
    line: String,
}

impl PendingProtocolLine {
    pub fn lease(&self) -> ProtocolLineLease {
        self.lease
    }

    pub fn line(&self) -> &str {
        &self.line
    }

    pub fn into_line(self) -> String {
        self.line
    }
}

#[derive(Clone, PartialEq)]
pub enum ClientEffect {
    SetPlayerPaused(bool),
    SetPlayerPosition(f64),
    SetPlayerPlaybackRate(f64),
    RequestUserList,
    SetRoom(String),
    SetReady {
        ready: bool,
        manually_initiated: bool,
    },
    SetReadyForUser {
        ready: bool,
        manually_initiated: bool,
        username: String,
    },
    SetFile(FilePayload),
    SetPlaylist(Vec<String>),
    SetPlaylistIndex(i64),
    /// Connection-scoped reliable Set-envelope control for playback prepare
    /// and ongoing room buffering policy requests. Observation
    /// acknowledgements use SendState.
    SendPlaybackBarrierSet {
        extension: Box<PlaybackBarrierSetExtension>,
        scope: PlaybackBarrierRequestScope,
    },
    SendState(StatePayload),
    RequestControllerAuth(ControllerAuthPayload),
    SendChat(String),
    NotifyChat(ChatNotification),
    NotifyControlledRoomCreation(ControlledRoomCreationNotification),
    NotifyControllerAuthTransition(ControllerAuthTransitionNotification),
    NotifyUserChange(UserChangeNotification),
    NotifyReconnectTransition(ReconnectTransitionNotification),
    NotifyAutoplayCountdown(AutoplayCountdownNotification),
    ScheduleReconnect(f64),
    StopReconnect,
}

impl std::fmt::Debug for ClientEffect {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SetPlayerPaused(value) => formatter
                .debug_tuple("SetPlayerPaused")
                .field(value)
                .finish(),
            Self::SetPlayerPosition(value) => formatter
                .debug_tuple("SetPlayerPosition")
                .field(value)
                .finish(),
            Self::SetPlayerPlaybackRate(value) => formatter
                .debug_tuple("SetPlayerPlaybackRate")
                .field(value)
                .finish(),
            Self::RequestUserList => formatter.write_str("RequestUserList"),
            Self::SetRoom(_) => formatter.write_str("SetRoom(<redacted>)"),
            Self::SetReady {
                ready,
                manually_initiated,
            } => formatter
                .debug_struct("SetReady")
                .field("ready", ready)
                .field("manually_initiated", manually_initiated)
                .finish(),
            Self::SetReadyForUser {
                ready,
                manually_initiated,
                ..
            } => formatter
                .debug_struct("SetReadyForUser")
                .field("ready", ready)
                .field("manually_initiated", manually_initiated)
                .field("username", &sorotte_secret::REDACTED_SECRET)
                .finish(),
            Self::SetFile(file) => formatter.debug_tuple("SetFile").field(file).finish(),
            Self::SetPlaylist(files) => formatter
                .debug_struct("SetPlaylist")
                .field("files_count", &files.len())
                .finish(),
            Self::SetPlaylistIndex(index) => formatter
                .debug_tuple("SetPlaylistIndex")
                .field(index)
                .finish(),
            Self::SendPlaybackBarrierSet { extension, scope } => formatter
                .debug_struct("SendPlaybackBarrierSet")
                .field("extension", extension)
                .field("scope", scope)
                .finish(),
            Self::SendState(state) => formatter.debug_tuple("SendState").field(state).finish(),
            Self::RequestControllerAuth(payload) => formatter
                .debug_tuple("RequestControllerAuth")
                .field(payload)
                .finish(),
            Self::SendChat(_) => formatter.write_str("SendChat(<redacted>)"),
            Self::NotifyChat(_) => formatter.write_str("NotifyChat(<redacted>)"),
            Self::NotifyControlledRoomCreation(notification) => formatter
                .debug_tuple("NotifyControlledRoomCreation")
                .field(notification)
                .finish(),
            Self::NotifyControllerAuthTransition(_) => {
                formatter.write_str("NotifyControllerAuthTransition(<redacted>)")
            }
            Self::NotifyUserChange(_) => formatter.write_str("NotifyUserChange(<redacted>)"),
            Self::NotifyReconnectTransition(notification) => formatter
                .debug_tuple("NotifyReconnectTransition")
                .field(notification)
                .finish(),
            Self::NotifyAutoplayCountdown(notification) => formatter
                .debug_tuple("NotifyAutoplayCountdown")
                .field(notification)
                .finish(),
            Self::ScheduleReconnect(delay_seconds) => formatter
                .debug_tuple("ScheduleReconnect")
                .field(delay_seconds)
                .finish(),
            Self::StopReconnect => formatter.write_str("StopReconnect"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClientEffectError {
    #[error("client effect is not supported: {0}")]
    Unsupported(&'static str),
    #[error("invalid file effect payload: {0}")]
    InvalidFilePayload(String),
    #[error("client effect failed: {0}")]
    OperationFailed(String),
}

impl ClientEffect {
    pub fn set_file_from_value(value: Value) -> Result<Self, ClientEffectError> {
        serde_json::from_value(value)
            .map(Self::SetFile)
            .map_err(|error| ClientEffectError::InvalidFilePayload(error.to_string()))
    }

    pub fn send_playback_barrier_set(
        extension: PlaybackBarrierSetExtension,
        scope: PlaybackBarrierRequestScope,
    ) -> Self {
        Self::SendPlaybackBarrierSet {
            extension: Box::new(extension),
            scope,
        }
    }
}

pub trait ClientEffectSink {
    fn emit(&mut self, effect: ClientEffect) -> Result<(), ClientEffectError>;

    /// Starts a fresh transport generation. Reliable commands remain owned by
    /// the sink, while connection-scoped effects from the previous transport
    /// are discarded.
    fn begin_protocol_connection_generation(&mut self) {}

    /// Allows connection-scoped effects for the current generation after the
    /// server handshake has made the session active.
    fn activate_protocol_connection_generation(&mut self) {}

    /// Retains only a playback-barrier request matching the current room and
    /// local-media episode. A newly prepared episode invalidates serialized
    /// intent for the previous media even when no replacement can be emitted.
    fn retain_protocol_playback_barrier_scope(
        &mut self,
        _room: &str,
        _local_media_generation: u64,
    ) {
    }

    /// Explicitly cancels any undelivered playback-barrier Set request without
    /// disturbing durable chat, playlist, or other protocol commands.
    fn cancel_protocol_playback_barrier_requests(&mut self) {}
}

pub(crate) fn client_effect_player_error(error: ClientEffectError) -> PlayerError {
    PlayerError::OperationFailed(error.to_string())
}

#[derive(Debug, Default)]
pub struct QueuedRuntimeControl {
    pub(crate) outbound_messages: ProtocolOutbox,
    reconnect_delays: Vec<f64>,
    stop_reconnect_calls: usize,
    chat_notifications: EffectOutbox<ChatNotification>,
    controlled_room_creation_notifications: EffectOutbox<ControlledRoomCreationNotification>,
    controller_auth_notifications: EffectOutbox<ControllerAuthTransitionNotification>,
    user_change_notifications: EffectOutbox<UserChangeNotification>,
    reconnect_notifications: EffectOutbox<ReconnectTransitionNotification>,
    autoplay_notifications: EffectOutbox<AutoplayCountdownNotification>,
}

impl QueuedRuntimeControl {
    pub fn outbound_messages(&self) -> &VecDeque<ProtocolMessage> {
        self.outbound_messages.pending()
    }

    pub fn reconnect_delays(&self) -> &[f64] {
        &self.reconnect_delays
    }

    pub fn stop_reconnect_calls(&self) -> usize {
        self.stop_reconnect_calls
    }

    pub fn autoplay_notifications(&self) -> &VecDeque<AutoplayCountdownNotification> {
        self.autoplay_notifications.pending()
    }

    pub fn chat_notifications(&self) -> &VecDeque<ChatNotification> {
        self.chat_notifications.pending()
    }

    pub fn controlled_room_creation_notifications(
        &self,
    ) -> &VecDeque<ControlledRoomCreationNotification> {
        self.controlled_room_creation_notifications.pending()
    }

    pub fn controller_auth_notifications(&self) -> &VecDeque<ControllerAuthTransitionNotification> {
        self.controller_auth_notifications.pending()
    }

    pub fn user_change_notifications(&self) -> &VecDeque<UserChangeNotification> {
        self.user_change_notifications.pending()
    }

    pub fn reconnect_notifications(&self) -> &VecDeque<ReconnectTransitionNotification> {
        self.reconnect_notifications.pending()
    }

    pub(crate) fn queue_connection_scoped_state(&mut self, state: StatePayload) -> bool {
        self.outbound_messages
            .push_connection_scoped_state(ProtocolMessage::state(state))
    }

    pub fn drain_outbound_messages(&mut self) -> Vec<ProtocolMessage> {
        self.outbound_messages.drain()
    }

    pub fn drain_outbound_message_lines(&mut self) -> Result<Vec<String>, ProtocolError> {
        let lines = self
            .outbound_messages
            .pending()
            .iter()
            .map(encode_message_line)
            .collect::<Result<Vec<_>, _>>()?;
        self.outbound_messages.clear();
        Ok(lines)
    }

    pub(crate) fn front_outbound_message_line(
        &self,
    ) -> Result<Option<PendingProtocolLine>, ProtocolError> {
        self.outbound_messages
            .front_for_delivery()
            .map(|(lease, message)| {
                encode_message_line(message).map(|line| PendingProtocolLine { lease, line })
            })
            .transpose()
    }

    pub(crate) fn acknowledge_outbound_message(
        &mut self,
        lease: ProtocolLineLease,
    ) -> Option<ProtocolMessage> {
        self.outbound_messages.acknowledge_front(lease)
    }

    pub(crate) fn release_outbound_message(&mut self, lease: ProtocolLineLease) -> bool {
        self.outbound_messages.release_front(lease)
    }

    pub fn drain_reconnect_delays(&mut self) -> Vec<f64> {
        std::mem::take(&mut self.reconnect_delays)
    }

    pub fn take_stop_reconnect_requested(&mut self) -> bool {
        let requested = self.stop_reconnect_calls > 0;
        self.stop_reconnect_calls = 0;
        requested
    }

    pub fn drain_autoplay_notifications(&mut self) -> Vec<AutoplayCountdownNotification> {
        self.autoplay_notifications.drain()
    }

    pub fn drain_chat_notifications(&mut self) -> Vec<ChatNotification> {
        self.chat_notifications.drain()
    }

    pub fn drain_controlled_room_creation_notifications(
        &mut self,
    ) -> Vec<ControlledRoomCreationNotification> {
        self.controlled_room_creation_notifications.drain()
    }

    pub fn drain_controller_auth_notifications(
        &mut self,
    ) -> Vec<ControllerAuthTransitionNotification> {
        self.controller_auth_notifications.drain()
    }

    pub fn drain_user_change_notifications(&mut self) -> Vec<UserChangeNotification> {
        self.user_change_notifications.drain()
    }

    pub fn drain_reconnect_notifications(&mut self) -> Vec<ReconnectTransitionNotification> {
        self.reconnect_notifications.drain()
    }

    notification_outbox_methods!(
        front_autoplay_notification,
        acknowledge_autoplay_notification,
        flush_autoplay_notifications,
        autoplay_notifications,
        AutoplayCountdownNotification
    );
    notification_outbox_methods!(
        front_chat_notification,
        acknowledge_chat_notification,
        flush_chat_notifications,
        chat_notifications,
        ChatNotification
    );
    notification_outbox_methods!(
        front_controlled_room_creation_notification,
        acknowledge_controlled_room_creation_notification,
        flush_controlled_room_creation_notifications,
        controlled_room_creation_notifications,
        ControlledRoomCreationNotification
    );
    notification_outbox_methods!(
        front_controller_auth_notification,
        acknowledge_controller_auth_notification,
        flush_controller_auth_notifications,
        controller_auth_notifications,
        ControllerAuthTransitionNotification
    );
    notification_outbox_methods!(
        front_user_change_notification,
        acknowledge_user_change_notification,
        flush_user_change_notifications,
        user_change_notifications,
        UserChangeNotification
    );
    notification_outbox_methods!(
        front_reconnect_notification,
        acknowledge_reconnect_notification,
        flush_reconnect_notifications,
        reconnect_notifications,
        ReconnectTransitionNotification
    );
}

impl ClientEffectSink for QueuedRuntimeControl {
    fn begin_protocol_connection_generation(&mut self) {
        self.outbound_messages.begin_connection_generation();
    }

    fn activate_protocol_connection_generation(&mut self) {
        self.outbound_messages.activate_connection_generation();
    }

    fn retain_protocol_playback_barrier_scope(&mut self, room: &str, local_media_generation: u64) {
        self.outbound_messages
            .retain_connection_scoped_reliable_scope(room, local_media_generation);
    }

    fn cancel_protocol_playback_barrier_requests(&mut self) {
        self.outbound_messages.cancel_connection_scoped_reliable();
    }

    fn emit(&mut self, effect: ClientEffect) -> Result<(), ClientEffectError> {
        match effect {
            ClientEffect::SetPlayerPaused(_) => {
                return Err(ClientEffectError::Unsupported("set_player_paused"));
            }
            ClientEffect::SetPlayerPosition(_) => {
                return Err(ClientEffectError::Unsupported("set_player_position"));
            }
            ClientEffect::SetPlayerPlaybackRate(_) => {
                return Err(ClientEffectError::Unsupported("set_player_playback_rate"));
            }
            ClientEffect::RequestUserList => self
                .outbound_messages
                .push_back(ProtocolMessage::list_request()),
            ClientEffect::SetRoom(room) => {
                self.outbound_messages.cancel_connection_scoped_reliable();
                let set_payload = SetPayload::new().with_room(RoomRef::new(room));
                self.outbound_messages
                    .push_back(ProtocolMessage::set(set_payload));
            }
            ClientEffect::SetReady {
                ready,
                manually_initiated,
            } => {
                let ready_payload =
                    ReadyPayload::new(ready).with_manually_initiated(manually_initiated);
                let set_payload = SetPayload::new().with_ready(ready_payload);
                self.outbound_messages
                    .push_back(ProtocolMessage::set(set_payload));
            }
            ClientEffect::SetReadyForUser {
                ready,
                manually_initiated,
                username,
            } => {
                let ready_payload = ReadyPayload::new(ready)
                    .with_manually_initiated(manually_initiated)
                    .with_username(username);
                let set_payload = SetPayload::new().with_ready(ready_payload);
                self.outbound_messages
                    .push_back(ProtocolMessage::set(set_payload));
            }
            ClientEffect::SetFile(file) => {
                let set_payload = SetPayload::new().with_file(file);
                self.outbound_messages
                    .push_back(ProtocolMessage::set(set_payload));
            }
            ClientEffect::SetPlaylist(files) => {
                let set_payload = SetPayload::new()
                    .with_playlist_change(playlist_change_with_plex_sidecar(files, true));
                self.outbound_messages
                    .push_back(ProtocolMessage::set(set_payload));
            }
            ClientEffect::SetPlaylistIndex(index) => {
                let set_payload =
                    SetPayload::new().with_playlist_index(PlaylistIndexPayload::new(index));
                self.outbound_messages
                    .push_back(ProtocolMessage::set(set_payload));
            }
            ClientEffect::SendPlaybackBarrierSet { extension, scope } => {
                if !scope.matches(&extension) {
                    return Err(ClientEffectError::OperationFailed(
                        "playback barrier request scope does not match its payload".to_owned(),
                    ));
                }
                let set_payload = SetPayload::new().with_playback_barrier_v1(*extension);
                let _ = self.outbound_messages.push_connection_scoped_reliable(
                    ProtocolMessage::set(set_payload),
                    scope.room,
                    scope.local_media_generation,
                    scope.request_nonce,
                );
            }
            ClientEffect::SendState(state) => {
                let _ = self.queue_connection_scoped_state(state);
            }
            ClientEffect::RequestControllerAuth(payload) => {
                let set_payload = SetPayload::new().with_controller_auth(payload);
                self.outbound_messages
                    .push_back(ProtocolMessage::set(set_payload));
            }
            ClientEffect::SendChat(message) => self
                .outbound_messages
                .push_back(ProtocolMessage::chat_text(message)),
            ClientEffect::NotifyChat(notification) => {
                self.chat_notifications.push_back(notification);
            }
            ClientEffect::NotifyControlledRoomCreation(notification) => self
                .controlled_room_creation_notifications
                .push_back(notification),
            ClientEffect::NotifyControllerAuthTransition(notification) => {
                self.controller_auth_notifications.push_back(notification);
            }
            ClientEffect::NotifyUserChange(notification) => {
                self.user_change_notifications.push_back(notification);
            }
            ClientEffect::NotifyReconnectTransition(notification) => {
                self.reconnect_notifications.push_back(notification);
            }
            ClientEffect::NotifyAutoplayCountdown(notification) => {
                self.autoplay_notifications.push_back(notification);
            }
            ClientEffect::ScheduleReconnect(delay_seconds) => {
                self.reconnect_delays.push(delay_seconds);
            }
            ClientEffect::StopReconnect => self.stop_reconnect_calls += 1,
        }
        Ok(())
    }
}

#[cfg(test)]
mod credential_debug_tests {
    use super::{
        ChatNotification, ClientEffect, ClientMediaMatchPeerFileState, ClientRuntimeAction,
        ControlledRoomCreationNotification, ReconnectPlaylistRestoreIntent, RoomPlaylistView,
        SecretValue, SharedFile,
    };

    #[test]
    fn controller_password_debug_canary_is_redacted_across_actions_and_notifications() {
        const MARKER: &str = "client-core-secret-canary-0c96a1";
        let action = ClientRuntimeAction::RequestControllerAuth {
            room: "+room:ABCDEF123456".to_owned(),
            password: SecretValue::new(MARKER),
        };
        let notification = ControlledRoomCreationNotification::Created {
            room: "+room:ABCDEF123456".to_owned(),
            password: SecretValue::new(MARKER),
        };

        for debug in [format!("{action:?}"), format!("{notification:?}")] {
            assert!(debug.contains("<redacted>"));
            assert!(!debug.contains(MARKER));
        }
    }

    #[test]
    fn tokenized_media_debug_canary_is_redacted_across_domain_carriers() {
        const MARKER: &str = "client-core-media-secret-canary-4a8f62";
        let target = format!("https://media.example/video?X-Plex-Token={MARKER}");
        let debug_values = [
            format!(
                "{:?}",
                SharedFile {
                    name: Some(target.clone()),
                    ..SharedFile::default()
                }
            ),
            format!(
                "{:?}",
                ClientMediaMatchPeerFileState {
                    file_name: Some(target.clone()),
                    ..ClientMediaMatchPeerFileState::default()
                }
            ),
            format!(
                "{:?}",
                RoomPlaylistView {
                    files: vec![target.clone()],
                    ..RoomPlaylistView::default()
                }
            ),
            format!(
                "{:?}",
                ReconnectPlaylistRestoreIntent {
                    files: vec![target.clone()],
                    index: Some(0),
                }
            ),
            format!(
                "{:?}",
                ClientRuntimeAction::SetPlaylist {
                    files: vec![target.clone()],
                }
            ),
            format!("{:?}", ClientEffect::SetPlaylist(vec![target.clone()])),
            format!(
                "{:?}",
                ChatNotification::Message {
                    username: None,
                    message: target,
                }
            ),
        ];

        for debug in debug_values {
            assert!(!debug.contains(MARKER), "leaky Debug output: {debug}");
        }
    }
}
