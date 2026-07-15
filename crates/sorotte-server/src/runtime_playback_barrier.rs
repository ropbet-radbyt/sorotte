use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoomBufferingTransition {
    Pause,
    Resume,
}

impl ServerRuntime {
    /// Revokes an older transport after its application-level playback
    /// operation has been rebound to a newer connection. The session is kept
    /// until the network reports the actual disconnect so cleanup remains
    /// ordered, but no further protocol input from it is authoritative.
    fn fence_and_close_playback_barrier_transport(&mut self, client_id: &str) {
        self.playback_barrier_fenced_clients
            .insert(client_id.to_owned());
        if !self.pending_transport_actions.iter().any(|action| {
            action.client_id == client_id && action.action == ServerTransportAction::Close
        }) {
            self.pending_transport_actions
                .push(DirectedTransportAction::new(
                    client_id,
                    ServerTransportAction::Close,
                ));
        }
    }

    /// Rejects every command from a superseded transport while reinforcing
    /// the close request in case input raced the original network teardown.
    pub(crate) fn reject_fenced_playback_barrier_transport(&mut self, client_id: &str) -> bool {
        if !self.playback_barrier_fenced_clients.contains(client_id) {
            return false;
        }
        self.fence_and_close_playback_barrier_transport(client_id);
        true
    }

    fn allocate_playback_barrier_generation(&mut self) -> Option<u64> {
        let generation = self.next_playback_barrier_generation.checked_add(1)?;
        self.next_playback_barrier_generation = generation;
        Some(generation)
    }

    fn playback_barrier_request_clock_seconds(&self) -> f64 {
        #[cfg(test)]
        if let Some(seconds) = self.playback_barrier_request_clock_override_seconds {
            return seconds;
        }
        self.playback_barrier_request_clock_started_at
            .elapsed()
            .as_secs_f64()
    }

    pub(crate) fn prune_playback_barrier_request_tombstones(&mut self) {
        let now_seconds = self.playback_barrier_request_clock_seconds();
        self.playback_barrier_request_tombstones
            .retain(|_, tombstone| tombstone.retain_until_seconds > now_seconds);
        self.prune_playback_barrier_new_identity_rate_history(now_seconds);
    }

    fn playback_barrier_retry_after_millis(delay_seconds: f64) -> u64 {
        let delay_millis = if delay_seconds.is_finite() && delay_seconds > 0.0 {
            (delay_seconds * 1_000.0).ceil() as u64
        } else {
            PLAYBACK_BARRIER_REQUEST_RETRY_MIN_MILLIS
        };
        delay_millis.clamp(
            PLAYBACK_BARRIER_REQUEST_RETRY_MIN_MILLIS,
            PLAYBACK_BARRIER_REQUEST_RETRY_MAX_MILLIS,
        )
    }

    fn prune_playback_barrier_new_identity_rate_history(&mut self, now_seconds: f64) {
        let cutoff_seconds = now_seconds
            - self
                .playback_barrier_new_identity_rate_policy
                .window_seconds;
        for history in self
            .playback_barrier_new_identity_rate_by_client
            .values_mut()
            .chain(self.playback_barrier_new_identity_rate_by_room.values_mut())
        {
            while history
                .front()
                .is_some_and(|event| event.observed_at_seconds <= cutoff_seconds)
            {
                history.pop_front();
            }
        }
        self.playback_barrier_new_identity_rate_by_client
            .retain(|_, history| !history.is_empty());
        self.playback_barrier_new_identity_rate_by_room
            .retain(|_, history| !history.is_empty());
    }

