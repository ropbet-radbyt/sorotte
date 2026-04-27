use super::*;

fn known_protocol_command(command: &str) -> bool {
    matches!(
        command,
        "Hello" | "Set" | "List" | "State" | "Chat" | "Error" | "TLS"
    )
}

fn top_level_protocol_commands(json_line: &str) -> Option<Vec<String>> {
    let value = decode_line(json_line).ok()?;
    let object = value.as_object()?;
    Some(object.keys().cloned().collect())
}

fn protocol_drop_error_message(error: &ServerRuntimeError, json_line: &str) -> Option<String> {
    match error {
        ServerRuntimeError::Protocol(ProtocolError::InvalidJson(_)) => {
            let Some(commands) = top_level_protocol_commands(json_line) else {
                return Some(format!("{LEGACY_SERVER_NOT_JSON_ERROR_PREFIX} {json_line}"));
            };
            if commands.iter().any(|command| command == "Hello") {
                return Some(LEGACY_SERVER_HELLO_ERROR.to_owned());
            }
            if commands
                .iter()
                .any(|command| known_protocol_command(command))
            {
                return Some(format!("{LEGACY_SERVER_NOT_JSON_ERROR_PREFIX} {json_line}"));
            }
            let command_payload = decode_line(json_line)
                .ok()
                .and_then(|value| {
                    value
                        .as_object()
                        .and_then(|object| object.values().next().cloned())
                })
                .unwrap_or(Value::Null);
            Some(unknown_command_error_message(&command_payload))
        }
        ServerRuntimeError::MissingSession(_) => Some(LEGACY_SERVER_NOT_KNOWN_ERROR.to_owned()),
        ServerRuntimeError::InvalidHello => Some(LEGACY_SERVER_HELLO_ERROR.to_owned()),
        _ => None,
    }
}

fn unknown_command_error_message(payload: &Value) -> String {
    format!("{LEGACY_SERVER_UNKNOWN_COMMAND_ERROR_PREFIX} {payload}")
}

struct LineFanoutFailure {
    outbound_messages: Vec<DirectedProtocolMessage>,
    error: ServerRuntimeError,
    protocol_error_message: Option<String>,
}

impl LineFanoutFailure {
    fn new(outbound_messages: Vec<DirectedProtocolMessage>, error: ServerRuntimeError) -> Self {
        Self {
            outbound_messages,
            error,
            protocol_error_message: None,
        }
    }

    fn with_protocol_error_message(
        outbound_messages: Vec<DirectedProtocolMessage>,
        error: ServerRuntimeError,
        protocol_error_message: String,
    ) -> Self {
        Self {
            outbound_messages,
            error,
            protocol_error_message: Some(protocol_error_message),
        }
    }
}

impl ServerRuntime {
    fn encode_directed_protocol_messages(
        messages: Vec<DirectedProtocolMessage>,
    ) -> Result<Vec<DirectedOutboundLine>, ServerRuntimeError> {
        messages
            .into_iter()
            .map(|message| {
                Ok(DirectedOutboundLine {
                    client_id: message.client_id,
                    line: encode_message_line(&message.message)?,
                })
            })
            .collect()
    }

