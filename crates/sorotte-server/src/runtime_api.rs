use super::*;

impl Default for ServerRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerRuntime {
    pub fn new() -> Self {
        Self::with_room_password_salt(generate_server_salt_legacy_compatible())
    }

    pub fn with_room_password_salt(salt: impl Into<SecretValue>) -> Self {
        let (persistence_events, _) = broadcast::channel(SERVER_PERSISTENCE_EVENT_CAPACITY);
        let mut persistent_room_quota_secret = [0_u8; 32];
        getrandom::fill(&mut persistent_room_quota_secret)
            .expect("operating system random source should be available");
        Self {
            domain: SyncDomain::default(),
            sessions: BTreeMap::new(),
            room_controllers: BTreeMap::new(),
            room_playlists: BTreeMap::new(),
            room_playback_states: BTreeMap::new(),
            room_playback_barriers: BTreeMap::new(),
            room_buffering_controls: BTreeMap::new(),
            room_readiness: BTreeMap::new(),
            readiness_reconnect_cache: BTreeMap::new(),
            readiness_reconnect_identity_by_client: BTreeMap::new(),
            mixed_readiness_policy: MixedReadinessPolicy::default(),
            pending_user_transport_by_client: BTreeMap::new(),
            next_readiness_membership_epoch: 1,
            playback_barrier_fenced_clients: BTreeSet::new(),
            playback_barrier_request_tombstones: BTreeMap::new(),
            playback_barrier_request_tombstone_policy:
                PlaybackBarrierRequestTombstonePolicy::default(),
            playback_barrier_request_clock_started_at: Instant::now(),
            #[cfg(test)]
            playback_barrier_request_clock_override_seconds: None,
            playback_barrier_new_identity_rate_policy:
                PlaybackBarrierNewIdentityRatePolicy::default(),
            playback_barrier_new_identity_rate_by_client: BTreeMap::new(),
            playback_barrier_new_identity_rate_by_room: BTreeMap::new(),
            playback_barrier_request_nonces: BTreeMap::new(),
            next_playback_barrier_generation: 0,
            next_playback_barrier_revision: 0,
            client_playback_states: BTreeMap::new(),
            client_room_join_sequence: BTreeMap::new(),
            next_room_join_sequence: 0,
            client_state_counters: BTreeMap::new(),
            client_last_state_update_at: BTreeMap::new(),
            client_next_periodic_state_at: BTreeMap::new(),
            client_peer_ips: BTreeMap::new(),
            time_now_override_seconds: None,
            room_password_provider: RoomPasswordProvider::new(salt.into()),
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
            tls_certificate_bundle_fingerprint: None,
            #[cfg(test)]
            tls_certificate_bundle_metadata_clock: None,
            tls_rotation_attempts: 0,
            pending_transport_actions: Vec::new(),
            persistent_rooms_enabled: false,
            max_persistent_rooms: DEFAULT_MAX_PERSISTENT_ROOMS,
            max_persistent_rooms_per_identity: DEFAULT_MAX_PERSISTENT_ROOMS_PER_IDENTITY,
            persistent_room_creation_cooldown_seconds:
                DEFAULT_PERSISTENT_ROOM_CREATION_COOLDOWN_SECONDS,
            persistent_room_inactivity_expiry_seconds:
                DEFAULT_PERSISTENT_ROOM_INACTIVITY_EXPIRY_SECONDS,
            persistent_room_owner_by_room: BTreeMap::new(),
            persistent_room_created_at_by_room: BTreeMap::new(),
            persistent_room_last_creation_by_identity: BTreeMap::new(),
            persistent_room_last_activity_at: BTreeMap::new(),
            persistent_room_quota_secret,
            isolate_rooms: false,
            chat_enabled: true,
            readiness_enabled: true,
            max_chat_message_length: DEFAULT_MAX_CHAT_MESSAGE_LENGTH,
            max_username_length: DEFAULT_MAX_USERNAME_LENGTH,
            room_persistence: None,
            persisted_room_names: BTreeSet::new(),
            next_room_persistence_version: 0,
            persistence_events,
            persistence_degraded_worker_count: Arc::new(AtomicUsize::new(0)),
            permanent_rooms: BTreeSet::new(),
            pending_compatibility_fallbacks: Vec::new(),
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
        self.motd_template = template.filter(|template| !template.trim().is_empty());
    }

    pub fn set_server_password_token(&mut self, token: Option<SecretValue>) {
        self.server_password_token = token.filter(|token| !token.is_empty());
    }

    pub fn set_mixed_readiness_policy(&mut self, policy: MixedReadinessPolicy) {
        self.mixed_readiness_policy = policy;
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
        // Reconfiguration is an explicit startup/control-plane boundary: finish
        // the old worker before opening or loading a replacement database.
        drop(self.stats_persistence.take());
        let store = StatsPersistenceStore::open(&db_path)?;
        self.stats_persistence = Some(StatsPersistenceService::start(
            store,
            self.persistence_events.clone(),
            self.persistence_degraded_worker_count.clone(),
        )?);
        self.initialize_stats_snapshot_schedule();
        Ok(())
    }

    pub fn set_tls_cert_path(&mut self, path: Option<PathBuf>) {
        self.tls_cert_path = path;
        self.tls_rotation_attempts = 0;
        self.refresh_tls_context_from_cert_path();
    }

    #[cfg(test)]
    pub(crate) fn set_tls_certificate_bundle_metadata_clock_for_test(
        &mut self,
        clock: TlsCertificateBundleMetadataClock,
    ) {
        self.tls_certificate_bundle_metadata_clock = Some(clock);
    }

    pub fn set_persistent_rooms_enabled(&mut self, enabled: bool) {
        self.persistent_rooms_enabled = enabled;
    }

    pub fn set_max_persistent_rooms(&mut self, max_rooms: usize) {
        self.max_persistent_rooms = max_rooms;
    }

    pub fn set_max_persistent_rooms_per_identity(&mut self, max_rooms: usize) {
        self.max_persistent_rooms_per_identity = max_rooms;
    }

    pub fn set_persistent_room_creation_cooldown_seconds(&mut self, cooldown_seconds: f64) {
        self.persistent_room_creation_cooldown_seconds =
            if cooldown_seconds.is_finite() && cooldown_seconds >= 0.0 {
                cooldown_seconds
            } else {
                DEFAULT_PERSISTENT_ROOM_CREATION_COOLDOWN_SECONDS
            };
    }

    pub fn set_persistent_room_inactivity_expiry_seconds(&mut self, expiry_seconds: f64) {
        self.persistent_room_inactivity_expiry_seconds =
            if expiry_seconds.is_finite() && expiry_seconds >= 0.0 {
                expiry_seconds
            } else {
                DEFAULT_PERSISTENT_ROOM_INACTIVITY_EXPIRY_SECONDS
            };
    }

    pub fn set_isolate_rooms(&mut self, enabled: bool) {
        self.isolate_rooms = enabled;
    }

    pub fn set_chat_enabled(&mut self, enabled: bool) {
        self.chat_enabled = enabled;
    }

    pub fn set_readiness_enabled(&mut self, enabled: bool) {
        self.readiness_enabled = enabled;
        if !enabled {
            self.pending_user_transport_by_client.clear();
        }
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
            self.persisted_room_names.clear();
            self.next_room_persistence_version = 0;
            return Ok(());
        };
        if self
            .room_persistence
            .as_ref()
            .is_some_and(|persistence| !persistence.flush())
        {
            return Err(ServerRuntimeError::PersistenceWorkerUnavailable("room"));
        }

        // Prepare the complete replacement before touching the working
        // service. A bad path or worker-start failure must leave the current
        // durable store attached.
        let store = RoomPersistenceStore::open(&db_path)?;
        let persistent_room_quota_secret = store.load_or_create_quota_secret()?;
        let persisted_rooms = store.load_rooms()?;
        let affected_rooms = self
            .persisted_room_names
            .iter()
            .chain(persisted_rooms.keys())
            .cloned()
            .collect::<BTreeSet<_>>();
        if let Some(occupied_room) = affected_rooms
            .iter()
            .find(|room_name| !self.clients_in_room(room_name).is_empty())
        {
            return Err(
                ServerRuntimeError::PersistentRoomDatabaseReconfigurationBusy(
                    occupied_room.clone(),
                ),
            );
        }
        let replacement_service = RoomPersistenceService::start(
            store,
            self.persistence_events.clone(),
            self.persistence_degraded_worker_count.clone(),
        )?;

        self.persistent_room_quota_secret = persistent_room_quota_secret;
        self.room_persistence = Some(replacement_service);
        self.replace_persisted_rooms_snapshot(persisted_rooms);
        self.apply_permanent_rooms_snapshot();
        Ok(())
    }