    /// Records a genuinely new application operation identity, returning the
    /// bounded retry delay when either the connection or room budget is full.
    /// An exact retry of an identity already observed in the current window is
    /// exempt so transient replay-capacity pressure cannot extend itself.
    fn playback_barrier_new_identity_retry_after_millis(
        &mut self,
        client_id: &str,
        room_name: &str,
        request_id: Option<&str>,
        request_nonce: u64,
    ) -> Option<u64> {
        let now_seconds = self.playback_barrier_request_clock_seconds();
        self.prune_playback_barrier_new_identity_rate_history(now_seconds);
        let username = self
            .sessions
            .get(client_id)
            .map(|session| session.username.clone())?;

        let exact_client_operation_seen = self
            .playback_barrier_new_identity_rate_by_client
            .get(client_id)
            .is_some_and(|history| {
                history.iter().any(|event| {
                    event.matches_operation(&username, room_name, request_id, request_nonce)
                })
            });
        let exact_room_operation_seen = request_id.is_some_and(|request_id| {
            self.playback_barrier_new_identity_rate_by_room
                .get(room_name)
                .is_some_and(|history| {
                    history.iter().any(|event| {
                        event
                            .request_id
                            .as_ref()
                            .map(|request_id| request_id.0.as_str())
                            == Some(request_id)
                            && event.request_nonce == request_nonce
                    })
                })
        });
        if exact_client_operation_seen || exact_room_operation_seen {
            return None;
        }

        let client_retry_at = self
            .playback_barrier_new_identity_rate_by_client
            .get(client_id)
            .filter(|history| {
                history.len()
                    >= self
                        .playback_barrier_new_identity_rate_policy
                        .max_per_client
            })
            .and_then(|history| history.front())
            .map(|event| {
                event.observed_at_seconds
                    + self
                        .playback_barrier_new_identity_rate_policy
                        .window_seconds
            });
        let room_retry_at = self
            .playback_barrier_new_identity_rate_by_room
            .get(room_name)
            .filter(|history| {
                history.len() >= self.playback_barrier_new_identity_rate_policy.max_per_room
            })
            .and_then(|history| history.front())
            .map(|event| {
                event.observed_at_seconds
                    + self
                        .playback_barrier_new_identity_rate_policy
                        .window_seconds
            });
        if let Some(retry_at_seconds) = client_retry_at
            .into_iter()
            .chain(room_retry_at)
            .max_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal))
        {
            return Some(Self::playback_barrier_retry_after_millis(
                retry_at_seconds - now_seconds,
            ));
        }

        let event = PlaybackBarrierNewIdentityRateEvent {
            username,
            room_name: room_name.to_owned(),
            request_id: request_id.map(PlaybackBarrierRequestId::new),
            request_nonce,
            observed_at_seconds: now_seconds,
        };
        self.playback_barrier_new_identity_rate_by_client
            .entry(client_id.to_owned())
            .or_default()
            .push_back(event.clone());
        self.playback_barrier_new_identity_rate_by_room
            .entry(room_name.to_owned())
            .or_default()
            .push_back(event);
        None
    }

    fn displaced_playback_barrier_requests(
        &self,
        room_name: &str,
        replacement_request_id: Option<&str>,
        include_barrier: bool,
        include_buffering_control: bool,
    ) -> BTreeMap<PlaybackBarrierRequestId, DisplacedPlaybackBarrierRequest> {
        let mut displaced = BTreeMap::new();
        if include_barrier
            && let Some(prepare) = self
                .room_playback_barriers
                .get(room_name)
                .map(|barrier| &barrier.prepare)
            && let Some(request_id) = prepare.request_id.as_deref()
            && replacement_request_id != Some(request_id)
        {
            displaced.insert(
                PlaybackBarrierRequestId::new(request_id),
                DisplacedPlaybackBarrierRequest {
                    request_nonce: prepare.request_nonce,
                    logical_media_id_digest: Some(playback_barrier_logical_media_id_digest(
                        &prepare.logical_media_id,
                    )),
                    media_generation: prepare.media_generation,
                },
            );
        }
        if include_buffering_control
            && let Some(config) = self
                .room_buffering_controls
                .get(room_name)
                .map(|control| &control.config)
            && let Some(request_id) = config.request_id.as_deref()
            && replacement_request_id != Some(request_id)
        {
            displaced
                .entry(PlaybackBarrierRequestId::new(request_id))
                .or_insert(DisplacedPlaybackBarrierRequest {
                    request_nonce: config.request_nonce,
                    logical_media_id_digest: None,
                    media_generation: config.media_generation,
                });
        }
        displaced
    }

    fn can_retain_displaced_playback_barrier_requests(
        &mut self,
        room_name: &str,
        displaced: &BTreeMap<PlaybackBarrierRequestId, DisplacedPlaybackBarrierRequest>,
    ) -> bool {
        self.prune_playback_barrier_request_tombstones();
        let additional_global = displaced
            .keys()
            .filter(|request_id| {
                !self
                    .playback_barrier_request_tombstones
                    .contains_key(&(room_name.to_owned(), (*request_id).clone()))
            })
            .count();
        let room_count = self
            .playback_barrier_request_tombstones
            .keys()
            .filter(|(tombstone_room, _)| tombstone_room == room_name)
            .count();
        room_count.saturating_add(additional_global)
            <= self.playback_barrier_request_tombstone_policy.max_per_room
            && self
                .playback_barrier_request_tombstones
                .len()
                .saturating_add(additional_global)
                <= self.playback_barrier_request_tombstone_policy.max_global
    }

    fn playback_barrier_replay_capacity_retry_after_millis(
        &self,
        room_name: &str,
        displaced: &BTreeMap<PlaybackBarrierRequestId, DisplacedPlaybackBarrierRequest>,
    ) -> u64 {
        let additional_global = displaced
            .keys()
            .filter(|request_id| {
                !self
                    .playback_barrier_request_tombstones
                    .contains_key(&(room_name.to_owned(), (*request_id).clone()))
            })
            .count();
        let room_count = self
            .playback_barrier_request_tombstones
            .keys()
            .filter(|(tombstone_room, _)| tombstone_room == room_name)
            .count();
        let required_room_expirations = room_count
            .saturating_add(additional_global)
            .saturating_sub(self.playback_barrier_request_tombstone_policy.max_per_room);
        let required_global_expirations = self
            .playback_barrier_request_tombstones
            .len()
            .saturating_add(additional_global)
            .saturating_sub(self.playback_barrier_request_tombstone_policy.max_global);
        let now_seconds = self.playback_barrier_request_clock_seconds();
        let mut room_expirations = self
            .playback_barrier_request_tombstones
            .iter()
            .filter(|((tombstone_room, _), _)| tombstone_room == room_name)
            .map(|(_, tombstone)| tombstone.retain_until_seconds)
            .collect::<Vec<_>>();
        room_expirations
            .sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
        let room_retry_at = required_room_expirations
            .checked_sub(1)
            .and_then(|index| room_expirations.get(index).copied());
        let mut global_expirations = self
            .playback_barrier_request_tombstones
            .values()
            .map(|tombstone| tombstone.retain_until_seconds)
            .collect::<Vec<_>>();
        global_expirations
            .sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
        let global_retry_at = required_global_expirations
            .checked_sub(1)
            .and_then(|index| global_expirations.get(index).copied());
        let retry_at_seconds = room_retry_at
            .into_iter()
            .chain(global_retry_at)
            .max_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(now_seconds + 1.0);
        Self::playback_barrier_retry_after_millis(retry_at_seconds - now_seconds)
    }

    fn retain_displaced_playback_barrier_requests(
        &mut self,
        room_name: &str,
        displaced: BTreeMap<PlaybackBarrierRequestId, DisplacedPlaybackBarrierRequest>,
    ) {
        if displaced.is_empty() {
            return;
        }
        let retain_until_seconds = self.playback_barrier_request_clock_seconds()
            + self.playback_barrier_request_tombstone_policy.ttl_seconds;
        for (request_id, displaced_request) in displaced {
            self.playback_barrier_request_tombstones.insert(
                (room_name.to_owned(), request_id),
                PlaybackBarrierRequestTombstone {
                    request_nonce: displaced_request.request_nonce,
                    logical_media_id_digest: displaced_request.logical_media_id_digest,
                    media_generation: displaced_request.media_generation,
                    retain_until_seconds,
                },
            );
        }
    }

    fn playback_barrier_retry_later(
        client_id: &str,
        request_id: Option<&str>,
        request_nonce: u64,
        retry_after_ms: u64,
    ) -> Vec<DirectedProtocolMessage> {
        let Some(request_id) = request_id else {
            // Legacy extension peers have no stable application identity to
            // correlate. Reject atomically without emitting a fatal generic
            // protocol error that would tear down their transport.
            return Vec::new();
        };
        vec![DirectedProtocolMessage::new(
            client_id,
            playback_barrier_set_message(PlaybackBarrierSetExtension::new().with_request_result(
                PlaybackBarrierRequestResultPayload::retry_later(
                    request_id,
                    request_nonce,
                    retry_after_ms,
                ),
            )),
        )]
    }

    /// Replays the server-authored lifecycle for an idempotent request. This
    /// never mutates room playback and deliberately includes a retained
    /// terminal commit only as history; clients scope its authority through
    /// the accompanying status phase.
    pub(crate) fn playback_barrier_snapshot_for_client(
        &self,
        room_name: &str,
        client_id: &str,
    ) -> Vec<DirectedProtocolMessage> {
        self.playback_barrier_snapshot_for_client_with_recovery(room_name, client_id, None)
    }

    fn playback_barrier_snapshot_for_client_with_recovery(
        &self,
        room_name: &str,
        client_id: &str,
        recovery: Option<PlaybackBarrierRecoveryPayload>,
    ) -> Vec<DirectedProtocolMessage> {
        let Some(barrier) = self.room_playback_barriers.get(room_name) else {
            return Vec::new();
        };
        let Some(session) = self.sessions.get(client_id) else {
            return Vec::new();
        };
        if session.room != room_name || !session.capabilities.playback_barrier_v1 {
            return Vec::new();
        }
        if self.playback_barrier_fenced_clients.contains(client_id) {
            return Vec::new();
        }

        let mut extension = PlaybackBarrierSetExtension::new()
            .with_prepare(barrier.prepare.clone())
            .with_status(
                self.playback_barrier_status(room_name)
                    .expect("stored playback barrier should have status"),
            );
        if let Some(commit) = barrier.commit.clone() {
            extension = extension.with_commit(commit);
        }
        if let Some(control) = self.room_buffering_controls.get(room_name) {
            extension = extension.with_buffering_policy(control.config.clone());
            if let Some(status) = self.room_buffering_status(room_name) {
                extension = extension.with_buffering_status(status);
            }
        }
        if let Some(recovery) = recovery {
            extension = extension.with_recovery(recovery);
        }
        self.redact_playback_barrier_request_identity_for_client(
            room_name,
            client_id,
            &mut extension,
        );
        vec![DirectedProtocolMessage::new(
            client_id,
            playback_barrier_set_message(extension),
        )]
    }

    fn room_buffering_snapshot_for_client(
        &self,
        room_name: &str,
        client_id: &str,
    ) -> Vec<DirectedProtocolMessage> {
        let Some(control) = self.room_buffering_controls.get(room_name) else {
            return Vec::new();
        };
        let Some(session) = self.sessions.get(client_id) else {
            return Vec::new();
        };
        if session.room != room_name || !session.capabilities.playback_barrier_v1 {
            return Vec::new();
        }
        if self.playback_barrier_fenced_clients.contains(client_id) {
            return Vec::new();
        }
        let mut extension =
            PlaybackBarrierSetExtension::new().with_buffering_policy(control.config.clone());
        if let Some(status) = self.room_buffering_status(room_name) {
            extension = extension.with_buffering_status(status);
        }
        self.redact_playback_barrier_request_identity_for_client(
            room_name,
            client_id,
            &mut extension,
        );
        vec![DirectedProtocolMessage::new(
            client_id,
            playback_barrier_set_message(extension),
        )]
    }

    /// Re-evaluates the dynamic eligible cohort and gives a newly capable
    /// room participant the authoritative ongoing policy. The targeted
    /// policy snapshot prompts that transport to report its current state;
    /// the status fanout keeps the rest of the room's denominator current.
    pub(crate) fn refresh_room_buffering_participant(
        &mut self,
        client_id: &str,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let Some(session) = self.sessions.get(client_id) else {
            return Ok(Vec::new());
        };
        if !session.capabilities.playback_barrier_v1
            || self.playback_barrier_fenced_clients.contains(client_id)
            || !self.room_buffering_controls.contains_key(&session.room)
        {
            return Ok(Vec::new());
        }
        let room_name = session.room.clone();
        let mut outbound =
            self.evaluate_room_buffering_at(&room_name, self.current_time_seconds(), true)?;
        outbound.extend(self.room_buffering_snapshot_for_client(&room_name, client_id));
        Ok(outbound)
    }

    pub(crate) fn handle_playback_barrier_set(
        &mut self,
        client_id: &str,
        extension: PlaybackBarrierSetExtension,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        if let Some(recovery) = extension.recovery {
            // Recovery is deliberately a two-phase exchange. Never interpret
            // a query and a speculative fresh prepare from the same envelope;
            // the client must wait for the explicit Active/Absent disposition.
            return self.handle_playback_barrier_recovery(client_id, recovery);
        }
        if self.playback_barrier_fenced_clients.contains(client_id) {
            return Ok(Vec::new());
        }
        let mut outbound = Vec::new();
        let had_prepare = extension.prepare.is_some();
        let mut apply_buffering_policy = extension.prepare.is_none();
        if let Some(prepare) = extension.prepare {
            let generation_before = self.next_playback_barrier_generation;
            outbound.extend(self.start_playback_barrier(client_id, prepare)?);
            apply_buffering_policy = self.next_playback_barrier_generation > generation_before;
        }
        if apply_buffering_policy && let Some(policy) = extension.buffering_policy {
            outbound.extend(self.configure_room_buffering_policy(
                client_id,
                policy,
                had_prepare,
            )?);
        }
        // Commit, barrier status, and buffering status are server-owned.
        Ok(outbound)
    }

    pub(crate) fn handle_playback_barrier_state(
        &mut self,
        client_id: &str,
        extension: PlaybackBarrierStateExtension,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        if self.playback_barrier_fenced_clients.contains(client_id) {
            return Ok(Vec::new());
        }
        let mut outbound = Vec::new();
        if let Some(ready) = extension.ready {
            let bridge_technical_readiness = self
                .sessions
                .get(client_id)
                .is_some_and(|session| !session.capabilities.readiness_v2);
            outbound.extend(self.record_playback_barrier_ready(
                client_id,
                ready,
                bridge_technical_readiness,
            )?);
        }
        if let Some(started) = extension.started {
            outbound.extend(self.record_playback_barrier_started(client_id, started));
        }
        if let Some(transport) = extension.transport {
            outbound.extend(self.record_room_buffering_report(client_id, transport)?);
        }
        Ok(outbound)
    }

    fn handle_playback_barrier_recovery(
        &mut self,
        client_id: &str,
        query: PlaybackBarrierRecoveryPayload,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let Some(session) = self.sessions.get(client_id).cloned() else {
            return Err(ServerRuntimeError::MissingSession(client_id.to_owned()));
        };
        if !session.capabilities.playback_barrier_v1 {
            return Ok(Vec::new());
        }

        let query_is_valid = query.disposition.is_none()
            && query.media_generation.is_none()
            && query.original_request_nonce > 0
            && query.recovery_nonce > 0
            && valid_playback_barrier_request_id(&query.request_id)
            && !query.logical_media_id.trim().is_empty()
            && query.logical_media_id.chars().count()
                <= PLAYBACK_BARRIER_MAX_LOGICAL_MEDIA_ID_CHARS;
        let authorized = !self.playback_barrier_fenced_clients.contains(client_id)
            && self.user_can_control_playlist(&session.username, &session.room);
        if !query_is_valid || !authorized {
            return Ok(vec![DirectedProtocolMessage::new(
                client_id,
                playback_barrier_set_message(PlaybackBarrierSetExtension::new().with_recovery(
                    playback_barrier_recovery_result(
                        &query,
                        PlaybackBarrierRecoveryDisposition::Rejected,
                        None,
                    ),
                )),
            )]);
        }

        self.prune_playback_barrier_request_tombstones();
        let current_barrier_request =
            self.room_playback_barriers
                .get(&session.room)
                .filter(|barrier| {
                    barrier.prepare.request_id.as_deref() == Some(query.request_id.as_str())
                });
        if let Some(barrier) = current_barrier_request {
            let exact_request = barrier.prepare.request_nonce == query.original_request_nonce
                && barrier.prepare.logical_media_id == query.logical_media_id;
            if !exact_request {
                return Ok(vec![DirectedProtocolMessage::new(
                    client_id,
                    playback_barrier_set_message(PlaybackBarrierSetExtension::new().with_recovery(
                        playback_barrier_recovery_result(
                            &query,
                            PlaybackBarrierRecoveryDisposition::Rejected,
                            Some(barrier.prepare.media_generation),
                        ),
                    )),
                )]);
            }
        }

        let current_policy_request =
            self.room_buffering_controls
                .get(&session.room)
                .filter(|control| {
                    control.config.request_id.as_deref() == Some(query.request_id.as_str())
                });
        if current_barrier_request.is_none()
            && let Some(control) = current_policy_request
        {
            if control.config.request_nonce != query.original_request_nonce {
                return Ok(vec![DirectedProtocolMessage::new(
                    client_id,
                    playback_barrier_set_message(PlaybackBarrierSetExtension::new().with_recovery(
                        playback_barrier_recovery_result(
                            &query,
                            PlaybackBarrierRecoveryDisposition::Rejected,
                            Some(control.config.media_generation),
                        ),
                    )),
                )]);
            }
            self.rebind_room_buffering_owner_if_newer(client_id, &session.room);
            let control = self
                .room_buffering_controls
                .get(&session.room)
                .expect("exact policy recovery should remain configured");
            let media_generation = control.config.media_generation;
            let mut extension = PlaybackBarrierSetExtension::new()
                .with_buffering_policy(control.config.clone())
                .with_recovery(playback_barrier_recovery_result(
                    &query,
                    PlaybackBarrierRecoveryDisposition::Recovered,
                    Some(media_generation),
                ));
            if let Some(status) = self.room_buffering_status(&session.room) {
                extension = extension.with_buffering_status(status);
            }
            self.redact_playback_barrier_request_identity_for_client(
                &session.room,
                client_id,
                &mut extension,
            );
            return Ok(vec![DirectedProtocolMessage::new(
                client_id,
                playback_barrier_set_message(extension),
            )]);
        }

        if current_barrier_request.is_none()
            && let Some(tombstone) = self.playback_barrier_request_tombstones.get(&(
                session.room.clone(),
                PlaybackBarrierRequestId::new(query.request_id.clone()),
            ))
        {
            let tombstone_matches = tombstone.request_nonce == query.original_request_nonce
                && tombstone.logical_media_id_digest.is_none_or(|digest| {
                    digest == playback_barrier_logical_media_id_digest(&query.logical_media_id)
                });
            let disposition = if tombstone_matches {
                PlaybackBarrierRecoveryDisposition::Superseded
            } else {
                PlaybackBarrierRecoveryDisposition::Rejected
            };
            return Ok(vec![DirectedProtocolMessage::new(
                client_id,
                playback_barrier_set_message(PlaybackBarrierSetExtension::new().with_recovery(
                    playback_barrier_recovery_result(
                        &query,
                        disposition,
                        Some(tombstone.media_generation),
                    ),
                )),
            )]);
        }

        let Some(barrier) = self.room_playback_barriers.get(&session.room) else {
            return Ok(vec![DirectedProtocolMessage::new(
                client_id,
                playback_barrier_set_message(PlaybackBarrierSetExtension::new().with_recovery(
                    playback_barrier_recovery_result(
                        &query,
                        PlaybackBarrierRecoveryDisposition::Absent,
                        None,
                    ),
                )),
            )]);
        };
        let media_generation = barrier.prepare.media_generation;
        let same_logical_media = barrier.prepare.logical_media_id == query.logical_media_id;
        let exact_request = same_logical_media
            && barrier.prepare.request_nonce == query.original_request_nonce
            && barrier.prepare.request_id.as_deref() == Some(query.request_id.as_str());
        let lifecycle_is_active = matches!(
            barrier.phase,
            PlaybackBarrierPhase::Preparing
                | PlaybackBarrierPhase::Committed
                | PlaybackBarrierPhase::AwaitingDecision
        );
        let disposition = if exact_request {
            PlaybackBarrierRecoveryDisposition::Recovered
        } else if same_logical_media && lifecycle_is_active {
            PlaybackBarrierRecoveryDisposition::Existing
        } else if same_logical_media {
            PlaybackBarrierRecoveryDisposition::Absent
        } else {
            PlaybackBarrierRecoveryDisposition::Superseded
        };
        let room_name = session.room.clone();

        if disposition == PlaybackBarrierRecoveryDisposition::Absent {
            return Ok(vec![DirectedProtocolMessage::new(
                client_id,
                playback_barrier_set_message(
                    PlaybackBarrierSetExtension::new()
                        .with_recovery(playback_barrier_recovery_result(&query, disposition, None)),
                ),
            )]);
        }
        if disposition == PlaybackBarrierRecoveryDisposition::Superseded {
            return Ok(vec![DirectedProtocolMessage::new(
                client_id,
                playback_barrier_set_message(PlaybackBarrierSetExtension::new().with_recovery(
                    playback_barrier_recovery_result(&query, disposition, Some(media_generation)),
                )),
            )]);
        }

        let mut outbound = Vec::new();
        if exact_request && self.rebind_playback_barrier_owner_if_newer(client_id, &room_name) {
            outbound.extend(self.playback_barrier_status_fanout(&room_name));
            outbound.extend(self.evaluate_room_buffering_at(
                &room_name,
                self.current_time_seconds(),
                true,
            )?);
        }
        let recovery =
            playback_barrier_recovery_result(&query, disposition, Some(media_generation));
        outbound.extend(self.playback_barrier_snapshot_for_client_with_recovery(
            &room_name,
            client_id,
            Some(recovery),
        ));
        Ok(outbound)
    }

    fn rebind_playback_barrier_owner_if_newer(&mut self, client_id: &str, room_name: &str) -> bool {
        let Some(new_session_sequence) = self.client_room_join_sequence.get(client_id).copied()
        else {
            return false;
        };
        let Some(session) = self.sessions.get(client_id).cloned() else {
            return false;
        };
        let Some(barrier) = self.room_playback_barriers.get(room_name) else {
            return false;
        };
        if new_session_sequence <= barrier.initiator_session_sequence {
            return false;
        }

        let old_client_id = barrier.initiator_client_id.clone();
        let barrier_phase = barrier.phase;
        let barrier = self
            .room_playback_barriers
            .get_mut(room_name)
            .expect("recovery candidate should remain present");
        let mut participant = barrier
            .participants
            .remove(&old_client_id)
            .or_else(|| barrier.participants.remove(client_id))
            .unwrap_or(RoomPlaybackBarrierParticipant {
                username: session.username.clone(),
                status: PlaybackBarrierParticipantStatus::pending(),
            });
        participant.username.clone_from(&session.username);
        if participant.status.degraded_reason == Some(PlaybackBarrierDegradedReason::Disconnected) {
            match barrier_phase {
                PlaybackBarrierPhase::Preparing => {
                    participant.status.phase = PlaybackBarrierParticipantPhase::Pending;
                    participant.status.degraded_reason = None;
                }
                PlaybackBarrierPhase::Committed => {
                    participant.status.phase = PlaybackBarrierParticipantPhase::Ready;
                    participant.status.degraded_reason = None;
                }
                PlaybackBarrierPhase::AwaitingDecision
                | PlaybackBarrierPhase::Complete
                | PlaybackBarrierPhase::Degraded => {}
            }
        }
        barrier
            .participants
            .insert(client_id.to_owned(), participant);
        barrier.initiator_client_id = client_id.to_owned();
        barrier.initiator_session_sequence = new_session_sequence;
        barrier.initiator_username.clone_from(&session.username);

        if old_client_id != client_id && self.sessions.contains_key(&old_client_id) {
            self.fence_and_close_playback_barrier_transport(&old_client_id);
        }
        if let Some(control) = self.room_buffering_controls.get_mut(room_name)
            && control.configured_by_client_id == old_client_id
        {
            control.config = control.requested_config.clone();
            control.configured_by_client_id = client_id.to_owned();
            control.configured_by_username = session.username;
            control.reports.remove(&old_client_id);
            control.reports.remove(client_id);
            control.condition_active_since = None;
            control.condition_clear_since = None;
        }
        true
    }

    fn rebind_room_buffering_owner_if_newer(&mut self, client_id: &str, room_name: &str) -> bool {
        let Some(session) = self.sessions.get(client_id).cloned() else {
            return false;
        };
        let Some(new_session_sequence) = self.client_room_join_sequence.get(client_id).copied()
        else {
            return false;
        };
        let Some(control) = self.room_buffering_controls.get(room_name) else {
            return false;
        };
        let old_client_id = control.configured_by_client_id.clone();
        if old_client_id == client_id {
            return false;
        }
        let old_session_sequence = self
            .client_room_join_sequence
            .get(&old_client_id)
            .copied()
            .unwrap_or_default();
        if new_session_sequence <= old_session_sequence {
            return false;
        }
        let control = self
            .room_buffering_controls
            .get_mut(room_name)
            .expect("policy recovery candidate should remain configured");
        control.config = control.requested_config.clone();
        control.configured_by_client_id = client_id.to_owned();
        control.configured_by_username = session.username;
        control.reports.remove(&old_client_id);
        control.reports.remove(client_id);
        control.condition_active_since = None;
        control.condition_clear_since = None;
        if self.sessions.contains_key(&old_client_id) {
            self.fence_and_close_playback_barrier_transport(&old_client_id);
        }
        true
    }

    fn configure_room_buffering_policy(
        &mut self,
        client_id: &str,
        mut config: RoomBufferingPolicyPayload,
        paired_with_new_prepare: bool,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let session = self
            .sessions
            .get(client_id)
            .cloned()
            .ok_or_else(|| ServerRuntimeError::MissingSession(client_id.to_owned()))?;
        let controlled_room = self
            .room_password_provider
            .is_controlled_room_name(&session.room);
        let authenticated_controller =
            controlled_room && self.user_is_room_controller(&session.username, &session.room);
        if !session.capabilities.playback_barrier_v1
            || (controlled_room && !authenticated_controller)
            || (!controlled_room && config.policy != RoomBufferingPolicy::Independent)
            || config.state_revision == Some(0)
            || (config.load_intent == MediaLoadIntent::TransportRefresh
                && config.media_generation != 0)
            || config
                .request_id
                .as_deref()
                .is_some_and(|request_id| !valid_playback_barrier_request_id(request_id))
        {
            return Ok(Vec::new());
        }
        let mut displaced_requests = BTreeMap::new();
        let mut replace_room_barrier = false;
        let mut targeted_public_independent = false;
        if config.media_generation == 0 {
            if config.request_nonce == 0 {
                return Ok(Vec::new());
            }
            self.prune_playback_barrier_request_tombstones();
            if !paired_with_new_prepare && let Some(request_id) = config.request_id.as_deref() {
                let exact_current_operation = self
                    .room_buffering_controls
                    .get(&session.room)
                    .is_some_and(|control| {
                        control.config.request_id.as_deref() == Some(request_id)
                            && control.config.request_nonce == config.request_nonce
                    });
                if exact_current_operation {
                    if let Some(barrier) = self
                        .room_playback_barriers
                        .get(&session.room)
                        .filter(|barrier| barrier.prepare.request_id.as_deref() == Some(request_id))
                    {
                        let same_owner = barrier.initiator_client_id == client_id
                            && self.client_room_join_sequence.get(client_id).is_some_and(
                                |sequence| *sequence == barrier.initiator_session_sequence,
                            );
                        return Ok(if same_owner {
                            self.playback_barrier_snapshot_for_client(&session.room, client_id)
                        } else {
                            // A replacement transport must recover the full
                            // shared start/policy lifecycle, not steal only
                            // its buffering-policy half.
                            Vec::new()
                        });
                    }
                    self.rebind_room_buffering_owner_if_newer(client_id, &session.room);
                    return Ok(self.room_buffering_snapshot_for_client(&session.room, client_id));
                }
                if self
                    .room_buffering_controls
                    .get(&session.room)
                    .is_some_and(|control| control.config.request_id.as_deref() == Some(request_id))
                    || self
                        .room_playback_barriers
                        .get(&session.room)
                        .is_some_and(|barrier| {
                            barrier.prepare.request_id.as_deref() == Some(request_id)
                        })
                {
                    // A stable operation ID cannot be rebound to a different
                    // nonce or change between start and policy-only intent.
                    return Ok(Vec::new());
                }
                if self.playback_barrier_request_tombstones.contains_key(&(
                    session.room.clone(),
                    PlaybackBarrierRequestId::new(request_id),
                )) {
                    return Ok(Vec::new());
                }
            }
            if paired_with_new_prepare {
                let Some(barrier) = self.room_playback_barriers.get(&session.room) else {
                    return Ok(Vec::new());
                };
                let same_request = barrier.initiator_client_id == client_id
                    && self
                        .client_room_join_sequence
                        .get(client_id)
                        .is_some_and(|sequence| *sequence == barrier.initiator_session_sequence)
                    && barrier.prepare.request_nonce == config.request_nonce
                    && barrier.prepare.request_id == config.request_id;
                if !same_request {
                    return Ok(Vec::new());
                }
                config.media_generation = barrier.prepare.media_generation;
                config.state_revision = barrier.state_revision;
                config.load_intent = barrier.prepare.load_intent;
            } else {
                if self
                    .playback_barrier_request_nonces
                    .get(client_id)
                    .is_some_and(|highest_nonce| config.request_nonce <= *highest_nonce)
                {
                    let exact_current_retry = self
                        .room_buffering_controls
                        .get(&session.room)
                        .is_some_and(|control| {
                            control.configured_by_client_id == client_id
                                && control.config.request_nonce == config.request_nonce
                        });
                    return Ok(if exact_current_retry {
                        self.room_buffering_snapshot_for_client(&session.room, client_id)
                    } else {
                        Vec::new()
                    });
                }
                let transport_refresh_identity =
                    if config.load_intent == MediaLoadIntent::TransportRefresh {
                        if self.room_playback_barriers.contains_key(&session.room) {
                            // A transport refresh may replace only a
                            // policy-only lifecycle. If a start lifecycle is
                            // retained, recover that operation instead so one
                            // request cannot transfer only half its ownership.
                            return Ok(Vec::new());
                        }
                        let Some(control) = self.room_buffering_controls.get(&session.room) else {
                            return Ok(Vec::new());
                        };
                        if control.configured_by_username != session.username {
                            return Ok(Vec::new());
                        }
                        Some((
                            control.config.media_generation,
                            control.config.state_revision,
                        ))
                    } else {
                        None
                    };
                targeted_public_independent = !controlled_room
                    && config.policy == RoomBufferingPolicy::Independent
                    && (config.load_intent == MediaLoadIntent::TransportRefresh
                        || !self.room_playback_barriers.contains_key(&session.room));
                let public_independent_identity = targeted_public_independent.then(|| {
                    self.room_buffering_controls
                        .get(&session.room)
                        .filter(|control| control.config.policy == RoomBufferingPolicy::Independent)
                        .map(|control| {
                            (
                                control.config.media_generation,
                                control.config.state_revision,
                            )
                        })
                });
                if targeted_public_independent {
                    // No replay cache is needed: every public Independent
                    // request is coalesced onto equivalent canonical state.
                } else if transport_refresh_identity.is_some() {
                    displaced_requests = self.displaced_playback_barrier_requests(
                        &session.room,
                        config.request_id.as_deref(),
                        false,
                        true,
                    );
                } else if !targeted_public_independent {
                    displaced_requests = self.displaced_playback_barrier_requests(
                        &session.room,
                        config.request_id.as_deref(),
                        true,
                        true,
                    );
                }
                if let Some(retry_after_ms) = self.playback_barrier_new_identity_retry_after_millis(
                    client_id,
                    &session.room,
                    config.request_id.as_deref(),
                    config.request_nonce,
                ) {
                    return Ok(Self::playback_barrier_retry_later(
                        client_id,
                        config.request_id.as_deref(),
                        config.request_nonce,
                        retry_after_ms,
                    ));
                }
                if !self.can_retain_displaced_playback_barrier_requests(
                    &session.room,
                    &displaced_requests,
                ) {
                    let retry_after_ms = self.playback_barrier_replay_capacity_retry_after_millis(
                        &session.room,
                        &displaced_requests,
                    );
                    return Ok(Self::playback_barrier_retry_later(
                        client_id,
                        config.request_id.as_deref(),
                        config.request_nonce,
                        retry_after_ms,
                    ));
                }
                self.playback_barrier_request_nonces
                    .insert(client_id.to_owned(), config.request_nonce);
                if let Some((media_generation, state_revision)) = transport_refresh_identity {
                    // A reconnect refresh is new intent, not a replay of the
                    // old serialized request. Rebind the freshly authorized
                    // owner's requested policy to the server's retained
                    // canonical media identity without allocating a barrier.
                    config.media_generation = media_generation;
                    config.state_revision = state_revision;
                } else if let Some((media_generation, state_revision)) =
                    public_independent_identity.flatten()
                {
                    // Public-room Independent policy carries no coordinated
                    // behavior. Coalesce its identity onto the one canonical
                    // generation instead of creating replay tombstones or
                    // allowing harmless request churn to consume generations.
                    config.media_generation = media_generation;
                    config.state_revision = state_revision;
                } else {
                    if self
                        .room_playback_barriers
                        .get(&session.room)
                        .is_some_and(|barrier| {
                            matches!(
                                barrier.phase,
                                PlaybackBarrierPhase::Preparing | PlaybackBarrierPhase::Committed
                            )
                        })
                    {
                        return Ok(Vec::new());
                    }
                    let Some(generation) = self.allocate_playback_barrier_generation() else {
                        return Ok(Vec::new());
                    };
                    config.media_generation = generation;
                    config.state_revision = None;
                    replace_room_barrier = !targeted_public_independent;
                }
            }
        }
        if let Some(barrier) = self.room_playback_barriers.get(&session.room)
            && (config.media_generation < barrier.prepare.media_generation
                || (config.media_generation == barrier.prepare.media_generation
                    && barrier.state_revision.is_some()
                    && config.state_revision != barrier.state_revision))
        {
            return Ok(Vec::new());
        }
        if let Some(active) = self.room_buffering_controls.get(&session.room)
            && (config.media_generation < active.config.media_generation
                || (config.media_generation == active.config.media_generation
                    && config.state_revision < active.config.state_revision))
        {
            return Ok(Vec::new());
        }

        config.quorum_percent = match config.policy {
            RoomBufferingPolicy::Quorum => Some(
                config
                    .quorum_percent
                    .unwrap_or(ROOM_BUFFERING_DEFAULT_QUORUM_PERCENT)
                    .clamp(1, 100),
            ),
            RoomBufferingPolicy::Independent
            | RoomBufferingPolicy::PauseController
            | RoomBufferingPolicy::PauseAnyEligible => None,
        };
        config.debounce_ms = Some(normalize_room_buffering_duration_ms(
            config.debounce_ms,
            ROOM_BUFFERING_DEFAULT_DEBOUNCE_SECONDS,
            0.0,
            ROOM_BUFFERING_MAX_DEBOUNCE_SECONDS,
        ));
        config.resume_hysteresis_ms = Some(normalize_room_buffering_duration_ms(
            config.resume_hysteresis_ms,
            ROOM_BUFFERING_DEFAULT_RESUME_HYSTERESIS_SECONDS,
            0.0,
            ROOM_BUFFERING_MAX_RESUME_HYSTERESIS_SECONDS,
        ));
        config.max_pause_ms = Some(normalize_room_buffering_duration_ms(
            config.max_pause_ms,
            ROOM_BUFFERING_DEFAULT_MAX_PAUSE_SECONDS,
            ROOM_BUFFERING_MIN_MAX_PAUSE_SECONDS,
            ROOM_BUFFERING_MAX_MAX_PAUSE_SECONDS,
        ));

        let room_name = session.room.clone();
        let now_seconds = self.current_time_seconds();
        self.retain_displaced_playback_barrier_requests(&room_name, displaced_requests);
        if replace_room_barrier {
            // The displaced identity was retained atomically before removing
            // terminal diagnostics from the room's canonical lifecycle.
            self.room_playback_barriers.remove(&room_name);
        }
        let old_policy_owned_pause = self
            .room_buffering_controls
            .remove(&room_name)
            .is_some_and(|control| control.paused_by_policy);
        let mut outbound = if old_policy_owned_pause {
            self.apply_room_buffering_transition(
                &room_name,
                RoomBufferingTransition::Resume,
                &session.username,
                now_seconds,
            )?
        } else {
            Vec::new()
        };
        self.room_buffering_controls.insert(
            room_name.clone(),
            RoomBufferingControl {
                config: config.clone(),
                requested_config: config.clone(),
                configured_by_client_id: client_id.to_owned(),
                configured_by_username: session.username,
                reports: BTreeMap::new(),
                condition_active_since: None,
                condition_clear_since: None,
                paused_by_policy: false,
                pause_deadline: None,
                fail_open_latched: false,
            },
        );
        let status = self
            .room_buffering_status(&room_name)
            .expect("new room buffering control should have status");
        if targeted_public_independent {
            outbound.extend(self.room_buffering_snapshot_for_client(&room_name, client_id));
        } else {
            outbound.extend(
                self.room_buffering_fanout(
                    &room_name,
                    PlaybackBarrierSetExtension::new()
                        .with_buffering_policy(config)
                        .with_buffering_status(status),
                ),
            );
        }
        Ok(outbound)
    }

    fn record_room_buffering_report(
        &mut self,
        client_id: &str,
        report: TransportBufferingReportPayload,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let session = self
            .sessions
            .get(client_id)
            .cloned()
            .ok_or_else(|| ServerRuntimeError::MissingSession(client_id.to_owned()))?;
        if !session.capabilities.playback_barrier_v1
            || report.media_generation == 0
            || report.state_revision == Some(0)
            || report
                .buffered_seconds
                .is_some_and(|seconds| !seconds.is_finite() || seconds < 0.0)
            || report.observed_at.is_some_and(|value| !value.is_finite())
        {
            return Ok(Vec::new());
        }
        let now_seconds = self.current_time_seconds();
        let Some(control) = self.room_buffering_controls.get_mut(&session.room) else {
            return Ok(Vec::new());
        };
        if report.media_generation != control.config.media_generation
            || (control.config.state_revision.is_some()
                && report.state_revision != control.config.state_revision)
        {
            return Ok(Vec::new());
        }
        control.reports.insert(
            client_id.to_owned(),
            RoomBufferingParticipantReport {
                username: session.username,
                buffering: report.buffering,
                buffered_seconds: report.buffered_seconds,
                reported_at_seconds: now_seconds,
            },
        );
        self.evaluate_room_buffering_at(&session.room, now_seconds, true)
    }

    fn evaluate_room_buffering_at(
        &mut self,
        room_name: &str,
        now_seconds: f64,
        always_publish_status: bool,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let before_status = self.room_buffering_status(room_name);
        let Some((condition_active, policy)) = self.room_buffering_condition(room_name) else {
            return Ok(Vec::new());
        };
        let mut transition = None;
        let configured_by_username;
        {
            let control = self
                .room_buffering_controls
                .get_mut(room_name)
                .expect("condition requires an active room buffering control");
            configured_by_username = control.configured_by_username.clone();
            let debounce_seconds = room_buffering_config_seconds(control.config.debounce_ms);
            let hysteresis_seconds =
                room_buffering_config_seconds(control.config.resume_hysteresis_ms);
            let max_pause_seconds = room_buffering_config_seconds(control.config.max_pause_ms);

            if policy == RoomBufferingPolicy::Independent {
                control.condition_active_since = None;
                control.condition_clear_since = None;
                control.fail_open_latched = false;
                if control.paused_by_policy {
                    control.paused_by_policy = false;
                    control.pause_deadline = None;
                    transition = Some(RoomBufferingTransition::Resume);
                }
            } else if control.fail_open_latched {
                control.condition_active_since = None;
                if condition_active {
                    control.condition_clear_since = None;
                } else {
                    let clear_since = control.condition_clear_since.get_or_insert(now_seconds);
                    if now_seconds - *clear_since >= hysteresis_seconds {
                        control.condition_clear_since = None;
                        control.fail_open_latched = false;
                    }
                }
            } else if control.paused_by_policy {
                if control
                    .pause_deadline
                    .is_some_and(|deadline| deadline <= now_seconds)
                {
                    control.paused_by_policy = false;
                    control.pause_deadline = None;
                    control.condition_active_since = None;
                    control.condition_clear_since = None;
                    control.fail_open_latched = true;
                    transition = Some(RoomBufferingTransition::Resume);
                } else if condition_active {
                    control.condition_clear_since = None;
                } else {
                    let clear_since = control.condition_clear_since.get_or_insert(now_seconds);
                    if now_seconds - *clear_since >= hysteresis_seconds {
                        control.paused_by_policy = false;
                        control.pause_deadline = None;
                        control.condition_active_since = None;
                        control.condition_clear_since = None;
                        transition = Some(RoomBufferingTransition::Resume);
                    }
                }
            } else if condition_active {
                control.condition_clear_since = None;
                let active_since = control.condition_active_since.get_or_insert(now_seconds);
                if now_seconds - *active_since >= debounce_seconds {
                    control.paused_by_policy = true;
                    control.pause_deadline = Some(now_seconds + max_pause_seconds);
                    control.condition_active_since = None;
                    transition = Some(RoomBufferingTransition::Pause);
                }
            } else {
                control.condition_active_since = None;
                control.condition_clear_since = None;
            }
        }

        let mut outbound = Vec::new();
        if let Some(transition) = transition {
            let room_was_in_target_state = match transition {
                RoomBufferingTransition::Pause => {
                    self.room_playback_state_at(room_name, now_seconds).paused
                }
                RoomBufferingTransition::Resume => {
                    !self.room_playback_state_at(room_name, now_seconds).paused
                }
            };
            if transition == RoomBufferingTransition::Pause && room_was_in_target_state {
                if let Some(control) = self.room_buffering_controls.get_mut(room_name) {
                    // Never claim ownership of, and later undo, a user-authored pause.
                    control.paused_by_policy = false;
                    control.pause_deadline = None;
                }
            } else {
                outbound.extend(self.apply_room_buffering_transition(
                    room_name,
                    transition,
                    &configured_by_username,
                    now_seconds,
                )?);
            }
        }

        let after_status = self.room_buffering_status(room_name);
        if (always_publish_status || before_status != after_status || !outbound.is_empty())
            && let Some(status) = after_status
        {
            outbound.extend(self.room_buffering_fanout(
                room_name,
                PlaybackBarrierSetExtension::new().with_buffering_status(status),
            ));
        }
        Ok(outbound)
    }

    fn room_buffering_condition(&self, room_name: &str) -> Option<(bool, RoomBufferingPolicy)> {
        let control = self.room_buffering_controls.get(room_name)?;
        let eligible: BTreeSet<&str> = self
            .sessions
            .iter()
            .filter(|(client_id, session)| {
                session.room == room_name
                    && session.capabilities.playback_barrier_v1
                    && !self.playback_barrier_fenced_clients.contains(*client_id)
            })
            .map(|(client_id, _)| client_id.as_str())
            .collect();
        let buffering_count = control
            .reports
            .iter()
            .filter(|(client_id, report)| eligible.contains(client_id.as_str()) && report.buffering)
            .count() as u32;
        let condition_active = match control.config.policy {
            RoomBufferingPolicy::Independent => false,
            RoomBufferingPolicy::PauseController => control
                .reports
                .get(&control.configured_by_client_id)
                .is_some_and(|report| {
                    eligible.contains(control.configured_by_client_id.as_str()) && report.buffering
                }),
            RoomBufferingPolicy::PauseAnyEligible => buffering_count > 0,
            RoomBufferingPolicy::Quorum => {
                let eligible_count = eligible.len() as u32;
                let required = room_buffering_quorum_required(
                    eligible_count,
                    control
                        .config
                        .quorum_percent
                        .unwrap_or(ROOM_BUFFERING_DEFAULT_QUORUM_PERCENT),
                );
                required > 0 && buffering_count >= required
            }
        };
        Some((condition_active, control.config.policy))
    }

    fn apply_room_buffering_transition(
        &mut self,
        room_name: &str,
        transition: RoomBufferingTransition,
        set_by: &str,
        now_seconds: f64,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let buffering_identity = self.room_buffering_controls.get(room_name).map(|control| {
            (
                control.config.media_generation,
                control.config.state_revision,
            )
        });
        if transition == RoomBufferingTransition::Resume
            && buffering_identity.is_some_and(|(media_generation, state_revision)| {
                !self.readiness_pause_owned_by_buffering_policy(
                    room_name,
                    media_generation,
                    state_revision,
                )
            })
        {
            return Ok(Vec::new());
        }
        let paused = transition == RoomBufferingTransition::Pause;
        let room_before = self.room_playback_state_at(room_name, now_seconds);
        if room_before.paused == paused {
            return Ok(Vec::new());
        }
        {
            let room_state = self.room_playback_state_mut(room_name);
            room_state.position = room_before.position;
            room_state.paused = paused;
            room_state.updated_at_seconds = now_seconds;
            room_state.set_by = Some(set_by.to_owned());
        }
        self.seed_room_client_playback_states(room_name, room_before.position, now_seconds);
        self.persist_room_if_needed(room_name)?;
        let mut outbound: Vec<_> = self
            .clients_in_room(room_name)
            .into_iter()
            .map(|peer_client| {
                let message = self.forced_state_sync_message_for_client(
                    &peer_client,
                    room_before.position,
                    paused,
                    false,
                    Some(set_by),
                );
                DirectedProtocolMessage::new(peer_client, message)
            })
            .collect();
        if let Some((media_generation, state_revision)) = buffering_identity {
            let owner = if paused {
                RoomPauseOwner::RoomBufferingPolicy {
                    media_generation,
                    state_revision,
                }
            } else {
                RoomPauseOwner::None
            };
            outbound.extend(self.set_readiness_pause_owner(room_name, owner, true));
        }
        Ok(outbound)
    }

    fn room_buffering_status(&self, room_name: &str) -> Option<RoomBufferingStatusPayload> {
        let control = self.room_buffering_controls.get(room_name)?;
        let eligible: BTreeMap<&str, &str> = self
            .sessions
            .iter()
            .filter(|(client_id, session)| {
                session.room == room_name
                    && session.capabilities.playback_barrier_v1
                    && !self.playback_barrier_fenced_clients.contains(*client_id)
            })
            .map(|(client_id, session)| (client_id.as_str(), session.username.as_str()))
            .collect();
        let buffering_clients = control
            .reports
            .iter()
            .filter(|(client_id, report)| {
                report.buffering && eligible.contains_key(client_id.as_str())
            })
            .map(|(_, report)| report.username.clone())
            .collect();
        let eligible_clients = eligible.len() as u32;
        let required_buffering_clients = match control.config.policy {
            RoomBufferingPolicy::Independent => 0,
            RoomBufferingPolicy::PauseController | RoomBufferingPolicy::PauseAnyEligible => {
                u32::from(eligible_clients > 0)
            }
            RoomBufferingPolicy::Quorum => room_buffering_quorum_required(
                eligible_clients,
                control
                    .config
                    .quorum_percent
                    .unwrap_or(ROOM_BUFFERING_DEFAULT_QUORUM_PERCENT),
            ),
        };
        let phase = if control.config.policy == RoomBufferingPolicy::Independent {
            RoomBufferingPhase::Independent
        } else if control.fail_open_latched {
            RoomBufferingPhase::FailOpen
        } else if control.paused_by_policy && control.condition_clear_since.is_some() {
            RoomBufferingPhase::DebouncingResume
        } else if control.paused_by_policy {
            RoomBufferingPhase::Paused
        } else if control.condition_active_since.is_some() {
            RoomBufferingPhase::DebouncingPause
        } else {
            RoomBufferingPhase::Monitoring
        };
        Some(RoomBufferingStatusPayload {
            config: control.config.clone(),
            phase,
            eligible_clients,
            required_buffering_clients,
            buffering_clients,
            pause_deadline: control.pause_deadline,
        })
    }

    fn room_buffering_fanout(
        &self,
        room_name: &str,
        extension: PlaybackBarrierSetExtension,
    ) -> Vec<DirectedProtocolMessage> {
        self.sessions
            .iter()
            .filter(|(client_id, session)| {
                session.room == room_name
                    && session.capabilities.playback_barrier_v1
                    && !self.playback_barrier_fenced_clients.contains(*client_id)
            })
            .map(|(client_id, _)| {
                let mut extension = extension.clone();
                self.redact_playback_barrier_request_identity_for_client(
                    room_name,
                    client_id,
                    &mut extension,
                );
                DirectedProtocolMessage::new(client_id, playback_barrier_set_message(extension))
            })
            .collect()
    }

    fn redact_playback_barrier_request_identity_for_client(
        &self,
        room_name: &str,
        client_id: &str,
        extension: &mut PlaybackBarrierSetExtension,
    ) {
        if self
            .room_playback_barriers
            .get(room_name)
            .is_none_or(|barrier| barrier.initiator_client_id != client_id)
            && let Some(prepare) = extension.prepare.as_mut()
        {
            prepare.request_id = None;
        }
        if self
            .room_buffering_controls
            .get(room_name)
            .is_none_or(|control| control.configured_by_client_id != client_id)
        {
            if let Some(policy) = extension.buffering_policy.as_mut() {
                policy.request_id = None;
            }
            if let Some(status) = extension.buffering_status.as_mut() {
                status.config.request_id = None;
            }
        }
    }

    fn start_playback_barrier(
        &mut self,
        client_id: &str,
        mut prepare: PrepareMediaPayload,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let session = self
            .sessions
            .get(client_id)
            .cloned()
            .ok_or_else(|| ServerRuntimeError::MissingSession(client_id.to_owned()))?;
        let readiness_governed =
            self.readiness_enabled && self.room_readiness.contains_key(&session.room);
        if !session.capabilities.playback_barrier_v1
            || (readiness_governed && !session.capabilities.readiness_v2)
            || !self.user_can_control_playlist(&session.username, &session.room)
            || prepare.media_generation != 0
            || prepare.request_nonce == 0
            || prepare.logical_media_id.trim().is_empty()
            || !prepare.target_position.is_finite()
            || prepare
                .request_id
                .as_deref()
                .is_some_and(|request_id| !valid_playback_barrier_request_id(request_id))
        {
            return Ok(Vec::new());
        }
        prepare.logical_media_id = truncate_text_to_max_chars(
            prepare.logical_media_id.trim(),
            PLAYBACK_BARRIER_MAX_LOGICAL_MEDIA_ID_CHARS,
        );
        let Some(initiator_session_sequence) =
            self.client_room_join_sequence.get(client_id).copied()
        else {
            return Ok(Vec::new());
        };

        if let Some(request_id) = prepare.request_id.as_deref()
            && let Some(barrier) = self.room_playback_barriers.get(&session.room)
            && barrier.prepare.request_id.as_deref() == Some(request_id)
        {
            let exact_operation = barrier.prepare.request_nonce == prepare.request_nonce
                && barrier.prepare.logical_media_id == prepare.logical_media_id;
            if !exact_operation {
                return Ok(Vec::new());
            }
            self.playback_barrier_request_nonces
                .entry(client_id.to_owned())
                .and_modify(|nonce| *nonce = (*nonce).max(prepare.request_nonce))
                .or_insert(prepare.request_nonce);
            let rebound = self.rebind_playback_barrier_owner_if_newer(client_id, &session.room);
            let mut outbound = if rebound {
                self.playback_barrier_status_fanout(&session.room)
            } else {
                Vec::new()
            };
            outbound.extend(self.playback_barrier_snapshot_for_client(&session.room, client_id));
            return Ok(outbound);
        }
        if let Some(request_id) = prepare.request_id.as_deref()
            && self
                .room_buffering_controls
                .get(&session.room)
                .is_some_and(|control| control.config.request_id.as_deref() == Some(request_id))
        {
            // An operation identity cannot change from policy-only intent into
            // a start request after its canonical policy has been accepted.
            return Ok(Vec::new());
        }
        self.prune_playback_barrier_request_tombstones();
        if prepare.request_id.as_ref().is_some_and(|request_id| {
            self.playback_barrier_request_tombstones.contains_key(&(
                session.room.clone(),
                PlaybackBarrierRequestId::new(request_id.clone()),
            ))
        }) {
            // This operation was accepted previously but is no longer the
            // room's current lifecycle. A delayed copy must never allocate a
            // fresh generation after supersession.
            return Ok(Vec::new());
        }

        if self
            .playback_barrier_request_nonces
            .get(client_id)
            .is_some_and(|highest_nonce| prepare.request_nonce <= *highest_nonce)
        {
            let exact_current_retry =
                self.room_playback_barriers
                    .get(&session.room)
                    .is_some_and(|barrier| {
                        barrier.initiator_client_id == client_id
                            && barrier.initiator_session_sequence == initiator_session_sequence
                            && barrier.prepare.request_nonce == prepare.request_nonce
                    });
            return Ok(if exact_current_retry {
                self.playback_barrier_snapshot_for_client(&session.room, client_id)
            } else {
                Vec::new()
            });
        }
        if let Some(retry_after_ms) = self.playback_barrier_new_identity_retry_after_millis(
            client_id,
            &session.room,
            prepare.request_id.as_deref(),
            prepare.request_nonce,
        ) {
            return Ok(Self::playback_barrier_retry_later(
                client_id,
                prepare.request_id.as_deref(),
                prepare.request_nonce,
                retry_after_ms,
            ));
        }
        let displaced_requests = if prepare.load_intent == MediaLoadIntent::TransportRefresh {
            BTreeMap::new()
        } else {
            self.displaced_playback_barrier_requests(
                &session.room,
                prepare.request_id.as_deref(),
                true,
                true,
            )
        };
        if !self.can_retain_displaced_playback_barrier_requests(&session.room, &displaced_requests)
        {
            let retry_after_ms = self.playback_barrier_replay_capacity_retry_after_millis(
                &session.room,
                &displaced_requests,
            );
            return Ok(Self::playback_barrier_retry_later(
                client_id,
                prepare.request_id.as_deref(),
                prepare.request_nonce,
                retry_after_ms,
            ));
        }
        // Consume every authorized, structurally valid nonce, including a
        // request rejected because it does not own the active lifecycle.
        // Retrying that stale user intent later must not start it.
        self.playback_barrier_request_nonces
            .insert(client_id.to_owned(), prepare.request_nonce);

        let mut supersedes_active = false;
        if let Some(barrier) = self.room_playback_barriers.get(&session.room) {
            let same_logical_media = barrier.prepare.logical_media_id == prepare.logical_media_id;
            if prepare.load_intent == MediaLoadIntent::TransportRefresh {
                return Ok(if same_logical_media {
                    self.playback_barrier_snapshot_for_client(&session.room, client_id)
                } else {
                    Vec::new()
                });
            }
            if matches!(
                barrier.phase,
                PlaybackBarrierPhase::Preparing | PlaybackBarrierPhase::Committed
            ) {
                let same_owner = barrier.initiator_client_id == client_id
                    && barrier.initiator_session_sequence == initiator_session_sequence;
                let replacement_intent_matches_identity = match prepare.load_intent {
                    MediaLoadIntent::NewPlayback => !same_logical_media,
                    MediaLoadIntent::Replay => same_logical_media,
                    MediaLoadIntent::TransportRefresh => false,
                };
                if !same_owner || !replacement_intent_matches_identity {
                    return Ok(Vec::new());
                }
                supersedes_active = true;
            } else {
                if prepare.load_intent == MediaLoadIntent::NewPlayback
                    && same_logical_media
                    && (barrier.initiator_client_id != client_id
                        || barrier.initiator_session_sequence != initiator_session_sequence)
                {
                    // A replacement controller or reconnected transport has no
                    // trustworthy local episode history. Its fresh, authorized
                    // nonce is nevertheless explicit playback intent, and the
                    // retained terminal identity makes Replay unambiguous.
                    prepare.load_intent = MediaLoadIntent::Replay;
                }
                match prepare.load_intent {
                    MediaLoadIntent::NewPlayback if same_logical_media => return Ok(Vec::new()),
                    MediaLoadIntent::Replay if !same_logical_media => return Ok(Vec::new()),
                    MediaLoadIntent::NewPlayback | MediaLoadIntent::Replay => {}
                    MediaLoadIntent::TransportRefresh => unreachable!("handled above"),
                }
            }
        } else if prepare.load_intent == MediaLoadIntent::TransportRefresh {
            return Ok(Vec::new());
        }

        let Some(generation) = self.allocate_playback_barrier_generation() else {
            return Ok(Vec::new());
        };
        prepare.media_generation = generation;
        let mut outbound = Vec::new();
        if supersedes_active {
            let barrier = self
                .room_playback_barriers
                .get_mut(&session.room)
                .expect("active supersession candidate should still exist");
            barrier.phase = PlaybackBarrierPhase::Degraded;
            for participant in barrier.participants.values_mut() {
                participant.status.phase = PlaybackBarrierParticipantPhase::Degraded;
                participant.status.degraded_reason =
                    Some(PlaybackBarrierDegradedReason::Superseded);
            }
            outbound.extend(self.playback_barrier_status_fanout(&session.room));
        }
        let now_seconds = self.current_time_seconds();
        let requested_timeout_seconds = prepare
            .timeout_ms
            .map(|timeout_ms| timeout_ms as f64 / 1_000.0)
            .unwrap_or(PLAYBACK_BARRIER_DEFAULT_TIMEOUT_SECONDS);
        let timeout_seconds = requested_timeout_seconds.clamp(
            PLAYBACK_BARRIER_MIN_TIMEOUT_SECONDS,
            PLAYBACK_BARRIER_MAX_TIMEOUT_SECONDS,
        );
        let deadline = now_seconds + timeout_seconds;
        prepare.target_position = prepare.target_position.max(0.0);
        prepare.timeout_action = Some(prepare.timeout_action.unwrap_or_default());
        prepare.timeout_ms = Some((timeout_seconds * 1_000.0) as u64);
        prepare.deadline = Some(deadline);

        let readiness_governed =
            self.readiness_enabled && self.room_readiness.contains_key(&session.room);
        let mut participants = BTreeMap::new();
        let mut excluded_legacy_clients = BTreeSet::new();
        for (peer_client_id, peer_session) in &self.sessions {
            if peer_session.room != session.room
                || self
                    .playback_barrier_fenced_clients
                    .contains(peer_client_id)
            {
                continue;
            }
            if peer_session.capabilities.playback_barrier_v1
                && (!readiness_governed || peer_session.capabilities.readiness_v2)
            {
                participants.insert(
                    peer_client_id.clone(),
                    RoomPlaybackBarrierParticipant {
                        username: peer_session.username.clone(),
                        status: PlaybackBarrierParticipantStatus::pending(),
                    },
                );
            } else {
                excluded_legacy_clients.insert(peer_session.username.clone());
            }
        }
        if participants.is_empty() {
            return Ok(Vec::new());
        }

        match prepare.policy {
            PlaybackBarrierPolicy::Quorum => {
                let eligible_count = participants.len().min(u32::MAX as usize) as u32;
                prepare.quorum = if let Some(quorum_percent) = prepare.quorum_percent {
                    let quorum_percent = quorum_percent.clamp(1, 100);
                    prepare.quorum_percent = Some(quorum_percent);
                    Some(room_buffering_quorum_required(
                        eligible_count,
                        quorum_percent,
                    ))
                } else {
                    Some(
                        prepare
                            .quorum
                            .unwrap_or(eligible_count)
                            .clamp(1, eligible_count),
                    )
                };
            }
            PlaybackBarrierPolicy::AllEligible | PlaybackBarrierPolicy::Controller => {
                prepare.quorum = None;
                prepare.quorum_percent = None;
            }
        }

        let room_name = session.room.clone();
        let target_position = prepare.target_position;
        self.retain_displaced_playback_barrier_requests(&room_name, displaced_requests);
        if self
            .room_buffering_controls
            .get(&room_name)
            .is_some_and(|control| control.config.media_generation != prepare.media_generation)
        {
            self.room_buffering_controls.remove(&room_name);
        } else if let Some(control) = self.room_buffering_controls.get_mut(&room_name) {
            // A start barrier owns the canonical pause until commit. Clear any
            // ongoing-policy episode so its timeout cannot resume the room
            // while the capable cohort is still preparing.
            control.reports.clear();
            control.condition_active_since = None;
            control.condition_clear_since = None;
            control.paused_by_policy = false;
            control.pause_deadline = None;
            control.fail_open_latched = false;
        }
        self.room_playback_barriers.insert(
            room_name.clone(),
            RoomPlaybackBarrier {
                prepare: prepare.clone(),
                commit: None,
                initiator_client_id: client_id.to_owned(),
                initiator_session_sequence,
                initiator_username: session.username.clone(),
                participants,
                excluded_legacy_clients,
                phase: PlaybackBarrierPhase::Preparing,
                state_revision: None,
                readiness_revision: None,
                deadline,
                started_deadline: None,
            },
        );
        {
            let room_state = self.room_playback_state_mut(&room_name);
            room_state.position = target_position;
            room_state.paused = true;
            room_state.updated_at_seconds = now_seconds;
            room_state.set_by = Some(session.username.clone());
        }
        self.seed_room_client_playback_states(&room_name, target_position, now_seconds);
        self.persist_room_if_needed(&room_name)?;

        outbound.extend(self.begin_readiness_generation(&room_name, generation)?);

        for peer_client in self.clients_in_room(&room_name) {
            let state_message = self.forced_state_sync_message_for_client(
                &peer_client,
                target_position,
                true,
                true,
                Some(&session.username),
            );
            outbound.push(DirectedProtocolMessage::new(peer_client, state_message));
        }
        let status = self
            .playback_barrier_status(&room_name)
            .expect("newly inserted playback barrier should have status");
        let extension = PlaybackBarrierSetExtension::new()
            .with_prepare(prepare)
            .with_status(status);
        outbound.extend(self.playback_barrier_fanout(&room_name, extension));
        Ok(outbound)
    }

    pub(crate) fn record_playback_barrier_ready(
        &mut self,
        client_id: &str,
        ready: MediaReadyPayload,
        bridge_technical_readiness: bool,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let Some(session) = self.sessions.get(client_id).cloned() else {
            return Err(ServerRuntimeError::MissingSession(client_id.to_owned()));
        };
        if !session.capabilities.playback_barrier_v1 {
            return Ok(Vec::new());
        }
        // Readiness V2 technical reports are the canonical, richer source for
        // V2 participants. In particular, mapping a temporarily or terminally
        // blocked report back through legacy MediaReady would erase its cause
        // and recovery state. The bridge is reserved for callers that have no
        // canonical V2 report for this observation.
        let mut outbound = if bridge_technical_readiness {
            self.apply_media_ready_to_v2(client_id, &ready)?
        } else {
            Vec::new()
        };
        let Some(barrier) = self.room_playback_barriers.get_mut(&session.room) else {
            return Ok(outbound);
        };
        if barrier.phase != PlaybackBarrierPhase::Preparing
            || ready.media_generation != barrier.prepare.media_generation
        {
            return Ok(outbound);
        }
        let Some(participant) = barrier.participants.get_mut(client_id) else {
            return Ok(outbound);
        };
        participant.status.phase = if ready.is_ready() {
            PlaybackBarrierParticipantPhase::Ready
        } else {
            PlaybackBarrierParticipantPhase::Pending
        };
        participant.status.readiness = Some(ready);
        participant.status.degraded_reason = None;

        if self.playback_barrier_policy_satisfied(&session.room) {
            outbound.extend(self.commit_playback_barrier(
                &session.room,
                false,
                self.current_time_seconds(),
            )?);
            return Ok(outbound);
        }
        outbound.extend(self.playback_barrier_status_fanout(&session.room));
        Ok(outbound)
    }

    fn record_playback_barrier_started(
        &mut self,
        client_id: &str,
        started: StartedAckPayload,
    ) -> Vec<DirectedProtocolMessage> {
        let Some(session) = self.sessions.get(client_id).cloned() else {
            return Vec::new();
        };
        if !session.capabilities.playback_barrier_v1 || !started.observed_position.is_finite() {
            return Vec::new();
        }
        let Some(barrier) = self.room_playback_barriers.get_mut(&session.room) else {
            return Vec::new();
        };
        if barrier.phase != PlaybackBarrierPhase::Committed
            || started.media_generation != barrier.prepare.media_generation
            || Some(started.state_revision) != barrier.state_revision
        {
            return Vec::new();
        }
        let Some(participant) = barrier.participants.get_mut(client_id) else {
            return Vec::new();
        };
        participant.status.phase = PlaybackBarrierParticipantPhase::Started;
        participant.status.observed_position = Some(started.observed_position.max(0.0));
        participant.status.degraded_reason = None;
        if barrier
            .participants
            .values()
            .all(|participant| participant.status.phase == PlaybackBarrierParticipantPhase::Started)
        {
            barrier.phase = PlaybackBarrierPhase::Complete;
        }
        self.playback_barrier_status_fanout(&session.room)
    }

    pub(crate) fn playback_barrier_policy_satisfied(&self, room_name: &str) -> bool {
        let Some(barrier) = self.room_playback_barriers.get(room_name) else {
            return false;
        };
        if !self.readiness_required_cohort_start_eligible(room_name) {
            return false;
        }
        let readiness_governed =
            self.readiness_enabled && self.room_readiness.contains_key(room_name);
        let participant_is_ready =
            |client_id: &str, participant: &RoomPlaybackBarrierParticipant| {
                if participant.status.phase != PlaybackBarrierParticipantPhase::Ready {
                    return false;
                }
                if !readiness_governed {
                    return true;
                }
                self.sessions.get(client_id).is_some_and(|session| {
                    session.room == room_name
                        && session.capabilities.playback_barrier_v1
                        && session.capabilities.readiness_v2
                        && self
                            .readiness_participant_is_start_eligible(room_name, &session.username)
                            == Some(true)
                })
            };
        match barrier.prepare.policy {
            PlaybackBarrierPolicy::AllEligible => {
                barrier.participants.iter().all(|(client_id, participant)| {
                    participant.status.phase == PlaybackBarrierParticipantPhase::Degraded
                        || participant_is_ready(client_id, participant)
                })
            }
            PlaybackBarrierPolicy::Controller => barrier
                .participants
                .get(&barrier.initiator_client_id)
                .is_some_and(|participant| {
                    participant_is_ready(&barrier.initiator_client_id, participant)
                }),
            PlaybackBarrierPolicy::Quorum => {
                let ready_count = barrier
                    .participants
                    .iter()
                    .filter(|(client_id, participant)| participant_is_ready(client_id, participant))
                    .count() as u32;
                ready_count >= barrier.prepare.quorum.unwrap_or(u32::MAX)
            }
        }
    }

    pub(crate) fn commit_playback_barrier(
        &mut self,
        room_name: &str,
        timed_out: bool,
        now_seconds: f64,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let Some((phase, media_generation)) = self
            .room_playback_barriers
            .get(room_name)
            .map(|barrier| (barrier.phase, barrier.prepare.media_generation))
        else {
            return Ok(Vec::new());
        };
        if phase != PlaybackBarrierPhase::Preparing {
            return Ok(Vec::new());
        }
        if self.readiness_enabled
            && self.room_readiness.contains_key(room_name)
            && (!self.playback_barrier_policy_satisfied(room_name)
                || !self.readiness_gate_owns_pause(room_name, media_generation))
        {
            return Ok(Vec::new());
        }
        let readiness_revision = self.readiness_revision_for_commit(room_name);
        let barrier = self
            .room_playback_barriers
            .get_mut(room_name)
            .expect("validated playback barrier should remain active");
        for participant in barrier.participants.values_mut() {
            if participant.status.phase == PlaybackBarrierParticipantPhase::Ready {
                continue;
            }
            if participant.status.phase == PlaybackBarrierParticipantPhase::Degraded
                && participant.status.degraded_reason
                    == Some(PlaybackBarrierDegradedReason::Disconnected)
            {
                continue;
            }
            participant.status.phase = if timed_out {
                PlaybackBarrierParticipantPhase::PrepareTimedOut
            } else {
                PlaybackBarrierParticipantPhase::Degraded
            };
            participant.status.degraded_reason = Some(if timed_out {
                PlaybackBarrierDegradedReason::PrepareTimeout
            } else {
                PlaybackBarrierDegradedReason::NotReadyAtCommit
            });
        }

        self.next_playback_barrier_revision = self.next_playback_barrier_revision.saturating_add(1);
        let revision = self.next_playback_barrier_revision;
        let started_deadline = now_seconds + PLAYBACK_BARRIER_STARTED_TIMEOUT_SECONDS;
        let media_generation = barrier.prepare.media_generation;
        let anchor_position = barrier.prepare.target_position;
        let initiator_username = barrier.initiator_username.clone();
        barrier.phase = PlaybackBarrierPhase::Committed;
        barrier.state_revision = Some(revision);
        barrier.readiness_revision = readiness_revision;
        barrier.started_deadline = Some(started_deadline);

        {
            let room_state = self.room_playback_state_mut(room_name);
            room_state.position = anchor_position;
            room_state.paused = false;
            room_state.updated_at_seconds = now_seconds;
            room_state.set_by = Some(initiator_username.clone());
        }
        self.seed_room_client_playback_states(room_name, anchor_position, now_seconds);
        self.persist_room_if_needed(room_name)?;

        let mut commit = CommitStartPayload::new(
            media_generation,
            revision,
            anchor_position,
            now_seconds,
            started_deadline,
        );
        if let Some(readiness_revision) = readiness_revision {
            commit = commit.with_readiness_revision(readiness_revision);
        }
        if let Some(barrier) = self.room_playback_barriers.get_mut(room_name) {
            barrier.commit = Some(commit.clone());
        }
        let status = self
            .playback_barrier_status(room_name)
            .expect("committed playback barrier should have status");
        let mut outbound = self.playback_barrier_fanout(
            room_name,
            PlaybackBarrierSetExtension::new()
                .with_commit(commit)
                .with_status(status),
        );
        for peer_client in self.clients_in_room(room_name) {
            let state_message = self.forced_state_sync_message_for_client(
                &peer_client,
                anchor_position,
                false,
                true,
                Some(&initiator_username),
            );
            outbound.push(DirectedProtocolMessage::new(peer_client, state_message));
        }
        if let Some(readiness_revision) = readiness_revision {
            outbound.extend(self.mark_readiness_gate_committed(
                room_name,
                media_generation,
                readiness_revision,
                revision,
            ));
        }
        Ok(outbound)
    }

    fn finish_prepare_timeout_without_commit(
        &mut self,
        room_name: &str,
        awaiting_controller_decision: bool,
    ) -> Vec<DirectedProtocolMessage> {
        let Some(barrier) = self.room_playback_barriers.get_mut(room_name) else {
            return Vec::new();
        };
        if barrier.phase != PlaybackBarrierPhase::Preparing {
            return Vec::new();
        }
        for participant in barrier.participants.values_mut() {
            if participant.status.phase == PlaybackBarrierParticipantPhase::Ready {
                continue;
            }
            participant.status.phase = PlaybackBarrierParticipantPhase::PrepareTimedOut;
            participant.status.degraded_reason =
                Some(PlaybackBarrierDegradedReason::PrepareTimeout);
        }
        barrier.phase = if awaiting_controller_decision {
            PlaybackBarrierPhase::AwaitingDecision
        } else {
            PlaybackBarrierPhase::Degraded
        };
        // The canonical room state intentionally remains paused. No commit or
        // transient unpause is emitted for either policy.
        self.playback_barrier_status_fanout(room_name)
    }

    /// Retires an `AskController` timeout once an authorized ordinary
    /// play/pause transition supplies the requested decision. The ordinary
    /// playstate remains the canonical control path; no stale barrier commit
    /// is manufactured or reactivated.
    pub(crate) fn retire_awaiting_playback_barrier_decision(
        &mut self,
        client_id: &str,
        room_name: &str,
    ) -> Vec<DirectedProtocolMessage> {
        let authorized = self.sessions.get(client_id).is_some_and(|session| {
            session.room == room_name
                && session.capabilities.playback_barrier_v1
                && self.user_can_control_playlist(&session.username, room_name)
        });
        if !authorized {
            return Vec::new();
        }
        let Some(barrier) = self.room_playback_barriers.get_mut(room_name) else {
            return Vec::new();
        };
        if barrier.phase != PlaybackBarrierPhase::AwaitingDecision {
            return Vec::new();
        }
        barrier.phase = PlaybackBarrierPhase::Degraded;
        self.playback_barrier_status_fanout(room_name)
    }

    pub(crate) fn collect_due_playback_barrier_updates_at(
        &mut self,
        now_seconds: f64,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let due_prepare: Vec<String> = self
            .room_playback_barriers
            .iter()
            .filter(|(_, barrier)| {
                barrier.phase == PlaybackBarrierPhase::Preparing && barrier.deadline <= now_seconds
            })
            .map(|(room_name, _)| room_name.clone())
            .collect();
        let mut outbound = Vec::new();
        for room_name in due_prepare {
            let timeout_action = self
                .room_playback_barriers
                .get(&room_name)
                .and_then(|barrier| barrier.prepare.timeout_action)
                .unwrap_or_default();
            match timeout_action {
                PlaybackBarrierTimeoutAction::Continue => {
                    let readiness_commit_blocked = self.readiness_enabled
                        && self.room_readiness.contains_key(&room_name)
                        && self
                            .room_playback_barriers
                            .get(&room_name)
                            .is_some_and(|barrier| {
                                !self.playback_barrier_policy_satisfied(&room_name)
                                    || !self.readiness_gate_owns_pause(
                                        &room_name,
                                        barrier.prepare.media_generation,
                                    )
                            });
                    if readiness_commit_blocked {
                        outbound
                            .extend(self.finish_prepare_timeout_without_commit(&room_name, false));
                        outbound.extend(self.mark_readiness_gate_degraded(
                            &room_name,
                            StartGateDegradedReason::TimedOut,
                        ));
                    } else {
                        outbound.extend(self.commit_playback_barrier(
                            &room_name,
                            true,
                            now_seconds,
                        )?);
                    }
                }
                PlaybackBarrierTimeoutAction::RemainPaused => {
                    outbound.extend(self.finish_prepare_timeout_without_commit(&room_name, false));
                    if self.readiness_enabled && self.room_readiness.contains_key(&room_name) {
                        outbound.extend(self.mark_readiness_gate_degraded(
                            &room_name,
                            StartGateDegradedReason::TimedOut,
                        ));
                    }
                }
                PlaybackBarrierTimeoutAction::AskController => {
                    outbound.extend(self.finish_prepare_timeout_without_commit(&room_name, true));
                    if self.readiness_enabled && self.room_readiness.contains_key(&room_name) {
                        // The barrier's AwaitingDecision phase carries the
                        // pending human-decision detail. The automatic gate
                        // itself has terminated, so expose the timeout rather
                        // than leaving its last waiting phase stale.
                        outbound.extend(self.mark_readiness_gate_degraded(
                            &room_name,
                            StartGateDegradedReason::TimedOut,
                        ));
                    }
                }
            }
        }

        let due_started: Vec<String> = self
            .room_playback_barriers
            .iter()
            .filter(|(_, barrier)| {
                barrier.phase == PlaybackBarrierPhase::Committed
                    && barrier
                        .started_deadline
                        .is_some_and(|deadline| deadline <= now_seconds)
            })
            .map(|(room_name, _)| room_name.clone())
            .collect();
        for room_name in due_started {
            let Some(barrier) = self.room_playback_barriers.get_mut(&room_name) else {
                continue;
            };
            for participant in barrier.participants.values_mut() {
                match participant.status.phase {
                    PlaybackBarrierParticipantPhase::Pending
                    | PlaybackBarrierParticipantPhase::Ready => {
                        participant.status.phase =
                            PlaybackBarrierParticipantPhase::StartedAckTimedOut;
                        participant.status.degraded_reason =
                            Some(PlaybackBarrierDegradedReason::StartedTimeout);
                    }
                    PlaybackBarrierParticipantPhase::Started
                    | PlaybackBarrierParticipantPhase::Degraded
                    | PlaybackBarrierParticipantPhase::PrepareTimedOut
                    | PlaybackBarrierParticipantPhase::StartedAckTimedOut => {}
                }
            }
            barrier.phase = PlaybackBarrierPhase::Degraded;
            outbound.extend(self.playback_barrier_status_fanout(&room_name));
        }
        let buffering_rooms: Vec<String> = self.room_buffering_controls.keys().cloned().collect();
        for room_name in buffering_rooms {
            outbound.extend(self.evaluate_room_buffering_at(&room_name, now_seconds, false)?);
        }
        Ok(outbound)
    }

    pub(crate) fn mark_room_buffering_participant_disconnected(
        &mut self,
        client_id: &str,
        room_name: &str,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let now_seconds = self.current_time_seconds();
        let Some(control) = self.room_buffering_controls.get_mut(room_name) else {
            return Ok(Vec::new());
        };
        if self.playback_barrier_fenced_clients.contains(client_id)
            && control.configured_by_client_id != client_id
            && !control.reports.contains_key(client_id)
        {
            // The recovered room has already transferred and scrubbed this
            // identity. A fenced connection can nevertheless own unrelated
            // state in its current room, so only that still-associated state
            // should participate in physical disconnect cleanup.
            return Ok(Vec::new());
        }
        control.reports.remove(client_id);
        if control.configured_by_client_id == client_id {
            let should_resume = control.paused_by_policy;
            let set_by = control.configured_by_username.clone();
            control.config.policy = RoomBufferingPolicy::Independent;
            control.config.quorum_percent = None;
            control.condition_active_since = None;
            control.condition_clear_since = None;
            control.paused_by_policy = false;
            control.pause_deadline = None;
            control.fail_open_latched = false;
            let mut outbound = if should_resume {
                self.apply_room_buffering_transition(
                    room_name,
                    RoomBufferingTransition::Resume,
                    &set_by,
                    now_seconds,
                )?
            } else {
                Vec::new()
            };
            if let Some(status) = self.room_buffering_status(room_name) {
                outbound.extend(self.room_buffering_fanout(
                    room_name,
                    PlaybackBarrierSetExtension::new().with_buffering_status(status),
                ));
            }
            return Ok(outbound);
        }
        self.evaluate_room_buffering_at(room_name, now_seconds, true)
    }

    pub(crate) fn mark_playback_barrier_participant_disconnected(
        &mut self,
        client_id: &str,
        room_name: &str,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let Some(barrier) = self.room_playback_barriers.get_mut(room_name) else {
            return Ok(Vec::new());
        };
        if self.playback_barrier_fenced_clients.contains(client_id)
            && barrier.initiator_client_id != client_id
            && !barrier.participants.contains_key(client_id)
        {
            return Ok(Vec::new());
        }
        if !matches!(
            barrier.phase,
            PlaybackBarrierPhase::Preparing | PlaybackBarrierPhase::Committed
        ) {
            return Ok(Vec::new());
        }
        let Some(participant) = barrier.participants.get_mut(client_id) else {
            return Ok(Vec::new());
        };
        participant.status.phase = PlaybackBarrierParticipantPhase::Degraded;
        participant.status.degraded_reason = Some(PlaybackBarrierDegradedReason::Disconnected);
        if barrier.phase == PlaybackBarrierPhase::Preparing
            && self.playback_barrier_policy_satisfied(room_name)
        {
            return self.commit_playback_barrier(room_name, false, self.current_time_seconds());
        }
        Ok(self.playback_barrier_status_fanout(room_name))
    }

    fn playback_barrier_status(&self, room_name: &str) -> Option<PlaybackBarrierStatusPayload> {
        let barrier = self.room_playback_barriers.get(room_name)?;
        let participants = barrier
            .participants
            .values()
            .map(|participant| (participant.username.clone(), participant.status.clone()))
            .collect();
        Some(PlaybackBarrierStatusPayload {
            media_generation: barrier.prepare.media_generation,
            state_revision: barrier.state_revision,
            phase: barrier.phase,
            policy: barrier.prepare.policy,
            quorum: barrier.prepare.quorum,
            deadline: barrier.started_deadline.unwrap_or(barrier.deadline),
            participants,
            excluded_legacy_clients: barrier.excluded_legacy_clients.clone(),
        })
    }

    pub(crate) fn playback_barrier_status_fanout(
        &self,
        room_name: &str,
    ) -> Vec<DirectedProtocolMessage> {
        let Some(status) = self.playback_barrier_status(room_name) else {
            return Vec::new();
        };
        self.playback_barrier_fanout(
            room_name,
            PlaybackBarrierSetExtension::new().with_status(status),
        )
    }

    fn playback_barrier_fanout(
        &self,
        room_name: &str,
        extension: PlaybackBarrierSetExtension,
    ) -> Vec<DirectedProtocolMessage> {
        let Some(barrier) = self.room_playback_barriers.get(room_name) else {
            return Vec::new();
        };
        barrier
            .participants
            .keys()
            .filter(|client_id| {
                self.sessions.get(*client_id).is_some_and(|session| {
                    session.room == room_name && session.capabilities.playback_barrier_v1
                })
            })
            .map(|client_id| {
                let mut extension = extension.clone();
                self.redact_playback_barrier_request_identity_for_client(
                    room_name,
                    client_id,
                    &mut extension,
                );
                DirectedProtocolMessage::new(client_id, playback_barrier_set_message(extension))
            })
            .collect()
    }
}

