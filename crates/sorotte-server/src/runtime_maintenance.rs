use super::*;

fn participant_status_report_age_ms(retained: &RetainedParticipantStatus, now_seconds: f64) -> u64 {
    let elapsed_seconds = now_seconds - retained.received_at_seconds;
    let observed_age_ms = if !elapsed_seconds.is_finite() || elapsed_seconds < 0.0 {
        // Clock rollback is a one-way fence for this report. A replacement
        // report creates a new retained value and therefore a new clock.
        u64::MAX
    } else {
        // Positive float-to-integer conversion saturates, so overflow and
        // positive infinity become the same fail-closed maximum age.
        (elapsed_seconds * 1_000.0).ceil() as u64
    };
    let monotonic_age_ms = retained
        .max_projected_report_age_ms
        .get()
        .max(observed_age_ms);
    retained.max_projected_report_age_ms.set(monotonic_age_ms);
    monotonic_age_ms
}

fn participant_status_availability(report_age_ms: u64) -> ParticipantStatusAvailability {
    // Availability describes the Sorotte report stream, not the age of any
    // optional player sample carried by the report. Lifecycle-only reports
    // (starting, disconnected, failed, unavailable) deliberately have no
    // player evidence and must remain fresh while they continue arriving.
    if report_age_ms <= PARTICIPANT_STATUS_FRESH_MILLIS {
        ParticipantStatusAvailability::Fresh
    } else if report_age_ms <= PARTICIPANT_STATUS_DELAYED_MILLIS {
        ParticipantStatusAvailability::Delayed
    } else {
        ParticipantStatusAvailability::Stale
    }
}

fn participant_status_correlation(
    reported_scope: Option<ParticipantPlaybackScope>,
    current_scope: ParticipantPlaybackScope,
) -> ParticipantStatusCorrelation {
    let Some(reported_scope) = reported_scope else {
        return ParticipantStatusCorrelation::Uncorrelated;
    };
    if reported_scope == current_scope {
        return ParticipantStatusCorrelation::Exact;
    }

    // Missing optional fence components are an absence of correlation
    // evidence, not proof that the report belongs to an older epoch. Only an
    // explicitly comparable mismatch is superseded.
    let state_revision_conflicts = reported_scope
        .state_revision
        .zip(current_scope.state_revision)
        .is_some_and(|(reported, current)| reported != current);
    let transport_revision_conflicts = reported_scope
        .transport_revision
        .zip(current_scope.transport_revision)
        .is_some_and(|(reported, current)| reported != current);
    if reported_scope.media_generation != current_scope.media_generation
        || state_revision_conflicts
        || transport_revision_conflicts
    {
        ParticipantStatusCorrelation::Superseded
    } else {
        ParticipantStatusCorrelation::Uncorrelated
    }
}

fn projected_participant_position(
    report: &ParticipantStatusReport,
    report_age_ms: u64,
    forward_delay_ms: Option<u64>,
) -> Option<f64> {
    let mut position = report.position_seconds?;
    let position_sample_age_ms = report.position_sample_age_ms?;
    if report.player_connection == ParticipantPlayerConnection::Connected
        && report.phase == ParticipantPlaybackPhase::Playing
        && report.logical_paused == Some(false)
        && report.paused_for_cache == Some(false)
    {
        let playback_rate = report.playback_rate?;
        let total_sample_age_ms = position_sample_age_ms
            .saturating_add(report_age_ms)
            .saturating_add(forward_delay_ms.unwrap_or_default());
        position += total_sample_age_ms as f64 / 1_000.0 * playback_rate;
    }
    position.is_finite().then_some(position)
}

fn compact_participant_status_snapshot(
    snapshot: &ParticipantStatusSnapshot,
) -> ParticipantStatusSnapshot {
    let participants = snapshot
        .participants
        .iter()
        .map(|(username, full)| {
            let mut compact = ParticipantStatusView::new(full.availability);
            compact.correlation = full.correlation;
            compact.player_connection = full.player_connection;
            compact.phase = full.phase;
            // Compact rows retain the report-stream clock for coarse
            // connection/phase freshness, but intentionally omit player
            // evidence and its independent sample clocks.
            compact.report_age_ms = full.report_age_ms;
            (username.clone(), compact)
        })
        .collect();
    ParticipantStatusSnapshot::new(snapshot.revision, participants)
        .with_mode(ParticipantStatusSnapshotMode::Compact)
}

fn unavailable_participant_status_snapshot(
    snapshot: &ParticipantStatusSnapshot,
) -> ParticipantStatusSnapshot {
    ParticipantStatusSnapshot::new(snapshot.revision, BTreeMap::new())
        .with_mode(ParticipantStatusSnapshotMode::Unavailable)
}

pub(crate) fn protocol_line_exceeds_maximum(encoded_len: usize) -> bool {
    encoded_len > DEFAULT_MAX_PROTOCOL_LINE_BYTES
}

fn split_participant_status_from_reliable_passthrough(
    message: ProtocolMessage,
) -> Vec<ProtocolMessage> {
    let mut periodic = match message {
        ProtocolMessage::State(message) => message,
        message => return vec![message],
    };
    let has_reliable_client_passthrough = periodic
        .state
        .ignoring_on_the_fly
        .as_ref()
        .and_then(|ignoring| ignoring.client)
        .is_some();
    let has_participant_status = periodic
        .state
        .participant_status_v1()
        .ok()
        .flatten()
        .is_some();
    if !has_reliable_client_passthrough || !has_participant_status {
        return vec![ProtocolMessage::State(periodic)];
    }

    // Participant snapshots are population-sized, self-healing advisory
    // state. Never let one hitch a ride on the reliable passthrough frame:
    // retain the one-shot client metadata there and emit a separate pure
    // coalescible State for the snapshot.
    let mut reliable = periodic.clone();
    reliable.state.extra.remove(SOROTTE_PARTICIPANT_STATUS_V1);
    periodic.state.ignoring_on_the_fly = None;
    if let Some(ping) = periodic.state.ping.as_mut() {
        ping.client_latency_calculation = None;
    }
    vec![
        ProtocolMessage::State(reliable),
        ProtocolMessage::State(periodic),
    ]
}

impl ServerRuntime {
    fn observe_tls_certificate_bundle(
        &self,
        path: &Path,
    ) -> Option<(
        TlsCertificateBundleFingerprint,
        Option<TlsCertificateBundleSnapshot>,
    )> {
        #[cfg(test)]
        if let Some(clock) = self.tls_certificate_bundle_metadata_clock.as_ref() {
            return Some((clock.fingerprint(), None));
        }
        let snapshot = read_tls_certificate_bundle_snapshot(path).ok()?;
        Some((snapshot.fingerprint(), Some(snapshot)))
    }

