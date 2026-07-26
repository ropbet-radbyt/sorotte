use super::*;

use std::collections::BTreeMap;

use serde_json::Value;
use sorotte_player_api::{
    LifecycleVerificationProjection, LoadAttemptId, PlayerAttachmentEpoch, PlayerCommandId,
    PlayerCommandSemanticResult, PlayerEvent, PlayerEventBatch, PlayerLoadAttemptResult,
    PlayerMediaGeneration, PlayerSemanticOutcome, SequencedPlayerSemanticOutcome, SnapshotField,
};
use sorotte_player_mpv::{
    LifecycleVerificationPlaylistEntry, LifecycleVerificationTrackedLoad,
    MpvLifecycleVerificationHarness,
};

const HISTORY_SEEDS: [u64; 3] = [0x00dd_5eed, 0xc0ff_ee42, 0xdec0_de01];
const PARTITION_SEEDS: [u64; 3] = [0x51a7_e123, 0xa77a_c411, 0x9e37_79b9];
const MAX_DRAIN_BATCHES: usize = 32;

#[derive(Debug, Clone, Default, PartialEq)]
struct DeliveredSemanticLedgers {
    adapter: Vec<SequencedPlayerSemanticOutcome>,
    client: Vec<SequencedPlayerSemanticOutcome>,
    gui: Vec<SequencedPlayerSemanticOutcome>,
}

impl DeliveredSemanticLedgers {
    fn record_new_batch(&mut self, batch: &PlayerEventBatch) {
        self.adapter.extend(batch.semantic_outcomes.clone());
        self.client.extend(batch.semantic_outcomes.clone());
        self.gui.extend(batch.semantic_outcomes.clone());
    }

