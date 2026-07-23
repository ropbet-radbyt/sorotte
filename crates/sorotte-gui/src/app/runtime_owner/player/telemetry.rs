use super::*;
use crate::app::runtime_owner::{
    GuiAttachedNativeSeekTracker, GuiAttachedPlayerPositionObservation,
    GuiCorePlayerConfigurationHealth, GuiPendingAttachedCoordinatorSeek,
    GuiStreamingDegradationOrigin,
};
use sorotte_player_api::{
    PlayerObservationTimestamp, PlayerTransportPhase, PlayerTransportTelemetryUpdate,
};
use sorotte_player_mpv::{
    MPV_SEEK_COMPLETION_TOLERANCE_SECONDS, MpvNetworkMediaPolicyOutcome,
    MpvNetworkMediaPolicyState, MpvNetworkOptionsHookHealth, MpvNetworkOptionsHookHealthTransition,
    MpvNetworkOptionsRuntimeHealthSnapshot,
};

const ATTACHED_NATIVE_SEEK_THRESHOLD_SECONDS: f64 = 1.0;
const ATTACHED_NATIVE_SEEK_MAX_OBSERVATION_AGE_SECONDS: f64 = 2.0;

impl GuiAttachedNativeSeekTracker {
    fn disarm_untrusted_position_evidence(&mut self) {
        self.position_anchor = None;
        self.interval_disarmed = true;
        self.seeking_since_anchor = false;
    }

