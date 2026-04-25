use super::*;

impl ServerRuntime {
    pub fn handle_line(
        &mut self,
        client_id: &str,
        json_line: &str,
    ) -> Result<Vec<String>, ServerRuntimeError> {
        let outbound_lines = self.handle_line_fanout(client_id, json_line)?;
        Ok(outbound_lines
            .into_iter()
            .filter(|line| line.client_id == client_id)
            .map(|line| line.line)
            .collect())
    }

    pub fn handle_line_fanout(
        &mut self,
        client_id: &str,
        json_line: &str,
    ) -> Result<Vec<DirectedOutboundLine>, ServerRuntimeError> {
        let message = decode_message_line(json_line)?;
        let outbound_messages = self.handle_protocol_message_fanout(client_id, message)?;
        outbound_messages
            .into_iter()
            .map(|message| {
                Ok(DirectedOutboundLine {
                    client_id: message.client_id,
                    line: encode_message_line(&message.message)?,
                })
            })
            .collect()
    }

    pub fn handle_transport_disconnect_fanout(
        &mut self,
        client_id: &str,
    ) -> Result<Vec<DirectedOutboundLine>, ServerRuntimeError> {
        let outbound_messages = self.timeout_disconnect_messages(client_id)?;
        outbound_messages
            .into_iter()
            .map(|message| {
                Ok(DirectedOutboundLine {
                    client_id: message.client_id,
                    line: encode_message_line(&message.message)?,
                })
            })
            .collect()
    }

    pub fn handle_line_fanout_with_transport_actions(
        &mut self,
        client_id: &str,
        json_line: &str,
    ) -> Result<ServerRuntimeDispatch, ServerRuntimeError> {
        let outbound_lines = self.handle_line_fanout(client_id, json_line)?;
        let transport_actions = self.drain_transport_actions();
        Ok(ServerRuntimeDispatch {
            outbound_lines,
            transport_actions,
        })
    }

    pub fn handle_protocol_message(
        &mut self,
        client_id: &str,
        message: ProtocolMessage,
    ) -> Result<Vec<ProtocolMessage>, ServerRuntimeError> {
        let outbound_messages = self.handle_protocol_message_fanout(client_id, message)?;
        Ok(outbound_messages
            .into_iter()
            .filter(|message| message.client_id == client_id)
            .map(|message| message.message)
            .collect())
    }

    pub fn handle_protocol_message_fanout(
        &mut self,
        client_id: &str,
        message: ProtocolMessage,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        match message {
            ProtocolMessage::Hello(payload) => self.handle_hello(client_id, payload.hello),
            ProtocolMessage::Set(payload) => self.handle_set(client_id, payload.set),
            ProtocolMessage::List(payload) => self.handle_list(client_id, payload.list),
            ProtocolMessage::State(payload) => self.handle_state(client_id, payload.state),
            ProtocolMessage::Tls(payload) => self.handle_tls(client_id, payload.tls),
            ProtocolMessage::Chat(payload) => self.handle_chat(client_id, payload.chat),
            ProtocolMessage::Error(_) => Ok(Vec::new()),
        }
    }

    pub(crate) fn handle_chat(
        &mut self,
        client_id: &str,
        chat: ChatPayload,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        if !self.chat_enabled {
            return Ok(Vec::new());
        }
        let session = self
            .sessions
            .get(client_id)
            .ok_or_else(|| ServerRuntimeError::MissingSession(client_id.to_owned()))?;
        let message = match chat {
            ChatPayload::Text(message) => message,
            ChatPayload::Message(message_payload) => message_payload.message,
        };
        let message = truncate_text_to_max_chars(&message, self.max_chat_message_length);
        let outbound_message = ProtocolMessage::chat_message(session.username.clone(), message);
        Ok(self
            .clients_in_room(&session.room)
            .into_iter()
            .map(|peer_client| DirectedProtocolMessage::new(&peer_client, outbound_message.clone()))
            .collect())
    }

