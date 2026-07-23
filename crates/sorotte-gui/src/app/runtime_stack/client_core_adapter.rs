use super::*;
use sorotte_secret::SecretValue;

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
    pub(in crate::app) runtime: ClientApplication<GuiNoopClientRuntimePlayer>,
    pub(super) pending_startup_protocol_lines: VecDeque<String>,
    pub(super) next_outbound_protocol_delivery_token: u64,
    pub(super) staged_outbound_protocol_delivery: Option<GuiStagedClientCoreProtocolDelivery>,
    pub(super) next_state_sync_heartbeat_at: Option<Instant>,
    pub(super) next_autoplay_tick_at: Option<Instant>,
    pub(super) pending_attached_player_local_runtime_actions: Vec<GuiAttachedPlayerRuntimeAction>,
    pub(super) playback_transport_adapter_epoch: u64,
    pub(super) last_streaming_quality_suggestion: Option<StreamingQualityDowngradeSuggestion>,
    pub(super) tracked_remote_usernames: BTreeSet<String>,
    pub(super) optimistic_room_playlist: Option<(String, RoomPlaylistView)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GuiClientCoreProtocolDeliverySource {
    Startup,
    Runtime,
}

pub(super) struct GuiStagedClientCoreProtocolDelivery {
    pub(super) token: u64,
    pub(super) line: String,
    pub(super) source: GuiClientCoreProtocolDeliverySource,
    pub(super) core_lease: Option<ProtocolLineLease>,
}

