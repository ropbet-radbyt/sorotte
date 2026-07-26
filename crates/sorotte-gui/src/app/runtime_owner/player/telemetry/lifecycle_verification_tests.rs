use super::*;

use std::fmt::Debug;

use serde_json::json;
use sorotte_client_core::{ClientRuntime, ClientSession, QueuedRuntimeControl};
use sorotte_player_api::{
    LifecycleVerificationAttemptProjection, LifecycleVerificationProjection, PlayerCommandId,
    PlayerEventBatch, PlayerMediaGeneration, PlayerSequenceBoundary, PlayerTransportPhase,
    SnapshotField,
};
use sorotte_player_mpv::{
    LifecycleVerificationPlaylistEntry, MpvLifecycleVerificationHarness, SimulatedPlayer,
};

mod trace_load_matrix;
mod trace_resolution;

type VerificationClientRuntime = ClientRuntime<SimulatedPlayer, QueuedRuntimeControl>;

fn verification_optional_field<T>(value: Option<T>) -> SnapshotField<T> {
    value.map_or(SnapshotField::KnownAbsent, SnapshotField::Known)
}

fn gui_lifecycle_verification_projection(
    owner: &GuiPersistedConfigRuntimeOwner,
) -> LifecycleVerificationProjection {
    let consumer = &owner.ordered_player_events;
    let physical_binding = consumer
        .transport_owner_attempt
        .and_then(|attempt_id| consumer.attempts.get(&attempt_id));
    let attempts: std::collections::BTreeMap<_, _> = consumer
        .attempts
        .iter()
        .map(|(attempt_id, binding)| {
            (
                *attempt_id,
                LifecycleVerificationAttemptProjection {
                    media_generation: binding.media_generation,
                    command_id: binding.command_id,
                    playlist_entry_id: binding.playlist_entry_id,
                    owns_transport: SnapshotField::Known(binding.owns_transport),
                    semantic_load_result: SnapshotField::Known(binding.semantic_load_result),
                    logical_ownership_revoked: SnapshotField::Known(
                        binding.logical_ownership_revoked,
                    ),
                    physical_terminal: SnapshotField::Known(binding.physical_terminal),
                },
            )
        })
        .collect();
    let terminal_load_results = attempts
        .iter()
        .filter_map(|(attempt_id, attempt)| match attempt.semantic_load_result {
            SnapshotField::Known(Some(result)) => Some((*attempt_id, result)),
            SnapshotField::Known(None)
            | SnapshotField::KnownAbsent
            | SnapshotField::Unavailable => None,
        })
        .collect();
    let playlist_attempt = owner.playlist_resolution_attempt.as_ref();

    LifecycleVerificationProjection {
        attachment_epoch: verification_optional_field(consumer.attachment_epoch),
        sequence_boundary: consumer.attachment_epoch.map_or(
            SnapshotField::KnownAbsent,
            |attachment_epoch| {
                SnapshotField::Known(PlayerSequenceBoundary::new(
                    attachment_epoch,
                    consumer.last_sequence,
                ))
            },
        ),
        in_flight_acknowledgement: verification_optional_field(
            consumer.applied_unacknowledged_token,
        ),
        pending_event_count: SnapshotField::Unavailable,
        retained_semantic_outcome_count: SnapshotField::Known(
            consumer.applied_semantic_outcomes.len(),
        ),
        snapshot_required: SnapshotField::Unavailable,

        physical_transport_owner: verification_optional_field(consumer.transport_owner_attempt),
        physical_media_generation: verification_optional_field(
            physical_binding.map(|binding| binding.media_generation),
        ),
        physical_playlist_entry_id: physical_binding
            .map_or(SnapshotField::KnownAbsent, |binding| {
                verification_optional_field(binding.playlist_entry_id)
            }),
        // The GUI may rewrite a provider URL back to a logical playlist
        // identity, so it cannot claim the adapter's physical path.
        physical_path: SnapshotField::Unavailable,
        // The GUI consumer retains semantic completion, not the adapter's
        // physical file-loaded evidence.
        physical_file_loaded: SnapshotField::Unavailable,
        logical_owner: SnapshotField::Unavailable,

        transport: consumer.transport.clone(),
        attempts,
        pending_commands: SnapshotField::Unavailable,
        terminal_command_results: SnapshotField::Unavailable,
        terminal_load_results: SnapshotField::Known(terminal_load_results),

        pending_playlist_resolution_attempt: playlist_attempt
            .map_or(SnapshotField::KnownAbsent, |attempt| {
                verification_optional_field(attempt.load_attempt_id)
            }),
        playlist_resolution_state: playlist_attempt.map_or(SnapshotField::KnownAbsent, |attempt| {
            SnapshotField::Known(format!("{:?}", attempt.state))
        }),
        fallback_pending: playlist_attempt.map_or(SnapshotField::KnownAbsent, |attempt| {
            SnapshotField::Known(attempt.fallback_pending)
        }),
        player_local_file: verification_optional_field(
            owner
                .player_local_file
                .as_ref()
                .map(|file| file.name.clone()),
        ),
        player_local_file_placeholder: SnapshotField::Known(owner.player_local_file_placeholder),
    }
}

