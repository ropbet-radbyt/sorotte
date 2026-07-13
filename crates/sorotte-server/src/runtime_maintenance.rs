use super::*;

impl ServerRuntime {
    pub(crate) fn current_time_seconds(&self) -> f64 {
        self.time_now_override_seconds
            .unwrap_or_else(current_unix_timestamp_seconds)
    }

    pub(crate) fn record_client_state_update_now(&mut self, client_id: &str) {
        self.client_last_state_update_at
            .insert(client_id.to_owned(), self.current_time_seconds());
    }

    pub(crate) fn initialize_stats_snapshot_schedule(&mut self) {
        self.initialize_stats_snapshot_schedule_at(self.current_time_seconds());
    }

    pub(crate) fn initialize_stats_snapshot_schedule_at(&mut self, now_seconds: f64) {
        if self.stats_persistence.is_none() {
            self.stats_next_snapshot_at_seconds = None;
            return;
        }
        self.stats_next_snapshot_at_seconds = Some(
            now_seconds
                + self.stats_snapshot_start_delay_seconds
                + self.stats_snapshot_interval_seconds,
        );
    }

    pub(crate) fn refresh_tls_context_from_cert_path(&mut self) {
        let Some(path) = self.tls_cert_path.as_ref() else {
            self.tls_server_config = None;
            self.tls_context_available = false;
            self.server_accepts_tls = false;
            self.tls_last_edit_cert_time = None;
            return;
        };
        if !tls_certificate_bundle_is_available(path) {
            self.tls_server_config = None;
            self.tls_context_available = false;
            self.server_accepts_tls = false;
            self.tls_last_edit_cert_time = None;
            return;
        }
        self.tls_last_edit_cert_time = tls_certificate_bundle_modified_time(path);
        match load_tls_server_config(path) {
            Ok(server_config) => {
                self.tls_server_config = Some(server_config);
                self.tls_context_available = true;
                self.server_accepts_tls = true;
            }
            Err(_) => {
                self.tls_server_config = None;
                self.tls_context_available = false;
                self.server_accepts_tls = false;
            }
        }
    }

    pub(crate) fn refresh_tls_context_after_cert_rotation_if_needed(&mut self) {
        let Some(path) = self.tls_cert_path.as_ref() else {
            return;
        };
        let Some(current_edit_time) = tls_certificate_bundle_modified_time(path) else {
            return;
        };
        if Some(current_edit_time) == self.tls_last_edit_cert_time {
            return;
        }
        self.refresh_tls_context_after_rotation_attempt();
    }

    pub(crate) fn refresh_tls_context_after_rotation_attempt(&mut self) {
        self.refresh_tls_context_from_cert_path();
        self.tls_rotation_attempts = self.tls_rotation_attempts.saturating_add(1);
        if self.tls_rotation_attempts < TLS_CERT_ROTATION_MAX_RETRIES {
            self.server_accepts_tls = true;
        }
    }

    pub(crate) fn collect_due_stats_snapshots_at(
        &mut self,
        now_seconds: f64,
    ) -> Result<(), ServerRuntimeError> {
        if self.stats_persistence.is_none() {
            self.stats_next_snapshot_at_seconds = None;
            return Ok(());
        }
        if self.stats_next_snapshot_at_seconds.is_none() {
            self.initialize_stats_snapshot_schedule_at(now_seconds);
        }
        let Some(mut next_snapshot_at_seconds) = self.stats_next_snapshot_at_seconds else {
            return Ok(());
        };
        while next_snapshot_at_seconds <= now_seconds {
            self.record_stats_snapshot_at(next_snapshot_at_seconds)?;
            next_snapshot_at_seconds += self.stats_snapshot_interval_seconds;
        }
        self.stats_next_snapshot_at_seconds = Some(next_snapshot_at_seconds);
        Ok(())
    }

    pub(crate) fn record_stats_snapshot_at(
        &self,
        snapshot_at_seconds: f64,
    ) -> Result<(), ServerRuntimeError> {
        let snapshot_time = snapshot_at_seconds.floor() as i64;
        let mut versions: Vec<String> = self
            .sessions
            .values()
            .map(|session| session.version.clone())
            .collect();
        versions.sort();
        if let Some(stats_persistence) = self.stats_persistence.as_ref() {
            stats_persistence.enqueue(ServerPersistenceEffect::RecordStatsSnapshot {
                snapshot_time,
                versions,
            });
        }
        Ok(())
    }

