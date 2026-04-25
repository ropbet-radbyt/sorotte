use super::*;

impl ClientSession {
    pub(super) fn apply_hello(&mut self, hello: syncplay_protocol::HelloPayload) {
        if self.reconnect_in_progress {
            self.reconnect_in_progress = false;
            self.reconnect_connected_intent = true;
        }

        let server_version = hello.effective_version().to_owned();
        self.server_readiness_supported = Some(
            Self::feature_bool(hello.features.as_ref(), "readiness").unwrap_or_else(|| {
                Self::meets_min_version_legacy_compatible(
                    &server_version,
                    LEGACY_USER_READY_MIN_VERSION,
                )
            }),
        );
        self.server_set_others_readiness_supported = Some(
            Self::feature_bool(hello.features.as_ref(), "setOthersReadiness").unwrap_or_else(
                || {
                    Self::meets_min_version_legacy_compatible(
                        &server_version,
                        LEGACY_SET_OTHERS_READINESS_MIN_VERSION,
                    )
                },
            ),
        );
        self.server_managed_rooms_supported = Some(
            Self::feature_bool(hello.features.as_ref(), "managedRooms").unwrap_or_else(|| {
                Self::meets_min_version_legacy_compatible(
                    &server_version,
                    LEGACY_MANAGED_ROOMS_MIN_VERSION,
                )
            }),
        );
        self.server_shared_playlists_supported = Some(
            Self::feature_bool(hello.features.as_ref(), "sharedPlaylists").unwrap_or_else(|| {
                Self::meets_min_version_legacy_compatible(
                    &server_version,
                    LEGACY_SHARED_PLAYLIST_MIN_VERSION,
                )
            }),
        );
        self.server_chat_supported = Some(
            Self::feature_bool(hello.features.as_ref(), "chat").unwrap_or_else(|| {
                Self::meets_min_version_legacy_compatible(&server_version, LEGACY_CHAT_MIN_VERSION)
            }),
        );
        self.server_persistent_rooms_supported =
            Some(Self::feature_bool(hello.features.as_ref(), "persistentRooms").unwrap_or(false));
        self.server_max_username_length = Some(
            Self::feature_usize(hello.features.as_ref(), "maxUsernameLength")
                .unwrap_or(LEGACY_FALLBACK_MAX_USERNAME_LENGTH),
        );
        self.server_max_room_name_length = Some(
            Self::feature_usize(hello.features.as_ref(), "maxRoomNameLength")
                .unwrap_or(LEGACY_FALLBACK_MAX_ROOM_NAME_LENGTH),
        );
        self.server_max_filename_length = Some(
            Self::feature_usize(hello.features.as_ref(), "maxFilenameLength")
                .unwrap_or(LEGACY_FALLBACK_MAX_FILENAME_LENGTH),
        );
        if self.chat_config.apply_server_max_chat_message_length {
            self.chat_config.max_chat_message_length =
                Self::feature_usize(hello.features.as_ref(), "maxChatMessageLength")
                    .unwrap_or(LEGACY_FALLBACK_MAX_CHAT_MESSAGE_LENGTH);
        }

        let username = hello.username;
        let room_name = hello.room.name;

        self.username = Some(username.clone());
        self.update_local_room(room_name.clone());

        self.controller_reidentify_intent = self
            .controlled_room_passwords
            .get(&room_name)
            .cloned()
            .map(|password| (room_name.clone(), password));

        self.set_user_room(&username, Some(room_name));
        self.set_user_ready(&username, false);

        if let Some(current_room) = self.room.clone()
            && let Some(pending_playlist) = self.pending_playlist.take()
        {
            self.room_playlists.insert(current_room, pending_playlist);
        }

        if let Some(restored_ready) = self.reconnect_ready_restore_snapshot.take() {
            self.reconnect_ready_restore_intent = Some(restored_ready);
            self.set_user_ready(&username, restored_ready);
        }

        if let Some(restored_file_payload) = self.reconnect_file_restore_snapshot.take() {
            let (has_file, file_name, file_size, file_duration) =
                Self::list_payload_file_info(Some(&restored_file_payload));
            self.set_user_file_info(&username, has_file, file_name, file_size, file_duration);
            self.reconnect_file_restore_intent = Some(restored_file_payload);
        }

        if let Some(restored_controller) = self.reconnect_controller_restore_snapshot.take() {
            self.set_user_controller(&username, restored_controller);
        }
    }

