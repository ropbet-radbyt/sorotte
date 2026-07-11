use super::*;

impl ClientSession {
    fn migrate_provisional_local_identity(&mut self, assigned_username: &str) -> bool {
        let Some(provisional_username) = self.model.connection.username.clone() else {
            return false;
        };
        if provisional_username == assigned_username {
            return false;
        }

        let provisional_user = self.model.room.users.remove(&provisional_username);
        let provisional_room = provisional_user
            .as_ref()
            .and_then(|user| user.room.clone())
            .or_else(|| self.model.room.name.clone());
        if let Some(room_name) = provisional_room.as_deref() {
            let _ = self
                .model
                .room
                .domain
                .leave_room(&provisional_username, room_name);
        }
        let provisional_file = provisional_user.and_then(|user| user.file);
        self.model
            .room
            .media_match_peer_tiers
            .remove(&provisional_username);
        self.model
            .room
            .media_match_peer_tiers
            .remove(assigned_username);

        if let Some(provisional_file) = provisional_file {
            let assigned_user = self
                .model
                .room
                .users
                .entry(assigned_username.to_owned())
                .or_default();
            if assigned_user.file.is_none() {
                assigned_user.file = Some(provisional_file);
            }
        }

        true
    }

    pub(super) fn apply_hello(&mut self, hello: ClientHello) {
        if self.model.reconnect.in_progress {
            self.model.reconnect.in_progress = false;
            self.model.reconnect.connected_intent = true;
        }

        if self.chat_config.apply_server_max_chat_message_length {
            self.chat_config.max_chat_message_length = hello.max_chat_message_length;
        }

        let username = hello.username;
        let room_name = hello.room;
        let identity_migrated = self.migrate_provisional_local_identity(&username);
        let server_assigned_ready = identity_migrated
            .then(|| {
                self.model
                    .room
                    .users
                    .get(&username)
                    .and_then(|user| user.ready)
            })
            .flatten();

        self.model.connection.username = Some(username.clone());
        self.update_local_room(room_name.clone());

        self.model.controller.reidentify_intent = self
            .model
            .controller
            .room_passwords
            .get(&room_name)
            .cloned()
            .map(|password| (room_name.clone(), password));

        self.set_user_room(&username, Some(room_name));
        self.set_user_ready(&username, server_assigned_ready.unwrap_or(false));

        if let Some(current_room) = self.model.room.name.clone()
            && let Some(pending_playlist) = self.model.playlist.pending.take()
        {
            self.model
                .playlist
                .rooms
                .insert(current_room, pending_playlist);
        }

        if let Some(restored_ready) = self.model.reconnect.ready_restore_snapshot.take() {
            self.model.reconnect.ready_restore_intent = Some(restored_ready);
            self.set_user_ready(&username, restored_ready);
        }

        if let Some(restored_file) = self.model.reconnect.file_restore_snapshot.take() {
            self.set_user_file(&username, Some(restored_file.clone()));
            self.model.reconnect.file_restore_intent = Some(restored_file);
        }

        if let Some(restored_controller) = self.model.reconnect.controller_restore_snapshot.take() {
            self.set_user_controller(&username, restored_controller);
        }

        self.model.connection.phase = ConnectionPhase::Active(hello.capabilities);
    }

