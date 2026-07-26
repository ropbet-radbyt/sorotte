use std::collections::BTreeSet;

use serde_json::Value;
use sorotte_player_api::{
    LifecycleVerificationProjection, LoadAttemptId, PlayerAdapter, PlayerCommandFailureKind,
    PlayerCommandId, PlayerError, PlayerEventAcknowledgementToken, PlayerEventBatch,
    PlayerMediaGeneration, PlayerTransportPhase, PlayerTransportSnapshot, SnapshotField,
};

use super::{MpvAdapter, TrackedCommandKind};
use crate::lifecycle::{AuthoritativePlaylistEntry, PlayerLifecycleInput};

/// One real tracked-load identity allocated at the verification boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[doc(hidden)]
pub struct LifecycleVerificationTrackedLoad {
    pub command_id: PlayerCommandId,
    pub attempt_id: LoadAttemptId,
    pub media_generation: PlayerMediaGeneration,
}

/// Authoritative playlist input accepted by the verification harness.
#[doc(hidden)]
pub type LifecycleVerificationPlaylistEntry = AuthoritativePlaylistEntry;

/// Deterministic, no-sleep driver for the adapter's real lifecycle boundaries.
#[derive(Debug)]
#[doc(hidden)]
pub struct MpvLifecycleVerificationHarness {
    adapter: MpvAdapter,
    now_tick: u64,
}

impl Default for MpvLifecycleVerificationHarness {
    fn default() -> Self {
        Self::new()
    }
}

impl MpvLifecycleVerificationHarness {
    pub fn new() -> Self {
        let adapter = MpvAdapter::default();
        let now_tick = adapter.player_lifecycle.now_tick;
        Self { adapter, now_tick }
    }

    pub fn from_adapter(adapter: MpvAdapter) -> Self {
        let now_tick = adapter.player_lifecycle.now_tick;
        Self { adapter, now_tick }
    }

    pub fn adapter(&self) -> &MpvAdapter {
        &self.adapter
    }

    pub fn adapter_mut(&mut self) -> &mut MpvAdapter {
        &mut self.adapter
    }

    pub fn into_adapter(self) -> MpvAdapter {
        self.adapter
    }

    pub fn projection(&self) -> LifecycleVerificationProjection {
        self.adapter.lifecycle_verification_projection()
    }

    pub fn accept_tracked_load(
        &mut self,
        target: impl Into<String>,
        baseline_playlist_entry_ids: impl IntoIterator<Item = i64>,
    ) -> LifecycleVerificationTrackedLoad {
        let tracked = self.begin_tracked_load(target, baseline_playlist_entry_ids);
        let attachment_epoch = self.adapter.lifecycle_epoch();
        self.adapter
            .apply_lifecycle_input(PlayerLifecycleInput::LoadAttemptAccepted {
                attachment_epoch,
                attempt_id: tracked.attempt_id,
            });
        self.adapter.accept_tracked_command(tracked.command_id);
        self.adapter
            .supersede_tracked_commands(Some(tracked.command_id), |kind| {
                kind.is_load_seek_or_play()
            });
        self.now_tick = self.adapter.player_lifecycle.now_tick;
        tracked
    }

    pub fn reject_tracked_load(
        &mut self,
        target: impl Into<String>,
        baseline_playlist_entry_ids: impl IntoIterator<Item = i64>,
        failure: PlayerCommandFailureKind,
    ) -> LifecycleVerificationTrackedLoad {
        let tracked = self.begin_tracked_load(target, baseline_playlist_entry_ids);
        let attachment_epoch = self.adapter.lifecycle_epoch();
        self.adapter
            .apply_lifecycle_input(PlayerLifecycleInput::LoadAttemptRejected {
                attachment_epoch,
                attempt_id: tracked.attempt_id,
                failure,
            });
        self.adapter
            .discard_unaccepted_tracked_command(tracked.command_id);
        if self.adapter.pending_load_generation == Some(tracked.media_generation) {
            self.adapter.pending_load_request = None;
            self.adapter.pending_load_generation = None;
        }
        tracked
    }