    pub(crate) fn collect_due_periodic_updates_at(
        &mut self,
        now: f64,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let mut outbound = self.collect_due_playback_barrier_updates_at(now)?;
        let mut due_clients: Vec<String> = self
            .client_next_periodic_state_at
            .iter()
            .filter(|(_, next_state_at)| **next_state_at <= now)
            .map(|(client_id, _)| client_id.clone())
            .collect();
        due_clients.sort();

        for client_id in due_clients {
            let Some(mut next_state_at) =
                self.client_next_periodic_state_at.get(&client_id).copied()
            else {
                continue;
            };
            while next_state_at <= now {
                self.client_next_periodic_state_at.insert(
                    client_id.clone(),
                    next_state_at + SERVER_STATE_INTERVAL_SECONDS,
                );
                outbound.extend(self.collect_periodic_tick_for_client_at(
                    &client_id,
                    next_state_at,
                    now,
                )?);
                if !self.sessions.contains_key(&client_id) {
                    break;
                }
                next_state_at += SERVER_STATE_INTERVAL_SECONDS;
            }
        }

        Ok(outbound)
    }

    pub(crate) fn collect_periodic_tick_for_client_at(
        &mut self,
        client_id: &str,
        ticked_at: f64,
        message_now_seconds: f64,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let Some(session) = self.sessions.get(client_id).cloned() else {
            return Ok(Vec::new());
        };
        // A fenced session still models a live transport until the network's
        // disconnect callback arrives. Periodic timeout cleanup must not
        // remove the session (and its fence) while the close event and a
        // racing protocol line are still in flight.
        if self.reject_fenced_playback_barrier_transport(client_id) {
            return Ok(Vec::new());
        }
        self.ensure_room_state(&session.room);
        if self.room_playback_state(&session.room).set_by.is_none()
            && let Some(set_by_username) = self.fallback_room_set_by_username(&session.room)
        {
            self.room_playback_state_mut(&session.room).set_by = Some(set_by_username);
        }
        let room_state = self.refresh_room_playback_state_from_clients_at(&session.room, ticked_at);

        let mut outbound = Vec::new();
        if let Some(state_message) = self.periodic_state_sync_message_for_client_at(
            client_id,
            room_state.position,
            room_state.paused,
            room_state.set_by.as_deref(),
            message_now_seconds,
        ) {
            outbound.push(DirectedProtocolMessage::new(client_id, state_message));
        }

        if self.client_timed_out(client_id, ticked_at) {
            self.pending_transport_actions
                .push(DirectedTransportAction::new(
                    client_id,
                    ServerTransportAction::Close,
                ));
            outbound.extend(self.timeout_disconnect_messages(client_id)?);
        }

        Ok(outbound)
    }

    pub(crate) fn fallback_room_set_by_username(&self, room_name: &str) -> Option<String> {
        self.sessions
            .iter()
            .filter(|(client_id, session)| {
                session.room == room_name
                    && !self.playback_barrier_fenced_clients.contains(*client_id)
            })
            .min_by_key(|(client_id, _)| self.client_room_join_order(client_id))
            .map(|(_, session)| session.username.clone())
    }

    pub(crate) fn assign_room_join_order(&mut self, client_id: &str) {
        self.client_room_join_sequence
            .insert(client_id.to_owned(), self.next_room_join_sequence);
        self.next_room_join_sequence = self.next_room_join_sequence.saturating_add(1);
    }

    pub(crate) fn client_room_join_order(&self, client_id: &str) -> u64 {
        self.client_room_join_sequence
            .get(client_id)
            .copied()
            .unwrap_or(u64::MAX)
    }

    pub(crate) fn periodic_state_sync_message_for_client_at(
        &mut self,
        client_id: &str,
        position: f64,
        paused: bool,
        set_by: Option<&str>,
        now_seconds: f64,
    ) -> Option<ProtocolMessage> {
        let server_ignoring_counter = self.server_ignoring_counter(client_id);
        let server_rtt_seconds = self.server_rtt_seconds(client_id);
        let (pending_client_latency, pending_client_ignoring) =
            self.take_client_passthrough_state_metadata_at(client_id, now_seconds);
        if server_ignoring_counter > 0 {
            return None;
        }

        Some(state_sync_message(
            position,
            paused,
            false,
            StateSyncOptions {
                set_by,
                client_latency_calculation: pending_client_latency,
                client_ignoring_counter: pending_client_ignoring,
                server_rtt_seconds,
                latency_calculation_seconds: Some(now_seconds),
                ..StateSyncOptions::default()
            },
        ))
    }

    pub(crate) fn client_timed_out(&self, client_id: &str, now_seconds: f64) -> bool {
        self.client_last_state_update_at
            .get(client_id)
            .is_some_and(|updated_at| now_seconds - updated_at > PROTOCOL_TIMEOUT_SECONDS)
    }

