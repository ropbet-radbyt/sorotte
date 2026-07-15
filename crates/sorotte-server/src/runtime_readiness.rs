use super::*;

impl ServerRuntime {
    pub(crate) fn attach_readiness_membership(
        &mut self,
        client_id: &str,
        presented_reconnect_token: Option<&SecretValue>,
        issue_reconnect_token: bool,
        seed_legacy_intent: bool,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        self.pending_user_transport_by_client.remove(client_id);
        let Some(session) = self.sessions.get(client_id).cloned() else {
            return Err(ServerRuntimeError::MissingSession(client_id.to_owned()));
        };
        if !self.readiness_enabled || !session.capabilities.readiness_v2 {
            self.clear_pending_user_transport(client_id);
            return Ok(Vec::new());
        }
        let participation_role = if session.capabilities.playback_barrier_v1 {
            StartParticipationRole::Required
        } else {
            StartParticipationRole::ExcludedLegacy
        };

        self.prune_readiness_reconnect_cache();
        let restored = if issue_reconnect_token {
            presented_reconnect_token
                .filter(|token| token.expose_secret().len() <= READINESS_MAX_OPERATION_ID_BYTES)
                .map(|token| readiness_reconnect_token_digest(token.expose_secret()))
                .and_then(|digest| {
                    self.readiness_reconnect_cache
                        .get(&digest)
                        .filter(|membership| {
                            membership.room_name == session.room
                                && membership.username == session.username
                        })?;
                    self.readiness_reconnect_cache.remove(&digest)
                })
        } else {
            None
        };
        let restored_membership = restored.is_some();
        if issue_reconnect_token {
            let reconnect_identity = if restored_membership {
                presented_reconnect_token
                    .cloned()
                    .expect("restored membership must have presented a reconnect token")
            } else {
                generate_readiness_reconnect_token()
            };
            self.readiness_reconnect_identity_by_client
                .insert(client_id.to_owned(), reconnect_identity);
        }

        let current_generation = self
            .room_readiness
            .get(&session.room)
            .and_then(|room| room.media_generation)
            .or_else(|| {
                self.room_playback_barriers
                    .get(&session.room)
                    .map(|barrier| barrier.prepare.media_generation)
            });
        let restored_revision = restored
            .as_ref()
            .map(|membership| membership.room_readiness_revision)
            .unwrap_or_default();
        let initialized_user_intent = if seed_legacy_intent
            && self.stored_user_ready(&session.username, &session.room) == Some(true)
        {
            UserReadinessIntent::Ready
        } else {
            UserReadinessIntent::NotReady
        };
        let mut participant = if let Some(restored) = restored {
            ServerReadinessParticipant {
                client_id: client_id.to_owned(),
                record: ParticipantReadiness {
                    membership_epoch: restored.membership_epoch,
                    last_technical_report_sequence: restored.last_technical_report_sequence,
                    user_intent: restored.user_intent,
                    user_intent_revision: restored.user_intent_revision,
                    last_user_mutation: restored.last_user_mutation,
                    technical_state: current_generation
                        .map_or(TechnicalPlayability::Unknown, |media_generation| {
                            TechnicalPlayability::Preparing { media_generation }
                        }),
                    terminal_technical_block: None,
                    participation_role,
                    room_ready: false,
                    start_eligible: false,
                },
                initialization_open: restored.initialization_open,
                highest_request_nonce: 0,
                accepted_operations: restored.accepted_operations,
                pending_automatic_pause_owner: None,
                // Client monotonic clocks may restart with a replacement
                // process. Sequence ordering survives reconnect; timestamp
                // monotonicity is scoped to one live transport.
                last_technical_observed_at: None,
            }
        } else {
            let membership_epoch = self.allocate_readiness_membership_epoch();
            ServerReadinessParticipant {
                client_id: client_id.to_owned(),
                record: ParticipantReadiness {
                    membership_epoch,
                    last_technical_report_sequence: 0,
                    // A live legacy-to-V2 capability upgrade is still the same
                    // authenticated room membership. Seed its acknowledged
                    // room-facing intent instead of silently forcing Ready
                    // back to NotReady. Fresh joins and V2 room switches have
                    // no true legacy projection and therefore remain NotReady.
                    user_intent: initialized_user_intent,
                    user_intent_revision: 0,
                    last_user_mutation: None,
                    technical_state: current_generation
                        .map_or(TechnicalPlayability::Unknown, |media_generation| {
                            TechnicalPlayability::Preparing { media_generation }
                        }),
                    terminal_technical_block: None,
                    participation_role,
                    room_ready: false,
                    start_eligible: false,
                },
                initialization_open: true,
                highest_request_nonce: 0,
                accepted_operations: BTreeMap::new(),
                pending_automatic_pause_owner: None,
                last_technical_observed_at: None,
            }
        };
        participant.client_id = client_id.to_owned();
        if restored_membership {
            participant.initialization_open = false;
        }
        // A readiness-capable client that cannot participate in the
        // generation-scoped playback barrier is explicitly excluded. It must
        // never remain Required while being absent from the technical cohort.
        participant.record.participation_role = participation_role;
        if let Some(media_generation) = current_generation
            && participant.record.technical_state.media_generation() != Some(media_generation)
        {
            participant.record.technical_state =
                TechnicalPlayability::Preparing { media_generation };
            participant.record.terminal_technical_block = None;
        }

        let revision = {
            let room = self.room_readiness.entry(session.room.clone()).or_default();
            room.revision = room.revision.max(restored_revision).saturating_add(1);
            room.media_generation = current_generation.or(room.media_generation);
            recompute_participant_readiness(&mut participant.record, room.media_generation);
            if participant.record.last_user_mutation.is_none() {
                participant.record.last_user_mutation = Some(ReadinessMutationMetadata::new(
                    ReadinessMutationSource::Initialization,
                    room.revision,
                ));
            }
            room.participants
                .insert(session.username.clone(), participant);
            room.revision
        };

        let active_media_generation = self
            .room_playback_barriers
            .get(&session.room)
            .filter(|barrier| barrier.phase == PlaybackBarrierPhase::Preparing)
            .map(|barrier| barrier.prepare.media_generation);
        self.reconcile_mixed_readiness_cohort_state(&session.room);
        if let Some(media_generation) = active_media_generation
            && self
                .room_readiness
                .get(&session.room)
                .is_some_and(|room| !matches!(room.pause_owner, RoomPauseOwner::User { .. }))
        {
            self.set_readiness_pause_owner(
                &session.room,
                RoomPauseOwner::ReadinessStartGate { media_generation },
                false,
            );
        }
        self.refresh_readiness_gate_phase(&session.room);
        let room_ready = self
            .readiness_record(&session.room, &session.username)
            .is_some_and(|record| record.room_ready);
        self.domain
            .set_ready(&session.username, &session.room, room_ready)?;

        let mut outbound = self.legacy_readiness_projection_fanout(
            &session.room,
            &session.username,
            room_ready,
            false,
            None,
        );
        outbound.extend(self.readiness_participant_fanout(&session.room, &session.username, None));
        outbound.extend(self.readiness_snapshot_fanout(&session.room));

        if active_media_generation.is_some() {
            // A mixed cohort policy change is always reflected in status; the
            // newly attached client also needs the current barrier snapshot
            // when it supports that extension.
            outbound.extend(self.playback_barrier_status_fanout(&session.room));
            outbound.extend(self.playback_barrier_snapshot_for_client(&session.room, client_id));
        }
        outbound.extend(self.maybe_commit_readiness_gate(&session.room)?);

        debug_assert!(revision > 0);
        Ok(outbound)
    }