    pub fn accept_same_generation_recovery(
        &mut self,
        media_generation: PlayerMediaGeneration,
        target: impl Into<String>,
        baseline_playlist_entry_ids: impl IntoIterator<Item = i64>,
    ) -> LoadAttemptId {
        let target = target.into();
        let attempt_id = self.adapter.submit_lifecycle_load(
            None,
            media_generation,
            &target,
            baseline_playlist_entry_ids
                .into_iter()
                .collect::<BTreeSet<_>>(),
        );
        let attachment_epoch = self.adapter.lifecycle_epoch();
        self.adapter
            .apply_lifecycle_input(PlayerLifecycleInput::LoadAttemptAccepted {
                attachment_epoch,
                attempt_id,
            });
        self.adapter.lifecycle_reconciliation_due = true;
        self.now_tick = self.adapter.player_lifecycle.now_tick;
        attempt_id
    }

    pub fn accept_tracked_pause(&mut self) -> PlayerCommandId {
        let command_id = self.adapter.register_tracked_command(
            self.adapter.media_generation(),
            TrackedCommandKind::Pause {
                logical_pause_observed: false,
            },
        );
        self.adapter.accept_tracked_command(command_id);
        command_id
    }

    pub fn apply_authoritative_snapshot(
        &mut self,
        entries: impl IntoIterator<Item = LifecycleVerificationPlaylistEntry>,
        current_path: Option<String>,
    ) {
        let attachment_epoch = self.adapter.lifecycle_epoch();
        let paused = self.adapter.observed_state.paused;
        let logical_pause = self.adapter.observed_state.logical_pause;
        let playback_rate = self.adapter.observed_state.playback_rate;
        let paused_for_cache = self.adapter.observed_state.paused_for_cache;
        let cache_buffering_percent = self.adapter.observed_state.cache_buffering_percent;
        let seeking = self.adapter.observed_state.seeking;
        let seekable = self.adapter.observed_state.seekable;
        let core_idle = self.adapter.observed_state.core_idle;
        let demuxer_cache_idle = self.adapter.observed_state.demuxer_cache_idle;
        let eof_reached = self.adapter.observed_state.eof_reached;
        let entries = entries.into_iter().collect::<Vec<_>>();
        let authoritative_current_entry_id = entries
            .iter()
            .find(|entry| entry.current)
            .map(|entry| entry.id);
        self.adapter
            .apply_lifecycle_input(PlayerLifecycleInput::PlaylistSnapshot {
                attachment_epoch,
                entries: entries.clone(),
                current_path: current_path.clone(),
            });
        self.adapter
            .observe_external_current_from_authority(&entries, current_path.as_deref());
        self.adapter.replay_deferred_start_file_if_bound();
        self.adapter.replay_deferred_file_loaded_if_bound();
        self.adapter.observed_state.path = current_path;
        self.adapter.observed_state.paused = paused;
        self.adapter.observed_state.logical_pause = logical_pause;
        self.adapter.observed_state.playback_rate = playback_rate;
        self.adapter.observed_state.paused_for_cache = paused_for_cache;
        self.adapter.observed_state.cache_buffering_percent = cache_buffering_percent;
        self.adapter.observed_state.seeking = seeking;
        self.adapter.observed_state.seekable = seekable;
        self.adapter.observed_state.core_idle = core_idle;
        self.adapter.observed_state.demuxer_cache_idle = demuxer_cache_idle;
        self.adapter.observed_state.eof_reached = eof_reached;
        self.adapter
            .publish_reconciled_transport_state(authoritative_current_entry_id);
        if self
            .adapter
            .player_lifecycle
            .requires_authoritative_snapshot()
        {
            self.adapter.publish_authoritative_lifecycle_snapshot();
        }
        self.adapter.lifecycle_reconciliation_due =
            self.adapter.player_lifecycle.reconciliation_required
                || self
                    .adapter
                    .player_lifecycle
                    .requires_authoritative_snapshot()
                || self.adapter.deferred_start_file_observation.is_some()
                || self.adapter.deferred_file_loaded_observation.is_some();
    }

