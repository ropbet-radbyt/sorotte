use super::*;

impl GuiClientCoreChatSessionRuntimeAdapter {
    pub(super) fn drain_gui_actions_impl(
        &mut self,
        state: &SorotteGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        let mut actions = Vec::new();
        let mut trailing_actions = Vec::new();
        let language = Some(state.runtime_language_tag_legacy_compatible());
        if let Err(error) = self.runtime.run_user_change_notifications_if_needed() {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: format!("Client-core user-change dispatch failed: {error}"),
            });
        } else {
            for notification in self.runtime.drain_user_change_notifications() {
                self.note_user_change(notification.clone());
                if let Some(action) = Self::user_change_action(notification, language) {
                    trailing_actions.push(action);
                }
            }
        }
        if let Err(error) = self.runtime.run_reconnect_transition_if_needed() {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: format!("Client-core reconnect transition dispatch failed: {error}"),
            });
        } else {
            trailing_actions.extend(
                self.runtime
                    .drain_reconnect_notifications()
                    .into_iter()
                    .flat_map(|notification| {
                        Self::reconnect_transition_actions(notification, language)
                    }),
            );
        }
        if let Err(error) = self.runtime.run_reconnect_state_restore_if_needed() {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: format!("Client-core reconnect state-restore dispatch failed: {error}"),
            });
        }
        if let Err(error) = self.runtime.run_reconnect_playlist_restore_if_needed() {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: format!("Client-core reconnect playlist-restore dispatch failed: {error}"),
            });
        }
        if let Err(error) = self
            .runtime
            .run_reconnect_state_restore_validation_if_needed()
        {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: format!("Client-core reconnect validation dispatch failed: {error}"),
            });
        }
        if !actions.iter().any(|action| {
            matches!(
                action,
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Error,
                    ..
                }
            )
        }) {
            trailing_actions.extend(
                self.runtime
                    .drain_reconnect_notifications()
                    .into_iter()
                    .flat_map(|notification| {
                        Self::reconnect_transition_actions(notification, language)
                    }),
            );
        } else {
            self.runtime.drain_reconnect_notifications();
        }
        if let Err(error) = self
            .runtime
            .run_controlled_room_creation_notifications_if_needed()
        {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: format!("Client-core controlled-room dispatch failed: {error}"),
            });
        } else {
            trailing_actions.extend(
                self.runtime
                    .drain_controlled_room_creation_notifications()
                    .into_iter()
                    .flat_map(Self::controlled_room_creation_action),
            );
        }
        if let Err(error) = self.runtime.run_controller_reidentify_if_needed() {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: format!("Client-core controller reidentify dispatch failed: {error}"),
            });
        }
        if let Err(error) = self.runtime.run_controller_auth_notifications_if_needed() {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: format!("Client-core controller-auth dispatch failed: {error}"),
            });
        } else {
            trailing_actions.extend(
                self.runtime
                    .drain_controller_auth_notifications()
                    .into_iter()
                    .flat_map(|notification| {
                        Self::controller_auth_transition_action(notification, language)
                    }),
            );
        }
        self.sync_autoplay_runtime(&mut actions);
        trailing_actions.extend(
            self.runtime
                .drain_autoplay_notifications()
                .into_iter()
                .flat_map(Self::autoplay_countdown_action),
        );
        if let Err(error) = self.runtime.run_chat_notifications_if_needed() {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: format!("Client-core chat notification dispatch failed: {error}"),
            });
        }
        self.queue_periodic_state_sync_heartbeat_if_due();

        let main_window_runtime_snapshot = self.main_window_runtime_snapshot(state);
        let mut interaction_state = state.clone();
        if let Some(snapshot) = main_window_runtime_snapshot.as_ref() {
            debug_assert!(
                interaction_state.apply_main_window_runtime_snapshot(snapshot.clone()),
                "runtime-projected main-window snapshots should remain shell-applicable"
            );
        }
        let interaction_runtime_snapshot = self.interaction_runtime_snapshot(
            state,
            &interaction_state,
            main_window_runtime_snapshot
                .as_ref()
                .map(|snapshot| snapshot.playlist.len())
                .unwrap_or_else(|| state.main_window.playlist.len()),
        );
        let menu_dialog_runtime_snapshot = self.menu_dialog_runtime_snapshot(
            state,
            main_window_runtime_snapshot
                .as_ref()
                .map(|snapshot| snapshot.shared_playlist_enabled)
                .unwrap_or(state.main_window.shared_playlist_enabled),
        );
        if let Some(snapshot) = main_window_runtime_snapshot
            && snapshot != MainWindowRuntimeSnapshot::from_shell_state(&state.main_window)
        {
            actions.push(GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot));
        }
        if let Some(snapshot) = interaction_runtime_snapshot {
            actions.push(GuiShellAction::ApplyGuiInteractionRuntimeSnapshot(snapshot));
        }
        if let Some(snapshot) = menu_dialog_runtime_snapshot {
            actions.push(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(snapshot));
        }

        actions.extend(
            self.runtime
                .drain_chat_notifications()
                .into_iter()
                .map(|notification| match notification {
                    ChatNotification::Message { username, message } => {
                        GuiShellAction::PushChatMessage {
                            sender: username.unwrap_or_else(|| "Server".to_owned()),
                            message,
                        }
                    }
                }),
        );
        actions.extend(trailing_actions);
        actions
    }
}
