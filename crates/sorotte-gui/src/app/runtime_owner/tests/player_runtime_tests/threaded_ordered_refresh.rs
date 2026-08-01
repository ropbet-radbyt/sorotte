use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex, mpsc},
    time::{Duration, Instant},
};

use sorotte_player_api::{
    LoadAttemptId, LocalFileUpdate, PlayerActiveLoadSnapshot, PlayerAdapter, PlayerAttachmentEpoch,
    PlayerCacheTelemetryUpdate, PlayerCommandId, PlayerCommandProgress,
    PlayerEventAcknowledgementToken, PlayerEventBatch, PlayerEventDeliveryMode,
    PlayerLocalFileObservation, PlayerMediaGeneration, PlayerMediaLoadObservation,
    PlayerMediaLoadOutcome, PlayerObservationBatch, PlayerPlaybackTelemetryUpdate,
    PlayerSequenceBoundary, PlayerTransportPhase, PlayerTransportSnapshot,
    PlayerTransportTelemetryUpdate, SnapshotField,
};

use super::*;

const WAIT_TIMEOUT: Duration = Duration::from_secs(2);
const ORDERED_PATH: &str = "C:\\ordered\\fresh.mkv";
const STALE_PATH: &str = "C:\\legacy\\stale.mkv";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AcknowledgementResult {
    Failed(PlayerEventAcknowledgementToken),
    Accepted(PlayerEventAcknowledgementToken),
}

#[derive(Default)]
struct AdapterProbeState {
    acknowledgement_results: Vec<AcknowledgementResult>,
    empty_batch_reads: usize,
    legacy_getter_calls: Vec<&'static str>,
    dropped: bool,
}

#[derive(Default)]
struct AdapterProbe {
    state: Mutex<AdapterProbeState>,
    changed: Condvar,
}

impl AdapterProbe {
    fn record(&self, update: impl FnOnce(&mut AdapterProbeState)) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        update(&mut state);
        drop(state);
        self.changed.notify_all();
    }

    fn wait_until(&self, description: &str, condition: impl Fn(&AdapterProbeState) -> bool) {
        let deadline = Instant::now() + WAIT_TIMEOUT;
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while !condition(&state) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "timed out waiting for {description}; acknowledgements={:?}, empty_batch_reads={}, legacy_getter_calls={:?}, dropped={}",
                state.acknowledgement_results,
                state.empty_batch_reads,
                state.legacy_getter_calls,
                state.dropped,
            );
            let (next_state, wait_result) = self
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state = next_state;
            assert!(
                !wait_result.timed_out() || condition(&state),
                "timed out waiting for {description}; acknowledgements={:?}, empty_batch_reads={}, legacy_getter_calls={:?}, dropped={}",
                state.acknowledgement_results,
                state.empty_batch_reads,
                state.legacy_getter_calls,
                state.dropped,
            );
        }
    }

    fn acknowledgement_results(&self) -> Vec<AcknowledgementResult> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .acknowledgement_results
            .clone()
    }

    fn legacy_getter_calls(&self) -> Vec<&'static str> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .legacy_getter_calls
            .clone()
    }
}

#[derive(Clone, Copy)]
enum LegacyGetterBehavior {
    Panic,
    ContradictoryResponses,
}

struct LegacyGetterPayloads {
    local_file_update: Option<LocalFileUpdate>,
    local_file_observation: Option<PlayerLocalFileObservation>,
    playback: Option<PlayerPlaybackTelemetryUpdate>,
    transport: Option<PlayerTransportTelemetryUpdate>,
    cache: Option<PlayerCacheTelemetryUpdate>,
    command_progress: Option<PlayerCommandProgress>,
    media_load_outcome: Option<PlayerMediaLoadOutcome>,
    media_load_observation: Option<PlayerMediaLoadObservation>,
    ordered_observation_batch: Option<PlayerObservationBatch>,
}

