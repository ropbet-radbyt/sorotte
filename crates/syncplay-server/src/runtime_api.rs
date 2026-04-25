use super::*;

impl Default for ServerRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerRuntime {
    pub fn new() -> Self {
        Self::with_room_password_salt(DEFAULT_CONTROLLED_ROOM_HASH_SALT)
    }

    pub fn with_room_password_salt(salt: impl Into<String>) -> Self {
        Self {
            domain: SyncDomain::default(),
            sessions: BTreeMap::new(),
            room_controllers: BTreeMap::new(),
            room_playlists: BTreeMap::new(),
            room_playback_states: BTreeMap::new(),
            client_state_counters: BTreeMap::new(),
            client_last_state_update_at: BTreeMap::new(),
            client_next_periodic_state_at: BTreeMap::new(),
            time_now_override_seconds: None,
            room_password_provider: RoomPasswordProvider::new(salt),
            server_password_token: None,
            motd_template: None,
            stats_persistence: None,
            stats_snapshot_start_delay_seconds: legacy_stats_snapshot_start_delay_seconds_for_port(
                0,
            ),
            stats_snapshot_interval_seconds: SERVER_STATS_SNAPSHOT_INTERVAL_SECONDS,
            stats_next_snapshot_at_seconds: None,
            tls_cert_path: None,
            tls_server_config: None,
            tls_context_available: false,
            server_accepts_tls: false,
            tls_last_edit_cert_time: None,
            tls_rotation_attempts: 0,
            pending_transport_actions: Vec::new(),
            persistent_rooms_enabled: false,
            isolate_rooms: false,
            chat_enabled: true,
            readiness_enabled: true,
            max_chat_message_length: DEFAULT_MAX_CHAT_MESSAGE_LENGTH,
            max_username_length: DEFAULT_MAX_USERNAME_LENGTH,
            room_persistence: None,
            permanent_rooms: BTreeSet::new(),
        }
    }

    pub fn with_motd_template(template: impl Into<String>) -> Self {
        let mut runtime = Self::new();
        runtime.set_motd_template(Some(template.into()));
        runtime
    }

    pub fn with_persistent_rooms_enabled(enabled: bool) -> Self {
        let mut runtime = Self::new();
        runtime.set_persistent_rooms_enabled(enabled);
        runtime
    }