fn seed_gui_playlist_resolution_attempt(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    target: &str,
    command_id: PlayerCommandId,
    media_generation: PlayerMediaGeneration,
) {
    const PLAYLIST_GENERATION: u64 = 1;

    owner.playlist_resolution.generation = PLAYLIST_GENERATION;
    owner.ensure_playlist_resolution_attempt(
        GuiPlaylistEntryId::next(),
        PLAYLIST_GENERATION,
        target,
        GuiPlaylistSourcePolicy::Automatic,
    );
    let attempt = owner
        .playlist_resolution_attempt
        .as_mut()
        .expect("verification playlist attempt");
    attempt.player_command_id = Some(command_id);
    attempt.player_media_generation = Some(media_generation);
    attempt.state = PlaylistResolutionAttemptState::Loading;
}

fn verification_client_runtime() -> VerificationClientRuntime {
    ClientRuntime::new(
        ClientSession::default(),
        SimulatedPlayer::new(),
        QueuedRuntimeControl::default(),
    )
}

fn assert_snapshot_field_compatible<T>(
    stage: &str,
    field: &str,
    expected: &SnapshotField<T>,
    actual: &SnapshotField<T>,
) where
    T: Debug + PartialEq,
{
    if matches!(expected, SnapshotField::Unavailable)
        || matches!(actual, SnapshotField::Unavailable)
    {
        return;
    }
    assert_eq!(actual, expected, "{stage}: {field}");
}