    pub(crate) fn handle_tls(
        &mut self,
        client_id: &str,
        tls: TlsPayload,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        if !tls.start_tls.contains("send") {
            return Ok(Vec::new());
        }
        let should_start_tls = if !self.sessions.contains_key(client_id) && self.server_accepts_tls
        {
            self.refresh_tls_context_after_cert_rotation_if_needed();
            self.tls_context_available
        } else {
            false
        };
        if should_start_tls {
            self.pending_transport_actions
                .push(DirectedTransportAction::new(
                    client_id,
                    ServerTransportAction::StartTls,
                ));
        }
        let start_tls = if should_start_tls { "true" } else { "false" };
        Ok(vec![DirectedProtocolMessage::new(
            client_id,
            ProtocolMessage::start_tls(start_tls),
        )])
    }

    pub(crate) fn handle_hello(
        &mut self,
        client_id: &str,
        hello: HelloPayload,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let requested_username =
            truncate_text_to_max_chars(hello.username.trim(), self.max_username_length);
        let room_name = hello.room.name.trim();
        let version = hello.effective_version().trim();
        if requested_username.is_empty() || room_name.is_empty() || version.is_empty() {
            return Err(ServerRuntimeError::InvalidHello);
        }
        if let Some(required_password_token) = self.server_password_token.as_deref() {
            let Some(server_password_token) = hello_server_password_token(&hello) else {
                return Ok(vec![DirectedProtocolMessage::new(
                    client_id,
                    ProtocolMessage::error_message(LEGACY_SERVER_PASSWORD_REQUIRED_ERROR),
                )]);
            };
            if !server_password_token_matches_legacy_compatible(
                server_password_token,
                required_password_token,
            ) {
                return Ok(vec![DirectedProtocolMessage::new(
                    client_id,
                    ProtocolMessage::error_message(LEGACY_SERVER_WRONG_PASSWORD_ERROR),
                )]);
            }
        }

        let advertised_features = hello.features.clone();
        if let Some(previous_session) = self.remove_session_tracking(client_id) {
            self.cleanup_room_if_empty(&previous_session.room)?;
        }
        let username = self.find_free_username(&requested_username, Some(client_id));
        self.domain.join_room(&username, room_name);
        self.ensure_room_state(room_name);
        self.sessions.insert(
            client_id.to_owned(),
            ServerSession {
                username: username.to_owned(),
                room: room_name.to_owned(),
                version: version.to_owned(),
                features: advertised_features.clone(),
            },
        );
        self.client_state_counters
            .insert(client_id.to_owned(), ClientStateCounters::default());
        let now = self.current_time_seconds();
        self.client_last_state_update_at
            .insert(client_id.to_owned(), now);
        self.client_next_periodic_state_at
            .insert(client_id.to_owned(), now + SERVER_STATE_INTERVAL_SECONDS);

        let mut outbound = Vec::new();
        let joined_message = user_joined_message_with_metadata(
            &username,
            room_name,
            version,
            advertised_features.clone(),
        );
        for existing_client in self.clients_visible_on_join(room_name, client_id) {
            outbound.push(DirectedProtocolMessage::new(
                existing_client,
                joined_message.clone(),
            ));
        }

        let room_playlist = self.room_playlist_state(room_name);
        let room_playback = self.room_playback_state(room_name);
        let playlist_snapshot_message = playlist_snapshot_change_message(
            room_playlist.files.clone(),
            room_playback.set_by.as_deref(),
        );
        outbound.push(DirectedProtocolMessage::new(
            client_id,
            playlist_snapshot_message,
        ));
        if let Some(index) = room_playlist.index {
            outbound.push(DirectedProtocolMessage::new(
                client_id,
                playlist_snapshot_index_message(index, room_playback.set_by.as_deref()),
            ));
        }

        let base_motd = motd_for_client_version(version, self.motd_template.as_deref());
        let motd = persistent_rooms_notice_motd(
            base_motd,
            self.persistent_rooms_enabled,
            advertised_features.as_ref(),
        );
        let mut response = HelloPayload::new(username.clone(), room_name, version)
            .with_realversion(SERVER_REAL_VERSION)
            .with_features(server_feature_list(
                self.persistent_rooms_enabled,
                self.isolate_rooms,
                self.chat_enabled,
                self.readiness_enabled,
                self.max_chat_message_length,
                self.max_username_length,
            ));
        response
            .extra
            .insert("motd".to_owned(), Value::String(motd));
        outbound.push(DirectedProtocolMessage::new(
            client_id,
            ProtocolMessage::hello(response),
        ));

        if self.persistent_rooms_enabled {
            self.enqueue_list_snapshots_for_clients(
                &mut outbound,
                self.clients_receiving_to_gui_only_list_updates(),
            );
        }

        if self.clients_in_room(room_name).len() > 1 {
            let idle_state_message = room_idle_state_message(self.current_time_seconds());
            for room_client in self.clients_in_room(room_name) {
                outbound.push(DirectedProtocolMessage::new(
                    room_client,
                    idle_state_message.clone(),
                ));
            }
        }

        Ok(outbound)
    }