    /// Sends an already-decoded raw mpv JSON value through the production ingress handler.
    pub fn ingest_decoded_mpv_json(&mut self, value: Value) {
        self.adapter.handle_ipc_event(&value);
    }

    pub fn advance_clock(&mut self, ticks: u64) {
        self.now_tick = self.now_tick.saturating_add(ticks);
        self.adapter
            .apply_lifecycle_input(PlayerLifecycleInput::TimerAdvanced {
                now_tick: self.now_tick,
            });
    }

    pub fn detect_event_gap(&mut self) {
        let attachment_epoch = self.adapter.lifecycle_epoch();
        self.adapter
            .apply_lifecycle_input(PlayerLifecycleInput::EventGapDetected { attachment_epoch });
    }

    pub fn take_event_batch(&mut self) -> Option<PlayerEventBatch> {
        PlayerAdapter::take_player_event_batch(&mut self.adapter)
    }

    pub fn acknowledge(
        &mut self,
        token: PlayerEventAcknowledgementToken,
    ) -> Result<(), PlayerError> {
        PlayerAdapter::acknowledge_player_event_batch(&mut self.adapter, token)
    }

    pub fn replace_attachment(&mut self) {
        self.adapter.reset_player_state_for_new_attachment();
        self.adapter
            .apply_lifecycle_input(PlayerLifecycleInput::AttachmentReplaced);
        self.adapter.next_lifecycle_transcript_ingress_sequence = 1;
        self.now_tick = self.adapter.player_lifecycle.now_tick;
    }

    fn begin_tracked_load(
        &mut self,
        target: impl Into<String>,
        baseline_playlist_entry_ids: impl IntoIterator<Item = i64>,
    ) -> LifecycleVerificationTrackedLoad {
        let target = target.into();
        let expected_generation =
            PlayerMediaGeneration::new(self.adapter.next_media_generation.max(1));
        let command_id = self.adapter.register_tracked_command(
            Some(expected_generation),
            TrackedCommandKind::Load {
                file_loaded: false,
                ready: false,
            },
        );
        let media_generation = self.adapter.allocate_media_generation();
        debug_assert_eq!(media_generation, expected_generation);
        let attempt_id = self.adapter.submit_lifecycle_load(
            Some(command_id),
            media_generation,
            &target,
            baseline_playlist_entry_ids
                .into_iter()
                .collect::<BTreeSet<_>>(),
        );
        self.adapter.pending_load_request = Some(target);
        self.adapter.pending_load_generation = Some(media_generation);
        LifecycleVerificationTrackedLoad {
            command_id,
            attempt_id,
            media_generation,
        }
    }
}

impl MpvAdapter {
    /// Returns reducer facts plus the adapter's keyed physical projection.
    #[doc(hidden)]
    pub fn lifecycle_verification_projection(&self) -> LifecycleVerificationProjection {
        let mut projection = self.player_lifecycle.lifecycle_verification_projection();
        projection.physical_transport_owner = self
            .active_load_attempt_id
            .map(SnapshotField::Known)
            .unwrap_or(SnapshotField::KnownAbsent);
        projection.physical_media_generation = self
            .active_media_generation
            .map(SnapshotField::Known)
            .unwrap_or(SnapshotField::KnownAbsent);
        projection.physical_playlist_entry_id = match self.active_playlist_entry_id {
            Some(playlist_entry_id) => i64::try_from(playlist_entry_id)
                .map(SnapshotField::Known)
                .unwrap_or(SnapshotField::Unavailable),
            None => SnapshotField::KnownAbsent,
        };
        projection.physical_path = self
            .current_path
            .clone()
            .map(SnapshotField::Known)
            .unwrap_or(SnapshotField::KnownAbsent);
        projection.physical_file_loaded = self
            .active_load_attempt_id
            .map(|_| SnapshotField::Known(self.active_file_loaded))
            .unwrap_or(SnapshotField::KnownAbsent);
        for (attempt_id, attempt) in &mut projection.attempts {
            attempt.owns_transport =
                SnapshotField::Known(self.active_load_attempt_id == Some(*attempt_id));
        }
        projection.transport = self.lifecycle_verification_transport_snapshot();
        projection
    }

