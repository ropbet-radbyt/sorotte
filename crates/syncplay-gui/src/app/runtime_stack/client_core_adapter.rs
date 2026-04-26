use super::*;

mod event_drain;
mod runtime_adapter_impl;

pub(in crate::app) struct GuiClientCoreChatSessionRuntimeAdapter {
    pub(super) username: String,
    pub(super) baseline_room: String,
    pub(super) pending_room_for_next_hello: Option<String>,
    pub(super) dont_slow_down_with_me: bool,
    pub(super) pending_ready_at_start_on_server_hello: bool,
    pub(super) request_user_list_on_first_state_without_media: bool,
    pub(super) runtime_settings: StoredClientSettingsRuntimeSnapshot,
    pub(in crate::app) runtime: ClientRuntime<GuiNoopClientRuntimePlayer, QueuedRuntimeControl>,
    pub(super) pending_startup_protocol_lines: VecDeque<String>,
    pub(super) next_state_sync_heartbeat_at: Option<Instant>,
    pub(super) next_autoplay_tick_at: Option<Instant>,
    pub(super) tracked_remote_usernames: BTreeSet<String>,
    pub(super) optimistic_room_playlist: Option<(String, RoomPlaylistView)>,
}

impl GuiClientCoreChatSessionRuntimeAdapter {
    const STATE_SYNC_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);

    #[cfg(test)]
    pub(in crate::app) fn new(
        username: impl Into<String>,
        room: impl Into<String>,
    ) -> Result<Self, String> {
        Self::new_with_control_password(username, room, None)
    }

    pub(in crate::app) fn new_with_control_password(
        username: impl Into<String>,
        room: impl Into<String>,
        controlled_room_password_override: Option<String>,
    ) -> Result<Self, String> {
        let username = username.into();
        let room = room.into();
        let runtime_settings = StoredClientSettingsRuntimeSnapshot {
            controlled_room_password_override,
            ..StoredClientSettingsRuntimeSnapshot::default()
        };
        let hello_json = Self::hello_json(&username, &room, &runtime_settings);
        let mut session = ClientSession::default();
        Self::apply_runtime_settings_to_session(&mut session, &runtime_settings, &room);

        Ok(Self {
            username,
            baseline_room: room,
            pending_room_for_next_hello: None,
            dont_slow_down_with_me: false,
            pending_ready_at_start_on_server_hello: false,
            request_user_list_on_first_state_without_media: true,
            runtime_settings,
            runtime: ClientRuntime::new(
                session,
                GuiNoopClientRuntimePlayer,
                QueuedRuntimeControl::default(),
            ),
            pending_startup_protocol_lines: VecDeque::from([hello_json]),
            next_state_sync_heartbeat_at: None,
            next_autoplay_tick_at: None,
            tracked_remote_usernames: BTreeSet::new(),
            optimistic_room_playlist: None,
        })
    }

    pub(in crate::app) fn apply_runtime_settings_snapshot(
        &mut self,
        runtime_settings: &StoredClientSettingsRuntimeSnapshot,
    ) {
        self.runtime_settings = runtime_settings.clone();
        self.username = self.runtime.session().username.clone().unwrap_or_else(|| {
            self.runtime_settings
                .settings
                .username
                .clone()
                .unwrap_or_default()
        });
        self.baseline_room = runtime_settings.settings.room.clone().unwrap_or_default();
        if self.runtime.session().username.is_none() {
            self.pending_ready_at_start_on_server_hello = true;
            if !self.pending_startup_protocol_lines.is_empty() {
                let username = self.current_username_for_next_hello();
                let room = self.current_room_for_next_hello();
                self.pending_startup_protocol_lines.clear();
                self.pending_startup_protocol_lines
                    .push_back(Self::hello_json(&username, &room, &self.runtime_settings));
            }
        }
        self.dont_slow_down_with_me = runtime_settings
            .settings
            .dont_slow_down_with_me
            .unwrap_or(false);
        let room = self.current_room_for_runtime_settings_sync();
        Self::apply_runtime_settings_to_session(
            self.runtime.session_mut(),
            &self.runtime_settings,
            &room,
        );
        let (readiness_supported, local_can_control, is_playing_music, recently_advanced) =
            self.autoplay_runtime_flags();
        self.runtime.update_autoplay_check(
            readiness_supported,
            local_can_control,
            is_playing_music,
            recently_advanced,
        );
    }

    fn apply_runtime_settings_to_session(
        session: &mut ClientSession,
        runtime_settings: &StoredClientSettingsRuntimeSnapshot,
        room: &str,
    ) {
        let behavior_defaults = SessionBehaviorConfig::default();
        let desync_defaults = DesyncCorrectionConfig::default();
        let readiness_defaults = ReadinessAutoplayConfig::default();

        if let Some(control_password) = runtime_settings
            .controlled_room_password_override
            .as_deref()
        {
            session.remember_control_password_for_room(room, control_password);
        }
        if let Some(autoplay_enabled) = runtime_settings.settings.autoplay_initial_state {
            session.set_autoplay_enabled(autoplay_enabled);
        }
        {
            let behavior_config = session.behavior_config_mut();
            behavior_config.show_same_room_osd = runtime_settings
                .settings
                .show_same_room_osd
                .unwrap_or(behavior_defaults.show_same_room_osd);
            behavior_config.show_osd_warnings = runtime_settings
                .settings
                .show_osd_warnings
                .unwrap_or(behavior_defaults.show_osd_warnings);
            behavior_config.show_noncontroller_osd = runtime_settings
                .settings
                .show_noncontroller_osd
                .unwrap_or(behavior_defaults.show_noncontroller_osd);
            behavior_config.show_different_room_osd = runtime_settings
                .settings
                .show_different_room_osd
                .unwrap_or(behavior_defaults.show_different_room_osd);
            behavior_config.pause_on_leave = runtime_settings
                .settings
                .pause_on_leave
                .unwrap_or(behavior_defaults.pause_on_leave);
            behavior_config.loop_at_end_of_playlist = runtime_settings
                .settings
                .loop_at_end_of_playlist
                .unwrap_or(behavior_defaults.loop_at_end_of_playlist);
            behavior_config.loop_single_files = runtime_settings
                .settings
                .loop_single_files
                .unwrap_or(behavior_defaults.loop_single_files);
            behavior_config.only_switch_to_trusted_domains = runtime_settings
                .settings
                .only_switch_to_trusted_domains
                .unwrap_or(behavior_defaults.only_switch_to_trusted_domains);
            behavior_config.trusted_domains = runtime_settings
                .settings
                .trusted_domains
                .clone()
                .unwrap_or_else(|| behavior_defaults.trusted_domains.clone());
        }
        {
            let desync_config = session.desync_config_mut();
            desync_config.rewind_on_desync = runtime_settings
                .settings
                .rewind_on_desync
                .unwrap_or(desync_defaults.rewind_on_desync);
            desync_config.fastforward_on_desync = runtime_settings
                .settings
                .fastforward_on_desync
                .unwrap_or(desync_defaults.fastforward_on_desync);
            desync_config.slow_on_desync = runtime_settings
                .settings
                .slow_on_desync
                .unwrap_or(desync_defaults.slow_on_desync);
            desync_config.rewind_threshold_seconds = runtime_settings
                .settings
                .rewind_threshold_seconds
                .unwrap_or(desync_defaults.rewind_threshold_seconds);
            desync_config.fastforward_threshold_seconds = runtime_settings
                .settings
                .fastforward_threshold_seconds
                .unwrap_or(desync_defaults.fastforward_threshold_seconds);
            desync_config.slowdown_threshold_seconds = runtime_settings
                .settings
                .slowdown_threshold_seconds
                .unwrap_or(desync_defaults.slowdown_threshold_seconds);
        }
        {
            let readiness_config = session.readiness_autoplay_config_mut();
            readiness_config.autoplay_require_same_filenames = runtime_settings
                .settings
                .autoplay_require_same_filenames
                .unwrap_or(readiness_defaults.autoplay_require_same_filenames);
            readiness_config.unpause_action = runtime_settings
                .settings
                .unpause_action
                .clone()
                .unwrap_or(readiness_defaults.unpause_action);
            readiness_config.auto_play_threshold = runtime_settings
                .settings
                .autoplay_min_users
                .as_ref()
                .map(|auto_play_threshold| match auto_play_threshold {
                    AutoplayThresholdOverride::Disable => None,
                    AutoplayThresholdOverride::Set(value) => Some(*value),
                })
                .unwrap_or(readiness_defaults.auto_play_threshold);
            readiness_config.show_duration_notification = runtime_settings
                .settings
                .show_duration_notification
                .unwrap_or(readiness_defaults.show_duration_notification);
        }
    }

    fn client_hello_features_legacy_compatible(
        runtime_settings: &StoredClientSettingsRuntimeSnapshot,
    ) -> Value {
        let mut features = Map::new();
        features.insert(
            "sharedPlaylists".to_owned(),
            Value::Bool(
                runtime_settings
                    .settings
                    .shared_playlist_enabled
                    .unwrap_or(true),
            ),
        );
        features.insert("chat".to_owned(), Value::Bool(true));
        features.insert("uiMode".to_owned(), Value::String("GUI".to_owned()));
        features.insert("featureList".to_owned(), Value::Bool(true));
        features.insert("readiness".to_owned(), Value::Bool(true));
        features.insert("managedRooms".to_owned(), Value::Bool(true));
        features.insert("persistentRooms".to_owned(), Value::Bool(true));
        features.insert("setOthersReadiness".to_owned(), Value::Bool(true));
        Value::Object(features)
    }

    fn hello_json(
        username: &str,
        room: &str,
        runtime_settings: &StoredClientSettingsRuntimeSnapshot,
    ) -> String {
        let mut hello = HelloPayload::new(username, room, SYNCPLAY_WIRE_VERSION_LEGACY)
            .with_realversion(SYNCPLAY_COMPAT_VERSION_LEGACY)
            .with_features(Self::client_hello_features_legacy_compatible(
                runtime_settings,
            ));
        if let Some(server_password) = runtime_settings
            .settings
            .server_password
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            hello.extra.insert(
                "password".to_owned(),
                Value::String(legacy_server_password_token(server_password)),
            );
        }

        encode_message_line(&ProtocolMessage::hello(hello))
            .expect("client-core GUI startup hello should encode")
    }

    fn current_username_for_next_hello(&self) -> String {
        self.runtime.session().username.clone().unwrap_or_else(|| {
            self.runtime_settings
                .settings
                .username
                .clone()
                .unwrap_or_else(|| self.username.clone())
        })
    }

    fn current_room_for_next_hello(&self) -> String {
        self.pending_room_for_next_hello
            .as_deref()
            .or_else(|| self.current_room_name())
            .map(str::to_owned)
            .unwrap_or_else(|| self.baseline_room.clone())
    }

    fn local_username_for_authoritative_updates(&self) -> Option<&str> {
        self.runtime
            .session()
            .username
            .as_deref()
            .or_else(|| (!self.username.is_empty()).then_some(self.username.as_str()))
    }

    fn current_room_for_runtime_settings_sync(&self) -> String {
        self.runtime
            .session()
            .local_room_command_target_with_legacy_fallback(&self.baseline_room)
    }

    pub(super) fn shared_playlist_server_supported(&self) -> bool {
        self.runtime.session().server_shared_playlists_supported() != Some(false)
    }

    pub(super) fn managed_rooms_server_supported(&self) -> bool {
        self.runtime.session().server_managed_rooms_supported() == Some(true)
    }

    pub(super) fn reset_session_for_reconnect(&mut self) {
        let username = self.current_username_for_next_hello();
        self.username = username.clone();
        self.baseline_room = self
            .runtime_settings
            .settings
            .room
            .clone()
            .unwrap_or_default();
        let room = self.current_room_for_next_hello();
        let mut session = ClientSession::default();
        Self::apply_runtime_settings_to_session(&mut session, &self.runtime_settings, &room);
        self.runtime = ClientRuntime::new(
            session,
            GuiNoopClientRuntimePlayer,
            QueuedRuntimeControl::default(),
        );
        self.pending_startup_protocol_lines.clear();
        self.pending_startup_protocol_lines
            .push_back(Self::hello_json(&username, &room, &self.runtime_settings));
        self.next_state_sync_heartbeat_at = None;
        self.next_autoplay_tick_at = None;
        self.pending_ready_at_start_on_server_hello = true;
        self.request_user_list_on_first_state_without_media = true;
        self.tracked_remote_usernames.clear();
        self.optimistic_room_playlist = None;
    }

    pub(super) fn prepare_transport_reconnect(&mut self) {
        let username = self.current_username_for_next_hello();
        self.username = username.clone();
        self.baseline_room = self
            .runtime_settings
            .settings
            .room
            .clone()
            .unwrap_or_default();
        let room = self.current_room_for_next_hello();
        self.pending_startup_protocol_lines.clear();
        self.pending_startup_protocol_lines
            .push_back(Self::hello_json(&username, &room, &self.runtime_settings));
        self.next_state_sync_heartbeat_at = None;
        self.next_autoplay_tick_at = None;
        self.pending_ready_at_start_on_server_hello =
            self.pending_ready_at_start_on_server_hello || !self.server_handshake_completed();
        self.tracked_remote_usernames.clear();
        self.optimistic_room_playlist = None;
    }

    fn current_room_name(&self) -> Option<&str> {
        self.runtime
            .session()
            .room
            .as_deref()
            .filter(|value| !value.is_empty())
    }

    fn latest_outbound_room_target_for_next_hello(&self) -> Option<String> {
        self.runtime
            .control()
            .outbound_messages()
            .iter()
            .rev()
            .find_map(|message| match message {
                ProtocolMessage::Set(set_message) => {
                    set_message.set.room.as_ref().map(|room| room.name.clone())
                }
                _ => None,
            })
    }

    fn message_updates_authoritative_local_room(&self, message: &ProtocolMessage) -> bool {
        let Some(local_username) = self.local_username_for_authoritative_updates() else {
            return matches!(message, ProtocolMessage::Hello(_));
        };
        match message {
            ProtocolMessage::Hello(_) => true,
            ProtocolMessage::Set(set_message) => {
                set_message.set.room.is_some()
                    || set_message
                        .set
                        .user
                        .as_ref()
                        .and_then(|users| users.get(local_username))
                        .and_then(|user| user.room.as_ref())
                        .is_some()
            }
            _ => false,
        }
    }

    fn message_updates_authoritative_local_ready_state(&self, message: &ProtocolMessage) -> bool {
        let Some(local_username) = self.local_username_for_authoritative_updates() else {
            return false;
        };
        match message {
            ProtocolMessage::Set(set_message) => {
                set_message.set.ready.as_ref().is_some_and(|ready| {
                    ready
                        .username
                        .as_deref()
                        .or(ready.set_by.as_deref())
                        .unwrap_or(local_username)
                        == local_username
                }) || set_message
                    .set
                    .user
                    .as_ref()
                    .and_then(|users| users.get(local_username))
                    .and_then(|user| user.is_ready)
                    .is_some()
            }
            ProtocolMessage::List(list_message) => match &list_message.list {
                ListPayload::Rooms(rooms) => rooms.values().any(|users| {
                    users
                        .get(local_username)
                        .and_then(|user| user.is_ready)
                        .is_some()
                }),
                ListPayload::Request(_) => false,
            },
            _ => false,
        }
    }

    fn sync_pending_room_for_next_hello_from_session(
        &mut self,
        message_updates_authoritative_local_room: bool,
    ) {
        if message_updates_authoritative_local_room
            || self.pending_room_for_next_hello.as_deref() == self.current_room_name()
        {
            self.pending_room_for_next_hello = None;
        }
    }

    fn room_playlist_matches_projection_target(
        current: &RoomPlaylistView,
        optimistic: &RoomPlaylistView,
    ) -> bool {
        current.files == optimistic.files && current.index == optimistic.index
    }

    pub(in crate::app) fn projected_current_room_playlist(&self) -> Option<&RoomPlaylistView> {
        let current_room = self.current_room_name();
        let optimistic_playlist =
            self.optimistic_room_playlist
                .as_ref()
                .and_then(|(room_name, playlist)| {
                    (Some(room_name.as_str()) == current_room).then_some(playlist)
                });
        let session_playlist = self.runtime.session().current_room_playlist();

        match (optimistic_playlist, session_playlist) {
            (Some(optimistic), Some(current))
                if !Self::room_playlist_matches_projection_target(current, optimistic) =>
            {
                Some(optimistic)
            }
            (Some(_), Some(current)) => Some(current),
            (Some(optimistic), None) => Some(optimistic),
            (None, Some(current)) => Some(current),
            (None, None) => None,
        }
    }

    fn projected_current_room_playlist_contains_entry(&self, entry: &str) -> bool {
        self.projected_current_room_playlist()
            .is_some_and(|playlist| playlist.files.iter().any(|file| file == entry))
    }

    fn sync_optimistic_room_playlist(&mut self) {
        let current_room = self.current_room_name();
        let should_clear = match self.optimistic_room_playlist.as_ref() {
            Some((room_name, _)) if Some(room_name.as_str()) != current_room => true,
            Some((_, optimistic)) => {
                self.runtime
                    .session()
                    .current_room_playlist()
                    .is_some_and(|current| {
                        Self::room_playlist_matches_projection_target(current, optimistic)
                    })
            }
            None => false,
        };
        if should_clear {
            self.optimistic_room_playlist = None;
        }
    }

    fn set_optimistic_current_room_playlist(
        &mut self,
        files: Vec<String>,
        selected_index: Option<usize>,
    ) {
        let Some(room_name) = self.current_room_name().map(str::to_owned) else {
            self.optimistic_room_playlist = None;
            return;
        };

        let index = if files.is_empty() {
            None
        } else {
            selected_index
                .filter(|index| *index < files.len())
                .or_else(|| {
                    self.projected_current_room_playlist().map(|playlist| {
                        let current_index =
                            playlist.index.and_then(|index| usize::try_from(index).ok());
                        SyncplayGuiShellAppState::shared_playlist_target_index_from_changed_entries(
                            &playlist.files,
                            current_index,
                            &files,
                        )
                        .min(files.len().saturating_sub(1))
                    })
                })
                .and_then(|index| i64::try_from(index).ok())
                .or(Some(0))
        };
        self.optimistic_room_playlist = Some((
            room_name,
            RoomPlaylistView {
                files,
                index,
                set_by: Some(self.username.clone()),
            },
        ));
    }

    fn queue_periodic_state_sync_heartbeat_if_due(&mut self) {
        if self.runtime.session().server_chat_supported().is_none() {
            self.next_state_sync_heartbeat_at = None;
            return;
        }

        let now = Instant::now();
        let Some(next_heartbeat_at) = self.next_state_sync_heartbeat_at else {
            self.next_state_sync_heartbeat_at = Some(now + Self::STATE_SYNC_HEARTBEAT_INTERVAL);
            return;
        };
        if now < next_heartbeat_at {
            return;
        }

        let _ = self
            .runtime
            .run_state_sync_heartbeat_legacy_ping_compatible(self.dont_slow_down_with_me);
        self.next_state_sync_heartbeat_at = Some(now + Self::STATE_SYNC_HEARTBEAT_INTERVAL);
    }

    fn autoplay_runtime_flags(&self) -> (bool, bool, bool, bool) {
        let session = self.runtime.session();
        let readiness_supported = session.server_readiness_supported().unwrap_or(false);
        let local_can_control = session.local_can_control().unwrap_or(false);
        let is_playing_music = session.is_playing_music();
        let recently_advanced = session.recently_advanced(system_time_seconds());
        (
            readiness_supported,
            local_can_control,
            is_playing_music,
            recently_advanced,
        )
    }

    fn sync_autoplay_runtime(&mut self, actions: &mut Vec<GuiShellAction>) {
        let (readiness_supported, local_can_control, is_playing_music, recently_advanced) =
            self.autoplay_runtime_flags();
        self.runtime.update_autoplay_check(
            readiness_supported,
            local_can_control,
            is_playing_music,
            recently_advanced,
        );

        if !self.runtime.session().autoplay_timer_is_running() {
            self.next_autoplay_tick_at = None;
            return;
        }

        let tick_interval =
            Duration::from_secs_f64(AUTOPLAY_TICK_INTERVAL_SECONDS.max(f64::EPSILON));
        let now = Instant::now();
        let Some(next_autoplay_tick_at) = self.next_autoplay_tick_at else {
            self.next_autoplay_tick_at = Some(now + tick_interval);
            return;
        };
        if now < next_autoplay_tick_at {
            return;
        }

        if let Err(error) = self.runtime.tick_autoplay(
            readiness_supported,
            local_can_control,
            is_playing_music,
            recently_advanced,
        ) {
            actions.push(GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Error,
                message: format!("Client-core autoplay dispatch failed: {error}"),
            });
            self.next_autoplay_tick_at = None;
            return;
        }

        self.next_autoplay_tick_at = if self.runtime.session().autoplay_timer_is_running() {
            Some(now + tick_interval)
        } else {
            None
        };
    }

    pub(in crate::app) fn flush_outbound_protocol_lines(&mut self) -> Result<Vec<String>, String> {
        let mut lines: Vec<_> = self.pending_startup_protocol_lines.drain(..).collect();
        lines.extend(
            self.runtime
                .flush_queued_protocol_lines()
                .map_err(|error| format!("Queued protocol line encoding failed: {error}"))?,
        );
        Ok(lines)
    }

    fn apply_protocol_message(&mut self, message: ProtocolMessage) -> Result<(), String> {
        let inbound_is_server_hello = matches!(&message, ProtocolMessage::Hello(_));
        let message_updates_authoritative_local_room =
            self.message_updates_authoritative_local_room(&message);
        let message_updates_authoritative_local_ready =
            self.message_updates_authoritative_local_ready_state(&message);
        let result = match message {
            ProtocolMessage::State(state_message) => {
                let _ = self
                    .runtime
                    .run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible(
                        state_message.state,
                        self.dont_slow_down_with_me,
                    );
                if self.request_user_list_on_first_state_without_media {
                    self.request_user_list_on_first_state_without_media = false;
                    let _ = self.runtime.run_request_user_list().map_err(|error| {
                        format!(
                            "Client-core user-list request dispatch failed after first state: {error}"
                        )
                    })?;
                }
                Ok(())
            }
            ProtocolMessage::Error(error_message) => {
                self.runtime.control_mut().stop_reconnect();
                Err(format!(
                    "Inbound client-session message apply failed: server error: {}",
                    error_message.error.message
                ))
            }
            other => self
                .runtime
                .session_mut()
                .apply_protocol_message(other)
                .map_err(|error| format!("Inbound client-session message apply failed: {error}")),
        };
        if result.is_ok() && inbound_is_server_hello && self.pending_ready_at_start_on_server_hello
        {
            let ready_at_start = self
                .runtime_settings
                .settings
                .ready_at_start
                .unwrap_or(false);
            let _ = self
                .runtime
                .run_set_ready_for_user("", ready_at_start, false)
                .map_err(|error| {
                    format!(
                        "Client-core ready-at-start dispatch failed after server Hello: {error}"
                    )
                })?;
        }
        if result.is_ok() {
            self.username = self.current_username_for_next_hello();
            if message_updates_authoritative_local_ready {
                self.pending_ready_at_start_on_server_hello = false;
            }
            self.sync_pending_room_for_next_hello_from_session(
                message_updates_authoritative_local_room,
            );
        }
        self.sync_optimistic_room_playlist();
        result
    }

    pub(in crate::app) fn apply_message_json(&mut self, json_line: &str) -> Result<(), String> {
        let messages = decode_message_lines(json_line)
            .map_err(|error| format!("Inbound client-session message decode failed: {error}"))?;
        for message in messages {
            self.apply_protocol_message(message)?;
        }
        Ok(())
    }
}