impl LegacyGetterPayloads {
    fn contradictory() -> Self {
        let generation = PlayerMediaGeneration::new(99);
        let stale_file = LocalFileUpdate::new("stale.mkv").with_path(STALE_PATH);
        Self {
            local_file_update: Some(stale_file.clone()),
            local_file_observation: Some(PlayerLocalFileObservation::new(
                stale_file,
                Some(generation),
                None,
            )),
            playback: Some(
                PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(false)
                    .with_position_seconds(999.0),
            ),
            transport: Some(
                PlayerTransportTelemetryUpdate::default()
                    .with_phase(PlayerTransportPhase::Playing)
                    .with_position_seconds(999.0)
                    .with_logical_pause(false),
            ),
            cache: Some(PlayerCacheTelemetryUpdate {
                media_generation: Some(generation),
                buffered_ahead_seconds: Some(999.0),
                ..PlayerCacheTelemetryUpdate::default()
            }),
            command_progress: Some(PlayerCommandProgress::accepted(
                PlayerCommandId::new(99),
                Some(generation),
                None,
            )),
            media_load_outcome: Some(PlayerMediaLoadOutcome::success(
                STALE_PATH,
                Some(STALE_PATH.to_owned()),
            )),
            media_load_observation: Some(PlayerMediaLoadObservation::new(
                PlayerMediaLoadOutcome::success(STALE_PATH, Some(STALE_PATH.to_owned())),
                Some(generation),
                None,
            )),
            ordered_observation_batch: Some(PlayerObservationBatch {
                legacy_playback_telemetry: Some(
                    PlayerPlaybackTelemetryUpdate::default()
                        .with_paused(false)
                        .with_position_seconds(999.0),
                ),
                ..PlayerObservationBatch::default()
            }),
        }
    }
}

struct OrderedAdapterScript {
    batches: VecDeque<PlayerEventBatch>,
    acknowledgement_failures_remaining: usize,
}

struct ThreadedOrderedPlayerAdapter {
    script: Arc<Mutex<OrderedAdapterScript>>,
    probe: Arc<AdapterProbe>,
    legacy_behavior: LegacyGetterBehavior,
    legacy_payloads: LegacyGetterPayloads,
}

impl ThreadedOrderedPlayerAdapter {
    fn record_legacy_getter_call(&self, getter: &'static str) {
        self.probe
            .record(|state| state.legacy_getter_calls.push(getter));
        if matches!(self.legacy_behavior, LegacyGetterBehavior::Panic) {
            panic!(
                "TC-GUI-ORDERED-001: acknowledged refresh called poisoned legacy getter {getter}"
            );
        }
    }
}

impl Drop for ThreadedOrderedPlayerAdapter {
    fn drop(&mut self) {
        self.probe.record(|state| state.dropped = true);
    }
}