    fn handle_line_fanout_messages(
        &mut self,
        client_id: &str,
        json_line: &str,
    ) -> Result<Vec<DirectedProtocolMessage>, Box<LineFanoutFailure>> {
        let items = decode_message_line_items(json_line)
            .map_err(|error| Box::new(LineFanoutFailure::new(Vec::new(), error.into())))?;
        let mut outbound_messages = Vec::new();

        for item in items {
            if item
                .command
                .as_deref()
                .is_some_and(|command| !known_protocol_command(command))
            {
                let protocol_error_message = unknown_command_error_message(&item.payload);
                let error = match item.message {
                    Ok(_) => ServerRuntimeError::Protocol(ProtocolError::ServerError {
                        message: protocol_error_message.clone(),
                    }),
                    Err(error) => error.into(),
                };
                return Err(Box::new(LineFanoutFailure::with_protocol_error_message(
                    outbound_messages,
                    error,
                    protocol_error_message,
                )));
            }

            let message = match item.message {
                Ok(message) => message,
                Err(error) => {
                    return Err(Box::new(LineFanoutFailure::new(
                        outbound_messages,
                        error.into(),
                    )));
                }
            };
            match self.handle_protocol_message_fanout(client_id, message) {
                Ok(messages) => outbound_messages.extend(messages),
                Err(error) => {
                    return Err(Box::new(LineFanoutFailure::new(outbound_messages, error)));
                }
            }
        }

        Ok(outbound_messages)
    }

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
        let outbound_messages = self
            .handle_line_fanout_messages(client_id, json_line)
            .map_err(|failure| {
                let LineFanoutFailure { error, .. } = *failure;
                error
            })?;
        Self::encode_directed_protocol_messages(outbound_messages)
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
        let outbound_lines = match self.handle_line_fanout_messages(client_id, json_line) {
            Ok(outbound_messages) => Self::encode_directed_protocol_messages(outbound_messages)?,
            Err(failure) => {
                let LineFanoutFailure {
                    outbound_messages,
                    error,
                    protocol_error_message,
                } = *failure;
                let mut outbound_lines =
                    Self::encode_directed_protocol_messages(outbound_messages)?;
                let Some(error_message) = protocol_error_message
                    .or_else(|| protocol_drop_error_message(&error, json_line))
                else {
                    return Err(error);
                };
                let error_line =
                    encode_message_line(&ProtocolMessage::error_message(error_message))?;
                outbound_lines.push(DirectedOutboundLine {
                    client_id: client_id.to_owned(),
                    line: error_line,
                });
                return Ok(ServerRuntimeDispatch {
                    outbound_lines,
                    transport_actions: vec![DirectedTransportAction::new(
                        client_id,
                        ServerTransportAction::Close,
                    )],
                });
            }
        };
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
        let (username, room_name) = {
            let session = self
                .sessions
                .get(client_id)
                .ok_or_else(|| ServerRuntimeError::MissingSession(client_id.to_owned()))?;
            (session.username.clone(), session.room.clone())
        };
        let message = match chat {
            ChatPayload::Text(message) => message,
            ChatPayload::Message(message_payload) => message_payload.message,
        };
        let message = truncate_text_to_max_chars(&message, self.max_chat_message_length);
        let outbound_message = ProtocolMessage::chat_message(username, message);
        Ok(self
            .chat_clients_in_room(&room_name)
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
        let room_name =
            truncate_text_to_max_chars(hello.room.name.trim(), DEFAULT_MAX_ROOM_NAME_LENGTH);
        let version = hello.effective_version().trim();
        if requested_username.is_empty() || room_name.is_empty() || version.is_empty() {
            return Err(ServerRuntimeError::InvalidHello);
        }
        if let Some(required_password_token) = self.server_password_token.as_deref() {
            let Some(server_password_token) = hello_server_password_token(&hello) else {
                self.pending_transport_actions
                    .push(DirectedTransportAction::new(
                        client_id,
                        ServerTransportAction::Close,
                    ));
                return Ok(vec![DirectedProtocolMessage::new(
                    client_id,
                    ProtocolMessage::error_message(LEGACY_SERVER_PASSWORD_REQUIRED_ERROR),
                )]);
            };
            if !server_password_token_matches_legacy_compatible(
                server_password_token,
                required_password_token,
            ) {
                self.pending_transport_actions
                    .push(DirectedTransportAction::new(
                        client_id,
                        ServerTransportAction::Close,
                    ));
                return Ok(vec![DirectedProtocolMessage::new(
                    client_id,
                    ProtocolMessage::error_message(LEGACY_SERVER_WRONG_PASSWORD_ERROR),
                )]);
            }
        }

        let advertised_features =
            legacy_client_features_for_version(version, hello.features.clone());
        if let Some(previous_session) = self.remove_session_tracking(client_id) {
            self.cleanup_room_if_empty(&previous_session.room)?;
        }
        let username = self.find_free_username(&requested_username, Some(client_id));
        let room_had_clients_before_join = !self.clients_in_room(&room_name).is_empty();
        let room_should_seed_joiner_position = room_had_clients_before_join
            || self.room_is_persistent(&room_name)
            || self.room_is_permanent(&room_name);
        self.ensure_room_state(&room_name);
        let now = self.current_time_seconds();
        let join_room_playback = self.refresh_room_playback_state_from_clients_at(&room_name, now);
        self.domain.join_room(&username, &room_name);
        self.sessions.insert(
            client_id.to_owned(),
            ServerSession {
                username: username.to_owned(),
                room: room_name.clone(),
                version: version.to_owned(),
                features: Some(advertised_features.clone()),
                file: None,
            },
        );
        self.assign_room_join_order(client_id);
        self.seed_client_playback_state(
            client_id,
            room_should_seed_joiner_position.then_some(join_room_playback.position),
            now,
        );
        self.client_state_counters
            .insert(client_id.to_owned(), ClientStateCounters::default());
        self.client_last_state_update_at
            .insert(client_id.to_owned(), now);
        self.client_next_periodic_state_at.insert(
            client_id.to_owned(),
            now + INITIAL_SERVER_STATE_DELAY_SECONDS,
        );

        let mut outbound = Vec::new();
        let joined_message = user_joined_message_with_metadata(
            &username,
            &room_name,
            version,
            Some(advertised_features.clone()),
        );
        for existing_client in self.clients_visible_on_join(&room_name, client_id) {
            outbound.push(DirectedProtocolMessage::new(
                existing_client,
                joined_message.clone(),
            ));
        }
        let ready_message = ready_update_message(&username, None, false, None);
        for room_client in self.clients_in_room(&room_name) {
            outbound.push(DirectedProtocolMessage::new(
                room_client,
                ready_message.clone(),
            ));
        }
        let room_playlist = self.room_playlist_state(&room_name);
        let playlist_snapshot_message = playlist_snapshot_change_message(
            room_playlist.files.clone(),
            join_room_playback.set_by.as_deref(),
        );
        outbound.push(DirectedProtocolMessage::new(
            client_id,
            playlist_snapshot_message,
        ));
        outbound.push(DirectedProtocolMessage::new(
            client_id,
            playlist_snapshot_index_message(
                room_playlist.index,
                join_room_playback.set_by.as_deref(),
            ),
        ));

        let base_motd = motd_for_client_version(version, self.motd_template.as_deref());
        let motd = persistent_rooms_notice_motd(
            base_motd,
            self.persistent_rooms_enabled,
            Some(&advertised_features),
        );
        let mut response = HelloPayload::new(username.clone(), room_name.clone(), version)
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
                self.clients_receiving_to_gui_only_list_updates(Some(&room_name)),
            );
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
        mut set: SetPayload,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let mut session = self
            .sessions
            .get(client_id)
            .cloned()
            .ok_or_else(|| ServerRuntimeError::MissingSession(client_id.to_owned()))?;