    pub(crate) fn detach_readiness_membership(
        &mut self,
        client_id: &str,
        retain_for_reconnect: bool,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        self.pending_user_transport_by_client.remove(client_id);
        let Some(session) = self.sessions.get(client_id).cloned() else {
            return Ok(Vec::new());
        };
        let mut removed = None;
        let mut room_revision = 0;
        if let Some(room) = self.room_readiness.get_mut(&session.room) {
            if room
                .participants
                .get(&session.username)
                .is_some_and(|participant| participant.client_id == client_id)
            {
                removed = room.participants.remove(&session.username);
            }
            if removed.is_some() {
                room.revision = room.revision.saturating_add(1);
            }
            room_revision = room.revision;
        }
        let Some(participant) = removed else {
            return Ok(Vec::new());
        };

        let reconnect_identity = if retain_for_reconnect {
            self.readiness_reconnect_identity_by_client
                .get(client_id)
                .cloned()
        } else {
            None
        };
        if let Some(reconnect_identity) = reconnect_identity {
            let record = participant.record;
            self.readiness_reconnect_cache.insert(
                readiness_reconnect_token_digest(reconnect_identity.expose_secret()),
                DetachedReadinessMembership {
                    room_name: session.room.clone(),
                    username: session.username.clone(),
                    membership_epoch: record.membership_epoch,
                    user_intent: record.user_intent,
                    user_intent_revision: record.user_intent_revision,
                    last_user_mutation: record.last_user_mutation,
                    last_technical_report_sequence: record.last_technical_report_sequence,
                    initialization_open: participant.initialization_open,
                    accepted_operations: participant.accepted_operations,
                    room_readiness_revision: room_revision,
                    detached_at_seconds: self.current_time_seconds(),
                },
            );
        }
        self.refresh_readiness_gate_phase(&session.room);
        let mut outbound = self.readiness_snapshot_fanout(&session.room);
        outbound.extend(self.maybe_commit_readiness_gate(&session.room)?);
        if self
            .room_readiness
            .get(&session.room)
            .is_some_and(|room| room.participants.is_empty())
        {
            self.room_readiness.remove(&session.room);
        }
        Ok(outbound)
    }

    pub(crate) fn handle_readiness_set(
        &mut self,
        client_id: &str,
        extension: ReadinessSetExtension,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        if !self.readiness_enabled {
            return Ok(Vec::new());
        }
        let Some(intent) = extension.intent else {
            // Participant updates, snapshots, and request results are server-owned.
            return Ok(Vec::new());
        };
        self.apply_readiness_intent(client_id, intent)
    }

    pub(crate) fn handle_readiness_state(
        &mut self,
        client_id: &str,
        extension: ReadinessStateExtension,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        if !self.readiness_enabled {
            return Ok(Vec::new());
        }
        let Some(report) = extension.technical else {
            return Ok(Vec::new());
        };
        if !self.technical_readiness_report_is_current(client_id, &report) {
            return Ok(Vec::new());
        }
        self.apply_technical_readiness_report(client_id, report)
    }

    pub(crate) fn apply_legacy_readiness_to_v2(
        &mut self,
        client_id: &str,
        target_username: &str,
        desired: bool,
        manually_initiated: bool,
    ) -> Result<Option<Vec<DirectedProtocolMessage>>, ServerRuntimeError> {
        if !self.readiness_enabled {
            return Ok(None);
        }
        let Some(session) = self.sessions.get(client_id).cloned() else {
            return Err(ServerRuntimeError::MissingSession(client_id.to_owned()));
        };
        let target_has_v2_membership = self
            .readiness_record(&session.room, target_username)
            .is_some();
        if session.capabilities.readiness_v2 {
            // A V2 actor must use the operation-, nonce-, and membership-scoped
            // readiness command for every V2 target. Treating a raw legacy
            // Set.ready as initialization would let reconnect projections and
            // stale compatibility bytes bypass those fences. A genuinely
            // legacy target still falls through to the authenticated legacy
            // controller path below the bridge.
            return Ok(if target_has_v2_membership {
                Some(Vec::new())
            } else {
                None
            });
        }
        if !target_has_v2_membership {
            return Ok(None);
        }
        let target_is_self = target_username == session.username;
        if !target_is_self && !self.user_can_control_playlist(&session.username, &session.room) {
            return Ok(Some(Vec::new()));
        }
        let canonical_source = if target_is_self {
            if manually_initiated {
                ReadinessMutationSource::DirectUser {
                    surface: DirectReadinessSurface::RemoteControlSurface,
                }
            } else {
                ReadinessMutationSource::Initialization
            }
        } else {
            ReadinessMutationSource::ControllerOverride {
                actor: session.username.clone(),
            }
        };
        let desired = if desired {
            UserReadinessIntent::Ready
        } else {
            UserReadinessIntent::NotReady
        };
        let Some((room_ready, revision)) = self.mutate_readiness_intent(
            &session.room,
            target_username,
            desired,
            canonical_source,
            &session.username,
            None,
        ) else {
            return Ok(Some(Vec::new()));
        };
        self.domain
            .set_ready(target_username, &session.room, room_ready)?;
        self.refresh_readiness_gate_phase(&session.room);

        let mut outbound = self.legacy_readiness_projection_fanout(
            &session.room,
            target_username,
            room_ready,
            manually_initiated || !target_is_self,
            Some(&session.username),
        );
        outbound.extend(self.readiness_participant_fanout(&session.room, target_username, None));
        outbound.extend(self.readiness_snapshot_fanout(&session.room));
        outbound.extend(self.maybe_commit_readiness_gate(&session.room)?);
        debug_assert!(revision > 0);
        Ok(Some(outbound))
    }

    pub(crate) fn begin_readiness_generation(
        &mut self,
        room_name: &str,
        media_generation: u64,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        self.pending_user_transport_by_client
            .retain(|_, pending| pending.room_name != room_name);
        if !self.readiness_enabled {
            return Ok(Vec::new());
        }
        let Some(room) = self.room_readiness.get_mut(room_name) else {
            return Ok(Vec::new());
        };
        let previous_projection: BTreeMap<String, bool> = room
            .participants
            .iter()
            .map(|(username, participant)| (username.clone(), participant.record.room_ready))
            .collect();
        room.revision = room.revision.saturating_add(1);
        room.media_generation = Some(media_generation);
        if !matches!(room.pause_owner, RoomPauseOwner::User { .. }) {
            room.pause_owner = RoomPauseOwner::ReadinessStartGate { media_generation };
        }
        for participant in room.participants.values_mut() {
            participant.record.technical_state =
                TechnicalPlayability::Preparing { media_generation };
            participant.record.terminal_technical_block = None;
            participant.pending_automatic_pause_owner = None;
            recompute_participant_readiness(&mut participant.record, Some(media_generation));
        }
        self.refresh_readiness_gate_phase(room_name);

        let mut outbound = Vec::new();
        let usernames: Vec<String> = self
            .room_readiness
            .get(room_name)
            .into_iter()
            .flat_map(|room| room.participants.keys().cloned())
            .collect();
        for username in usernames {
            let room_ready = self
                .readiness_record(room_name, &username)
                .is_some_and(|record| record.room_ready);
            self.domain.set_ready(&username, room_name, room_ready)?;
            if previous_projection.get(&username).copied() != Some(room_ready) {
                outbound.extend(self.legacy_readiness_projection_fanout(
                    room_name, &username, room_ready, false, None,
                ));
            }
            outbound.extend(self.readiness_participant_fanout(room_name, &username, None));
        }
        outbound.extend(self.readiness_snapshot_fanout(room_name));
        Ok(outbound)
    }

    pub(crate) fn readiness_revision_for_commit(&self, room_name: &str) -> Option<u64> {
        if !self.readiness_enabled {
            return None;
        }
        self.room_readiness.get(room_name).and_then(|room| {
            matches!(room.pause_owner, RoomPauseOwner::ReadinessStartGate { .. })
                .then_some(room.revision)
        })
    }