impl GuiClientCoreChatSessionRuntimeAdapter {
    const STATE_SYNC_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);

    fn startup_protocol_delivery_is_staged(&self) -> bool {
        self.staged_outbound_protocol_delivery
            .as_ref()
            .is_some_and(|delivery| delivery.source == GuiClientCoreProtocolDeliverySource::Startup)
    }

    fn replace_pending_startup_hello_if_unstaged(&mut self, line: String) {
        if self.pending_startup_protocol_lines.is_empty()
            || self.startup_protocol_delivery_is_staged()
        {
            return;
        }
        self.pending_startup_protocol_lines.clear();
        self.pending_startup_protocol_lines.push_back(line);
    }

    fn dispatch_command_to_application(
        runtime: &mut ClientApplication<GuiNoopClientRuntimePlayer>,
        command: ClientCommand,
    ) -> Result<bool, String> {
        let events = runtime.dispatch(command);
        if let Some(ClientEvent::OperationFailed { message, .. }) = events
            .iter()
            .find(|event| matches!(event, ClientEvent::OperationFailed { .. }))
        {
            return Err(message.clone());
        }
        Ok(events
            .iter()
            .find_map(ClientEvent::command_changed)
            .unwrap_or(false))
    }

    fn dispatch_application_command(&mut self, command: ClientCommand) -> Result<bool, String> {
        Self::dispatch_command_to_application(&mut self.runtime, command)
    }

    fn application_settings(
        runtime_settings: &StoredClientSettingsRuntimeSnapshot,
        active_room: impl Into<String>,
    ) -> ClientApplicationSettings {
        ClientApplicationSettings::new(runtime_settings.config.clone())
            .with_active_room(active_room)
    }

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
        controlled_room_password_override: Option<SecretValue>,
    ) -> Result<Self, String> {
        let username = username.into();
        let room = room.into();
        let mut runtime_settings = StoredClientSettingsRuntimeSnapshot {
            controlled_room_password_override: controlled_room_password_override.clone(),
            ..StoredClientSettingsRuntimeSnapshot::default()
        };
        runtime_settings.config.connection.username = Username::new(username.clone()).ok();
        runtime_settings.config.connection.room = RoomName::new(room.clone()).ok();
        runtime_settings.config.connection.controlled_room_password =
            controlled_room_password_override;
        let hello_json = Self::hello_json(&username, &room, &runtime_settings, None);
        let mut runtime = ClientApplication::with_default_session(GuiNoopClientRuntimePlayer);
        Self::dispatch_command_to_application(
            &mut runtime,
            ClientCommand::update_settings(Self::application_settings(&runtime_settings, &room)),
        )?;
        Self::dispatch_command_to_application(&mut runtime, ClientCommand::BeginConnecting)?;

        let playback_transport_adapter_epoch = runtime.playback_transport_adapter_epoch();
        Ok(Self {
            username,
            baseline_room: room,
            pending_room_for_next_hello: None,
            dont_slow_down_with_me: false,
            pending_ready_at_start_on_server_hello: false,
            request_user_list_on_first_state_without_media: true,
            runtime_settings,
            runtime,
            pending_startup_protocol_lines: VecDeque::from([hello_json]),
            next_outbound_protocol_delivery_token: 1,
            staged_outbound_protocol_delivery: None,
            next_state_sync_heartbeat_at: None,
            next_autoplay_tick_at: None,
            pending_attached_player_local_runtime_actions: Vec::new(),
            playback_transport_adapter_epoch,
            last_streaming_quality_suggestion: None,
            tracked_remote_usernames: BTreeSet::new(),
            optimistic_room_playlist: None,
        })
    }

    pub(in crate::app) fn apply_runtime_settings_snapshot(
        &mut self,
        runtime_settings: &StoredClientSettingsRuntimeSnapshot,
    ) -> Result<(), String> {
        let session_username_missing = self.runtime.session().username().is_none();
        let next_username = self
            .runtime
            .session()
            .username()
            .map(str::to_owned)
            .unwrap_or_else(|| {
                runtime_settings
                    .config
                    .connection
                    .username
                    .as_ref()
                    .map(|username| username.as_str().to_owned())
                    .unwrap_or_default()
            });
        let next_baseline_room = runtime_settings
            .config
            .connection
            .room
            .as_ref()
            .map(|room| room.as_str().to_owned())
            .unwrap_or_default();
        let active_room = self
            .runtime
            .session()
            .local_room_command_target_with_legacy_fallback(&next_baseline_room);
        Self::dispatch_command_to_application(
            &mut self.runtime,
            ClientCommand::update_settings(Self::application_settings(
                runtime_settings,
                active_room,
            )),
        )?;

        self.runtime_settings = runtime_settings.clone();
        self.username = next_username;
        self.baseline_room = next_baseline_room;
        self.dont_slow_down_with_me = runtime_settings
            .config
            .synchronization
            .dont_slow_down_with_me;
        if session_username_missing {
            self.pending_ready_at_start_on_server_hello = true;
            if !self.pending_startup_protocol_lines.is_empty() {
                let username = self.current_username_for_next_hello();
                let room = self.current_room_for_next_hello();
                let reconnect_token = self
                    .runtime
                    .session()
                    .readiness_reconnect_token_for_room(&room)
                    .map(str::to_owned);
                let hello = Self::hello_json(
                    &username,
                    &room,
                    &self.runtime_settings,
                    reconnect_token.as_deref(),
                );
                self.replace_pending_startup_hello_if_unstaged(hello);
            }
        }
        let (readiness_supported, local_can_control, is_playing_music, recently_advanced) =
            self.autoplay_runtime_flags();
        self.runtime.update_autoplay_check(
            readiness_supported,
            local_can_control,
            is_playing_music,
            recently_advanced,
        );
        Ok(())
    }

    fn client_hello_features_legacy_compatible(
        runtime_settings: &StoredClientSettingsRuntimeSnapshot,
    ) -> Value {
        let mut features = Map::new();
        features.insert(
            "sharedPlaylists".to_owned(),
            Value::Bool(runtime_settings.config.playback.shared_playlist_enabled),
        );
        features.insert("chat".to_owned(), Value::Bool(true));
        features.insert("uiMode".to_owned(), Value::String("GUI".to_owned()));
        features.insert("featureList".to_owned(), Value::Bool(true));
        features.insert("readiness".to_owned(), Value::Bool(true));
        features.insert("managedRooms".to_owned(), Value::Bool(true));
        features.insert("persistentRooms".to_owned(), Value::Bool(true));
        features.insert("setOthersReadiness".to_owned(), Value::Bool(true));
        features.insert("mediaMatch".to_owned(), Value::Bool(true));
        features.insert("sorottePlexPlaylistUris".to_owned(), Value::Bool(true));
        ClientSession::advertise_readiness_v2(&mut features);
        ClientSession::advertise_playback_barrier_v1(&mut features);
        Value::Object(features)
    }

    fn hello_json(
        username: &str,
        room: &str,
        runtime_settings: &StoredClientSettingsRuntimeSnapshot,
        readiness_reconnect_token: Option<&str>,
    ) -> String {
        let mut hello = HelloPayload::new(username, room, SYNCPLAY_WIRE_VERSION_LEGACY)
            .with_realversion(SYNCPLAY_COMPAT_VERSION_LEGACY)
            .with_features(Self::client_hello_features_legacy_compatible(
                runtime_settings,
            ));
        if let Some(server_password) = runtime_settings
            .config
            .connection
            .server_password
            .as_ref()
            .map(|password| password.expose_secret())
            .filter(|value| !value.is_empty())
        {
            hello.extra.insert(
                "password".to_owned(),
                Value::String(legacy_server_password_token(server_password)),
            );
        }
        if let Some(reconnect_token) = readiness_reconnect_token.filter(|token| !token.is_empty()) {
            hello.extra.insert(
                SOROTTE_READINESS_RECONNECT_TOKEN.to_owned(),
                Value::String(reconnect_token.to_owned()),
            );
        }

        encode_message_line(&ProtocolMessage::hello(hello))
            .expect("client-core GUI startup hello should encode")
    }

    fn current_username_for_next_hello(&self) -> String {
        self.runtime
            .session()
            .username()
            .map(str::to_owned)
            .unwrap_or_else(|| {
                self.runtime_settings
                    .config
                    .connection
                    .username
                    .as_ref()
                    .map(|username| username.as_str().to_owned())
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
            .username()
            .or_else(|| (!self.username.is_empty()).then_some(self.username.as_str()))
    }

    pub(super) fn shared_playlist_server_supported(&self) -> bool {
        self.runtime.session().server_shared_playlists_supported()
    }

    pub(super) fn managed_rooms_server_supported(&self) -> bool {
        self.runtime.session().server_managed_rooms_supported()
    }

    pub(super) fn reset_session_for_reconnect(&mut self) -> Result<(), String> {
        let username = self.current_username_for_next_hello();
        self.username = username.clone();
        self.baseline_room = self
            .runtime_settings
            .config
            .connection
            .room
            .as_ref()
            .map(|room| room.as_str().to_owned())
            .unwrap_or_default();
        let room = self.current_room_for_next_hello();
        let reconnect_token = self
            .runtime
            .session()
            .readiness_reconnect_token_for_room(&room)
            .map(str::to_owned);
        let mut runtime = ClientApplication::with_default_session(GuiNoopClientRuntimePlayer);
        Self::dispatch_command_to_application(
            &mut runtime,
            ClientCommand::update_settings(Self::application_settings(
                &self.runtime_settings,
                &room,
            )),
        )?;
        self.runtime = runtime;
        self.staged_outbound_protocol_delivery = None;
        Self::dispatch_command_to_application(
            &mut self.runtime,
            ClientCommand::Reconnect { attempt: 0 },
        )?;
        self.pending_startup_protocol_lines.clear();
        self.pending_startup_protocol_lines
            .push_back(Self::hello_json(
                &username,
                &room,
                &self.runtime_settings,
                reconnect_token.as_deref(),
            ));
        self.next_state_sync_heartbeat_at = None;
        self.next_autoplay_tick_at = None;
        self.pending_attached_player_local_runtime_actions.clear();
        self.pending_ready_at_start_on_server_hello = true;
        self.request_user_list_on_first_state_without_media = true;
        self.tracked_remote_usernames.clear();
        self.optimistic_room_playlist = None;
        Ok(())
    }

    pub(super) fn prepare_transport_reconnect(&mut self) {
        self.staged_outbound_protocol_delivery = None;
        let _ = self
            .runtime
            .dispatch(ClientCommand::Reconnect { attempt: 0 });
        let username = self.current_username_for_next_hello();
        self.username = username.clone();
        self.baseline_room = self
            .runtime_settings
            .settings
            .room
            .clone()
            .unwrap_or_default();
        let room = self.current_room_for_next_hello();
        let reconnect_token = self
            .runtime
            .session()
            .readiness_reconnect_token_for_room(&room)
            .map(str::to_owned);
        self.pending_startup_protocol_lines.clear();
        self.pending_startup_protocol_lines
            .push_back(Self::hello_json(
                &username,
                &room,
                &self.runtime_settings,
                reconnect_token.as_deref(),
            ));
        self.next_state_sync_heartbeat_at = None;
        self.next_autoplay_tick_at = None;
        self.pending_attached_player_local_runtime_actions.clear();
        self.pending_ready_at_start_on_server_hello =
            self.pending_ready_at_start_on_server_hello || !self.server_handshake_completed();
        self.tracked_remote_usernames.clear();
        self.optimistic_room_playlist = None;
    }

    fn current_room_name(&self) -> Option<&str> {
        self.runtime
            .session()
            .room()
            .filter(|value| !value.is_empty())
    }

    fn latest_outbound_room_target_for_next_hello(&self) -> Option<String> {
        self.runtime
            .pending_protocol_messages()
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
        current.files == optimistic.files
            && current.index == optimistic.index
            && current.revision >= optimistic.revision
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
                        SorotteGuiShellAppState::shared_playlist_target_index_from_changed_entries(
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
        let revision = self
            .projected_current_room_playlist()
            .map(|playlist| playlist.revision)
            .unwrap_or(1);
        self.optimistic_room_playlist = Some((
            room_name,
            RoomPlaylistView {
                files,
                index,
                set_by: Some(self.username.clone()),
                revision,
            },
        ));
    }

    fn queue_periodic_state_sync_heartbeat_if_due(&mut self) {
        if !self.runtime.session().is_active() {
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
        let readiness_supported = session.server_readiness_supported();
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

        let paused_before_tick = self.runtime.session().local_paused();
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
        if paused_before_tick != Some(false) && self.runtime.session().local_paused() == Some(false)
        {
            self.pending_attached_player_local_runtime_actions.push(
                GuiAttachedPlayerRuntimeAction::Paused {
                    paused: false,
                    cause: PlayerCommandCause::AutomaticReadinessStart,
                },
            );
        }

        self.next_autoplay_tick_at = if self.runtime.session().autoplay_timer_is_running() {
            Some(now + tick_interval)
        } else {
            None
        };
    }

    pub(in crate::app) fn flush_outbound_protocol_lines(&mut self) -> Result<Vec<String>, String> {
        if let Some(staged) = self.staged_outbound_protocol_delivery.as_ref() {
            return Err(format!(
                "Cannot drain outbound protocol lines while delivery receipt {} is staged.",
                staged.token
            ));
        }
        if !self.pending_startup_protocol_lines.is_empty() && !self.runtime.session().is_active() {
            let _ = Self::dispatch_command_to_application(
                &mut self.runtime,
                ClientCommand::TransportConnected,
            )?;
        }
        let mut lines: Vec<_> = self.pending_startup_protocol_lines.drain(..).collect();
        lines.extend(
            self.runtime
                .flush_queued_protocol_lines()
                .map_err(|error| format!("Queued protocol line encoding failed: {error}"))?,
        );
        Ok(lines)
    }

    pub(in crate::app) fn begin_outbound_protocol_delivery(
        &mut self,
    ) -> Result<Option<GuiOutboundProtocolDelivery>, String> {
        if self.staged_outbound_protocol_delivery.is_some() {
            return Ok(None);
        }

        let (line, source, core_lease) =
            if let Some(line) = self.pending_startup_protocol_lines.front() {
                let line = line.clone();
                if !self.runtime.session().is_active() {
                    let _ = Self::dispatch_command_to_application(
                        &mut self.runtime,
                        ClientCommand::TransportConnected,
                    )?;
                }
                (line, GuiClientCoreProtocolDeliverySource::Startup, None)
            } else {
                if !self.runtime.session().is_active() {
                    return Ok(None);
                }
                let Some(pending) = self
                    .runtime
                    .pending_protocol_line()
                    .map_err(|error| format!("Queued protocol line encoding failed: {error}"))?
                else {
                    return Ok(None);
                };
                let lease = pending.lease();
                (
                    pending.into_line(),
                    GuiClientCoreProtocolDeliverySource::Runtime,
                    Some(lease),
                )
            };

        let token = self.next_outbound_protocol_delivery_token;
        self.next_outbound_protocol_delivery_token = self
            .next_outbound_protocol_delivery_token
            .wrapping_add(1)
            .max(1);
        self.staged_outbound_protocol_delivery = Some(GuiStagedClientCoreProtocolDelivery {
            token,
            line: line.clone(),
            source,
            core_lease,
        });
        Ok(Some(GuiOutboundProtocolDelivery::new(token, line)))
    }

    pub(in crate::app) fn acknowledge_outbound_protocol_delivery(
        &mut self,
        token: u64,
    ) -> Result<(), String> {
        let Some(staged) = self.staged_outbound_protocol_delivery.as_ref() else {
            return Err(format!(
                "Outbound protocol delivery receipt {token} had no staged session line."
            ));
        };
        if staged.token != token {
            return Err(format!(
                "Outbound protocol delivery receipt {token} did not match staged receipt {}.",
                staged.token
            ));
        }

        let staged_source = staged.source;
        let staged_line = staged.line.clone();
        let staged_core_lease = staged.core_lease;

        match staged_source {
            GuiClientCoreProtocolDeliverySource::Startup => {
                if self.pending_startup_protocol_lines.front() != Some(&staged_line) {
                    return Err(
                        "Outbound startup protocol delivery receipt did not match the startup outbox front."
                            .to_owned(),
                    );
                }
                self.pending_startup_protocol_lines.pop_front();
            }
            GuiClientCoreProtocolDeliverySource::Runtime => {
                let Some(core_lease) = staged_core_lease else {
                    return Err(
                        "Outbound protocol delivery receipt did not retain its client-core lease."
                            .to_owned(),
                    );
                };
                if self.runtime.acknowledge_protocol_line(core_lease).is_none() {
                    return Err(
                        "Outbound protocol delivery receipt did not match the client-core outbox lease."
                            .to_owned(),
                    );
                }
            }
        }
        self.staged_outbound_protocol_delivery = None;
        Ok(())
    }

    pub(in crate::app) fn fail_outbound_protocol_delivery(
        &mut self,
        token: u64,
    ) -> Result<(), String> {
        let Some(staged) = self.staged_outbound_protocol_delivery.as_ref() else {
            return Ok(());
        };
        if staged.token != token {
            return Ok(());
        }
        let core_lease = staged.core_lease;
        self.staged_outbound_protocol_delivery = None;
        if let Some(core_lease) = core_lease {
            let _ = self.runtime.release_protocol_line(core_lease);
        }
        Ok(())
    }

    fn apply_protocol_message(
        &mut self,
        message: ProtocolMessage,
        received_at_seconds: f64,
    ) -> Result<(), String> {
        let inbound_is_server_hello = matches!(&message, ProtocolMessage::Hello(_));
        let message_updates_authoritative_local_room =
            self.message_updates_authoritative_local_room(&message);
        let message_updates_authoritative_local_ready =
            self.message_updates_authoritative_local_ready_state(&message);
        let result = match message {
            ProtocolMessage::State(state_message) => {
                let _ = self
                    .runtime
                    .run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible_at(
                        state_message.state,
                        self.dont_slow_down_with_me,
                        received_at_seconds,
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
                self.runtime
                    .emit_effect(ClientEffect::StopReconnect)
                    .map_err(|error| {
                        format!("Client-core stop-reconnect effect failed: {error}")
                    })?;
                Err(format!(
                    "Inbound client-session message apply failed: server error: {}",
                    error_message.error.message
                ))
            }
            other => self
                .runtime
                .session_mut()
                .apply_protocol_message_at(other, received_at_seconds)
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
        self.apply_message_json_at(json_line, system_time_seconds())
    }

    pub(in crate::app) fn apply_message_json_at(
        &mut self,
        json_line: &str,
        received_at_seconds: f64,
    ) -> Result<(), String> {
        let items = decode_message_line_items(json_line)
            .map_err(|error| format!("Inbound client-session message decode failed: {error}"))?;
        for item in items {
            let message = item.message.map_err(|error| {
                format!("Inbound client-session message decode failed: {error}")
            })?;
            self.apply_protocol_message(message, received_at_seconds)?;
        }
        Ok(())
    }
}