    fn apply_tls_certificate_bundle_observation(
        &mut self,
        path: &Path,
        fingerprint: TlsCertificateBundleFingerprint,
        snapshot: Option<TlsCertificateBundleSnapshot>,
    ) {
        self.tls_certificate_bundle_fingerprint = Some(fingerprint);
        let loaded = match snapshot.as_ref() {
            Some(snapshot) => load_tls_server_config_from_snapshot(path, snapshot),
            None => load_tls_server_config(path),
        };
        match loaded {
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

    pub(crate) fn current_time_seconds(&self) -> f64 {
        self.time_now_override_seconds
            .unwrap_or_else(current_unix_timestamp_seconds)
    }

    pub(crate) fn local_time_seconds(&self) -> f64 {
        self.local_clock.now()
    }

    pub(crate) fn local_time_for_wall_sample(&self, wall: f64) -> f64 {
        if self.time_now_override_seconds.is_some() && !self.local_clock.independent_override {
            self.local_clock.observe(wall)
        } else {
            self.local_time_seconds()
        }
    }

    pub(crate) fn record_client_state_update_now(&mut self, client_id: &str) {
        self.client_last_state_update_at
            .insert(client_id.to_owned(), self.local_time_seconds());
    }

    pub(crate) fn initialize_stats_snapshot_schedule(&mut self) {
        self.initialize_stats_snapshot_schedule_at(self.current_time_seconds());
    }

    pub(crate) fn initialize_stats_snapshot_schedule_at(&mut self, now_seconds: f64) {
        if self.stats_persistence.is_none() {
            self.stats_next_snapshot_at_seconds = None;
            return;
        }
        self.stats_next_snapshot_at_seconds = Some(
            now_seconds
                + self.stats_snapshot_start_delay_seconds
                + self.stats_snapshot_interval_seconds,
        );
    }

    pub(crate) fn refresh_tls_context_from_cert_path(&mut self) {
        let Some(path) = self.tls_cert_path.as_ref() else {
            self.tls_server_config = None;
            self.tls_context_available = false;
            self.server_accepts_tls = false;
            self.tls_certificate_bundle_fingerprint = None;
            return;
        };
        let path = path.clone();
        let Some((fingerprint, snapshot)) = self.observe_tls_certificate_bundle(&path) else {
            self.tls_server_config = None;
            self.tls_context_available = false;
            self.server_accepts_tls = false;
            self.tls_certificate_bundle_fingerprint = None;
            return;
        };
        self.apply_tls_certificate_bundle_observation(&path, fingerprint, snapshot);
    }

    pub(crate) fn refresh_tls_context_after_cert_rotation_if_needed(&mut self) {
        let Some(path) = self.tls_cert_path.as_ref() else {
            return;
        };
        let path = path.clone();
        let Some((fingerprint, snapshot)) = self.observe_tls_certificate_bundle(&path) else {
            return;
        };
        if Some(fingerprint) == self.tls_certificate_bundle_fingerprint {
            return;
        }
        self.apply_tls_certificate_bundle_observation(&path, fingerprint, snapshot);
        self.tls_rotation_attempts = self.tls_rotation_attempts.saturating_add(1);
        if self.tls_rotation_attempts < TLS_CERT_ROTATION_MAX_RETRIES {
            self.server_accepts_tls = true;
        }
    }

    pub(crate) fn collect_due_stats_snapshots_at(
        &mut self,
        now_seconds: f64,
    ) -> Result<(), ServerRuntimeError> {
        if self.stats_persistence.is_none() {
            self.stats_next_snapshot_at_seconds = None;
            return Ok(());
        }
        if self.stats_next_snapshot_at_seconds.is_none() {
            self.initialize_stats_snapshot_schedule_at(now_seconds);
        }
        let Some(mut next_snapshot_at_seconds) = self.stats_next_snapshot_at_seconds else {
            return Ok(());
        };
        while next_snapshot_at_seconds <= now_seconds {
            self.record_stats_snapshot_at(next_snapshot_at_seconds)?;
            next_snapshot_at_seconds += self.stats_snapshot_interval_seconds;
        }
        self.stats_next_snapshot_at_seconds = Some(next_snapshot_at_seconds);
        Ok(())
    }

    pub(crate) fn record_stats_snapshot_at(
        &self,
        snapshot_at_seconds: f64,
    ) -> Result<(), ServerRuntimeError> {
        let snapshot_time = snapshot_at_seconds.floor() as i64;
        let mut versions: Vec<String> = self
            .sessions
            .values()
            .map(|session| session.version.clone())
            .collect();
        versions.sort();
        if let Some(stats_persistence) = self.stats_persistence.as_ref() {
            stats_persistence.enqueue(ServerPersistenceEffect::RecordStatsSnapshot {
                snapshot_time,
                versions,
            });
        }
        Ok(())
    }

    pub(crate) fn collect_due_periodic_updates_at(
        &mut self,
        now: f64,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let mut outbound = self.collect_due_playback_barrier_updates_at(now)?;
        let elapsed_now = self.local_time_for_wall_sample(now);
        if now.is_finite() {
            for retained in self.client_participant_status.values() {
                let _ = participant_status_report_age_ms(retained, now);
            }
        }
        let mut due_clients: Vec<String> = self
            .client_next_periodic_state_at
            .iter()
            .filter(|(_, next_state_at)| **next_state_at <= elapsed_now)
            .map(|(client_id, _)| client_id.clone())
            .collect();
        due_clients.sort();

        // Remove every timed-out member before producing any recipient's
        // complete room snapshot. Otherwise lexical client ordering can let a
        // live recipient observe a zombie that is removed later in the same
        // maintenance batch.
        for client_id in &due_clients {
            if !self.sessions.contains_key(client_id) {
                continue;
            }
            if self.reject_fenced_playback_barrier_transport(client_id) {
                continue;
            }
            if !self.client_timed_out(client_id, now) {
                continue;
            }
            self.pending_transport_actions
                .push(DirectedTransportAction::new(
                    client_id,
                    ServerTransportAction::Close,
                ));
            outbound.extend(self.timeout_disconnect_messages(client_id)?);
        }

        for client_id in due_clients {
            if !self.client_next_periodic_state_at.contains_key(&client_id) {
                continue;
            }
            // Coalescible periodic State has no useful intermediate history.
            // Rebase the cadence from the one materialized update. This is
            // constant-time after any clock jump and prevents an immediate
            // second tick caused by floating-point catch-up arithmetic.
            self.client_next_periodic_state_at.insert(
                client_id.clone(),
                elapsed_now + SERVER_STATE_INTERVAL_SECONDS,
            );
            outbound.extend(self.collect_periodic_tick_for_client_at(&client_id, now, now)?);
        }

        Ok(outbound)
    }

    pub(crate) fn collect_periodic_tick_for_client_at(
        &mut self,
        client_id: &str,
        ticked_at: f64,
        message_now_seconds: f64,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let Some(session) = self.sessions.get(client_id).cloned() else {
            return Ok(Vec::new());
        };
        // A fenced session still models a live transport until the network's
        // disconnect callback arrives. Periodic timeout cleanup must not
        // remove the session (and its fence) while the close event and a
        // racing protocol line are still in flight.
        if self.reject_fenced_playback_barrier_transport(client_id) {
            return Ok(Vec::new());
        }
        self.ensure_room_state(&session.room);
        if self.room_playback_state(&session.room).set_by.is_none()
            && let Some(set_by_username) = self.fallback_room_set_by_username(&session.room)
        {
            self.room_playback_state_mut(&session.room).set_by = Some(set_by_username);
        }
        let room_state = self.refresh_room_playback_state_from_clients_at(&session.room, ticked_at);

        let mut outbound = Vec::new();
        if let Some(state_message) = self.periodic_state_sync_message_for_client_at(
            client_id,
            room_state.position,
            room_state.paused,
            room_state.set_by.as_deref(),
            message_now_seconds,
        ) {
            outbound.extend(
                split_participant_status_from_reliable_passthrough(state_message)
                    .into_iter()
                    .map(|message| DirectedProtocolMessage::new(client_id, message)),
            );
        }

        if self.client_timed_out(client_id, ticked_at) {
            self.pending_transport_actions
                .push(DirectedTransportAction::new(
                    client_id,
                    ServerTransportAction::Close,
                ));
            outbound.extend(self.timeout_disconnect_messages(client_id)?);
        }

        Ok(outbound)
    }

    pub(crate) fn fallback_room_set_by_username(&self, room_name: &str) -> Option<String> {
        self.sessions
            .iter()
            .filter(|(client_id, session)| {
                session.room == room_name
                    && !self.playback_barrier_fenced_clients.contains(*client_id)
            })
            .min_by_key(|(client_id, _)| self.client_room_join_order(client_id))
            .map(|(_, session)| session.username.clone())
    }

    pub(crate) fn assign_room_join_order(&mut self, client_id: &str) {
        self.client_room_join_sequence
            .insert(client_id.to_owned(), self.next_room_join_sequence);
        self.next_room_join_sequence = self.next_room_join_sequence.saturating_add(1);
    }

    pub(crate) fn client_room_join_order(&self, client_id: &str) -> u64 {
        self.client_room_join_sequence
            .get(client_id)
            .copied()
            .unwrap_or(u64::MAX)
    }

    pub(crate) fn periodic_state_sync_message_for_client_at(
        &mut self,
        client_id: &str,
        position: f64,
        paused: bool,
        set_by: Option<&str>,
        now_seconds: f64,
    ) -> Option<ProtocolMessage> {
        let room = self.sessions.get(client_id)?.room.clone();
        let server_ignoring_counter = self.server_ignoring_counter(client_id);
        let server_rtt_seconds = self.server_rtt_seconds(client_id);
        let (pending_client_latency, pending_client_ignoring) =
            self.take_client_passthrough_state_metadata_at(client_id, now_seconds);
        if server_ignoring_counter > 0 {
            return None;
        }
        let participant_status =
            self.participant_status_snapshot_for_client_at(client_id, now_seconds);
        let transport_revision = self.transport_authority_revision_for_room(&room);
        let ping_timestamp = self.record_ping_challenge(client_id, now_seconds);
        let build_message = |participant_status| {
            state_sync_message(
                position,
                paused,
                false,
                StateSyncOptions {
                    set_by,
                    transport_revision,
                    client_latency_calculation: pending_client_latency,
                    client_ignoring_counter: pending_client_ignoring,
                    server_rtt_seconds,
                    latency_calculation_seconds: Some(ping_timestamp),
                    participant_status,
                    ..StateSyncOptions::default()
                },
            )
        };
        let mut message = build_message(participant_status.clone());
        if protocol_line_exceeds_maximum(encode_message_line(&message).ok()?.len())
            && let Some(mut compact) = participant_status.clone()
            && let Some(snapshot) = compact.snapshot.as_ref()
        {
            let compact_snapshot = compact_participant_status_snapshot(snapshot);
            self.cache_participant_status_snapshot_representation(client_id, &compact_snapshot);
            compact.snapshot = Some(compact_snapshot);
            message = build_message(Some(compact.clone()));
            if protocol_line_exceeds_maximum(encode_message_line(&message).ok()?.len())
                && let Some(snapshot) = compact.snapshot.as_ref()
            {
                let unavailable_snapshot = unavailable_participant_status_snapshot(snapshot);
                self.cache_participant_status_snapshot_representation(
                    client_id,
                    &unavailable_snapshot,
                );
                compact.snapshot = Some(unavailable_snapshot);
                message = build_message(Some(compact));
            }
        }
        Some(message)
    }

    fn cache_participant_status_snapshot_representation(
        &mut self,
        client_id: &str,
        snapshot: &ParticipantStatusSnapshot,
    ) {
        let Some(room) = self
            .sessions
            .get(client_id)
            .map(|session| session.room.as_str())
        else {
            return;
        };
        let Some(cached) = self.participant_status_snapshot_cache.get_mut(room) else {
            return;
        };
        let representation_escalates = (cached.snapshot.mode
            == ParticipantStatusSnapshotMode::Full
            && snapshot.mode != ParticipantStatusSnapshotMode::Full)
            || (cached.snapshot.mode == ParticipantStatusSnapshotMode::Compact
                && snapshot.mode == ParticipantStatusSnapshotMode::Unavailable);
        if cached.snapshot.revision == snapshot.revision && representation_escalates {
            cached.snapshot = snapshot.clone();
        }
    }

    /// Mutation/invariant-only seam for verifying that cached snapshot
    /// degradation is revision-scoped without widening the release API.
    #[cfg(test)]
    pub(crate) fn cache_participant_status_snapshot_representation_for_test(
        &mut self,
        client_id: &str,
        snapshot: &ParticipantStatusSnapshot,
    ) {
        self.cache_participant_status_snapshot_representation(client_id, snapshot);
    }

    pub(crate) fn client_timed_out(&self, client_id: &str, now_seconds: f64) -> bool {
        self.client_last_state_update_at
            .get(client_id)
            .is_some_and(|updated_at| {
                self.local_time_for_wall_sample(now_seconds) - updated_at > PROTOCOL_TIMEOUT_SECONDS
            })
    }

    pub(crate) fn timeout_disconnect_messages(
        &mut self,
        client_id: &str,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let Some(session) = self.sessions.get(client_id).cloned() else {
            return Ok(Vec::new());
        };
        let mut outbound_messages =
            self.mark_playback_barrier_participant_disconnected(client_id, &session.room)?;
        outbound_messages
            .extend(self.mark_room_buffering_participant_disconnected(client_id, &session.room)?);
        outbound_messages.extend(self.detach_readiness_membership(client_id, true)?);
        let Some(session) = self.remove_session_tracking(client_id) else {
            return Ok(outbound_messages);
        };
        outbound_messages.extend(self.refresh_mixed_readiness_cohort(&session.room)?);
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
        outbound_messages.extend(
            recipients
                .into_iter()
                .map(|peer_client| DirectedProtocolMessage::new(peer_client, left_message.clone())),
        );
        if self.persistent_rooms_enabled {
            self.enqueue_list_snapshots_for_clients(
                &mut outbound_messages,
                self.clients_receiving_to_gui_only_list_updates(None),
            );
        }
        Ok(outbound_messages)
    }

    pub(crate) fn remove_session_tracking(&mut self, client_id: &str) -> Option<ServerSession> {
        if !self.sessions.contains_key(client_id) {
            return None;
        }
        let status_supported = self
            .sessions
            .get(client_id)
            .is_some_and(|session| session.capabilities.participant_status_v1);
        let membership_epoch = self.client_room_join_order(client_id);
        emit_server_lifecycle_transition(
            "ROOM-LEAVE-001",
            "room-membership",
            TargetKind::ServerState,
            Trigger::Shutdown,
            Disposition::Submitted,
            &[("membership-epoch", membership_epoch)],
        );
        emit_server_lifecycle_transition(
            "PLAYLIST-LOSE-AUTHORITY-001",
            "playlist-selection",
            TargetKind::ServerState,
            Trigger::Shutdown,
            Disposition::Applied,
            &[("membership-epoch", membership_epoch)],
        );
        self.clear_participant_status_for_client(client_id);
        if status_supported {
            emit_server_lifecycle_transition(
                "STATUS-UNAVAILABLE-001",
                "participant-status",
                TargetKind::ServerState,
                Trigger::Shutdown,
                Disposition::Applied,
                &[("membership-epoch", membership_epoch)],
            );
        }
        emit_server_lifecycle_transition(
            "SESSION-DISCONNECT-001",
            "session",
            TargetKind::ServerState,
            Trigger::Shutdown,
            Disposition::Applied,
            &[("membership-epoch", membership_epoch)],
        );
        let session = self.sessions.remove(client_id)?;
        self.playback_barrier_fenced_clients.remove(client_id);
        let _ = self.domain.leave_room(&session.username, &session.room);
        self.remove_room_controller(&session.username, &session.room);
        self.client_state_counters.remove(client_id);
        self.client_playback_states.remove(client_id);
        self.client_participant_status_last_sequence
            .remove(client_id);
        self.client_room_join_sequence.remove(client_id);
        self.pending_user_transport_by_client.remove(client_id);
        self.readiness_reconnect_identity_by_client
            .remove(client_id);
        self.playback_barrier_request_nonces.remove(client_id);
        self.playback_barrier_new_identity_rate_by_client
            .remove(client_id);
        self.client_last_state_update_at.remove(client_id);
        self.client_next_periodic_state_at.remove(client_id);
        self.client_peer_ips.remove(client_id);
        emit_server_lifecycle_transition(
            "ROOM-OUTSIDE-001",
            "room-membership",
            TargetKind::ServerState,
            Trigger::Shutdown,
            Disposition::Applied,
            &[("membership-epoch", membership_epoch)],
        );
        Some(session)
    }

    /// Retires one connection's advisory status and every cached projection
    /// that could still contain its old membership or report. The session
    /// room is included even when no report was retained, because cached
    /// snapshots also contain `AwaitingReport` and `Unsupported` members.
    pub(crate) fn clear_participant_status_for_client(&mut self, client_id: &str) {
        let retained = self.client_participant_status.remove(client_id);
        if let Some(retained) = retained.as_ref() {
            emit_server_lifecycle_transition(
                "STATUS-WITHDRAW-001",
                "participant-status",
                TargetKind::ServerState,
                Trigger::Internal,
                Disposition::Applied,
                &[
                    ("membership-epoch", retained.room_join_sequence),
                    ("report-sequence", retained.report.report_sequence),
                ],
            );
        }
        let retained_room = retained.as_ref().map(|retained| retained.room.as_str());
        let session_room = self
            .sessions
            .get(client_id)
            .map(|session| session.room.clone());

        if let Some(room) = retained_room {
            self.participant_status_snapshot_cache.remove(room);
        }
        if let Some(room) = session_room.as_deref()
            && retained_room != Some(room)
        {
            self.participant_status_snapshot_cache.remove(room);
        }
    }

    pub(crate) fn clear_participant_status_for_room(&mut self, room: &str) {
        self.client_participant_status
            .retain(|_, retained| retained.room != room);
        self.participant_status_snapshot_cache.remove(room);
    }

    pub(crate) fn advance_participant_status_media_generation(
        &mut self,
        room: &str,
        authoritative_generation: Option<u64>,
    ) {
        self.ensure_room_state(room);
        let scope = self
            .room_participant_status_scopes
            .entry(room.to_owned())
            .or_default();
        scope.media_generation = authoritative_generation
            .filter(|generation| *generation > 0)
            .unwrap_or_else(|| scope.media_generation.saturating_add(1));
        scope.transport_revision = scope.transport_revision.saturating_add(1);
        self.participant_status_snapshot_cache.remove(room);
    }

    pub(crate) fn advance_participant_status_transport_revision(&mut self, room: &str) {
        self.ensure_room_state(room);
        let scope = self
            .room_participant_status_scopes
            .entry(room.to_owned())
            .or_default();
        scope.transport_revision = scope.transport_revision.saturating_add(1);
        self.participant_status_snapshot_cache.remove(room);
    }

    fn participant_status_scope_for_room(&self, room: &str) -> ParticipantPlaybackScope {
        let state_revision = self
            .room_playback_barriers
            .get(room)
            .and_then(|barrier| barrier.state_revision);
        self.room_participant_status_scopes
            .get(room)
            .copied()
            .unwrap_or_default()
            .to_wire(state_revision)
    }

    pub(crate) fn transport_authority_revision_for_room(&self, room: &str) -> Option<u64> {
        self.room_participant_status_scopes
            .get(room)
            .map(|scope| scope.transport_revision)
    }

    fn participant_status_snapshot_for_client_at(
        &mut self,
        client_id: &str,
        now_seconds: f64,
    ) -> Option<ParticipantStatusStateExtension> {
        let recipient = self.sessions.get(client_id)?.clone();
        if !recipient.capabilities.participant_status_v1 {
            return None;
        }
        let generated_at_bits = now_seconds.to_bits();
        if let Some(cached) = self.participant_status_snapshot_cache.get(&recipient.room)
            && cached.generated_at_bits == generated_at_bits
        {
            return Some(
                ParticipantStatusStateExtension::new()
                    .with_scope(self.participant_status_scope_for_room(&recipient.room))
                    .with_snapshot(cached.snapshot.clone()),
            );
        }

        let revision = self
            .room_participant_status_snapshot_revisions
            .entry(recipient.room.clone())
            .or_default();
        *revision = revision.checked_add(1)?;
        let revision = *revision;

        let current_scope = self.participant_status_scope_for_room(&recipient.room);
        let room_position = self
            .room_playback_states
            .get(&recipient.room)
            .map(|state| state.position_at(now_seconds));

        let participants = self
            .sessions
            .iter()
            .filter(|(peer_client_id, session)| {
                session.room == recipient.room
                    && !self
                        .playback_barrier_fenced_clients
                        .contains(*peer_client_id)
            })
            .map(|(peer_client_id, session)| {
                let view = if !session.capabilities.participant_status_v1 {
                    ParticipantStatusView::new(ParticipantStatusAvailability::Unsupported)
                } else if let Some(retained) = self.client_participant_status.get(peer_client_id)
                    && retained.room == session.room
                    && retained.username == session.username
                    && retained.room_join_sequence == self.client_room_join_order(peer_client_id)
                {
                    let report_age_ms = participant_status_report_age_ms(retained, now_seconds);
                    let correlation = participant_status_correlation(
                        retained.report.playback_scope,
                        current_scope,
                    );
                    let availability = participant_status_availability(report_age_ms);
                    if retained.projected_availability.advance(availability) {
                        let transition = match availability {
                            ParticipantStatusAvailability::Delayed => Some("STATUS-DELAY-001"),
                            ParticipantStatusAvailability::Stale => Some("STATUS-STALE-001"),
                            _ => None,
                        };
                        if let Some(transition) = transition {
                            emit_server_lifecycle_transition(
                                transition,
                                "participant-status",
                                TargetKind::ServerState,
                                Trigger::Timer,
                                Disposition::Applied,
                                &[
                                    ("membership-epoch", retained.room_join_sequence),
                                    ("report-sequence", retained.report.report_sequence),
                                ],
                            );
                        }
                    }
                    let mut view = ParticipantStatusView::new(availability);
                    view.correlation = Some(correlation);
                    view.player_connection = Some(retained.report.player_connection);
                    view.report_age_ms = Some(report_age_ms);
                    if availability != ParticipantStatusAvailability::Stale {
                        // Connection and coarse phase remain useful while a
                        // player is applying a newer scope. A legacy or
                        // partially fenced report may retain its own local
                        // media evidence, but only exact correlation can
                        // produce a room-derived offset.
                        view.phase = Some(retained.report.phase);
                        if correlation != ParticipantStatusCorrelation::Superseded {
                            // Snapshot ages are evaluated at the server's
                            // projection instant. Including report residence
                            // and the validated forward delay exactly once
                            // lets receivers continue the same evidence clock
                            // instead of granting a second fresh window.
                            let projected_sample_age_ms =
                                retained.report.sample_age_ms.map(|sample_age_ms| {
                                    sample_age_ms.saturating_add(report_age_ms).saturating_add(
                                        retained.forward_delay_ms.unwrap_or_default(),
                                    )
                                });
                            let projected_position_sample_age_ms =
                                retained.report.position_sample_age_ms.map(|sample_age_ms| {
                                    sample_age_ms.saturating_add(report_age_ms).saturating_add(
                                        retained.forward_delay_ms.unwrap_or_default(),
                                    )
                                });
                            view.playback_scope = retained.report.playback_scope;
                            view.sample_age_ms = projected_sample_age_ms;
                            view.position_sample_age_ms = projected_position_sample_age_ms;

                            if projected_position_sample_age_ms.is_some_and(|sample_age_ms| {
                                sample_age_ms <= PARTICIPANT_STATUS_DELAYED_MILLIS
                            }) {
                                view.timeline_kind = Some(retained.report.timeline_kind);
                                view.position_seconds = projected_participant_position(
                                    &retained.report,
                                    report_age_ms,
                                    retained.forward_delay_ms,
                                );
                            }
                            if projected_sample_age_ms.is_some_and(|sample_age_ms| {
                                sample_age_ms <= PARTICIPANT_STATUS_DELAYED_MILLIS
                            }) {
                                view.logical_paused = retained.report.logical_paused;
                                view.playback_rate = retained.report.playback_rate;
                                view.paused_for_cache = retained.report.paused_for_cache;
                                view.cache_percent = retained.report.cache_percent;
                                view.buffered_ahead_seconds =
                                    retained.report.buffered_ahead_seconds;
                            }

                            let total_sample_age_ms = retained
                                .report
                                .sample_age_ms
                                .zip(retained.forward_delay_ms)
                                .map(|(sample_age, forward_delay_ms)| {
                                    sample_age
                                        .saturating_add(report_age_ms)
                                        .saturating_add(forward_delay_ms)
                                });
                            let total_position_sample_age_ms = retained
                                .report
                                .position_sample_age_ms
                                .zip(retained.forward_delay_ms)
                                .map(|(sample_age, forward_delay_ms)| {
                                    sample_age
                                        .saturating_add(report_age_ms)
                                        .saturating_add(forward_delay_ms)
                                });
                            let offset_is_safe = correlation == ParticipantStatusCorrelation::Exact
                                && availability == ParticipantStatusAvailability::Fresh
                                && current_scope.state_revision.is_some()
                                && current_scope.transport_revision.is_some()
                                && retained.report.playback_scope == Some(current_scope)
                                && retained.report.player_connection
                                    == ParticipantPlayerConnection::Connected
                                && retained.report.timeline_kind == ParticipantTimelineKind::Vod
                                && matches!(
                                    retained.report.phase,
                                    ParticipantPlaybackPhase::Playing
                                        | ParticipantPlaybackPhase::ReadyPaused
                                        | ParticipantPlaybackPhase::Rebuffering
                                )
                                && match retained.report.phase {
                                    ParticipantPlaybackPhase::Playing => {
                                        retained.report.logical_paused == Some(false)
                                            && retained.report.playback_rate.is_some()
                                            && retained.report.paused_for_cache == Some(false)
                                    }
                                    ParticipantPlaybackPhase::ReadyPaused => {
                                        retained.report.logical_paused == Some(true)
                                            && retained.report.paused_for_cache == Some(false)
                                    }
                                    ParticipantPlaybackPhase::Rebuffering => {
                                        retained.report.paused_for_cache == Some(true)
                                    }
                                    _ => false,
                                }
                                && total_sample_age_ms.is_some_and(|sample_age| {
                                    sample_age <= PARTICIPANT_STATUS_FRESH_MILLIS
                                })
                                && total_position_sample_age_ms.is_some_and(|sample_age| {
                                    sample_age <= PARTICIPANT_STATUS_FRESH_MILLIS
                                });
                            if offset_is_safe
                                && let (Some(participant_position), Some(room_position)) =
                                    (view.position_seconds, room_position)
                            {
                                let offset = participant_position - room_position;
                                if offset.is_finite()
                                    && offset.abs()
                                        <= PARTICIPANT_STATUS_MAX_ABSOLUTE_ROOM_OFFSET_SECONDS
                                {
                                    view.room_offset_seconds = Some(offset);
                                }
                            }
                        }
                    }
                    view.redact_ineligible_media_evidence();
                    view
                } else {
                    ParticipantStatusView::new(ParticipantStatusAvailability::AwaitingReport)
                };
                (session.username.clone(), view)
            })
            .collect();

        let snapshot = ParticipantStatusSnapshot::new(revision, participants);
        self.participant_status_snapshot_cache.insert(
            recipient.room.clone(),
            CachedParticipantStatusSnapshot {
                generated_at_bits,
                snapshot: snapshot.clone(),
            },
        );
        Some(
            ParticipantStatusStateExtension::new()
                .with_scope(current_scope)
                .with_snapshot(snapshot),
        )
    }

    pub(crate) fn apply_persisted_rooms_snapshot(
        &mut self,
        persisted_rooms: BTreeMap<String, PersistedRoomState>,
    ) {
        let now_seconds = self.current_time_seconds();
        for (room_name, persisted_room) in persisted_rooms {
            let persisted_last_activity_at_seconds = persisted_room.last_activity_at_seconds;
            let position = persisted_room.position;
            let owner_bucket = persisted_room
                .owner_bucket
                .unwrap_or_else(|| LEGACY_PERSISTENT_ROOM_OWNER_BUCKET.to_owned());
            let mut playlist = RoomPlaylistState {
                files: persisted_room.files,
                index: persisted_room.index,
                epoch: 0,
            };
            playlist.normalize_index();
            self.room_playlists.insert(room_name.clone(), playlist);
            // Historical rows used zero as a placeholder. Treat missing,
            // non-finite, or future timestamps as activity at startup so a
            // legacy database is never purged immediately after upgrading.
            let has_valid_persisted_activity = persisted_last_activity_at_seconds.is_finite()
                && persisted_last_activity_at_seconds > 0.0
                && persisted_last_activity_at_seconds <= now_seconds;
            let last_activity_at_seconds = if has_valid_persisted_activity {
                persisted_last_activity_at_seconds
            } else {
                now_seconds
            };
            self.persistent_room_last_activity_at
                .insert(room_name.clone(), last_activity_at_seconds);
            let created_at_seconds = if persisted_room.created_at_seconds.is_finite()
                && persisted_room.created_at_seconds > 0.0
                && persisted_room.created_at_seconds <= now_seconds
            {
                persisted_room.created_at_seconds
            } else {
                last_activity_at_seconds
            };
            self.persistent_room_owner_by_room
                .insert(room_name.clone(), owner_bucket.clone());
            self.persistent_room_created_at_by_room
                .insert(room_name.clone(), created_at_seconds);
            self.persistent_room_last_creation_by_identity
                .entry(owner_bucket.clone())
                .and_modify(|last_creation| {
                    *last_creation = last_creation.max(created_at_seconds);
                })
                .or_insert(created_at_seconds);
            let room_playback = self
                .room_playback_states
                .entry(room_name.clone())
                .or_default();
            room_playback.position = position;
            room_playback.updated_at_seconds = now_seconds;
            self.room_participant_status_scopes
                .entry(room_name.clone())
                .or_default();
            if !has_valid_persisted_activity && self.room_persistence.is_some() {
                // Make the startup-time fallback a one-time migration. Without
                // this write-back, repeatedly restarting a legacy database
                // whose placeholder is zero would renew its grace forever.
                let playlist = self.room_playlist_state(&room_name).clone();
                let version = self.next_room_persistence_version();
                self.room_persistence
                    .as_ref()
                    .expect("room persistence presence checked above")
                    .enqueue(ServerPersistenceEffect::SaveRoom {
                        room_name,
                        files: playlist.files,
                        playlist_index: playlist.index,
                        position,
                        last_activity_at_seconds,
                        owner_bucket: Some(owner_bucket),
                        created_at_seconds,
                        version,
                    });
            }
        }
    }

    pub(crate) fn replace_persisted_rooms_snapshot(
        &mut self,
        persisted_rooms: BTreeMap<String, PersistedRoomState>,
    ) {
        let replacement_names = persisted_rooms.keys().cloned().collect::<BTreeSet<_>>();
        let replaced_names = self
            .persisted_room_names
            .union(&replacement_names)
            .cloned()
            .collect::<Vec<_>>();
        for room_name in replaced_names {
            self.remove_room_runtime_state(&room_name);
        }
        self.persistent_room_owner_by_room.clear();
        self.persistent_room_created_at_by_room.clear();
        self.persistent_room_last_creation_by_identity.clear();
        self.persistent_room_last_activity_at.clear();
        self.persisted_room_names = replacement_names;
        self.next_room_persistence_version = persisted_rooms
            .values()
            .map(|room| room.version)
            .max()
            .unwrap_or(0);
        self.apply_persisted_rooms_snapshot(persisted_rooms);
    }

    pub(crate) fn apply_permanent_rooms_snapshot(&mut self) {
        let now_seconds = self.current_time_seconds();
        for room_name in self.permanent_rooms.clone() {
            self.room_playlists
                .entry(room_name.clone())
                .or_insert_with(|| RoomPlaylistState {
                    files: Vec::new(),
                    // Syncplay seeds file-backed permanent room placeholders
                    // with index zero even while their playlist is empty.
                    // Preserve that wire-visible join snapshot until a client
                    // explicitly changes the index.
                    index: Some(0),
                    epoch: 0,
                });
            self.room_controllers.entry(room_name.clone()).or_default();
            self.room_participant_status_scopes
                .entry(room_name.clone())
                .or_default();
            self.room_playback_states
                .entry(room_name)
                .or_insert_with(|| RoomPlaybackState::new_at(now_seconds));
        }
    }

    pub(crate) fn reconcile_permanent_rooms_snapshot(
        &mut self,
        previous_permanent_rooms: &BTreeSet<String>,
    ) {
        let removed_rooms = previous_permanent_rooms
            .difference(&self.permanent_rooms)
            .cloned()
            .collect::<Vec<_>>();
        for room_name in removed_rooms {
            if self.clients_in_room(&room_name).is_empty()
                && self.room_playlist_state(&room_name).files.is_empty()
            {
                self.remove_room_runtime_state(&room_name);
                self.delete_persisted_room_if_needed(&room_name)
                    .expect("enqueueing a permanent-room deletion cannot fail");
            }
        }
        self.apply_permanent_rooms_snapshot();
    }

    pub(crate) fn room_is_persistent(&self, room_name: &str) -> bool {
        self.persistent_rooms_enabled && !room_name_is_marked_temporary(room_name)
    }

    pub(crate) fn room_is_permanent(&self, room_name: &str) -> bool {
        self.permanent_rooms.contains(room_name)
    }

    pub(crate) fn room_should_be_retained_when_empty(&self, room_name: &str) -> bool {
        self.room_is_persistent(room_name) && !self.room_playlist_state(room_name).files.is_empty()
    }

    pub(crate) fn persistent_room_creation_identity(&self, client_id: &str) -> String {
        let source_identity = self
            .client_peer_ips
            .get(client_id)
            .map(|peer_ip| {
                let normalized_peer_ip = match peer_ip.trim().parse::<IpAddr>() {
                    Ok(IpAddr::V6(address)) => address
                        .to_ipv4_mapped()
                        .map(IpAddr::V4)
                        .unwrap_or(IpAddr::V6(address))
                        .to_string(),
                    Ok(address) => address.to_string(),
                    Err(_) => peer_ip.trim().to_ascii_lowercase(),
                };
                format!("peer-ip:{normalized_peer_ip}")
            })
            // Direct-runtime callers do not have a transport peer address. Keep
            // their quota namespace session-scoped; production network sessions
            // are keyed by peer IP above.
            .unwrap_or_else(|| format!("session:{client_id}"));
        self.persistent_room_owner_bucket(&source_identity)
    }

    fn persistent_room_owner_bucket(&self, source_identity: &str) -> String {
        let mut inner_pad = [0x36_u8; 64];
        let mut outer_pad = [0x5c_u8; 64];
        for (index, secret_byte) in self.persistent_room_quota_secret.iter().enumerate() {
            inner_pad[index] ^= secret_byte;
            outer_pad[index] ^= secret_byte;
        }
        let mut inner = Sha256::new();
        inner.update(inner_pad);
        inner.update(source_identity.as_bytes());
        let inner_digest = inner.finalize();
        let mut outer = Sha256::new();
        outer.update(outer_pad);
        outer.update(inner_digest);
        let digest = outer.finalize();
        let mut bucket = String::with_capacity("quota:v1:".len() + digest.len() * 2);
        bucket.push_str("quota:v1:");
        for byte in digest {
            write!(&mut bucket, "{byte:02x}").expect("writing to String cannot fail");
        }
        bucket
    }

    pub(crate) fn persistent_room_creation_required(
        &self,
        room_name: &str,
        files: &[String],
    ) -> bool {
        !files.is_empty()
            && self.room_is_persistent(room_name)
            && !self.room_is_permanent(room_name)
            && self.room_playlist_state(room_name).files.is_empty()
    }

    pub(crate) fn persistent_room_creation_allowed(
        &self,
        room_name: &str,
        files: &[String],
        identity: &str,
        now_seconds: f64,
    ) -> bool {
        if !self.persistent_room_creation_required(room_name, files) {
            return true;
        }

        let retained_client_rooms = self
            .room_playlists
            .iter()
            .filter(|(existing_room, playlist)| {
                !playlist.files.is_empty()
                    && self.room_is_persistent(existing_room)
                    && !self.room_is_permanent(existing_room)
            })
            .count();
        if retained_client_rooms >= self.max_persistent_rooms {
            return false;
        }
        let rooms_owned_by_identity = self
            .persistent_room_owner_by_room
            .values()
            .filter(|owner| owner.as_str() == identity)
            .count();
        if rooms_owned_by_identity >= self.max_persistent_rooms_per_identity {
            return false;
        }
        self.persistent_room_last_creation_by_identity
            .get(identity)
            .is_none_or(|last_creation| {
                now_seconds >= *last_creation
                    && now_seconds - *last_creation
                        >= self.persistent_room_creation_cooldown_seconds
            })
    }

    pub(crate) fn record_persistent_room_creation(
        &mut self,
        room_name: &str,
        identity: String,
        now_seconds: f64,
    ) {
        self.persistent_room_owner_by_room
            .insert(room_name.to_owned(), identity.clone());
        self.persistent_room_created_at_by_room
            .insert(room_name.to_owned(), now_seconds);
        self.persistent_room_last_creation_by_identity
            .insert(identity, now_seconds);
        let active_owners: BTreeSet<_> = self
            .persistent_room_owner_by_room
            .values()
            .cloned()
            .collect();
        self.persistent_room_last_creation_by_identity
            .retain(|identity, last_creation| {
                active_owners.contains(identity)
                    || (now_seconds >= *last_creation
                        && now_seconds - *last_creation
                            <= self.persistent_room_creation_cooldown_seconds)
            });
    }

    pub(crate) fn release_persistent_room_ownership(&mut self, room_name: &str) {
        self.persistent_room_owner_by_room.remove(room_name);
        self.persistent_room_created_at_by_room.remove(room_name);
    }

    pub(crate) fn expire_inactive_persistent_rooms_at(
        &mut self,
        now_seconds: f64,
    ) -> Result<bool, ServerRuntimeError> {
        if self.persistent_room_inactivity_expiry_seconds <= 0.0 {
            return Ok(false);
        }
        let expired_rooms: Vec<_> = self
            .persistent_room_last_activity_at
            .iter()
            .filter(|(room_name, last_activity)| {
                self.clients_in_room(room_name).is_empty()
                    && self.room_is_persistent(room_name)
                    && !self.room_is_permanent(room_name)
                    && now_seconds >= **last_activity
                    && now_seconds - **last_activity
                        >= self.persistent_room_inactivity_expiry_seconds
            })
            .map(|(room_name, _)| room_name.clone())
            .collect();
        for room_name in &expired_rooms {
            self.remove_room_runtime_state(room_name);
            self.delete_persisted_room_if_needed(room_name)?;
        }
        Ok(!expired_rooms.is_empty())
    }

    pub(crate) fn persist_room_if_needed(
        &mut self,
        room_name: &str,
    ) -> Result<(), ServerRuntimeError> {
        self.persist_room_at_if_needed(room_name, self.current_time_seconds())
    }

    fn persist_room_at_if_needed(
        &mut self,
        room_name: &str,
        now_seconds: f64,
    ) -> Result<(), ServerRuntimeError> {
        if !self.room_is_persistent(room_name) {
            return Ok(());
        }
        self.persistent_room_last_activity_at
            .insert(room_name.to_owned(), now_seconds);
        if self.room_persistence.is_none() {
            return Ok(());
        }
        let playlist = self.room_playlist_state(room_name).clone();
        if !self.persisted_room_names.contains(room_name)
            && !self.room_is_permanent(room_name)
            && playlist.files.is_empty()
        {
            return Ok(());
        }
        let playback = self.room_playback_state_at(room_name, now_seconds);
        let version = self.next_room_persistence_version();
        let owner_bucket = self.persistent_room_owner_by_room.get(room_name).cloned();
        let created_at_seconds = self
            .persistent_room_created_at_by_room
            .get(room_name)
            .copied()
            .unwrap_or(now_seconds);
        self.persisted_room_names.insert(room_name.to_owned());
        self.room_persistence
            .as_ref()
            .expect("room persistence presence checked above")
            .enqueue(ServerPersistenceEffect::SaveRoom {
                room_name: room_name.to_owned(),
                files: playlist.files,
                playlist_index: playlist.index,
                position: playback.position,
                last_activity_at_seconds: now_seconds,
                owner_bucket,
                created_at_seconds,
                version,
            });
        Ok(())
    }

    fn persistent_room_activity_heartbeat_interval_seconds(&self) -> f64 {
        if self.persistent_room_inactivity_expiry_seconds <= 0.0 {
            return PERSISTENT_ROOM_ACTIVITY_HEARTBEAT_MAX_INTERVAL_SECONDS;
        }
        (self.persistent_room_inactivity_expiry_seconds / 2.0).clamp(
            SERVER_NETWORK_TICK_INTERVAL_SECONDS,
            PERSISTENT_ROOM_ACTIVITY_HEARTBEAT_MAX_INTERVAL_SECONDS,
        )
    }

    pub(crate) fn persist_occupied_room_activity_if_due_at(
        &mut self,
        room_name: &str,
        now_seconds: f64,
    ) -> Result<(), ServerRuntimeError> {
        if !self.room_is_persistent(room_name)
            || self.clients_in_room(room_name).is_empty()
            || (!self.persisted_room_names.contains(room_name)
                && !self.room_is_permanent(room_name)
                && self.room_playlist_state(room_name).files.is_empty())
            || self
                .persistent_room_last_activity_at
                .get(room_name)
                .is_some_and(|last_activity| {
                    now_seconds >= *last_activity
                        && now_seconds - *last_activity
                            < self.persistent_room_activity_heartbeat_interval_seconds()
                })
        {
            return Ok(());
        }
        self.persist_room_at_if_needed(room_name, now_seconds)
    }

    pub(crate) fn persist_occupied_room_activity_if_due_at_for_all_rooms(
        &mut self,
        now_seconds: f64,
    ) -> Result<(), ServerRuntimeError> {
        let occupied_rooms = self
            .sessions
            .values()
            .map(|session| session.room.clone())
            .collect::<BTreeSet<_>>();
        for room_name in occupied_rooms {
            self.persist_occupied_room_activity_if_due_at(&room_name, now_seconds)?;
        }
        Ok(())
    }

    pub(crate) fn delete_persisted_room_if_needed(
        &mut self,
        room_name: &str,
    ) -> Result<(), ServerRuntimeError> {
        if self.room_persistence.is_none() || !self.persisted_room_names.remove(room_name) {
            return Ok(());
        }
        let version = self.next_room_persistence_version();
        self.room_persistence
            .as_ref()
            .expect("room persistence presence checked above")
            .enqueue(ServerPersistenceEffect::DeleteRoom {
                room_name: room_name.to_owned(),
                version,
            });
        Ok(())
    }

    fn next_room_persistence_version(&mut self) -> u64 {
        self.next_room_persistence_version = self
            .next_room_persistence_version
            .checked_add(1)
            .filter(|version| *version <= i64::MAX as u64)
            .expect("room persistence version exhausted");
        self.next_room_persistence_version
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
        self.remove_room_runtime_state(room_name);
        self.delete_persisted_room_if_needed(room_name)?;
        Ok(())
    }

    fn remove_room_runtime_state(&mut self, room_name: &str) {
        // The final participant may already have removed the readiness room,
        // but every alternate retirement path must still close an active gate
        // before its canonical barrier is discarded.
        let _ = self.clear_readiness_gate(room_name);
        self.room_controllers.remove(room_name);
        self.room_playlists.remove(room_name);
        self.room_playback_states.remove(room_name);
        self.room_playback_barriers.remove(room_name);
        self.room_buffering_controls.remove(room_name);
        self.room_participant_status_scopes.remove(room_name);
        self.room_participant_status_snapshot_revisions
            .remove(room_name);
        self.participant_status_snapshot_cache.remove(room_name);
        self.playback_barrier_request_tombstones
            .retain(|(tombstone_room, _), _| tombstone_room != room_name);
        self.playback_barrier_new_identity_rate_by_room
            .remove(room_name);
        self.persistent_room_owner_by_room.remove(room_name);
        self.persistent_room_created_at_by_room.remove(room_name);
        self.persistent_room_last_activity_at.remove(room_name);
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
        self.room_participant_status_scopes
            .entry(room_name.to_owned())
            .or_default();
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

    pub(crate) fn seed_client_playback_state(
        &mut self,
        client_id: &str,
        position: Option<f64>,
        now_seconds: f64,
    ) {
        let position = position.filter(|position| position.is_finite());
        let transport_revision = self
            .sessions
            .get(client_id)
            .and_then(|session| self.transport_authority_revision_for_room(&session.room));
        self.client_playback_states.insert(
            client_id.to_owned(),
            ClientPlaybackState::new(position, now_seconds, transport_revision),
        );
    }

    pub(crate) fn record_client_playback_state_sample(
        &mut self,
        client_id: &str,
        position: Option<f64>,
        now_seconds: f64,
        transport_revision: Option<u64>,
    ) {
        let position = position.filter(|position| position.is_finite());
        let playback_state = self
            .client_playback_states
            .entry(client_id.to_owned())
            .or_insert_with(|| ClientPlaybackState::new(None, now_seconds, transport_revision));
        if let Some(position) = position {
            playback_state.position = Some(position);
        }
        playback_state.updated_at_seconds = now_seconds;
        playback_state.transport_revision = transport_revision;
    }

    pub(crate) fn seed_room_client_playback_states(
        &mut self,
        room_name: &str,
        position: f64,
        now_seconds: f64,
    ) {
        if !position.is_finite() {
            return;
        }
        for client_id in self.clients_in_room(room_name) {
            self.seed_client_playback_state(&client_id, Some(position), now_seconds);
        }
    }

    pub(crate) fn slowest_room_playback_client_at(
        &self,
        room_name: &str,
        room_paused: bool,
        now_seconds: f64,
    ) -> Option<(String, f64)> {
        let controlled_room = self
            .room_password_provider
            .is_controlled_room_name(room_name);
        let current_transport_revision = self.transport_authority_revision_for_room(room_name);
        let mut slowest: Option<(String, f64, u64)> = None;
        for (client_id, session) in &self.sessions {
            if session.room != room_name || self.playback_barrier_fenced_clients.contains(client_id)
            {
                continue;
            }
            if controlled_room && !self.user_is_room_controller(&session.username, room_name) {
                continue;
            }
            if session.file.is_none() {
                continue;
            }
            let Some(position) = self
                .client_playback_states
                .get(client_id)
                .and_then(|state| {
                    if state.transport_revision.is_some()
                        && state.transport_revision != current_transport_revision
                    {
                        return None;
                    }
                    state.position_at(room_paused, now_seconds)
                })
            else {
                continue;
            };
            let room_join_order = self.client_room_join_order(client_id);
            if slowest
                .as_ref()
                .is_none_or(|(_, slowest_position, slowest_room_join_order)| {
                    position < *slowest_position
                        || (position == *slowest_position
                            && room_join_order < *slowest_room_join_order)
                })
            {
                slowest = Some((session.username.clone(), position, room_join_order));
            }
        }
        slowest.map(|(username, position, _)| (username, position))
    }

    pub(crate) fn refresh_room_playback_state_from_clients_at(
        &mut self,
        room_name: &str,
        now_seconds: f64,
    ) -> RoomPlaybackState {
        let current = self.room_playback_state(room_name);
        let age_seconds = now_seconds - current.updated_at_seconds;
        if !age_seconds.is_finite() || age_seconds <= SERVER_STATE_INTERVAL_SECONDS {
            return current.aged_at(now_seconds);
        }
        let Some((set_by, position)) =
            self.slowest_room_playback_client_at(room_name, current.paused, now_seconds)
        else {
            return current.aged_at(now_seconds);
        };
        let room_playback = self.room_playback_state_mut(room_name);
        room_playback.position = position;
        room_playback.updated_at_seconds = now_seconds;
        room_playback.set_by = Some(set_by);
        room_playback.clone()
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
        if ![latency_calculation, sender_rtt]
            .into_iter()
            .all(|value| value.is_finite() && value >= 0.0)
            || sender_rtt > PROTOCOL_TIMEOUT_SECONDS
        {
            return;
        }

        let wall_now = self.current_time_seconds();
        let elapsed_now = self.local_time_seconds();
        let Some(state_counters) = self.client_state_counters.get_mut(client_id) else {
            return;
        };
        state_counters
            .outstanding_ping_challenges
            .retain(|(_, sent)| (0.0..=PROTOCOL_TIMEOUT_SECONDS).contains(&(elapsed_now - sent)));
        // Echoes are identities, not peer-provided elapsed durations. Removing
        // the matched challenge makes duplicate/reordered delivery harmless.
        let Some(index) = state_counters
            .outstanding_ping_challenges
            .iter()
            .position(|(wire, _)| wire.to_bits() == latency_calculation.to_bits())
        else {
            return;
        };
        let (_, sent_at) = state_counters
            .outstanding_ping_challenges
            .remove(index)
            .expect("matched challenge");
        let current_rtt_seconds = elapsed_now - sent_at;
        if !(0.0..=PROTOCOL_TIMEOUT_SECONDS).contains(&current_rtt_seconds) {
            return;
        }
        state_counters.ping_rtt_seconds = current_rtt_seconds;
        if state_counters.ping_average_rtt_seconds == 0.0 {
            state_counters.ping_average_rtt_seconds = current_rtt_seconds;
        }
        state_counters.ping_average_rtt_seconds = state_counters.ping_average_rtt_seconds
            * PING_MOVING_AVERAGE_WEIGHT
            + current_rtt_seconds * (1.0 - PING_MOVING_AVERAGE_WEIGHT);
        state_counters.ping_forward_delay_seconds = state_counters.ping_average_rtt_seconds / 2.0
            + (current_rtt_seconds - sender_rtt).max(0.0);
        state_counters.ping_forward_delay_seconds = state_counters
            .ping_forward_delay_seconds
            .clamp(0.0, PROTOCOL_TIMEOUT_SECONDS);
        state_counters.ping_metrics_observed_at_seconds = Some(elapsed_now);
        state_counters.ping_metrics_wall_observed_at_seconds = Some(wall_now);
    }

    fn record_ping_challenge(&mut self, client_id: &str, wire_timestamp: f64) -> f64 {
        let mut wire_timestamp = if wire_timestamp.is_finite() && wire_timestamp > 0.0 {
            wire_timestamp.min(1.0e15)
        } else {
            0.0
        };
        if let Some(previous) = self.last_ping_challenge_timestamp
            && wire_timestamp <= previous
        {
            wire_timestamp = f64::from_bits(previous.to_bits().saturating_add(1));
        }
        self.last_ping_challenge_timestamp = Some(wire_timestamp);
        let sent_at = self.local_time_seconds();
        let challenges = &mut self
            .client_state_counters
            .entry(client_id.to_owned())
            .or_default()
            .outstanding_ping_challenges;
        challenges.retain(|(wire, sent)| {
            (0.0..=PROTOCOL_TIMEOUT_SECONDS).contains(&(sent_at - sent))
                && wire.to_bits() != wire_timestamp.to_bits()
        });
        let retired = challenges.len().saturating_sub(63);
        challenges.drain(..retired);
        challenges.push_back((wire_timestamp, sent_at));
        wire_timestamp
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
            .filter(|state| {
                state
                    .ping_metrics_observed_at_seconds
                    .is_some_and(|observed| {
                        (0.0..=PROTOCOL_TIMEOUT_SECONDS)
                            .contains(&(self.local_time_seconds() - observed))
                    })
            })
            .map(|state_counters| state_counters.ping_forward_delay_seconds)
            .unwrap_or_default()
    }

    /// Returns the server-owned one-way estimate already used for canonical
    /// inbound playback timing. Participant projection accepts it only while
    /// both the estimate and its monotonic observation age are finite and
    /// within the live protocol timeout. Clock rollback and ancient ping
    /// echoes therefore fail closed instead of manufacturing timing evidence.
    pub(crate) fn participant_status_forward_delay_ms_at(
        &mut self,
        client_id: &str,
        now_seconds: f64,
    ) -> Option<u64> {
        let elapsed = self.local_time_for_wall_sample(now_seconds);
        let state_counters = self.client_state_counters.get_mut(client_id)?;
        let observed_at_seconds = state_counters.ping_metrics_observed_at_seconds?;
        let observation_age_seconds = elapsed - observed_at_seconds;
        if !observation_age_seconds.is_finite()
            || !(0.0..=PROTOCOL_TIMEOUT_SECONDS).contains(&observation_age_seconds)
            || state_counters
                .ping_metrics_wall_observed_at_seconds
                .is_some_and(|observed| now_seconds < observed)
        {
            state_counters.ping_metrics_observed_at_seconds = None;
            return None;
        }
        let forward_delay_seconds = state_counters.ping_forward_delay_seconds;
        if !forward_delay_seconds.is_finite()
            || !(0.0..=PROTOCOL_TIMEOUT_SECONDS).contains(&forward_delay_seconds)
        {
            state_counters.ping_metrics_observed_at_seconds = None;
            return None;
        }
        Some((forward_delay_seconds * 1_000.0).ceil() as u64)
    }

    pub(crate) fn take_client_passthrough_state_metadata(
        &mut self,
        client_id: &str,
    ) -> (Option<f64>, Option<u32>) {
        self.take_client_passthrough_state_metadata_at(client_id, self.current_time_seconds())
    }

    pub(crate) fn take_client_passthrough_state_metadata_at(
        &mut self,
        client_id: &str,
        now_seconds: f64,
    ) -> (Option<f64>, Option<u32>) {
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
        do_seek: impl Into<Option<bool>>,
        set_by: Option<&str>,
    ) -> ProtocolMessage {
        let server_ignoring_counter = self.next_server_ignoring_counter(client_id);
        let server_rtt_seconds = self.server_rtt_seconds(client_id);
        let (pending_client_latency, pending_client_ignoring) =
            self.take_client_passthrough_state_metadata(client_id);
        let room = self
            .sessions
            .get(client_id)
            .map(|session| session.room.clone());
        let transport_revision = room
            .as_deref()
            .and_then(|room| self.transport_authority_revision_for_room(room));
        let participant_status = self.sessions.get(client_id).and_then(|session| {
            session.capabilities.participant_status_v1.then(|| {
                ParticipantStatusStateExtension::new()
                    .with_scope(self.participant_status_scope_for_room(&session.room))
            })
        });
        let wire_timestamp = self.current_time_seconds();
        let wire_timestamp = self.record_ping_challenge(client_id, wire_timestamp);
        state_sync_message(
            position,
            paused,
            do_seek,
            StateSyncOptions {
                set_by,
                transport_revision,
                server_ignoring_counter: Some(server_ignoring_counter),
                client_latency_calculation: pending_client_latency,
                client_ignoring_counter: pending_client_ignoring,
                server_rtt_seconds,
                latency_calculation_seconds: Some(wire_timestamp),
                participant_status,
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

    pub(crate) fn room_uses_playback_lifecycle_authority(&self, room_name: &str) -> bool {
        self.sessions
            .values()
            .any(|session| session.room == room_name && session.capabilities.playback_barrier_v1)
    }

    pub(crate) fn legacy_readiness_chat_clients_in_room(&self, room_name: &str) -> Vec<String> {
        self.sessions
            .iter()
            .filter(|(_, session)| {
                session.room == room_name
                    && client_version_meets_minimum(&session.version, LEGACY_CHAT_MIN_VERSION)
                    && !session.capabilities.remote_readiness
            })
            .map(|(client_id, _)| client_id.clone())
            .collect()
    }

    pub(crate) fn chat_clients_in_room(&self, room_name: &str) -> Vec<String> {
        self.sessions
            .iter()
            .filter(|(_, session)| {
                session.room == room_name
                    && client_version_meets_minimum(&session.version, LEGACY_CHAT_MIN_VERSION)
            })
            .map(|(client_id, _)| client_id.clone())
            .collect()
    }

    pub(crate) fn clients_all(&self) -> Vec<String> {
        self.sessions.keys().cloned().collect()
    }

    pub(crate) fn clients_receiving_to_gui_only_list_updates(
        &self,
        room_name: Option<&str>,
    ) -> Vec<String> {
        self.sessions
            .iter()
            .filter(|(_, session)| {
                session.capabilities.ui_mode_advertised
                    && (!self.isolate_rooms
                        || room_name.is_some_and(|room_name| session.room == room_name))
            })
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
        self.stored_user_ready(username, room_name)
    }

    pub(crate) fn stored_user_ready(&self, username: &str, room_name: &str) -> Option<bool> {
        self.domain.users_in_room(room_name).and_then(|users| {
            users
                .into_iter()
                .find(|user| user.username == username)
                .and_then(|user| user.ready)
        })
    }

    pub(crate) fn file_payload_for_client_from_source(
        &self,
        client_id: &str,
        source_client_id: &str,
        file: &ServerSharedFile,
    ) -> Value {
        file.to_wire_value(
            self.client_session_supports_media_match(client_id) && client_id != source_client_id,
        )
    }

    pub(crate) fn playlist_change_message_for_client(
        &self,
        client_id: &str,
        files: Vec<String>,
        set_by: Option<&str>,
        epoch: u64,
    ) -> ProtocolMessage {
        let mut playlist_change = playlist_change_with_plex_sidecar(
            files,
            self.client_session_supports_sorotte_plex_playlist_uris(client_id),
        )
        .with_playlist_epoch(epoch);
        playlist_change = if let Some(set_by) = set_by {
            playlist_change.with_user(set_by)
        } else {
            playlist_change.with_null_user()
        };
        ProtocolMessage::set(SetPayload::new().with_playlist_change(playlist_change))
    }

    fn client_session_supports_media_match(&self, client_id: &str) -> bool {
        self.sessions
            .get(client_id)
            .is_some_and(|session| session.capabilities.media_match)
    }

    fn client_session_supports_sorotte_plex_playlist_uris(&self, client_id: &str) -> bool {
        self.sessions
            .get(client_id)
            .is_some_and(|session| session.capabilities.plex_playlist_uris)
    }

    fn sanitize_list_rooms_snapshot_for_client(
        &self,
        client_id: &str,
        rooms: &mut BTreeMap<String, BTreeMap<String, ListUserEntry>>,
    ) {
        let supports_media_match = self.client_session_supports_media_match(client_id);
        let own_username = self
            .sessions
            .get(client_id)
            .map(|session| session.username.as_str());
        for room_entries in rooms.values_mut() {
            for (username, entry) in room_entries.iter_mut() {
                if supports_media_match && own_username != Some(username.as_str()) {
                    continue;
                }
                let Some(file) = entry.file.as_mut() else {
                    continue;
                };
                let Some(file_object) = file.as_object_mut() else {
                    continue;
                };
                file_object.remove("mediaMatch");
            }
        }
    }

    pub(crate) fn list_rooms_snapshot_for_client(
        &self,
        client_id: &str,
    ) -> BTreeMap<String, BTreeMap<String, ListUserEntry>> {
        let mut rooms = if self.isolate_rooms {
            let Some(session) = self.sessions.get(client_id) else {
                return BTreeMap::new();
            };
            let mut all_rooms = self.list_rooms_snapshot();
            let mut rooms = BTreeMap::new();
            if let Some(room_entries) = all_rooms.remove(&session.room) {
                rooms.insert(session.room.clone(), room_entries);
            }
            rooms
        } else {
            self.list_rooms_snapshot()
        };
        if self
            .sessions
            .get(client_id)
            .is_some_and(|session| session.capabilities.is_gui_user())
        {
            self.add_empty_room_dummy_entries(&mut rooms);
        }
        self.sanitize_list_rooms_snapshot_for_client(client_id, &mut rooms);
        self.compact_list_for_limit(client_id, &mut rooms);
        rooms
    }

    pub(crate) fn list_rooms_snapshot(&self) -> BTreeMap<String, BTreeMap<String, ListUserEntry>> {
        let mut rooms = BTreeMap::new();
        for session in self.sessions.values() {
            let ready = self.user_ready(&session.username, &session.room);
            let mut entry = ListUserEntry::new()
                .with_position(0.0)
                .with_file(
                    session
                        .file
                        .as_ref()
                        .map(|file| file.to_wire_value(true))
                        .unwrap_or_else(|| json!({})),
                )
                .with_controller(self.user_is_room_controller(&session.username, &session.room));
            if let Some(ready) = ready {
                entry = entry.with_is_ready(ready);
            }
            entry = entry.with_features(session.capabilities.to_wire_value());
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
            rooms.entry(room_name).or_default().insert(
                frame_limits::empty_room_identity(dummy_count),
                legacy_dummy_list_entry(),
            );
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
    ) -> Option<String> {
        let all_names: BTreeSet<String> = self
            .sessions
            .iter()
            .filter(|(client_id, _)| {
                excluded_client_id.is_none_or(|excluded| *client_id != excluded)
            })
            .map(|(_, session)| session.username.to_ascii_lowercase())
            .collect();

        if !all_names.contains(&username.to_ascii_lowercase()) {
            return Some(username.to_owned());
        }

        // Legacy Syncplay strips a colliding trailing-underscore run back to
        // its stem, then appends underscores until the name is free. Keep that
        // externally visible allocation order, but reserve suffix space inside
        // the advertised scalar limit so a hostile collision sequence remains
        // finite and bounded.
        let trimmed = username.trim_end_matches('_');
        let collision_stem = if username.ends_with('_') {
            if trimmed.is_empty() { "_" } else { trimmed }
        } else {
            username
        };
        for suffix_chars in 1..=self.max_username_length {
            let prefix_chars = self.max_username_length - suffix_chars;
            let mut chosen_username = collision_stem
                .chars()
                .take(prefix_chars)
                .collect::<String>();
            chosen_username.extend(std::iter::repeat_n('_', suffix_chars));
            if !all_names.contains(&chosen_username.to_ascii_lowercase()) {
                return Some(chosen_username);
            }
        }
        None
    }
}

#[cfg(test)]
mod ping_challenge_tests {
    use super::*;

    #[test]
    fn wire_challenges_canonicalize_invalid_clocks_and_remain_unique() {
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0, -0.0, 0.0] {
            let mut runtime = ServerRuntime::new();
            assert_eq!(runtime.record_ping_challenge("peer", invalid).to_bits(), 0);
            let next = runtime.record_ping_challenge("peer", invalid);
            assert!(next.is_finite() && next > 0.0);
        }
        let mut runtime = ServerRuntime::new();
        assert_eq!(runtime.record_ping_challenge("peer", 12.5), 12.5);
        assert_eq!(runtime.record_ping_challenge("peer", f64::MAX), 1.0e15);
    }
}
