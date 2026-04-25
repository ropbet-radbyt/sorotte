use super::*;

#[derive(Debug)]
pub struct ClientRuntime<P, C> {
    session: ClientSession,
    player: P,
    control: C,
    pub(crate) ping_metrics_legacy_compatible: ClientPingMetricsLegacyCompatible,
    pending_player_playback_telemetry_updates: Vec<PlayerPlaybackTelemetryUpdate>,
}

impl<P, C> ClientRuntime<P, C>
where
    P: PlayerAdapter,
    C: ClientRuntimeControl,
{
    pub fn new(session: ClientSession, player: P, control: C) -> Self {
        Self {
            session,
            player,
            control,
            ping_metrics_legacy_compatible: ClientPingMetricsLegacyCompatible::default(),
            pending_player_playback_telemetry_updates: Vec::new(),
        }
    }

    fn finalize_local_playlist_index_switch_if_needed(&mut self, actions: &[ClientRuntimeAction]) {
        if !actions
            .iter()
            .any(|action| matches!(action, ClientRuntimeAction::SetPlaylistIndex { .. }))
        {
            return;
        }

        let now_seconds = unix_wall_clock_time_seconds_legacy_compatible();
        self.session
            .begin_local_playlist_index_reset_intent(true, now_seconds);
        self.session.apply_player_playback_telemetry_update(
            &PlayerPlaybackTelemetryUpdate::default()
                .with_paused(true)
                .with_position_seconds(0.0),
        );
        self.control.send_state(
            StatePayload::new()
                .with_playstate(PlaystatePayload::new().with_position(0.0).with_paused(true)),
        );
    }

    fn dispatch_runtime_actions_with_session_rollback(
        &mut self,
        session_snapshot: ClientSessionLocalActionSnapshot,
        actions: &[ClientRuntimeAction],
    ) -> Result<(), PlayerError> {
        match ClientSession::dispatch_runtime_actions(actions, &mut self.player, &mut self.control)
        {
            Ok(()) => Ok(()),
            Err(err) => {
                self.session.restore_local_action_state(session_snapshot);
                Err(err)
            }
        }
    }

    pub fn session(&self) -> &ClientSession {
        &self.session
    }

    pub fn session_mut(&mut self) -> &mut ClientSession {
        &mut self.session
    }

    pub fn reconnect_state_restore_correction_metrics(
        &self,
    ) -> &ReconnectStateRestoreCorrectionMetrics {
        self.session.reconnect_state_restore_correction_metrics()
    }

    pub fn reconnect_state_restore_correction_state_snapshot(
        &self,
    ) -> ReconnectStateRestoreCorrectionStateSnapshot {
        self.session
            .reconnect_state_restore_correction_state_snapshot()
    }

    pub fn control(&self) -> &C {
        &self.control
    }

    pub fn control_mut(&mut self) -> &mut C {
        &mut self.control
    }

    pub fn player(&self) -> &P {
        &self.player
    }

    pub fn player_mut(&mut self) -> &mut P {
        &mut self.player
    }

    pub fn current_room_playstate_legacy_ping_compatible_at(
        &self,
        now_seconds: f64,
    ) -> Option<RoomPlaystateView> {
        let mut room_playstate = self.session.current_room_playstate_at(now_seconds)?;
        if room_playstate.paused == Some(false)
            && let Some(position) = room_playstate.position
        {
            let forward_delay = self.ping_metrics_legacy_compatible.forward_delay_seconds();
            if forward_delay.is_finite() && forward_delay > 0.0 {
                room_playstate.position = Some(position + forward_delay);
            }
        }
        Some(room_playstate)
    }

    pub fn current_room_playstate_legacy_ping_compatible_now(&self) -> Option<RoomPlaystateView> {
        self.current_room_playstate_legacy_ping_compatible_at(
            unix_wall_clock_time_seconds_legacy_compatible(),
        )
    }

    pub fn into_parts(self) -> (ClientSession, P, C) {
        (self.session, self.player, self.control)
    }

    pub fn run_readiness_unpause_attempt(
        &mut self,
        now_seconds: f64,
        readiness_supported: bool,
        local_can_control: bool,
        is_playing_music: bool,
    ) -> Result<(), PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let session_snapshot = self.session.snapshot_local_action_state();
        let actions = self.session.runtime_actions_for_readiness_unpause_attempt(
            now_seconds,
            readiness_supported,
            local_can_control,
            is_playing_music,
        );
        self.dispatch_runtime_actions_with_session_rollback(session_snapshot, &actions)
    }

    pub fn update_autoplay_check(
        &mut self,
        readiness_supported: bool,
        local_can_control: bool,
        is_playing_music: bool,
        recently_advanced: bool,
    ) {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        self.session.autoplay_check(
            readiness_supported,
            local_can_control,
            is_playing_music,
            recently_advanced,
        );
    }

    pub fn tick_autoplay(
        &mut self,
        readiness_supported: bool,
        local_can_control: bool,
        is_playing_music: bool,
        recently_advanced: bool,
    ) -> Result<(), PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let session_snapshot = self.session.snapshot_local_action_state();
        let actions = self.session.autoplay_countdown_tick(
            readiness_supported,
            local_can_control,
            is_playing_music,
            recently_advanced,
        );
        self.dispatch_runtime_actions_with_session_rollback(session_snapshot, &actions)
    }

    pub fn run_reconnect_retry(&mut self, retries: u32) -> Result<(), PlayerError> {
        let actions = self.session.runtime_actions_for_reconnect_retry(retries);
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn run_controller_auth_notifications_if_needed(&mut self) -> Result<(), PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_controller_auth_notifications_if_needed();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn run_controlled_room_creation_notifications_if_needed(
        &mut self,
    ) -> Result<(), PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_controlled_room_creation_notifications_if_needed();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn run_chat_notifications_if_needed(&mut self) -> Result<(), PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_chat_notifications_if_needed();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn run_user_change_notifications_if_needed(&mut self) -> Result<(), PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_user_change_notifications_if_needed();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn run_reconnect_transition_if_needed(&mut self) -> Result<(), PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_reconnect_transition_if_needed();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn run_reconnect_state_restore_if_needed(&mut self) -> Result<(), PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_reconnect_state_restore_if_needed();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn run_reconnect_state_restore_validation_if_needed(&mut self) -> Result<(), PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let actions = self
            .session
            .runtime_actions_for_reconnect_state_restore_validation_if_needed();
        if actions.is_empty() {
            return Ok(());
        }

        let mut attempted_correction = false;
        for action in &actions {
            let is_correction_action = matches!(
                action,
                ClientRuntimeAction::SetPaused(_) | ClientRuntimeAction::SetPosition(_)
            );
            attempted_correction |= is_correction_action;
            if is_correction_action {
                self.session
                    .reconnect_state_restore_correction_metrics
                    .correction_actions_attempted = self
                    .session
                    .reconnect_state_restore_correction_metrics
                    .correction_actions_attempted
                    .saturating_add(1);
            }

            if let Err(err) = ClientSession::dispatch_runtime_actions(
                std::slice::from_ref(action),
                &mut self.player,
                &mut self.control,
            ) {
                if is_correction_action {
                    self.session
                        .reconnect_state_restore_correction_metrics
                        .correction_action_failures = self
                        .session
                        .reconnect_state_restore_correction_metrics
                        .correction_action_failures
                        .saturating_add(1);
                    if let Some(notification) = self
                        .session
                        .defer_reconnect_state_restore_validation_after_correction_failure()
                    {
                        self.control.notify_reconnect_transition(notification);
                    }
                    return Ok(());
                }
                return Err(err);
            }

            if is_correction_action {
                self.session
                    .reconnect_state_restore_correction_metrics
                    .correction_actions_succeeded = self
                    .session
                    .reconnect_state_restore_correction_metrics
                    .correction_actions_succeeded
                    .saturating_add(1);
            }
            self.session
                .apply_successful_reconnect_state_restore_validation_action(action);
        }

        if attempted_correction {
            self.session
                .complete_reconnect_state_restore_validation_after_success();
        }

        Ok(())
    }

    pub fn run_room_pause_sync_if_needed(&mut self) -> Result<(), PlayerError> {
        // Reconnect validation owns correction immediately after reconnect state restore.
        if self.session.reconnect_state_restore_validation_pending {
            return Ok(());
        }

        self.sync_player_playback_telemetry_into_session_and_buffer();

        let Some(room_playstate) = self.current_room_playstate_legacy_ping_compatible_now() else {
            return Ok(());
        };
        let room_seeked = room_playstate.do_seek == Some(true);
        let pause_mismatch = room_playstate
            .paused
            .is_some_and(|room_paused| self.session.local_paused.unwrap_or(true) != room_paused);
        if !room_seeked && !pause_mismatch {
            return Ok(());
        }
        let set_by_is_self = self
            .session
            .username
            .as_deref()
            .zip(room_playstate.set_by.as_deref())
            .is_some_and(|(username, set_by)| username == set_by);
        if set_by_is_self {
            return Ok(());
        }

        let target_position = if (room_seeked || room_playstate.paused == Some(true))
            && let Some(room_position) = room_playstate.position.filter(|value| value.is_finite())
        {
            Some(room_position)
        } else {
            None
        };
        let target_paused = if pause_mismatch {
            room_playstate.paused
        } else {
            None
        };
        if target_position.is_none() && target_paused.is_none() {
            return Ok(());
        }

        if let Some(room_position) = target_position {
            ClientSession::dispatch_runtime_actions(
                &[ClientRuntimeAction::SetPosition(room_position)],
                &mut self.player,
                &mut self.control,
            )?;
            self.session.local_position = Some(room_position);
        }
        if let Some(room_paused) = target_paused {
            ClientSession::dispatch_runtime_actions(
                &[ClientRuntimeAction::SetPaused(room_paused)],
                &mut self.player,
                &mut self.control,
            )?;
            // Mirror the confirmed local state to avoid duplicate correction attempts until telemetry catches up.
            self.session.local_paused = Some(room_paused);
        }
        Ok(())
    }

    pub fn run_desync_correction_if_needed(
        &mut self,
        now_seconds: f64,
        local_can_control: bool,
        dont_slow_down_with_me: bool,
        speed_supported: bool,
    ) -> Result<(), PlayerError> {
        // Reconnect validation owns the correction window immediately after reconnect restore.
        if self.session.reconnect_state_restore_validation_pending {
            return Ok(());
        }

        self.sync_player_playback_telemetry_into_session_and_buffer();
        let Some(local_position) = self.session.local_position else {
            return Ok(());
        };
        let local_position =
            self.desync_local_position_with_legacy_ping_forward_delay(local_position);
        let Some(room_playstate) = self.session.current_room_playstate_at(now_seconds) else {
            self.session.behind_first_detected_at_seconds = None;
            return Ok(());
        };

        let actions = self
            .session
            .runtime_actions_for_desync_correction_against_room_playstate(
                room_playstate,
                now_seconds,
                local_position,
                local_can_control,
                dont_slow_down_with_me,
                speed_supported,
            );
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    fn desync_local_position_with_legacy_ping_forward_delay(&self, local_position: f64) -> f64 {
        let Some(room_playstate) = self.session.current_room_playstate() else {
            return local_position;
        };
        if room_playstate.paused != Some(false) || room_playstate.do_seek == Some(true) {
            return local_position;
        }

        let forward_delay = self.ping_metrics_legacy_compatible.forward_delay_seconds();
        if !forward_delay.is_finite() || forward_delay <= 0.0 {
            return local_position;
        }

        // Compare against an estimate of "room position now" by moving local position back by
        // the inferred one-way/forward delay before evaluating threshold-based desync actions.
        local_position - forward_delay
    }

    pub fn run_reconnect_playlist_restore_if_needed(&mut self) -> Result<(), PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_reconnect_playlist_restore_if_needed();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn run_controller_reidentify_if_needed(&mut self) -> Result<(), PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_controller_reidentify_if_needed();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn run_send_chat_message(
        &mut self,
        message: impl Into<String>,
    ) -> Result<bool, PlayerError> {
        if self.session.server_chat_supported().is_none() {
            return Ok(false);
        }
        let actions = self
            .session
            .runtime_actions_for_outbound_chat_message(message.into());
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_player_chat_input_if_needed(&mut self) -> Result<usize, PlayerError> {
        let mut sent = 0usize;
        while let Some(message) = self.player.take_pending_chat_request() {
            if self.run_send_chat_message(message)? {
                sent += 1;
            }
        }
        Ok(sent)
    }

    pub fn run_toggle_ready(&mut self, manually_initiated: bool) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_ready_toggle(manually_initiated);
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_set_ready_for_user(
        &mut self,
        username: impl Into<String>,
        ready: bool,
        manually_initiated: bool,
    ) -> Result<bool, PlayerError> {
        let actions = self.session.runtime_actions_for_local_user_ready_set(
            username.into(),
            ready,
            manually_initiated,
        );
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_local_media_opened_not_ready(&mut self) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_media_opened_not_ready();
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_request_controller_auth(
        &mut self,
        room: impl Into<String>,
        password: impl Into<String>,
    ) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_controller_auth_request(room.into(), password.into());
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_set_room(&mut self, room: impl Into<String>) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_room_switch(room.into());
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_set_room_with_legacy_fallback(
        &mut self,
        default_room: impl Into<String>,
    ) -> Result<bool, PlayerError> {
        let default_room = default_room.into();
        let room = self
            .session
            .local_room_command_target_with_legacy_fallback(&default_room);
        self.run_set_room(room)
    }

    pub fn run_toggle_pause(&mut self) -> Result<bool, PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let session_snapshot = self.session.snapshot_local_action_state();
        let actions = self.session.runtime_actions_for_local_pause_toggle();
        let sent = !actions.is_empty();
        self.dispatch_runtime_actions_with_session_rollback(session_snapshot, &actions)
            .map(|_| sent)
    }

    pub fn run_set_paused(&mut self, paused: bool) -> Result<bool, PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let session_snapshot = self.session.snapshot_local_action_state();
        let actions = self.session.runtime_actions_for_local_pause_set(paused);
        let sent = !actions.is_empty();
        self.dispatch_runtime_actions_with_session_rollback(session_snapshot, &actions)
            .map(|_| sent)
    }

    pub fn run_request_user_list(&mut self) -> Result<bool, PlayerError> {
        let actions = self.session.runtime_actions_for_local_user_list_request();
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_set_playlist_index(&mut self, index: i64) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_playlist_index_set(index);
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)?;
        self.session
            .apply_local_playlist_runtime_actions_legacy_compatible(&actions);
        self.finalize_local_playlist_index_switch_if_needed(&actions);
        Ok(sent)
    }

    pub fn run_advance_playlist_index(&mut self) -> Result<bool, PlayerError> {
        let actions = self.session.runtime_actions_for_local_playlist_next();
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)?;
        self.session
            .apply_local_playlist_runtime_actions_legacy_compatible(&actions);
        self.finalize_local_playlist_index_switch_if_needed(&actions);
        Ok(sent)
    }

    pub fn run_queue_playlist_item(
        &mut self,
        file_name: impl Into<String>,
        select_after_queue: bool,
    ) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_playlist_queue(file_name.into(), select_after_queue);
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)?;
        self.session
            .apply_local_playlist_runtime_actions_legacy_compatible(&actions);
        Ok(sent)
    }

    pub fn run_delete_playlist_index(&mut self, index: i64) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_playlist_delete(index);
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)?;
        self.session
            .apply_local_playlist_runtime_actions_legacy_compatible(&actions);
        Ok(sent)
    }

    pub fn run_replace_playlist(
        &mut self,
        files: Vec<String>,
        selected_index: Option<usize>,
    ) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_playlist_replace(files, selected_index);
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)?;
        self.session
            .apply_local_playlist_runtime_actions_legacy_compatible(&actions);
        Ok(sent)
    }

    pub fn run_undo_playlist_change(&mut self) -> Result<bool, PlayerError> {
        let actions = self.session.runtime_actions_for_local_playlist_undo();
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)?;
        self.session
            .apply_local_playlist_runtime_actions_legacy_compatible(&actions);
        Ok(sent)
    }

    pub fn run_shuffle_remaining_playlist(&mut self) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_playlist_shuffle_remaining();
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)?;
        self.session
            .apply_local_playlist_runtime_actions_legacy_compatible(&actions);
        Ok(sent)
    }

    pub fn run_shuffle_entire_playlist(&mut self) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_playlist_shuffle_entire();
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)?;
        self.session
            .apply_local_playlist_runtime_actions_legacy_compatible(&actions);
        Ok(sent)
    }

    pub fn run_seek_to_position(&mut self, target_position: f64) -> Result<bool, PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let session_snapshot = self.session.snapshot_local_action_state();
        let actions = self.session.runtime_actions_for_local_seek(target_position);
        let sent = !actions.is_empty();
        self.dispatch_runtime_actions_with_session_rollback(session_snapshot, &actions)
            .map(|_| sent)
    }

    pub fn run_seek_by_offset(&mut self, offset_seconds: f64) -> Result<bool, PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let session_snapshot = self.session.snapshot_local_action_state();
        let actions = self
            .session
            .runtime_actions_for_local_seek_offset(offset_seconds);
        let sent = !actions.is_empty();
        self.dispatch_runtime_actions_with_session_rollback(session_snapshot, &actions)
            .map(|_| sent)
    }

    pub fn run_undo_seek(&mut self) -> Result<bool, PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let session_snapshot = self.session.snapshot_local_action_state();
        let actions = self.session.runtime_actions_for_local_seek_undo();
        let sent = !actions.is_empty();
        self.dispatch_runtime_actions_with_session_rollback(session_snapshot, &actions)
            .map(|_| sent)
    }

    pub fn run_disconnect(&mut self, now_seconds: f64) -> Result<(), PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let session_snapshot = self.session.snapshot_local_action_state();
        let actions = self.session.handle_disconnect(now_seconds);
        self.dispatch_runtime_actions_with_session_rollback(session_snapshot, &actions)
    }

    pub fn publish_local_file_legacy_compatible(
        &mut self,
        file_payload: &Value,
        filename_privacy_mode: PrivacyMode,
        filesize_privacy_mode: PrivacyMode,
    ) -> Result<(), PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_file_publish_legacy_compatible(
                file_payload,
                filename_privacy_mode,
                filesize_privacy_mode,
            );
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn publish_pending_local_file_update_legacy_compatible(
        &mut self,
        filename_privacy_mode: PrivacyMode,
        filesize_privacy_mode: PrivacyMode,
    ) -> Result<bool, PlayerError> {
        let Some(local_file_update) = self.player.take_local_file_update() else {
            return Ok(false);
        };

        let file_payload = Self::local_file_update_payload(&local_file_update);
        self.publish_local_file_legacy_compatible(
            &file_payload,
            filename_privacy_mode,
            filesize_privacy_mode,
        )?;
        Ok(true)
    }

    fn local_file_update_payload(local_file_update: &LocalFileUpdate) -> Value {
        let mut payload = Map::new();
        payload.insert(
            "name".to_owned(),
            Value::String(local_file_update.name.clone()),
        );
        if let Some(duration_seconds) = local_file_update.duration_seconds {
            payload.insert("duration".to_owned(), Value::from(duration_seconds));
        }
        if let Some(size_bytes) = local_file_update.size_bytes {
            payload.insert("size".to_owned(), Value::from(size_bytes));
        }
        if let Some(path) = local_file_update.path.as_ref() {
            payload.insert("path".to_owned(), Value::String(path.clone()));
        }
        Value::Object(payload)
    }

    fn sync_player_playback_telemetry_into_session_and_buffer(&mut self) {
        while let Some(update) = self.player.take_playback_telemetry_update() {
            self.session.apply_player_playback_telemetry_update(&update);
            self.pending_player_playback_telemetry_updates.push(update);
        }
    }
}