    fn observe(&mut self, update: &PlayerTransportTelemetryUpdate) -> Option<bool> {
        let Some(media_generation) = update.media_generation.map(|generation| generation.get())
        else {
            if update.position_seconds.is_some() {
                self.disarm_untrusted_position_evidence();
            }
            return None;
        };
        if self.media_generation != Some(media_generation) {
            *self = Self {
                media_generation: Some(media_generation),
                ..Self::default()
            };
        }

        let observation_clock = update.observed_at.and_then(|timestamp| {
            let observed_at_seconds = timestamp.elapsed_since_adapter_start().as_secs_f64();
            let delivery_reference_seconds = timestamp
                .delivery_reference_since_adapter_start()
                .as_secs_f64();
            let observation_age_seconds = delivery_reference_seconds - observed_at_seconds;
            (observed_at_seconds.is_finite()
                && delivery_reference_seconds.is_finite()
                && observation_age_seconds >= 0.0)
                .then_some((observed_at_seconds, observation_age_seconds))
        });
        if update.observed_at.is_some() && observation_clock.is_none() {
            self.disarm_untrusted_position_evidence();
            return None;
        }
        let observed_at_seconds =
            observation_clock.map(|(observed_at_seconds, _)| observed_at_seconds);
        if observed_at_seconds.is_some_and(|observed_at_seconds| {
            self.last_observed_at_seconds
                .is_some_and(|latest| observed_at_seconds < latest)
        }) {
            self.disarm_untrusted_position_evidence();
            return None;
        }
        if let Some(observed_at_seconds) = observed_at_seconds {
            self.last_observed_at_seconds = Some(observed_at_seconds);
        }
        let position_clock_is_usable = update.position_seconds.is_none()
            || observation_clock.is_some_and(|(_, observation_age_seconds)| {
                observation_age_seconds <= ATTACHED_NATIVE_SEEK_MAX_OBSERVATION_AGE_SECONDS
            });
        if !position_clock_is_usable {
            self.disarm_untrusted_position_evidence();
        }

        let previously_seeking = self.seeking == Some(true)
            || self.phase == Some(PlayerTransportPhase::Seeking)
            || self.seeking_since_anchor;
        let state_transition = update.phase.is_some_and(|phase| self.phase != Some(phase))
            || update
                .playback_rate
                .is_some_and(|rate| self.playback_rate != Some(rate))
            || update
                .logical_pause
                .is_some_and(|paused| self.logical_pause != Some(paused))
            || update
                .paused_for_cache
                .is_some_and(|paused| self.paused_for_cache != Some(paused))
            || update
                .core_idle
                .is_some_and(|core_idle| self.core_idle != Some(core_idle));

        if let Some(phase) = update.phase {
            self.phase = Some(phase);
        }
        if let Some(playback_rate) = update.playback_rate {
            self.playback_rate =
                (playback_rate.is_finite() && playback_rate > 0.0).then_some(playback_rate);
        }
        if let Some(logical_pause) = update.logical_pause {
            self.logical_pause = Some(logical_pause);
        }
        if let Some(paused_for_cache) = update.paused_for_cache {
            self.paused_for_cache = Some(paused_for_cache);
        }
        if let Some(seeking) = update.seeking {
            self.seeking = Some(seeking);
        }
        if let Some(core_idle) = update.core_idle {
            self.core_idle = Some(core_idle);
        }

        if self.phase == Some(PlayerTransportPhase::Loading) {
            self.disarm_untrusted_position_evidence();
            return None;
        }

        let currently_seeking =
            self.seeking == Some(true) || self.phase == Some(PlayerTransportPhase::Seeking);
        if currently_seeking {
            self.seeking_since_anchor = true;
            if let (Some(anchor), Some(observed_at_seconds)) =
                (self.position_anchor.as_mut(), observed_at_seconds)
            {
                anchor.observed_at_seconds = anchor.observed_at_seconds.max(observed_at_seconds);
            }
            return None;
        }
        if state_transition && !previously_seeking {
            self.interval_disarmed = true;
        }
        if !position_clock_is_usable {
            return None;
        }

        let (position_seconds, observed_at_seconds) =
            update.position_seconds.zip(observed_at_seconds)?;
        let Some((phase, playback_rate, logical_pause, paused_for_cache, false, core_idle)) = self
            .phase
            .zip(self.playback_rate)
            .zip(self.logical_pause)
            .zip(self.paused_for_cache)
            .zip(self.seeking)
            .zip(self.core_idle)
            .map(
                |(
                    ((((phase, playback_rate), logical_pause), paused_for_cache), seeking),
                    core_idle,
                )| {
                    (
                        phase,
                        playback_rate,
                        logical_pause,
                        paused_for_cache,
                        seeking,
                        core_idle,
                    )
                },
            )
        else {
            self.position_anchor = None;
            self.interval_disarmed = true;
            return Some(false);
        };
        let current = GuiAttachedPlayerPositionObservation {
            media_generation,
            observed_at_seconds,
            position_seconds,
            phase,
            playback_rate,
            logical_pause,
            paused_for_cache,
            core_idle,
        };
        if !current.is_stable() {
            self.position_anchor = Some(current);
            self.interval_disarmed = true;
            return Some(false);
        }
        if self.interval_disarmed && !self.seeking_since_anchor {
            self.position_anchor = Some(current);
            self.interval_disarmed = false;
            return Some(false);
        }

        let unexpected_position_jump = self.position_anchor.is_some_and(|previous| {
            previous.media_generation == current.media_generation
                && previous.is_stable()
                && current.observed_at_seconds >= previous.observed_at_seconds
                && (self.seeking_since_anchor || previous.same_motion_regime(current))
                && {
                    let elapsed_seconds =
                        current.observed_at_seconds - previous.observed_at_seconds;
                    let expected_advance = if previous.logical_pause {
                        0.0
                    } else {
                        elapsed_seconds * previous.playback_rate
                    };
                    let actual_advance = current.position_seconds - previous.position_seconds;
                    actual_advance < expected_advance - ATTACHED_NATIVE_SEEK_THRESHOLD_SECONDS
                        || actual_advance
                            > expected_advance + ATTACHED_NATIVE_SEEK_THRESHOLD_SECONDS
                }
        });
        self.position_anchor = Some(current);
        self.interval_disarmed = false;
        self.seeking_since_anchor = false;
        Some(unexpected_position_jump)
    }
}