    pub(crate) fn mark_readiness_gate_committed(
        &mut self,
        room_name: &str,
        media_generation: u64,
        readiness_revision: u64,
        playback_revision: u64,
    ) -> Vec<DirectedProtocolMessage> {
        self.pending_user_transport_by_client
            .retain(|_, pending| pending.room_name != room_name);
        let Some(room) = self.room_readiness.get_mut(room_name) else {
            return Vec::new();
        };
        room.revision = room.revision.saturating_add(1);
        room.start_gate_phase = RoomStartGatePhase::Committed {
            media_generation,
            readiness_revision,
            playback_revision,
        };
        room.pause_owner = RoomPauseOwner::None;
        self.readiness_snapshot_fanout(room_name)
    }

    pub(crate) fn mark_readiness_gate_degraded(
        &mut self,
        room_name: &str,
        reason: StartGateDegradedReason,
    ) -> Vec<DirectedProtocolMessage> {
        let Some(room) = self.room_readiness.get_mut(room_name) else {
            return Vec::new();
        };
        let Some(media_generation) = room.media_generation else {
            return Vec::new();
        };
        room.revision = room.revision.saturating_add(1);
        room.start_gate_phase = RoomStartGatePhase::Degraded {
            media_generation,
            reason,
        };
        self.readiness_snapshot_fanout(room_name)
    }

    pub(crate) fn readiness_gate_owns_pause(&self, room_name: &str, media_generation: u64) -> bool {
        self.room_readiness.get(room_name).is_none_or(|room| {
            matches!(
                room.pause_owner,
                RoomPauseOwner::ReadinessStartGate {
                    media_generation: owned_generation,
                } if owned_generation == media_generation
            )
        })
    }

    pub(crate) fn readiness_participant_is_start_eligible(
        &self,
        room_name: &str,
        username: &str,
    ) -> Option<bool> {
        self.readiness_record(room_name, username)
            .map(|record| record.start_eligible)
    }

    /// Returns whether the entire readiness-governed start cohort is both
    /// represented in the active playback barrier and eligible for its current
    /// generation. Rooms without a V2 coordinator retain the V1-only policy.
    pub(crate) fn readiness_required_cohort_start_eligible(&self, room_name: &str) -> bool {
        if !self.readiness_enabled {
            return true;
        }
        let Some(room) = self.room_readiness.get(room_name) else {
            return true;
        };
        if self.readiness_mixed_room_blocks_start(room_name) {
            return false;
        }
        let Some(barrier) = self.room_playback_barriers.get(room_name) else {
            return false;
        };
        let required: Vec<_> = room
            .participants
            .values()
            .filter(|participant| {
                participant.record.participation_role == StartParticipationRole::Required
            })
            .collect();
        !required.is_empty()
            && required.iter().all(|participant| {
                participant.record.start_eligible
                    && barrier.participants.contains_key(&participant.client_id)
                    && self
                        .sessions
                        .get(&participant.client_id)
                        .is_some_and(|session| {
                            session.room == room_name
                                && session.capabilities.readiness_v2
                                && session.capabilities.playback_barrier_v1
                        })
            })
    }

    fn readiness_mixed_room_blocks_start(&self, room_name: &str) -> bool {
        if self.mixed_readiness_policy == MixedReadinessPolicy::ExcludeLegacy
            || !self.room_readiness.contains_key(room_name)
        {
            return false;
        }
        self.sessions.iter().any(|(client_id, session)| {
            session.room == room_name
                && !self.playback_barrier_fenced_clients.contains(client_id)
                && !(session.capabilities.readiness_v2 && session.capabilities.playback_barrier_v1)
        })
    }

    /// Re-evaluates the explicit mixed-room cohort after feature negotiation or
    /// a V2 member joins an already-preparing V1 room. In a readiness-governed
    /// room, only clients supporting both extensions are technical participants;
    /// every other current member is wire-visible as `excludedLegacyClients`.
    pub(crate) fn refresh_mixed_readiness_cohort(
        &mut self,
        room_name: &str,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let (changed_usernames, barrier_changed) =
            self.reconcile_mixed_readiness_cohort_state(room_name);
        if changed_usernames.is_empty() && !barrier_changed {
            return Ok(Vec::new());
        }
        self.refresh_readiness_gate_phase(room_name);
        let mut outbound = Vec::new();
        for username in &changed_usernames {
            outbound.extend(self.readiness_participant_fanout(room_name, username, None));
        }
        if !changed_usernames.is_empty() {
            outbound.extend(self.readiness_snapshot_fanout(room_name));
        }
        if barrier_changed {
            outbound.extend(self.playback_barrier_status_fanout(room_name));
        }
        outbound.extend(self.maybe_commit_readiness_gate(room_name)?);
        Ok(outbound)
    }

    fn reconcile_mixed_readiness_cohort_state(&mut self, room_name: &str) -> (Vec<String>, bool) {
        if !self.readiness_enabled || !self.room_readiness.contains_key(room_name) {
            return (Vec::new(), false);
        }

        let media_generation = self
            .room_readiness
            .get(room_name)
            .and_then(|room| room.media_generation);
        let desired_roles: BTreeMap<String, StartParticipationRole> = self
            .room_readiness
            .get(room_name)
            .into_iter()
            .flat_map(|room| room.participants.iter())
            .map(|(username, participant)| {
                let required = self
                    .sessions
                    .get(&participant.client_id)
                    .is_some_and(|session| {
                        session.room == room_name
                            && session.username == *username
                            && session.capabilities.readiness_v2
                            && session.capabilities.playback_barrier_v1
                            && !self
                                .playback_barrier_fenced_clients
                                .contains(&participant.client_id)
                    });
                (
                    username.clone(),
                    if required {
                        StartParticipationRole::Required
                    } else {
                        StartParticipationRole::ExcludedLegacy
                    },
                )
            })
            .collect();

        let mut changed_usernames = Vec::new();
        if let Some(room) = self.room_readiness.get_mut(room_name) {
            for (username, participant) in &mut room.participants {
                let desired = desired_roles
                    .get(username)
                    .copied()
                    .unwrap_or(StartParticipationRole::ExcludedLegacy);
                if participant.record.participation_role != desired {
                    participant.record.participation_role = desired;
                    recompute_participant_readiness(&mut participant.record, media_generation);
                    changed_usernames.push(username.clone());
                }
            }
            if !changed_usernames.is_empty() {
                room.revision = room.revision.saturating_add(1);
            }
        }

        let required_clients: BTreeMap<String, String> = self
            .room_readiness
            .get(room_name)
            .into_iter()
            .flat_map(|room| room.participants.iter())
            .filter(|(_, participant)| {
                participant.record.participation_role == StartParticipationRole::Required
            })
            .map(|(username, participant)| (participant.client_id.clone(), username.clone()))
            .collect();
        let excluded_clients: BTreeSet<String> = self
            .sessions
            .iter()
            .filter(|(client_id, session)| {
                session.room == room_name
                    && !required_clients.contains_key(*client_id)
                    && !self.playback_barrier_fenced_clients.contains(*client_id)
            })
            .map(|(_, session)| session.username.clone())
            .collect();

        let mut barrier_changed = false;
        if let Some(barrier) = self
            .room_playback_barriers
            .get_mut(room_name)
            .filter(|barrier| barrier.phase == PlaybackBarrierPhase::Preparing)
        {
            let previous_participants = barrier.participants.clone();
            let previous_excluded = barrier.excluded_legacy_clients.clone();
            barrier
                .participants
                .retain(|client_id, _| required_clients.contains_key(client_id));
            for (client_id, username) in &required_clients {
                barrier
                    .participants
                    .entry(client_id.clone())
                    .or_insert_with(|| RoomPlaybackBarrierParticipant {
                        username: username.clone(),
                        status: PlaybackBarrierParticipantStatus::pending(),
                    });
            }
            barrier.excluded_legacy_clients = excluded_clients;
            barrier_changed = barrier.participants != previous_participants
                || barrier.excluded_legacy_clients != previous_excluded;
        }

        (changed_usernames, barrier_changed)
    }

