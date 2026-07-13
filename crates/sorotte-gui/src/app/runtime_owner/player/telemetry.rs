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
        projected_state: &mut SorotteGuiShellAppState,
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
        let mut transport_updates = Vec::new();
        let mut media_load_outcomes = Vec::new();
        let mut local_file_updates = Vec::new();
        while player.take_command_progress().is_some() {
            // GUI coordinator commands remain completion-gated by the same
            // transport observations forwarded below. Tracked open progress
            // is drained here so adapter state remains bounded; load failures
            // are surfaced through PlayerMediaLoadOutcome.
        }
        while let Some(update) = player.take_playback_telemetry_update() {
            playback_updates.push(update);
        }
        while let Some(update) = player.take_transport_telemetry_update() {
            transport_updates.push(update);
        }
        while let Some(outcome) = player.take_media_load_outcome() {
            media_load_outcomes.push(outcome);
        }
        while let Some(update) = player.take_local_file_update() {
            local_file_updates.push(update);
        }
        let now = Instant::now();
        if self
            .pending_attached_player_pause_command
            .is_some_and(|pending| pending.suppress_until <= now)
        {
            self.pending_attached_player_pause_command = None;
        }
        for update in playback_updates {
            if let Some(paused_for_cache) = update.paused_for_cache {
                self.player_paused_for_cache = Some(paused_for_cache);
            }
            if let Some(cache_buffering_percent) = update.cache_buffering_percent {
                self.player_cache_buffering_percent = Some(cache_buffering_percent);
            }
            if (update.paused_for_cache.is_some() || update.cache_buffering_percent.is_some())
                && let Some(session) = self.session.as_mut()
                && let Err(error) = session.sync_local_playback_cache_state(
                    update.paused_for_cache,
                    update.cache_buffering_percent,
                )
            {
                eprintln!(
                    "warning: failed to mirror attached-player cache buffering state into the session runtime: {error}"
                );
            }
            if let Some(paused) = update.paused
                && self.player_paused_for_cache != Some(true)
            {
                let accept_paused = match self.pending_attached_player_pause_command {
                    Some(pending) if pending.suppress_until > now => {
                        self.player_paused = Some(pending.target_paused);
                        paused == pending.target_paused
                    }
                    _ => true,
                };
                if accept_paused {
                    self.player_paused = Some(paused);
                }
            }
            if let Some(position_seconds) = update.position_seconds {
                self.player_position_seconds = Some(position_seconds - user_offset_seconds);
            }
        }
        for outcome in media_load_outcomes {
            self.handle_player_media_load_outcome(outcome);
        }
        for mut update in local_file_updates {
            if let Some(override_update) = self.logical_media_override_for_loaded_target(&update) {
                update = override_update;
            }
            let file_changed = Self::local_file_update_replaces_current_file(
                self.player_local_file.as_ref(),
                &update,
            );
            if file_changed {
                let _ = self
                    .interrupt_attached_playback_recovery_impl("observed media transport change");
                let logical_id = logical_media_id_for_local_file_update(&update);
                let kind = if update.path.as_deref().is_some_and(browser_is_url)
                    || browser_is_url(&update.name)
                {
                    MediaTransportKind::NetworkVod
                } else {
                    MediaTransportKind::LocalFile
                };
                if let Some(session) = self.session.as_mut()
                    && let Err(error) = session.prepare_attached_playback_media(
                        logical_id,
                        kind,
                        MediaLoadIntent::TransportRefresh,
                        system_time_seconds(),
                    )
                {
                    eprintln!(
                        "warning: failed to prepare attached-player logical media generation: {error}"
                    );
                }
            }
            self.player_local_file = Some(update);
            self.player_local_file_placeholder = false;
            if file_changed || self.player_position_seconds.is_none() {
                self.player_position_seconds = Some(0.0);
            }
        }
        for update in transport_updates {
            let update = transport_update_on_room_timeline(update, user_offset_seconds);
            if let Some(paused_for_cache) = update.paused_for_cache {
                self.player_paused_for_cache = Some(paused_for_cache);
            }
            if let Some(position_seconds) = update.position_seconds {
                self.player_position_seconds = Some(position_seconds);
            }
            if let Some(logical_pause) = update.logical_pause
                && self.player_paused_for_cache != Some(true)
            {
                self.player_paused = Some(logical_pause);
            }
            let actions = self.session.as_mut().and_then(|session| {
                match session.sync_attached_player_transport_telemetry(
                    update,
                    system_time_seconds(),
                ) {
                    Ok(actions) => Some(actions),
                    Err(error) => {
                        eprintln!(
                            "warning: failed to feed attached-player transport telemetry to client-core coordinator: {error}"
                        );
                        None
                    }
                }
            });
            if let Some(actions) = actions {
                let _ = self
                    .apply_attached_player_runtime_actions_impl(actions, "transport observation");
            }
        }
        let quality_suggestion = self
            .session
            .as_mut()
            .and_then(|session| session.take_streaming_quality_downgrade_suggestion());
        if let Some(suggestion) = quality_suggestion {
            let reason = match suggestion.reason {
                StreamingQualitySuggestionReason::RepeatedRebuffering => {
                    "repeated buffering was observed"
                }
                StreamingQualitySuggestionReason::InsufficientObservedInputRate => {
                    "the observed input rate is below the selected stream's needs"
                }
            };
            self.queue_stream_warning(format!(
                "Stream quality suggestion: change from '{}' to '{}' because {reason}. Sorotte did not change quality automatically.",
                suggestion.current.config_value(),
                suggestion.recommended.config_value(),
            ));
        }
        let timeout_action = self
            .session
            .as_mut()
            .and_then(|session| session.take_playback_barrier_timeout_action());
        match timeout_action {
            Some(PlaybackBarrierTimeoutAction::RemainPaused) => self.queue_stream_warning(
                "Playback start timed out and the room was kept paused. The controller can start it manually when ready."
                    .to_owned(),
            ),
            Some(PlaybackBarrierTimeoutAction::AskController) => self.queue_stream_warning(
                "Playback start timed out. The room is paused and waiting for the controller to decide whether to continue."
                    .to_owned(),
            ),
            Some(PlaybackBarrierTimeoutAction::Continue) | None => {}
        }
        self.clamp_player_position_to_file_duration();
    }

    pub(super) fn player_local_file_ready_for_attached_sync(&self) -> bool {
        self.player_local_file.is_some() && !self.player_local_file_placeholder
    }

    fn logical_media_override_for_loaded_target(
        &mut self,
        update: &LocalFileUpdate,
    ) -> Option<LocalFileUpdate> {
        let pending = self.pending_logical_media_override.as_ref()?;
        let loaded_target = pending.loaded_target_secret.as_str();
        let current_matches_logical = self.player_local_file.as_ref().is_some_and(|current| {
            Self::local_file_identity_matches(current, &pending.logical_file)
        });
        let update_is_url =
            update.path.as_deref().is_some_and(browser_is_url) || browser_is_url(&update.name);
        let matches_path = update
            .path
            .as_deref()
            .is_some_and(|path| path == loaded_target);
        let matches_name = update.name == loaded_target;
        if !(matches_path || matches_name || (current_matches_logical && update_is_url)) {
            return None;
        }
        Some(pending.logical_file.clone())
    }
}