fn assert_projection_compatible(
    stage: &str,
    expected: &LifecycleVerificationProjection,
    actual: &LifecycleVerificationProjection,
) {
    assert_snapshot_field_compatible(
        stage,
        "attachment epoch",
        &expected.attachment_epoch,
        &actual.attachment_epoch,
    );
    assert_snapshot_field_compatible(
        stage,
        "sequence boundary",
        &expected.sequence_boundary,
        &actual.sequence_boundary,
    );
    assert_snapshot_field_compatible(
        stage,
        "in-flight acknowledgement",
        &expected.in_flight_acknowledgement,
        &actual.in_flight_acknowledgement,
    );
    assert_snapshot_field_compatible(
        stage,
        "retained semantic outcomes",
        &expected.retained_semantic_outcome_count,
        &actual.retained_semantic_outcome_count,
    );
    assert_snapshot_field_compatible(
        stage,
        "physical transport owner",
        &expected.physical_transport_owner,
        &actual.physical_transport_owner,
    );
    assert_snapshot_field_compatible(
        stage,
        "physical media generation",
        &expected.physical_media_generation,
        &actual.physical_media_generation,
    );
    assert_snapshot_field_compatible(
        stage,
        "physical playlist entry",
        &expected.physical_playlist_entry_id,
        &actual.physical_playlist_entry_id,
    );
    assert_snapshot_field_compatible(
        stage,
        "physical path",
        &expected.physical_path,
        &actual.physical_path,
    );
    assert_snapshot_field_compatible(
        stage,
        "physical file-loaded",
        &expected.physical_file_loaded,
        &actual.physical_file_loaded,
    );
    assert_snapshot_field_compatible(
        stage,
        "logical owner",
        &expected.logical_owner,
        &actual.logical_owner,
    );
    assert_snapshot_field_compatible(
        stage,
        "transport attempt",
        &expected.transport.load_attempt_id,
        &actual.transport.load_attempt_id,
    );
    assert_snapshot_field_compatible(
        stage,
        "transport generation",
        &expected.transport.media_generation,
        &actual.transport.media_generation,
    );
    assert_snapshot_field_compatible(
        stage,
        "transport phase",
        &expected.transport.phase,
        &actual.transport.phase,
    );
    assert_snapshot_field_compatible(
        stage,
        "transport paused-for-cache",
        &expected.transport.paused_for_cache,
        &actual.transport.paused_for_cache,
    );
    assert_snapshot_field_compatible(
        stage,
        "transport cache percentage",
        &expected.transport.cache_percentage,
        &actual.transport.cache_percentage,
    );
    assert_snapshot_field_compatible(
        stage,
        "transport position",
        &expected.transport.position_seconds,
        &actual.transport.position_seconds,
    );
    assert_snapshot_field_compatible(
        stage,
        "transport logical pause",
        &expected.transport.logical_pause,
        &actual.transport.logical_pause,
    );
    assert_snapshot_field_compatible(
        stage,
        "transport seeking",
        &expected.transport.seeking,
        &actual.transport.seeking,
    );
    assert_snapshot_field_compatible(
        stage,
        "transport eof",
        &expected.transport.eof_reached,
        &actual.transport.eof_reached,
    );

    if let SnapshotField::Known(actual_owner) = actual.physical_transport_owner {
        assert!(
            actual.attempts.contains_key(&actual_owner),
            "{stage}: physical owner {actual_owner:?} has no attempt binding"
        );
    }

    if let (SnapshotField::Known(expected_results), SnapshotField::Known(actual_results)) = (
        &expected.terminal_load_results,
        &actual.terminal_load_results,
    ) {
        for (attempt_id, result) in actual_results {
            assert_eq!(
                expected_results.get(attempt_id),
                Some(result),
                "{stage}: retained semantic result for {attempt_id:?}"
            );
        }
    }

    for (attempt_id, actual_attempt) in &actual.attempts {
        let expected_attempt = expected.attempts.get(attempt_id).unwrap_or_else(|| {
            panic!("{stage}: consumer projected unknown attempt {attempt_id:?}")
        });
        assert_eq!(
            actual_attempt.media_generation, expected_attempt.media_generation,
            "{stage}: attempt generation for {attempt_id:?}"
        );
        assert_eq!(
            actual_attempt.command_id, expected_attempt.command_id,
            "{stage}: attempt command for {attempt_id:?}"
        );
        assert_eq!(
            actual_attempt.playlist_entry_id, expected_attempt.playlist_entry_id,
            "{stage}: attempt playlist entry for {attempt_id:?}"
        );
        assert_snapshot_field_compatible(
            stage,
            "attempt transport ownership",
            &expected_attempt.owns_transport,
            &actual_attempt.owns_transport,
        );
        assert_snapshot_field_compatible(
            stage,
            "attempt semantic result",
            &expected_attempt.semantic_load_result,
            &actual_attempt.semantic_load_result,
        );
        assert_snapshot_field_compatible(
            stage,
            "attempt logical revocation",
            &expected_attempt.logical_ownership_revoked,
            &actual_attempt.logical_ownership_revoked,
        );
        assert_snapshot_field_compatible(
            stage,
            "attempt physical terminal",
            &expected_attempt.physical_terminal,
            &actual_attempt.physical_terminal,
        );
    }
}