    pub(crate) fn set_readiness_pause_owner(
        &mut self,
        room_name: &str,
        owner: RoomPauseOwner,
        publish: bool,
    ) -> Vec<DirectedProtocolMessage> {
        let Some(room) = self.room_readiness.get_mut(room_name) else {
            return Vec::new();
        };
        if room.pause_owner == owner {
            return Vec::new();
        }
        room.revision = room.revision.saturating_add(1);
        room.pause_owner = owner;
        if publish {
            self.readiness_snapshot_fanout(room_name)
        } else {
            Vec::new()
        }
    }

    pub(crate) fn readiness_pause_owned_by_buffering_policy(
        &self,
        room_name: &str,
        media_generation: u64,
        state_revision: Option<u64>,
    ) -> bool {
        self.room_readiness.get(room_name).is_none_or(|room| {
            matches!(
                room.pause_owner,
                RoomPauseOwner::RoomBufferingPolicy {
                    media_generation: owned_generation,
                    state_revision: owned_revision,
                } if owned_generation == media_generation && owned_revision == state_revision
            )
        })
    }

    pub(crate) fn claim_user_pause_ownership(
        &mut self,
        room_name: &str,
        actor: &str,
    ) -> Vec<DirectedProtocolMessage> {
        if let Some(control) = self.room_buffering_controls.get_mut(room_name) {
            control.paused_by_policy = false;
            control.pause_deadline = None;
            control.condition_active_since = None;
            control.condition_clear_since = None;
        }
        let mut outbound = self.retire_committed_playback_barrier_for_user_pause(room_name);
        outbound.extend(self.set_readiness_pause_owner(
            room_name,
            RoomPauseOwner::User {
                actor: actor.to_owned(),
            },
            false,
        ));
        self.refresh_readiness_gate_phase(room_name);
        outbound.extend(self.readiness_snapshot_fanout(room_name));
        outbound
    }

    fn store_pending_user_transport(
        &mut self,
        client_id: &str,
        room_name: &str,
        actor: &str,
        desired_paused: bool,
        evidence: PendingUserTransportEvidence,
    ) {
        let expires_at_seconds =
            self.current_time_seconds() + READINESS_USER_TRANSPORT_GRACE_SECONDS;
        self.pending_user_transport_by_client.insert(
            client_id.to_owned(),
            PendingUserTransportTransition {
                room_name: room_name.to_owned(),
                actor: actor.to_owned(),
                desired_paused,
                evidence,
                expires_at_seconds,
            },
        );
    }

    pub(crate) fn consume_pending_user_transport(
        &mut self,
        client_id: &str,
        room_name: &str,
        actor: &str,
        desired_paused: bool,
        evidence: PendingUserTransportEvidence,
    ) -> bool {
        let now_seconds = self.current_time_seconds();
        self.pending_user_transport_by_client
            .remove(client_id)
            .is_some_and(|pending| {
                pending.room_name == room_name
                    && pending.actor == actor
                    && pending.desired_paused == desired_paused
                    && pending.evidence == evidence
                    && now_seconds <= pending.expires_at_seconds
            })
    }

    pub(crate) fn stage_unclassified_user_transport_observation(
        &mut self,
        client_id: &str,
        room_name: &str,
        actor: &str,
        observed_paused: bool,
    ) {
        self.store_pending_user_transport(
            client_id,
            room_name,
            actor,
            observed_paused,
            PendingUserTransportEvidence::UnclassifiedObservation,
        );
    }

    fn clear_pending_user_transport(&mut self, client_id: &str) {
        self.pending_user_transport_by_client.remove(client_id);
    }

    fn apply_staged_user_transport_transition(
        &mut self,
        client_id: &str,
        room_name: &str,
        actor: &str,
        desired_paused: bool,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let now_seconds = self.current_time_seconds();
        let room_state_before = self.room_playback_state_at(room_name, now_seconds);
        let mut outbound = self.retire_awaiting_playback_barrier_decision(client_id, room_name);
        if desired_paused {
            outbound.extend(self.claim_user_pause_ownership(room_name, actor));
        } else {
            outbound.extend(self.set_readiness_pause_owner(room_name, RoomPauseOwner::None, true));
        }
        if room_state_before.paused == desired_paused {
            return Ok(outbound);
        }

        let watcher_position = self
            .client_playback_states
            .get(client_id)
            .and_then(|state| state.position_at(desired_paused, now_seconds))
            .unwrap_or(room_state_before.position);
        {
            let room_state = self.room_playback_state_mut(room_name);
            room_state.position = watcher_position;
            room_state.paused = desired_paused;
            room_state.updated_at_seconds = now_seconds;
            room_state.set_by = Some(actor.to_owned());
        }
        self.seed_room_client_playback_states(room_name, watcher_position, now_seconds);
        self.persist_room_if_needed(room_name)?;
        let room_state = self.room_playback_state_at(room_name, now_seconds);
        for peer_client in self.clients_in_room(room_name) {
            outbound.push(DirectedProtocolMessage::new(
                peer_client.clone(),
                self.forced_state_sync_message_for_client(
                    &peer_client,
                    room_state.position,
                    room_state.paused,
                    false,
                    Some(actor),
                ),
            ));
        }
        Ok(outbound)
    }

    pub(crate) fn automatic_pause_owner_from_readiness(
        &self,
        room_name: &str,
        username: &str,
    ) -> Option<RoomPauseOwner> {
        let participant = self
            .room_readiness
            .get(room_name)?
            .participants
            .get(username)?;
        if let Some(owner) = &participant.pending_automatic_pause_owner {
            return Some(owner.clone());
        }
        let technical_state = &participant.record.technical_state;
        match technical_state {
            TechnicalPlayability::TemporarilyBlocked {
                cause: TechnicalBlockCause::Recovery,
                ..
            }
            | TechnicalPlayability::TerminallyBlocked {
                cause: TechnicalBlockCause::Recovery,
                ..
            } => Some(RoomPauseOwner::Recovery),
            TechnicalPlayability::TemporarilyBlocked {
                cause: TechnicalBlockCause::EndOfFile,
                ..
            }
            | TechnicalPlayability::TerminallyBlocked {
                cause: TechnicalBlockCause::EndOfFile,
                ..
            } => Some(RoomPauseOwner::EndOfPlaylist),
            _ => None,
        }
    }

    pub(crate) fn automatic_transport_owner_for_observation(
        &self,
        room_name: &str,
        username: &str,
        observed_paused: bool,
    ) -> Option<RoomPauseOwner> {
        let room = self.room_readiness.get(room_name)?;
        if observed_paused {
            return match &room.pause_owner {
                RoomPauseOwner::ReadinessStartGate { .. }
                | RoomPauseOwner::RoomBufferingPolicy { .. }
                | RoomPauseOwner::Recovery
                | RoomPauseOwner::EndOfPlaylist => Some(room.pause_owner.clone()),
                RoomPauseOwner::None => {
                    self.automatic_pause_owner_from_readiness(room_name, username)
                }
                RoomPauseOwner::User { .. } => None,
            };
        }

        (room.pause_owner == RoomPauseOwner::Recovery
            && matches!(
                self.readiness_record(room_name, username)
                    .map(|record| &record.technical_state),
                Some(TechnicalPlayability::Playable { .. })
            ))
        .then_some(RoomPauseOwner::Recovery)
    }