fn normalize_room_buffering_duration_ms(
    requested_ms: Option<u64>,
    default_seconds: f64,
    min_seconds: f64,
    max_seconds: f64,
) -> u64 {
    let requested_seconds = requested_ms
        .map(|milliseconds| milliseconds as f64 / 1_000.0)
        .unwrap_or(default_seconds);
    (requested_seconds.clamp(min_seconds, max_seconds) * 1_000.0) as u64
}

fn valid_playback_barrier_request_id(request_id: &str) -> bool {
    !request_id.is_empty()
        && request_id.len() <= PLAYBACK_BARRIER_MAX_REQUEST_ID_BYTES
        && request_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn playback_barrier_logical_media_id_digest(logical_media_id: &str) -> [u8; 32] {
    Sha256::digest(logical_media_id.as_bytes()).into()
}

fn room_buffering_config_seconds(value_ms: Option<u64>) -> f64 {
    value_ms.unwrap_or_default() as f64 / 1_000.0
}

fn room_buffering_quorum_required(eligible_clients: u32, quorum_percent: u32) -> u32 {
    if eligible_clients == 0 {
        return 0;
    }
    eligible_clients
        .saturating_mul(quorum_percent.clamp(1, 100))
        .saturating_add(99)
        / 100
}

fn playback_barrier_recovery_result(
    query: &PlaybackBarrierRecoveryPayload,
    disposition: PlaybackBarrierRecoveryDisposition,
    media_generation: Option<u64>,
) -> PlaybackBarrierRecoveryPayload {
    let result = PlaybackBarrierRecoveryPayload::result(
        query.request_id.clone(),
        query.original_request_nonce,
        query.recovery_nonce,
        query.logical_media_id.clone(),
        disposition,
    );
    if let Some(media_generation) = media_generation {
        result.with_media_generation(media_generation)
    } else {
        result
    }
}
