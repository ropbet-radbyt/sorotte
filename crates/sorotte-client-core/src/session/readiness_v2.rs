use super::*;

impl ClientSession {
    pub fn advertise_readiness_v2(features: &mut Map<String, Value>) {
        features.insert(SOROTTE_READINESS_V2.to_owned(), Value::Bool(true));
    }

    pub fn readiness_snapshot(&self) -> Option<&RoomReadinessSnapshot> {
        self.model.readiness.canonical_snapshot.as_ref()
    }

    pub fn pending_readiness_intent(&self) -> Option<&PendingReadinessIntent> {
        self.model.readiness.pending_intent.as_ref()
    }

    pub fn canonical_participant_readiness(
        &self,
        username: &str,
    ) -> Option<&ParticipantReadinessUpdate> {
        self.model
            .readiness
            .canonical_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.participants.get(username))
    }

    pub fn displayed_user_readiness_intent(&self, username: &str) -> Option<UserReadinessIntent> {
        if let Some(pending) = self.model.readiness.pending_intent.as_ref() {
            let pending_username = pending.target_username.as_deref().or(self
                .model
                .connection
                .username
                .as_deref());
            if pending_username == Some(username) {
                return Some(pending.desired);
            }
        }
        self.canonical_participant_readiness(username)
            .map(|participant| participant.user_intent)
            .or_else(|| {
                self.user_ready(username).map(|ready| {
                    if ready {
                        UserReadinessIntent::Ready
                    } else {
                        UserReadinessIntent::NotReady
                    }
                })
            })
    }

    pub(crate) fn reset_readiness_v2_for_new_room(&mut self) {
        self.model.readiness.canonical_snapshot = None;
        self.model.readiness.canonical_room = None;
        self.model
            .readiness
            .awaiting_readiness_reconciliation_snapshot = false;
        self.model.readiness.pending_intent = None;
    }

    pub(crate) fn mark_readiness_v2_reconnect_pending(&mut self) {
        self.model
            .readiness
            .awaiting_readiness_reconciliation_snapshot = true;
        if let Some(pending) = self.model.readiness.pending_intent.as_mut() {
            pending.needs_send = true;
        }
    }

    pub(crate) fn mark_pending_readiness_delivery_failed(&mut self) {
        if let Some(pending) = self.model.readiness.pending_intent.as_mut() {
            pending.needs_send = true;
        }
    }

    pub fn runtime_actions_for_indirect_player_intent(
        &mut self,
        paused: bool,
        surface: PlayerInteractionSurface,
    ) -> Vec<ClientRuntimeAction> {
        if !self.server_readiness_supported() {
            return Vec::new();
        }
        let desired = if paused {
            UserReadinessIntent::NotReady
        } else {
            UserReadinessIntent::Ready
        };
        if !self.server_readiness_v2_supported() {
            let ready = desired == UserReadinessIntent::Ready;
            if self
                .model
                .connection
                .username
                .as_deref()
                .and_then(|username| self.model.room.users.get(username))
                .and_then(|user| user.ready)
                == Some(ready)
            {
                return Vec::new();
            }
            self.apply_local_ready_state_optimistically(ready);
            return vec![ClientRuntimeAction::SetReady {
                ready,
                manually_initiated: true,
            }];
        }
        self.runtime_actions_for_readiness_intent(
            desired,
            UserReadinessMutationSource::IndirectPlayer {
                action: if paused {
                    sorotte_protocol::PlayerReadinessAction::Pause
                } else {
                    sorotte_protocol::PlayerReadinessAction::Play
                },
                surface,
            },
            None,
        )
    }

    pub fn runtime_actions_for_direct_readiness_intent(
        &mut self,
        desired: UserReadinessIntent,
        surface: sorotte_protocol::DirectReadinessSurface,
        target_username: Option<String>,
    ) -> Vec<ClientRuntimeAction> {
        self.runtime_actions_for_readiness_intent(
            desired,
            UserReadinessMutationSource::DirectUser { surface },
            target_username,
        )
    }

    /// Applies an explicit room-entry readiness policy. V2 preserves the
    /// initialization provenance; legacy peers receive the established
    /// non-manual Ready payload.
    pub fn runtime_actions_for_initial_readiness_intent(
        &mut self,
        desired: UserReadinessIntent,
    ) -> Vec<ClientRuntimeAction> {
        if self.model.connection.username.is_none() || !self.server_readiness_supported() {
            return Vec::new();
        }
        if !self.server_readiness_v2_supported() {
            return vec![ClientRuntimeAction::SetReady {
                ready: desired == UserReadinessIntent::Ready,
                manually_initiated: false,
            }];
        }
        if desired == UserReadinessIntent::NotReady {
            // Not Ready is the canonical V2 join state. Initialization is a
            // one-shot promotion to Ready, so transmitting the no-op value
            // would create a request the server is required to reject.
            return Vec::new();
        }
        self.runtime_actions_for_readiness_intent(
            desired,
            UserReadinessMutationSource::Initialization,
            None,
        )
    }

    fn runtime_actions_for_readiness_intent(
        &mut self,
        desired: UserReadinessIntent,
        source: UserReadinessMutationSource,
        target_username: Option<String>,
    ) -> Vec<ClientRuntimeAction> {
        if !self.server_readiness_v2_supported() {
            return Vec::new();
        }
        let Some(local_username) = self.model.connection.username.clone() else {
            return Vec::new();
        };
        let Some(room) = self.model.room.name.clone() else {
            return Vec::new();
        };
        if target_username.as_deref().is_some_and(str::is_empty) {
            return Vec::new();
        }
        if self
            .model
            .readiness
            .pending_intent
            .as_ref()
            .is_some_and(|pending| {
                pending.room == room
                    && pending.desired == desired
                    && pending.source == source
                    && pending.target_username == target_username
            })
        {
            return Vec::new();
        }

        let membership_epoch = self
            .canonical_participant_readiness(&local_username)
            .map_or(0, |participant| participant.membership_epoch);
        self.model.readiness.next_request_nonce = self
            .model
            .readiness
            .next_request_nonce
            .wrapping_add(1)
            .max(1);
        let request_nonce = self.model.readiness.next_request_nonce;
        let expected_user_intent_revision = self
            .canonical_participant_readiness(
                target_username
                    .as_deref()
                    .unwrap_or(local_username.as_str()),
            )
            .map(|participant| participant.user_intent_revision);
        let operation_id = new_readiness_operation_id(request_nonce);

        self.model.readiness.pending_intent = Some(PendingReadinessIntent {
            room,
            operation_id,
            request_nonce,
            membership_epoch,
            desired,
            source,
            target_username,
            expected_user_intent_revision,
            scope_from_rejection_result: false,
            needs_send: true,
        });
        self.pending_readiness_reconciliation_action()
            .into_iter()
            .collect()
    }

    pub(crate) fn pending_readiness_reconciliation_action(
        &mut self,
    ) -> Option<ClientRuntimeAction> {
        if !self.server_readiness_v2_supported() {
            return None;
        }
        let room = self.model.room.name.clone()?;
        let local_username = self.model.connection.username.clone()?;
        let membership_epoch = self
            .canonical_participant_readiness(&local_username)?
            .membership_epoch;
        if membership_epoch == 0 {
            return None;
        }
        let pending_target = self
            .model
            .readiness
            .pending_intent
            .as_ref()
            .and_then(|pending| pending.target_username.clone())
            .unwrap_or_else(|| local_username.clone());
        let canonical_user_intent_revision = self
            .canonical_participant_readiness(&pending_target)
            .map(|participant| participant.user_intent_revision);
        let pending = self.model.readiness.pending_intent.as_mut()?;
        if pending.room != room || !pending.needs_send {
            return None;
        }
        // A rejection result can be newer than the last canonical snapshot. Keep
        // the scope supplied by that result until the matching snapshot arrives
        // (or a strictly newer revision supersedes it); otherwise the immediate
        // retry would repeat the stale revision and membership epoch that the
        // server just rejected. Outside that narrow response/snapshot race, a
        // reconnect baseline may legitimately move either revision or epoch in
        // either direction after a server restart.
        let canonical_scope_is_current = !pending.scope_from_rejection_result
            || match (
                canonical_user_intent_revision,
                pending.expected_user_intent_revision,
            ) {
                (Some(canonical), Some(pending_revision)) if canonical > pending_revision => true,
                (Some(canonical), Some(pending_revision)) if canonical == pending_revision => {
                    membership_epoch == pending.membership_epoch
                }
                (Some(_), None) => membership_epoch == pending.membership_epoch,
                _ => false,
            };
        if canonical_scope_is_current && pending.membership_epoch != membership_epoch {
            pending.membership_epoch = membership_epoch;
            self.model.readiness.next_request_nonce = self
                .model
                .readiness
                .next_request_nonce
                .wrapping_add(1)
                .max(1);
            pending.request_nonce = self.model.readiness.next_request_nonce;
        }
        if canonical_scope_is_current {
            pending.expected_user_intent_revision = canonical_user_intent_revision;
            pending.scope_from_rejection_result = false;
        }
        let mut request = ReadinessIntentRequest::new(
            pending.operation_id.clone(),
            pending.request_nonce,
            pending.membership_epoch,
            pending.desired,
            pending.source.clone(),
        );
        if let Some(target_username) = pending.target_username.clone() {
            request = request.with_target_username(target_username);
        }
        if let Some(expected_user_intent_revision) = pending.expected_user_intent_revision {
            request = request.with_expected_user_intent_revision(expected_user_intent_revision);
        }
        let request_membership_epoch = pending.membership_epoch;
        pending.needs_send = false;
        Some(ClientRuntimeAction::SetReadinessIntent {
            request: Box::new(request),
            scope: ReadinessIntentScope::new(room, request_membership_epoch),
        })
    }

    pub fn runtime_action_for_technical_readiness(
        &self,
        report: TechnicalReadinessReport,
    ) -> Option<ClientRuntimeAction> {
        (self.server_readiness_v2_supported()
            && report.media_generation != 0
            && report.membership_epoch != 0
            && report.report_sequence != 0)
            .then_some(ClientRuntimeAction::ReportTechnicalReadiness(report))
    }

    pub(super) fn apply_readiness_v2_extension(&mut self, extension: ReadinessSetExtension) {
        let current_revision = self
            .model
            .readiness
            .canonical_snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.room_readiness_revision);

        if let Some(snapshot) = extension.snapshot {
            let accepts_reconciliation_baseline = self
                .model
                .readiness
                .awaiting_readiness_reconciliation_snapshot;
            if accepts_reconciliation_baseline
                || snapshot.room_readiness_revision >= current_revision
            {
                self.model.readiness.canonical_room = self.model.room.name.clone();
                self.model.readiness.canonical_snapshot = Some(snapshot);
                self.model
                    .readiness
                    .awaiting_readiness_reconciliation_snapshot = false;
            }
        }

        if let Some(participant) = extension.participant {
            let snapshot = self
                .model
                .readiness
                .canonical_snapshot
                .get_or_insert_with(|| RoomReadinessSnapshot {
                    room_readiness_revision: participant.room_readiness_revision,
                    media_generation: participant.technical_state.media_generation,
                    start_gate_phase: sorotte_protocol::RoomStartGatePhase::Inactive,
                    pause_owner: sorotte_protocol::RoomPauseOwner::None,
                    mixed_readiness_policy: Default::default(),
                    participants: BTreeMap::new(),
                });
            if participant.room_readiness_revision >= snapshot.room_readiness_revision {
                snapshot.room_readiness_revision = participant.room_readiness_revision;
                snapshot
                    .participants
                    .insert(participant.username.clone(), participant);
                self.model.readiness.canonical_room = self.model.room.name.clone();
            }
        }

        let accepted_operation = self
            .model
            .readiness
            .canonical_snapshot
            .as_ref()
            .and_then(|snapshot| {
                snapshot
                    .participants
                    .values()
                    .find_map(|participant| participant.accepted_operation_id.as_deref())
            })
            .map(str::to_owned);
        let pending_operation = self
            .model
            .readiness
            .pending_intent
            .as_ref()
            .map(|pending| pending.operation_id.clone());
        if accepted_operation.as_deref() == pending_operation.as_deref()
            && pending_operation.is_some()
        {
            self.model.readiness.pending_intent = None;
        }

        if let Some(result) = extension.request_result
            && self
                .model
                .readiness
                .pending_intent
                .as_ref()
                .is_some_and(|pending| pending.operation_id == result.operation_id)
        {
            match result.status {
                ReadinessRequestResultStatus::Accepted
                | ReadinessRequestResultStatus::Duplicate
                | ReadinessRequestResultStatus::Superseded
                | ReadinessRequestResultStatus::RejectedUnauthorized
                | ReadinessRequestResultStatus::RejectedInvalid => {
                    self.model.readiness.pending_intent = None;
                }
                ReadinessRequestResultStatus::RejectedStaleMembership
                | ReadinessRequestResultStatus::RejectedStaleNonce
                | ReadinessRequestResultStatus::RejectedRevisionConflict => {
                    self.model.readiness.next_request_nonce = self
                        .model
                        .readiness
                        .next_request_nonce
                        .wrapping_add(1)
                        .max(1);
                    let target_username = self
                        .model
                        .readiness
                        .pending_intent
                        .as_ref()
                        .and_then(|pending| pending.target_username.as_deref())
                        .or(self.model.connection.username.as_deref());
                    let retry_user_intent_revision = result.user_intent_revision.or_else(|| {
                        target_username.and_then(|target_username| {
                            self.model
                                .readiness
                                .canonical_snapshot
                                .as_ref()
                                .and_then(|snapshot| snapshot.participants.get(target_username))
                                .map(|participant| participant.user_intent_revision)
                        })
                    });
                    if let Some(pending) = self.model.readiness.pending_intent.as_mut() {
                        pending.request_nonce = self.model.readiness.next_request_nonce;
                        if let Some(membership_epoch) = result.membership_epoch {
                            pending.membership_epoch = membership_epoch;
                        }
                        pending.expected_user_intent_revision = retry_user_intent_revision;
                        pending.scope_from_rejection_result = result.user_intent_revision.is_some()
                            || result.membership_epoch.is_some();
                        pending.needs_send = true;
                    }
                }
            }
        }

        let canonical_projection = self
            .model
            .readiness
            .canonical_snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .participants
                    .values()
                    .map(|participant| (participant.username.clone(), participant.room_ready))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for (username, room_ready) in canonical_projection {
            self.set_user_ready_state(&username, Some(room_ready));
        }
    }
}

fn new_readiness_operation_id(request_nonce: u64) -> String {
    use std::fmt::Write as _;

    let mut random = [0_u8; 16];
    if getrandom::getrandom(&mut random).is_err() {
        random[..8].copy_from_slice(&request_nonce.to_le_bytes());
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        random[8..].copy_from_slice(&timestamp.to_le_bytes());
    }
    let mut operation_id = String::with_capacity(45);
    operation_id.push_str("readiness-v2-");
    for byte in random {
        write!(&mut operation_id, "{byte:02x}")
            .expect("writing readiness operation ID into String cannot fail");
    }
    operation_id
}