    fn apply_readiness_intent(
        &mut self,
        client_id: &str,
        request: ReadinessIntentRequest,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let Some(session) = self.sessions.get(client_id).cloned() else {
            return Err(ServerRuntimeError::MissingSession(client_id.to_owned()));
        };
        if !self.readiness_enabled || !session.capabilities.readiness_v2 {
            self.clear_pending_user_transport(client_id);
            return Ok(Vec::new());
        }
        let target_username = request
            .target_username
            .clone()
            .unwrap_or_else(|| session.username.clone());
        let target_is_self = target_username == session.username;
        let authorized =
            target_is_self || self.user_can_control_playlist(&session.username, &session.room);
        let current_epoch = self
            .readiness_record(&session.room, &session.username)
            .map(|record| record.membership_epoch);
        let reject = |runtime: &Self, status, epoch| {
            runtime.readiness_request_result_message(
                client_id,
                &request,
                status,
                runtime
                    .room_readiness
                    .get(&session.room)
                    .map(|room| room.revision),
                epoch,
            )
        };
        if !authorized {
            self.clear_pending_user_transport(client_id);
            return Ok(reject(
                self,
                ReadinessRequestResultStatus::RejectedUnauthorized,
                current_epoch,
            ));
        }
        if request.operation_id.is_empty()
            || request.operation_id.len() > READINESS_MAX_OPERATION_ID_BYTES
            || request.request_nonce == 0
            || !readiness_intent_source_matches_desired(&request.source, request.desired)
        {
            self.clear_pending_user_transport(client_id);
            return Ok(reject(
                self,
                ReadinessRequestResultStatus::RejectedInvalid,
                current_epoch,
            ));
        }
        if current_epoch != Some(request.membership_epoch) {
            self.clear_pending_user_transport(client_id);
            return Ok(reject(
                self,
                ReadinessRequestResultStatus::RejectedStaleMembership,
                current_epoch,
            ));
        }

        let operation_key = ReadinessOperationId::new(request.operation_id.clone());
        let existing = self
            .room_readiness
            .get(&session.room)
            .and_then(|room| room.participants.get(&session.username))
            .and_then(|participant| participant.accepted_operations.get(&operation_key))
            .cloned();
        if let Some(existing) = existing {
            let same_operation = existing.membership_epoch == request.membership_epoch
                && existing.desired == request.desired
                && existing.source == request.source
                && existing.target_username == request.target_username;
            if !same_operation {
                self.clear_pending_user_transport(client_id);
                return Ok(reject(
                    self,
                    ReadinessRequestResultStatus::RejectedInvalid,
                    current_epoch,
                ));
            }
            let superseded = self
                .readiness_record(&session.room, &target_username)
                .is_none_or(|record| {
                    record.user_intent_revision != existing.accepted_user_intent_revision
                });
            if superseded {
                self.clear_pending_user_transport(client_id);
            }
            return Ok(reject(
                self,
                if superseded {
                    ReadinessRequestResultStatus::Superseded
                } else {
                    ReadinessRequestResultStatus::Duplicate
                },
                current_epoch,
            ));
        }

        let is_initialization =
            matches!(request.source, UserReadinessMutationSource::Initialization);
        if is_initialization
            && (!target_is_self
                || self
                    .room_readiness
                    .get(&session.room)
                    .and_then(|room| room.participants.get(&session.username))
                    .is_none_or(|participant| {
                        !participant.initialization_open
                            || participant.record.user_intent != UserReadinessIntent::NotReady
                            || participant.record.user_intent_revision != 0
                            || !matches!(
                                participant
                                    .record
                                    .last_user_mutation
                                    .as_ref()
                                    .map(|mutation| &mutation.source),
                                Some(ReadinessMutationSource::Initialization)
                            )
                    }))
        {
            self.clear_pending_user_transport(client_id);
            return Ok(reject(
                self,
                ReadinessRequestResultStatus::RejectedInvalid,
                current_epoch,
            ));
        }

        let (highest_nonce, target_user_intent_revision) = self
            .room_readiness
            .get(&session.room)
            .map(|room| {
                (
                    room.participants
                        .get(&session.username)
                        .map(|participant| participant.highest_request_nonce)
                        .unwrap_or_default(),
                    room.participants
                        .get(&target_username)
                        .map(|participant| participant.record.user_intent_revision),
                )
            })
            .unwrap_or_default();
        if request.request_nonce <= highest_nonce {
            self.clear_pending_user_transport(client_id);
            return Ok(reject(
                self,
                ReadinessRequestResultStatus::RejectedStaleNonce,
                current_epoch,
            ));
        }
        if let Some(actor) = self
            .room_readiness
            .get_mut(&session.room)
            .and_then(|room| room.participants.get_mut(&session.username))
        {
            actor.highest_request_nonce = request.request_nonce;
        }
        let Some(target_user_intent_revision) = target_user_intent_revision else {
            self.clear_pending_user_transport(client_id);
            return Ok(reject(
                self,
                ReadinessRequestResultStatus::RejectedInvalid,
                current_epoch,
            ));
        };
        if request
            .expected_user_intent_revision
            .is_some_and(|expected| expected != target_user_intent_revision)
        {
            self.clear_pending_user_transport(client_id);
            return Ok(reject(
                self,
                ReadinessRequestResultStatus::RejectedRevisionConflict,
                current_epoch,
            ));
        }

        let may_control_room_transport =
            target_is_self && self.user_can_control_playlist(&session.username, &session.room);
        let indirect_player_desired_paused = match &request.source {
            UserReadinessMutationSource::IndirectPlayer {
                action: PlayerReadinessAction::Pause,
                ..
            } => Some(true),
            UserReadinessMutationSource::IndirectPlayer {
                action: PlayerReadinessAction::Play,
                ..
            } => Some(false),
            UserReadinessMutationSource::Initialization
            | UserReadinessMutationSource::DirectUser { .. } => None,
        };
        let staged_observation_matches = indirect_player_desired_paused.is_some_and(|paused| {
            may_control_room_transport
                && self.consume_pending_user_transport(
                    client_id,
                    &session.room,
                    &session.username,
                    paused,
                    PendingUserTransportEvidence::UnclassifiedObservation,
                )
        });
        if !staged_observation_matches {
            self.clear_pending_user_transport(client_id);
        }
        let canonical_source = if target_is_self {
            canonical_user_source(request.source.clone())
        } else {
            ReadinessMutationSource::ControllerOverride {
                actor: session.username.clone(),
            }
        };
        let is_rearming = request.desired == UserReadinessIntent::Ready;
        let Some((room_ready, accepted_revision)) = self.mutate_readiness_intent(
            &session.room,
            &target_username,
            request.desired,
            canonical_source,
            &session.username,
            Some(request.operation_id.as_str()),
        ) else {
            return Ok(reject(
                self,
                ReadinessRequestResultStatus::RejectedInvalid,
                current_epoch,
            ));
        };
        let accepted_user_intent_revision = self
            .readiness_record(&session.room, &target_username)
            .map(|record| record.user_intent_revision)
            .unwrap_or_default();
        if let Some(actor) = self
            .room_readiness
            .get_mut(&session.room)
            .and_then(|room| room.participants.get_mut(&session.username))
        {
            actor.initialization_open = false;
            actor.accepted_operations.insert(
                operation_key,
                AcceptedReadinessOperation {
                    membership_epoch: request.membership_epoch,
                    desired: request.desired,
                    source: request.source.clone(),
                    target_username: request.target_username.clone(),
                    accepted_revision,
                    accepted_user_intent_revision,
                },
            );
            trim_readiness_operations(&mut actor.accepted_operations);
        }

        self.domain
            .set_ready(&target_username, &session.room, room_ready)?;
        let mut transport_outbound = Vec::new();
        let gate_is_preparing = self
            .room_playback_barriers
            .get(&session.room)
            .is_some_and(|barrier| barrier.phase == PlaybackBarrierPhase::Preparing);
        if let Some(desired_paused) = indirect_player_desired_paused
            && may_control_room_transport
        {
            let gate_holds_play = !desired_paused && gate_is_preparing;
            if staged_observation_matches && !gate_holds_play {
                transport_outbound.extend(self.apply_staged_user_transport_transition(
                    client_id,
                    &session.room,
                    &session.username,
                    desired_paused,
                )?);
            } else if self.room_playback_state(&session.room).paused == desired_paused {
                transport_outbound.extend(
                    self.retire_awaiting_playback_barrier_decision(client_id, &session.room),
                );
                if desired_paused {
                    transport_outbound
                        .extend(self.claim_user_pause_ownership(&session.room, &session.username));
                }
            } else if !gate_holds_play {
                self.store_pending_user_transport(
                    client_id,
                    &session.room,
                    &session.username,
                    desired_paused,
                    PendingUserTransportEvidence::AcceptedIndirectAction,
                );
            }
        }
        if is_rearming && gate_is_preparing {
            let media_generation = self.room_playback_barriers[&session.room]
                .prepare
                .media_generation;
            self.set_readiness_pause_owner(
                &session.room,
                RoomPauseOwner::ReadinessStartGate { media_generation },
                false,
            );
        }
        self.refresh_readiness_gate_phase(&session.room);

        let manually_initiated = !is_initialization;
        let mut outbound = self.legacy_readiness_projection_fanout(
            &session.room,
            &target_username,
            room_ready,
            manually_initiated,
            Some(&session.username),
        );
        outbound.extend(transport_outbound);
        outbound.extend(self.readiness_participant_fanout(
            &session.room,
            &target_username,
            Some((client_id, request.operation_id.as_str())),
        ));
        outbound.extend(self.readiness_snapshot_fanout(&session.room));
        outbound.extend(self.readiness_request_result_message(
            client_id,
            &request,
            ReadinessRequestResultStatus::Accepted,
            Some(accepted_revision),
            current_epoch,
        ));
        outbound.extend(self.maybe_commit_readiness_gate(&session.room)?);
        Ok(outbound)
    }