    pub fn set_permanent_rooms<I, S>(&mut self, permanent_rooms: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let previous_permanent_rooms = std::mem::replace(
            &mut self.permanent_rooms,
            permanent_rooms.into_iter().map(Into::into).collect(),
        );
        self.reconcile_permanent_rooms_snapshot(&previous_permanent_rooms);
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

    pub fn drain_compatibility_fallbacks(&mut self) -> Vec<ServerCompatibilityFallback> {
        std::mem::take(&mut self.pending_compatibility_fallbacks)
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

    #[cfg(test)]
    pub(crate) fn set_playback_barrier_request_tombstone_policy_for_tests(
        &mut self,
        ttl_seconds: f64,
        max_per_room: usize,
        max_global: usize,
    ) {
        self.playback_barrier_request_tombstone_policy = PlaybackBarrierRequestTombstonePolicy {
            ttl_seconds: if ttl_seconds.is_finite() && ttl_seconds > 0.0 {
                ttl_seconds
            } else {
                PLAYBACK_BARRIER_REQUEST_TOMBSTONE_TTL_SECONDS
            },
            max_per_room: max_per_room.max(1),
            max_global: max_global.max(1),
        };
    }

    #[cfg(test)]
    pub(crate) fn set_playback_barrier_request_clock_for_tests(&mut self, seconds: f64) {
        self.playback_barrier_request_clock_override_seconds = Some(if seconds.is_finite() {
            seconds.max(0.0)
        } else {
            0.0
        });
    }

    #[cfg(test)]
    pub(crate) fn set_playback_barrier_new_identity_rate_policy_for_tests(
        &mut self,
        window_seconds: f64,
        max_per_client: usize,
        max_per_room: usize,
    ) {
        self.playback_barrier_new_identity_rate_policy = PlaybackBarrierNewIdentityRatePolicy {
            window_seconds: if window_seconds.is_finite() && window_seconds > 0.0 {
                window_seconds
            } else {
                PLAYBACK_BARRIER_NEW_IDENTITY_RATE_WINDOW_SECONDS
            },
            max_per_client: max_per_client.max(1),
            max_per_room: max_per_room.max(1),
        };
        self.playback_barrier_new_identity_rate_by_client.clear();
        self.playback_barrier_new_identity_rate_by_room.clear();
    }

    pub fn subscribe_persistence_events(&self) -> broadcast::Receiver<ServerPersistenceEvent> {
        self.persistence_events.subscribe()
    }

    pub fn persistence_is_degraded(&self) -> bool {
        self.persistence_degraded_worker_count
            .load(Ordering::Acquire)
            > 0
    }

    /// Explicit durability barrier for shutdown coordination and tests. Model
    /// transitions enqueue persistence effects without waiting on this boundary.
    pub fn flush_persistence(&self) -> Result<(), ServerRuntimeError> {
        if self
            .room_persistence
            .as_ref()
            .is_some_and(|persistence| !persistence.flush())
        {
            return Err(ServerRuntimeError::PersistenceWorkerUnavailable("room"));
        }
        if self
            .stats_persistence
            .as_ref()
            .is_some_and(|persistence| !persistence.flush())
        {
            return Err(ServerRuntimeError::PersistenceWorkerUnavailable("stats"));
        }
        Ok(())
    }

    pub fn drain_transport_actions(&mut self) -> Vec<DirectedTransportAction> {
        std::mem::take(&mut self.pending_transport_actions)
    }

    pub fn advance_time_and_collect_fanout(
        &mut self,
        delta_seconds: f64,
    ) -> Result<Vec<DirectedOutboundLine>, ServerRuntimeError> {
        Ok(self
            .advance_time_and_collect_dispatch(delta_seconds)?
            .outbound_lines)
    }

    pub fn advance_time_and_collect_dispatch(
        &mut self,
        delta_seconds: f64,
    ) -> Result<ServerRuntimeDispatch, ServerRuntimeError> {
        let base_now = self.current_time_seconds();
        let advanced_now = if delta_seconds.is_finite() && delta_seconds > 0.0 {
            base_now + delta_seconds
        } else {
            base_now
        };
        self.advance_time_to_and_collect_dispatch(advanced_now)
    }

    pub fn advance_time_to_and_collect_dispatch(
        &mut self,
        now_seconds: f64,
    ) -> Result<ServerRuntimeDispatch, ServerRuntimeError> {
        let now_seconds = if now_seconds.is_finite() {
            now_seconds
        } else {
            self.current_time_seconds()
        };
        self.time_now_override_seconds = Some(now_seconds);
        self.collect_dispatch_at(now_seconds)
    }

    pub fn collect_dispatch_at(
        &mut self,
        now_seconds: f64,
    ) -> Result<ServerRuntimeDispatch, ServerRuntimeError> {
        let now_seconds = if now_seconds.is_finite() {
            now_seconds
        } else {
            self.current_time_seconds()
        };
        self.prune_playback_barrier_request_tombstones();
        self.persist_occupied_room_activity_if_due_at_for_all_rooms(now_seconds)?;
        let expired_persistent_rooms = self.expire_inactive_persistent_rooms_at(now_seconds)?;
        let mut outbound_messages = self.collect_due_periodic_updates_at(now_seconds)?;
        if expired_persistent_rooms && self.persistent_rooms_enabled {
            self.enqueue_list_snapshots_for_clients(
                &mut outbound_messages,
                self.clients_receiving_to_gui_only_list_updates(None),
            );
        }
        self.collect_due_stats_snapshots_at(now_seconds)?;
        let outbound_lines = outbound_messages
            .into_iter()
            .map(|message| {
                let delivery = match &message.message {
                    ProtocolMessage::State(state)
                        if state.state.ignoring_on_the_fly.is_none()
                            && state
                                .state
                                .playstate
                                .as_ref()
                                .and_then(|playstate| playstate.do_seek)
                                != Some(true) =>
                    {
                        ServerOutboundDelivery::CoalesciblePeriodicState
                    }
                    _ => ServerOutboundDelivery::Reliable,
                };
                Ok(DirectedOutboundLine {
                    client_id: message.client_id,
                    line: encode_message_line(&message.message)?,
                    delivery,
                })
            })
            .collect::<Result<Vec<_>, ServerRuntimeError>>()?;
        let transport_actions = self.drain_transport_actions();
        Ok(ServerRuntimeDispatch {
            outbound_lines,
            transport_actions,
        })
    }
}
