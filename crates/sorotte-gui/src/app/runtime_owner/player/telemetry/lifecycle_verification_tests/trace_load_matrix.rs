use super::*;

use sorotte_player_api::LoadAttemptId;
use sorotte_player_mpv::LifecycleVerificationTrackedLoad;

const PREDECESSOR_TARGET: &str = "https://media.example.test/predecessor-a.mkv";
const PREDECESSOR_ENTRY_ID: i64 = 101;
const SUCCESSOR_ENTRY_ID: i64 = 202;
const EXTERNAL_ENTRY_ID: i64 = 303;

struct TraceWorld {
    harness: MpvLifecycleVerificationHarness,
    client: VerificationClientRuntime,
    gui: GuiPersistedConfigRuntimeOwner,
}

impl TraceWorld {
    fn new() -> Self {
        Self {
            harness: MpvLifecycleVerificationHarness::new(),
            client: verification_client_runtime(),
            gui: GuiPersistedConfigRuntimeOwner::default(),
        }
    }

    fn flush(&mut self, stage: &str) -> PlayerEventBatch {
        apply_replay_and_acknowledge_batch(
            stage,
            &mut self.harness,
            &mut self.client,
            &mut self.gui,
        )
    }

    fn establish_active_predecessor(&mut self, stage: &str) -> LifecycleVerificationTrackedLoad {
        let tracked = self.harness.accept_tracked_load(PREDECESSOR_TARGET, []);
        self.harness.apply_authoritative_snapshot(
            [LifecycleVerificationPlaylistEntry::new(
                PREDECESSOR_ENTRY_ID,
                Some(PREDECESSOR_TARGET.to_owned()),
                false,
            )],
            None,
        );
        self.flush(&format!("{stage}: predecessor bound"));

        self.harness.ingest_decoded_mpv_json(json!({
            "event": "start-file",
            "playlist_entry_id": PREDECESSOR_ENTRY_ID,
        }));
        self.flush(&format!("{stage}: predecessor starting"));

        self.harness.ingest_decoded_mpv_json(json!({
            "event": "property-change",
            "name": "path",
            "data": PREDECESSOR_TARGET,
        }));
        self.harness
            .ingest_decoded_mpv_json(json!({ "event": "file-loaded" }));
        let active = self.flush(&format!("{stage}: predecessor active"));
        assert_eq!(
            load_result_count(
                std::slice::from_ref(&active),
                tracked.attempt_id,
                PlayerLoadAttemptResult::Loaded,
            ),
            1,
            "{stage}: predecessor load must complete once"
        );

        assert_physical_projection(
            &format!("{stage}: predecessor projection"),
            &self.harness.projection(),
            Some((
                tracked.attempt_id,
                tracked.media_generation,
                PREDECESSOR_ENTRY_ID,
                PREDECESSOR_TARGET,
                true,
            )),
        );
        tracked
    }
}

fn load_result_count(
    batches: &[PlayerEventBatch],
    attempt_id: LoadAttemptId,
    expected: PlayerLoadAttemptResult,
) -> usize {
    batches
        .iter()
        .flat_map(|batch| &batch.semantic_outcomes)
        .filter(|outcome| {
            matches!(
                &outcome.outcome,
                PlayerSemanticOutcome::LoadAttempt(load)
                    if load.attempt_id == attempt_id && load.result == expected
            )
        })
        .count()
}

fn load_outcome_count(batches: &[PlayerEventBatch], attempt_id: LoadAttemptId) -> usize {
    batches
        .iter()
        .flat_map(|batch| &batch.semantic_outcomes)
        .filter(|outcome| {
            matches!(
                &outcome.outcome,
                PlayerSemanticOutcome::LoadAttempt(load) if load.attempt_id == attempt_id
            )
        })
        .count()
}

fn assert_attempt_result(
    stage: &str,
    world: &TraceWorld,
    attempt_id: LoadAttemptId,
    expected: Option<PlayerLoadAttemptResult>,
) {
    let producer = world.harness.projection();
    let client = world.client.lifecycle_verification_projection();
    let gui = gui_lifecycle_verification_projection(&world.gui);
    for (layer, projection) in [("adapter", &producer), ("client", &client), ("GUI", &gui)] {
        assert_eq!(
            projection.attempts[&attempt_id].semantic_load_result,
            SnapshotField::Known(expected),
            "{stage}: {layer} semantic load result"
        );
    }
}

