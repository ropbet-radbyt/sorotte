use super::super::*;

impl ClientSession {
    pub fn runtime_actions_for_outbound_chat_message(
        &self,
        message: String,
    ) -> Vec<ClientRuntimeAction> {
        if !self.server_chat_supported() {
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
        &mut self,
        manually_initiated: bool,
    ) -> Vec<ClientRuntimeAction> {
        self.runtime_actions_for_local_ready_toggle_from(
            manually_initiated,
            sorotte_protocol::DirectReadinessSurface::RemoteControlSurface,
        )
    }

    pub fn runtime_actions_for_local_ready_toggle_from(
        &mut self,
        manually_initiated: bool,
        surface: sorotte_protocol::DirectReadinessSurface,
    ) -> Vec<ClientRuntimeAction> {
        if self.model.connection.username.is_none() || !self.server_readiness_supported() {
            return Vec::new();
        }
        if self.server_readiness_v2_supported() {
            let Some(username) = self.model.connection.username.clone() else {
                return Vec::new();
            };
            let desired = if self.displayed_user_readiness_intent(&username)
                == Some(UserReadinessIntent::Ready)
            {
                UserReadinessIntent::NotReady
            } else {
                UserReadinessIntent::Ready
            };
            return self.runtime_actions_for_direct_readiness_intent(desired, surface, None);
        }
        vec![ClientRuntimeAction::SetReady {
            ready: !self.local_user_ready(),
            manually_initiated,
        }]
    }

    pub fn runtime_actions_for_local_user_ready_set(
        &mut self,
        username: String,
        ready: bool,
        manually_initiated: bool,
    ) -> Vec<ClientRuntimeAction> {
        self.runtime_actions_for_local_user_ready_set_from(
            username,
            ready,
            manually_initiated,
            sorotte_protocol::DirectReadinessSurface::RemoteControlSurface,
        )
    }

    pub fn runtime_actions_for_local_user_ready_set_from(
        &mut self,
        username: String,
        ready: bool,
        manually_initiated: bool,
        surface: sorotte_protocol::DirectReadinessSurface,
    ) -> Vec<ClientRuntimeAction> {
        if self.model.connection.username.is_none() {
            return Vec::new();
        }
        if username.is_empty() {
            if !self.server_readiness_supported() {
                return Vec::new();
            }
            if self.server_readiness_v2_supported() {
                return self.runtime_actions_for_direct_readiness_intent(
                    if ready {
                        UserReadinessIntent::Ready
                    } else {
                        UserReadinessIntent::NotReady
                    },
                    surface,
                    None,
                );
            }
            return vec![ClientRuntimeAction::SetReady {
                ready,
                manually_initiated,
            }];
        }
        if !self.server_readiness_supported() || !self.server_set_others_readiness_supported() {
            return Vec::new();
        }
        if self.local_can_control() != Some(true) {
            return Vec::new();
        }
        if self.server_readiness_v2_supported() {
            return self.runtime_actions_for_direct_readiness_intent(
                if ready {
                    UserReadinessIntent::Ready
                } else {
                    UserReadinessIntent::NotReady
                },
                surface,
                Some(username),
            );
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
        password: SecretValue,
    ) -> Vec<ClientRuntimeAction> {
        if self.model.connection.username.is_none() {
            return Vec::new();
        }
        if !self.server_managed_rooms_supported() {
            return Vec::new();
        }
        if room.is_empty() {
            return Vec::new();
        }
        let password = SecretValue::new(Self::normalize_control_password_legacy_compatible(
            password.expose_secret(),
        ));
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
        if !self.is_active() {
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
            self.model.readiness.reconnect_token = None;
            if self.server_readiness_v2_supported() {
                self.stop_autoplay_countdown();
            }
            self.reset_readiness_v2_for_new_room();
            self.model.controller.pending_local_room_switch_target = Some(room.clone());
            self.reset_playlist_index_transition_tracking();
        }
        let mut actions = vec![
            ClientRuntimeAction::SetRoom { room: room.clone() },
            ClientRuntimeAction::RequestUserList,
        ];
        let controller_password = inline_password
            .filter(|password| !password.is_empty())
            .map(SecretValue::from)
            .or_else(|| self.model.controller.room_passwords.get(&room).cloned());
        if self.server_managed_rooms_supported()
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
        if self.model.connection.username.is_none() || !self.is_active() {
            return Vec::new();
        }
        vec![ClientRuntimeAction::RequestUserList]
    }
}