impl PlayerAdapter for ThreadedOrderedPlayerAdapter {
    fn name(&self) -> &'static str {
        "threaded-ordered-refresh-test"
    }

    fn player_event_delivery_mode(&self) -> PlayerEventDeliveryMode {
        PlayerEventDeliveryMode::OrderedAcknowledgedBatches
    }

    fn take_player_event_batch(&mut self) -> Option<PlayerEventBatch> {
        let batch = self
            .script
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .batches
            .front()
            .cloned();
        if batch.is_none() {
            self.probe.record(|state| state.empty_batch_reads += 1);
        }
        batch
    }

    fn acknowledge_player_event_batch(
        &mut self,
        token: PlayerEventAcknowledgementToken,
    ) -> Result<(), sorotte_player_api::PlayerError> {
        let mut script = self
            .script
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let expected = script
            .batches
            .front()
            .map(|batch| batch.acknowledgement_token);
        if expected != Some(token) {
            return Err(sorotte_player_api::PlayerError::OperationFailed(
                "unexpected synthetic acknowledgement token".to_owned(),
            ));
        }
        if script.acknowledgement_failures_remaining != 0 {
            script.acknowledgement_failures_remaining -= 1;
            drop(script);
            self.probe.record(|state| {
                state
                    .acknowledgement_results
                    .push(AcknowledgementResult::Failed(token));
            });
            return Err(sorotte_player_api::PlayerError::OperationFailed(
                "synthetic acknowledgement failure".to_owned(),
            ));
        }
        script.batches.pop_front();
        drop(script);
        self.probe.record(|state| {
            state
                .acknowledgement_results
                .push(AcknowledgementResult::Accepted(token));
        });
        Ok(())
    }

    fn take_ordered_event_batch(&mut self) -> Option<PlayerObservationBatch> {
        self.record_legacy_getter_call("take_ordered_event_batch");
        self.legacy_payloads.ordered_observation_batch.take()
    }

    fn request_ordered_event_reacquisition(&mut self) {
        self.record_legacy_getter_call("request_ordered_event_reacquisition");
    }

    fn take_command_progress(&mut self) -> Option<PlayerCommandProgress> {
        self.record_legacy_getter_call("take_command_progress");
        self.legacy_payloads.command_progress.take()
    }

    fn take_playback_telemetry_update(&mut self) -> Option<PlayerPlaybackTelemetryUpdate> {
        self.record_legacy_getter_call("take_playback_telemetry_update");
        self.legacy_payloads.playback.take()
    }

    fn take_transport_telemetry_update(&mut self) -> Option<PlayerTransportTelemetryUpdate> {
        self.record_legacy_getter_call("take_transport_telemetry_update");
        self.legacy_payloads.transport.take()
    }

    fn take_cache_telemetry_update(&mut self) -> Option<PlayerCacheTelemetryUpdate> {
        self.record_legacy_getter_call("take_cache_telemetry_update");
        self.legacy_payloads.cache.take()
    }

    fn take_media_load_outcome(&mut self) -> Option<PlayerMediaLoadOutcome> {
        self.record_legacy_getter_call("take_media_load_outcome");
        self.legacy_payloads.media_load_outcome.take()
    }

    fn take_media_load_observation(&mut self) -> Option<PlayerMediaLoadObservation> {
        self.record_legacy_getter_call("take_media_load_observation");
        self.legacy_payloads.media_load_observation.take()
    }

    fn take_local_file_update(&mut self) -> Option<LocalFileUpdate> {
        self.record_legacy_getter_call("take_local_file_update");
        self.legacy_payloads.local_file_update.take()
    }

    fn take_local_file_observation(&mut self) -> Option<PlayerLocalFileObservation> {
        self.record_legacy_getter_call("take_local_file_observation");
        self.legacy_payloads.local_file_observation.take()
    }
}

fn ordered_snapshot_batch(
    token_value: u64,
    path: &str,
    position_seconds: f64,
    paused: bool,
) -> PlayerEventBatch {
    let attachment_epoch = PlayerAttachmentEpoch::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let generation = PlayerMediaGeneration::new(1);
    PlayerEventBatch {
        attachment_epoch,
        sequence_boundary: PlayerSequenceBoundary::new(attachment_epoch, 0),
        authoritative_snapshot: Some(sorotte_player_api::PlayerAuthoritativeSnapshot {
            attachment_epoch,
            sequence_boundary: PlayerSequenceBoundary::new(attachment_epoch, 0),
            transport: PlayerTransportSnapshot {
                load_attempt_id: SnapshotField::Known(attempt_id),
                media_generation: SnapshotField::Known(generation),
                phase: SnapshotField::Known(if paused {
                    PlayerTransportPhase::ReadyPaused
                } else {
                    PlayerTransportPhase::Playing
                }),
                position_seconds: SnapshotField::Known(position_seconds),
                playback_rate: SnapshotField::Known(1.0),
                logical_pause: SnapshotField::Known(paused),
                paused_for_cache: SnapshotField::Known(false),
                cache_percentage: SnapshotField::Known(100.0),
                seeking: SnapshotField::Known(false),
                seekable: SnapshotField::Known(true),
                eof_reached: SnapshotField::Known(false),
                ..PlayerTransportSnapshot::default()
            },
            active_load: SnapshotField::Known(PlayerActiveLoadSnapshot {
                attempt_id,
                media_generation: generation,
                command_id: Some(PlayerCommandId::new(1)),
                playlist_entry_id: Some(1),
                physical_file_loaded: true,
                semantic_load_result: Some(sorotte_player_api::PlayerLoadAttemptResult::Loaded),
                logical_ownership_revoked: false,
            }),
            current_playlist_entry_id: SnapshotField::Known(1),
            current_path: SnapshotField::Known(path.to_owned()),
        }),
        events: Vec::new(),
        semantic_outcomes: Vec::new(),
        acknowledgement_token: PlayerEventAcknowledgementToken::new(attachment_epoch, token_value),
    }
}