fn assert_physical_projection(
    stage: &str,
    projection: &LifecycleVerificationProjection,
    expected: Option<(LoadAttemptId, PlayerMediaGeneration, i64, &str, bool)>,
) {
    match expected {
        Some((attempt_id, media_generation, playlist_entry_id, path, file_loaded)) => {
            let attempt = projection.attempts.get(&attempt_id).unwrap_or_else(|| {
                panic!("{stage}: physical owner {attempt_id:?} has no attempt projection")
            });
            assert_eq!(
                projection.physical_transport_owner,
                SnapshotField::Known(attempt_id),
                "{stage}: physical owner"
            );
            assert_eq!(
                projection.physical_media_generation,
                SnapshotField::Known(media_generation),
                "{stage}: physical generation"
            );
            assert_eq!(
                attempt.media_generation, media_generation,
                "{stage}: owner attempt generation"
            );
            assert_eq!(
                projection.physical_playlist_entry_id,
                SnapshotField::Known(playlist_entry_id),
                "{stage}: physical playlist entry"
            );
            assert_eq!(
                attempt.playlist_entry_id,
                Some(playlist_entry_id),
                "{stage}: owner attempt playlist entry"
            );
            assert_eq!(
                projection.physical_path,
                SnapshotField::Known(path.to_owned()),
                "{stage}: physical path"
            );
            assert_eq!(
                projection.physical_file_loaded,
                SnapshotField::Known(file_loaded),
                "{stage}: physical file-loaded flag"
            );
            assert_eq!(
                projection.transport.load_attempt_id,
                SnapshotField::Known(attempt_id),
                "{stage}: transport attempt"
            );
            assert_eq!(
                projection.transport.media_generation,
                SnapshotField::Known(media_generation),
                "{stage}: transport generation"
            );
            assert_ne!(
                projection.transport.phase,
                SnapshotField::Known(PlayerTransportPhase::Empty),
                "{stage}: an owned projection cannot be empty"
            );
        }
        None => {
            assert_eq!(
                projection.physical_transport_owner,
                SnapshotField::KnownAbsent,
                "{stage}: physical owner"
            );
            assert_eq!(
                projection.physical_media_generation,
                SnapshotField::KnownAbsent,
                "{stage}: physical generation"
            );
            assert_eq!(
                projection.physical_playlist_entry_id,
                SnapshotField::KnownAbsent,
                "{stage}: physical playlist entry"
            );
            assert_eq!(
                projection.physical_path,
                SnapshotField::KnownAbsent,
                "{stage}: physical path"
            );
            assert_eq!(
                projection.physical_file_loaded,
                SnapshotField::KnownAbsent,
                "{stage}: physical file-loaded flag"
            );
            assert_eq!(
                projection.transport.load_attempt_id,
                SnapshotField::KnownAbsent,
                "{stage}: transport attempt"
            );
            assert_eq!(
                projection.transport.media_generation,
                SnapshotField::KnownAbsent,
                "{stage}: transport generation"
            );
            assert_eq!(
                projection.transport.phase,
                SnapshotField::Known(PlayerTransportPhase::Empty),
                "{stage}: transport phase"
            );
        }
    }
}

#[derive(Clone, Copy)]
struct NormalStartVariant {
    name: &'static str,
    requested_target: &'static str,
    observed_path: &'static str,
    expected_gui_name: &'static str,
    duration_seconds: Option<f64>,
    same_generation: bool,
}