    pub(crate) fn handle_list(
        &self,
        client_id: &str,
        list: ListPayload,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        if !self.sessions.contains_key(client_id) {
            return Err(ServerRuntimeError::MissingSession(client_id.to_owned()));
        }
        match list {
            ListPayload::Request(_) => {
                let rooms = self.list_rooms_snapshot_for_client(client_id);
                Ok(vec![DirectedProtocolMessage::new(
                    client_id,
                    ProtocolMessage::list(ListPayload::rooms(rooms)),
                )])
            }
            ListPayload::Rooms(_) => Ok(Vec::new()),
        }
    }

    pub(crate) fn handle_set(
        &mut self,
        client_id: &str,
        set: SetPayload,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let mut session = self
            .sessions
            .get(client_id)
            .cloned()
            .ok_or_else(|| ServerRuntimeError::MissingSession(client_id.to_owned()))?;

        let mut outbound_messages = Vec::new();

        if let Some(room_ref) = set.room
            && session.room != room_ref.name
        {
            let previous_room = session.room.clone();
            self.domain.leave_room(&session.username, &previous_room)?;
            self.remove_room_controller(&session.username, &previous_room);
            self.domain.join_room(&session.username, &room_ref.name);
            self.ensure_room_state(&room_ref.name);
            session.room = room_ref.name.clone();
            self.sessions.insert(client_id.to_owned(), session.clone());
            self.client_next_periodic_state_at.insert(
                client_id.to_owned(),
                self.current_time_seconds() + SERVER_STATE_INTERVAL_SECONDS,
            );
            self.cleanup_room_if_empty(&previous_room)?;

            outbound_messages.push(DirectedProtocolMessage::new(
                client_id,
                room_idle_state_message(self.current_time_seconds()),
            ));
            outbound_messages.push(DirectedProtocolMessage::new(
                client_id,
                self.forced_state_sync_message_for_client(client_id, 0.0, true, true, None),
            ));

            let room_update_message = user_room_update_message(&session.username, &session.room);
            for peer_client in
                self.room_switch_visibility_recipients(client_id, &previous_room, &session.room)
            {
                outbound_messages.push(DirectedProtocolMessage::new(
                    peer_client,
                    room_update_message.clone(),
                ));
            }
            if self.isolate_rooms {
                let left_message = user_event_message(
                    &session.username,
                    &previous_room,
                    json!({
                        "left": true,
                    }),
                );
                for peer_client in self.clients_in_room(&previous_room) {
                    outbound_messages.push(DirectedProtocolMessage::new(
                        peer_client,
                        left_message.clone(),
                    ));
                }
            }

            let room_playlist = self.room_playlist_state(&session.room);
            let room_playback = self.room_playback_state(&session.room);
            let playlist_snapshot_message = playlist_snapshot_change_message(
                room_playlist.files.clone(),
                room_playback.set_by.as_deref(),
            );
            outbound_messages.push(DirectedProtocolMessage::new(
                client_id,
                playlist_snapshot_message,
            ));
            if let Some(index) = room_playlist.index {
                outbound_messages.push(DirectedProtocolMessage::new(
                    client_id,
                    playlist_snapshot_index_message(index, room_playback.set_by.as_deref()),
                ));
            }

            if self.persistent_rooms_enabled {
                self.enqueue_list_snapshots_for_clients(
                    &mut outbound_messages,
                    self.clients_receiving_to_gui_only_list_updates(),
                );
            }
        }

        if let Some(mut playlist_change) = set.playlist_change {
            self.ensure_room_state(&session.room);
            if self.user_can_control_playlist(&session.username, &session.room) {
                let new_files = playlist_change.files.clone();
                self.room_playlist_state_mut(&session.room).files = new_files;
                self.persist_room_if_needed(&session.room)?;
                playlist_change.user = Some(session.username.clone());
                let playlist_message =
                    ProtocolMessage::set(SetPayload::new().with_playlist_change(playlist_change));
                for peer_client in self.clients_in_room(&session.room) {
                    outbound_messages.push(DirectedProtocolMessage::new(
                        peer_client,
                        playlist_message.clone(),
                    ));
                }
            } else {
                let room_state = self.room_playlist_state(&session.room);
                let correction_change = ProtocolMessage::set(
                    SetPayload::new().with_playlist_change(
                        PlaylistChangePayload::new(room_state.files.iter().cloned())
                            .with_user(session.room.clone()),
                    ),
                );
                outbound_messages.push(DirectedProtocolMessage::new(client_id, correction_change));
            }
        }

        if let Some(mut playlist_index) = set.playlist_index {
            self.ensure_room_state(&session.room);
            if self.user_can_control_playlist(&session.username, &session.room) {
                self.room_playlist_state_mut(&session.room).index = Some(playlist_index.index);
                self.persist_room_if_needed(&session.room)?;
                playlist_index.user = Some(session.username.clone());
                let playlist_message =
                    ProtocolMessage::set(SetPayload::new().with_playlist_index(playlist_index));
                for peer_client in self.clients_in_room(&session.room) {
                    outbound_messages.push(DirectedProtocolMessage::new(
                        peer_client,
                        playlist_message.clone(),
                    ));
                }
            }
        }

        if let Some(controller_auth) = set.controller_auth {
            let room_to_check = controller_auth.room.unwrap_or_else(|| session.room.clone());
            let auth_password = controller_auth.password.unwrap_or_default();
            match self
                .room_password_provider
                .check(&room_to_check, &auth_password)
            {
                Ok(success) => {
                    if success {
                        self.add_room_controller(&session.username, &session.room);
                    }
                    let auth_message =
                        controller_auth_status_message(&session.username, &session.room, success);
                    let recipients = if self.isolate_rooms {
                        self.clients_in_room(&session.room)
                    } else {
                        self.clients_all()
                    };
                    for peer_client in recipients {
                        outbound_messages.push(DirectedProtocolMessage::new(
                            peer_client,
                            auth_message.clone(),
                        ));
                    }
                }
                Err(RoomPasswordCheckError::NotControlledRoom) => {
                    let new_room_name = self
                        .room_password_provider
                        .controlled_room_name_for(&room_to_check, &auth_password);
                    let new_room_message =
                        new_controlled_room_message(&new_room_name, &auth_password);
                    outbound_messages
                        .push(DirectedProtocolMessage::new(client_id, new_room_message));
                }
                Err(RoomPasswordCheckError::InvalidPassword) => {
                    let auth_message =
                        controller_auth_status_message(&session.username, &session.room, false);
                    for peer_client in self.clients_in_room(&session.room) {
                        outbound_messages.push(DirectedProtocolMessage::new(
                            peer_client,
                            auth_message.clone(),
                        ));
                    }
                }
            }
        }

        if self.readiness_enabled
            && let Some(ready) = set.ready
        {
            self.domain
                .set_ready(&session.username, &session.room, ready.is_ready)?;
            let ready_message = ready_update_message(
                &session.username,
                ready.is_ready,
                ready.manually_initiated.unwrap_or(true),
                ready.set_by.as_deref(),
            );
            for peer_client in self.clients_in_room(&session.room) {
                outbound_messages.push(DirectedProtocolMessage::new(
                    peer_client,
                    ready_message.clone(),
                ));
            }
        }

        Ok(outbound_messages)
    }