    fn mutate_readiness_intent(
        &mut self,
        room_name: &str,
        target_username: &str,
        desired: UserReadinessIntent,
        source: ReadinessMutationSource,
        actor: &str,
        operation_id: Option<&str>,
    ) -> Option<(bool, u64)> {
        let room = self.room_readiness.get_mut(room_name)?;
        room.revision = room.revision.saturating_add(1);
        let revision = room.revision;
        let participant = room.participants.get_mut(target_username)?;
        participant.record.user_intent = desired;
        participant.record.user_intent_revision =
            participant.record.user_intent_revision.saturating_add(1);
        let mut metadata = ReadinessMutationMetadata::new(source, revision).with_actor(actor);
        if let Some(operation_id) = operation_id {
            metadata = metadata.with_operation_id(operation_id);
        }
        participant.record.last_user_mutation = Some(metadata);
        recompute_participant_readiness(&mut participant.record, room.media_generation);
        Some((participant.record.room_ready, revision))
    }

    fn apply_technical_readiness_report(
        &mut self,
        client_id: &str,
        report: TechnicalReadinessReport,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        let Some(session) = self.sessions.get(client_id).cloned() else {
            return Err(ServerRuntimeError::MissingSession(client_id.to_owned()));
        };
        if !self.technical_readiness_report_is_current(client_id, &report) {
            return Ok(Vec::new());
        }
        let current_generation = self
            .room_readiness
            .get(&session.room)
            .and_then(|room| room.media_generation)
            .expect("validated technical report should have a current generation");
        let canonical_transport_paused = self.room_playback_state(&session.room).paused;
        let report_sequence = report.report_sequence;
        let observed_at = report.observed_at;
        let reported_pause_owner = match report.reason {
            Some(TechnicalBlockCause::EndOfFile) => Some(RoomPauseOwner::EndOfPlaylist),
            Some(TechnicalBlockCause::Recovery) => Some(RoomPauseOwner::Recovery),
            _ => None,
        };

        let technical_state = match report.phase {
            TechnicalPlayabilityPhase::Unknown => TechnicalPlayability::Unknown,
            TechnicalPlayabilityPhase::Preparing => TechnicalPlayability::Preparing {
                media_generation: current_generation,
            },
            TechnicalPlayabilityPhase::Playable => TechnicalPlayability::Playable {
                media_generation: current_generation,
            },
            TechnicalPlayabilityPhase::TemporarilyBlocked => {
                TechnicalPlayability::TemporarilyBlocked {
                    media_generation: current_generation,
                    cause: report.reason.unwrap_or(TechnicalBlockCause::Unknown),
                    recovery: report.recovery.unwrap_or(RecoveryStage::NotStarted),
                }
            }
            TechnicalPlayabilityPhase::TerminallyBlocked => {
                TechnicalPlayability::TerminallyBlocked {
                    media_generation: current_generation,
                    cause: report.reason.unwrap_or(TechnicalBlockCause::Unknown),
                }
            }
        };
        let (before_room_ready, changed, room_ready) = {
            let Some(room) = self.room_readiness.get_mut(&session.room) else {
                return Ok(Vec::new());
            };
            let Some(participant) = room.participants.get_mut(&session.username) else {
                return Ok(Vec::new());
            };
            let before_room_ready = participant.record.room_ready;
            let before_state = participant.record.technical_state.clone();
            let before_block = participant.record.terminal_technical_block.clone();
            let before_sequence = participant.record.last_technical_report_sequence;
            participant.record.technical_state = technical_state;
            participant.record.last_technical_report_sequence = report_sequence;
            if let Some(observed_at) = observed_at {
                participant.last_technical_observed_at = Some(observed_at);
            }
            match reported_pause_owner.clone() {
                Some(owner) => participant.pending_automatic_pause_owner = Some(owner),
                None if matches!(
                    participant.record.technical_state,
                    TechnicalPlayability::Unknown | TechnicalPlayability::Playable { .. }
                ) =>
                {
                    participant.pending_automatic_pause_owner = None;
                }
                None => {}
            }
            match &participant.record.technical_state {
                TechnicalPlayability::TerminallyBlocked { cause, .. } => {
                    participant.record.terminal_technical_block =
                        Some(TechnicalReadinessBlock::new(current_generation, *cause));
                }
                TechnicalPlayability::Playable { .. } => {
                    participant.record.terminal_technical_block = None;
                }
                TechnicalPlayability::Unknown
                | TechnicalPlayability::Preparing { .. }
                | TechnicalPlayability::TemporarilyBlocked { .. } => {}
            }
            recompute_participant_readiness(&mut participant.record, Some(current_generation));
            let changed = before_state != participant.record.technical_state
                || before_block != participant.record.terminal_technical_block
                || before_sequence != participant.record.last_technical_report_sequence;
            if changed {
                room.revision = room.revision.saturating_add(1);
            }
            (before_room_ready, changed, participant.record.room_ready)
        };
        if changed {
            self.domain
                .set_ready(&session.username, &session.room, room_ready)?;
        }
        let current_pause_owner = self
            .room_readiness
            .get(&session.room)
            .map(|room| room.pause_owner.clone())
            .unwrap_or_default();
        let mut pause_owner_changed = false;
        if !canonical_transport_paused && current_pause_owner == RoomPauseOwner::Recovery {
            self.set_readiness_pause_owner(&session.room, RoomPauseOwner::None, false);
            pause_owner_changed = true;
        } else if canonical_transport_paused
            && let Some(owner) = reported_pause_owner
            && (matches!(current_pause_owner, RoomPauseOwner::None) || current_pause_owner == owner)
        {
            pause_owner_changed = current_pause_owner != owner;
            self.set_readiness_pause_owner(&session.room, owner, false);
        }
        if !changed && !pause_owner_changed {
            return Ok(Vec::new());
        }
        self.refresh_readiness_gate_phase(&session.room);
        let mut outbound = Vec::new();
        if changed && before_room_ready != room_ready {
            outbound.extend(self.legacy_readiness_projection_fanout(
                &session.room,
                &session.username,
                room_ready,
                false,
                None,
            ));
        }
        if changed {
            outbound.extend(self.readiness_participant_fanout(
                &session.room,
                &session.username,
                None,
            ));
        }
        outbound.extend(self.readiness_snapshot_fanout(&session.room));
        // Target-specific barrier evidence may arrive before the participant's
        // generic V2 eligibility. Re-evaluate the gate when that final
        // technical transition lands, while still requiring the independent
        // MediaReady participant phase in playback_barrier_policy_satisfied().
        outbound.extend(self.maybe_commit_readiness_gate(&session.room)?);
        Ok(outbound)
    }