fn apply_replay_and_acknowledge_batch(
    stage: &str,
    harness: &mut MpvLifecycleVerificationHarness,
    client: &mut VerificationClientRuntime,
    gui: &mut GuiPersistedConfigRuntimeOwner,
) -> PlayerEventBatch {
    let batch = harness
        .take_event_batch()
        .unwrap_or_else(|| panic!("{stage}: expected player event batch"));
    assert_eq!(
        harness.take_event_batch(),
        Some(batch.clone()),
        "{stage}: unacknowledged adapter batch must replay byte-for-byte"
    );

    let producer_projection = harness.projection();
    let client_error = client
        .apply_ordered_player_event_batch_for_verification(&batch, 0.0)
        .unwrap_or_else(|error| panic!("{stage}: client rejected batch: {error}"));
    assert!(
        client_error.is_none(),
        "{stage}: client batch application returned {client_error:?}"
    );
    gui.apply_ordered_player_event_batch(&batch, 0.0)
        .unwrap_or_else(|error| panic!("{stage}: GUI rejected batch: {error}"));

    let client_once = client.lifecycle_verification_projection();
    let gui_once = gui_lifecycle_verification_projection(gui);
    assert_projection_compatible(
        &format!("{stage}: adapter to client"),
        &producer_projection,
        &client_once,
    );
    assert_projection_compatible(
        &format!("{stage}: adapter to GUI"),
        &producer_projection,
        &gui_once,
    );
    assert_projection_compatible(&format!("{stage}: client to GUI"), &client_once, &gui_once);

    assert!(
        client
            .apply_ordered_player_event_batch_for_verification(&batch, 0.0)
            .expect("client replay validation")
            .is_none(),
        "{stage}: client replay produced an application error"
    );
    gui.apply_ordered_player_event_batch(&batch, 0.0)
        .expect("GUI replay validation");
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

    harness
        .acknowledge(batch.acknowledgement_token)
        .unwrap_or_else(|error| panic!("{stage}: adapter acknowledgement failed: {error}"));
    client.compact_acknowledged_player_event_batch_for_verification(
        batch.acknowledgement_token,
        batch.sequence_boundary,
    );
    gui.ordered_player_events
        .compact_acknowledged_delivery(batch.acknowledgement_token, batch.sequence_boundary);

    let acknowledged_producer = harness.projection();
    let acknowledged_client = client.lifecycle_verification_projection();
    let acknowledged_gui = gui_lifecycle_verification_projection(gui);
    assert_projection_compatible(
        &format!("{stage}: acknowledged adapter to client"),
        &acknowledged_producer,
        &acknowledged_client,
    );
    assert_projection_compatible(
        &format!("{stage}: acknowledged adapter to GUI"),
        &acknowledged_producer,
        &acknowledged_gui,
    );
    assert_eq!(
        acknowledged_client.in_flight_acknowledgement,
        SnapshotField::KnownAbsent,
        "{stage}: client acknowledgement must compact its replay token"
    );
    assert_eq!(
        acknowledged_gui.in_flight_acknowledgement,
        SnapshotField::KnownAbsent,
        "{stage}: GUI acknowledgement must compact its replay token"
    );

    batch
}

#[test]
fn gui_verification_projection_preserves_real_playlist_attempt() {
    let mut owner = GuiPersistedConfigRuntimeOwner::default();
    let command_id = PlayerCommandId::new(41);
    let media_generation = PlayerMediaGeneration::new(7);

    seed_gui_playlist_resolution_attempt(
        &mut owner,
        "normal-start.mkv",
        command_id,
        media_generation,
    );

    let projection = gui_lifecycle_verification_projection(&owner);
    assert_eq!(
        projection.pending_playlist_resolution_attempt,
        SnapshotField::KnownAbsent
    );
    assert_eq!(
        projection.playlist_resolution_state,
        SnapshotField::Known("Loading".to_owned())
    );
    assert_eq!(projection.fallback_pending, SnapshotField::Known(false));
}

