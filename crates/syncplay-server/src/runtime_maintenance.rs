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
        if self.stats_persistence.is_none() {
            self.stats_next_snapshot_at_seconds = None;
            return;
        }
        self.stats_next_snapshot_at_seconds = Some(
            self.current_time_seconds()
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
        self.tls_last_edit_cert_time = tls_certificate_file_modified_time(path);
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
        let Some(current_edit_time) = tls_certificate_file_modified_time(path) else {
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

    pub(crate) fn collect_due_stats_snapshots(&mut self) -> Result<(), ServerRuntimeError> {
        let Some(stats_persistence) = self.stats_persistence.clone() else {
            self.stats_next_snapshot_at_seconds = None;
            return Ok(());
        };
        if self.stats_next_snapshot_at_seconds.is_none() {
            self.initialize_stats_snapshot_schedule();
        }
        let Some(mut next_snapshot_at_seconds) = self.stats_next_snapshot_at_seconds else {
            return Ok(());
        };
        let now_seconds = self.current_time_seconds();
        while next_snapshot_at_seconds <= now_seconds {
            self.record_stats_snapshot_at(&stats_persistence, next_snapshot_at_seconds)?;
            next_snapshot_at_seconds += self.stats_snapshot_interval_seconds;
        }
        self.stats_next_snapshot_at_seconds = Some(next_snapshot_at_seconds);
        Ok(())
    }

    pub(crate) fn record_stats_snapshot_at(
        &self,
        stats_persistence: &StatsPersistenceStore,
        snapshot_at_seconds: f64,
    ) -> Result<(), ServerRuntimeError> {
        let snapshot_time = snapshot_at_seconds.floor() as i64;
        let mut versions: Vec<String> = self
            .sessions
            .values()
            .map(|session| session.version.clone())
            .collect();
        versions.sort();
        for version in versions {
            stats_persistence.add_version_log(snapshot_time, &version)?;
        }
        Ok(())
    }

    pub(crate) fn collect_due_periodic_updates(
        &mut self,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let now = self.current_time_seconds();
        let mut due_clients: Vec<String> = self
            .client_next_periodic_state_at
            .iter()
            .filter(|(_, next_state_at)| **next_state_at <= now)
            .map(|(client_id, _)| client_id.clone())
            .collect();
        due_clients.sort();

        let mut outbound = Vec::new();
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
                outbound.extend(self.collect_periodic_tick_for_client(&client_id, next_state_at)?);
                if !self.sessions.contains_key(&client_id) {
                    break;
                }
                next_state_at += SERVER_STATE_INTERVAL_SECONDS;
            }
        }

        Ok(outbound)
    }

    pub(crate) fn collect_periodic_tick_for_client(
        &mut self,
        client_id: &str,
        ticked_at: f64,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let Some(session) = self.sessions.get(client_id).cloned() else {
            return Ok(Vec::new());
        };
        self.ensure_room_state(&session.room);
        if self.room_playback_state(&session.room).set_by.is_none()
            && let Some(set_by_username) = self.fallback_room_set_by_username(&session.room)
        {
            self.room_playback_state_mut(&session.room).set_by = Some(set_by_username);
        }
        let room_state = self.room_playback_state_at(&session.room, ticked_at);

        let mut outbound = Vec::new();
        if let Some(state_message) = self.periodic_state_sync_message_for_client(
            client_id,
            room_state.position,
            room_state.paused,
            room_state.set_by.as_deref(),
        ) {
            outbound.push(DirectedProtocolMessage::new(client_id, state_message));
        }

        if self.client_timed_out(client_id, ticked_at) {
            outbound.extend(self.timeout_disconnect_messages(client_id)?);
        }

        Ok(outbound)
    }

    pub(crate) fn fallback_room_set_by_username(&self, room_name: &str) -> Option<String> {
        let mut usernames: Vec<String> = self
            .sessions
            .values()
            .filter(|session| session.room == room_name)
            .map(|session| session.username.clone())
            .collect();
        usernames.sort();
        usernames.into_iter().next()
    }

    pub(crate) fn periodic_state_sync_message_for_client(
        &mut self,
        client_id: &str,
        position: f64,
        paused: bool,
        set_by: Option<&str>,
    ) -> Option<ProtocolMessage> {
        let server_ignoring_counter = self.server_ignoring_counter(client_id);
        let server_rtt_seconds = self.server_rtt_seconds(client_id);
        let (pending_client_latency, pending_client_ignoring) =
            self.take_client_passthrough_state_metadata(client_id);
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
                latency_calculation_seconds: Some(self.current_time_seconds()),
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
        let Some(session) = self.remove_session_tracking(client_id) else {
            return Ok(Vec::new());
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
        let mut outbound_messages: Vec<_> = recipients
            .into_iter()
            .map(|peer_client| DirectedProtocolMessage::new(peer_client, left_message.clone()))
            .collect();
        if self.persistent_rooms_enabled {
            self.enqueue_list_snapshots_for_clients(
                &mut outbound_messages,
                self.clients_receiving_to_gui_only_list_updates(),
            );
        }
        Ok(outbound_messages)
    }

    pub(crate) fn remove_session_tracking(&mut self, client_id: &str) -> Option<ServerSession> {
        let session = self.sessions.remove(client_id)?;
        let _ = self.domain.leave_room(&session.username, &session.room);
        self.remove_room_controller(&session.username, &session.room);
        self.client_state_counters.remove(client_id);
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

    pub(crate) fn persist_room_if_needed(&self, room_name: &str) -> Result<(), ServerRuntimeError> {
        if !self.room_is_persistent(room_name) {
            return Ok(());
        }
        let Some(room_persistence) = self.room_persistence.as_ref() else {
            return Ok(());
        };
        let playlist = self.room_playlist_state(room_name);
        let playback = self.room_playback_state_at(room_name, self.current_time_seconds());
        room_persistence.save_room(
            room_name,
            &playlist.files,
            playlist.index,
            playback.position,
        )?;
        Ok(())
    }

    pub(crate) fn delete_persisted_room_if_needed(
        &self,
        room_name: &str,
    ) -> Result<(), ServerRuntimeError> {
        let Some(room_persistence) = self.room_persistence.as_ref() else {
            return Ok(());
        };
        room_persistence.delete_room(room_name)?;
        Ok(())
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
        let now_seconds = self.current_time_seconds();
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
        do_seek: bool,
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
                    && !client_supports_feature(session.features.as_ref(), "setOthersReadiness")
            })
            .map(|(client_id, _)| client_id.clone())
            .collect()
    }

    pub(crate) fn clients_all(&self) -> Vec<String> {
        self.sessions.keys().cloned().collect()
    }

    pub(crate) fn clients_receiving_to_gui_only_list_updates(&self) -> Vec<String> {
        self.sessions
            .iter()
            .filter(|(_, session)| features_include_ui_mode(session.features.as_ref()))
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
        self.domain.users_in_room(room_name).and_then(|users| {
            users
                .into_iter()
                .find(|user| user.username == username)
                .map(|user| user.ready)
        })
    }

    pub(crate) fn list_rooms_snapshot_for_client(
        &self,
        client_id: &str,
    ) -> BTreeMap<String, BTreeMap<String, ListUserEntry>> {
        if self.isolate_rooms {
            let Some(session) = self.sessions.get(client_id) else {
                return BTreeMap::new();
            };
            let mut all_rooms = self.list_rooms_snapshot();
            let mut rooms = BTreeMap::new();
            if let Some(room_entries) = all_rooms.remove(&session.room) {
                rooms.insert(session.room.clone(), room_entries);
            }
            return rooms;
        }
        let mut rooms = self.list_rooms_snapshot();
        if client_is_gui_user(
            self.sessions
                .get(client_id)
                .and_then(|session| session.features.as_ref()),
        ) {
            self.add_empty_room_dummy_entries(&mut rooms);
        }
        rooms
    }

    pub(crate) fn list_rooms_snapshot(&self) -> BTreeMap<String, BTreeMap<String, ListUserEntry>> {
        let mut rooms = BTreeMap::new();
        for session in self.sessions.values() {
            let ready = self.user_ready(&session.username, &session.room);
            let mut entry = ListUserEntry::new()
                .with_position(0.0)
                .with_file(session.file.clone().unwrap_or_else(|| json!({})))
                .with_controller(self.user_is_room_controller(&session.username, &session.room));
            if let Some(ready) = ready {
                entry = entry.with_is_ready(ready);
            }
            if let Some(features) = &session.features {
                entry = entry.with_features(features.clone());
            }
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