fn run_normal_start_variant(variant: NormalStartVariant) {
    let mut world = TraceWorld::new();
    let predecessor = world.establish_active_predecessor(variant.name);

    let (attempt_id, media_generation) = if variant.same_generation {
        let attempt_id = world.harness.accept_same_generation_recovery(
            predecessor.media_generation,
            variant.requested_target,
            [PREDECESSOR_ENTRY_ID],
        );
        seed_gui_playlist_resolution_attempt(
            &mut world.gui,
            variant.requested_target,
            predecessor.command_id,
            predecessor.media_generation,
        );
        (attempt_id, predecessor.media_generation)
    } else {
        let successor = world
            .harness
            .accept_tracked_load(variant.requested_target, [PREDECESSOR_ENTRY_ID]);
        seed_gui_playlist_resolution_attempt(
            &mut world.gui,
            variant.requested_target,
            successor.command_id,
            successor.media_generation,
        );
        (successor.attempt_id, successor.media_generation)
    };

    world.harness.apply_authoritative_snapshot(
        [
            LifecycleVerificationPlaylistEntry::new(
                PREDECESSOR_ENTRY_ID,
                Some(PREDECESSOR_TARGET.to_owned()),
                false,
            ),
            LifecycleVerificationPlaylistEntry::new(
                SUCCESSOR_ENTRY_ID,
                Some(variant.requested_target.to_owned()),
                false,
            ),
        ],
        Some(PREDECESSOR_TARGET.to_owned()),
    );
    let bound = world.flush(&format!("{}: successor bound", variant.name));
    assert!(bound.events.iter().any(|event| {
        matches!(
            &event.event,
            PlayerEvent::LoadAttemptBound {
                attempt_id: observed,
                media_generation: observed_generation,
                playlist_entry_id: SUCCESSOR_ENTRY_ID,
                ..
            } if *observed == attempt_id && *observed_generation == media_generation
        )
    }));

    world.harness.ingest_decoded_mpv_json(json!({
        "event": "start-file",
        "playlist_entry_id": SUCCESSOR_ENTRY_ID,
    }));
    let starting = world.flush(&format!("{}: successor starting", variant.name));
    assert!(starting.events.iter().any(|event| {
        matches!(
            &event.event,
            PlayerEvent::LoadAttemptStarting {
                attempt_id: observed,
                media_generation: observed_generation,
                playlist_entry_id: SUCCESSOR_ENTRY_ID,
                owns_transport: true,
                ..
            } if *observed == attempt_id && *observed_generation == media_generation
        )
    }));

    let producer_starting = world.harness.projection();
    let client_starting = world.client.lifecycle_verification_projection();
    let gui_starting = gui_lifecycle_verification_projection(&world.gui);
    for (layer, projection) in [
        ("adapter", &producer_starting),
        ("client", &client_starting),
        ("GUI", &gui_starting),
    ] {
        assert_eq!(
            projection.physical_transport_owner,
            SnapshotField::Known(attempt_id),
            "{}: {layer} must project B as transport owner after start-file",
            variant.name
        );
        assert_eq!(
            projection.transport.phase,
            SnapshotField::Known(PlayerTransportPhase::Loading),
            "{}: {layer} must receive B's Loading phase",
            variant.name
        );
        assert_eq!(
            projection.attempts[&predecessor.attempt_id].owns_transport,
            SnapshotField::Known(false),
            "{}: {layer} must revoke A's physical transport ownership",
            variant.name
        );
        assert_eq!(
            projection.attempts[&attempt_id].semantic_load_result,
            SnapshotField::Known(None),
            "{}: {layer} must not infer semantic success from start-file",
            variant.name
        );
    }
    assert_eq!(
        gui_starting.pending_playlist_resolution_attempt,
        SnapshotField::Known(attempt_id),
        "{}: GUI playlist resolution must retain B's LoadAttemptId",
        variant.name
    );
    assert_eq!(
        gui_starting.playlist_resolution_state,
        SnapshotField::Known("Loading".to_owned()),
        "{}: GUI playlist resolution must remain Loading",
        variant.name
    );
    assert_eq!(
        load_result_count(
            std::slice::from_ref(&starting),
            attempt_id,
            PlayerLoadAttemptResult::Loaded,
        ),
        0,
        "{}: start-file must not report load success",
        variant.name
    );

    world.harness.ingest_decoded_mpv_json(json!({
        "event": "property-change",
        "name": "paused-for-cache",
        "data": true,
    }));
    world.harness.ingest_decoded_mpv_json(json!({
        "event": "property-change",
        "name": "cache-buffering-state",
        "data": 37.5,
    }));
    let cache = world.flush(&format!("{}: successor cache telemetry", variant.name));
    let producer_cache = world.harness.projection();
    let client_cache = world.client.lifecycle_verification_projection();
    let gui_cache = gui_lifecycle_verification_projection(&world.gui);
    for (layer, projection) in [
        ("adapter", &producer_cache),
        ("client", &client_cache),
        ("GUI", &gui_cache),
    ] {
        assert_eq!(
            projection.transport.load_attempt_id,
            SnapshotField::Known(attempt_id),
            "{}: {layer} cache telemetry owner",
            variant.name
        );
        assert_eq!(
            projection.transport.paused_for_cache,
            SnapshotField::Known(true),
            "{}: {layer} paused-for-cache delta",
            variant.name
        );
        assert_eq!(
            projection.transport.cache_percentage,
            SnapshotField::Known(37.5),
            "{}: {layer} cache percentage delta",
            variant.name
        );
    }
    assert_attempt_result(
        &format!("{}: before file-loaded", variant.name),
        &world,
        attempt_id,
        None,
    );
    assert_eq!(
        gui_cache.playlist_resolution_state,
        SnapshotField::Known("Loading".to_owned()),
        "{}: cache telemetry cannot complete GUI playlist resolution",
        variant.name
    );
    assert_eq!(
        load_result_count(
            &[bound.clone(), starting.clone(), cache.clone()],
            attempt_id,
            PlayerLoadAttemptResult::Loaded,
        ),
        0,
        "{}: no pre-file-loaded batch may report success",
        variant.name
    );

    if let Some(duration_seconds) = variant.duration_seconds {
        world.harness.ingest_decoded_mpv_json(json!({
            "event": "property-change",
            "name": "duration",
            "data": duration_seconds,
        }));
    }
    world.harness.ingest_decoded_mpv_json(json!({
        "event": "property-change",
        "name": "path",
        "data": variant.observed_path,
    }));
    world
        .harness
        .ingest_decoded_mpv_json(json!({ "event": "file-loaded" }));
    let active = world.flush(&format!("{}: successor active", variant.name));
    assert!(active.events.iter().any(|event| {
        matches!(
            &event.event,
            PlayerEvent::LoadAttemptActive {
                attempt_id: observed,
                media_generation: observed_generation,
                playlist_entry_id: SUCCESSOR_ENTRY_ID,
                ..
            } if *observed == attempt_id && *observed_generation == media_generation
        )
    }));
    assert!(
        active.events.iter().any(|event| {
            matches!(
                &event.event,
                PlayerEvent::LocalFileChanged {
                    attempt_id: observed,
                    media_generation: observed_generation,
                    update,
                } if *observed == attempt_id
                    && *observed_generation == media_generation
                    && update.name == variant.expected_gui_name
            )
        }),
        "{}: correlated physical path must be delivered as LocalFileChanged",
        variant.name
    );
    assert_eq!(
        load_result_count(
            &[bound, starting, cache, active],
            attempt_id,
            PlayerLoadAttemptResult::Loaded,
        ),
        1,
        "{}: B must emit exactly one Loaded semantic result",
        variant.name
    );
    assert_attempt_result(
        &format!("{}: after file-loaded", variant.name),
        &world,
        attempt_id,
        Some(PlayerLoadAttemptResult::Loaded),
    );
    let gui_active = gui_lifecycle_verification_projection(&world.gui);
    assert_eq!(
        gui_active.playlist_resolution_state,
        SnapshotField::Known("Active".to_owned()),
        "{}: GUI playlist resolution must become Active",
        variant.name
    );
    assert_eq!(
        gui_active.player_local_file,
        SnapshotField::Known(variant.expected_gui_name.to_owned()),
        "{}: GUI must retain the observed physical path",
        variant.name
    );
    if let Some(duration_seconds) = variant.duration_seconds {
        assert_eq!(
            world
                .gui
                .player_local_file
                .as_ref()
                .and_then(|file| file.duration_seconds),
            Some(duration_seconds),
            "{}: local-file duration metadata must survive the ordered path",
            variant.name
        );
    }
    if variant.same_generation {
        assert_eq!(
            media_generation, predecessor.media_generation,
            "{}: recovery successor must retain A's generation",
            variant.name
        );
    }
}