    pub(super) fn apply_set(&mut self, commands: Vec<ClientSetCommand>, now_seconds: Option<f64>) {
        for command in commands {
            let mut room = None;
            let mut users = None;
            let mut controller_auth = None;
            let mut new_controlled_room = None;
            let mut ready = None;
            let mut playlist_change = None;
            let mut playlist_index = None;
            let mut features = None;
            match command {
                ClientSetCommand::Room(value) => room = Some(value),
                ClientSetCommand::Users(value) => users = Some(value),
                ClientSetCommand::ControllerAuth(value) => controller_auth = Some(value),
                ClientSetCommand::NewControlledRoom(value) => new_controlled_room = Some(value),
                ClientSetCommand::Ready(value) => ready = Some(value),
                ClientSetCommand::PlaylistChange { files, user } => {
                    playlist_change = Some((files, user));
                }
                ClientSetCommand::PlaylistIndex { index, user } => {
                    playlist_index = Some((index, user));
                }
                ClientSetCommand::Features {
                    username,
                    capabilities,
                } => features = Some((username, capabilities)),
            }

            if let Some(room) = room {
                if let Some(username) = self.model.connection.username.clone() {
                    let room_changed = self.user_room(&username) != Some(room.as_str());
                    self.set_user_room(&username, Some(room.clone()));
                    if room_changed {
                        self.set_user_controller(&username, false);
                    }
                }
                self.update_local_room(room);
            }

            if let Some(users) = users {
                for user_payload in users {
                    let username = user_payload.username;
                    let was_local_user =
                        self.model.connection.username.as_deref() == Some(username.as_str());
                    let previous_user_view = self.model.room.users.get(&username).cloned();

                    if user_payload.left {
                        if !was_local_user {
                            self.queue_user_left_notification_if_relevant(
                                &username,
                                previous_user_view,
                            );
                        }
                        self.remove_user(&username);
                        continue;
                    }

                    if let Some(room) = user_payload.room {
                        let room_changed = previous_user_view
                            .as_ref()
                            .and_then(|view| view.room.as_deref())
                            != Some(room.as_str());
                        self.set_user_room(&username, Some(room.clone()));
                        if room_changed {
                            self.set_user_controller(&username, false);
                        }
                        if was_local_user {
                            self.update_local_room(room);
                        }
                    }

                    if let Some(file) = user_payload.file {
                        self.set_user_file(&username, Some(file));
                    }

                    if let Some(controller) = user_payload.controller {
                        self.set_user_controller(&username, controller);
                    }

                    if let Some(capabilities) = user_payload.capabilities {
                        self.set_user_capabilities(&username, Some(capabilities));
                    }

                    if let Some(is_ready) = user_payload.ready {
                        self.set_user_ready(&username, is_ready);
                    }

                    if !was_local_user {
                        self.queue_user_change_notification_if_relevant(
                            &username,
                            previous_user_view,
                        );
                    }
                }
            }

            if let Some(ready) = ready {
                let target_username = ready
                    .username
                    .or_else(|| self.model.connection.username.clone());
                if let Some(target_username) = target_username {
                    if self.user_room(&target_username).is_none()
                        && let Some(room_name) = self.model.room.name.clone()
                    {
                        self.set_user_room(&target_username, Some(room_name));
                    }
                    self.set_user_ready_state(&target_username, ready.ready);
                }
            }

            if let Some((username, capabilities)) = features {
                let target_username = username.or_else(|| self.model.connection.username.clone());
                if let Some(target_username) = target_username {
                    self.set_user_capabilities(&target_username, Some(capabilities));
                }
            }

            if let Some(controller_auth) = controller_auth {
                let target_username = controller_auth
                    .user
                    .or_else(|| self.model.connection.username.clone());
                let target_room = controller_auth
                    .room
                    .or_else(|| self.model.room.name.clone());
                let target_is_local_user = target_username
                    .as_deref()
                    .zip(self.model.connection.username.as_deref())
                    .is_some_and(|(target, local)| target == local);
                let target_room_matches_local_room = target_room
                    .as_deref()
                    .zip(self.model.room.name.as_deref())
                    .is_some_and(|(target, local)| target == local);

                match controller_auth.success {
                    Some(true) => {
                        if let Some(target_username) = target_username.as_deref() {
                            self.set_user_controller(target_username, true);
                        }
                        if target_is_local_user
                            && target_room_matches_local_room
                            && let (Some(target_room), Some(password)) = (
                                target_room.as_deref(),
                                self.model.controller.last_auth_password_attempt.clone(),
                            )
                        {
                            self.remember_control_password_for_room(target_room, &password);
                        }
                        if target_room_matches_local_room
                            && let (Some(target_username), Some(target_room)) =
                                (target_username, target_room)
                        {
                            self.pending_controller_auth_notifications.push(
                                ControllerAuthTransitionNotification::Succeeded {
                                    username: target_username,
                                    room: target_room,
                                    hide_from_osd: !self.behavior_config.show_same_room_osd,
                                },
                            );
                        }
                    }
                    Some(false) => {
                        if target_is_local_user
                            && let (Some(target_username), Some(target_room)) =
                                (target_username, target_room)
                        {
                            let hide_from_osd = self
                                .noncontroller_event_hide_from_osd_legacy_compatible(
                                    &target_username,
                                );
                            self.pending_controller_auth_notifications.push(
                                ControllerAuthTransitionNotification::Failed {
                                    username: target_username,
                                    room: target_room,
                                    hide_from_osd,
                                },
                            );
                        }
                    }
                    None => {}
                }
            }

            if let Some(new_controlled_room) = new_controlled_room
                && let (Some(room_name), Some(password)) =
                    (new_controlled_room.room_name, new_controlled_room.password)
            {
                let password = password.expose_secret();
                let normalized_password =
                    Self::normalize_control_password_legacy_compatible(password);
                self.model.readiness.autoplay_enabled = false;
                self.stop_autoplay_countdown();
                self.remember_control_password_for_room(&room_name, password);
                self.pending_controlled_room_creation_notifications.push(
                    ControlledRoomCreationNotification::Created {
                        room: room_name.clone(),
                        password: normalized_password.clone().into(),
                    },
                );

                if let Some(local_username) = self.model.connection.username.clone() {
                    self.update_local_room(room_name.clone());
                    self.set_user_room(&local_username, Some(room_name.clone()));
                    self.set_user_controller(&local_username, false);
                    if Self::is_controlled_room_name(&room_name) && !normalized_password.is_empty()
                    {
                        self.model.controller.controlled_room_switch_intent =
                            Some(room_name.clone());
                        self.model.controller.reidentify_intent =
                            Some((room_name, normalized_password));
                    }
                }
            }

            if let Some((playlist_change_files, playlist_change_user)) = playlist_change {
                let mut skip_playlist_change_apply = false;
                if playlist_change_user.is_none() && playlist_change_files.is_empty() {
                    if let Some(restore_intent) =
                        self.model.reconnect.playlist_restore_snapshot.take()
                    {
                        self.model.reconnect.playlist_restore_intent = Some(restore_intent);
                        skip_playlist_change_apply = true;
                    }
                } else {
                    self.model.reconnect.playlist_restore_snapshot = None;
                }

                if !skip_playlist_change_apply {
                    if let Some(room_name) =
                        self.resolve_room_for_playlist_update(playlist_change_user.as_deref())
                    {
                        let previous_active_target = self
                            .model
                            .playlist
                            .rooms
                            .get(&room_name)
                            .and_then(|playlist| playlist.index)
                            .and_then(|index| {
                                self.playlist_target_for_room_index(&room_name, index)
                            })
                            .map(str::to_owned);
                        if let Some(previous_active_target) = previous_active_target {
                            self.model
                                .playlist
                                .active_targets_before_index_update
                                .insert(room_name.clone(), previous_active_target);
                        } else {
                            self.model
                                .playlist
                                .active_targets_before_index_update
                                .remove(&room_name);
                        }
                        let current_files = self
                            .model
                            .playlist
                            .rooms
                            .get(&room_name)
                            .map(|playlist| playlist.files.clone())
                            .unwrap_or_default();
                        self.capture_playlist_undo_snapshot_legacy_compatible(
                            &room_name,
                            &current_files,
                            &playlist_change_files,
                        );

                        let playlist = self.model.playlist.rooms.entry(room_name).or_default();
                        playlist.files = playlist_change_files;
                        playlist.set_by = playlist_change_user;
                    } else {
                        let pending_playlist = self
                            .model
                            .playlist
                            .pending
                            .get_or_insert_with(Default::default);
                        pending_playlist.files = playlist_change_files;
                        pending_playlist.set_by = playlist_change_user;
                    }
                }
            }

            if let Some((playlist_index_value, playlist_index_user)) = playlist_index {
                if playlist_index_user.is_some() {
                    self.model.reconnect.playlist_restore_snapshot = None;
                }

                let room_name =
                    self.resolve_room_for_playlist_update(playlist_index_user.as_deref());
                let preserved_active_target = room_name.as_deref().and_then(|room_name| {
                    self.model
                        .playlist
                        .active_targets_before_index_update
                        .get(room_name)
                        .cloned()
                });
                let set_by_local = playlist_index_user
                    .as_deref()
                    .zip(self.model.connection.username.as_deref())
                    .is_some_and(|(set_by, local_username)| set_by == local_username);
                if set_by_local {
                    self.model.playback.last_advanced_at_seconds = now_seconds;
                }
                let preserves_active_target_after_playlist_change = room_name
                    .as_deref()
                    .and_then(|room_name| {
                        playlist_index_value.and_then(|index| {
                            self.playlist_target_for_room_index(room_name, index)
                                .map(str::to_owned)
                        })
                    })
                    .zip(preserved_active_target.as_deref())
                    .is_some_and(|(next_target, previous_target)| next_target == previous_target);

                let should_queue_playlist_reset = if !self
                    .should_track_playlist_index_transition_for_room(room_name.as_deref())
                {
                    false
                } else if !self.model.playlist.received_first_index {
                    self.model.playlist.received_first_index = true;
                    false
                } else if set_by_local && self.model.playlist.suppress_next_self_index_reset {
                    self.model.playlist.suppress_next_self_index_reset = false;
                    false
                } else {
                    !preserves_active_target_after_playlist_change
                };
                if should_queue_playlist_reset {
                    self.note_recent_rewind(
                        now_seconds.unwrap_or_else(unix_wall_clock_time_seconds_legacy_compatible),
                    );
                    self.queue_playlist_index_reset_intent(false);
                }

                if let Some(room_name) = room_name.as_deref() {
                    let playlist = self
                        .model
                        .playlist
                        .rooms
                        .entry(room_name.to_owned())
                        .or_default();
                    playlist.index = playlist_index_value;
                    if playlist_index_user.is_some() {
                        playlist.set_by = playlist_index_user;
                    }
                } else {
                    let pending_playlist = self
                        .model
                        .playlist
                        .pending
                        .get_or_insert_with(Default::default);
                    pending_playlist.index = playlist_index_value;
                    if playlist_index_user.is_some() {
                        pending_playlist.set_by = playlist_index_user;
                    }
                }
                if let Some(room_name) = room_name.as_deref() {
                    self.model
                        .playlist
                        .active_targets_before_index_update
                        .remove(room_name);
                }
            }
        }
    }