fn threaded_runtime(
    legacy_behavior: LegacyGetterBehavior,
    acknowledgement_failures_remaining: usize,
) -> (
    GuiQueuedRuntimeBridge,
    GuiThreadedRuntimeOwnerPump,
    Arc<AdapterProbe>,
    mpsc::Receiver<()>,
) {
    let probe = Arc::new(AdapterProbe::default());
    let script = Arc::new(Mutex::new(OrderedAdapterScript {
        batches: VecDeque::from([ordered_snapshot_batch(1, ORDERED_PATH, 12.5, true)]),
        acknowledgement_failures_remaining,
    }));
    let adapter = ThreadedOrderedPlayerAdapter {
        script,
        probe: probe.clone(),
        legacy_behavior,
        legacy_payloads: LegacyGetterPayloads::contradictory(),
    };
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(adapter)));

    let (runtime, handle) = GuiQueuedRuntimeBridge::new();
    let (repaint_tx, repaint_rx) = mpsc::channel();
    handle.set_repaint_notifier(move || {
        let _ = repaint_tx.send(());
    });
    let pump = GuiThreadedRuntimeOwnerPump::new_with_poll_interval(
        handle,
        owner,
        Duration::from_millis(2),
    )
    .expect("production threaded runtime owner should spawn");
    (runtime, pump, probe, repaint_rx)
}

fn drain_and_apply_until(
    runtime: &mut GuiQueuedRuntimeBridge,
    pump: &mut GuiThreadedRuntimeOwnerPump,
    state: &mut SorotteGuiShellAppState,
    repaint_rx: &mpsc::Receiver<()>,
    description: &str,
    condition: impl Fn(&SorotteGuiShellAppState) -> bool,
) -> Vec<GuiShellAction> {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    let mut observed_actions = Vec::new();
    loop {
        let actions = GuiNativeRuntimeBridge::drain_runtime_actions(runtime);
        if !actions.is_empty() {
            for action in actions {
                let action_for_evidence = action.clone();
                let _ = state.apply(action);
                observed_actions.push(action_for_evidence);
            }
            GuiNativeRuntimePump::pump(pump, state);
        }
        if condition(state) {
            return observed_actions;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "timed out waiting for {description}; playlist={:?}, paused={}",
            state
                .main_window
                .playlist
                .iter()
                .map(|row| row.label.as_str())
                .collect::<Vec<_>>(),
            state.main_window.playback_paused,
        );
        repaint_rx
            .recv_timeout(remaining)
            .unwrap_or_else(|error| panic!("timed out waiting for {description}: {error}"));
    }
}

fn assert_ordered_projection_is_atomic(actions: &[GuiShellAction], expected_paused: bool) {
    let matching_snapshots = actions
        .iter()
        .filter_map(|action| match action {
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot)
                if snapshot.playlist == ["fresh.mkv".to_owned()] =>
            {
                Some(snapshot)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        !matching_snapshots.is_empty(),
        "threaded refresh should publish the ordered file projection"
    );
    assert!(
        matching_snapshots
            .iter()
            .all(|snapshot| snapshot.playback_paused == expected_paused),
        "file identity and pause state must come from the same ordered projection"
    );
    assert!(
        actions.iter().all(|action| match action {
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot) => snapshot
                .playlist
                .iter()
                .all(|entry| !entry.contains("stale.mkv")),
            _ => true,
        }),
        "contradictory legacy identity must never leak into the shell projection"
    );
}