#[test]
fn trace_a_normal_start_owns_transport_before_file_loaded_for_all_target_classes() {
    for variant in [
        NormalStartVariant {
            name: "trace A local file",
            requested_target: "C:/media/replacement-b.mkv",
            observed_path: "C:/media/replacement-b.mkv",
            expected_gui_name: "replacement-b.mkv",
            duration_seconds: Some(7_201.25),
            same_generation: false,
        },
        NormalStartVariant {
            name: "trace A network VOD",
            requested_target: "https://media.example.test/vod-b.mkv",
            observed_path: "https://media.example.test/vod-b.mkv",
            expected_gui_name: "https://media.example.test/vod-b.mkv",
            duration_seconds: None,
            same_generation: false,
        },
        NormalStartVariant {
            name: "trace A YouTube extractor",
            requested_target: "https://www.youtube.com/watch?v=trace-b",
            observed_path: "https://rr.example.googlevideo.com/videoplayback?id=trace-b",
            expected_gui_name: "https://rr.example.googlevideo.com/videoplayback?id=trace-b",
            duration_seconds: None,
            same_generation: false,
        },
        NormalStartVariant {
            name: "trace A same-generation recovery",
            requested_target: "https://media.example.test/recovery-b.mkv",
            observed_path: "https://media.example.test/recovery-b.mkv",
            expected_gui_name: "https://media.example.test/recovery-b.mkv",
            duration_seconds: None,
            same_generation: true,
        },
    ] {
        run_normal_start_variant(variant);
    }
}