    fn lifecycle_verification_transport_snapshot(&self) -> PlayerTransportSnapshot {
        PlayerTransportSnapshot {
            load_attempt_id: self
                .active_load_attempt_id
                .map(SnapshotField::Known)
                .unwrap_or(SnapshotField::KnownAbsent),
            media_generation: self
                .active_media_generation
                .map(SnapshotField::Known)
                .unwrap_or(SnapshotField::KnownAbsent),
            observed_at: SnapshotField::Unavailable,
            phase: SnapshotField::Known(self.transport_phase),
            position_seconds: verification_field(self.observed_state.position_seconds),
            playback_rate: verification_field(self.observed_state.playback_rate),
            logical_pause: verification_field(self.observed_state.logical_pause),
            paused_for_cache: verification_field(self.observed_state.paused_for_cache),
            cache_percentage: verification_field(self.observed_state.cache_buffering_percent),
            seeking: verification_field(self.observed_state.seeking),
            seekable: verification_field(self.observed_state.seekable),
            timeline_kind: SnapshotField::Known(self.timeline_kind),
            core_idle: verification_field(self.observed_state.core_idle),
            demuxer_cache_idle: verification_field(self.observed_state.demuxer_cache_idle),
            playback_restart_sequence: SnapshotField::Known(self.playback_restart_sequence),
            eof_reached: verification_field(self.observed_state.eof_reached),
            seekable_ranges: SnapshotField::Unavailable,
            known_live_seekable_window: self
                .latest_cached_seekable_window
                .map(SnapshotField::Known)
                .unwrap_or(SnapshotField::KnownAbsent),
            buffered_duration_seconds: verification_field(
                self.observed_state.buffered_ahead_seconds,
            ),
            buffered_bytes: verification_field(self.observed_state.buffered_ahead_bytes),
            input_rate_bytes_per_second: verification_field(
                self.observed_state.input_rate_bytes_per_second,
            ),
            error_kind: if self.transport_phase == PlayerTransportPhase::Failed {
                SnapshotField::Unavailable
            } else {
                SnapshotField::KnownAbsent
            },
        }
    }
}