fn shutdown_and_assert_bounded(pump: &mut GuiThreadedRuntimeOwnerPump, probe: &AdapterProbe) {
    let started = Instant::now();
    GuiNativeRuntimePump::shutdown(pump);
    assert!(
        started.elapsed() < WAIT_TIMEOUT,
        "threaded runtime shutdown should remain bounded"
    );
    probe.wait_until("ordered adapter drop after runtime shutdown", |state| {
        state.dropped
    });
}

#[test]
fn threaded_ordered_refresh_projects_success_without_calling_poisoned_legacy_getters() {
    let (mut runtime, mut pump, probe, repaint_rx) =
        threaded_runtime(LegacyGetterBehavior::Panic, 0);
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(false),
        ..StoredClientSettingsMvp::default()
    });

    GuiNativeRuntimePump::pump(&mut pump, &state);
    probe.wait_until("ordered batch acknowledgement", |probe_state| {
        !probe_state.acknowledgement_results.is_empty()
            || !probe_state.legacy_getter_calls.is_empty()
            || probe_state.dropped
    });
    assert!(
        probe.legacy_getter_calls().is_empty(),
        "acknowledged refresh touched a poisoned legacy getter"
    );
    assert_eq!(
        probe.acknowledgement_results(),
        vec![AcknowledgementResult::Accepted(
            PlayerEventAcknowledgementToken::new(PlayerAttachmentEpoch::new(1), 1)
        )]
    );

    let actions = drain_and_apply_until(
        &mut runtime,
        &mut pump,
        &mut state,
        &repaint_rx,
        "atomic ordered player projection",
        |state| {
            state
                .main_window
                .playlist
                .iter()
                .map(|row| row.label.as_str())
                .eq(["fresh.mkv"])
                && state.main_window.playback_paused
        },
    );
    assert_ordered_projection_is_atomic(&actions, true);
    assert!(probe.legacy_getter_calls().is_empty());

    shutdown_and_assert_bounded(&mut pump, &probe);
}

#[test]
fn threaded_ordered_refresh_recovers_ack_failure_without_mixed_or_stale_delivery() {
    let (mut runtime, mut pump, probe, repaint_rx) =
        threaded_runtime(LegacyGetterBehavior::ContradictoryResponses, 1);
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(false),
        ..StoredClientSettingsMvp::default()
    });
    let token = PlayerEventAcknowledgementToken::new(PlayerAttachmentEpoch::new(1), 1);

    GuiNativeRuntimePump::pump(&mut pump, &state);
    probe.wait_until(
        "failed acknowledgement replay and recovery",
        |probe_state| {
            probe_state.acknowledgement_results.len() >= 2
                || !probe_state.legacy_getter_calls.is_empty()
                || probe_state.dropped
        },
    );
    assert_eq!(
        probe.acknowledgement_results(),
        vec![
            AcknowledgementResult::Failed(token),
            AcknowledgementResult::Accepted(token),
        ],
        "the same unacknowledged batch should be replayed and then accepted"
    );
    assert!(
        probe.legacy_getter_calls().is_empty(),
        "acknowledged refresh mixed in contradictory legacy responses"
    );

    let actions = drain_and_apply_until(
        &mut runtime,
        &mut pump,
        &mut state,
        &repaint_rx,
        "recovered ordered player projection",
        |state| {
            state
                .main_window
                .playlist
                .iter()
                .map(|row| row.label.as_str())
                .eq(["fresh.mkv"])
                && state.main_window.playback_paused
        },
    );
    assert_ordered_projection_is_atomic(&actions, true);

    probe.wait_until("post-recovery empty ordered refresh", |probe_state| {
        probe_state.empty_batch_reads >= 2 || !probe_state.legacy_getter_calls.is_empty()
    });
    let trailing_actions = GuiNativeRuntimeBridge::drain_runtime_actions(&mut runtime);
    assert_ordered_projection_is_atomic(&actions, true);
    assert!(
        trailing_actions.iter().all(|action| match action {
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot) => snapshot
                .playlist
                .iter()
                .all(|entry| !entry.contains("stale.mkv")),
            _ => true,
        }),
        "post-recovery polling must not deliver stale legacy state"
    );
    assert!(probe.legacy_getter_calls().is_empty());

    shutdown_and_assert_bounded(&mut pump, &probe);
}