    pub fn set_motd_template(&mut self, template: Option<String>) {
        self.motd_template = template.and_then(|template| {
            let trimmed = template.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            }
        });
    }

    pub fn set_server_password_token(&mut self, token: Option<String>) {
        self.server_password_token = token.filter(|token| !token.is_empty());
    }

    pub fn with_stats_db_path(db_path: impl Into<PathBuf>) -> Result<Self, ServerRuntimeError> {
        let mut runtime = Self::new();
        runtime.set_stats_db_path(Some(db_path.into()))?;
        Ok(runtime)
    }

    pub fn with_tls_cert_path(path: impl Into<PathBuf>) -> Self {
        let mut runtime = Self::new();
        runtime.set_tls_cert_path(Some(path.into()));
        runtime
    }

    pub fn set_stats_snapshot_start_delay_seconds(&mut self, delay_seconds: f64) {
        self.stats_snapshot_start_delay_seconds =
            if delay_seconds.is_finite() && delay_seconds >= 0.0 {
                delay_seconds
            } else {
                0.0
            };
        if self.stats_persistence.is_some() {
            self.initialize_stats_snapshot_schedule();
        }
    }

    pub fn set_stats_snapshot_start_delay_for_port(&mut self, port: u16) {
        self.set_stats_snapshot_start_delay_seconds(
            legacy_stats_snapshot_start_delay_seconds_for_port(port),
        );
    }

    pub fn set_stats_snapshot_interval_seconds(&mut self, interval_seconds: f64) {
        self.stats_snapshot_interval_seconds =
            if interval_seconds.is_finite() && interval_seconds > 0.0 {
                interval_seconds
            } else {
                SERVER_STATS_SNAPSHOT_INTERVAL_SECONDS
            };
        if self.stats_persistence.is_some() {
            self.initialize_stats_snapshot_schedule();
        }
    }

    pub fn set_stats_db_path(
        &mut self,
        db_path: Option<PathBuf>,
    ) -> Result<(), ServerRuntimeError> {
        let Some(db_path) = db_path else {
            self.stats_persistence = None;
            self.stats_next_snapshot_at_seconds = None;
            return Ok(());
        };
        let stats_persistence = StatsPersistenceStore::open(&db_path)?;
        self.stats_persistence = Some(stats_persistence);
        self.initialize_stats_snapshot_schedule();
        Ok(())
    }

    pub fn set_tls_cert_path(&mut self, path: Option<PathBuf>) {
        self.tls_cert_path = path;
        self.tls_rotation_attempts = 0;
        self.refresh_tls_context_from_cert_path();
    }

    pub fn set_persistent_rooms_enabled(&mut self, enabled: bool) {
        self.persistent_rooms_enabled = enabled;
    }

    pub fn set_isolate_rooms(&mut self, enabled: bool) {
        self.isolate_rooms = enabled;
    }

    pub fn set_chat_enabled(&mut self, enabled: bool) {
        self.chat_enabled = enabled;
    }

    pub fn set_readiness_enabled(&mut self, enabled: bool) {
        self.readiness_enabled = enabled;
    }

    pub fn set_max_chat_message_length(&mut self, max_chars: usize) {
        self.max_chat_message_length = max_chars;
    }

    pub fn set_max_username_length(&mut self, max_chars: usize) {
        self.max_username_length = max_chars;
    }

    pub fn with_persistent_rooms_db_path(
        db_path: impl Into<PathBuf>,
    ) -> Result<Self, ServerRuntimeError> {
        let mut runtime = Self::new();
        runtime.set_persistent_rooms_db_path(Some(db_path.into()))?;
        runtime.set_persistent_rooms_enabled(true);
        Ok(runtime)
    }

    pub fn with_permanent_rooms_file_path(
        permanent_rooms_file_path: impl Into<PathBuf>,
    ) -> Result<Self, ServerRuntimeError> {
        let mut runtime = Self::new();
        runtime.set_permanent_rooms_file_path(Some(permanent_rooms_file_path.into()))?;
        Ok(runtime)
    }

    pub fn set_persistent_rooms_db_path(
        &mut self,
        db_path: Option<PathBuf>,
    ) -> Result<(), ServerRuntimeError> {
        let Some(db_path) = db_path else {
            self.room_persistence = None;
            return Ok(());
        };
        let persistence = RoomPersistenceStore::open(&db_path)?;
        let persisted_rooms = persistence.load_rooms()?;
        self.room_persistence = Some(persistence);
        self.apply_persisted_rooms_snapshot(persisted_rooms);
        self.apply_permanent_rooms_snapshot();
        Ok(())
    }

    pub fn set_permanent_rooms<I, S>(&mut self, permanent_rooms: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.permanent_rooms = permanent_rooms.into_iter().map(Into::into).collect();
        self.apply_permanent_rooms_snapshot();
    }

    pub fn set_permanent_rooms_file_path(
        &mut self,
        permanent_rooms_file_path: Option<PathBuf>,
    ) -> Result<(), ServerRuntimeError> {
        let Some(path) = permanent_rooms_file_path else {
            self.set_permanent_rooms(Vec::<String>::new());
            return Ok(());
        };
        if !path.is_file() {
            self.set_permanent_rooms(Vec::<String>::new());
            return Ok(());
        }
        let file_contents = fs::read_to_string(&path).map_err(|source| {
            ServerRuntimeError::PermanentRoomsFileRead {
                path: path.clone(),
                source,
            }
        })?;
        self.set_permanent_rooms(parse_permanent_rooms_file(&file_contents));
        Ok(())
    }

    pub fn bootstrap_room(&mut self, room_name: &str) {
        self.domain.join_room("bootstrap", room_name);
    }

    pub fn room_is_present(&self, room_name: &str) -> bool {
        self.domain.users_in_room(room_name).is_some()
    }

    pub fn session(&self, client_id: &str) -> Option<&ServerSession> {
        self.sessions.get(client_id)
    }

    pub fn tls_cert_path(&self) -> Option<PathBuf> {
        self.tls_cert_path.clone()
    }

    pub(crate) fn tls_server_config(&self) -> Option<Arc<ServerConfig>> {
        self.tls_server_config.clone()
    }

    pub fn set_time_now_override_seconds(&mut self, seconds: Option<f64>) {
        self.time_now_override_seconds = seconds;
    }

    pub fn drain_transport_actions(&mut self) -> Vec<DirectedTransportAction> {
        std::mem::take(&mut self.pending_transport_actions)
    }

    pub fn advance_time_and_collect_fanout(
        &mut self,
        delta_seconds: f64,
    ) -> Result<Vec<DirectedOutboundLine>, ServerRuntimeError> {
        let base_now = self.current_time_seconds();
        let advanced_now = if delta_seconds.is_finite() && delta_seconds > 0.0 {
            base_now + delta_seconds
        } else {
            base_now
        };
        self.time_now_override_seconds = Some(advanced_now);
        let outbound_messages = self.collect_due_periodic_updates()?;
        self.collect_due_stats_snapshots()?;
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
}