    pub(super) fn apply_list(&mut self, rooms: BTreeMap<String, BTreeMap<String, ClientListUser>>) {
        self.model.room.users.clear();
        self.model.room.media_match_peer_tiers.clear();
        self.model.room.known_rooms.clear();
        self.model.room.domain = SyncDomain::default();

        let mut resolved_self_room = None;
        let current_username = self.model.connection.username.clone();

        for (room_name, room_users) in rooms {
            self.model.room.known_rooms.insert(room_name.clone());
            for (username, user_entry) in room_users {
                if username.trim().is_empty() {
                    continue;
                }
                self.set_user_room(&username, Some(room_name.clone()));
                self.set_user_file(&username, user_entry.file);
                self.set_user_controller(&username, user_entry.controller);
                self.set_user_ready_state(&username, user_entry.ready);
                self.set_user_capabilities(&username, user_entry.capabilities);
                if current_username.as_deref() == Some(username.as_str()) {
                    resolved_self_room = Some(room_name.clone());
                }
            }
        }

        if let Some(resolved_self_room) = resolved_self_room {
            self.update_local_room(resolved_self_room);
        }
    }

    pub(super) fn apply_state_at(
        &mut self,
        state_payload: ClientStateUpdate,
        now_seconds: Option<f64>,
    ) {
        let Some(playstate) = state_payload.playstate else {
            return;
        };

        let room_name = playstate
            .set_by
            .as_deref()
            .and_then(|username| self.user_room(username).map(str::to_owned))
            .or_else(|| self.model.room.name.clone());

        if let Some(room_name) = room_name {
            self.merge_room_playstate(
                room_name,
                playstate,
                now_seconds.unwrap_or_else(unix_wall_clock_time_seconds_legacy_compatible),
            );
        }
    }