impl GuiAttachedPlayerPositionObservation {
    fn is_stable(self) -> bool {
        !self.paused_for_cache
            && self.playback_rate.is_finite()
            && self.playback_rate > 0.0
            && matches!(
                (self.phase, self.logical_pause, self.core_idle),
                (PlayerTransportPhase::Playing, false, false)
                    | (PlayerTransportPhase::ReadyPaused, true, _)
            )
    }

    fn same_motion_regime(self, other: Self) -> bool {
        self.phase == other.phase
            && self.playback_rate == other.playback_rate
            && self.logical_pause == other.logical_pause
            && self.paused_for_cache == other.paused_for_cache
            && self.core_idle == other.core_idle
    }
}

impl GuiPersistedConfigRuntimeOwner {
    pub(in crate::app::runtime_owner) fn note_attached_coordinator_seek_dispatched(
        &mut self,
        target_position_seconds: f64,
    ) {
        self.pending_attached_coordinator_seek =
            target_position_seconds
                .is_finite()
                .then_some(GuiPendingAttachedCoordinatorSeek {
                    player_attachment_epoch: self.player_attachment_epoch,
                    media_generation: self.attached_native_seek_tracker.media_generation,
                    issued_after_observed_at_seconds: self
                        .attached_native_seek_tracker
                        .last_observed_at_seconds,
                    target_position_seconds,
                });
    }

    fn consume_matching_attached_coordinator_seek(
        &mut self,
        media_generation: Option<PlayerMediaGeneration>,
        observed_at: Option<PlayerObservationTimestamp>,
        position_seconds: f64,
    ) -> bool {
        let Some(pending) = self.pending_attached_coordinator_seek else {
            return false;
        };
        let observed_generation = media_generation.map(PlayerMediaGeneration::get);
        let observed_at_seconds =
            observed_at.map(|timestamp| timestamp.elapsed_since_adapter_start().as_secs_f64());
        let lifecycle_matches = pending.player_attachment_epoch == self.player_attachment_epoch
            && pending
                .media_generation
                .zip(observed_generation)
                .is_none_or(|(expected, observed)| expected == observed)
            && pending
                .issued_after_observed_at_seconds
                .zip(observed_at_seconds)
                .is_none_or(|(issued_after, observed)| observed > issued_after);
        if !lifecycle_matches {
            self.pending_attached_coordinator_seek = None;
            return false;
        }
        if (pending.target_position_seconds - position_seconds).abs()
            <= MPV_SEEK_COMPLETION_TOLERANCE_SECONDS
        {
            self.pending_attached_coordinator_seek = None;
            return true;
        }
        false
    }

