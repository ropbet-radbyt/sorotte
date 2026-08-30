use super::*;

impl<P, C> ClientRuntime<P, C>
where
    P: PlayerAdapter,
    C: ClientEffectSink,
{
    pub(crate) fn dispatch_runtime_actions_with_room_switch_coordination(
        &mut self,
        actions: &[ClientRuntimeAction],
        preserve_uninitiated_media: bool,
    ) -> Result<(), PlayerError> {
        for action in actions {
            self.dispatch_runtime_actions_with_causal_tracking(std::slice::from_ref(action))?;
            if matches!(action, ClientRuntimeAction::SetRoom { .. }) {
                if preserve_uninitiated_media {
                    self.playback_coordination
                        .handle_authoritative_playback_barrier_room_change();
                } else {
                    self.playback_coordination
                        .discard_room_scoped_playback_barrier_intent();
                }
            }
        }
        Ok(())
    }

    pub fn run_send_chat_message(
        &mut self,
        message: impl Into<String>,
    ) -> Result<bool, PlayerError> {
        if !self.session.is_active() {
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
        self.run_toggle_ready_from(
            manually_initiated,
            sorotte_protocol::DirectReadinessSurface::RemoteControlSurface,
        )
    }

    pub fn run_toggle_ready_from(
        &mut self,
        manually_initiated: bool,
        surface: sorotte_protocol::DirectReadinessSurface,
    ) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_ready_toggle_from(manually_initiated, surface);
        let sent = !actions.is_empty() || self.session.pending_readiness_intent().is_some();
        match ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
        {
            Ok(()) => Ok(sent),
            Err(error) => {
                self.session.mark_pending_readiness_delivery_failed();
                Err(error)
            }
        }
    }

    pub fn run_set_ready_for_user(
        &mut self,
        username: impl Into<String>,
        ready: bool,
        manually_initiated: bool,
    ) -> Result<bool, PlayerError> {
        self.run_set_ready_for_user_from(
            username,
            ready,
            manually_initiated,
            sorotte_protocol::DirectReadinessSurface::RemoteControlSurface,
        )
    }

    pub fn run_set_ready_for_user_from(
        &mut self,
        username: impl Into<String>,
        ready: bool,
        manually_initiated: bool,
        surface: sorotte_protocol::DirectReadinessSurface,
    ) -> Result<bool, PlayerError> {
        let actions = self.session.runtime_actions_for_local_user_ready_set_from(
            username.into(),
            ready,
            manually_initiated,
            surface,
        );
        let sent = !actions.is_empty() || self.session.pending_readiness_intent().is_some();
        match ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
        {
            Ok(()) => Ok(sent),
            Err(error) => {
                self.session.mark_pending_readiness_delivery_failed();
                Err(error)
            }
        }
    }

    pub fn run_initial_readiness_intent(&mut self, ready: bool) -> Result<bool, PlayerError> {
        let desired = if ready {
            sorotte_protocol::UserReadinessIntent::Ready
        } else {
            sorotte_protocol::UserReadinessIntent::NotReady
        };
        let actions = self
            .session
            .runtime_actions_for_initial_readiness_intent(desired);
        let sent = !actions.is_empty() || self.session.pending_readiness_intent().is_some();
        match ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
        {
            Ok(()) => Ok(sent),
            Err(error) => {
                self.session.mark_pending_readiness_delivery_failed();
                Err(error)
            }
        }
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
        password: impl Into<SecretValue>,
    ) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_controller_auth_request(room.into(), password.into());
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    /// Records an intentional Sorotte/player Play or Pause independently from
    /// whether the physical player command can be carried out. This is used by
    /// attached-player surfaces that perform their own player I/O.
    pub fn run_direct_player_readiness_intent(
        &mut self,
        paused: bool,
        surface: sorotte_protocol::PlayerInteractionSurface,
    ) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_indirect_player_intent(paused, surface);
        let sent = !actions.is_empty() || self.session.pending_readiness_intent().is_some();
        match ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
        {
            Ok(()) => Ok(sent),
            Err(error) => {
                self.session.mark_pending_readiness_delivery_failed();
                Err(error)
            }
        }
    }

    pub fn run_set_room(&mut self, room: impl Into<String>) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_room_switch(room.into());
        let sent = !actions.is_empty();
        self.dispatch_runtime_actions_with_room_switch_coordination(&actions, false)
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
        if self.session.model.local_pause_change_in_flight() {
            return Err(PlayerError::OperationFailed(
                "a local pause change is already in progress".to_owned(),
            ));
        }
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let original_paused = self.session.model.playback.local_paused;
        let original_ready = self
            .session
            .username()
            .and_then(|username| self.session.user_ready(username));
        let original_last_paused_on_leave_at_seconds =
            self.session.model.playback.last_paused_on_leave_at_seconds;
        let current_gate_holds_play = self.readiness_gate_holds_current_playback();
        let actions = self
            .session
            .runtime_actions_for_local_pause_toggle_with_gate_hold(Some(current_gate_holds_play));
        let planned_paused = self.session.model.playback.local_paused;
        let planned_ready = self
            .session
            .username()
            .and_then(|username| self.session.user_ready(username));
        let planned_last_paused_on_leave_at_seconds =
            self.session.model.playback.last_paused_on_leave_at_seconds;
        self.session.model.apply_local_pause_state(
            original_paused,
            original_ready,
            original_last_paused_on_leave_at_seconds,
        );
        let sent = !actions.is_empty();
        if !sent {
            return Ok(false);
        }
        if planned_paused == Some(true) {
            // A user pause owns the transport immediately; do not wait for
            // the reflected room state before restoring a catch-up rate.
            let _ =
                self.interrupt_playback_recovery(unix_wall_clock_time_seconds_legacy_compatible());
        }
        let effects = Self::local_pause_effects(&actions)?;
        let result = self.run_model_event(ClientEvent::LocalPauseChangeRequested {
            original_paused,
            original_ready,
            original_last_paused_on_leave_at_seconds,
            planned_paused,
            planned_ready,
            planned_last_paused_on_leave_at_seconds,
            effects,
        });
        if let Err(error) = result {
            self.session.mark_pending_readiness_delivery_failed();
            return Err(error);
        }
        Ok(true)
    }

    pub fn run_set_paused(&mut self, paused: bool) -> Result<bool, PlayerError> {
        if self.session.model.local_pause_change_in_flight() {
            return Err(PlayerError::OperationFailed(
                "a local pause change is already in progress".to_owned(),
            ));
        }
        self.sync_player_playback_telemetry_into_session_and_buffer();
        if paused {
            let _ =
                self.interrupt_playback_recovery(unix_wall_clock_time_seconds_legacy_compatible());
        }
        let original_paused = self.session.model.playback.local_paused;
        let original_ready = self
            .session
            .username()
            .and_then(|username| self.session.user_ready(username));
        let original_last_paused_on_leave_at_seconds =
            self.session.model.playback.last_paused_on_leave_at_seconds;
        let current_gate_holds_play = self.readiness_gate_holds_current_playback();
        let actions = self
            .session
            .runtime_actions_for_local_pause_set_with_gate_hold(
                paused,
                Some(current_gate_holds_play),
            );
        let planned_paused = self.session.model.playback.local_paused;
        let planned_ready = self
            .session
            .username()
            .and_then(|username| self.session.user_ready(username));
        let planned_last_paused_on_leave_at_seconds =
            self.session.model.playback.last_paused_on_leave_at_seconds;
        self.session.model.apply_local_pause_state(
            original_paused,
            original_ready,
            original_last_paused_on_leave_at_seconds,
        );
        let sent = !actions.is_empty();
        if !sent {
            return Ok(false);
        }
        let effects = Self::local_pause_effects(&actions)?;
        let result = self.run_model_event(ClientEvent::LocalPauseChangeRequested {
            original_paused,
            original_ready,
            original_last_paused_on_leave_at_seconds,
            planned_paused,
            planned_ready,
            planned_last_paused_on_leave_at_seconds,
            effects,
        });
        if let Err(error) = result {
            self.session.mark_pending_readiness_delivery_failed();
            return Err(error);
        }
        Ok(true)
    }

    fn local_pause_effects(
        actions: &[ClientRuntimeAction],
    ) -> Result<Vec<ClientEffect>, PlayerError> {
        actions
            .iter()
            .map(|action| match action {
                ClientRuntimeAction::SetPaused(paused) => {
                    Ok(ClientEffect::SetPlayerPaused(*paused))
                }
                ClientRuntimeAction::SetReady {
                    ready,
                    manually_initiated,
                } => Ok(ClientEffect::SetReady {
                    ready: *ready,
                    manually_initiated: *manually_initiated,
                }),
                ClientRuntimeAction::SetReadinessIntent { request, scope } => {
                    Ok(ClientEffect::SendReadinessIntent {
                        request: request.clone(),
                        scope: scope.clone(),
                    })
                }
                other => Err(PlayerError::OperationFailed(format!(
                    "invalid local pause effect sequence: {other:?}"
                ))),
            })
            .collect()
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
        self.run_local_playlist_action_batch(actions)
    }

    pub fn run_advance_playlist_index(&mut self) -> Result<bool, PlayerError> {
        let actions = self.session.runtime_actions_for_local_playlist_next();
        self.run_local_playlist_action_batch(actions)
    }

    /// Advances the canonical shared playlist exactly once for an owned,
    /// natural player completion.
    ///
    /// The completed physical file must still be the canonical selection. If
    /// another participant already advanced the room, the late terminal edge
    /// is consumed without acting so it cannot skip the newly selected item.
    /// A completion observed before the playlist snapshot arrives remains
    /// pending while authority is absent, but it cannot acquire a selection
    /// identity retroactively and is consumed once that mismatch is knowable.
    pub fn run_advance_playlist_after_natural_completion(&mut self) -> Result<bool, PlayerError> {
        let Some(completion) = self.pending_natural_playback_completion.as_ref() else {
            return Ok(false);
        };
        let Some(playlist) = self.session.current_room_playlist() else {
            return Ok(false);
        };
        let Some(index) = playlist.index.and_then(|index| usize::try_from(index).ok()) else {
            return Ok(false);
        };
        let current_selection_revision = self.session.current_room_playlist_selection_revision();
        let completion_matches_selection_identity =
            completion.playlist_selection_revision.is_some()
                && completion.playlist_selection_revision == current_selection_revision
                && completion.playlist_revision == Some(playlist.revision)
                && completion.playlist_index == playlist.index;
        if !completion_matches_selection_identity {
            // File names are not playlist-entry identities: duplicate rows,
            // loop wraparound, and a peer mutation may all select the same
            // visible target. Only the exact canonical selection observed at
            // EOF may advance. A completion received before any playlist
            // snapshot cannot acquire that identity retroactively.
            self.pending_natural_playback_completion = None;
            return Ok(false);
        }
        let Some(target) = playlist.files.get(index) else {
            self.pending_natural_playback_completion = None;
            return Ok(false);
        };
        let completion_matches_selection = completion
            .completed_file
            .as_ref()
            .is_some_and(|file| local_file_matches_playlist_target(file, target));
        if !completion_matches_selection {
            // The canonical room has moved on (or the terminal could not be
            // tied to a publishable file). Either way, fail closed rather than
            // advancing an unrelated selection.
            self.pending_natural_playback_completion = None;
            return Ok(false);
        }

        let actions = self
            .session
            .runtime_actions_for_verified_local_playlist_next();
        let result = self.run_local_playlist_action_batch(actions);
        if result.is_ok() {
            // `Ok(false)` is a legitimate terminal playlist boundary (for
            // example, the final item with looping disabled), so the physical
            // EOF is consumed regardless of whether a State mutation exists.
            self.pending_natural_playback_completion = None;
        }
        result
    }

    fn current_local_playlist_target(&self) -> Option<String> {
        let playlist = self.session.current_room_playlist()?;
        let index = playlist
            .index
            .and_then(|index| usize::try_from(index).ok())?;
        playlist.files.get(index).cloned()
    }

    fn run_local_playlist_action_batch(
        &mut self,
        actions: Vec<ClientRuntimeAction>,
    ) -> Result<bool, PlayerError> {
        if actions.is_empty() {
            return Ok(false);
        }

        let selected_target_before = self
            .current_local_playlist_target()
            // Before a shared playlist has an index, the attached player can
            // already be playing the target that a compound batch selects.
            // Treat that announced local file as the effective prior target so
            // importing/reordering a playlist around it does not rewind it.
            .or_else(|| self.session.current_user_file_name().map(str::to_owned));
        let dedicated_index_request = actions.len() == 1
            && matches!(actions[0], ClientRuntimeAction::SetPlaylistIndex { .. });
        let replay_media = dedicated_index_request
            .then(|| self.v2_replay_media_for_playlist_actions(&actions))
            .flatten();

        self.dispatch_runtime_actions_with_pause_cause(
            &actions,
            PlayerCommandCause::PlaylistTransition,
        )?;
        self.session
            .apply_local_playlist_runtime_actions_legacy_compatible(&actions);

        // Do not use the attached-file fallback after applying the batch: an
        // empty or index-less playlist has no selectable target and therefore
        // cannot consume a reset intent.
        let selected_target_after = self.current_local_playlist_target();
        let selected_target_changed =
            selected_target_after.is_some() && selected_target_before != selected_target_after;
        // An explicit index request is also a replay request when it selects the
        // already-active row. Compound batches reset only when their optimistic
        // projection changes the selected media target. A pure playlist reorder
        // can move the active target to a different numeric index and must not
        // reopen or rewind that same media.
        self.finalize_local_playlist_selection_switch_if_needed(
            selected_target_changed || dedicated_index_request,
        )?;
        self.begin_v2_replay_episode(replay_media);
        Ok(true)
    }

    fn v2_replay_media_for_playlist_actions(
        &self,
        actions: &[ClientRuntimeAction],
    ) -> Option<(LogicalMediaId, MediaTransportKind)> {
        if !self.session.server_readiness_v2_supported() {
            return None;
        }
        let current_index = self.session.current_room_playlist()?.index?;
        actions
            .iter()
            .any(|action| {
                matches!(
                    action,
                    ClientRuntimeAction::SetPlaylistIndex { index }
                        if *index == current_index
                )
            })
            .then(|| self.playback_coordination.current_media_for_replay())
            .flatten()
    }

    fn begin_v2_replay_episode(
        &mut self,
        replay_media: Option<(LogicalMediaId, MediaTransportKind)>,
    ) {
        let Some((logical_id, kind)) = replay_media else {
            return;
        };
        self.prepare_playback_media_with_intent(
            logical_id,
            kind,
            MediaLoadIntent::Replay,
            unix_wall_clock_time_seconds_legacy_compatible(),
        );
    }

    pub fn run_queue_playlist_item(
        &mut self,
        file_name: impl Into<String>,
        select_after_queue: bool,
    ) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_playlist_queue(file_name.into(), select_after_queue);
        self.run_local_playlist_action_batch(actions)
    }

    pub fn run_delete_playlist_index(&mut self, index: i64) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_playlist_delete(index);
        self.run_local_playlist_action_batch(actions)
    }

    pub fn run_replace_playlist(
        &mut self,
        files: Vec<String>,
        selected_index: Option<usize>,
    ) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_playlist_replace(files, selected_index);
        self.run_local_playlist_action_batch(actions)
    }

    pub fn run_undo_playlist_change(&mut self) -> Result<bool, PlayerError> {
        let actions = self.session.runtime_actions_for_local_playlist_undo();
        self.run_local_playlist_action_batch(actions)
    }

    pub fn run_shuffle_remaining_playlist(&mut self) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_playlist_shuffle_remaining();
        self.run_local_playlist_action_batch(actions)
    }

    pub fn run_shuffle_entire_playlist(&mut self) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_playlist_shuffle_entire();
        self.run_local_playlist_action_batch(actions)
    }

    pub fn run_seek_to_position(&mut self, target_position: f64) -> Result<bool, PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        // Cleanup is retried from subsequent observations if the adapter
        // rejects it; a failed rate reset must not swallow the user's seek.
        let _ = self.interrupt_playback_recovery(unix_wall_clock_time_seconds_legacy_compatible());
        let session_snapshot = self.session.snapshot_local_action_state();
        let actions = self.session.runtime_actions_for_local_seek(target_position);
        let sent = !actions.is_empty();
        self.dispatch_runtime_actions_with_session_rollback(session_snapshot, &actions)
            .map(|_| sent)
    }

    pub fn run_seek_by_offset(&mut self, offset_seconds: f64) -> Result<bool, PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let _ = self.interrupt_playback_recovery(unix_wall_clock_time_seconds_legacy_compatible());
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
        let _ = self.interrupt_playback_recovery(unix_wall_clock_time_seconds_legacy_compatible());
        let session_snapshot = self.session.snapshot_local_action_state();
        let actions = self.session.runtime_actions_for_local_seek_undo();
        let sent = !actions.is_empty();
        self.dispatch_runtime_actions_with_session_rollback(session_snapshot, &actions)
            .map(|_| sent)
    }

    pub fn run_disconnect(&mut self, now_seconds: f64) -> Result<(), PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        // Disconnect remains authoritative even if the player has already
        // gone away and cannot acknowledge the best-effort rate cleanup.
        let _ = self.interrupt_playback_recovery(now_seconds);
        let session_snapshot = self.session.snapshot_local_action_state();
        let actions = self.session.handle_disconnect(now_seconds);
        self.dispatch_runtime_actions_with_session_rollback_and_pause_cause(
            session_snapshot,
            &actions,
            PlayerCommandCause::TransportRefresh,
        )
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
        self.publish_pending_local_file_update_legacy_compatible_at(
            filename_privacy_mode,
            filesize_privacy_mode,
            unix_wall_clock_time_seconds_legacy_compatible(),
        )
    }

    /// Publishes one pending file observation using the caller's lifecycle
    /// clock. Runtime owners that use a monotonic clock must call this variant
    /// so startup evidence and later transport/status heartbeats remain in the
    /// same clock domain.
    pub fn publish_pending_local_file_update_legacy_compatible_at(
        &mut self,
        filename_privacy_mode: PrivacyMode,
        filesize_privacy_mode: PrivacyMode,
        now_seconds: f64,
    ) -> Result<bool, PlayerError> {
        let ordered_delivery = self.player.player_event_delivery_mode()
            == sorotte_player_api::PlayerEventDeliveryMode::OrderedAcknowledgedBatches;
        let local_file_update = if ordered_delivery {
            self.drain_player_transport_coordination(now_seconds)?;
            if self
                .playback_coordination
                .ordered_transport_awaits_snapshot()
            {
                return Ok(false);
            }
            self.pending_ordered_local_file_updates.front().cloned()
        } else {
            self.player.take_local_file_update()
        };
        let Some(local_file_update) = local_file_update else {
            return Ok(false);
        };

        let logical_id = logical_media_id_for_local_file_update(&local_file_update);
        let transport_kind = if local_file_update
            .path
            .as_deref()
            .is_some_and(|path| path.contains("://"))
            || local_file_update.name.contains("://")
        {
            MediaTransportKind::NetworkVod
        } else {
            MediaTransportKind::LocalFile
        };
        let file_payload = Self::local_file_update_payload(&local_file_update);
        self.last_local_file_update = Some(local_file_update.clone());
        self.publish_local_file_legacy_compatible(
            &file_payload,
            filename_privacy_mode,
            filesize_privacy_mode,
        )?;
        // Queue the compatible file announcement before an optional Sorotte
        // start barrier so peers learn the source before their preparation
        // deadline begins.
        self.prepare_playback_media_for_current_file_publication(
            logical_id,
            transport_kind,
            now_seconds,
        )?;
        if ordered_delivery {
            self.pending_ordered_local_file_updates.acknowledge_front();
        }
        Ok(true)
    }

    pub(crate) fn local_file_update_payload(local_file_update: &LocalFileUpdate) -> Value {
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

    pub(crate) fn sync_player_playback_telemetry_into_session_and_buffer(&mut self) {
        if self.player.player_event_delivery_mode()
            == sorotte_player_api::PlayerEventDeliveryMode::OrderedAcknowledgedBatches
        {
            return;
        }
        while let Some(update) = self.player.take_playback_telemetry_update() {
            self.observe_pending_reconnect_rate_reset(update.playback_rate);
            self.session.apply_player_playback_telemetry_update(&update);
            // Telemetry is a coalescible state effect: keep one pending snapshot
            // and let newer fields supersede older values before delivery.
            if let Some(pending) = self.pending_player_playback_telemetry_updates.back_mut() {
                pending.paused = update.paused.or(pending.paused);
                pending.position_seconds = update.position_seconds.or(pending.position_seconds);
                pending.playback_rate = update.playback_rate.or(pending.playback_rate);
                pending.paused_for_cache = update.paused_for_cache.or(pending.paused_for_cache);
                pending.cache_buffering_percent = update
                    .cache_buffering_percent
                    .or(pending.cache_buffering_percent);
            } else {
                self.pending_player_playback_telemetry_updates
                    .push_back(update);
            }
        }
    }
}

fn local_file_matches_playlist_target(file: &LocalFileUpdate, target: &str) -> bool {
    if file.name == target || file.path.as_deref() == Some(target) {
        return true;
    }
    if target.contains("://") {
        return false;
    }
    let target_name = std::path::Path::new(target)
        .file_name()
        .and_then(|name| name.to_str());
    target_name.is_some_and(|target_name| {
        target_name == file.name
            || file.path.as_deref().is_some_and(|path| {
                std::path::Path::new(path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    == Some(target_name)
            })
    })
}