    pub(crate) fn timeout_disconnect_messages(
        &mut self,
        client_id: &str,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let Some(session) = self.sessions.get(client_id).cloned() else {
            return Ok(Vec::new());
        };
        let mut outbound_messages =
            self.mark_playback_barrier_participant_disconnected(client_id, &session.room)?;
        outbound_messages
            .extend(self.mark_room_buffering_participant_disconnected(client_id, &session.room)?);
        let Some(session) = self.remove_session_tracking(client_id) else {
            return Ok(outbound_messages);
        };
        self.cleanup_room_if_empty(&session.room)?;
        let left_message = user_event_message(
            &session.username,
            &session.room,
            json!({
                "left": true,
            }),
        );
        let mut recipients = if self.isolate_rooms {
            self.clients_in_room(&session.room)
        } else {
            self.clients_all()
        };
        recipients.push(client_id.to_owned());
        outbound_messages.extend(
            recipients
                .into_iter()
                .map(|peer_client| DirectedProtocolMessage::new(peer_client, left_message.clone())),
        );
        if self.persistent_rooms_enabled {
            self.enqueue_list_snapshots_for_clients(
                &mut outbound_messages,
                self.clients_receiving_to_gui_only_list_updates(None),
            );
        }
        Ok(outbound_messages)
    }

    pub(crate) fn remove_session_tracking(&mut self, client_id: &str) -> Option<ServerSession> {
        let session = self.sessions.remove(client_id)?;
        self.playback_barrier_fenced_clients.remove(client_id);
        let _ = self.domain.leave_room(&session.username, &session.room);
        self.remove_room_controller(&session.username, &session.room);
        self.client_state_counters.remove(client_id);
        self.client_playback_states.remove(client_id);
        self.client_room_join_sequence.remove(client_id);
        self.playback_barrier_request_nonces.remove(client_id);
        self.client_last_state_update_at.remove(client_id);
        self.client_next_periodic_state_at.remove(client_id);
        Some(session)
    }

    pub(crate) fn apply_persisted_rooms_snapshot(
        &mut self,
        persisted_rooms: BTreeMap<String, PersistedRoomState>,
    ) {
        let now_seconds = self.current_time_seconds();
        for (room_name, persisted_room) in persisted_rooms {
            self.room_playlists.insert(
                room_name.clone(),
                RoomPlaylistState {
                    files: persisted_room.files,
                    index: persisted_room.index,
                },
            );
            let room_playback = self.room_playback_states.entry(room_name).or_default();
            room_playback.position = persisted_room.position;
            room_playback.updated_at_seconds = now_seconds;
        }
    }

    pub(crate) fn apply_permanent_rooms_snapshot(&mut self) {
        if self.room_persistence.is_none() {
            return;
        }
        let now_seconds = self.current_time_seconds();
        for room_name in self.permanent_rooms.clone() {
            self.room_playlists
                .entry(room_name.clone())
                .or_insert_with(|| RoomPlaylistState {
                    files: Vec::new(),
                    index: Some(0),
                });
            self.room_controllers.entry(room_name.clone()).or_default();
            self.room_playback_states
                .entry(room_name)
                .or_insert_with(|| RoomPlaybackState::new_at(now_seconds));
        }
    }

    pub(crate) fn room_is_persistent(&self, room_name: &str) -> bool {
        self.persistent_rooms_enabled && !room_name_is_marked_temporary(room_name)
    }

    pub(crate) fn room_is_permanent(&self, room_name: &str) -> bool {
        self.room_persistence.is_some() && self.permanent_rooms.contains(room_name)
    }

    pub(crate) fn room_should_be_retained_when_empty(&self, room_name: &str) -> bool {
        self.room_is_persistent(room_name) && !self.room_playlist_state(room_name).files.is_empty()
    }

    pub(crate) fn persist_room_if_needed(
        &mut self,
        room_name: &str,
    ) -> Result<(), ServerRuntimeError> {
        if !self.room_is_persistent(room_name) {
            return Ok(());
        }
        if self.room_persistence.is_none() {
            return Ok(());
        }
        let playlist = self.room_playlist_state(room_name).clone();
        let playback = self.room_playback_state_at(room_name, self.current_time_seconds());
        let version = self.next_room_persistence_version(room_name);
        self.room_persistence
            .as_ref()
            .expect("room persistence presence checked above")
            .enqueue(ServerPersistenceEffect::SaveRoom {
                room_name: room_name.to_owned(),
                files: playlist.files,
                playlist_index: playlist.index,
                position: playback.position,
                version,
            });
        Ok(())
    }

