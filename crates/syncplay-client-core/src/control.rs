use super::*;

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
    pub(crate) outbound_messages: Vec<ProtocolMessage>,
    reconnect_delays: Vec<f64>,
    stop_reconnect_calls: usize,
    chat_notifications: Vec<ChatNotification>,
    controlled_room_creation_notifications: Vec<ControlledRoomCreationNotification>,
    controller_auth_notifications: Vec<ControllerAuthTransitionNotification>,
    user_change_notifications: Vec<UserChangeNotification>,
    reconnect_notifications: Vec<ReconnectTransitionNotification>,
    autoplay_notifications: Vec<AutoplayCountdownNotification>,
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

    pub fn outbound_messages(&self) -> &[ProtocolMessage] {
        &self.outbound_messages
    }

    pub fn reconnect_delays(&self) -> &[f64] {
        &self.reconnect_delays
    }

    pub fn stop_reconnect_calls(&self) -> usize {
        self.stop_reconnect_calls
    }

    pub fn autoplay_notifications(&self) -> &[AutoplayCountdownNotification] {
        &self.autoplay_notifications
    }

    pub fn chat_notifications(&self) -> &[ChatNotification] {
        &self.chat_notifications
    }

    pub fn controlled_room_creation_notifications(&self) -> &[ControlledRoomCreationNotification] {
        &self.controlled_room_creation_notifications
    }

    pub fn controller_auth_notifications(&self) -> &[ControllerAuthTransitionNotification] {
        &self.controller_auth_notifications
    }

    pub fn user_change_notifications(&self) -> &[UserChangeNotification] {
        &self.user_change_notifications
    }

    pub fn reconnect_notifications(&self) -> &[ReconnectTransitionNotification] {
        &self.reconnect_notifications
    }

    pub fn drain_outbound_messages(&mut self) -> Vec<ProtocolMessage> {
        std::mem::take(&mut self.outbound_messages)
    }

    pub fn drain_outbound_message_lines(&mut self) -> Result<Vec<String>, ProtocolError> {
        let messages = self.drain_outbound_messages();
        messages
            .iter()
            .map(encode_message_line)
            .collect::<Result<Vec<_>, _>>()
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
        std::mem::take(&mut self.autoplay_notifications)
    }

    pub fn drain_chat_notifications(&mut self) -> Vec<ChatNotification> {
        std::mem::take(&mut self.chat_notifications)
    }

    pub fn drain_controlled_room_creation_notifications(
        &mut self,
    ) -> Vec<ControlledRoomCreationNotification> {
        std::mem::take(&mut self.controlled_room_creation_notifications)
    }

    pub fn drain_controller_auth_notifications(
        &mut self,
    ) -> Vec<ControllerAuthTransitionNotification> {
        std::mem::take(&mut self.controller_auth_notifications)
    }

    pub fn drain_user_change_notifications(&mut self) -> Vec<UserChangeNotification> {
        std::mem::take(&mut self.user_change_notifications)
    }

    pub fn drain_reconnect_notifications(&mut self) -> Vec<ReconnectTransitionNotification> {
        std::mem::take(&mut self.reconnect_notifications)
    }
}

impl ClientRuntimeControl for QueuedRuntimeControl {
    fn request_user_list(&mut self) {
        self.outbound_messages.push(ProtocolMessage::list_request());
    }

    fn set_room(&mut self, room: String) {
        let set_payload = SetPayload::new().with_room(RoomRef::new(room));
        self.outbound_messages
            .push(ProtocolMessage::set(set_payload));
    }

    fn set_ready(&mut self, ready: bool, manually_initiated: bool) {
        let ready_payload = ReadyPayload::new(ready).with_manually_initiated(manually_initiated);
        let set_payload = SetPayload::new().with_ready(ready_payload);
        self.outbound_messages
            .push(ProtocolMessage::set(set_payload));
    }

    fn set_ready_for_user(&mut self, ready: bool, manually_initiated: bool, username: String) {
        let ready_payload = ReadyPayload::new(ready)
            .with_manually_initiated(manually_initiated)
            .with_username(username);
        let set_payload = SetPayload::new().with_ready(ready_payload);
        self.outbound_messages
            .push(ProtocolMessage::set(set_payload));
    }

    fn set_file(&mut self, file_payload: Value) {
        let Some(file_payload) = Self::file_payload_from_value(file_payload) else {
            return;
        };
        let set_payload = SetPayload::new().with_file(file_payload);
        self.outbound_messages
            .push(ProtocolMessage::set(set_payload));
    }

    fn set_playlist(&mut self, files: Vec<String>) {
        let set_payload = SetPayload::new().with_playlist_change(PlaylistChangePayload::new(files));
        self.outbound_messages
            .push(ProtocolMessage::set(set_payload));
    }

    fn set_playlist_index(&mut self, index: i64) {
        let set_payload = SetPayload::new().with_playlist_index(PlaylistIndexPayload::new(index));
        self.outbound_messages
            .push(ProtocolMessage::set(set_payload));
    }

    fn send_state(&mut self, state: StatePayload) {
        self.outbound_messages.push(ProtocolMessage::state(state));
    }

    fn request_controller_auth(&mut self, room: String, password: String) {
        let payload = ControllerAuthPayload::new()
            .with_room(room)
            .with_password(password);
        let set_payload = SetPayload::new().with_controller_auth(payload);
        self.outbound_messages
            .push(ProtocolMessage::set(set_payload));
    }

    fn send_chat(&mut self, message: String) {
        self.outbound_messages
            .push(ProtocolMessage::chat_text(message));
    }

    fn notify_chat(&mut self, notification: ChatNotification) {
        self.chat_notifications.push(notification);
    }

    fn notify_controlled_room_creation(
        &mut self,
        notification: ControlledRoomCreationNotification,
    ) {
        self.controlled_room_creation_notifications
            .push(notification);
    }

    fn notify_controller_auth_transition(
        &mut self,
        notification: ControllerAuthTransitionNotification,
    ) {
        self.controller_auth_notifications.push(notification);
    }

    fn notify_user_change(&mut self, notification: UserChangeNotification) {
        self.user_change_notifications.push(notification);
    }

    fn schedule_reconnect(&mut self, delay_seconds: f64) {
        self.reconnect_delays.push(delay_seconds);
    }

    fn stop_reconnect(&mut self) {
        self.stop_reconnect_calls += 1;
    }

    fn notify_reconnect_transition(&mut self, notification: ReconnectTransitionNotification) {
        self.reconnect_notifications.push(notification);
    }

    fn notify_autoplay_countdown(&mut self, notification: AutoplayCountdownNotification) {
        self.autoplay_notifications.push(notification);
    }
}
