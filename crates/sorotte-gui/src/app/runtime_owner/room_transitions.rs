use super::*;

impl GuiPersistedConfigRuntimeOwner {
    pub(super) fn push_runtime_unavailable(handle: &GuiQueuedRuntimeBridgeHandle, message: String) {
        handle.push_actions([
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: message.clone(),
            },
            GuiShellAction::AnnounceSystemChatEvent(message),
        ]);
    }

    fn default_room_for_legacy_fallback(
        &self,
        projected_state: &SorotteGuiShellAppState,
    ) -> String {
        self.session_default_room
            .clone()
            .or_else(|| {
                projected_state
                    .saved_session_connect_target()
                    .map(|target| target.room)
            })
            .unwrap_or_else(|| {
                Self::detached_runtime_settings_for_state(projected_state)
                    .settings
                    .room
                    .unwrap_or_default()
            })
    }

    pub(super) fn augment_runtime_actions_for_room_transitions(
        &mut self,
        projected_state: &SorotteGuiShellAppState,
        actions: Vec<GuiShellAction>,
    ) -> Vec<GuiShellAction> {
        let mut current_room = projected_state.main_window.room_name.clone();
        let mut augmented_actions = Vec::with_capacity(actions.len());
        for action in actions {
            match action {
                GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot) => {
                    let next_room = snapshot.room_name.clone();
                    let room_transition_actions =
                        self.room_transition_confirmation_actions(&current_room, &next_room);
                    current_room = next_room;
                    augmented_actions
                        .push(GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot));
                    augmented_actions.extend(room_transition_actions);
                }
                other => augmented_actions.push(other),
            }
        }
        augmented_actions
    }

    fn room_transition_confirmation_actions(
        &mut self,
        previous_room: &str,
        next_room: &str,
    ) -> Vec<GuiShellAction> {
        if previous_room == next_room {
            return Vec::new();
        }

        let Some(request) = self.pending_room_change_request.take() else {
            return Vec::new();
        };

        match request {
            GuiPendingRoomChangeRequest::Join { .. }
            | GuiPendingRoomChangeRequest::ReturnToDefault { .. } => {}
        }

        vec![GuiShellAction::EditConfigurationText {
            section: "Connection",
            label: "Room",
            value: next_room.to_owned().into(),
        }]
    }

    pub(super) fn request_room_join_runtime(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
        room: String,
    ) {
        let Some(session) = self.session.as_mut() else {
            self.pending_room_change_request = None;
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                "Changing rooms requires an active session runtime.".to_owned(),
            );
            return;
        };

        match session.set_room(room.clone()) {
            Ok(()) => {
                self.pending_room_change_request = Some(GuiPendingRoomChangeRequest::Join {
                    requested_room: room,
                });
            }
            Err(error) => {
                self.pending_room_change_request = None;
                Self::push_runtime_error_notification(handle, projected_state, error);
            }
        }
    }

    pub(super) fn request_room_leave_runtime(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SorotteGuiShellAppState,
    ) {
        let previous_room = projected_state.main_window.room_name.clone();
        let default_room = self.default_room_for_legacy_fallback(projected_state);
        let Some(session) = self.session.as_mut() else {
            self.pending_room_change_request = None;
            Self::push_runtime_error_notification(
                handle,
                projected_state,
                "Returning to the default room requires an active session runtime.".to_owned(),
            );
            return;
        };

        let room_change_result = if default_room.is_empty() {
            session.set_room_with_legacy_fallback(default_room)
        } else {
            session.set_room(default_room)
        };
        match room_change_result {
            Ok(()) => {
                self.pending_room_change_request =
                    Some(GuiPendingRoomChangeRequest::ReturnToDefault { previous_room });
            }
            Err(error) => {
                self.pending_room_change_request = None;
                Self::push_runtime_error_notification(handle, projected_state, error);
            }
        }
    }
}