    pub(super) fn apply_set(&mut self, set_payload: SetPayload, now_seconds: Option<f64>) {
        if let Some(room) = set_payload.room {
            if let Some(username) = self.username.clone() {
                let room_changed = self.user_room(&username) != Some(room.name.as_str());
                self.set_user_room(&username, Some(room.name.clone()));
                if room_changed {
                    self.set_user_controller(&username, false);
                }
            }
            self.update_local_room(room.name);
        }

        if let Some(users) = set_payload.user {
            for (username, user_payload) in users {
                let was_local_user = self.username.as_deref() == Some(username.as_str());
                let previous_user_view = self.user_views.get(&username).cloned();

                if user_payload
                    .event
                    .as_ref()
                    .and_then(|event| event.get("left"))
                    .and_then(|value| value.as_bool())
                    == Some(true)
                {
                    if !was_local_user {
                        self.queue_user_left_notification_if_relevant(
                            &username,
                            previous_user_view,
                        );
                    }
                    self.remove_user(&username);
                    continue;
                }

                let room_payload = user_payload.room;
                if let Some(room) = room_payload {
                    let room_changed = previous_user_view
                        .as_ref()
                        .and_then(|view| view.room.as_deref())
                        != Some(room.name.as_str());
                    self.set_user_room(&username, Some(room.name.clone()));
                    if room_changed {
                        self.set_user_controller(&username, false);
                    }
                    if was_local_user {
                        self.update_local_room(room.name);
                    }
                }

                // Legacy modUser only applies file updates when the payload is truthy.
                if let Some(file) = user_payload.file.as_ref()
                    && Self::legacy_json_value_truthy(file)
                {
                    let (file_name, file_size, file_duration) =
                        Self::file_metadata_from_payload(file);
                    self.set_user_file_info(&username, true, file_name, file_size, file_duration);
                }

                if let Some(controller) = user_payload.controller {
                    self.set_user_controller(&username, controller);
                }

                if let Some(features) = user_payload.features {
                    self.set_user_features(&username, Some(features));
                }

                if let Some(is_ready) = user_payload.is_ready {
                    self.set_user_ready(&username, is_ready);
                }

                if !was_local_user {
                    self.queue_user_change_notification_if_relevant(&username, previous_user_view);
                }
            }
        }

        if let Some(ready) = set_payload.ready {
            let target_username = ready
                .username
                .or(ready.set_by)
                .or_else(|| self.username.clone());
            if let Some(target_username) = target_username {
                if self.user_room(&target_username).is_none()
                    && let Some(room_name) = self.room.clone()
                {
                    self.set_user_room(&target_username, Some(room_name));
                }
                self.set_user_ready(&target_username, ready.is_ready);
            }
        }

        if let Some(features_update) = set_payload.features {
            let target_username = features_update
                .get("username")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| self.username.clone());
            let target_features = features_update.get("features").cloned().or_else(|| {
                features_update
                    .get("username")
                    .is_none()
                    .then_some(features_update.clone())
            });
            if let Some(target_username) = target_username {
                self.set_user_features(&target_username, target_features);
            }
        }