    pub(crate) fn delete_persisted_room_if_needed(
        &mut self,
        room_name: &str,
    ) -> Result<(), ServerRuntimeError> {
        if self.room_persistence.is_none() {
            return Ok(());
        }
        let version = self.next_room_persistence_version(room_name);
        self.room_persistence
            .as_ref()
            .expect("room persistence presence checked above")
            .enqueue(ServerPersistenceEffect::DeleteRoom {
                room_name: room_name.to_owned(),
                version,
            });
        Ok(())
    }

    fn next_room_persistence_version(&mut self, room_name: &str) -> u64 {
        let version = self
            .room_persistence_versions
            .entry(room_name.to_owned())
            .or_default();
        *version = version.saturating_add(1);
        *version
    }

    pub(crate) fn cleanup_room_if_empty(
        &mut self,
        room_name: &str,
    ) -> Result<(), ServerRuntimeError> {
        if !self.clients_in_room(room_name).is_empty() {
            return Ok(());
        }
        if self.room_is_permanent(room_name) {
            self.persist_room_if_needed(room_name)?;
            return Ok(());
        }
        if self.room_should_be_retained_when_empty(room_name) {
            self.persist_room_if_needed(room_name)?;
            return Ok(());
        }
        self.room_controllers.remove(room_name);
        self.room_playlists.remove(room_name);
        self.room_playback_states.remove(room_name);
        self.room_playback_barriers.remove(room_name);
        self.room_buffering_controls.remove(room_name);
        self.playback_barrier_request_tombstones
            .retain(|(tombstone_room, _), _| tombstone_room != room_name);
        self.delete_persisted_room_if_needed(room_name)?;
        Ok(())
    }

    pub(crate) fn ensure_room_state(&mut self, room_name: &str) {
        let now_seconds = self.current_time_seconds();
        self.room_playlists.entry(room_name.to_owned()).or_default();
        self.room_controllers
            .entry(room_name.to_owned())
            .or_default();
        self.room_playback_states
            .entry(room_name.to_owned())
            .or_insert_with(|| RoomPlaybackState::new_at(now_seconds));
    }

    pub(crate) fn room_playlist_state_mut(&mut self, room_name: &str) -> &mut RoomPlaylistState {
        self.room_playlists.entry(room_name.to_owned()).or_default()
    }