#[test]
fn normal_start_before_file_loaded_fans_out_through_both_production_consumers() {
    const TARGET: &str = "https://media.example/normal-start.mkv";
    const PLAYLIST_ENTRY_ID: i64 = 17;

    let mut harness = MpvLifecycleVerificationHarness::new();
    let tracked = harness.accept_tracked_load(TARGET, []);
    let mut client = verification_client_runtime();
    let mut gui = GuiPersistedConfigRuntimeOwner::default();
    seed_gui_playlist_resolution_attempt(
        &mut gui,
        TARGET,
        tracked.command_id,
        tracked.media_generation,
    );

    harness.apply_authoritative_snapshot(
        [LifecycleVerificationPlaylistEntry::new(
            PLAYLIST_ENTRY_ID,
            Some(TARGET.to_owned()),
            false,
        )],
        None,
    );
    let bound = apply_replay_and_acknowledge_batch("bound", &mut harness, &mut client, &mut gui);
    assert!(bound.events.iter().any(|event| {
        matches!(
            event.event,
            PlayerEvent::LoadAttemptBound {
                attempt_id,
                media_generation,
                playlist_entry_id: PLAYLIST_ENTRY_ID,
                ..
            } if attempt_id == tracked.attempt_id
                && media_generation == tracked.media_generation
        )
    }));

    harness.ingest_decoded_mpv_json(json!({
        "event": "start-file",
        "playlist_entry_id": PLAYLIST_ENTRY_ID,
    }));
    let starting =
        apply_replay_and_acknowledge_batch("starting", &mut harness, &mut client, &mut gui);
    assert!(starting.events.iter().any(|event| {
        matches!(
            event.event,
            PlayerEvent::LoadAttemptStarting {
                attempt_id,
                media_generation,
                playlist_entry_id: PLAYLIST_ENTRY_ID,
                owns_transport: true,
                ..
            } if attempt_id == tracked.attempt_id
                && media_generation == tracked.media_generation
        )
    }));
    assert!(starting.events.iter().all(|event| {
        !matches!(
            event.event,
            PlayerEvent::LoadAttemptActive {
                attempt_id,
                ..
            } if attempt_id == tracked.attempt_id
        )
    }));

    let producer_starting = harness.projection();
    let client_starting = client.lifecycle_verification_projection();
    let gui_starting = gui_lifecycle_verification_projection(&gui);
    assert_eq!(
        producer_starting.physical_transport_owner,
        SnapshotField::Known(tracked.attempt_id)
    );
    assert_eq!(
        producer_starting.physical_file_loaded,
        SnapshotField::Known(false)
    );
    assert_eq!(
        producer_starting.transport.phase,
        SnapshotField::Known(PlayerTransportPhase::Loading)
    );
    assert_eq!(
        client_starting.attempts[&tracked.attempt_id].owns_transport,
        SnapshotField::Known(true)
    );
    assert_eq!(
        client_starting.attempts[&tracked.attempt_id].semantic_load_result,
        SnapshotField::Known(None)
    );
    assert_eq!(
        gui_starting.pending_playlist_resolution_attempt,
        SnapshotField::Known(tracked.attempt_id)
    );
    assert_eq!(
        gui_starting.playlist_resolution_state,
        SnapshotField::Known("Loading".to_owned())
    );
    assert_eq!(gui_starting.fallback_pending, SnapshotField::Known(false));
    assert_eq!(gui_starting.physical_path, SnapshotField::Unavailable);

    harness.ingest_decoded_mpv_json(json!({
        "event": "property-change",
        "name": "path",
        "data": TARGET,
    }));
    harness.ingest_decoded_mpv_json(json!({ "event": "file-loaded" }));
    let active = apply_replay_and_acknowledge_batch("active", &mut harness, &mut client, &mut gui);
    assert!(active.events.iter().any(|event| {
        matches!(
            event.event,
            PlayerEvent::LoadAttemptActive {
                attempt_id,
                media_generation,
                playlist_entry_id: PLAYLIST_ENTRY_ID,
                ..
            } if attempt_id == tracked.attempt_id
                && media_generation == tracked.media_generation
        )
    }));

    let producer_active = harness.projection();
    let client_active = client.lifecycle_verification_projection();
    let gui_active = gui_lifecycle_verification_projection(&gui);
    assert_eq!(
        producer_active.physical_file_loaded,
        SnapshotField::Known(true)
    );
    assert_eq!(
        producer_active.physical_path,
        SnapshotField::Known(TARGET.to_owned())
    );
    assert_eq!(
        client_active.attempts[&tracked.attempt_id].semantic_load_result,
        SnapshotField::Known(Some(PlayerLoadAttemptResult::Loaded))
    );
    assert_eq!(
        gui_active.attempts[&tracked.attempt_id].semantic_load_result,
        SnapshotField::Known(Some(PlayerLoadAttemptResult::Loaded))
    );
    assert_eq!(
        gui_active.pending_playlist_resolution_attempt,
        SnapshotField::Known(tracked.attempt_id)
    );
    assert_eq!(
        gui_active.playlist_resolution_state,
        SnapshotField::Known("Active".to_owned())
    );
    assert_eq!(
        gui_active.player_local_file,
        SnapshotField::Known(TARGET.to_owned())
    );
}