        let mut outbound_messages = Vec::new();

        let mut commands = if set.command_order.is_empty() {
            Vec::new()
        } else {
            set.command_order.clone()
        };
        for fallback_command in [
            "room",
            "file",
            "controllerAuth",
            "ready",
            "playlistChange",
            "playlistIndex",
            "features",
        ] {
            if !commands.iter().any(|command| command == fallback_command) {
                commands.push(fallback_command.to_owned());
            }
        }

        for command in commands {
            match command.as_str() {
                "room" => {
                    let Some(room_ref) = set.room.take() else {
                        continue;
                    };
                    let new_room_name =
                        truncate_text_to_max_chars(&room_ref.name, DEFAULT_MAX_ROOM_NAME_LENGTH);
                    if session.room == new_room_name {
                        continue;
                    }

                    let previous_room = session.room.clone();
                    let previous_ready = self.stored_user_ready(&session.username, &previous_room);
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
                    let new_room_had_clients_before_join =
                        !self.clients_in_room(&new_room_name).is_empty();
                    let new_room_should_seed_position = new_room_had_clients_before_join
                        || self.room_is_persistent(&new_room_name)
                        || self.room_is_permanent(&new_room_name);
                    self.domain.leave_room(&session.username, &previous_room)?;
                    self.remove_room_controller(&session.username, &previous_room);
                    self.ensure_room_state(&new_room_name);
                    let now_seconds = self.current_time_seconds();
                    let room_playback = self
                        .refresh_room_playback_state_from_clients_at(&new_room_name, now_seconds);
                    self.domain.join_room_with_ready(
                        &session.username,
                        &new_room_name,
                        previous_ready,
                    );
                    self.assign_room_join_order(client_id);
                    self.seed_client_playback_state(
                        client_id,
                        new_room_should_seed_position.then_some(room_playback.position),
                        now_seconds,
                    );
                    session.room = new_room_name;
                    self.sessions.insert(client_id.to_owned(), session.clone());
                    self.client_next_periodic_state_at.insert(
                        client_id.to_owned(),
                        now_seconds + SERVER_STATE_INTERVAL_SECONDS,
                    );
                    self.cleanup_room_if_empty(&previous_room)?;

                    let room_set_by = room_playback.set_by.clone();
                    outbound_messages.push(DirectedProtocolMessage::new(
                        client_id,
                        self.forced_state_sync_message_for_client(
                            client_id,
                            room_playback.position,
                            room_playback.paused,
                            true,
                            room_set_by.as_deref(),
                        ),
                    ));

                    if self.isolate_rooms
                        && let Some(file) = session.file.clone().filter(legacy_json_value_truthy)
                    {
                        let file_message =
                            user_file_update_message(&session.username, &session.room, file);
                        for peer_client in self.clients_in_room(&session.room) {
                            outbound_messages.push(DirectedProtocolMessage::new(
                                peer_client,
                                file_message.clone(),
                            ));
                        }
                    }

                    let room_update_message =
                        user_room_update_message(&session.username, &session.room);
                    for peer_client in self.room_switch_visibility_recipients(
                        client_id,
                        &previous_room,
                        &session.room,
                    ) {
                        outbound_messages.push(DirectedProtocolMessage::new(
                            peer_client,
                            room_update_message.clone(),
                        ));
                    }
                    let ready_message = ready_update_message(
                        &session.username,
                        if self.readiness_enabled {
                            previous_ready
                        } else {
                            None
                        },
                        false,
                        None,
                    );
                    for peer_client in self.clients_in_room(&session.room) {
                        outbound_messages.push(DirectedProtocolMessage::new(
                            peer_client,
                            ready_message.clone(),
                        ));
                    }

                    let room_playlist = self.room_playlist_state(&session.room);
                    let playlist_snapshot_message = playlist_snapshot_change_message(
                        room_playlist.files.clone(),
                        room_playback.set_by.as_deref(),
                    );
                    outbound_messages.push(DirectedProtocolMessage::new(
                        client_id,
                        playlist_snapshot_message,
                    ));
                    outbound_messages.push(DirectedProtocolMessage::new(
                        client_id,
                        playlist_snapshot_index_message(
                            room_playlist.index,
                            room_playback.set_by.as_deref(),
                        ),
                    ));

                    if self.persistent_rooms_enabled {
                        self.enqueue_list_snapshots_for_clients(
                            &mut outbound_messages,
                            self.clients_receiving_to_gui_only_list_updates(Some(&session.room)),
                        );
                    }
                }
                "file" => {
                    let Some(file_payload) = set.file.take() else {
                        continue;
                    };
                    let mut file =
                        serde_json::to_value(file_payload).map_err(ProtocolError::from)?;
                    truncate_file_payload_name(&mut file, DEFAULT_MAX_FILENAME_LENGTH);
                    session.file = Some(file.clone());
                    self.sessions.insert(client_id.to_owned(), session.clone());

                    if legacy_json_value_truthy(&file) {
                        let file_message =
                            user_file_update_message(&session.username, &session.room, file);
                        let recipients = if self.isolate_rooms {
                            self.clients_in_room(&session.room)
                        } else {
                            self.clients_all()
                        };
                        for peer_client in recipients {
                            outbound_messages.push(DirectedProtocolMessage::new(
                                peer_client,
                                file_message.clone(),
                            ));
                        }
                    }
                }
                "controllerAuth" => {
                    let Some(controller_auth) = set.controller_auth.take() else {
                        continue;
                    };
                    let room_to_check =
                        controller_auth.room.unwrap_or_else(|| session.room.clone());
                    let auth_password = controller_auth.password.unwrap_or_default();
                    match self
                        .room_password_provider
                        .check(&room_to_check, &auth_password)
                    {
                        Ok(success) => {
                            if success {
                                self.add_room_controller(&session.username, &session.room);
                            }
                            let auth_message = controller_auth_status_message(
                                &session.username,
                                &session.room,
                                success,
                            );
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
                            let auth_message = controller_auth_status_message(
                                &session.username,
                                &session.room,
                                false,
                            );
                            for peer_client in self.clients_in_room(&session.room) {
                                outbound_messages.push(DirectedProtocolMessage::new(
                                    peer_client,
                                    auth_message.clone(),
                                ));
                            }
                        }
                    }
                }
                "ready" => {
                    let Some(ready) = set.ready.take() else {
                        continue;
                    };
                    let manually_initiated = ready.manually_initiated.unwrap_or(false);
                    let Some(is_ready) = ready.is_ready else {
                        continue;
                    };
                    let outbound_ready = if self.readiness_enabled {
                        Some(is_ready)
                    } else {
                        None
                    };
                    let requested_username = ready.username.as_deref().unwrap_or(&session.username);
                    if requested_username != session.username {
                        if self.user_can_control_playlist(&session.username, &session.room)
                            && self
                                .domain
                                .set_ready(requested_username, &session.room, is_ready)
                                .is_ok()
                        {
                            let ready_message = ready_update_message(
                                requested_username,
                                outbound_ready,
                                manually_initiated,
                                Some(&session.username),
                            );
                            for peer_client in self.clients_in_room(&session.room) {
                                outbound_messages.push(DirectedProtocolMessage::new(
                                    peer_client,
                                    ready_message.clone(),
                                ));
                            }
                            let chat_message = readiness_legacy_chat_message(
                                &session.username,
                                requested_username,
                                is_ready,
                            );
                            for peer_client in
                                self.legacy_readiness_chat_clients_in_room(&session.room)
                            {
                                outbound_messages.push(DirectedProtocolMessage::new(
                                    peer_client,
                                    chat_message.clone(),
                                ));
                            }
                        }
                    } else {
                        self.domain
                            .set_ready(&session.username, &session.room, is_ready)?;
                        let ready_message = ready_update_message(
                            &session.username,
                            outbound_ready,
                            manually_initiated,
                            ready.set_by.as_deref(),
                        );
                        for peer_client in self.clients_in_room(&session.room) {
                            outbound_messages.push(DirectedProtocolMessage::new(
                                peer_client,
                                ready_message.clone(),
                            ));
                        }
                    }
                }
                "playlistChange" => {
                    let Some(mut playlist_change) = set.playlist_change.take() else {
                        continue;
                    };
                    self.ensure_room_state(&session.room);
                    if self.user_can_control_playlist(&session.username, &session.room)
                        && playlist_is_valid(&playlist_change.files)
                    {
                        let new_files = playlist_change.files.clone();
                        self.room_playlist_state_mut(&session.room).files = new_files;
                        self.persist_room_if_needed(&session.room)?;
                        playlist_change.user = Some(session.username.clone());
                        let playlist_message = ProtocolMessage::set(
                            SetPayload::new().with_playlist_change(playlist_change),
                        );
                        for peer_client in self.clients_in_room(&session.room) {
                            outbound_messages.push(DirectedProtocolMessage::new(
                                peer_client,
                                playlist_message.clone(),
                            ));
                        }
                    } else {
                        let room_state = self.room_playlist_state(&session.room);
                        outbound_messages.push(DirectedProtocolMessage::new(
                            client_id,
                            playlist_snapshot_change_message(
                                room_state.files.clone(),
                                Some(&session.room),
                            ),
                        ));
                        outbound_messages.push(DirectedProtocolMessage::new(
                            client_id,
                            playlist_snapshot_index_message(room_state.index, Some(&session.room)),
                        ));
                    }
                }
                "playlistIndex" => {
                    let Some(mut playlist_index) = set.playlist_index.take() else {
                        continue;
                    };
                    self.ensure_room_state(&session.room);
                    if self.user_can_control_playlist(&session.username, &session.room) {
                        self.room_playlist_state_mut(&session.room).index =
                            playlist_index.index_value();
                        self.persist_room_if_needed(&session.room)?;
                        playlist_index.user = Some(session.username.clone());
                        let playlist_message = ProtocolMessage::set(
                            SetPayload::new().with_playlist_index(playlist_index),
                        );
                        for peer_client in self.clients_in_room(&session.room) {
                            outbound_messages.push(DirectedProtocolMessage::new(
                                peer_client,
                                playlist_message.clone(),
                            ));
                        }
                    } else {
                        let room_state = self.room_playlist_state(&session.room);
                        outbound_messages.push(DirectedProtocolMessage::new(
                            client_id,
                            playlist_snapshot_index_message(room_state.index, Some(&session.room)),
                        ));
                    }
                }
                "features" => {
                    let Some(features) = set.features.take() else {
                        continue;
                    };
                    session.features = Some(features);
                    self.sessions.insert(client_id.to_owned(), session.clone());
                }
                _ => {}
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
        let now_seconds = self.current_time_seconds();
        let room_state_before = self.room_playback_state_at(&session.room, now_seconds);
        let can_control_room = self.user_can_control_playlist(&session.username, &session.room);
        let do_seek = playstate.do_seek;
        let forward_delay_seconds = self.forward_delay_seconds(client_id);
        let pause_changed = playstate
            .paused
            .is_some_and(|paused| paused != room_state_before.paused);
        let sample_paused = playstate.paused.unwrap_or(room_state_before.paused);
        let playback_sample_position = playstate.position.map(|mut position| {
            if !sample_paused {
                position += forward_delay_seconds;
            }
            position
        });
        self.record_client_playback_state_sample(client_id, playback_sample_position, now_seconds);

        if !do_seek.unwrap_or(false) && !pause_changed {
            return Ok(Vec::new());
        }

        if can_control_room {
            let watcher_position = self
                .client_playback_states
                .get(client_id)
                .and_then(|state| state.position_at(sample_paused, now_seconds))
                .unwrap_or(room_state_before.position);
            {
                let room_state = self.room_playback_state_mut(&session.room);
                room_state.position = watcher_position;
                room_state.paused = sample_paused;
                room_state.updated_at_seconds = now_seconds;
                room_state.set_by = Some(session.username.clone());
            }
            self.seed_room_client_playback_states(&session.room, watcher_position, now_seconds);
            self.persist_room_if_needed(&session.room)?;
            let room_state = self.room_playback_state_at(&session.room, now_seconds);
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