    pub(crate) fn room_playlist_state(&self, room_name: &str) -> RoomPlaylistState {
        self.room_playlists
            .get(room_name)
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn room_playback_state_mut(&mut self, room_name: &str) -> &mut RoomPlaybackState {
        let now_seconds = self.current_time_seconds();
        self.room_playback_states
            .entry(room_name.to_owned())
            .or_insert_with(|| RoomPlaybackState::new_at(now_seconds))
    }

    pub(crate) fn room_playback_state(&self, room_name: &str) -> RoomPlaybackState {
        self.room_playback_states
            .get(room_name)
            .cloned()
            .unwrap_or_else(|| RoomPlaybackState::new_at(self.current_time_seconds()))
    }

    pub(crate) fn room_playback_state_at(
        &self,
        room_name: &str,
        now_seconds: f64,
    ) -> RoomPlaybackState {
        self.room_playback_state(room_name).aged_at(now_seconds)
    }

    pub(crate) fn seed_client_playback_state(
        &mut self,
        client_id: &str,
        position: Option<f64>,
        now_seconds: f64,
    ) {
        let position = position.filter(|position| position.is_finite());
        self.client_playback_states.insert(
            client_id.to_owned(),
            ClientPlaybackState::new(position, now_seconds),
        );
    }

    pub(crate) fn record_client_playback_state_sample(
        &mut self,
        client_id: &str,
        position: Option<f64>,
        now_seconds: f64,
    ) {
        let position = position.filter(|position| position.is_finite());
        let playback_state = self
            .client_playback_states
            .entry(client_id.to_owned())
            .or_insert_with(|| ClientPlaybackState::new(None, now_seconds));
        if let Some(position) = position {
            playback_state.position = Some(position);
        }
        playback_state.updated_at_seconds = now_seconds;
    }

    pub(crate) fn seed_room_client_playback_states(
        &mut self,
        room_name: &str,
        position: f64,
        now_seconds: f64,
    ) {
        if !position.is_finite() {
            return;
        }
        for client_id in self.clients_in_room(room_name) {
            self.seed_client_playback_state(&client_id, Some(position), now_seconds);
        }
    }

    pub(crate) fn slowest_room_playback_client_at(
        &self,
        room_name: &str,
        room_paused: bool,
        now_seconds: f64,
    ) -> Option<(String, f64)> {
        let controlled_room = self
            .room_password_provider
            .is_controlled_room_name(room_name);
        let mut slowest: Option<(String, f64, u64)> = None;
        for (client_id, session) in &self.sessions {
            if session.room != room_name || self.playback_barrier_fenced_clients.contains(client_id)
            {
                continue;
            }
            if controlled_room && !self.user_is_room_controller(&session.username, room_name) {
                continue;
            }
            if session.file.is_none() {
                continue;
            }
            let Some(position) = self
                .client_playback_states
                .get(client_id)
                .and_then(|state| state.position_at(room_paused, now_seconds))
            else {
                continue;
            };
            let room_join_order = self.client_room_join_order(client_id);
            if slowest
                .as_ref()
                .is_none_or(|(_, slowest_position, slowest_room_join_order)| {
                    position < *slowest_position
                        || (position == *slowest_position
                            && room_join_order < *slowest_room_join_order)
                })
            {
                slowest = Some((session.username.clone(), position, room_join_order));
            }
        }
        slowest.map(|(username, position, _)| (username, position))
    }

    pub(crate) fn refresh_room_playback_state_from_clients_at(
        &mut self,
        room_name: &str,
        now_seconds: f64,
    ) -> RoomPlaybackState {
        let current = self.room_playback_state(room_name);
        let age_seconds = now_seconds - current.updated_at_seconds;
        if !age_seconds.is_finite() || age_seconds <= SERVER_STATE_INTERVAL_SECONDS {
            return current.aged_at(now_seconds);
        }
        let Some((set_by, position)) =
            self.slowest_room_playback_client_at(room_name, current.paused, now_seconds)
        else {
            return current.aged_at(now_seconds);
        };
        let room_playback = self.room_playback_state_mut(room_name);
        room_playback.position = position;
        room_playback.updated_at_seconds = now_seconds;
        room_playback.set_by = Some(set_by);
        room_playback.clone()
    }

    pub(crate) fn acknowledge_server_ignoring_counter(
        &mut self,
        client_id: &str,
        server_counter: u32,
    ) {
        let Some(state_counters) = self.client_state_counters.get_mut(client_id) else {
            return;
        };
        if state_counters.server_ignoring_on_the_fly == server_counter {
            state_counters.server_ignoring_on_the_fly = 0;
        }
    }

    pub(crate) fn server_ignoring_counter(&self, client_id: &str) -> u32 {
        self.client_state_counters
            .get(client_id)
            .map(|state_counters| state_counters.server_ignoring_on_the_fly)
            .unwrap_or_default()
    }

    pub(crate) fn next_server_ignoring_counter(&mut self, client_id: &str) -> u32 {
        let state_counters = self
            .client_state_counters
            .entry(client_id.to_owned())
            .or_default();
        state_counters.server_ignoring_on_the_fly =
            state_counters.server_ignoring_on_the_fly.saturating_add(1);
        state_counters.server_ignoring_on_the_fly
    }

    pub(crate) fn queue_client_ignoring_counter(
        &mut self,
        client_id: &str,
        client_ignoring_counter: u32,
    ) {
        let state_counters = self
            .client_state_counters
            .entry(client_id.to_owned())
            .or_default();
        state_counters.pending_client_ignoring_on_the_fly = Some(client_ignoring_counter);
    }

    pub(crate) fn queue_client_latency_calculation(
        &mut self,
        client_id: &str,
        client_latency: f64,
    ) {
        let now_seconds = self.current_time_seconds();
        let state_counters = self
            .client_state_counters
            .entry(client_id.to_owned())
            .or_default();
        state_counters.pending_client_latency_calculation = Some(client_latency);
        state_counters.pending_client_latency_calculation_arrival_time = Some(now_seconds);
    }

    pub(crate) fn ingest_client_ping_metrics(
        &mut self,
        client_id: &str,
        latency_calculation: Option<f64>,
        client_rtt: Option<f64>,
    ) {
        let Some(latency_calculation) = latency_calculation else {
            return;
        };
        let sender_rtt = client_rtt.unwrap_or(0.0);
        if !latency_calculation.is_finite() || !sender_rtt.is_finite() || sender_rtt < 0.0 {
            return;
        }

        let current_rtt_seconds = self.current_time_seconds() - latency_calculation;
        if !current_rtt_seconds.is_finite() || current_rtt_seconds < 0.0 {
            return;
        }

        let state_counters = self
            .client_state_counters
            .entry(client_id.to_owned())
            .or_default();
        state_counters.ping_rtt_seconds = current_rtt_seconds;
        if state_counters.ping_average_rtt_seconds == 0.0 {
            state_counters.ping_average_rtt_seconds = current_rtt_seconds;
        }
        state_counters.ping_average_rtt_seconds = state_counters.ping_average_rtt_seconds
            * PING_MOVING_AVERAGE_WEIGHT
            + current_rtt_seconds * (1.0 - PING_MOVING_AVERAGE_WEIGHT);
        if sender_rtt < current_rtt_seconds {
            state_counters.ping_forward_delay_seconds =
                state_counters.ping_average_rtt_seconds / 2.0 + (current_rtt_seconds - sender_rtt);
        } else {
            state_counters.ping_forward_delay_seconds =
                state_counters.ping_average_rtt_seconds / 2.0;
        }
    }

    pub(crate) fn server_rtt_seconds(&self, client_id: &str) -> f64 {
        self.client_state_counters
            .get(client_id)
            .map(|state_counters| state_counters.ping_rtt_seconds)
            .unwrap_or_default()
    }

    pub(crate) fn forward_delay_seconds(&self, client_id: &str) -> f64 {
        self.client_state_counters
            .get(client_id)
            .map(|state_counters| state_counters.ping_forward_delay_seconds)
            .unwrap_or_default()
    }

    pub(crate) fn take_client_passthrough_state_metadata(
        &mut self,
        client_id: &str,
    ) -> (Option<f64>, Option<u32>) {
        self.take_client_passthrough_state_metadata_at(client_id, self.current_time_seconds())
    }

    pub(crate) fn take_client_passthrough_state_metadata_at(
        &mut self,
        client_id: &str,
        now_seconds: f64,
    ) -> (Option<f64>, Option<u32>) {
        let state_counters = self
            .client_state_counters
            .entry(client_id.to_owned())
            .or_default();
        let pending_client_latency = state_counters.pending_client_latency_calculation.take();
        let pending_client_latency_arrival_time = state_counters
            .pending_client_latency_calculation_arrival_time
            .take();
        let pending_client_latency = pending_client_latency.map(|client_latency| {
            let processing_time = pending_client_latency_arrival_time
                .map(|arrival_time| now_seconds - arrival_time)
                .filter(|processing_time| processing_time.is_finite() && *processing_time >= 0.0)
                .unwrap_or(0.0);
            client_latency + processing_time
        });
        let pending_client_ignoring = state_counters.pending_client_ignoring_on_the_fly.take();
        (pending_client_latency, pending_client_ignoring)
    }

    pub(crate) fn forced_state_sync_message_for_client(
        &mut self,
        client_id: &str,
        position: f64,
        paused: bool,
        do_seek: impl Into<Option<bool>>,
        set_by: Option<&str>,
    ) -> ProtocolMessage {
        let server_ignoring_counter = self.next_server_ignoring_counter(client_id);
        let server_rtt_seconds = self.server_rtt_seconds(client_id);
        let (pending_client_latency, pending_client_ignoring) =
            self.take_client_passthrough_state_metadata(client_id);
        state_sync_message(
            position,
            paused,
            do_seek,
            StateSyncOptions {
                set_by,
                server_ignoring_counter: Some(server_ignoring_counter),
                client_latency_calculation: pending_client_latency,
                client_ignoring_counter: pending_client_ignoring,
                server_rtt_seconds,
                latency_calculation_seconds: Some(self.current_time_seconds()),
            },
        )
    }

    pub(crate) fn add_room_controller(&mut self, username: &str, room_name: &str) {
        self.ensure_room_state(room_name);
        if let Some(room_controllers) = self.room_controllers.get_mut(room_name) {
            room_controllers.insert(username.to_owned());
        }
    }

    pub(crate) fn remove_room_controller(&mut self, username: &str, room_name: &str) {
        if let Some(room_controllers) = self.room_controllers.get_mut(room_name) {
            room_controllers.remove(username);
        }
    }

    pub(crate) fn user_is_room_controller(&self, username: &str, room_name: &str) -> bool {
        self.room_controllers
            .get(room_name)
            .is_some_and(|controllers| controllers.contains(username))
    }

    pub(crate) fn user_can_control_playlist(&self, username: &str, room_name: &str) -> bool {
        !self
            .room_password_provider
            .is_controlled_room_name(room_name)
            || self.user_is_room_controller(username, room_name)
    }

    pub(crate) fn clients_in_room(&self, room_name: &str) -> Vec<String> {
        self.sessions
            .iter()
            .filter(|(_, session)| session.room == room_name)
            .map(|(client_id, _)| client_id.clone())
            .collect()
    }

    pub(crate) fn legacy_readiness_chat_clients_in_room(&self, room_name: &str) -> Vec<String> {
        self.sessions
            .iter()
            .filter(|(_, session)| {
                session.room == room_name
                    && client_version_meets_minimum(&session.version, LEGACY_CHAT_MIN_VERSION)
                    && !session.capabilities.remote_readiness
            })
            .map(|(client_id, _)| client_id.clone())
            .collect()
    }

    pub(crate) fn chat_clients_in_room(&self, room_name: &str) -> Vec<String> {
        self.sessions
            .iter()
            .filter(|(_, session)| {
                session.room == room_name
                    && client_version_meets_minimum(&session.version, LEGACY_CHAT_MIN_VERSION)
            })
            .map(|(client_id, _)| client_id.clone())
            .collect()
    }

    pub(crate) fn clients_all(&self) -> Vec<String> {
        self.sessions.keys().cloned().collect()
    }

    pub(crate) fn clients_receiving_to_gui_only_list_updates(
        &self,
        room_name: Option<&str>,
    ) -> Vec<String> {
        self.sessions
            .iter()
            .filter(|(_, session)| {
                session.capabilities.ui_mode_advertised
                    && (!self.isolate_rooms
                        || room_name.is_some_and(|room_name| session.room == room_name))
            })
            .map(|(client_id, _)| client_id.clone())
            .collect()
    }

    pub(crate) fn clients_all_excluding(&self, excluded_client_id: &str) -> Vec<String> {
        self.sessions
            .keys()
            .filter(|client_id| client_id.as_str() != excluded_client_id)
            .cloned()
            .collect()
    }

    pub(crate) fn clients_visible_on_join(
        &self,
        room_name: &str,
        joining_client_id: &str,
    ) -> Vec<String> {
        if self.isolate_rooms {
            self.clients_in_room(room_name)
                .into_iter()
                .filter(|client_id| client_id != joining_client_id)
                .collect()
        } else {
            self.clients_all_excluding(joining_client_id)
        }
    }

    pub(crate) fn room_switch_visibility_recipients(
        &self,
        moving_client_id: &str,
        _previous_room: &str,
        current_room: &str,
    ) -> Vec<String> {
        if !self.isolate_rooms {
            return self.clients_all();
        }
        let mut recipients = BTreeSet::new();
        recipients.insert(moving_client_id.to_owned());
        for client_id in self.clients_in_room(current_room) {
            recipients.insert(client_id);
        }
        recipients.into_iter().collect()
    }

    pub(crate) fn user_ready(&self, username: &str, room_name: &str) -> Option<bool> {
        if !self.readiness_enabled {
            return None;
        }
        self.stored_user_ready(username, room_name)
    }

    pub(crate) fn stored_user_ready(&self, username: &str, room_name: &str) -> Option<bool> {
        self.domain.users_in_room(room_name).and_then(|users| {
            users
                .into_iter()
                .find(|user| user.username == username)
                .and_then(|user| user.ready)
        })
    }

    pub(crate) fn file_payload_for_client_from_source(
        &self,
        client_id: &str,
        source_client_id: &str,
        file: &ServerSharedFile,
    ) -> Value {
        file.to_wire_value(
            self.client_session_supports_media_match(client_id) && client_id != source_client_id,
        )
    }

    pub(crate) fn playlist_change_message_for_client(
        &self,
        client_id: &str,
        files: Vec<String>,
        set_by: Option<&str>,
    ) -> ProtocolMessage {
        let mut playlist_change = playlist_change_with_plex_sidecar(
            files,
            self.client_session_supports_sorotte_plex_playlist_uris(client_id),
        );
        playlist_change = if let Some(set_by) = set_by {
            playlist_change.with_user(set_by)
        } else {
            playlist_change.with_null_user()
        };
        ProtocolMessage::set(SetPayload::new().with_playlist_change(playlist_change))
    }

    fn client_session_supports_media_match(&self, client_id: &str) -> bool {
        self.sessions
            .get(client_id)
            .is_some_and(|session| session.capabilities.media_match)
    }

    fn client_session_supports_sorotte_plex_playlist_uris(&self, client_id: &str) -> bool {
        self.sessions
            .get(client_id)
            .is_some_and(|session| session.capabilities.plex_playlist_uris)
    }

    fn sanitize_list_rooms_snapshot_for_client(
        &self,
        client_id: &str,
        rooms: &mut BTreeMap<String, BTreeMap<String, ListUserEntry>>,
    ) {
        let supports_media_match = self.client_session_supports_media_match(client_id);
        let own_username = self
            .sessions
            .get(client_id)
            .map(|session| session.username.as_str());
        for room_entries in rooms.values_mut() {
            for (username, entry) in room_entries.iter_mut() {
                if supports_media_match && own_username != Some(username.as_str()) {
                    continue;
                }
                let Some(file) = entry.file.as_mut() else {
                    continue;
                };
                let Some(file_object) = file.as_object_mut() else {
                    continue;
                };
                file_object.remove("mediaMatch");
            }
        }
    }

    pub(crate) fn list_rooms_snapshot_for_client(
        &self,
        client_id: &str,
    ) -> BTreeMap<String, BTreeMap<String, ListUserEntry>> {
        let mut rooms = if self.isolate_rooms {
            let Some(session) = self.sessions.get(client_id) else {
                return BTreeMap::new();
            };
            let mut all_rooms = self.list_rooms_snapshot();
            let mut rooms = BTreeMap::new();
            if let Some(room_entries) = all_rooms.remove(&session.room) {
                rooms.insert(session.room.clone(), room_entries);
            }
            rooms
        } else {
            self.list_rooms_snapshot()
        };
        if self
            .sessions
            .get(client_id)
            .is_some_and(|session| session.capabilities.is_gui_user())
        {
            self.add_empty_room_dummy_entries(&mut rooms);
        }
        self.sanitize_list_rooms_snapshot_for_client(client_id, &mut rooms);
        rooms
    }

    pub(crate) fn list_rooms_snapshot(&self) -> BTreeMap<String, BTreeMap<String, ListUserEntry>> {
        let mut rooms = BTreeMap::new();
        for session in self.sessions.values() {
            let ready = self.user_ready(&session.username, &session.room);
            let mut entry = ListUserEntry::new()
                .with_position(0.0)
                .with_file(
                    session
                        .file
                        .as_ref()
                        .map(|file| file.to_wire_value(true))
                        .unwrap_or_else(|| json!({})),
                )
                .with_controller(self.user_is_room_controller(&session.username, &session.room));
            if let Some(ready) = ready {
                entry = entry.with_is_ready(ready);
            }
            entry = entry.with_features(session.capabilities.to_wire_value());
            rooms
                .entry(session.room.clone())
                .or_insert_with(BTreeMap::new)
                .insert(session.username.clone(), entry);
        }
        rooms
    }

    pub(crate) fn add_empty_room_dummy_entries(
        &self,
        rooms: &mut BTreeMap<String, BTreeMap<String, ListUserEntry>>,
    ) {
        let mut known_rooms = BTreeSet::new();
        known_rooms.extend(self.room_controllers.keys().cloned());
        known_rooms.extend(self.room_playlists.keys().cloned());
        known_rooms.extend(self.room_playback_states.keys().cloned());

        let mut dummy_count = 0usize;
        for room_name in known_rooms {
            if !self.clients_in_room(&room_name).is_empty() {
                continue;
            }
            dummy_count = dummy_count.saturating_add(1);
            rooms
                .entry(room_name)
                .or_default()
                .insert(" ".repeat(dummy_count), legacy_dummy_list_entry());
        }
    }

    pub(crate) fn enqueue_list_snapshots_for_clients(
        &self,
        outbound_messages: &mut Vec<DirectedProtocolMessage>,
        recipients: Vec<String>,
    ) {
        for client_id in recipients {
            let rooms = self.list_rooms_snapshot_for_client(&client_id);
            outbound_messages.push(DirectedProtocolMessage::new(
                client_id,
                ProtocolMessage::list(ListPayload::rooms(rooms)),
            ));
        }
    }

    pub(crate) fn find_free_username(
        &self,
        username: &str,
        excluded_client_id: Option<&str>,
    ) -> String {
        let mut chosen_username = username.to_owned();
        let all_names: BTreeSet<String> = self
            .sessions
            .iter()
            .filter(|(client_id, _)| {
                excluded_client_id.is_none_or(|excluded| *client_id != excluded)
            })
            .map(|(_, session)| session.username.to_ascii_lowercase())
            .collect();

        if all_names.contains(&chosen_username.to_ascii_lowercase())
            && chosen_username.ends_with('_')
        {
            chosen_username = chosen_username.trim_end_matches('_').to_owned();
            if chosen_username.is_empty() {
                chosen_username = "_".to_owned();
            }
        }

        while all_names.contains(&chosen_username.to_ascii_lowercase()) {
            chosen_username.push('_');
        }
        chosen_username
    }
}
