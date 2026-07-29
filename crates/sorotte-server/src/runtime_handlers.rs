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
                    delivery: ServerOutboundDelivery::Reliable,
                })
            })
            .collect()
    }

    fn handle_line_fanout_messages(
        &mut self,
        client_id: &str,
        json_line: &str,
    ) -> Result<Vec<DirectedProtocolMessage>, Box<LineFanoutFailure>> {
        self.handle_line_fanout_messages_for_peer(client_id, json_line, None)
    }

    fn handle_line_fanout_messages_for_peer(
        &mut self,
        client_id: &str,
        json_line: &str,
        peer_ip: Option<&str>,
    ) -> Result<Vec<DirectedProtocolMessage>, Box<LineFanoutFailure>> {
        // A recovered application operation transfers authority to a newer
        // connection. Reject the superseded transport before decoding so a
        // repeated Hello, a batched command, or malformed input cannot pass
        // through a command-specific path or replace its session while the
        // network close is still in flight.
        if self.reject_fenced_playback_barrier_transport(client_id) {
            return Ok(Vec::new());
        }
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
            match self.handle_protocol_message_fanout_for_peer(client_id, message, peer_ip) {
                Ok(messages) => {
                    outbound_messages.extend(messages);
                    // STARTTLS is a hard transport boundary. Once accepted,
                    // no remaining item from this plaintext line may execute;
                    // the client must resend application messages after the
                    // socket has completed its TLS upgrade.
                    if self.pending_transport_actions.iter().any(|action| {
                        action.client_id == client_id
                            && action.action == ServerTransportAction::StartTls
                    }) {
                        break;
                    }
                }
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
                    delivery: ServerOutboundDelivery::Reliable,
                })
            })
            .collect()
    }

    pub fn handle_line_fanout_with_transport_actions(
        &mut self,
        client_id: &str,
        json_line: &str,
    ) -> Result<ServerRuntimeDispatch, ServerRuntimeError> {
        self.handle_line_fanout_with_transport_actions_for_peer(client_id, json_line, None)
    }

    pub fn handle_line_fanout_with_transport_actions_for_peer(
        &mut self,
        client_id: &str,
        json_line: &str,
        peer_ip: Option<&str>,
    ) -> Result<ServerRuntimeDispatch, ServerRuntimeError> {
        let outbound_lines = match self
            .handle_line_fanout_messages_for_peer(client_id, json_line, peer_ip)
        {
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
                    delivery: ServerOutboundDelivery::Reliable,
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
        self.handle_protocol_message_fanout_for_peer(client_id, message, None)
    }

    pub fn handle_protocol_message_fanout_for_peer(
        &mut self,
        client_id: &str,
        message: ProtocolMessage,
        peer_ip: Option<&str>,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        // The decoded-message API is also public, so enforce the same
        // transport-wide revocation boundary here rather than relying only on
        // the JSON-line entry point.
        if self.reject_fenced_playback_barrier_transport(client_id) {
            return Ok(Vec::new());
        }
        let normalized = normalize_server_protocol_message(message);
        self.pending_compatibility_fallbacks
            .extend(normalized.fallbacks);
        match normalized.command {
            ServerInboundCommand::Hello(hello) => {
                self.handle_hello_for_peer(client_id, hello, peer_ip)
            }
            ServerInboundCommand::Set(commands) => self.handle_set(client_id, commands),
            ServerInboundCommand::ListRequest => self.handle_list(client_id),
            ServerInboundCommand::State(state) => self.handle_state(client_id, state),
            ServerInboundCommand::Tls(start_tls) => self.handle_tls(client_id, start_tls),
            ServerInboundCommand::Chat(message) => self.handle_chat(client_id, message),
            ServerInboundCommand::Ignore => Ok(Vec::new()),
        }
    }

    pub(crate) fn handle_chat(
        &mut self,
        client_id: &str,
        message: String,
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
        start_tls_request: String,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        if !start_tls_request.contains("send") {
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

    pub(crate) fn handle_hello_for_peer(
        &mut self,
        client_id: &str,
        hello: ServerHelloCommand,
        peer_ip: Option<&str>,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let requested_username =
            truncate_text_to_max_chars(hello.username.trim(), self.max_username_length);
        let room_name = truncate_text_to_max_chars(hello.room.trim(), DEFAULT_MAX_ROOM_NAME_LENGTH);
        let version = hello.version.trim();
        if requested_username.is_empty() || room_name.is_empty() || version.is_empty() {
            return Err(ServerRuntimeError::InvalidHello);
        }
        if let Some(required_password_token) = self.server_password_token.as_ref() {
            let Some(server_password_token) = hello.password_token.as_ref() else {
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
                server_password_token.expose_secret(),
                required_password_token.expose_secret(),
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

        let readiness_reconnect_token = hello.readiness_reconnect_token;
        let capabilities = hello.capabilities;
        let mut replacement_outbound = Vec::new();
        if let Some(previous_session) = self.sessions.get(client_id).cloned() {
            replacement_outbound.extend(self.detach_readiness_membership(client_id, true)?);
            let connection_rate_history = self
                .playback_barrier_new_identity_rate_by_client
                .remove(client_id);
            replacement_outbound.extend(self.mark_playback_barrier_participant_disconnected(
                client_id,
                &previous_session.room,
            )?);
            replacement_outbound.extend(
                self.mark_room_buffering_participant_disconnected(
                    client_id,
                    &previous_session.room,
                )?,
            );
            self.remove_session_tracking(client_id);
            if let Some(connection_rate_history) = connection_rate_history {
                self.playback_barrier_new_identity_rate_by_client
                    .insert(client_id.to_owned(), connection_rate_history);
            }
            replacement_outbound
                .extend(self.refresh_mixed_readiness_cohort(&previous_session.room)?);
            self.cleanup_room_if_empty(&previous_session.room)?;
        }
        let username = self
            .find_free_username(&requested_username, Some(client_id))
            .ok_or(ServerRuntimeError::InvalidHello)?;
        if let Some(peer_ip) = peer_ip.filter(|peer_ip| !peer_ip.is_empty()) {
            self.client_peer_ips
                .insert(client_id.to_owned(), peer_ip.to_owned());
        } else {
            self.client_peer_ips.remove(client_id);
        }
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
                capabilities: capabilities.clone(),
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

        let mut outbound = replacement_outbound;
        let joined_message = user_joined_message_with_metadata(
            &username,
            &room_name,
            version,
            Some(capabilities.to_wire_value()),
        );
        for existing_client in self.clients_visible_on_join(&room_name, client_id) {
            outbound.push(DirectedProtocolMessage::new(
                existing_client,
                joined_message.clone(),
            ));
        }
        if !(self.readiness_enabled && capabilities.readiness_v2) {
            let ready_message = ready_update_message(&username, None, false, None);
            for room_client in self.clients_in_room(&room_name) {
                outbound.push(DirectedProtocolMessage::new(
                    room_client,
                    ready_message.clone(),
                ));
            }
        }
        if self.persistent_rooms_enabled {
            self.enqueue_list_snapshots_for_clients(
                &mut outbound,
                self.clients_receiving_to_gui_only_list_updates(Some(&room_name)),
            );
        }
        let room_playlist = self.room_playlist_state(&room_name);
        let playlist_snapshot_message = self.playlist_change_message_for_client(
            client_id,
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

        let base_motd = motd_for_client_context(
            version,
            self.motd_template.as_deref(),
            peer_ip.unwrap_or_default(),
            &username,
            &room_name,
        );
        let motd = persistent_rooms_notice_motd(
            base_motd,
            self.persistent_rooms_enabled,
            capabilities.persistent_rooms,
        );
        let readiness_attach_outbound = if self.readiness_enabled && capabilities.readiness_v2 {
            self.attach_readiness_membership(
                client_id,
                readiness_reconnect_token.as_ref(),
                true,
                false,
            )?
        } else {
            Vec::new()
        };
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
        if let Some(reconnect_identity) = self.readiness_reconnect_identity_by_client.get(client_id)
        {
            response.extra.insert(
                SOROTTE_READINESS_RECONNECT_TOKEN.to_owned(),
                Value::String(reconnect_identity.expose_secret().to_owned()),
            );
        }
        outbound.push(DirectedProtocolMessage::new(
            client_id,
            ProtocolMessage::hello(response),
        ));
        outbound.extend(readiness_attach_outbound);
        outbound.extend(self.refresh_mixed_readiness_cohort(&room_name)?);

        outbound.extend(self.refresh_room_buffering_participant(client_id)?);

        Ok(outbound)
    }

    pub(crate) fn handle_list(
        &self,
        client_id: &str,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        if !self.sessions.contains_key(client_id) {
            return Err(ServerRuntimeError::MissingSession(client_id.to_owned()));
        }
        let rooms = self.list_rooms_snapshot_for_client(client_id);
        Ok(vec![DirectedProtocolMessage::new(
            client_id,
            ProtocolMessage::list(ListPayload::rooms(rooms)),
        )])
    }

    pub(crate) fn handle_set(
        &mut self,
        client_id: &str,
        commands: Vec<ServerSetCommand>,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let mut session = self
            .sessions
            .get(client_id)
            .cloned()
            .ok_or_else(|| ServerRuntimeError::MissingSession(client_id.to_owned()))?;

        let mut outbound_messages = Vec::new();

        for command in commands {
            let mut room = None;
            let mut file = None;
            let mut controller_auth = None;
            let mut ready = None;
            let mut playlist_change = None;
            let mut playlist_index = None;
            let mut features = None;
            let mut playback_barrier = None;
            let mut readiness = None;
            let command_name = match command {
                ServerSetCommand::Room(value) => {
                    room = Some(value);
                    "room"
                }
                ServerSetCommand::File(value) => {
                    file = Some(value);
                    "file"
                }
                ServerSetCommand::ControllerAuth { room, password } => {
                    controller_auth = Some((room, password));
                    "controllerAuth"
                }
                ServerSetCommand::Ready {
                    ready: is_ready,
                    manually_initiated,
                    username,
                    set_by,
                } => {
                    ready = Some((is_ready, manually_initiated, username, set_by));
                    "ready"
                }
                ServerSetCommand::PlaylistChange(files) => {
                    playlist_change = Some(files);
                    "playlistChange"
                }
                ServerSetCommand::PlaylistIndex(index) => {
                    playlist_index = Some(index);
                    "playlistIndex"
                }
                ServerSetCommand::Features(capabilities) => {
                    features = Some(capabilities);
                    "features"
                }
                ServerSetCommand::PlaybackBarrier(extension) => {
                    playback_barrier = Some(*extension);
                    SOROTTE_PLAYBACK_BARRIER_V1
                }
                ServerSetCommand::Readiness(extension) => {
                    readiness = Some(*extension);
                    SOROTTE_READINESS_V2
                }
            };
            match command_name {
                "room" => {
                    let Some(room_name) = room.take() else {
                        continue;
                    };
                    let new_room_name =
                        truncate_text_to_max_chars(&room_name, DEFAULT_MAX_ROOM_NAME_LENGTH);
                    if session.room == new_room_name {
                        continue;
                    }

                    let previous_room = session.room.clone();
                    outbound_messages.extend(self.detach_readiness_membership(client_id, false)?);
                    outbound_messages.extend(self.mark_playback_barrier_participant_disconnected(
                        client_id,
                        &previous_room,
                    )?);
                    outbound_messages.extend(
                        self.mark_room_buffering_participant_disconnected(
                            client_id,
                            &previous_room,
                        )?,
                    );
                    let previous_ready =
                        if self.readiness_enabled && session.capabilities.readiness_v2 {
                            Some(false)
                        } else {
                            self.stored_user_ready(&session.username, &previous_room)
                        };
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
                    outbound_messages.extend(self.refresh_mixed_readiness_cohort(&previous_room)?);
                    // The moving client must observe its canonical room echo
                    // before any unscoped readiness V2 snapshot for the new
                    // membership. Otherwise an old-room revision can poison
                    // comparison or the later room echo can clear the freshly
                    // attached membership.
                    let mut new_room_readiness_outbound =
                        if self.readiness_enabled && session.capabilities.readiness_v2 {
                            self.attach_readiness_membership(client_id, None, false, false)?
                        } else {
                            Vec::new()
                        };
                    new_room_readiness_outbound
                        .extend(self.refresh_mixed_readiness_cohort(&session.room)?);
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
                        && let Some(file) = session.file.clone()
                    {
                        for peer_client in self.clients_in_room(&session.room) {
                            let file_message = user_file_update_message(
                                &session.username,
                                &session.room,
                                self.file_payload_for_client_from_source(
                                    &peer_client,
                                    client_id,
                                    &file,
                                ),
                            );
                            outbound_messages
                                .push(DirectedProtocolMessage::new(peer_client, file_message));
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
                    outbound_messages.extend(new_room_readiness_outbound);
                    if !(self.readiness_enabled && session.capabilities.readiness_v2) {
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
                    }
                    if self.persistent_rooms_enabled {
                        self.enqueue_list_snapshots_for_clients(
                            &mut outbound_messages,
                            self.clients_receiving_to_gui_only_list_updates(Some(&session.room)),
                        );
                    }

                    let room_playlist = self.room_playlist_state(&session.room);
                    let playlist_snapshot_message = self.playlist_change_message_for_client(
                        client_id,
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
                    outbound_messages.extend(self.refresh_room_buffering_participant(client_id)?);
                }
                "file" => {
                    let Some(file_update) = file.take() else {
                        continue;
                    };
                    session.file = file_update.clone();
                    self.sessions.insert(client_id.to_owned(), session.clone());

                    let Some(file) = file_update else {
                        continue;
                    };

                    let recipients = if self.isolate_rooms {
                        self.clients_in_room(&session.room)
                    } else {
                        self.clients_all()
                    };
                    for peer_client in recipients {
                        let file_message = user_file_update_message(
                            &session.username,
                            &session.room,
                            self.file_payload_for_client_from_source(
                                &peer_client,
                                client_id,
                                &file,
                            ),
                        );
                        outbound_messages
                            .push(DirectedProtocolMessage::new(peer_client, file_message));
                    }
                }
                "controllerAuth" => {
                    let Some((auth_room, auth_password)) = controller_auth.take() else {
                        continue;
                    };
                    let auth_room = auth_room.unwrap_or_else(|| session.room.clone());
                    let auth_password = auth_password.expose_secret();
                    match self.room_password_provider.check(&auth_room, auth_password) {
                        Ok(success) => {
                            if success {
                                self.add_room_controller(&session.username, &auth_room);
                            }
                            let auth_message = controller_auth_status_message(
                                &session.username,
                                &auth_room,
                                success,
                            );
                            for peer_client in self.clients_in_room(&auth_room) {
                                outbound_messages.push(DirectedProtocolMessage::new(
                                    peer_client,
                                    auth_message.clone(),
                                ));
                            }
                        }
                        Err(RoomPasswordCheckError::NotControlledRoom) => {
                            let new_room_name = self
                                .room_password_provider
                                .controlled_room_name_for(&auth_room, auth_password);
                            let new_room_message =
                                new_controlled_room_message(&new_room_name, auth_password);
                            outbound_messages
                                .push(DirectedProtocolMessage::new(client_id, new_room_message));
                        }
                        Err(RoomPasswordCheckError::InvalidPassword) => {
                            let auth_message = controller_auth_status_message(
                                &session.username,
                                &auth_room,
                                false,
                            );
                            for peer_client in self.clients_in_room(&auth_room) {
                                outbound_messages.push(DirectedProtocolMessage::new(
                                    peer_client,
                                    auth_message.clone(),
                                ));
                            }
                        }
                    }
                }
                "ready" => {
                    let Some((is_ready, manually_initiated, username, _set_by)) = ready.take()
                    else {
                        continue;
                    };
                    let outbound_ready = if self.readiness_enabled {
                        Some(is_ready)
                    } else {
                        None
                    };
                    let requested_username = username.as_deref().unwrap_or(&session.username);
                    if let Some(v2_outbound) = self.apply_legacy_readiness_to_v2(
                        client_id,
                        requested_username,
                        is_ready,
                        manually_initiated,
                    )? {
                        outbound_messages.extend(v2_outbound);
                        continue;
                    }
                    // Preserve the byte shape expected by legacy-only rooms:
                    // Python Syncplay does not synthesize `setBy` for ordinary
                    // self-Ready fanout. Controller overrides retain their
                    // authenticated actor in every room; V2/mixed self updates
                    // also derive the actor from the session, never client input.
                    let authenticated_self_actor_projection = (self.readiness_enabled
                        && self.room_readiness.contains_key(&session.room))
                    .then_some(session.username.as_str());
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
                            manually_initiated
                                .then_some(authenticated_self_actor_projection)
                                .flatten(),
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
                    let Some(new_files) = playlist_change.take() else {
                        continue;
                    };
                    self.ensure_room_state(&session.room);
                    let now_seconds = self.current_time_seconds();
                    let creation_identity = self.persistent_room_creation_identity(client_id);
                    let creation_required =
                        self.persistent_room_creation_required(&session.room, &new_files);
                    if self.user_can_control_playlist(&session.username, &session.room)
                        && playlist_is_valid(&new_files)
                        && self.persistent_room_creation_allowed(
                            &session.room,
                            &new_files,
                            &creation_identity,
                            now_seconds,
                        )
                    {
                        // A playlist replacement and a playlist-index update are
                        // distinct Syncplay protocol operations. Preserve the
                        // last explicit index here; clients that want another
                        // item send playlistIndex separately.
                        self.room_playlist_state_mut(&session.room).files = new_files.clone();
                        if creation_required {
                            self.record_persistent_room_creation(
                                &session.room,
                                creation_identity,
                                now_seconds,
                            );
                        } else if new_files.is_empty() {
                            self.release_persistent_room_ownership(&session.room);
                        }
                        self.persist_room_if_needed(&session.room)?;
                        for peer_client in self.clients_in_room(&session.room) {
                            let playlist_message = self.playlist_change_message_for_client(
                                &peer_client,
                                new_files.clone(),
                                Some(&session.username),
                            );
                            outbound_messages
                                .push(DirectedProtocolMessage::new(peer_client, playlist_message));
                        }
                    } else {
                        let room_state = self.room_playlist_state(&session.room);
                        outbound_messages.push(DirectedProtocolMessage::new(
                            client_id,
                            self.playlist_change_message_for_client(
                                client_id,
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
                    let Some(index) = playlist_index.take() else {
                        continue;
                    };
                    self.ensure_room_state(&session.room);
                    let index_is_valid =
                        self.room_playlist_state(&session.room).accepts_index(index);
                    if self.user_can_control_playlist(&session.username, &session.room)
                        && index_is_valid
                    {
                        let previous_index = self.room_playlist_state(&session.room).index;
                        self.room_playlist_state_mut(&session.room).index = index;
                        self.persist_room_if_needed(&session.room)?;
                        if previous_index != index
                            && self.room_readiness.get(&session.room).is_some_and(|room| {
                                matches!(
                                    room.pause_owner,
                                    RoomPauseOwner::None
                                        | RoomPauseOwner::ReadinessStartGate { .. }
                                        | RoomPauseOwner::EndOfPlaylist
                                )
                            })
                        {
                            outbound_messages.extend(self.set_readiness_pause_owner(
                                &session.room,
                                RoomPauseOwner::EndOfPlaylist,
                                true,
                            ));
                        }
                        let playlist_index = PlaylistIndexPayload::from_optional(index)
                            .with_user(session.username.clone());
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
                    let Some(capabilities) = features.take() else {
                        continue;
                    };
                    let previously_supported = session.capabilities.playback_barrier_v1;
                    let now_supported = capabilities.playback_barrier_v1;
                    let previously_supported_readiness =
                        self.readiness_enabled && session.capabilities.readiness_v2;
                    let now_supported_readiness =
                        self.readiness_enabled && capabilities.readiness_v2;
                    session.capabilities = capabilities;
                    self.sessions.insert(client_id.to_owned(), session.clone());
                    if !previously_supported && now_supported {
                        outbound_messages
                            .extend(self.refresh_room_buffering_participant(client_id)?);
                    } else if previously_supported && !now_supported {
                        outbound_messages.extend(
                            self.mark_room_buffering_participant_disconnected(
                                client_id,
                                &session.room,
                            )?,
                        );
                    }
                    if !previously_supported_readiness && now_supported_readiness {
                        let reconnect_identity = self
                            .readiness_reconnect_identity_by_client
                            .get(client_id)
                            .cloned();
                        outbound_messages.extend(self.attach_readiness_membership(
                            client_id,
                            reconnect_identity.as_ref(),
                            true,
                            true,
                        )?);
                    } else if previously_supported_readiness && !now_supported_readiness {
                        outbound_messages
                            .extend(self.detach_readiness_membership(client_id, true)?);
                    }
                    if previously_supported != now_supported
                        || previously_supported_readiness != now_supported_readiness
                    {
                        outbound_messages
                            .extend(self.refresh_mixed_readiness_cohort(&session.room)?);
                    }
                }
                SOROTTE_PLAYBACK_BARRIER_V1 => {
                    let Some(extension) = playback_barrier.take() else {
                        continue;
                    };
                    outbound_messages
                        .extend(self.handle_playback_barrier_set(client_id, extension)?);
                }
                SOROTTE_READINESS_V2 => {
                    let Some(extension) = readiness.take() else {
                        continue;
                    };
                    outbound_messages.extend(self.handle_readiness_set(client_id, extension)?);
                }
                _ => {}
            }
        }

        Ok(outbound_messages)
    }

    pub(crate) fn handle_state(
        &mut self,
        client_id: &str,
        mut state: ServerStateCommand,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let session = self
            .sessions
            .get(client_id)
            .cloned()
            .ok_or_else(|| ServerRuntimeError::MissingSession(client_id.to_owned()))?;

        if let Some(server_ignoring_counter) = state.server_ignoring {
            self.acknowledge_server_ignoring_counter(client_id, server_ignoring_counter);
        }
        if let Some(client_ignoring_counter) = state.client_ignoring {
            self.queue_client_ignoring_counter(client_id, client_ignoring_counter);
        }
        if let Some(ping) = state.ping.as_ref() {
            if let Some(client_latency_calculation) = ping.client_latency_calculation {
                self.queue_client_latency_calculation(client_id, client_latency_calculation);
            }
            self.ingest_client_ping_metrics(client_id, ping.latency_calculation, ping.client_rtt);
        }
        self.record_client_state_update_now(client_id);
        self.persist_occupied_room_activity_if_due_at(&session.room, self.current_time_seconds())?;
        let had_barrier_ack = state
            .playback_barrier
            .as_ref()
            .is_some_and(|extension| extension.ready.is_some() || extension.started.is_some());
        let had_readiness_report = state
            .readiness
            .as_ref()
            .is_some_and(|extension| extension.technical.is_some());
        let mut barrier_outbound = if let Some(extension) = state.readiness.take() {
            self.handle_readiness_state(client_id, extension)?
        } else {
            Vec::new()
        };
        if let Some(extension) = state.playback_barrier.take() {
            barrier_outbound.extend(self.handle_playback_barrier_state(client_id, extension)?);
        }
        let buffering_changed_room_playback = barrier_outbound.iter().any(|directed| {
            matches!(
                &directed.message,
                ProtocolMessage::State(message) if message.state.playstate.is_some()
            )
        });
        if self.server_ignoring_counter(client_id) > 0 {
            return Ok(barrier_outbound);
        }
        // Barrier observations are acknowledgements, not playback-control
        // commands. Ignore any stale optimistic playstate bundled alongside
        // them, especially the paused sample sent with the final MediaReady.
        if had_barrier_ack
            || had_readiness_report
            || buffering_changed_room_playback
            || self
                .room_playback_barriers
                .get(&session.room)
                .is_some_and(|barrier| barrier.phase == PlaybackBarrierPhase::Preparing)
        {
            return Ok(barrier_outbound);
        }

        let Some(playstate) = state.playstate else {
            return Ok(barrier_outbound);
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
        let readiness_v2_transport = self.readiness_enabled
            && session.capabilities.readiness_v2
            && self.room_readiness.contains_key(&session.room);
        let v2_transport_change = can_control_room && pause_changed && readiness_v2_transport;
        let observed_paused = playstate.paused.unwrap_or(room_state_before.paused);
        let accepted_v2_user_transport = v2_transport_change
            && self.consume_pending_user_transport(
                client_id,
                &session.room,
                &session.username,
                observed_paused,
                PendingUserTransportEvidence::AcceptedIndirectAction,
            );
        let automatic_v2_transport_owner = v2_transport_change
            .then(|| {
                self.automatic_transport_owner_for_observation(
                    &session.room,
                    &session.username,
                    observed_paused,
                )
            })
            .flatten();
        let pause_is_already_automatic = playstate.paused == Some(true)
            && self.room_readiness.get(&session.room).is_some_and(|room| {
                !matches!(
                    room.pause_owner,
                    RoomPauseOwner::None | RoomPauseOwner::User { .. }
                )
            });
        let legacy_transport_change = can_control_room
            && pause_changed
            && !readiness_v2_transport
            && !pause_is_already_automatic;
        let sample_paused = playstate.paused.unwrap_or(room_state_before.paused);
        let playback_sample_position = playstate.position.map(|mut position| {
            if !sample_paused {
                position += forward_delay_seconds;
            }
            position
        });
        self.record_client_playback_state_sample(client_id, playback_sample_position, now_seconds);

        if v2_transport_change
            && !accepted_v2_user_transport
            && automatic_v2_transport_owner.is_none()
        {
            self.stage_unclassified_user_transport_observation(
                client_id,
                &session.room,
                &session.username,
                observed_paused,
            );
            barrier_outbound.push(DirectedProtocolMessage::new(
                client_id,
                self.forced_state_sync_message_for_client(
                    client_id,
                    room_state_before.position,
                    room_state_before.paused,
                    false,
                    room_state_before.set_by.as_deref(),
                ),
            ));
            return Ok(barrier_outbound);
        }
        if accepted_v2_user_transport || legacy_transport_change {
            barrier_outbound
                .extend(self.retire_awaiting_playback_barrier_decision(client_id, &session.room));
        }
        if (accepted_v2_user_transport || legacy_transport_change) && observed_paused {
            barrier_outbound
                .extend(self.claim_user_pause_ownership(&session.room, &session.username));
        } else if observed_paused
            && let Some(owner) = automatic_v2_transport_owner
            && self.room_readiness.get(&session.room).is_some_and(|room| {
                matches!(room.pause_owner, RoomPauseOwner::None) || room.pause_owner == owner
            })
        {
            barrier_outbound.extend(self.set_readiness_pause_owner(&session.room, owner, true));
        }

        if can_control_room
            && playstate.paused == Some(false)
            && room_state_before.paused
            && self.readiness_enabled
            && self.room_readiness.contains_key(&session.room)
            && self
                .room_playback_barriers
                .get(&session.room)
                .is_some_and(|barrier| barrier.phase == PlaybackBarrierPhase::Preparing)
        {
            barrier_outbound.push(DirectedProtocolMessage::new(
                client_id,
                self.forced_state_sync_message_for_client(
                    client_id,
                    room_state_before.position,
                    true,
                    false,
                    room_state_before.set_by.as_deref(),
                ),
            ));
            return Ok(barrier_outbound);
        }

        if !do_seek.unwrap_or(false) && !pause_changed {
            return Ok(barrier_outbound);
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
            self.advance_transport_authority_revision(&session.room);
            self.seed_room_client_playback_states(&session.room, watcher_position, now_seconds);
            self.persist_room_if_needed(&session.room)?;
            if pause_changed && !sample_paused {
                barrier_outbound.extend(self.set_readiness_pause_owner(
                    &session.room,
                    RoomPauseOwner::None,
                    true,
                ));
            }
            let room_state = self.room_playback_state_at(&session.room, now_seconds);
            let mut outbound_messages = barrier_outbound;
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
        barrier_outbound.extend([
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
        ]);
        Ok(barrier_outbound)
    }
}