fn verification_field<T>(value: Option<T>) -> SnapshotField<T> {
    value
        .map(SnapshotField::Known)
        .unwrap_or(SnapshotField::Unavailable)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use sorotte_player_api::{
        PlayerAttachmentEpoch, PlayerCommandFailureKind, PlayerCommandSemanticResult,
        PlayerLoadAttemptResult, SnapshotField,
    };

    use super::{LifecycleVerificationPlaylistEntry, MpvLifecycleVerificationHarness};

    fn known<T>(field: &SnapshotField<T>) -> &T {
        let SnapshotField::Known(value) = field else {
            panic!("verification producer must project this field as known");
        };
        value
    }

    #[test]
    fn tracked_load_uses_real_reducer_and_physical_projection_boundaries() {
        let mut harness = MpvLifecycleVerificationHarness::new();
        let tracked = harness.accept_tracked_load("C:/media/movie.mkv", []);

        let accepted = harness.projection();
        assert_eq!(
            accepted.logical_owner,
            SnapshotField::Known(tracked.attempt_id)
        );
        assert_eq!(
            accepted.physical_transport_owner,
            SnapshotField::KnownAbsent
        );
        assert!(known(&accepted.pending_commands).contains(&tracked.command_id));
        assert_eq!(
            accepted.attempts[&tracked.attempt_id].semantic_load_result,
            SnapshotField::Known(None)
        );

        harness.apply_authoritative_snapshot(
            [LifecycleVerificationPlaylistEntry::new(
                77,
                Some("C:/media/movie.mkv".to_owned()),
                true,
            )],
            Some("C:/media/movie.mkv".to_owned()),
        );
        let bound = harness.projection();
        assert_eq!(
            bound.physical_transport_owner,
            SnapshotField::Known(tracked.attempt_id)
        );
        assert_eq!(
            bound.physical_path,
            SnapshotField::Known("C:/media/movie.mkv".to_owned())
        );
        assert_eq!(bound.physical_file_loaded, SnapshotField::Known(false));

        harness.ingest_decoded_mpv_json(json!({
            "event": "start-file",
            "playlist_entry_id": 77
        }));
        harness.ingest_decoded_mpv_json(json!({
            "event": "property-change",
            "name": "path",
            "data": "C:/media/movie.mkv"
        }));
        harness.ingest_decoded_mpv_json(json!({ "event": "file-loaded" }));

        let loaded = harness.projection();
        assert_eq!(
            loaded.physical_transport_owner,
            SnapshotField::Known(tracked.attempt_id)
        );
        assert_eq!(loaded.physical_file_loaded, SnapshotField::Known(true));
        assert_eq!(
            loaded.attempts[&tracked.attempt_id].semantic_load_result,
            SnapshotField::Known(Some(PlayerLoadAttemptResult::Loaded))
        );
        assert_eq!(
            known(&loaded.terminal_command_results).get(&tracked.command_id),
            Some(&PlayerCommandSemanticResult::Completed)
        );

        let batch = harness.take_event_batch().expect("load event batch");
        assert_eq!(
            harness.projection().in_flight_acknowledgement,
            SnapshotField::Known(batch.acknowledgement_token)
        );
        harness
            .acknowledge(batch.acknowledgement_token)
            .expect("matching acknowledgement");
        assert_eq!(
            harness.projection().attempts[&tracked.attempt_id].semantic_load_result,
            SnapshotField::Known(Some(PlayerLoadAttemptResult::Loaded))
        );
    }

    #[test]
    fn rejected_and_timed_out_loads_are_scriptable_without_sleeping() {
        let mut rejected = MpvLifecycleVerificationHarness::new();
        let rejected_load = rejected.reject_tracked_load(
            "C:/media/rejected.mkv",
            [],
            PlayerCommandFailureKind::Unknown,
        );
        let rejected_projection = rejected.projection();
        assert_eq!(
            known(&rejected_projection.terminal_command_results)[&rejected_load.command_id],
            PlayerCommandSemanticResult::Failed(PlayerCommandFailureKind::Unknown)
        );
        assert_eq!(
            known(&rejected_projection.terminal_load_results)[&rejected_load.attempt_id],
            PlayerLoadAttemptResult::NeverStarted
        );
        assert_eq!(
            rejected_projection.attempts[&rejected_load.attempt_id].physical_terminal,
            SnapshotField::Known(true)
        );

        let mut timed_out = MpvLifecycleVerificationHarness::new();
        let accepted = timed_out.accept_tracked_load("https://media.test/slow", []);
        timed_out.advance_clock(60_000);
        let timed_out_projection = timed_out.projection();
        assert_eq!(
            known(&timed_out_projection.terminal_load_results)[&accepted.attempt_id],
            PlayerLoadAttemptResult::Indeterminate
        );
        assert_eq!(
            timed_out_projection.physical_transport_owner,
            SnapshotField::KnownAbsent
        );
    }

    #[test]
    fn late_file_loaded_after_indeterminate_does_not_invent_loaded_semantics() {
        let mut harness = MpvLifecycleVerificationHarness::new();
        let tracked = harness.accept_tracked_load("https://media.test/late", []);
        harness.advance_clock(60_000);
        assert_eq!(
            known(&harness.projection().terminal_load_results)[&tracked.attempt_id],
            PlayerLoadAttemptResult::Indeterminate
        );

        let timeout_batch = harness.take_event_batch().expect("timeout batch");
        harness
            .acknowledge(timeout_batch.acknowledgement_token)
            .expect("timeout acknowledgement");
        assert_eq!(
            harness.projection().attempts[&tracked.attempt_id].semantic_load_result,
            SnapshotField::Known(Some(PlayerLoadAttemptResult::Indeterminate))
        );

        harness.apply_authoritative_snapshot(
            [LifecycleVerificationPlaylistEntry::new(
                91,
                Some("https://media.test/late".to_owned()),
                true,
            )],
            Some("https://media.test/late".to_owned()),
        );
        harness.ingest_decoded_mpv_json(json!({
            "event": "start-file",
            "playlist_entry_id": 91
        }));
        harness.ingest_decoded_mpv_json(json!({ "event": "file-loaded" }));

        let late = harness.projection();
        assert_eq!(
            late.attempts[&tracked.attempt_id].semantic_load_result,
            SnapshotField::Known(Some(PlayerLoadAttemptResult::Indeterminate))
        );
        assert_eq!(
            known(&late.terminal_load_results)[&tracked.attempt_id],
            PlayerLoadAttemptResult::Indeterminate
        );
        let late_batch = harness.take_event_batch().expect("late physical batch");
        assert!(late_batch.semantic_outcomes.iter().all(|outcome| {
            !matches!(
                &outcome.outcome,
                sorotte_player_api::PlayerSemanticOutcome::LoadAttempt(load)
                    if load.attempt_id == tracked.attempt_id
            )
        }));
    }

    #[test]
    fn gap_snapshot_acknowledgement_and_attachment_replacement_are_scriptable() {
        let mut harness = MpvLifecycleVerificationHarness::new();
        let tracked = harness.accept_tracked_load("C:/media/epoch-one.mkv", []);
        harness.detect_event_gap();
        assert_eq!(
            harness.projection().snapshot_required,
            SnapshotField::Known(true)
        );

        harness.apply_authoritative_snapshot(
            [LifecycleVerificationPlaylistEntry::new(
                9,
                Some("C:/media/epoch-one.mkv".to_owned()),
                true,
            )],
            Some("C:/media/epoch-one.mkv".to_owned()),
        );
        assert_eq!(
            harness.projection().snapshot_required,
            SnapshotField::Known(false)
        );
        let gap_batch = harness.take_event_batch().expect("gap recovery batch");
        harness
            .acknowledge(gap_batch.acknowledgement_token)
            .expect("gap recovery acknowledgement");

        harness.replace_attachment();
        let replacement = harness.projection();
        assert_eq!(
            replacement.attachment_epoch,
            SnapshotField::Known(PlayerAttachmentEpoch::new(2))
        );
        assert_eq!(
            replacement.physical_transport_owner,
            SnapshotField::KnownAbsent
        );
        assert!(!replacement.attempts.contains_key(&tracked.attempt_id));
        assert_eq!(replacement.snapshot_required, SnapshotField::Known(true));
    }

    #[test]
    fn same_generation_recovery_can_bind_and_start_as_the_successor() {
        let mut harness = MpvLifecycleVerificationHarness::new();
        let initial = harness.accept_tracked_load("https://media.test/live", []);
        harness.apply_authoritative_snapshot(
            [LifecycleVerificationPlaylistEntry::new(
                10,
                Some("https://media.test/live".to_owned()),
                true,
            )],
            Some("https://media.test/live".to_owned()),
        );
        harness.ingest_decoded_mpv_json(json!({
            "event": "start-file",
            "playlist_entry_id": 10
        }));
        harness.ingest_decoded_mpv_json(json!({ "event": "file-loaded" }));

        let recovery = harness.accept_same_generation_recovery(
            initial.media_generation,
            "https://media.test/live",
            [10],
        );
        assert_eq!(
            harness.projection().attempts[&recovery].media_generation,
            initial.media_generation
        );

        harness.apply_authoritative_snapshot(
            [
                LifecycleVerificationPlaylistEntry::new(
                    10,
                    Some("https://media.test/live".to_owned()),
                    false,
                ),
                LifecycleVerificationPlaylistEntry::new(
                    11,
                    Some("https://media.test/live".to_owned()),
                    true,
                ),
            ],
            Some("https://media.test/live".to_owned()),
        );
        harness.ingest_decoded_mpv_json(json!({
            "event": "start-file",
            "playlist_entry_id": 11
        }));

        let started = harness.projection();
        assert_eq!(started.attempts[&recovery].playlist_entry_id, Some(11));
        assert_eq!(
            started.physical_transport_owner,
            SnapshotField::Known(recovery)
        );
        assert_eq!(
            started.physical_media_generation,
            SnapshotField::Known(initial.media_generation)
        );
    }
}
