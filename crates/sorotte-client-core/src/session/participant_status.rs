use super::*;
use crate::model::ParticipantStatusReceipt;

impl ClientSession {
    /// Adds the additive participant-status capability to an existing Hello feature map.
    pub fn advertise_participant_status_v1(features: &mut Map<String, Value>) {
        features.insert(SOROTTE_PARTICIPANT_STATUS_V1.to_owned(), Value::Bool(true));
    }

    pub fn server_participant_status_v1_supported(&self) -> bool {
        self.is_active() && self.model.connection.participant_status_v1
    }

    pub fn user_participant_status_v1_supported(&self, username: &str) -> Option<bool> {
        self.model
            .room
            .participant_status_capabilities
            .get(username)
            .copied()
    }

    /// Returns a status view whose age includes time elapsed since this client
    /// received the server snapshot.
    pub fn user_participant_status_at(
        &self,
        username: &str,
        now_seconds: f64,
    ) -> Option<ClientParticipantStatusView> {
        self.model.room.users.get(username)?;
        let mut status = self.model.room.participant_statuses.get(username)?.clone();
        let Some(receipt) = self.model.room.participant_status_receipts.get(username) else {
            return Some(status.fail_closed_stale());
        };
        if receipt.clock_invalidated.load(AtomicOrdering::Acquire) {
            return Some(status.fail_closed_stale());
        }
        let received_at = receipt.received_at_seconds;
        let elapsed_seconds = now_seconds - received_at;
        let invalid_clock = !elapsed_seconds.is_finite()
            || elapsed_seconds < 0.0
            || status
                .report_age_seconds
                .is_some_and(|age| !(age + elapsed_seconds).is_finite());
        if invalid_clock {
            // A wall-clock rollback or arithmetic overflow is a one-way
            // evidence fence for this snapshot. Do not let precise fields
            // reappear merely because the wall clock later catches up.
            receipt
                .clock_invalidated
                .store(true, AtomicOrdering::Release);
            return Some(status.fail_closed_stale());
        }
        if self.model.room.participant_status_snapshot_mode == ParticipantStatusSnapshotMode::Full
            && let Some(authoritative_scope) =
                self.model.room.participant_status_authoritative_scope
            && status.status.correlation == Some(ParticipantStatusCorrelation::Exact)
            && status.status.playback_scope != Some(authoritative_scope)
        {
            status.redact_precise_scope_evidence();
        }
        Some(status.aged_by(elapsed_seconds))
    }

    pub(super) fn clear_participant_status_views(&mut self) {
        self.invalidate_participant_status_evidence();
        self.model.room.participant_status_snapshot_revision = None;
    }

    pub(super) fn invalidate_participant_status_evidence(&mut self) {
        self.model.room.participant_statuses.clear();
        self.model.room.participant_status_receipts.clear();
        self.model.room.participant_status_snapshot_mode = ParticipantStatusSnapshotMode::Full;
        self.model.room.participant_status_authoritative_scope = None;
    }

    pub fn participant_status_authoritative_scope(&self) -> Option<ParticipantPlaybackScope> {
        self.model.room.participant_status_authoritative_scope
    }

    fn participant_status_scope_is_older(
        candidate: ParticipantPlaybackScope,
        current: ParticipantPlaybackScope,
    ) -> bool {
        use std::cmp::Ordering;

        match (candidate.transport_revision, current.transport_revision) {
            (Some(candidate_revision), Some(current_revision)) => {
                match candidate_revision.cmp(&current_revision) {
                    Ordering::Less => return true,
                    Ordering::Greater => return false,
                    Ordering::Equal => {}
                }
            }
            // Once the server has supplied a monotonic transport fence, an
            // unfenced scope cannot supersede it. Conversely, introduction
            // of that fence is authoritative even if an independent media
            // generation allocator restarts at a lower number.
            (None, Some(_)) => return true,
            (Some(_), None) => return false,
            (None, None) => {}
        }
        match candidate.media_generation.cmp(&current.media_generation) {
            Ordering::Less => return true,
            Ordering::Greater => return false,
            Ordering::Equal => {}
        }
        candidate.state_revision < current.state_revision
    }

    fn participant_status_inbound_is_enabled(&self) -> bool {
        self.is_active()
            && self.server_participant_status_v1_supported()
            && self.model.room.name.is_some()
    }

