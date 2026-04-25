use super::*;

impl GuiPersistedConfigRuntimeOwner {
    pub(in crate::app::runtime_owner) fn emit_gui_actions_to_attached_player_impl(
        &mut self,
        actions: &[GuiShellAction],
    ) {
        let Some(player) = self.player.as_mut().and_then(GuiOwnedPlayer::as_mpv_mut) else {
            return;
        };
        let mut already_emitted_osd_messages = BTreeSet::new();
        for action in actions {
            match action {
                GuiShellAction::PushChatMessage { sender, message } => {
                    if let Err(error) =
                        player.show_syncplay_legacy_chat_message(&format!("<{sender}> {message}"))
                    {
                        eprintln!(
                            "warning: failed to display GUI chat notification via mpv OSD: {error}"
                        );
                    }
                }
                GuiShellAction::PushTransientNotification { level, message } => {
                    already_emitted_osd_messages.insert(message.clone());
                    let kind = match level {
                        GuiTransientNotificationLevel::Info
                        | GuiTransientNotificationLevel::Success => {
                            LegacySyncplayOsdKind::Notification
                        }
                        GuiTransientNotificationLevel::Warning
                        | GuiTransientNotificationLevel::Error => LegacySyncplayOsdKind::Alert,
                    };
                    if let Err(error) = player.show_syncplay_legacy_message(message, kind) {
                        eprintln!(
                            "warning: failed to display GUI notification via mpv OSD: {error}"
                        );
                    }
                }
                GuiShellAction::AnnounceSystemChatEvent(message)
                    if already_emitted_osd_messages.insert(message.clone()) =>
                {
                    if let Err(error) = player
                        .show_syncplay_legacy_message(message, LegacySyncplayOsdKind::Notification)
                    {
                        eprintln!(
                            "warning: failed to display GUI system-chat event via mpv OSD: {error}"
                        );
                    }
                }
                _ => {}
            }
        }
    }

    pub(in crate::app::runtime_owner) fn drain_player_chat_input_impl(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        projected_state: &mut SyncplayGuiShellAppState,
    ) {
        let mut errors = Vec::new();
        let chat_ready = self
            .session
            .as_ref()
            .is_some_and(|session| session.attached_player_chat_input_ready());
        let unavailable_message = self
            .session
            .as_ref()
            .map(|session| session.attached_player_chat_input_unavailable_message())
            .unwrap_or_else(|| {
                "Chat input from the attached player requires an active session with chat support."
                    .to_owned()
            });
        loop {
            let pending_chat = self
                .player
                .as_mut()
                .and_then(|player| player.take_pending_chat_request());
            let Some(message) = pending_chat else {
                break;
            };
            if !chat_ready {
                errors.push(unavailable_message.clone());
                continue;
            }
            let Some(session) = self.session.as_mut() else {
                errors.push(unavailable_message.clone());
                continue;
            };
            let send_result = session.send_chat_message(message.clone());
            if let Err(error) = send_result {
                errors.push(format!(
                    "Chat input from the attached player could not be sent: {error}"
                ));
            }
        }

        if !errors.is_empty() {
            Self::push_actions_and_project(
                handle,
                projected_state,
                errors
                    .into_iter()
                    .flat_map(|message| {
                        [
                            GuiShellAction::PushTransientNotification {
                                level: GuiTransientNotificationLevel::Error,
                                message: message.clone(),
                            },
                            GuiShellAction::AnnounceSystemChatEvent(message),
                        ]
                    })
                    .collect(),
            );
        }
    }

    pub(in crate::app::runtime_owner) fn refresh_player_state_impl(&mut self) {
        let user_offset_seconds = self.user_offset_seconds;
        let Some(player) = self.player.as_mut() else {
            return;
        };
        let mut playback_updates = Vec::new();
        let mut media_load_outcomes = Vec::new();
        let mut local_file_updates = Vec::new();
        while let Some(update) = player.take_playback_telemetry_update() {
            playback_updates.push(update);
        }
        while let Some(outcome) = player.take_media_load_outcome() {
            media_load_outcomes.push(outcome);
        }
        while let Some(update) = player.take_local_file_update() {
            local_file_updates.push(update);
        }
        for update in playback_updates {
            if let Some(paused) = update.paused {
                self.player_paused = Some(paused);
            }
            if let Some(position_seconds) = update.position_seconds {
                self.player_position_seconds = Some(position_seconds - user_offset_seconds);
            }
        }
        for outcome in media_load_outcomes {
            self.handle_player_media_load_outcome(outcome);
        }
        for update in local_file_updates {
            let file_changed = Self::local_file_update_replaces_current_file(
                self.player_local_file.as_ref(),
                &update,
            );
            self.player_local_file = Some(update);
            self.player_local_file_placeholder = false;
            if file_changed || self.player_position_seconds.is_none() {
                self.player_position_seconds = Some(0.0);
            }
        }
        self.clamp_player_position_to_file_duration();
    }

    pub(super) fn player_local_file_ready_for_attached_sync(&self) -> bool {
        self.player_local_file.is_some() && !self.player_local_file_placeholder
    }
}