    fn sync_attached_player_position_observation(
        &mut self,
        position_seconds: f64,
        unexpected_position_jump: bool,
    ) -> bool {
        let position_already_owned_by_session = self.session.as_ref().is_some_and(|session| {
            session
                .local_position_seconds()
                .is_some_and(|known_position| {
                    (known_position - position_seconds).abs()
                        <= MPV_SEEK_COMPLETION_TOLERANCE_SECONDS
                })
        });

        let mut publish_succeeded = true;
        if unexpected_position_jump && !position_already_owned_by_session {
            let _ = self.interrupt_attached_playback_recovery_impl("native player seek");
            publish_succeeded = match self
                .session
                .as_mut()
                .map(|session| session.record_manual_seek_to_position(position_seconds))
            {
                Some(Ok(true)) | None => true,
                Some(Ok(false)) => false,
                Some(Err(error)) => {
                    eprintln!(
                        "warning: failed to publish native attached-player seek to the room: {error}"
                    );
                    false
                }
            };
        }

        if publish_succeeded
            && let Some(session) = self.session.as_mut()
            && let Err(error) = session.sync_local_playback_telemetry(
                // Pause edges have their own causal classifier. Mirroring the
                // just-observed pause value here would erase a native Play or
                // Pause edge before transport telemetry can classify it.
                None,
                Some(position_seconds),
            )
        {
            eprintln!(
                "warning: failed to ground the session position in attached-player telemetry: {error}"
            );
        }
        publish_succeeded
    }

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
        let mut command_progress_updates = Vec::new();
        let mut media_load_outcomes = Vec::new();
        let mut local_file_updates = Vec::new();
        while let Some(progress) = player.take_command_progress() {
            command_progress_updates.push(progress);
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
        let mut hook_health_transitions = Vec::new();
        let mut media_policy_outcomes = Vec::new();
        let mut network_options_snapshot = None;
        let mut mpv_connected = true;
        if let Some(player) = player.as_mpv_mut() {
            while let Some(transition) = player.take_network_options_hook_health_transition() {
                hook_health_transitions.push(transition);
            }
            while let Some(outcome) = player.take_network_media_policy_outcome() {
                media_policy_outcomes.push(outcome);
            }
            network_options_snapshot = Some(player.network_options_runtime_health_snapshot());
            mpv_connected = player.is_connected();
        }
        for transition in hook_health_transitions {
            match transition {
                MpvNetworkOptionsHookHealthTransition::Recovered => {
                    self.record_network_options_hook_recovered();
                }
                MpvNetworkOptionsHookHealthTransition::Degraded(error) if mpv_connected => {
                    self.mark_network_options_hook_degraded(format!(
                        "mpv playback remains available, but Sorotte's core streaming-settings hook needs retry or player restart: {error}"
                    ));
                }
                MpvNetworkOptionsHookHealthTransition::Degraded(error) => {
                    self.player_apply_state.mark_streaming_apply_failed();
                    self.detach_player();
                    self.player_unavailability_reason = Some(format!(
                        "mpv JSON IPC became unavailable while maintaining Sorotte's core streaming-settings hook: {error}"
                    ));
                    return;
                }
            }
        }
        for outcome in media_policy_outcomes {
            match outcome {
                MpvNetworkMediaPolicyOutcome::NoActiveMedia
                | MpvNetworkMediaPolicyOutcome::LocalMediaUnchanged
                | MpvNetworkMediaPolicyOutcome::NetworkMediaUpdated => {
                    self.record_network_media_transition_recovered();
                }
                MpvNetworkMediaPolicyOutcome::Failed(error) if mpv_connected => {
                    self.mark_network_media_transition_apply_failed(format!(
                        "mpv switched to network media, but configured streaming settings could not be applied to the new file: {error}"
                    ));
                }
                MpvNetworkMediaPolicyOutcome::Failed(error) => {
                    self.player_apply_state.mark_streaming_apply_failed();
                    self.detach_player();
                    self.player_unavailability_reason = Some(format!(
                        "mpv JSON IPC became unavailable while applying configured streaming settings to newly active network media: {error}"
                    ));
                    return;
                }
            }
        }
        if let Some(snapshot) = network_options_snapshot
            && !self.reconcile_network_options_runtime_health_snapshot(snapshot, mpv_connected)
        {
            return;
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
            if let Some(position_seconds) = update.position_seconds {
                self.player_position_seconds = Some(position_seconds - user_offset_seconds);
            }
            if let Some(paused) = update.paused
                && self.player_paused_for_cache != Some(true)
            {
                let application_pause_command_active = self
                    .pending_attached_player_pause_command
                    .is_some_and(|pending| pending.suppress_until > now);
                let previous_paused = self.player_paused;
                let accept_paused = match self.pending_attached_player_pause_command {
                    Some(pending) if pending.suppress_until > now => {
                        self.player_paused = Some(pending.target_paused);
                        paused == pending.target_paused
                    }
                    _ => true,
                };
                if accept_paused {
                    if !application_pause_command_active
                        && previous_paused != Some(paused)
                        && paused
                        && self.attached_player_position_is_end_of_file()
                        && let Some(session) = self.session.as_mut()
                        && let Err(error) =
                            session.observe_external_player_end_of_file(system_time_seconds())
                    {
                        eprintln!(
                            "warning: failed to classify attached-player EOF as a technical transition: {error}"
                        );
                    }
                    self.player_paused = Some(paused);
                }
            }
        }
        for outcome in media_load_outcomes {
            self.handle_playlist_media_load_outcome(&outcome);
            self.handle_player_media_load_outcome(outcome);
        }
        for mut update in local_file_updates {
            self.handle_untracked_playlist_local_file_observation(&update);
            let tracked_playlist_load_unconfirmed =
                self.tracked_playlist_resolution_load_matches_local_file(&update);
            let mut logical_override_confirmed = None;
            if let Some((override_update, confirmed)) =
                self.logical_media_override_for_loaded_target(&update)
            {
                update = override_update;
                logical_override_confirmed = Some(confirmed);
            }
            let file_changed = Self::local_file_update_replaces_current_file(
                self.player_local_file.as_ref(),
                &update,
            );
            if file_changed {
                self.pending_local_attached_pause_override = None;
                self.pending_attached_coordinator_seek = None;
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
            self.player_local_file_placeholder = tracked_playlist_load_unconfirmed
                || logical_override_confirmed.is_some_and(|confirmed| !confirmed);
            if file_changed || self.player_position_seconds.is_none() {
                self.player_position_seconds = Some(0.0);
            }
        }
        // A tracked load's terminal result is the final authority for the
        // provisional identity observed in the queues above. Processing it
        // last prevents an earlier file-loaded observation from resurrecting
        // media that the same command subsequently rejected.
        for progress in command_progress_updates {
            self.handle_playlist_resolution_command_progress(progress);
        }
        for update in transport_updates {
            self.reconcile_pending_logical_override_media_generation(update.media_generation);
            let update = transport_update_on_room_timeline(update, user_offset_seconds);
            let previous_native_seek_tracker = self.attached_native_seek_tracker;
            let native_seek_classification = self.attached_native_seek_tracker.observe(&update);
            if let Some(paused_for_cache) = update.paused_for_cache {
                self.player_paused_for_cache = Some(paused_for_cache);
            }
            if let Some(position_seconds) = update.position_seconds {
                if let Some(unexpected_position_jump) = native_seek_classification {
                    let coordinator_seek_owned = self.consume_matching_attached_coordinator_seek(
                        update.media_generation,
                        update.observed_at,
                        position_seconds,
                    );
                    let position_accepted = self.sync_attached_player_position_observation(
                        position_seconds,
                        unexpected_position_jump && !coordinator_seek_owned,
                    );
                    if unexpected_position_jump && !position_accepted {
                        self.attached_native_seek_tracker.position_anchor =
                            previous_native_seek_tracker.position_anchor;
                        self.attached_native_seek_tracker.interval_disarmed =
                            previous_native_seek_tracker.interval_disarmed;
                        self.attached_native_seek_tracker.seeking_since_anchor =
                            previous_native_seek_tracker.seeking_since_anchor;
                    }
                }
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

    pub(in crate::app::runtime_owner) fn mark_network_media_transition_apply_failed(
        &mut self,
        reason: String,
    ) {
        self.player_apply_state.mark_streaming_apply_failed();
        self.pending_apply_requirements_refresh_required = true;
        self.core_player_configuration_health =
            GuiCorePlayerConfigurationHealth::StreamingDegraded {
                reason: reason.clone(),
                retryable_in_place: true,
                origin: GuiStreamingDegradationOrigin::AuthoritativeMediaTransition,
            };
        self.player_unavailability_reason = Some(reason);
    }

    fn reconcile_network_options_runtime_health_snapshot(
        &mut self,
        snapshot: MpvNetworkOptionsRuntimeHealthSnapshot,
        mpv_connected: bool,
    ) -> bool {
        // Apply the snapshot after both event queues every time. A transition can be enqueued by
        // the maintenance performed while draining the other channel, so revision equality alone
        // is not sufficient to skip this final authoritative reconciliation.
        self.network_options_runtime_health_revision = Some(snapshot.revision);
        match snapshot.hook_health {
            MpvNetworkOptionsHookHealth::Ready => self.record_network_options_hook_recovered(),
            MpvNetworkOptionsHookHealth::Degraded(reason) if mpv_connected => {
                self.mark_network_options_hook_degraded(format!(
                    "mpv playback remains available, but Sorotte's core streaming-settings hook needs retry or player restart: {reason}"
                ));
            }
            MpvNetworkOptionsHookHealth::Degraded(reason) => {
                self.player_apply_state.mark_streaming_apply_failed();
                self.detach_player();
                self.player_unavailability_reason = Some(format!(
                    "mpv JSON IPC became unavailable while maintaining Sorotte's core streaming-settings hook: {reason}"
                ));
                return false;
            }
            MpvNetworkOptionsHookHealth::Pending => {}
        }
        match snapshot.media_policy {
            MpvNetworkMediaPolicyState::NoActiveMedia
            | MpvNetworkMediaPolicyState::LocalMediaUnchanged
            | MpvNetworkMediaPolicyState::NetworkMediaUpdated => {
                self.record_network_media_transition_recovered();
            }
            MpvNetworkMediaPolicyState::Failed(reason) if mpv_connected => {
                if !matches!(
                    self.core_player_configuration_health,
                    GuiCorePlayerConfigurationHealth::StreamingDegraded {
                        origin: GuiStreamingDegradationOrigin::ExplicitApply,
                        ..
                    }
                ) {
                    self.mark_network_media_transition_apply_failed(format!(
                        "mpv switched to network media, but configured streaming settings could not be applied to the new file: {reason}"
                    ));
                }
            }
            MpvNetworkMediaPolicyState::Failed(reason) => {
                self.player_apply_state.mark_streaming_apply_failed();
                self.detach_player();
                self.player_unavailability_reason = Some(format!(
                    "mpv JSON IPC became unavailable while applying configured streaming settings to newly active network media: {reason}"
                ));
                return false;
            }
            MpvNetworkMediaPolicyState::Unknown
            | MpvNetworkMediaPolicyState::AwaitingAuthoritativeLoad => {}
        }
        true
    }

    fn mark_network_options_hook_degraded(&mut self, reason: String) {
        self.network_options_hook_failure_reason = Some(reason.clone());
        // Hook health is independent of an explicit media-policy apply that is still awaiting
        // its authoritative load result. Preserve that latch so NoActive/Local/Network/Failed can
        // resolve the policy baseline even while future hook transitions remain unprotected.
        self.player_apply_state.core_reapply_required = true;
        self.pending_apply_requirements_refresh_required = true;
        if matches!(
            self.core_player_configuration_health,
            GuiCorePlayerConfigurationHealth::StreamingDegraded {
                origin: GuiStreamingDegradationOrigin::ExplicitApply
                    | GuiStreamingDegradationOrigin::AuthoritativeMediaTransition,
                ..
            }
        ) {
            return;
        }
        self.core_player_configuration_health =
            GuiCorePlayerConfigurationHealth::StreamingDegraded {
                reason: reason.clone(),
                retryable_in_place: true,
                origin: GuiStreamingDegradationOrigin::NetworkOptionsHook,
            };
        self.player_unavailability_reason = Some(reason);
    }

    fn record_network_options_hook_recovered(&mut self) {
        let Some(hook_failure_reason) = self.network_options_hook_failure_reason.take() else {
            return;
        };
        let hook_issue_is_projected = matches!(
            self.core_player_configuration_health,
            GuiCorePlayerConfigurationHealth::StreamingDegraded {
                origin: GuiStreamingDegradationOrigin::NetworkOptionsHook,
                ..
            }
        );
        self.pending_apply_requirements_refresh_required = true;
        if !hook_issue_is_projected {
            return;
        }
        self.player_apply_state.core_reapply_required = false;
        self.core_player_configuration_health = GuiCorePlayerConfigurationHealth::Ready;
        if self.player_unavailability_reason.as_deref() == Some(hook_failure_reason.as_str()) {
            self.player_unavailability_reason = None;
        }
    }

    pub(in crate::app::runtime_owner) fn record_network_media_transition_recovered(&mut self) {
        if self.player_apply_state.streaming_apply_awaiting_transition
            && self
                .player_apply_state
                .process_target_is_applied(&self.player_launch_state)
        {
            self.player_apply_state
                .record_streaming_options_applied(&self.player_launch_state);
            self.pending_apply_requirements_refresh_required = true;
            self.core_player_configuration_health = GuiCorePlayerConfigurationHealth::Ready;
            if !self.restore_network_options_hook_degradation() {
                self.player_unavailability_reason = None;
            }
            return;
        }
        let transition_failure_reason = match &self.core_player_configuration_health {
            GuiCorePlayerConfigurationHealth::StreamingDegraded {
                reason,
                origin: GuiStreamingDegradationOrigin::AuthoritativeMediaTransition,
                ..
            } => reason.clone(),
            GuiCorePlayerConfigurationHealth::Ready
            | GuiCorePlayerConfigurationHealth::StreamingDegraded { .. } => return,
        };
        if !self
            .player_apply_state
            .process_target_is_applied(&self.player_launch_state)
            || !self
                .player_apply_state
                .streaming_options_are_applied(&self.player_launch_state)
        {
            return;
        }

        self.player_apply_state.core_reapply_required = false;
        self.pending_apply_requirements_refresh_required = true;
        self.core_player_configuration_health = GuiCorePlayerConfigurationHealth::Ready;
        if !self.restore_network_options_hook_degradation()
            && self.player_unavailability_reason.as_deref()
                == Some(transition_failure_reason.as_str())
        {
            self.player_unavailability_reason = None;
        }
    }

    pub(in crate::app::runtime_owner) fn player_local_file_ready_for_attached_sync(&self) -> bool {
        self.player_local_file.is_some()
            && self.player_local_file_identity_confirmed_for_shared_sync()
    }

    fn logical_media_override_for_loaded_target(
        &mut self,
        update: &LocalFileUpdate,
    ) -> Option<(LocalFileUpdate, bool)> {
        let scope_matches = self
            .pending_logical_media_override
            .as_ref()
            .is_some_and(|pending| {
                pending.playlist_row_id.is_none()
                    || (pending.playlist_generation == self.playlist_resolution.generation
                        && self
                            .playlist_resolution_attempt
                            .as_ref()
                            .is_some_and(|attempt| {
                                Some(attempt.row_id) == pending.playlist_row_id
                                    && attempt.playlist_generation == pending.playlist_generation
                                    && attempt.player_command_id == pending.player_command_id
                            }))
            });
        if !scope_matches {
            self.pending_logical_media_override = None;
            return None;
        }
        let exact_target_match =
            self.pending_logical_media_override
                .as_ref()
                .is_some_and(|pending| {
                    let loaded_target = pending.loaded_target_secret.as_str();
                    update
                        .path
                        .as_deref()
                        .is_some_and(|path| path == loaded_target)
                        || update.name == loaded_target
                });
        if !exact_target_match {
            // Any unmatched file observation is an external/superseding media
            // generation. It must never inherit an older Plex logical identity.
            self.pending_logical_media_override = None;
            return None;
        }

        let (logical_file, consume) = {
            let pending = self
                .pending_logical_media_override
                .as_mut()
                .expect("exact pending logical override should exist");
            pending.logical_file_observed = true;
            (pending.logical_file.clone(), pending.load_completed)
        };
        if consume {
            self.pending_logical_media_override = None;
        }
        Some((logical_file, consume))
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