    pub(super) fn apply_participant_status_update(
        &mut self,
        scope: Option<ParticipantPlaybackScope>,
        snapshot: Option<ParticipantStatusSnapshot>,
        scope_invalid: bool,
        received_at_seconds: f64,
    ) {
        if !self.participant_status_inbound_is_enabled() {
            self.clear_participant_status_views();
            return;
        }
        let Some(room) = self.model.room.name.as_deref() else {
            self.clear_participant_status_views();
            return;
        };
        let room = room.to_owned();

        if scope_invalid {
            // A malformed authoritative scope may describe a new epoch. Do
            // not commit its bundled snapshot under the previous scope, and
            // retire old precision without reopening the revision fence.
            self.invalidate_participant_status_evidence();
            return;
        }

        if scope.is_some_and(|candidate| {
            candidate.media_generation == 0
                || candidate.state_revision == Some(0)
                || candidate.transport_revision == Some(0)
        }) {
            // Schema-valid zero values are semantically malformed authority,
            // just like an undecodable scope. Retire the old epoch without
            // reopening the monotonic snapshot tombstone.
            self.invalidate_participant_status_evidence();
            return;
        }
        let snapshot = snapshot.filter(|snapshot| snapshot.revision != 0);
        if let Some(snapshot) = snapshot.as_ref()
            && self
                .model
                .room
                .participant_status_snapshot_revision
                .is_some_and(|revision| snapshot.revision <= revision)
        {
            // Scope and snapshot share one extension transaction. A stale
            // bundled snapshot cannot roll authoritative scope backward.
            return;
        }
        if let (Some(candidate), Some(current)) = (
            scope,
            self.model.room.participant_status_authoritative_scope,
        ) && Self::participant_status_scope_is_older(candidate, current)
        {
            // Reject the complete transaction: accepting a newer snapshot
            // under an older scope would reintroduce precisely the evidence
            // that the authority fence is intended to suppress.
            return;
        }

        let Some(snapshot) = snapshot else {
            if let Some(scope) = scope
                && self.model.room.participant_status_authoritative_scope != Some(scope)
            {
                self.model.room.participant_status_authoritative_scope = Some(scope);
                for status in self.model.room.participant_statuses.values_mut() {
                    status.redact_precise_scope_evidence();
                }
            }
            return;
        };

        let snapshot_revision = snapshot.revision;
        let snapshot_mode = snapshot.mode;
        let mut participant_statuses = BTreeMap::new();
        let mut receipts = BTreeMap::new();

        // Every extension is a complete current-room snapshot. Missing members
        // therefore lose an older projection, while membership itself remains
        // authoritative through List/Set.user.
        for (username, wire_view) in snapshot.participants {
            if snapshot_mode == ParticipantStatusSnapshotMode::Unavailable {
                // Unavailable mode is a population-size fallback and never
                // carries participant rows, even if a contradictory sender
                // supplied them.
                continue;
            }
            if username.trim().is_empty() {
                continue;
            }
            let Some(user) = self.model.room.users.get(&username) else {
                continue;
            };
            if user.room.as_deref() != Some(room.as_str()) {
                continue;
            }
            if self
                .model
                .room
                .participant_status_capabilities
                .get(&username)
                == Some(&false)
                && wire_view.availability != ParticipantStatusAvailability::Unsupported
            {
                // Explicit peer withdrawal is authoritative until a later
                // List/Set.user capability update opts the peer back in. A
                // field-free server-owned Unsupported row remains admissible.
                continue;
            }
            let mut view = ClientParticipantStatusView::from_wire(wire_view);
            if snapshot_mode == ParticipantStatusSnapshotMode::Compact {
                view.retain_compact_snapshot_fields();
            } else if snapshot_mode == ParticipantStatusSnapshotMode::Full
                && let Some(authoritative_scope) =
                    scope.or(self.model.room.participant_status_authoritative_scope)
                && view.status.correlation == Some(ParticipantStatusCorrelation::Exact)
                && view.status.playback_scope != Some(authoritative_scope)
            {
                view.redact_precise_scope_evidence();
            }
            participant_statuses.insert(username.clone(), view);
            receipts.insert(username, ParticipantStatusReceipt::new(received_at_seconds));
        }
        if snapshot_mode == ParticipantStatusSnapshotMode::Unavailable {
            for (username, user) in &self.model.room.users {
                if user.room.as_deref() != Some(room.as_str())
                    || self
                        .model
                        .room
                        .participant_status_capabilities
                        .get(username)
                        != Some(&true)
                {
                    continue;
                }
                participant_statuses.insert(
                    username.clone(),
                    ClientParticipantStatusView::from_wire(ParticipantStatusView::new(
                        ParticipantStatusAvailability::Unavailable,
                    )),
                );
                receipts.insert(
                    username.clone(),
                    ParticipantStatusReceipt::new(received_at_seconds),
                );
            }
        }

        // Commit scope and complete snapshot together only after every fence
        // has accepted the extension. In particular, do not first redact a
        // compact snapshot that is authoritative in this same message.
        if let Some(scope) = scope {
            self.model.room.participant_status_authoritative_scope = Some(scope);
        }
        self.model.room.participant_status_snapshot_revision = Some(snapshot_revision);
        self.model.room.participant_status_snapshot_mode = snapshot_mode;
        self.model.room.participant_statuses = participant_statuses;
        self.model.room.participant_status_receipts = receipts;
    }

    /// Returns the infrequent List snapshot position for compatibility UIs.
    /// It is intentionally named as a snapshot so callers do not treat it as
    /// live desynchronization evidence.
    pub fn user_legacy_list_position_snapshot_seconds(&self, username: &str) -> Option<f64> {
        self.model
            .room
            .legacy_list_position_snapshots
            .get(username)
            .copied()
    }
}