    fn technical_readiness_report_is_current(
        &self,
        client_id: &str,
        report: &TechnicalReadinessReport,
    ) -> bool {
        let Some(session) = self.sessions.get(client_id) else {
            return false;
        };
        if !self.readiness_enabled
            || !session.capabilities.readiness_v2
            || report.membership_epoch == 0
            || report.report_sequence == 0
        {
            return false;
        }
        let Some(participant) = self
            .room_readiness
            .get(&session.room)
            .filter(|room| room.media_generation == Some(report.media_generation))
            .and_then(|room| room.participants.get(&session.username))
        else {
            return false;
        };
        if participant.client_id != client_id
            || participant.record.membership_epoch != report.membership_epoch
            || report.report_sequence <= participant.record.last_technical_report_sequence
            || report.observed_at.is_some_and(|observed_at| {
                !observed_at.is_finite()
                    || participant
                        .last_technical_observed_at
                        .is_some_and(|last_observed_at| observed_at < last_observed_at)
            })
        {
            return false;
        }
        report.authoritative_playback_revision
            == self.authoritative_playback_revision(&session.room, report.reason)
    }

    fn authoritative_playback_revision(
        &self,
        room_name: &str,
        reason: Option<TechnicalBlockCause>,
    ) -> Option<u64> {
        if reason == Some(TechnicalBlockCause::RoomBufferingPolicy)
            && let Some(state_revision) = self
                .room_buffering_controls
                .get(room_name)
                .filter(|control| {
                    self.room_readiness
                        .get(room_name)
                        .and_then(|room| room.media_generation)
                        == Some(control.config.media_generation)
                })
                .and_then(|control| control.config.state_revision)
        {
            return Some(state_revision);
        }
        self.room_playback_barriers
            .get(room_name)
            .and_then(|barrier| barrier.commit.as_ref().and(barrier.state_revision))
    }

    fn maybe_commit_readiness_gate(
        &mut self,
        room_name: &str,
    ) -> Result<Vec<DirectedProtocolMessage>, ServerRuntimeError> {
        if self
            .room_playback_barriers
            .get(room_name)
            .is_some_and(|barrier| barrier.phase == PlaybackBarrierPhase::Preparing)
            && self.playback_barrier_policy_satisfied(room_name)
        {
            return self.commit_playback_barrier(room_name, false, self.current_time_seconds());
        }
        Ok(Vec::new())
    }

    fn refresh_readiness_gate_phase(&mut self, room_name: &str) {
        let Some((
            media_generation,
            readiness_revision,
            pause_owner,
            participants,
            current_start_gate_phase,
        )) = self.room_readiness.get(room_name).and_then(|room| {
            room.media_generation.map(|media_generation| {
                (
                    media_generation,
                    room.revision,
                    room.pause_owner.clone(),
                    room.participants
                        .values()
                        .map(|participant| participant.record.clone())
                        .collect::<Vec<_>>(),
                    room.start_gate_phase.clone(),
                )
            })
        })
        else {
            return;
        };
        let barrier = self.room_playback_barriers.get(room_name);
        let barrier_phase = barrier.map(|barrier| barrier.phase);
        let barrier_has_committed = barrier.is_some_and(|barrier| barrier.commit.is_some());
        let required: Vec<_> = participants
            .iter()
            .filter(|participant| {
                participant.participation_role == StartParticipationRole::Required
            })
            .collect();
        let phase = match barrier_phase {
            Some(PlaybackBarrierPhase::Preparing) => {
                if self.readiness_mixed_room_blocks_start(room_name) {
                    RoomStartGatePhase::Degraded {
                        media_generation,
                        reason: StartGateDegradedReason::IncompatibleLegacyParticipant,
                    }
                } else if required.is_empty() {
                    RoomStartGatePhase::Degraded {
                        media_generation,
                        reason: StartGateDegradedReason::NoRequiredParticipants,
                    }
                } else if matches!(&pause_owner, RoomPauseOwner::User { .. }) {
                    RoomStartGatePhase::Degraded {
                        media_generation,
                        reason: StartGateDegradedReason::UserPaused,
                    }
                } else if !matches!(
                    &pause_owner,
                    RoomPauseOwner::ReadinessStartGate {
                        media_generation: owned_generation,
                    } if *owned_generation == media_generation
                ) {
                    RoomStartGatePhase::Degraded {
                        media_generation,
                        reason: StartGateDegradedReason::PauseOwnershipLost,
                    }
                } else if required.iter().any(|participant| {
                    matches!(
                        participant.technical_state,
                        TechnicalPlayability::TerminallyBlocked { .. }
                    )
                }) {
                    RoomStartGatePhase::Degraded {
                        media_generation,
                        reason: StartGateDegradedReason::TechnicalFailure,
                    }
                } else if required.iter().any(|participant| !participant.room_ready) {
                    RoomStartGatePhase::WaitingForIntent { media_generation }
                } else if required
                    .iter()
                    .any(|participant| !participant.start_eligible)
                {
                    RoomStartGatePhase::WaitingForTechnicalReadiness { media_generation }
                } else {
                    RoomStartGatePhase::ReadyToCommit {
                        media_generation,
                        readiness_revision,
                    }
                }
            }
            Some(PlaybackBarrierPhase::Committed) => self
                .room_playback_barriers
                .get(room_name)
                .and_then(|barrier| {
                    Some(RoomStartGatePhase::Committed {
                        media_generation,
                        readiness_revision: barrier.readiness_revision?,
                        playback_revision: barrier.state_revision?,
                    })
                })
                .unwrap_or(RoomStartGatePhase::Inactive),
            Some(PlaybackBarrierPhase::AwaitingDecision | PlaybackBarrierPhase::Degraded)
                if !barrier_has_committed =>
            {
                match current_start_gate_phase {
                    RoomStartGatePhase::Degraded {
                        media_generation: degraded_generation,
                        reason,
                    } if degraded_generation == media_generation => RoomStartGatePhase::Degraded {
                        media_generation,
                        reason,
                    },
                    _ => RoomStartGatePhase::Degraded {
                        media_generation,
                        reason: StartGateDegradedReason::Cancelled,
                    },
                }
            }
            Some(
                PlaybackBarrierPhase::AwaitingDecision
                | PlaybackBarrierPhase::Complete
                | PlaybackBarrierPhase::Degraded,
            )
            | None => RoomStartGatePhase::Inactive,
        };
        if let Some(room) = self.room_readiness.get_mut(room_name) {
            room.start_gate_phase = phase;
        }
    }

    fn readiness_record(&self, room_name: &str, username: &str) -> Option<&ParticipantReadiness> {
        self.room_readiness
            .get(room_name)?
            .participants
            .get(username)
            .map(|participant| &participant.record)
    }