fn transport_update_on_room_timeline(
    mut update: sorotte_player_api::PlayerTransportTelemetryUpdate,
    user_offset_seconds: f64,
) -> sorotte_player_api::PlayerTransportTelemetryUpdate {
    update.position_seconds = update
        .position_seconds
        .map(|position| position - user_offset_seconds);
    update.seekable_ranges = update.seekable_ranges.map(|ranges| {
        ranges
            .into_iter()
            .map(|range| range.shifted(-user_offset_seconds))
            .collect()
    });
    update
}

#[cfg(test)]
mod transport_timeline_tests {
    use super::transport_update_on_room_timeline;
    use sorotte_player_api::{
        PlayerMediaGeneration, PlayerObservationTimestamp, PlayerSeekableRange,
        PlayerTransportPhase, PlayerTransportTelemetryUpdate,
    };
    use std::time::Duration;

    fn update(phase: PlayerTransportPhase, player_position: f64) -> PlayerTransportTelemetryUpdate {
        let mut update = PlayerTransportTelemetryUpdate::new(
            PlayerMediaGeneration::new(1),
            PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(1)),
        )
        .with_phase(phase)
        .with_position_seconds(player_position);
        update.seekable_ranges = Some(vec![PlayerSeekableRange::new(
            player_position - 10.0,
            player_position + 30.0,
        )]);
        update
    }

    #[test]
    fn positive_offset_is_removed_for_barrier_and_normal_sync_observations() {
        let normalized =
            transport_update_on_room_timeline(update(PlayerTransportPhase::ReadyPaused, 15.0), 5.0);
        assert_eq!(normalized.position_seconds, Some(10.0));
        assert_eq!(
            normalized.seekable_ranges,
            Some(vec![PlayerSeekableRange::new(0.0, 40.0)])
        );
    }

    #[test]
    fn negative_offset_is_removed_for_rebuffer_recovery_observations() {
        let normalized =
            transport_update_on_room_timeline(update(PlayerTransportPhase::Rebuffering, 5.0), -5.0);
        assert_eq!(normalized.position_seconds, Some(10.0));
        assert_eq!(
            normalized.seekable_ranges,
            Some(vec![PlayerSeekableRange::new(0.0, 40.0)])
        );
    }
}