    pub(super) fn apply_chat(&mut self, notification: ChatNotification) {
        self.pending_chat_notifications.push(notification);
    }

    pub(super) fn sanitize_chat_message_legacy_compatible(message: &str) -> String {
        message
            .chars()
            .filter(|character| *character != '\n' && *character != '\r')
            .collect()
    }

    pub(super) fn truncate_chat_message_legacy_compatible(
        message: &str,
        max_length: usize,
    ) -> String {
        message.chars().take(max_length).collect()
    }

    pub(super) fn parse_numeric_version_components(version: &str) -> Option<Vec<u32>> {
        let trimmed = version.trim();
        if trimmed.is_empty() {
            return None;
        }

        let mut components = Vec::new();
        for part in trimmed.split('.') {
            if part.is_empty() {
                return None;
            }
            components.push(part.parse::<u32>().ok()?);
        }
        Some(components)
    }

    pub(crate) fn meets_min_version_legacy_compatible(version: &str, min_version: &str) -> bool {
        let Some(mut version_components) = Self::parse_numeric_version_components(version) else {
            return false;
        };
        let Some(mut min_version_components) = Self::parse_numeric_version_components(min_version)
        else {
            return false;
        };

        let width = version_components.len().max(min_version_components.len());
        version_components.resize(width, 0);
        min_version_components.resize(width, 0);
        version_components >= min_version_components
    }
}