#[derive(Clone, Copy)]
enum AuthorityOutcome {
    PredecessorCurrent,
    Empty,
    SuccessorAppeared,
    ExternalCurrent,
}

fn run_replacement_timeout_authority(outcome: AuthorityOutcome) {
    let label = match outcome {
        AuthorityOutcome::PredecessorCurrent => "trace D predecessor current",
        AuthorityOutcome::Empty => "trace D empty",
        AuthorityOutcome::SuccessorAppeared => "trace D successor appeared",
        AuthorityOutcome::ExternalCurrent => "trace D external current",
    };
    const SUCCESSOR_TARGET: &str = "https://media.example.test/never-started-b.mkv";
    const EXTERNAL_TARGET: &str = "C:/external/unrelated-x.mkv";

    let mut world = TraceWorld::new();
    let predecessor = world.establish_active_predecessor(label);
    let successor = world
        .harness
        .accept_tracked_load(SUCCESSOR_TARGET, [PREDECESSOR_ENTRY_ID]);
    seed_gui_playlist_resolution_attempt(
        &mut world.gui,
        SUCCESSOR_TARGET,
        successor.command_id,
        successor.media_generation,
    );

    // Raw predecessor ingress advances the reducer's own clock independently
    // of the harness driver. A full two-deadline step remains deterministic
    // without depending on how many predecessor observations were ingested.
    world.harness.advance_clock(120_000);
    let timeout = world.flush(&format!("{label}: replacement timeout"));
    assert_eq!(
        load_result_count(
            std::slice::from_ref(&timeout),
            successor.attempt_id,
            PlayerLoadAttemptResult::Indeterminate,
        ),
        1,
        "{label}: replacement timeout must emit one Indeterminate result"
    );
    assert_attempt_result(
        &format!("{label}: timed out"),
        &world,
        successor.attempt_id,
        Some(PlayerLoadAttemptResult::Indeterminate),
    );
    assert_physical_projection(
        &format!("{label}: before authority"),
        &world.harness.projection(),
        Some((
            predecessor.attempt_id,
            predecessor.media_generation,
            PREDECESSOR_ENTRY_ID,
            PREDECESSOR_TARGET,
            true,
        )),
    );
    assert_ne!(
        world.harness.projection().physical_path,
        SnapshotField::Known(SUCCESSOR_TARGET.to_owned()),
        "{label}: submitted target B cannot become the physical path before an ownership boundary"
    );

    world.harness.detect_event_gap();
    match outcome {
        AuthorityOutcome::PredecessorCurrent => world.harness.apply_authoritative_snapshot(
            [LifecycleVerificationPlaylistEntry::new(
                PREDECESSOR_ENTRY_ID,
                Some(PREDECESSOR_TARGET.to_owned()),
                true,
            )],
            Some(PREDECESSOR_TARGET.to_owned()),
        ),
        AuthorityOutcome::Empty => world.harness.apply_authoritative_snapshot([], None),
        AuthorityOutcome::SuccessorAppeared => world.harness.apply_authoritative_snapshot(
            [
                LifecycleVerificationPlaylistEntry::new(
                    PREDECESSOR_ENTRY_ID,
                    Some(PREDECESSOR_TARGET.to_owned()),
                    false,
                ),
                LifecycleVerificationPlaylistEntry::new(
                    SUCCESSOR_ENTRY_ID,
                    Some(SUCCESSOR_TARGET.to_owned()),
                    true,
                ),
            ],
            Some(SUCCESSOR_TARGET.to_owned()),
        ),
        AuthorityOutcome::ExternalCurrent => world.harness.apply_authoritative_snapshot(
            [LifecycleVerificationPlaylistEntry::new(
                EXTERNAL_ENTRY_ID,
                Some(EXTERNAL_TARGET.to_owned()),
                true,
            )],
            Some(EXTERNAL_TARGET.to_owned()),
        ),
    }
    let authority = world.flush(&format!("{label}: authoritative recovery"));
    assert!(
        authority.authoritative_snapshot.is_some(),
        "{label}: event-gap recovery must use an authoritative snapshot"
    );
    assert_eq!(
        load_outcome_count(std::slice::from_ref(&authority), successor.attempt_id),
        0,
        "{label}: authority cannot emit a second semantic result for B"
    );

    match outcome {
        AuthorityOutcome::PredecessorCurrent => {
            let projection = world.harness.projection();
            assert_physical_projection(
                label,
                &projection,
                Some((
                    predecessor.attempt_id,
                    predecessor.media_generation,
                    PREDECESSOR_ENTRY_ID,
                    PREDECESSOR_TARGET,
                    true,
                )),
            );
            assert_eq!(
                projection.attempts[&predecessor.attempt_id].logical_ownership_revoked,
                SnapshotField::Known(true),
                "{label}: physical A must not regain revoked logical ownership"
            );
            assert_ne!(
                projection.logical_owner,
                SnapshotField::Known(predecessor.attempt_id),
                "{label}: A cannot reclaim logical ownership"
            );
        }
        AuthorityOutcome::Empty => {
            assert_physical_projection(label, &world.harness.projection(), None);
        }
        AuthorityOutcome::SuccessorAppeared => {
            let projection = world.harness.projection();
            let bound = &projection.attempts[&successor.attempt_id];
            assert_eq!(
                bound.playlist_entry_id,
                Some(SUCCESSOR_ENTRY_ID),
                "{label}: the strict target match must bind B"
            );
            assert_eq!(
                bound.semantic_load_result,
                SnapshotField::Known(Some(PlayerLoadAttemptResult::Indeterminate)),
                "{label}: authoritative appearance must not invent a second semantic result"
            );
            assert_eq!(
                bound.owns_transport,
                SnapshotField::Known(false),
                "{label}: quiescent B remains fail-closed until correlated file-loaded"
            );
            assert_physical_projection(
                label,
                &projection,
                Some((
                    predecessor.attempt_id,
                    predecessor.media_generation,
                    PREDECESSOR_ENTRY_ID,
                    PREDECESSOR_TARGET,
                    true,
                )),
            );
        }
        AuthorityOutcome::ExternalCurrent => {
            let projection = world.harness.projection();
            if let Some(successor_projection) = projection.attempts.get(&successor.attempt_id) {
                assert_eq!(
                    successor_projection.playlist_entry_id, None,
                    "{label}: unrelated X must not bind to B"
                );
            }
            let external_attempt = projection
                .attempts
                .iter()
                .find(|(_, attempt)| {
                    attempt.command_id.is_none()
                        && attempt.playlist_entry_id == Some(EXTERNAL_ENTRY_ID)
                })
                .map(|(attempt_id, attempt)| (*attempt_id, attempt.media_generation))
                .expect("trace D external current: authority must allocate an external X attempt");
            assert_ne!(
                external_attempt.1, successor.media_generation,
                "{label}: external X needs a distinct generation"
            );
            assert_physical_projection(
                label,
                &projection,
                Some((
                    external_attempt.0,
                    external_attempt.1,
                    EXTERNAL_ENTRY_ID,
                    EXTERNAL_TARGET,
                    true,
                )),
            );
        }
    }
}