        if let Some(controller_auth) = set_payload.controller_auth {
            let target_username = controller_auth.user.or_else(|| self.username.clone());
            let target_room = controller_auth.room.or_else(|| self.room.clone());
            let target_is_local_user = target_username
                .as_deref()
                .zip(self.username.as_deref())
                .is_some_and(|(target, local)| target == local);
            let target_room_matches_local_room = target_room
                .as_deref()
                .zip(self.room.as_deref())
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
                            self.last_controller_auth_password_attempt.clone(),
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
                            .noncontroller_event_hide_from_osd_legacy_compatible(&target_username);
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

        if let Some(new_controlled_room) = set_payload.new_controlled_room
            && let (Some(room_name), Some(password)) =
                (new_controlled_room.room_name, new_controlled_room.password)
        {
            let normalized_password = Self::normalize_control_password_legacy_compatible(&password);
            self.autoplay_enabled = false;
            self.stop_autoplay_countdown();
            self.remember_control_password_for_room(&room_name, &password);
            self.pending_controlled_room_creation_notifications.push(
                ControlledRoomCreationNotification::Created {
                    room: room_name.clone(),
                    password: normalized_password.clone(),
                },
            );

            if let Some(local_username) = self.username.clone() {
                self.update_local_room(room_name.clone());
                self.set_user_room(&local_username, Some(room_name.clone()));
                self.set_user_controller(&local_username, false);
                if Self::is_controlled_room_name(&room_name) && !normalized_password.is_empty() {
                    self.controlled_room_switch_intent = Some(room_name.clone());
                    self.controller_reidentify_intent = Some((room_name, normalized_password));
                }
            }
        }

        if let Some(playlist_change) = set_payload.playlist_change {
            let mut skip_playlist_change_apply = false;
            if playlist_change.user.is_none() && playlist_change.files.is_empty() {
                if let Some(restore_intent) = self.reconnect_playlist_restore_snapshot.take() {
                    self.reconnect_playlist_restore_intent = Some(restore_intent);
                    skip_playlist_change_apply = true;
                }
            } else {
                self.reconnect_playlist_restore_snapshot = None;
            }

            if !skip_playlist_change_apply {
                let playlist_change_files = playlist_change.files;
                let playlist_change_user = playlist_change.user;
                if let Some(room_name) =
                    self.resolve_room_for_playlist_update(playlist_change_user.as_deref())
                {
                    let previous_active_target = self
                        .room_playlists
                        .get(&room_name)
                        .and_then(|playlist| playlist.index)
                        .and_then(|index| self.playlist_target_for_room_index(&room_name, index))
                        .map(str::to_owned);
                    if let Some(previous_active_target) = previous_active_target {
                        self.playlist_active_targets_before_index_update
                            .insert(room_name.clone(), previous_active_target);
                    } else {
                        self.playlist_active_targets_before_index_update
                            .remove(&room_name);
                    }
                    let current_files = self
                        .room_playlists
                        .get(&room_name)
                        .map(|playlist| playlist.files.clone())
                        .unwrap_or_default();
                    self.capture_playlist_undo_snapshot_legacy_compatible(
                        &room_name,
                        &current_files,
                        &playlist_change_files,
                    );

                    let playlist = self.room_playlists.entry(room_name).or_default();
                    playlist.files = playlist_change_files;
                    playlist.set_by = playlist_change_user;
                } else {
                    let pending_playlist =
                        self.pending_playlist.get_or_insert_with(Default::default);
                    pending_playlist.files = playlist_change_files;
                    pending_playlist.set_by = playlist_change_user;
                }
            }
        }

        if let Some(playlist_index) = set_payload.playlist_index {
            if playlist_index.user.is_some() {
                self.reconnect_playlist_restore_snapshot = None;
            }

            let room_name = self.resolve_room_for_playlist_update(playlist_index.user.as_deref());
            let preserved_active_target = room_name.as_deref().and_then(|room_name| {
                self.playlist_active_targets_before_index_update
                    .get(room_name)
                    .cloned()
            });
            let set_by_local = playlist_index
                .user
                .as_deref()
                .zip(self.username.as_deref())
                .is_some_and(|(set_by, local_username)| set_by == local_username);
            if set_by_local {
                self.last_advanced_at_seconds = now_seconds;
            }
            let preserves_active_target_after_playlist_change = room_name
                .as_deref()
                .and_then(|room_name| {
                    self.playlist_target_for_room_index(room_name, playlist_index.index)
                        .map(str::to_owned)
                })
                .zip(preserved_active_target.as_deref())
                .is_some_and(|(next_target, previous_target)| next_target == previous_target);

            let should_queue_playlist_reset =
                if !self.should_track_playlist_index_transition_for_room(room_name.as_deref()) {
                    false
                } else if !self.received_first_playlist_index {
                    self.received_first_playlist_index = true;
                    false
                } else if set_by_local && self.suppress_next_self_playlist_index_reset {
                    self.suppress_next_self_playlist_index_reset = false;
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
                let playlist = self.room_playlists.entry(room_name.to_owned()).or_default();
                playlist.index = Some(playlist_index.index);
                if playlist_index.user.is_some() {
                    playlist.set_by = playlist_index.user;
                }
            } else {
                let pending_playlist = self.pending_playlist.get_or_insert_with(Default::default);
                pending_playlist.index = Some(playlist_index.index);
                if playlist_index.user.is_some() {
                    pending_playlist.set_by = playlist_index.user;
                }
            }
            if let Some(room_name) = room_name.as_deref() {
                self.playlist_active_targets_before_index_update
                    .remove(room_name);
            }
        }
    }