    fn readiness_participant_update(
        &self,
        room_name: &str,
        username: &str,
        accepted_operation_id: Option<&str>,
    ) -> Option<ParticipantReadinessUpdate> {
        let room = self.room_readiness.get(room_name)?;
        let record = &room.participants.get(username)?.record;
        let source = record
            .last_user_mutation
            .as_ref()
            .map(|mutation| mutation.source.clone())
            .unwrap_or(ReadinessMutationSource::Initialization);
        Some(ParticipantReadinessUpdate {
            room_readiness_revision: room.revision,
            membership_epoch: record.membership_epoch,
            last_technical_report_sequence: record.last_technical_report_sequence,
            username: username.to_owned(),
            user_intent: record.user_intent,
            user_intent_revision: record.user_intent_revision,
            user_intent_source: source,
            last_user_mutation: record.last_user_mutation.clone(),
            terminal_technical_block: record.terminal_technical_block.clone(),
            technical_state: record.technical_state.summary(),
            participation_role: record.participation_role,
            room_ready: record.room_ready,
            start_eligible: record.start_eligible,
            accepted_operation_id: accepted_operation_id.map(str::to_owned),
        })
    }

    fn readiness_snapshot(&self, room_name: &str) -> Option<RoomReadinessSnapshot> {
        let room = self.room_readiness.get(room_name)?;
        let participants = room
            .participants
            .keys()
            .filter_map(|username| {
                self.readiness_participant_update(room_name, username, None)
                    .map(|update| (username.clone(), update))
            })
            .collect();
        Some(RoomReadinessSnapshot {
            room_readiness_revision: room.revision,
            media_generation: room.media_generation,
            start_gate_phase: room.start_gate_phase.clone(),
            pause_owner: room.pause_owner.clone(),
            mixed_readiness_policy: self.mixed_readiness_policy,
            participants,
        })
    }

    fn readiness_participant_fanout(
        &self,
        room_name: &str,
        username: &str,
        accepted_operation: Option<(&str, &str)>,
    ) -> Vec<DirectedProtocolMessage> {
        self.readiness_v2_clients_in_room(room_name)
            .into_iter()
            .filter_map(|recipient| {
                let operation_id = accepted_operation
                    .filter(|(client_id, _)| *client_id == recipient)
                    .map(|(_, operation_id)| operation_id);
                let participant =
                    self.readiness_participant_update(room_name, username, operation_id)?;
                Some(DirectedProtocolMessage::new(
                    recipient,
                    readiness_set_message(
                        ReadinessSetExtension::new().with_participant(participant),
                    ),
                ))
            })
            .collect()
    }

    fn readiness_snapshot_fanout(&self, room_name: &str) -> Vec<DirectedProtocolMessage> {
        let Some(snapshot) = self.readiness_snapshot(room_name) else {
            return Vec::new();
        };
        self.readiness_v2_clients_in_room(room_name)
            .into_iter()
            .map(|client_id| {
                DirectedProtocolMessage::new(
                    client_id,
                    readiness_set_message(
                        ReadinessSetExtension::new().with_snapshot(snapshot.clone()),
                    ),
                )
            })
            .collect()
    }

    fn legacy_readiness_projection_fanout(
        &self,
        room_name: &str,
        username: &str,
        room_ready: bool,
        manually_initiated: bool,
        set_by: Option<&str>,
    ) -> Vec<DirectedProtocolMessage> {
        let message = ready_update_message(
            username,
            self.readiness_enabled.then_some(room_ready),
            manually_initiated,
            set_by,
        );
        self.clients_in_room(room_name)
            .into_iter()
            .map(|client_id| DirectedProtocolMessage::new(client_id, message.clone()))
            .collect()
    }

    fn readiness_request_result_message(
        &self,
        client_id: &str,
        request: &ReadinessIntentRequest,
        status: ReadinessRequestResultStatus,
        room_readiness_revision: Option<u64>,
        membership_epoch: Option<u64>,
    ) -> Vec<DirectedProtocolMessage> {
        let mut result = ReadinessRequestResultPayload::new(
            request.operation_id.clone(),
            request.request_nonce,
            status,
        );
        if let Some(revision) = room_readiness_revision {
            result = result.with_room_readiness_revision(revision);
        }
        if let Some(epoch) = membership_epoch {
            result = result.with_membership_epoch(epoch);
        }
        if let Some(session) = self.sessions.get(client_id) {
            let target_username = request
                .target_username
                .as_deref()
                .unwrap_or(session.username.as_str());
            if let Some(user_intent_revision) = self
                .readiness_record(&session.room, target_username)
                .map(|record| record.user_intent_revision)
            {
                result = result.with_user_intent_revision(user_intent_revision);
            }
        }
        vec![DirectedProtocolMessage::new(
            client_id,
            readiness_set_message(ReadinessSetExtension::new().with_request_result(result)),
        )]
    }

    fn readiness_v2_clients_in_room(&self, room_name: &str) -> Vec<String> {
        self.sessions
            .iter()
            .filter(|(client_id, session)| {
                session.room == room_name
                    && self.readiness_enabled
                    && session.capabilities.readiness_v2
                    && !self.playback_barrier_fenced_clients.contains(*client_id)
            })
            .map(|(client_id, _)| client_id.clone())
            .collect()
    }

    fn allocate_readiness_membership_epoch(&mut self) -> u64 {
        let epoch = self.next_readiness_membership_epoch;
        self.next_readiness_membership_epoch =
            self.next_readiness_membership_epoch.saturating_add(1);
        epoch
    }

    fn prune_readiness_reconnect_cache(&mut self) {
        let now_seconds = self.current_time_seconds();
        self.readiness_reconnect_cache.retain(|_, membership| {
            now_seconds - membership.detached_at_seconds <= READINESS_RECONNECT_TTL_SECONDS
        });
    }
}

fn canonical_user_source(source: UserReadinessMutationSource) -> ReadinessMutationSource {
    match source {
        UserReadinessMutationSource::Initialization => ReadinessMutationSource::Initialization,
        UserReadinessMutationSource::DirectUser { surface } => {
            ReadinessMutationSource::DirectUser { surface }
        }
        UserReadinessMutationSource::IndirectPlayer { action, surface } => {
            ReadinessMutationSource::IndirectPlayer { action, surface }
        }
    }
}

fn readiness_intent_source_matches_desired(
    source: &UserReadinessMutationSource,
    desired: UserReadinessIntent,
) -> bool {
    match source {
        UserReadinessMutationSource::Initialization => desired == UserReadinessIntent::Ready,
        UserReadinessMutationSource::DirectUser { .. } => true,
        UserReadinessMutationSource::IndirectPlayer {
            action: PlayerReadinessAction::Play,
            ..
        } => desired == UserReadinessIntent::Ready,
        UserReadinessMutationSource::IndirectPlayer {
            action: PlayerReadinessAction::Pause,
            ..
        } => desired == UserReadinessIntent::NotReady,
    }
}

fn recompute_participant_readiness(
    record: &mut ParticipantReadiness,
    current_generation: Option<u64>,
) {
    record.room_ready = record.user_intent == UserReadinessIntent::Ready
        && record.terminal_technical_block.is_none();
    record.start_eligible = record.participation_role == StartParticipationRole::Required
        && record.room_ready
        && matches!(
            record.technical_state,
            TechnicalPlayability::Playable { media_generation }
                if Some(media_generation) == current_generation
        );
}

fn trim_readiness_operations(
    operations: &mut BTreeMap<ReadinessOperationId, AcceptedReadinessOperation>,
) {
    while operations.len() > READINESS_MAX_RETAINED_OPERATIONS_PER_MEMBERSHIP {
        let Some(oldest_key) = operations
            .iter()
            .min_by_key(|(_, operation)| operation.accepted_revision)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        operations.remove(&oldest_key);
    }
}

fn generate_readiness_reconnect_token() -> SecretValue {
    let mut bytes = [0_u8; 32];
    getrandom::getrandom(&mut bytes)
        .expect("operating system random source should issue reconnect identities");
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    SecretValue::from(encoded)
}

fn readiness_reconnect_token_digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}