impl<P> ClientRuntime<P, QueuedRuntimeControl>
where
    P: PlayerAdapter,
{
    fn outbound_state_sync_position_seconds(
        &self,
        now_seconds: f64,
        dont_slow_down_with_me: bool,
    ) -> Option<f64> {
        let local_position = self.session.local_position?;
        if !dont_slow_down_with_me {
            return Some(local_position);
        }

        self.session
            .current_room_playstate_at(now_seconds)
            .and_then(|playstate| playstate.position)
            .or(Some(local_position))
    }

    pub fn run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible(
        &mut self,
        inbound_state: StatePayload,
        dont_slow_down_with_me: bool,
    ) -> bool {
        self.ping_metrics_legacy_compatible
            .observe_inbound_state(&inbound_state);
        let local_state_change_global_playstate = self
            .adjusted_inbound_playstate_for_local_state_change_legacy_ping_compatible(
                &inbound_state,
            );
        self.run_state_sync_reconcile_with_inbound_state_with_local_state_change_override(
            inbound_state,
            self.ping_metrics_legacy_compatible
                .client_latency_calculation_now(),
            self.ping_metrics_legacy_compatible.client_rtt_seconds(),
            dont_slow_down_with_me,
            local_state_change_global_playstate,
        )
    }

    pub fn run_state_sync_reconcile_with_inbound_state(
        &mut self,
        inbound_state: StatePayload,
        client_latency_calculation: f64,
        client_rtt: f64,
        dont_slow_down_with_me: bool,
    ) -> bool {
        self.run_state_sync_reconcile_with_inbound_state_with_local_state_change_override(
            inbound_state,
            client_latency_calculation,
            client_rtt,
            dont_slow_down_with_me,
            None,
        )
    }

    fn run_state_sync_reconcile_with_inbound_state_with_local_state_change_override(
        &mut self,
        inbound_state: StatePayload,
        client_latency_calculation: f64,
        client_rtt: f64,
        dont_slow_down_with_me: bool,
        local_state_change_global_playstate: Option<RoomPlaystateView>,
    ) -> bool {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let now_seconds = unix_wall_clock_time_seconds_legacy_compatible();

        let (Some(local_position), Some(local_paused)) = (
            self.outbound_state_sync_position_seconds(now_seconds, dont_slow_down_with_me),
            self.session.local_paused,
        ) else {
            self.session.apply_state(inbound_state);
            return false;
        };

        let outbound_state = self
            .session
            .reconcile_state_and_build_response_with_local_state_change_override(
                inbound_state,
                local_position,
                local_paused,
                client_latency_calculation,
                client_rtt,
                local_state_change_global_playstate,
            );
        self.control
            .outbound_messages
            .push(ProtocolMessage::state(outbound_state));
        true
    }

    fn adjusted_inbound_playstate_for_local_state_change_legacy_ping_compatible(
        &self,
        inbound_state: &StatePayload,
    ) -> Option<RoomPlaystateView> {
        let playstate = inbound_state.playstate.as_ref()?;
        let mut position = playstate.position;
        if playstate.paused == Some(false) {
            let forward_delay = self.ping_metrics_legacy_compatible.forward_delay_seconds();
            if forward_delay.is_finite()
                && forward_delay > 0.0
                && let Some(raw_position) = position.filter(|value| value.is_finite())
            {
                position = Some(raw_position + forward_delay);
            }
        }

        Some(RoomPlaystateView {
            position,
            paused: playstate.paused,
            do_seek: Some(playstate.do_seek.unwrap_or(false)),
            set_by: playstate.set_by.clone(),
        })
    }

    pub fn run_state_sync_heartbeat_legacy_ping_compatible(
        &mut self,
        dont_slow_down_with_me: bool,
    ) -> bool {
        if self.session.server_chat_supported().is_none() {
            return false;
        }

        self.sync_player_playback_telemetry_into_session_and_buffer();
        let now_seconds = unix_wall_clock_time_seconds_legacy_compatible();

        let client_latency_calculation = self
            .ping_metrics_legacy_compatible
            .client_latency_calculation_now();
        let client_rtt = self.ping_metrics_legacy_compatible.client_rtt_seconds();

        let outbound_state = if let (Some(local_position), Some(local_paused)) = (
            self.outbound_state_sync_position_seconds(now_seconds, dont_slow_down_with_me),
            self.session.local_paused,
        ) {
            self.session.reconcile_state_and_build_response(
                StatePayload::new(),
                local_position,
                local_paused,
                client_latency_calculation,
                client_rtt,
            )
        } else {
            StatePayload::new().with_ping(
                PingPayload::new()
                    .with_client_latency_calculation(client_latency_calculation)
                    .with_client_rtt(client_rtt),
            )
        };

        self.control
            .outbound_messages
            .push(ProtocolMessage::state(outbound_state));
        true
    }

    pub fn flush_queued_protocol_messages(&mut self) -> Vec<ProtocolMessage> {
        self.control.drain_outbound_messages()
    }

    pub fn flush_queued_protocol_lines(&mut self) -> Result<Vec<String>, ProtocolError> {
        self.control.drain_outbound_message_lines()
    }

    pub fn drain_reconnect_requests(&mut self) -> Vec<f64> {
        self.control.drain_reconnect_delays()
    }

    pub fn take_stop_reconnect_requested(&mut self) -> bool {
        self.control.take_stop_reconnect_requested()
    }

    pub fn drain_autoplay_notifications(&mut self) -> Vec<AutoplayCountdownNotification> {
        self.control.drain_autoplay_notifications()
    }

    pub fn drain_chat_notifications(&mut self) -> Vec<ChatNotification> {
        self.control.drain_chat_notifications()
    }

    pub fn drain_controlled_room_creation_notifications(
        &mut self,
    ) -> Vec<ControlledRoomCreationNotification> {
        self.control.drain_controlled_room_creation_notifications()
    }

    pub fn drain_controller_auth_notifications(
        &mut self,
    ) -> Vec<ControllerAuthTransitionNotification> {
        self.control.drain_controller_auth_notifications()
    }

    pub fn drain_user_change_notifications(&mut self) -> Vec<UserChangeNotification> {
        self.control.drain_user_change_notifications()
    }

    pub fn drain_reconnect_notifications(&mut self) -> Vec<ReconnectTransitionNotification> {
        self.control.drain_reconnect_notifications()
    }

    pub fn drain_player_playback_telemetry_updates(
        &mut self,
    ) -> Vec<PlayerPlaybackTelemetryUpdate> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        std::mem::take(&mut self.pending_player_playback_telemetry_updates)
    }

    pub fn flush_queued_protocol_lines_to_transport<F>(
        &mut self,
        mut send_line: F,
    ) -> Result<(), ProtocolError>
    where
        F: FnMut(&str) -> Result<(), ProtocolError>,
    {
        let lines = self.flush_queued_protocol_lines()?;
        for line in &lines {
            send_line(line)?;
        }
        Ok(())
    }

    pub fn drain_reconnect_intents<FS, FT>(
        &mut self,
        mut schedule_reconnect: FS,
        mut stop_reconnect: FT,
    ) where
        FS: FnMut(f64),
        FT: FnMut(),
    {
        for delay_seconds in self.drain_reconnect_requests() {
            schedule_reconnect(delay_seconds);
        }
        if self.take_stop_reconnect_requested() {
            stop_reconnect();
        }
    }

    pub fn drain_autoplay_notifications_to_sink<F, E>(&mut self, mut notify: F) -> Result<(), E>
    where
        F: FnMut(&AutoplayCountdownNotification) -> Result<(), E>,
    {
        for notification in self.drain_autoplay_notifications() {
            notify(&notification)?;
        }
        Ok(())
    }

    pub fn drain_controller_auth_notifications_to_sink<F, E>(
        &mut self,
        mut notify: F,
    ) -> Result<(), E>
    where
        F: FnMut(&ControllerAuthTransitionNotification) -> Result<(), E>,
    {
        for notification in self.drain_controller_auth_notifications() {
            notify(&notification)?;
        }
        Ok(())
    }

    pub fn drain_controlled_room_creation_notifications_to_sink<F, E>(
        &mut self,
        mut notify: F,
    ) -> Result<(), E>
    where
        F: FnMut(&ControlledRoomCreationNotification) -> Result<(), E>,
    {
        for notification in self.drain_controlled_room_creation_notifications() {
            notify(&notification)?;
        }
        Ok(())
    }

    pub fn drain_chat_notifications_to_sink<F, E>(&mut self, mut notify: F) -> Result<(), E>
    where
        F: FnMut(&ChatNotification) -> Result<(), E>,
    {
        for notification in self.drain_chat_notifications() {
            notify(&notification)?;
        }
        Ok(())
    }

    pub fn drain_user_change_notifications_to_sink<F, E>(&mut self, mut notify: F) -> Result<(), E>
    where
        F: FnMut(&UserChangeNotification) -> Result<(), E>,
    {
        for notification in self.drain_user_change_notifications() {
            notify(&notification)?;
        }
        Ok(())
    }

    pub fn drain_reconnect_notifications_to_sink<F, E>(&mut self, mut notify: F) -> Result<(), E>
    where
        F: FnMut(&ReconnectTransitionNotification) -> Result<(), E>,
    {
        for notification in self.drain_reconnect_notifications() {
            notify(&notification)?;
        }
        Ok(())
    }

    pub fn drain_player_playback_telemetry_updates_to_sink<F, E>(
        &mut self,
        mut notify: F,
    ) -> Result<(), E>
    where
        F: FnMut(&PlayerPlaybackTelemetryUpdate) -> Result<(), E>,
    {
        for update in self.drain_player_playback_telemetry_updates() {
            notify(&update)?;
        }
        Ok(())
    }
}