#[test]
fn event_gap_snapshot_before_file_loaded_does_not_invent_semantic_success() {
    const TARGET: &str = "https://media.example/snapshot-starting.mkv";
    const PLAYLIST_ENTRY_ID: i64 = 23;

    let mut harness = MpvLifecycleVerificationHarness::new();
    let tracked = harness.accept_tracked_load(TARGET, []);
    let mut client = verification_client_runtime();
    let mut gui = GuiPersistedConfigRuntimeOwner::default();
    seed_gui_playlist_resolution_attempt(
        &mut gui,
        TARGET,
        tracked.command_id,
        tracked.media_generation,
    );

    harness.apply_authoritative_snapshot(
        [LifecycleVerificationPlaylistEntry::new(
            PLAYLIST_ENTRY_ID,
            Some(TARGET.to_owned()),
            false,
        )],
        None,
    );
    apply_replay_and_acknowledge_batch("gap bound", &mut harness, &mut client, &mut gui);

    harness.ingest_decoded_mpv_json(json!({
        "event": "start-file",
        "playlist_entry_id": PLAYLIST_ENTRY_ID,
    }));
    apply_replay_and_acknowledge_batch("gap starting", &mut harness, &mut client, &mut gui);
    assert_eq!(
        harness.projection().physical_file_loaded,
        SnapshotField::Known(false)
    );
    gui.player_local_file = Some(LocalFileUpdate::new(TARGET).with_path(TARGET));
    gui.player_local_file_placeholder = true;

    harness.detect_event_gap();
    harness.apply_authoritative_snapshot(
        [LifecycleVerificationPlaylistEntry::new(
            PLAYLIST_ENTRY_ID,
            Some(TARGET.to_owned()),
            true,
        )],
        Some(TARGET.to_owned()),
    );
    let recovery = apply_replay_and_acknowledge_batch(
        "gap snapshot before file-loaded",
        &mut harness,
        &mut client,
        &mut gui,
    );
    assert!(
        recovery.authoritative_snapshot.is_some(),
        "event-gap recovery must traverse the authoritative snapshot path"
    );

    let client_attempt = &client.lifecycle_verification_projection().attempts[&tracked.attempt_id];
    let gui_attempt = &gui_lifecycle_verification_projection(&gui).attempts[&tracked.attempt_id];
    assert_ne!(
        client_attempt.semantic_load_result,
        SnapshotField::Known(Some(PlayerLoadAttemptResult::Loaded)),
        "a physical starting snapshot is not semantic load success"
    );
    assert_ne!(
        gui_attempt.semantic_load_result,
        SnapshotField::Known(Some(PlayerLoadAttemptResult::Loaded)),
        "a physical starting snapshot is not semantic load success"
    );
    assert_eq!(
        gui_lifecycle_verification_projection(&gui).playlist_resolution_state,
        SnapshotField::Known("Loading".to_owned()),
        "snapshot path evidence before file-loaded must not complete playlist resolution"
    );
    assert!(
        gui.player_local_file_placeholder,
        "snapshot path evidence before file-loaded must not confirm the logical placeholder"
    );
}

