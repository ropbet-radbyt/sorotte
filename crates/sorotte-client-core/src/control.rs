use super::*;
use std::collections::VecDeque;

use crate::outbox::EffectOutbox;

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

#[derive(Debug, Clone, PartialEq)]
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
        file_payload: Value,
    },
    SetPlaylist {
        files: Vec<String>,
    },
    SetPlaylistIndex {
        index: i64,
    },
    RequestControllerAuth {
        room: String,
        password: String,
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

pub trait ClientRuntimeControl {
    fn request_user_list(&mut self) {}
    fn set_room(&mut self, _room: String) {}
    fn set_ready(&mut self, ready: bool, manually_initiated: bool);
    fn set_ready_for_user(&mut self, ready: bool, manually_initiated: bool, _username: String) {
        self.set_ready(ready, manually_initiated);
    }
    fn set_file(&mut self, _file_payload: Value) {}
    fn set_playlist(&mut self, _files: Vec<String>) {}
    fn set_playlist_index(&mut self, _index: i64) {}
    fn send_state(&mut self, _state: StatePayload) {}
    fn request_controller_auth(&mut self, _room: String, _password: String) {}
    fn send_chat(&mut self, _message: String) {}
    fn notify_chat(&mut self, _notification: ChatNotification) {}
    fn notify_controlled_room_creation(
        &mut self,
        _notification: ControlledRoomCreationNotification,
    ) {
    }
    fn notify_controller_auth_transition(
        &mut self,
        _notification: ControllerAuthTransitionNotification,
    ) {
    }
    fn notify_user_change(&mut self, _notification: UserChangeNotification) {}
    fn notify_reconnect_transition(&mut self, _notification: ReconnectTransitionNotification) {}
    fn schedule_reconnect(&mut self, delay_seconds: f64);
    fn stop_reconnect(&mut self);
    fn notify_autoplay_countdown(&mut self, _notification: AutoplayCountdownNotification) {}
}

#[derive(Debug, Default)]
pub struct QueuedRuntimeControl {
    pub(crate) outbound_messages: EffectOutbox<ProtocolMessage>,
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
    fn file_payload_from_value(file_payload: Value) -> Option<FilePayload> {
        let Value::Object(mut fields) = file_payload else {
            return None;
        };

        let name = fields
            .remove("name")
            .and_then(|value| value.as_str().map(str::to_owned));
        let duration = fields.remove("duration").and_then(|value| value.as_f64());
        let size = fields.remove("size");
        let path = fields
            .remove("path")
            .and_then(|value| value.as_str().map(str::to_owned));

        Some(FilePayload {
            name,
            duration,
            size,
            path,
            extra: fields.into_iter().collect(),
        })
    }

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

    pub(crate) fn front_outbound_message_line(&self) -> Result<Option<String>, ProtocolError> {
        self.outbound_messages
            .front()
            .map(encode_message_line)
            .transpose()
    }

    pub(crate) fn acknowledge_outbound_message(&mut self) -> Option<ProtocolMessage> {
        self.outbound_messages.acknowledge_front()
    }

    pub(crate) fn flush_outbound_message_lines<F>(
        &mut self,
        mut send_line: F,
    ) -> Result<(), ProtocolError>
    where
        F: FnMut(&str) -> Result<(), ProtocolError>,
    {
        self.outbound_messages.try_flush(|message| {
            let line = encode_message_line(message)?;
            send_line(&line)
        })
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

impl ClientRuntimeControl for QueuedRuntimeControl {
    fn request_user_list(&mut self) {
        self.outbound_messages
            .push_back(ProtocolMessage::list_request());
    }

    fn set_room(&mut self, room: String) {
        let set_payload = SetPayload::new().with_room(RoomRef::new(room));
        self.outbound_messages
            .push_back(ProtocolMessage::set(set_payload));
    }

    fn set_ready(&mut self, ready: bool, manually_initiated: bool) {
        let ready_payload = ReadyPayload::new(ready).with_manually_initiated(manually_initiated);
        let set_payload = SetPayload::new().with_ready(ready_payload);
        self.outbound_messages
            .push_back(ProtocolMessage::set(set_payload));
    }

    fn set_ready_for_user(&mut self, ready: bool, manually_initiated: bool, username: String) {
        let ready_payload = ReadyPayload::new(ready)
            .with_manually_initiated(manually_initiated)
            .with_username(username);
        let set_payload = SetPayload::new().with_ready(ready_payload);
        self.outbound_messages
            .push_back(ProtocolMessage::set(set_payload));
    }

    fn set_file(&mut self, file_payload: Value) {
        let Some(file_payload) = Self::file_payload_from_value(file_payload) else {
            return;
        };
        let set_payload = SetPayload::new().with_file(file_payload);
        self.outbound_messages
            .push_back(ProtocolMessage::set(set_payload));
    }

    fn set_playlist(&mut self, files: Vec<String>) {
        let set_payload =
            SetPayload::new().with_playlist_change(playlist_change_with_plex_sidecar(files, true));
        self.outbound_messages
            .push_back(ProtocolMessage::set(set_payload));
    }

    fn set_playlist_index(&mut self, index: i64) {
        let set_payload = SetPayload::new().with_playlist_index(PlaylistIndexPayload::new(index));
        self.outbound_messages
            .push_back(ProtocolMessage::set(set_payload));
    }

    fn send_state(&mut self, state: StatePayload) {
        self.outbound_messages
            .push_back(ProtocolMessage::state(state));
    }

    fn request_controller_auth(&mut self, room: String, password: String) {
        let payload = ControllerAuthPayload::new()
            .with_room(room)
            .with_password(password);
        let set_payload = SetPayload::new().with_controller_auth(payload);
        self.outbound_messages
            .push_back(ProtocolMessage::set(set_payload));
    }

    fn send_chat(&mut self, message: String) {
        self.outbound_messages
            .push_back(ProtocolMessage::chat_text(message));
    }

    fn notify_chat(&mut self, notification: ChatNotification) {
        self.chat_notifications.push_back(notification);
    }

    fn notify_controlled_room_creation(
        &mut self,
        notification: ControlledRoomCreationNotification,
    ) {
        self.controlled_room_creation_notifications
            .push_back(notification);
    }

    fn notify_controller_auth_transition(
        &mut self,
        notification: ControllerAuthTransitionNotification,
    ) {
        self.controller_auth_notifications.push_back(notification);
    }

    fn notify_user_change(&mut self, notification: UserChangeNotification) {
        self.user_change_notifications.push_back(notification);
    }

    fn schedule_reconnect(&mut self, delay_seconds: f64) {
        self.reconnect_delays.push(delay_seconds);
    }

    fn stop_reconnect(&mut self) {
        self.stop_reconnect_calls += 1;
    }

    fn notify_reconnect_transition(&mut self, notification: ReconnectTransitionNotification) {
        self.reconnect_notifications.push_back(notification);
    }

    fn notify_autoplay_countdown(&mut self, notification: AutoplayCountdownNotification) {
        self.autoplay_notifications.push_back(notification);
    }
}
