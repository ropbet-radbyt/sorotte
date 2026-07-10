use super::super::*;

impl ClientSession {
    pub fn runtime_actions_for_outbound_chat_message(
        &self,
        message: String,
    ) -> Vec<ClientRuntimeAction> {
        if self.model.capabilities.chat != Some(true) {
            return Vec::new();
        }
        if self.chat_config.max_chat_message_length == 0 {
            return Vec::new();
        }
        let sanitized = Self::sanitize_chat_message_legacy_compatible(&message);
        let truncated = Self::truncate_chat_message_legacy_compatible(
            &sanitized,
            self.chat_config.max_chat_message_length,
        );
        vec![ClientRuntimeAction::SendChat { message: truncated }]
    }

    pub fn runtime_actions_for_local_ready_toggle(
        &self,
        manually_initiated: bool,
    ) -> Vec<ClientRuntimeAction> {
        if self.model.connection.username.is_none()
            || self.model.capabilities.readiness != Some(true)
        {
            return Vec::new();
        }
        vec![ClientRuntimeAction::SetReady {
            ready: !self.local_user_ready(),
            manually_initiated,
        }]
    }

    pub fn runtime_actions_for_local_user_ready_set(
        &self,
        username: String,
        ready: bool,
        manually_initiated: bool,
    ) -> Vec<ClientRuntimeAction> {
        if self.model.connection.username.is_none() {
            return Vec::new();
        }
        if username.is_empty() {
            if self.model.capabilities.readiness != Some(true) {
                return Vec::new();
            }
            return vec![ClientRuntimeAction::SetReady {
                ready,
                manually_initiated,
            }];
        }
        if self.model.capabilities.readiness != Some(true)
            || self.model.capabilities.set_others_readiness != Some(true)
        {
            return Vec::new();
        }
        if self.local_can_control() != Some(true) {
            return Vec::new();
        }
        vec![ClientRuntimeAction::SetReadyForUser {
            ready,
            manually_initiated,
            username,
        }]
    }

    pub fn runtime_actions_for_local_controller_auth_request(
        &mut self,
        room: String,
        password: String,
    ) -> Vec<ClientRuntimeAction> {
        if self.model.connection.username.is_none() {
            return Vec::new();
        }
        if self.model.capabilities.managed_rooms != Some(true) {
            return Vec::new();
        }
        if room.is_empty() {
            return Vec::new();
        }
        let password = Self::normalize_control_password_legacy_compatible(&password);
        self.model.controller.last_auth_password_attempt = Some(password.clone());
        vec![
            ClientRuntimeAction::NotifyControllerAuthTransition(
                ControllerAuthTransitionNotification::Attempting { room: room.clone() },
            ),
            ClientRuntimeAction::RequestControllerAuth { room, password },
        ]
    }

    pub fn runtime_actions_for_local_room_switch(
        &mut self,
        room: String,
    ) -> Vec<ClientRuntimeAction> {
        if self.model.capabilities.chat.is_none() {
            return Vec::new();
        }
        let (room, inline_password) =
            Self::normalize_runtime_controlled_room_input_legacy_compatible(room);
        if room.is_empty() {
            return Vec::new();
        }
        if let Some(password) = inline_password.as_deref() {
            self.remember_control_password_for_room(&room, password);
        }
        let tracked_room = self
            .model
            .controller
            .pending_local_room_switch_target
            .as_deref()
            .or(self.model.room.name.as_deref());
        if tracked_room != Some(room.as_str()) {
            self.model.controller.pending_local_room_switch_target = Some(room.clone());
            self.reset_playlist_index_transition_tracking();
        }
        let mut actions = vec![
            ClientRuntimeAction::SetRoom { room: room.clone() },
            ClientRuntimeAction::RequestUserList,
        ];
        let controller_password = inline_password
            .filter(|password| !password.is_empty())
            .or_else(|| self.model.controller.room_passwords.get(&room).cloned());
        if self.model.capabilities.managed_rooms == Some(true)
            && let Some(password) = controller_password
        {
            self.model.controller.last_auth_password_attempt = Some(password.clone());
            actions.push(ClientRuntimeAction::NotifyControllerAuthTransition(
                ControllerAuthTransitionNotification::Attempting { room: room.clone() },
            ));
            actions.push(ClientRuntimeAction::RequestControllerAuth { room, password });
        }
        actions
    }

    pub fn local_room_command_target_with_legacy_fallback(&self, default_room: &str) -> String {
        let Some(username) = self.model.connection.username.as_deref() else {
            return default_room.to_owned();
        };
        if let Some(room_name) = self
            .user_room(username)
            .filter(|room_name| !room_name.is_empty())
            .filter(|room_name| Self::is_controlled_room_name(room_name))
        {
            return room_name.to_owned();
        }
        self.user_file_name(username)
            .filter(|file_name| !file_name.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| default_room.to_owned())
    }

    pub fn runtime_actions_for_local_pause_toggle(&mut self) -> Vec<ClientRuntimeAction> {
        let now_seconds = unix_wall_clock_time_seconds_legacy_compatible();
        let target_paused = !self.effective_local_paused_state(now_seconds);
        self.runtime_actions_for_local_pause_change(target_paused, now_seconds)
    }

    pub fn runtime_actions_for_local_pause_set(
        &mut self,
        paused: bool,
    ) -> Vec<ClientRuntimeAction> {
        self.runtime_actions_for_local_pause_change(
            paused,
            unix_wall_clock_time_seconds_legacy_compatible(),
        )
    }

    pub fn runtime_actions_for_local_user_list_request(&self) -> Vec<ClientRuntimeAction> {
        if self.model.connection.username.is_none() || self.model.capabilities.chat.is_none() {
            return Vec::new();
        }
        vec![ClientRuntimeAction::RequestUserList]
    }
}