    pub(super) fn apply_list(&mut self, list_payload: ListPayload) {
        let ListPayload::Rooms(rooms) = list_payload else {
            return;
        };

        self.user_views.clear();
        self.known_rooms.clear();
        self.domain = SyncDomain::default();

        let mut resolved_self_room = None;
        let current_username = self.username.clone();

        for (room_name, room_users) in rooms {
            self.known_rooms.insert(room_name.clone());
            for (username, user_entry) in room_users {
                if username.trim().is_empty() {
                    continue;
                }
                self.set_user_room(&username, Some(room_name.clone()));
                let (has_file, file_name, file_size, file_duration) =
                    Self::list_payload_file_info(user_entry.file.as_ref());
                self.set_user_file_info(&username, has_file, file_name, file_size, file_duration);
                self.set_user_controller(&username, user_entry.controller.unwrap_or(false));
                self.set_user_ready_state(&username, user_entry.is_ready);
                self.set_user_features(&username, user_entry.features);
                if current_username.as_deref() == Some(username.as_str()) {
                    resolved_self_room = Some(room_name.clone());
                }
            }
        }

        if let Some(resolved_self_room) = resolved_self_room {
            self.update_local_room(resolved_self_room);
        }
    }

    pub(crate) fn apply_state(&mut self, state_payload: StatePayload) {
        self.apply_state_at(state_payload, None);
    }

    pub(super) fn apply_state_at(&mut self, state_payload: StatePayload, now_seconds: Option<f64>) {
        let Some(playstate) = state_payload.playstate else {
            return;
        };

        let room_name = playstate
            .set_by
            .as_deref()
            .and_then(|username| self.user_room(username).map(str::to_owned))
            .or_else(|| self.room.clone());

        if let Some(room_name) = room_name {
            self.merge_room_playstate(
                room_name,
                playstate,
                now_seconds.unwrap_or_else(unix_wall_clock_time_seconds_legacy_compatible),
            );
        }
    }

    pub(super) fn apply_chat(&mut self, chat_payload: ChatPayload) {
        let notification = match chat_payload {
            ChatPayload::Text(message) => ChatNotification::Message {
                username: None,
                message,
            },
            ChatPayload::Message(message_payload) => ChatNotification::Message {
                username: Some(message_payload.username),
                message: message_payload.message,
            },
        };
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

    pub(super) fn feature_bool(features: Option<&Value>, name: &str) -> Option<bool> {
        features
            .and_then(|feature_map| feature_map.get(name))
            .and_then(|value| value.as_bool())
    }

    pub(super) fn feature_usize(features: Option<&Value>, name: &str) -> Option<usize> {
        features
            .and_then(|feature_map| feature_map.get(name))
            .and_then(|value| value.as_u64())
            .and_then(|value| usize::try_from(value).ok())
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

    pub(super) fn meets_min_version_legacy_compatible(version: &str, min_version: &str) -> bool {
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