    fn assert_equal(&self, stage: &str) {
        assert_eq!(
            self.client, self.adapter,
            "{stage}: client semantic ledger diverged from delivered adapter outcomes"
        );
        assert_eq!(
            self.gui, self.adapter,
            "{stage}: GUI semantic ledger diverged from delivered adapter outcomes"
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
struct CrossLayerDigest {
    adapter: LifecycleVerificationProjection,
    client: LifecycleVerificationProjection,
    gui: LifecycleVerificationProjection,
    semantic_outcomes: Vec<SequencedPlayerSemanticOutcome>,
}

#[derive(Debug, Clone)]
struct DeliveryPlan {
    name: String,
    partitions: Vec<usize>,
    delay_acknowledgement: bool,
    repeat_unacknowledged_batch: bool,
}

#[derive(Debug, Clone, Copy)]
struct TraceLoadIdentity {
    media_generation: PlayerMediaGeneration,
}

impl From<LifecycleVerificationTrackedLoad> for TraceLoadIdentity {
    fn from(value: LifecycleVerificationTrackedLoad) -> Self {
        Self {
            media_generation: value.media_generation,
        }
    }
}

#[derive(Debug, Clone)]
struct SnapshotEntry {
    playlist_entry_id: i64,
    target: String,
    current: bool,
}

impl SnapshotEntry {
    fn new(playlist_entry_id: i64, target: impl Into<String>, current: bool) -> Self {
        Self {
            playlist_entry_id,
            target: target.into(),
            current,
        }
    }
}

#[derive(Debug, Clone)]
enum GeneratedAction {
    AcceptTrackedLoad {
        slot: u8,
        target: String,
        baseline_playlist_entry_ids: Vec<i64>,
    },
    AcceptSameGenerationRecovery {
        slot: u8,
        source_slot: u8,
        target: String,
        baseline_playlist_entry_ids: Vec<i64>,
    },
    AuthoritativeSnapshot {
        entries: Vec<SnapshotEntry>,
        current_path: Option<String>,
    },
    RawMpv(Value),
    AdvanceClock(u64),
    EventGap,
    ReplaceAttachment,
}

#[derive(Default)]
struct GeneratedContext {
    loads: BTreeMap<u8, TraceLoadIdentity>,
}

fn apply_batch_once(
    stage: &str,
    batch: &PlayerEventBatch,
    client: &mut VerificationClientRuntime,
    gui: &mut GuiPersistedConfigRuntimeOwner,
) {
    let client_error = client
        .apply_ordered_player_event_batch_for_verification(batch, 0.0)
        .unwrap_or_else(|error| panic!("{stage}: client rejected batch: {error}"));
    assert!(
        client_error.is_none(),
        "{stage}: client batch application returned {client_error:?}"
    );
    gui.apply_ordered_player_event_batch(batch, 0.0)
        .unwrap_or_else(|error| panic!("{stage}: GUI rejected batch: {error}"));
}

fn apply_new_batch(
    stage: &str,
    batch: &PlayerEventBatch,
    client: &mut VerificationClientRuntime,
    gui: &mut GuiPersistedConfigRuntimeOwner,
    ledgers: &mut DeliveredSemanticLedgers,
    repeat: bool,
    harness: &mut MpvLifecycleVerificationHarness,
) {
    apply_batch_once(stage, batch, client, gui);
    ledgers.record_new_batch(batch);

    if !repeat {
        return;
    }

    let client_once = client.lifecycle_verification_projection();
    let gui_once = gui_lifecycle_verification_projection(gui);
    assert_eq!(
        harness.take_event_batch(),
        Some(batch.clone()),
        "{stage}: producer must replay the unacknowledged batch byte-for-byte"
    );
    apply_batch_once(&format!("{stage} replay"), batch, client, gui);
    assert_eq!(
        client.lifecycle_verification_projection(),
        client_once,
        "{stage}: client replay must be idempotent"
    );
    assert_eq!(
        gui_lifecycle_verification_projection(gui),
        gui_once,
        "{stage}: GUI replay must be idempotent"
    );
}

fn replay_applied_batch(
    stage: &str,
    harness: &mut MpvLifecycleVerificationHarness,
    batch: &PlayerEventBatch,
    client: &mut VerificationClientRuntime,
    gui: &mut GuiPersistedConfigRuntimeOwner,
) {
    assert_eq!(
        harness.take_event_batch(),
        Some(batch.clone()),
        "{stage}: delayed acknowledgement must retain the exact batch"
    );
    let client_once = client.lifecycle_verification_projection();
    let gui_once = gui_lifecycle_verification_projection(gui);
    apply_batch_once(stage, batch, client, gui);
    assert_eq!(
        client.lifecycle_verification_projection(),
        client_once,
        "{stage}: delayed client replay must be idempotent"
    );
    assert_eq!(
        gui_lifecycle_verification_projection(gui),
        gui_once,
        "{stage}: delayed GUI replay must be idempotent"
    );
}

fn acknowledge_batch(
    stage: &str,
    harness: &mut MpvLifecycleVerificationHarness,
    batch: &PlayerEventBatch,
    client: &mut VerificationClientRuntime,
    gui: &mut GuiPersistedConfigRuntimeOwner,
) {
    harness
        .acknowledge(batch.acknowledgement_token)
        .unwrap_or_else(|error| panic!("{stage}: adapter acknowledgement failed: {error}"));
    client.compact_acknowledged_player_event_batch_for_verification(
        batch.acknowledgement_token,
        batch.sequence_boundary,
    );
    gui.ordered_player_events
        .compact_acknowledged_delivery(batch.acknowledgement_token, batch.sequence_boundary);
}

fn drain_immediately(
    stage: &str,
    harness: &mut MpvLifecycleVerificationHarness,
    client: &mut VerificationClientRuntime,
    gui: &mut GuiPersistedConfigRuntimeOwner,
    ledgers: &mut DeliveredSemanticLedgers,
) -> usize {
    let mut drained = 0;
    while let Some(batch) = harness.take_event_batch() {
        assert!(
            drained < MAX_DRAIN_BATCHES,
            "{stage}: event delivery did not converge within {MAX_DRAIN_BATCHES} batches"
        );
        apply_new_batch(
            &format!("{stage} batch {drained}"),
            &batch,
            client,
            gui,
            ledgers,
            true,
            harness,
        );
        acknowledge_batch(
            &format!("{stage} batch {drained}"),
            harness,
            &batch,
            client,
            gui,
        );
        drained += 1;
    }
    drained
}

fn normalize_projection(
    mut projection: LifecycleVerificationProjection,
) -> LifecycleVerificationProjection {
    // Observation timestamps are sampled at real ingress time and are not an
    // ownership or semantic fact. Delivery partitioning must not compare them.
    projection.transport.observed_at = SnapshotField::Unavailable;
    projection
}

fn final_cross_layer_digest(
    stage: &str,
    harness: &MpvLifecycleVerificationHarness,
    client: &VerificationClientRuntime,
    gui: &GuiPersistedConfigRuntimeOwner,
    ledgers: &DeliveredSemanticLedgers,
) -> CrossLayerDigest {
    ledgers.assert_equal(stage);
    let adapter = normalize_projection(harness.projection());
    let client = normalize_projection(client.lifecycle_verification_projection());
    let gui = normalize_projection(gui_lifecycle_verification_projection(gui));
    assert_projection_compatible(stage, &adapter, &client);
    assert_projection_compatible(stage, &adapter, &gui);
    assert_projection_compatible(stage, &client, &gui);
    assert_eq!(
        adapter.in_flight_acknowledgement,
        SnapshotField::KnownAbsent,
        "{stage}: adapter must have no unacknowledged batch"
    );
    assert_eq!(
        client.in_flight_acknowledgement,
        SnapshotField::KnownAbsent,
        "{stage}: client must have no unacknowledged batch"
    );
    assert_eq!(
        gui.in_flight_acknowledgement,
        SnapshotField::KnownAbsent,
        "{stage}: GUI must have no unacknowledged batch"
    );
    CrossLayerDigest {
        adapter,
        client,
        gui,
        semantic_outcomes: ledgers.adapter.clone(),
    }
}

#[allow(clippy::too_many_arguments)]
fn establish_loaded_media(
    stage: &str,
    target: &str,
    playlist_entry_id: i64,
    position_seconds: f64,
    harness: &mut MpvLifecycleVerificationHarness,
    client: &mut VerificationClientRuntime,
    gui: &mut GuiPersistedConfigRuntimeOwner,
    ledgers: &mut DeliveredSemanticLedgers,
) -> LifecycleVerificationTrackedLoad {
    let tracked = harness.accept_tracked_load(target, []);
    harness.apply_authoritative_snapshot(
        [LifecycleVerificationPlaylistEntry::new(
            playlist_entry_id,
            Some(target.to_owned()),
            false,
        )],
        None,
    );
    drain_immediately(
        &format!("{stage} accepted and bound"),
        harness,
        client,
        gui,
        ledgers,
    );

    harness.ingest_decoded_mpv_json(json!({
        "event": "start-file",
        "playlist_entry_id": playlist_entry_id,
    }));
    harness.ingest_decoded_mpv_json(json!({
        "event": "property-change",
        "name": "path",
        "data": target,
    }));
    harness.ingest_decoded_mpv_json(json!({ "event": "file-loaded" }));
    harness.ingest_decoded_mpv_json(json!({
        "event": "property-change",
        "name": "time-pos",
        "data": position_seconds,
    }));
    drain_immediately(&format!("{stage} active"), harness, client, gui, ledgers);
    tracked
}

fn terminal_command_count(
    batch: &PlayerEventBatch,
    command_id: PlayerCommandId,
    result: PlayerCommandSemanticResult,
) -> usize {
    batch
        .semantic_outcomes
        .iter()
        .filter(|outcome| {
            matches!(
                outcome.outcome,
                PlayerSemanticOutcome::Command(command)
                    if command.command_id == command_id && command.result == result
            )
        })
        .count()
}

fn terminal_load_count(
    batch: &PlayerEventBatch,
    attempt_id: LoadAttemptId,
    result: PlayerLoadAttemptResult,
) -> usize {
    batch
        .semantic_outcomes
        .iter()
        .filter(|outcome| {
            matches!(
                &outcome.outcome,
                PlayerSemanticOutcome::LoadAttempt(load)
                    if load.attempt_id == attempt_id && load.result == result
            )
        })
        .count()
}

#[test]
fn attachment_replacement_delivers_old_terminal_handoff_before_reused_playlist_entry() {
    const OLD_TARGET: &str = "C:/verification/core-one.mkv";
    const PENDING_TARGET: &str = "https://media.example/pending-core-one";
    const NEW_TARGET: &str = "C:/verification/core-two.mkv";
    const REUSED_PLAYLIST_ENTRY_ID: i64 = 1;

    let mut harness = MpvLifecycleVerificationHarness::new();
    let mut client = verification_client_runtime();
    let mut gui = GuiPersistedConfigRuntimeOwner::default();
    let mut ledgers = DeliveredSemanticLedgers::default();
    let old = establish_loaded_media(
        "attachment old core",
        OLD_TARGET,
        REUSED_PLAYLIST_ENTRY_ID,
        44.0,
        &mut harness,
        &mut client,
        &mut gui,
        &mut ledgers,
    );
    assert_eq!(gui.player_position_seconds, Some(44.0));

    let pending = harness.accept_tracked_load(PENDING_TARGET, [REUSED_PLAYLIST_ENTRY_ID]);
    drain_immediately(
        "attachment pending old command",
        &mut harness,
        &mut client,
        &mut gui,
        &mut ledgers,
    );

    harness.replace_attachment();
    harness.ingest_decoded_mpv_json(json!({
        "event": "start-file",
        "playlist_entry_id": REUSED_PLAYLIST_ENTRY_ID,
    }));
    harness.apply_authoritative_snapshot(
        [LifecycleVerificationPlaylistEntry::new(
            REUSED_PLAYLIST_ENTRY_ID,
            Some(NEW_TARGET.to_owned()),
            true,
        )],
        Some(NEW_TARGET.to_owned()),
    );
    harness.ingest_decoded_mpv_json(json!({
        "event": "property-change",
        "name": "path",
        "data": NEW_TARGET,
    }));
    harness.ingest_decoded_mpv_json(json!({ "event": "file-loaded" }));
    harness.ingest_decoded_mpv_json(json!({
        "event": "property-change",
        "name": "time-pos",
        "data": 3.0,
    }));

    let new_attempt = match harness.projection().physical_transport_owner {
        SnapshotField::Known(attempt_id) => attempt_id,
        other => panic!("new core should own transport before delivery, got {other:?}"),
    };
    assert_ne!(new_attempt, old.attempt_id);
    assert_ne!(new_attempt, pending.attempt_id);

    let old_handoff = harness
        .take_event_batch()
        .expect("old attachment terminal handoff");
    assert_eq!(
        old_handoff.attachment_epoch,
        PlayerAttachmentEpoch::new(1),
        "the old attachment must be delivered before epoch 2 becomes visible"
    );
    assert_eq!(
        terminal_command_count(
            &old_handoff,
            pending.command_id,
            PlayerCommandSemanticResult::TransportDisconnected,
        ),
        1,
        "the accepted old command must disconnect exactly once"
    );
    assert_eq!(
        terminal_load_count(
            &old_handoff,
            pending.attempt_id,
            PlayerLoadAttemptResult::TransportDisconnected,
        ),
        1,
        "the accepted old load must disconnect exactly once"
    );
    assert!(
        old_handoff
            .events
            .iter()
            .all(|event| event.order.attachment_epoch == PlayerAttachmentEpoch::new(1)),
        "the old handoff must not mix new-epoch events"
    );
    assert!(
        old_handoff
            .semantic_outcomes
            .iter()
            .all(|outcome| { outcome.order.attachment_epoch == PlayerAttachmentEpoch::new(1) }),
        "the old handoff must not mix new-epoch outcomes"
    );

    apply_new_batch(
        "old attachment handoff",
        &old_handoff,
        &mut client,
        &mut gui,
        &mut ledgers,
        true,
        &mut harness,
    );
    acknowledge_batch(
        "old attachment handoff",
        &mut harness,
        &old_handoff,
        &mut client,
        &mut gui,
    );
    assert_eq!(
        client.lifecycle_verification_projection().attachment_epoch,
        SnapshotField::Known(PlayerAttachmentEpoch::new(1)),
        "client must acknowledge the old handoff before observing epoch 2"
    );
    assert_eq!(
        gui_lifecycle_verification_projection(&gui).attachment_epoch,
        SnapshotField::Known(PlayerAttachmentEpoch::new(1)),
        "GUI must acknowledge the old handoff before observing epoch 2"
    );

    let mut new_epoch_batches = Vec::new();
    let mut drained = 0;
    while let Some(batch) = harness.take_event_batch() {
        assert!(
            drained < MAX_DRAIN_BATCHES,
            "attachment replacement did not converge"
        );
        assert_eq!(
            batch.attachment_epoch,
            PlayerAttachmentEpoch::new(2),
            "only epoch 2 may follow the old terminal handoff"
        );
        assert!(
            batch
                .events
                .iter()
                .all(|event| event.order.attachment_epoch == PlayerAttachmentEpoch::new(2))
        );
        assert!(
            batch
                .semantic_outcomes
                .iter()
                .all(|outcome| { outcome.order.attachment_epoch == PlayerAttachmentEpoch::new(2) })
        );
        apply_new_batch(
            &format!("new attachment batch {drained}"),
            &batch,
            &mut client,
            &mut gui,
            &mut ledgers,
            true,
            &mut harness,
        );
        acknowledge_batch(
            &format!("new attachment batch {drained}"),
            &mut harness,
            &batch,
            &mut client,
            &mut gui,
        );
        new_epoch_batches.push(batch);
        drained += 1;
    }
    assert!(
        !new_epoch_batches.is_empty(),
        "replacement must expose a new-epoch batch"
    );
    assert!(
        new_epoch_batches.iter().any(|batch| {
            batch.events.iter().any(|event| {
                matches!(
                    event.event,
                    PlayerEvent::AttachmentReplaced { previous_epoch }
                        if previous_epoch == PlayerAttachmentEpoch::new(1)
                )
            }) || batch
                .authoritative_snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.attachment_epoch == PlayerAttachmentEpoch::new(2))
        }),
        "epoch 2 must begin with its replacement event or an authoritative replacement snapshot"
    );

    let final_digest = final_cross_layer_digest(
        "attachment replacement final",
        &harness,
        &client,
        &gui,
        &ledgers,
    );
    for projection in [
        &final_digest.adapter,
        &final_digest.client,
        &final_digest.gui,
    ] {
        assert_eq!(
            projection.attachment_epoch,
            SnapshotField::Known(PlayerAttachmentEpoch::new(2))
        );
        assert_eq!(
            projection.physical_transport_owner,
            SnapshotField::Known(new_attempt)
        );
        assert_eq!(
            projection.physical_playlist_entry_id,
            SnapshotField::Known(REUSED_PLAYLIST_ENTRY_ID)
        );
        assert!(
            !projection.attempts.contains_key(&old.attempt_id),
            "old active attempt leaked into epoch 2"
        );
        assert!(
            !projection.attempts.contains_key(&pending.attempt_id),
            "old pending attempt leaked into epoch 2"
        );
    }
    assert_eq!(
        final_digest.adapter.physical_path,
        SnapshotField::Known(NEW_TARGET.to_owned())
    );
    assert_eq!(gui.player_position_seconds, Some(3.0));
    let gui_path = gui
        .player_local_file
        .as_ref()
        .map(|file| file.path.as_deref().unwrap_or(file.name.as_str()));
    assert_ne!(
        gui_path,
        Some(OLD_TARGET),
        "the old core path must not leak into the replacement GUI projection"
    );
    if let Some(gui_path) = gui_path {
        assert_eq!(
            gui_path, NEW_TARGET,
            "any available GUI path must belong to the replacement core"
        );
    }
    assert_eq!(
        final_digest.client.transport.position_seconds,
        SnapshotField::Known(3.0)
    );
    assert_eq!(
        final_digest.gui.transport.position_seconds,
        SnapshotField::Known(3.0)
    );
    assert_eq!(
        final_digest
            .semantic_outcomes
            .iter()
            .filter(|outcome| {
                matches!(
                    outcome.outcome,
                    PlayerSemanticOutcome::Command(command)
                        if command.command_id == pending.command_id
                            && command.result
                                == PlayerCommandSemanticResult::TransportDisconnected
                )
            })
            .count(),
        1,
        "replayed old handoff must be recorded exactly once"
    );
}

#[test]
fn gap_snapshot_retains_indeterminate_until_ack_then_correlates_late_physical_effect() {
    const TARGET: &str = "https://media.example/gap-late-effect.mkv";
    const PLAYLIST_ENTRY_ID: i64 = 71;

    let mut harness = MpvLifecycleVerificationHarness::new();
    let mut client = verification_client_runtime();
    let mut gui = GuiPersistedConfigRuntimeOwner::default();
    let mut ledgers = DeliveredSemanticLedgers::default();
    let tracked = harness.accept_tracked_load(TARGET, []);
    harness.apply_authoritative_snapshot(
        [LifecycleVerificationPlaylistEntry::new(
            PLAYLIST_ENTRY_ID,
            Some(TARGET.to_owned()),
            false,
        )],
        None,
    );
    drain_immediately(
        "gap setup",
        &mut harness,
        &mut client,
        &mut gui,
        &mut ledgers,
    );

    // The adapter ingress clock may have advanced while the accepted/bound
    // setup batches were delivered. Advance beyond the full semantic window
    // from any such deterministic setup tick.
    harness.advance_clock(120_000);
    harness.detect_event_gap();
    harness.apply_authoritative_snapshot(
        [LifecycleVerificationPlaylistEntry::new(
            PLAYLIST_ENTRY_ID,
            Some(TARGET.to_owned()),
            true,
        )],
        Some(TARGET.to_owned()),
    );
    let recovery_batch = harness
        .take_event_batch()
        .expect("timeout and event-gap recovery batch");
    assert!(
        recovery_batch.authoritative_snapshot.is_some(),
        "event gap must be covered by an authoritative snapshot"
    );
    assert_eq!(
        harness.projection().snapshot_required,
        SnapshotField::Known(false),
        "the authoritative snapshot must close the producer's event-gap latch"
    );
    assert_eq!(
        terminal_load_count(
            &recovery_batch,
            tracked.attempt_id,
            PlayerLoadAttemptResult::Indeterminate,
        ),
        1,
        "the semantic timeout must remain in the unacknowledged recovery batch"
    );
    apply_new_batch(
        "gap recovery unacknowledged",
        &recovery_batch,
        &mut client,
        &mut gui,
        &mut ledgers,
        false,
        &mut harness,
    );

    harness.ingest_decoded_mpv_json(json!({
        "event": "start-file",
        "playlist_entry_id": PLAYLIST_ENTRY_ID,
    }));
    harness.ingest_decoded_mpv_json(json!({
        "event": "property-change",
        "name": "paused-for-cache",
        "data": true,
    }));
    harness.ingest_decoded_mpv_json(json!({
        "event": "property-change",
        "name": "cache-buffering-state",
        "data": 37.5,
    }));
    harness.ingest_decoded_mpv_json(json!({
        "event": "property-change",
        "name": "path",
        "data": TARGET,
    }));
    harness.ingest_decoded_mpv_json(json!({ "event": "file-loaded" }));

    replay_applied_batch(
        "gap recovery delayed acknowledgement",
        &mut harness,
        &recovery_batch,
        &mut client,
        &mut gui,
    );
    assert_eq!(
        harness.projection().attempts[&tracked.attempt_id].semantic_load_result,
        SnapshotField::Known(Some(PlayerLoadAttemptResult::Indeterminate)),
        "late physical evidence must not replace the retained semantic timeout"
    );
    acknowledge_batch(
        "gap recovery delayed acknowledgement",
        &mut harness,
        &recovery_batch,
        &mut client,
        &mut gui,
    );

    let late_batch = harness
        .take_event_batch()
        .expect("late physical effect batch");
    assert!(late_batch.events.iter().any(|event| {
        matches!(
            event.event,
            PlayerEvent::LoadAttemptActive { attempt_id, .. }
                if attempt_id == tracked.attempt_id
        )
    }));
    assert!(
        late_batch.semantic_outcomes.iter().all(|outcome| {
            !matches!(
                &outcome.outcome,
                PlayerSemanticOutcome::LoadAttempt(load)
                    if load.attempt_id == tracked.attempt_id
                        && load.result == PlayerLoadAttemptResult::Loaded
            )
        }),
        "late activation must not invent a second semantic outcome"
    );
    apply_new_batch(
        "gap late physical effect",
        &late_batch,
        &mut client,
        &mut gui,
        &mut ledgers,
        true,
        &mut harness,
    );
    acknowledge_batch(
        "gap late physical effect",
        &mut harness,
        &late_batch,
        &mut client,
        &mut gui,
    );
    let additional_batches = drain_immediately(
        "gap finite convergence",
        &mut harness,
        &mut client,
        &mut gui,
        &mut ledgers,
    );
    assert!(
        additional_batches < MAX_DRAIN_BATCHES,
        "gap recovery must converge in finite batches"
    );

    let final_digest =
        final_cross_layer_digest("gap recovery final", &harness, &client, &gui, &ledgers);
    for projection in [
        &final_digest.adapter,
        &final_digest.client,
        &final_digest.gui,
    ] {
        assert_eq!(
            projection.physical_transport_owner,
            SnapshotField::Known(tracked.attempt_id),
            "physical ownership must survive acknowledgement compaction"
        );
        assert_eq!(
            projection.attempts[&tracked.attempt_id].semantic_load_result,
            SnapshotField::Known(Some(PlayerLoadAttemptResult::Indeterminate))
        );
        assert_eq!(
            projection.attempts.len(),
            1,
            "late Sorotte-owned load effects must not be reclassified as an external attempt"
        );
    }
    assert_eq!(
        final_digest
            .semantic_outcomes
            .iter()
            .filter(|outcome| {
                matches!(
                    &outcome.outcome,
                    PlayerSemanticOutcome::LoadAttempt(load)
                        if load.attempt_id == tracked.attempt_id
                            && load.result == PlayerLoadAttemptResult::Indeterminate
                )
            })
            .count(),
        1
    );
    assert_eq!(
        final_digest
            .semantic_outcomes
            .iter()
            .filter(|outcome| {
                matches!(
                    &outcome.outcome,
                    PlayerSemanticOutcome::LoadAttempt(load)
                        if load.attempt_id == tracked.attempt_id
                            && load.result == PlayerLoadAttemptResult::Loaded
                )
            })
            .count(),
        0
    );
}

fn apply_generated_action(
    action: &GeneratedAction,
    harness: &mut MpvLifecycleVerificationHarness,
    context: &mut GeneratedContext,
) {
    match action {
        GeneratedAction::AcceptTrackedLoad {
            slot,
            target,
            baseline_playlist_entry_ids,
        } => {
            let tracked =
                harness.accept_tracked_load(target.clone(), baseline_playlist_entry_ids.clone());
            assert!(
                context.loads.insert(*slot, tracked.into()).is_none(),
                "generated load slot {slot} was reused"
            );
        }
        GeneratedAction::AcceptSameGenerationRecovery {
            slot,
            source_slot,
            target,
            baseline_playlist_entry_ids,
        } => {
            let source = context.loads[source_slot];
            let _attempt_id = harness.accept_same_generation_recovery(
                source.media_generation,
                target.clone(),
                baseline_playlist_entry_ids.clone(),
            );
            assert!(
                context
                    .loads
                    .insert(
                        *slot,
                        TraceLoadIdentity {
                            media_generation: source.media_generation,
                        },
                    )
                    .is_none(),
                "generated recovery slot {slot} was reused"
            );
        }
        GeneratedAction::AuthoritativeSnapshot {
            entries,
            current_path,
        } => {
            harness.apply_authoritative_snapshot(
                entries.iter().map(|entry| {
                    LifecycleVerificationPlaylistEntry::new(
                        entry.playlist_entry_id,
                        Some(entry.target.clone()),
                        entry.current,
                    )
                }),
                current_path.clone(),
            );
        }
        GeneratedAction::RawMpv(value) => {
            harness.ingest_decoded_mpv_json(value.clone());
        }
        GeneratedAction::AdvanceClock(ticks) => harness.advance_clock(*ticks),
        GeneratedAction::EventGap => harness.detect_event_gap(),
        GeneratedAction::ReplaceAttachment => harness.replace_attachment(),
    }
}

fn random_partitions(length: usize, seed: u64) -> Vec<usize> {
    let mut random = seed;
    let mut remaining = length;
    let mut partitions = Vec::new();
    while remaining > 0 {
        random = random
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let width = ((random >> 32) as usize % 7 + 1).min(remaining);
        partitions.push(width);
        remaining -= width;
    }
    partitions
}

fn fixed_partitions(length: usize) -> Vec<usize> {
    const WIDTHS: [usize; 5] = [2, 5, 1, 4, 3];
    let mut remaining = length;
    let mut partitions = Vec::new();
    let mut index = 0;
    while remaining > 0 {
        let width = WIDTHS[index % WIDTHS.len()].min(remaining);
        partitions.push(width);
        remaining -= width;
        index += 1;
    }
    partitions
}

fn generated_history(seed: u64) -> Vec<GeneratedAction> {
    let stream = format!("https://media.example/generated-stream-{seed:016x}");
    let late = format!("https://media.example/generated-late-{seed:016x}");
    let pending = format!("https://media.example/generated-pending-{seed:016x}");
    let external = format!("C:/verification/generated-external-{seed:016x}.mkv");
    let old_terminal_reason = if seed & 1 == 0 { "stop" } else { "error" };
    let cache_percentage = 20.0 + (seed % 61) as f64;
    let duplicate_file_loaded_count = usize::from(seed & 2 != 0) + 1;
    let mut history = vec![
        GeneratedAction::AcceptTrackedLoad {
            slot: 0,
            target: stream.clone(),
            baseline_playlist_entry_ids: Vec::new(),
        },
        GeneratedAction::AuthoritativeSnapshot {
            entries: vec![SnapshotEntry::new(10, stream.clone(), false)],
            current_path: None,
        },
        GeneratedAction::RawMpv(json!({
            "event": "start-file",
            "playlist_entry_id": 10,
        })),
        GeneratedAction::RawMpv(json!({
            "event": "property-change",
            "name": "path",
            "data": stream,
        })),
        GeneratedAction::RawMpv(json!({ "event": "file-loaded" })),
        GeneratedAction::RawMpv(json!({
            "event": "property-change",
            "name": "paused-for-cache",
            "data": true,
        })),
        GeneratedAction::RawMpv(json!({
            "event": "property-change",
            "name": "cache-buffering-state",
            "data": cache_percentage,
        })),
        GeneratedAction::AcceptSameGenerationRecovery {
            slot: 1,
            source_slot: 0,
            target: stream.clone(),
            baseline_playlist_entry_ids: vec![10],
        },
        GeneratedAction::AuthoritativeSnapshot {
            entries: vec![
                SnapshotEntry::new(10, stream.clone(), true),
                SnapshotEntry::new(20, stream.clone(), false),
            ],
            current_path: Some(stream.clone()),
        },
        GeneratedAction::RawMpv(json!({
            "event": "start-file",
            "playlist_entry_id": 20,
        })),
        GeneratedAction::RawMpv(json!({
            "event": "property-change",
            "name": "paused-for-cache",
            "data": true,
        })),
        GeneratedAction::RawMpv(json!({
            "event": "property-change",
            "name": "cache-buffering-state",
            "data": cache_percentage / 2.0,
        })),
        GeneratedAction::RawMpv(json!({
            "event": "end-file",
            "playlist_entry_id": 10,
            "reason": old_terminal_reason,
        })),
        GeneratedAction::RawMpv(json!({
            "event": "property-change",
            "name": "path",
            "data": stream,
        })),
        GeneratedAction::RawMpv(json!({ "event": "file-loaded" })),
        GeneratedAction::AcceptTrackedLoad {
            slot: 2,
            target: late.clone(),
            baseline_playlist_entry_ids: vec![10, 20],
        },
        GeneratedAction::AuthoritativeSnapshot {
            entries: vec![
                SnapshotEntry::new(20, stream.clone(), true),
                SnapshotEntry::new(30, late.clone(), false),
            ],
            current_path: Some(stream.clone()),
        },
        GeneratedAction::AdvanceClock(120_000),
        GeneratedAction::EventGap,
        GeneratedAction::AuthoritativeSnapshot {
            entries: vec![SnapshotEntry::new(30, late.clone(), true)],
            current_path: Some(late.clone()),
        },
        GeneratedAction::RawMpv(json!({
            "event": "start-file",
            "playlist_entry_id": 30,
        })),
        GeneratedAction::RawMpv(json!({
            "event": "property-change",
            "name": "path",
            "data": late,
        })),
    ];
    history.extend(
        std::iter::repeat_with(|| GeneratedAction::RawMpv(json!({ "event": "file-loaded" })))
            .take(duplicate_file_loaded_count),
    );
    history.extend([
        GeneratedAction::RawMpv(json!({
            "event": "end-file",
            "playlist_entry_id": 30,
            "reason": "stop",
        })),
        GeneratedAction::RawMpv(json!({
            "event": "end-file",
            "playlist_entry_id": 30,
            "reason": "stop",
        })),
        GeneratedAction::AcceptTrackedLoad {
            slot: 3,
            target: pending,
            baseline_playlist_entry_ids: vec![30],
        },
        GeneratedAction::ReplaceAttachment,
        GeneratedAction::RawMpv(json!({
            "event": "start-file",
            "playlist_entry_id": 1,
        })),
        GeneratedAction::AuthoritativeSnapshot {
            entries: vec![SnapshotEntry::new(1, external.clone(), true)],
            current_path: Some(external.clone()),
        },
        GeneratedAction::RawMpv(json!({
            "event": "property-change",
            "name": "path",
            "data": external,
        })),
        GeneratedAction::RawMpv(json!({ "event": "file-loaded" })),
        GeneratedAction::RawMpv(json!({
            "event": "property-change",
            "name": "time-pos",
            "data": 3.0 + (seed % 7) as f64,
        })),
    ]);
    history
}

fn run_generated_history(
    history_seed: u64,
    history: &[GeneratedAction],
    plan: &DeliveryPlan,
) -> CrossLayerDigest {
    assert_eq!(
        plan.partitions.iter().sum::<usize>(),
        history.len(),
        "{} does not cover generated history {history_seed:#x}",
        plan.name
    );

    let mut harness = MpvLifecycleVerificationHarness::new();
    let mut client = verification_client_runtime();
    let mut gui = GuiPersistedConfigRuntimeOwner::default();
    let mut context = GeneratedContext::default();
    let mut ledgers = DeliveredSemanticLedgers::default();
    let mut pending_batch: Option<PlayerEventBatch> = None;
    let mut cursor = 0;

    for (pump_index, partition) in plan.partitions.iter().copied().enumerate() {
        for action in &history[cursor..cursor + partition] {
            apply_generated_action(action, &mut harness, &mut context);
        }
        cursor += partition;
        let stage = format!("history {history_seed:#x} {} pump {pump_index}", plan.name);

        if let Some(batch) = pending_batch.take() {
            replay_applied_batch(
                &format!("{stage} delayed replay"),
                &mut harness,
                &batch,
                &mut client,
                &mut gui,
            );
            acknowledge_batch(
                &format!("{stage} delayed acknowledgement"),
                &mut harness,
                &batch,
                &mut client,
                &mut gui,
            );
        }

        if let Some(batch) = harness.take_event_batch() {
            apply_new_batch(
                &stage,
                &batch,
                &mut client,
                &mut gui,
                &mut ledgers,
                plan.repeat_unacknowledged_batch,
                &mut harness,
            );
            if plan.delay_acknowledgement {
                pending_batch = Some(batch);
            } else {
                acknowledge_batch(
                    &format!("{stage} acknowledgement"),
                    &mut harness,
                    &batch,
                    &mut client,
                    &mut gui,
                );
            }
        }
    }
    assert_eq!(cursor, history.len());

    if let Some(batch) = pending_batch {
        replay_applied_batch(
            &format!("history {history_seed:#x} {} final replay", plan.name),
            &mut harness,
            &batch,
            &mut client,
            &mut gui,
        );
        acknowledge_batch(
            &format!(
                "history {history_seed:#x} {} final acknowledgement",
                plan.name
            ),
            &mut harness,
            &batch,
            &mut client,
            &mut gui,
        );
    }
    let drained = drain_immediately(
        &format!("history {history_seed:#x} {} final drain", plan.name),
        &mut harness,
        &mut client,
        &mut gui,
        &mut ledgers,
    );
    assert!(
        drained < MAX_DRAIN_BATCHES,
        "history {history_seed:#x} {} failed finite convergence",
        plan.name
    );
    assert_eq!(harness.take_event_batch(), None);

    final_cross_layer_digest(
        &format!("history {history_seed:#x} {} final", plan.name),
        &harness,
        &client,
        &gui,
        &ledgers,
    )
}

#[test]
fn generated_lifecycle_histories_converge_across_delivery_plans() {
    for history_seed in HISTORY_SEEDS {
        let history = generated_history(history_seed);
        let baseline_plan = DeliveryPlan {
            name: "one event per pump".to_owned(),
            partitions: vec![1; history.len()],
            delay_acknowledgement: false,
            repeat_unacknowledged_batch: false,
        };
        let expected = run_generated_history(history_seed, &history, &baseline_plan);

        let mut plans = vec![
            DeliveryPlan {
                name: "all events in one pump".to_owned(),
                partitions: vec![history.len()],
                delay_acknowledgement: false,
                repeat_unacknowledged_batch: true,
            },
            DeliveryPlan {
                name: "fixed partitions with delayed acknowledgement".to_owned(),
                partitions: fixed_partitions(history.len()),
                delay_acknowledgement: true,
                repeat_unacknowledged_batch: true,
            },
        ];
        plans.extend(
            PARTITION_SEEDS
                .into_iter()
                .map(|partition_seed| DeliveryPlan {
                    name: format!(
                        "random partitions seed {partition_seed:#x} with delayed acknowledgement"
                    ),
                    partitions: random_partitions(history.len(), partition_seed),
                    delay_acknowledgement: true,
                    repeat_unacknowledged_batch: true,
                }),
        );

        for plan in plans {
            let actual = run_generated_history(history_seed, &history, &plan);
            assert_eq!(
                actual, expected,
                "generated lifecycle history {history_seed:#x} diverged under {}",
                plan.name
            );
        }
    }
}