#[test]
fn late_active_after_indeterminate_does_not_invent_a_second_loaded_result() {
    const TARGET: &str = "https://media.example/late-indeterminate.mkv";
    const PLAYLIST_ENTRY_ID: i64 = 29;

    let mut harness = MpvLifecycleVerificationHarness::new();
    let tracked = harness.accept_tracked_load(TARGET, []);
    let mut client = verification_client_runtime();
    let mut gui = GuiPersistedConfigRuntimeOwner::default();
    seed_gui_playlist_resolution_attempt(
        &mut gui,
        TARGET,
        tracked.command_id,
        tracked.media_generation,
    );

    harness.apply_authoritative_snapshot(
        [LifecycleVerificationPlaylistEntry::new(
            PLAYLIST_ENTRY_ID,
            Some(TARGET.to_owned()),
            false,
        )],
        None,
    );
    apply_replay_and_acknowledge_batch(
        "late indeterminate bound",
        &mut harness,
        &mut client,
        &mut gui,
    );
    harness.advance_clock(60_000);
    let timed_out = apply_replay_and_acknowledge_batch(
        "late indeterminate timeout",
        &mut harness,
        &mut client,
        &mut gui,
    );
    assert!(timed_out.semantic_outcomes.iter().any(|outcome| {
        matches!(
            &outcome.outcome,
            PlayerSemanticOutcome::LoadAttempt(load)
                if load.attempt_id == tracked.attempt_id
                    && load.result == PlayerLoadAttemptResult::Indeterminate
        )
    }));

    harness.ingest_decoded_mpv_json(json!({
        "event": "start-file",
        "playlist_entry_id": PLAYLIST_ENTRY_ID,
    }));
    let late_start = apply_replay_and_acknowledge_batch(
        "late indeterminate start",
        &mut harness,
        &mut client,
        &mut gui,
    );
    assert!(late_start.events.iter().any(|event| {
        matches!(
            event.event,
            PlayerEvent::LoadAttemptStarting {
                attempt_id,
                owns_transport: false,
                ..
            } if attempt_id == tracked.attempt_id
        )
    }));

    harness.ingest_decoded_mpv_json(json!({
        "event": "property-change",
        "name": "path",
        "data": TARGET,
    }));
    harness.ingest_decoded_mpv_json(json!({ "event": "file-loaded" }));
    let late_active = apply_replay_and_acknowledge_batch(
        "late active after indeterminate",
        &mut harness,
        &mut client,
        &mut gui,
    );
    assert!(
        late_active.semantic_outcomes.iter().all(|outcome| {
            !matches!(
                &outcome.outcome,
                PlayerSemanticOutcome::LoadAttempt(load)
                    if load.attempt_id == tracked.attempt_id
                        && load.result == PlayerLoadAttemptResult::Loaded
            )
        }),
        "late physical activation must not emit a second semantic result"
    );
    assert_eq!(
        harness.projection().attempts[&tracked.attempt_id].semantic_load_result,
        SnapshotField::Known(Some(PlayerLoadAttemptResult::Indeterminate))
    );
    assert_ne!(
        client.lifecycle_verification_projection().attempts[&tracked.attempt_id]
            .semantic_load_result,
        SnapshotField::Known(Some(PlayerLoadAttemptResult::Loaded))
    );
    assert_ne!(
        gui_lifecycle_verification_projection(&gui).attempts[&tracked.attempt_id]
            .semantic_load_result,
        SnapshotField::Known(Some(PlayerLoadAttemptResult::Loaded))
    );
}

#[test]
fn semantic_timeout_after_start_keeps_the_physical_transport_owner() {
    const TARGET: &str = "https://media.example/started-timeout.mkv";
    const PLAYLIST_ENTRY_ID: i64 = 31;

    let mut harness = MpvLifecycleVerificationHarness::new();
    let tracked = harness.accept_tracked_load(TARGET, []);
    let mut client = verification_client_runtime();
    let mut gui = GuiPersistedConfigRuntimeOwner::default();
    seed_gui_playlist_resolution_attempt(
        &mut gui,
        TARGET,
        tracked.command_id,
        tracked.media_generation,
    );
    harness.apply_authoritative_snapshot(
        [LifecycleVerificationPlaylistEntry::new(
            PLAYLIST_ENTRY_ID,
            Some(TARGET.to_owned()),
            false,
        )],
        None,
    );
    apply_replay_and_acknowledge_batch(
        "started timeout bound",
        &mut harness,
        &mut client,
        &mut gui,
    );
    harness.ingest_decoded_mpv_json(json!({
        "event": "start-file",
        "playlist_entry_id": PLAYLIST_ENTRY_ID,
    }));
    apply_replay_and_acknowledge_batch(
        "started timeout owns transport",
        &mut harness,
        &mut client,
        &mut gui,
    );
    assert_eq!(
        harness.projection().physical_transport_owner,
        SnapshotField::Known(tracked.attempt_id)
    );

    harness.advance_clock(60_000);
    let timeout = apply_replay_and_acknowledge_batch(
        "semantic timeout after physical start",
        &mut harness,
        &mut client,
        &mut gui,
    );
    assert!(timeout.semantic_outcomes.iter().any(|outcome| {
        matches!(
            &outcome.outcome,
            PlayerSemanticOutcome::LoadAttempt(load)
                if load.attempt_id == tracked.attempt_id
                    && load.result == PlayerLoadAttemptResult::Indeterminate
        )
    }));
    for projection in [
        harness.projection(),
        client.lifecycle_verification_projection(),
        gui_lifecycle_verification_projection(&gui),
    ] {
        assert_eq!(
            projection.physical_transport_owner,
            SnapshotField::Known(tracked.attempt_id),
            "semantic timeout must not clear a live physical owner"
        );
        assert_eq!(
            projection.attempts[&tracked.attempt_id].semantic_load_result,
            SnapshotField::Known(Some(PlayerLoadAttemptResult::Indeterminate))
        );
    }
}