    pub(crate) fn handle_state(
        &mut self,
        client_id: &str,
        state: StatePayload,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let session = self
            .sessions
            .get(client_id)
            .cloned()
            .ok_or_else(|| ServerRuntimeError::MissingSession(client_id.to_owned()))?;

        if let Some(ignore) = state.ignoring_on_the_fly.as_ref() {
            if let Some(server_ignoring_counter) = ignore.server {
                self.acknowledge_server_ignoring_counter(client_id, server_ignoring_counter);
            }
            if let Some(client_ignoring_counter) = ignore.client {
                self.queue_client_ignoring_counter(client_id, client_ignoring_counter);
            }
        }
        if let Some(ping) = state.ping.as_ref() {
            if let Some(client_latency_calculation) = ping.client_latency_calculation {
                self.queue_client_latency_calculation(client_id, client_latency_calculation);
            }
            self.ingest_client_ping_metrics(client_id, ping.latency_calculation, ping.client_rtt);
        }
        if self.server_ignoring_counter(client_id) > 0 {
            return Ok(Vec::new());
        }
        self.record_client_state_update_now(client_id);

        let Some(playstate) = state.playstate else {
            return Ok(Vec::new());
        };

        self.ensure_room_state(&session.room);
        let room_state_before = self.room_playback_state(&session.room);
        let can_control_room = self.user_can_control_playlist(&session.username, &session.room);
        let do_seek = playstate.do_seek.unwrap_or(false);
        let forward_delay_seconds = self.forward_delay_seconds(client_id);
        let pause_changed = playstate
            .paused
            .is_some_and(|paused| paused != room_state_before.paused);

        if can_control_room {
            let room_state = self.room_playback_state_mut(&session.room);
            if let Some(paused) = playstate.paused {
                room_state.paused = paused;
            }
            if let Some(mut position) = playstate.position {
                if !playstate.paused.unwrap_or(false) {
                    position += forward_delay_seconds;
                }
                room_state.position = position;
            }
            room_state.set_by = Some(session.username.clone());
            self.persist_room_if_needed(&session.room)?;
        }

        if !do_seek && !pause_changed {
            return Ok(Vec::new());
        }

        if can_control_room {
            let room_state = self.room_playback_state(&session.room);
            let mut outbound_messages = Vec::new();
            for peer_client in self.clients_in_room(&session.room) {
                let state_message = self.forced_state_sync_message_for_client(
                    &peer_client,
                    room_state.position,
                    room_state.paused,
                    do_seek,
                    Some(&session.username),
                );
                outbound_messages.push(DirectedProtocolMessage::new(peer_client, state_message));
            }
            return Ok(outbound_messages);
        }

        let watcher_pause_state = playstate.paused.unwrap_or(room_state_before.paused);
        Ok(vec![
            DirectedProtocolMessage::new(
                client_id,
                self.forced_state_sync_message_for_client(
                    client_id,
                    room_state_before.position,
                    watcher_pause_state,
                    false,
                    Some(&session.username),
                ),
            ),
            DirectedProtocolMessage::new(
                client_id,
                self.forced_state_sync_message_for_client(
                    client_id,
                    room_state_before.position,
                    room_state_before.paused,
                    true,
                    room_state_before.set_by.as_deref(),
                ),
            ),
        ])
    }
}