#[test]
fn trace_d_replacement_timeout_reconciles_all_authoritative_outcomes() {
    for outcome in [
        AuthorityOutcome::PredecessorCurrent,
        AuthorityOutcome::Empty,
        AuthorityOutcome::SuccessorAppeared,
        AuthorityOutcome::ExternalCurrent,
    ] {
        run_replacement_timeout_authority(outcome);
    }
}

#[derive(Clone, Copy)]
enum OldTerminalOrdering {
    BeforeSuccessorFileLoaded,
    AfterSuccessorFileLoaded,
}

fn run_same_generation_terminal_ordering(ordering: OldTerminalOrdering) {
    const RECOVERY_TARGET: &str = "https://media.example.test/same-generation-recovery.mkv";
    let label = match ordering {
        OldTerminalOrdering::BeforeSuccessorFileLoaded => {
            "trace E predecessor terminal before successor file-loaded"
        }
        OldTerminalOrdering::AfterSuccessorFileLoaded => {
            "trace E predecessor terminal after successor file-loaded"
        }
    };

    let mut world = TraceWorld::new();
    let predecessor = world.establish_active_predecessor(label);
    let successor = world.harness.accept_same_generation_recovery(
        predecessor.media_generation,
        RECOVERY_TARGET,
        [PREDECESSOR_ENTRY_ID],
    );
    seed_gui_playlist_resolution_attempt(
        &mut world.gui,
        RECOVERY_TARGET,
        predecessor.command_id,
        predecessor.media_generation,
    );

    world.harness.apply_authoritative_snapshot(
        [
            LifecycleVerificationPlaylistEntry::new(
                PREDECESSOR_ENTRY_ID,
                Some(PREDECESSOR_TARGET.to_owned()),
                false,
            ),
            LifecycleVerificationPlaylistEntry::new(
                SUCCESSOR_ENTRY_ID,
                Some(RECOVERY_TARGET.to_owned()),
                false,
            ),
        ],
        Some(PREDECESSOR_TARGET.to_owned()),
    );
    let mut batches = vec![world.flush(&format!("{label}: successor bound"))];

    world.harness.ingest_decoded_mpv_json(json!({
        "event": "start-file",
        "playlist_entry_id": SUCCESSOR_ENTRY_ID,
    }));
    let starting = world.flush(&format!("{label}: successor starting"));
    assert!(starting.events.iter().any(|event| {
        matches!(
            &event.event,
            PlayerEvent::LoadAttemptStarting {
                attempt_id,
                media_generation,
                owns_transport: true,
                ..
            } if *attempt_id == successor && *media_generation == predecessor.media_generation
        )
    }));
    batches.push(starting);

    world.harness.ingest_decoded_mpv_json(json!({
        "event": "property-change",
        "name": "paused-for-cache",
        "data": true,
    }));
    world.harness.ingest_decoded_mpv_json(json!({
        "event": "property-change",
        "name": "cache-buffering-state",
        "data": 61.25,
    }));
    batches.push(world.flush(&format!("{label}: successor cache telemetry")));
    let before_terminal = world.harness.projection();
    assert_eq!(
        before_terminal.physical_transport_owner,
        SnapshotField::Known(successor),
        "{label}: normal recovery start must make B the physical owner"
    );
    assert_eq!(
        before_terminal.attempts[&predecessor.attempt_id].owns_transport,
        SnapshotField::Known(false),
        "{label}: A cannot retain transport after B starts"
    );
    assert_eq!(
        before_terminal.transport.paused_for_cache,
        SnapshotField::Known(true),
        "{label}: recovery cache policy must be attached to B"
    );
    assert_eq!(
        before_terminal.transport.cache_percentage,
        SnapshotField::Known(61.25),
        "{label}: recovery cache percentage must be attached to B"
    );
    let recovery_count_before_terminal = world
        .harness
        .adapter()
        .network_stream_recovery_attempt_count();

    let emit_old_terminal = |world: &mut TraceWorld| {
        let raw = match ordering {
            OldTerminalOrdering::BeforeSuccessorFileLoaded => json!({
                "event": "end-file",
                "playlist_entry_id": PREDECESSOR_ENTRY_ID,
                "reason": "stop",
            }),
            OldTerminalOrdering::AfterSuccessorFileLoaded => json!({
                "event": "end-file",
                "playlist_entry_id": PREDECESSOR_ENTRY_ID,
                "reason": "error",
                "file_error": "superseded predecessor failed",
            }),
        };
        world.harness.ingest_decoded_mpv_json(raw);
        world.flush(&format!("{label}: predecessor terminal"))
    };

    if matches!(ordering, OldTerminalOrdering::BeforeSuccessorFileLoaded) {
        batches.push(emit_old_terminal(&mut world));
    }

    world.harness.ingest_decoded_mpv_json(json!({
        "event": "property-change",
        "name": "path",
        "data": RECOVERY_TARGET,
    }));
    world
        .harness
        .ingest_decoded_mpv_json(json!({ "event": "file-loaded" }));
    batches.push(world.flush(&format!("{label}: successor active")));

    if matches!(ordering, OldTerminalOrdering::AfterSuccessorFileLoaded) {
        batches.push(emit_old_terminal(&mut world));
    }

    let old_terminals = batches
        .iter()
        .flat_map(|batch| &batch.events)
        .filter(|event| {
            matches!(
                &event.event,
                PlayerEvent::LoadAttemptTerminal { attempt_id, .. }
                    if *attempt_id == predecessor.attempt_id
            )
        })
        .count();
    assert_eq!(
        old_terminals, 1,
        "{label}: A's delayed terminal must close A exactly once"
    );
    assert!(
        batches.iter().flat_map(|batch| &batch.events).all(|event| {
            !matches!(
                &event.event,
                PlayerEvent::LogicalPlaybackTerminal {
                    media_generation,
                    ..
                } if *media_generation == predecessor.media_generation
            )
        }),
        "{label}: A's terminal cannot emit a logical terminal for the shared generation"
    );
    assert_eq!(
        load_result_count(&batches, successor, PlayerLoadAttemptResult::Loaded,),
        1,
        "{label}: B must emit one semantic load success"
    );
    assert_attempt_result(
        &format!("{label}: final successor"),
        &world,
        successor,
        Some(PlayerLoadAttemptResult::Loaded),
    );
    let final_projection = world.harness.projection();
    assert_physical_projection(
        label,
        &final_projection,
        Some((
            successor,
            predecessor.media_generation,
            SUCCESSOR_ENTRY_ID,
            RECOVERY_TARGET,
            true,
        )),
    );
    assert_eq!(
        final_projection.transport.paused_for_cache,
        SnapshotField::Known(true),
        "{label}: A's terminal must not clear B's cache policy"
    );
    assert_eq!(
        final_projection.transport.cache_percentage,
        SnapshotField::Known(61.25),
        "{label}: A's terminal must not clear B's cache telemetry"
    );
    assert_eq!(
        world
            .harness
            .adapter()
            .network_stream_recovery_attempt_count(),
        recovery_count_before_terminal,
        "{label}: delayed A terminal cannot consume another recovery attempt"
    );
}

#[test]
fn trace_e_same_generation_recovery_survives_both_old_terminal_orderings() {
    for ordering in [
        OldTerminalOrdering::BeforeSuccessorFileLoaded,
        OldTerminalOrdering::AfterSuccessorFileLoaded,
    ] {
        run_same_generation_terminal_ordering(ordering);
    }
}
