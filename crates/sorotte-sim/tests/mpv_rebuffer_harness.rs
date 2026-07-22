#![cfg(unix)]

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sorotte_client_core::{
    CoordinatorCommandId, CoordinatorPlayerCommand, DesiredRoomPlayback,
    DesiredRoomPlaybackUpdateKind, LogicalMediaId, MediaTransportKind, PlaybackCoordinator,
    PlaybackCoordinatorAction, PlaybackCoordinatorConfig, PlayerTransportObservation,
    SeekPreparationTerminalOutcome, SeekTargetAvailability,
};
use sorotte_player_api::{
    PlayerAdapter, PlayerCacheTelemetryUpdate, PlayerCommand, PlayerCommandId,
    PlayerCommandProgressState, PlayerCommandResult, PlayerPlayIntent, PlayerTransportPhase,
    PlayerTransportTelemetryUpdate,
};
use sorotte_player_mpv::{ConnectedMpvPlayer, MpvAdapter, MpvNetworkMediaPolicyApplicationState};
use sorotte_sim::{BurstStall, FaultInjectingHttpServer, HttpMediaFixture, NetworkFaultProfile};

const TEST_DURATION: Duration = Duration::from_secs(20);
const POLL_INTERVAL: Duration = Duration::from_millis(20);
const SEMANTICS_TIMEOUT: Duration = Duration::from_secs(10);
const PR_STALL_TIMEOUT: Duration = Duration::from_secs(14);
const CACHE_CAP_TIMEOUT: Duration = Duration::from_secs(18);

#[test]
#[ignore = "required Linux CI integration test; requires the mpv binary"]
fn real_mpv_pause_seek_resume_semantics() {
    let media = TemporaryMedia::wav(8);
    let (process, mut player) = MpvProcess::start(10_000);
    player
        .execute(PlayerCommand::SetPaused(true))
        .expect("real-mpv semantics fixture should begin intentionally paused");
    let mut observed = PlayerTransportTelemetryUpdate::default();

    let load_restart_baseline = observed.playback_restart_sequence.unwrap_or(0);
    let load_id = player
        .execute_tracked(PlayerCommand::OpenFile(media.path_string()))
        .expect("mpv should accept the local deterministic fixture");
    wait_for_completed_command(&mut player, &mut observed, load_id, "paused media load");
    wait_for_observation(
        &mut player,
        &mut observed,
        "ReadyPaused with mpv's paused core-idle semantics",
        |state| {
            state.phase == Some(PlayerTransportPhase::ReadyPaused)
                && state.logical_pause == Some(true)
                && state.paused_for_cache == Some(false)
                && state.core_idle == Some(true)
        },
    );

    let start_after_load_id = player
        .execute_tracked(PlayerCommand::Play(PlayerPlayIntent::StartAfterLoad {
            baseline_restart_sequence: load_restart_baseline,
        }))
        .expect("mpv should accept start-after-load");
    wait_for_completed_command(
        &mut player,
        &mut observed,
        start_after_load_id,
        "start after load",
    );
    wait_for_observation(
        &mut player,
        &mut observed,
        "playing advancement after load",
        |state| {
            state.phase == Some(PlayerTransportPhase::Playing)
                && state.logical_pause == Some(false)
                && state
                    .position_seconds
                    .is_some_and(|position| position > 0.02)
        },
    );
    assert!(
        observed.playback_restart_sequence.unwrap_or(0) > load_restart_baseline,
        "post-load start must be acknowledged by a newer playback-restart: {observed:?}"
    );

    pause_and_wait(&mut player, &mut observed, "pause before ordinary resume");
    let resume_restart_baseline = observed.playback_restart_sequence.unwrap_or(0);
    let resume_position_baseline = observed.position_seconds.unwrap_or_default();
    let resume_id = player
        .execute_tracked(PlayerCommand::Play(PlayerPlayIntent::Resume))
        .expect("mpv should accept ordinary resume");
    wait_for_completed_command(
        &mut player,
        &mut observed,
        resume_id,
        "ordinary resume without playback-restart",
    );
    wait_for_observation(
        &mut player,
        &mut observed,
        "fresh advancement after ordinary resume",
        |state| {
            state.logical_pause == Some(false)
                && state
                    .position_seconds
                    .is_some_and(|position| position > resume_position_baseline + 0.01)
        },
    );
    assert_eq!(
        observed.playback_restart_sequence.unwrap_or(0),
        resume_restart_baseline,
        "ordinary pause-to-play resume should not depend on a new playback-restart"
    );

    pause_and_wait(&mut player, &mut observed, "pause before seek-backed start");
    let seek_restart_baseline = observed.playback_restart_sequence.unwrap_or(0);
    seek_and_wait(
        &mut player,
        &mut observed,
        2.0,
        "seek while intentionally paused",
    );
    wait_for_observation(
        &mut player,
        &mut observed,
        "paused seek retains ReadyPaused and core-idle=true",
        |state| {
            state.phase == Some(PlayerTransportPhase::ReadyPaused)
                && state.logical_pause == Some(true)
                && state.core_idle == Some(true)
                && state
                    .position_seconds
                    .is_some_and(|position| (position - 2.0).abs() <= 0.5)
        },
    );

    let start_after_seek_id = player
        .execute_tracked(PlayerCommand::Play(PlayerPlayIntent::StartAfterSeek {
            baseline_restart_sequence: seek_restart_baseline,
        }))
        .expect("mpv should accept start-after-seek");
    wait_for_completed_command(
        &mut player,
        &mut observed,
        start_after_seek_id,
        "start after paused seek",
    );
    wait_for_observation(
        &mut player,
        &mut observed,
        "advancement after seek-backed start",
        |state| {
            state.logical_pause == Some(false)
                && state
                    .position_seconds
                    .is_some_and(|position| position > 2.01)
        },
    );
    assert!(
        observed.playback_restart_sequence.unwrap_or(0) > seek_restart_baseline,
        "post-seek start must observe a newer playback-restart before advancement: {observed:?}"
    );

    drop(player);
    drop(process);

    verify_barrier_start_then_ordinary_pause_with_real_mpv(&media);
    verify_one_bounded_rebuffer_episode_with_real_mpv();
    verify_mid_play_unbuffered_seek_is_bounded_with_real_mpv();
}

#[test]
#[ignore = "required Linux CI integration test; requires the mpv binary"]
fn real_mpv_cache_cap_drains_and_input_resumes() {
    const MAX_FORWARD_BYTES: u64 = 524_288;
    const HOLD_AFTER_BODY_BYTES: usize = 600_000;
    let cache_options = [
        ("cache", "yes"),
        ("cache-pause", "yes"),
        ("cache-pause-initial", "yes"),
        ("cache-pause-wait", "0.3"),
        // Make the byte cap, rather than the time cap, the deterministic limiting factor.
        ("cache-secs", "60"),
        ("demuxer-max-bytes", "524288"),
        ("demuxer-max-back-bytes", "65536"),
    ];
    let server = FaultInjectingHttpServer::start_with_transmission_holds(
        BTreeMap::from([(
            "/cap.wav".to_owned(),
            HttpMediaFixture::static_bytes("audio/wav", pcm_wav(30)).with_faults(
                NetworkFaultProfile {
                    bytes_per_second: Some(2_000_000),
                    ..NetworkFaultProfile::default()
                },
            ),
        )]),
        BTreeMap::from([("/cap.wav".to_owned(), HOLD_AFTER_BODY_BYTES)]),
    )
    .expect("cache-cap HTTP fixture should start");
    let (process, connected) = MpvProcess::start_with_cache_options(30_000, &cache_options);
    let mut player = connected.into_inner();
    player.configure_network_media_options(cache_options);
    player
        .execute(PlayerCommand::SetPaused(true))
        .expect("cache-cap fixture should begin intentionally paused");
    let mut observed = PlayerTransportTelemetryUpdate::default();
    let mut observed_cache = PlayerCacheTelemetryUpdate::default();
    let load_id = player
        .execute_tracked(PlayerCommand::OpenFile(server.url("/cap.wav")))
        .expect("mpv should accept the byte-cap fixture");
    wait_for_completed_command(&mut player, &mut observed, load_id, "byte-cap media load");

    let expected_cache_options = cache_options
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    let policy_deadline = Instant::now() + SEMANTICS_TIMEOUT;
    loop {
        drain_transport_updates(&mut player, &mut observed);
        drain_cache_updates(&mut player, &mut observed_cache);
        let diagnostics = player.network_media_diagnostic_snapshot();
        if diagnostics.media_generation == observed.media_generation
            && diagnostics.load_sequence.is_some()
            && diagnostics.application_state == Some(MpvNetworkMediaPolicyApplicationState::Applied)
            && diagnostics.verification_complete
            && diagnostics.desired_cache_options == expected_cache_options
            && diagnostics.effective_cache_options == expected_cache_options
        {
            break;
        }
        assert!(
            Instant::now() < policy_deadline,
            "real mpv never verified the generation-bound network policy through the Lua hook/readback path: diagnostics={diagnostics:?}, transport={observed:?}, cache={observed_cache:?}"
        );
        sleep(POLL_INTERVAL);
    }

    let cap_deadline = Instant::now() + CACHE_CAP_TIMEOUT;
    let (capped_forward_bytes, capped_cache_end_seconds) = loop {
        drain_transport_updates(&mut player, &mut observed);
        drain_cache_updates(&mut player, &mut observed_cache);
        if observed.demuxer_cache_idle == Some(true)
            && observed.paused_for_cache == Some(false)
            && observed.logical_pause == Some(true)
            && observed.phase == Some(PlayerTransportPhase::ReadyPaused)
            && observed_cache
                .buffered_ahead_bytes
                .is_some_and(|bytes| bytes >= MAX_FORWARD_BYTES / 2)
            && observed_cache
                .cache_end_seconds
                .is_some_and(|cache_end| cache_end.is_finite())
            && observed_cache.eof != Some(true)
        {
            break (
                observed_cache
                    .buffered_ahead_bytes
                    .expect("the cap predicate established forward bytes"),
                observed_cache
                    .cache_end_seconds
                    .expect("the cap predicate established a finite cache end"),
            );
        }
        assert!(
            Instant::now() < cap_deadline,
            "mpv never reached an intentional byte-cap idle state: transport={observed:?}, cache={observed_cache:?}"
        );
        sleep(POLL_INTERVAL);
    };

    let position_before_start = observed.position_seconds.unwrap_or_default();
    assert!(
        capped_cache_end_seconds > position_before_start,
        "the capped cache end must be ahead of the playback head: position={position_before_start:.3}, cache_end={capped_cache_end_seconds:.3}, observed={observed:?}"
    );
    assert!(
        server.wait_for_held_transmissions(1, Duration::from_secs(3)),
        "the deterministic HTTP fixture must hold the response after filling the initial cap"
    );
    player
        .execute(PlayerCommand::SetPaused(false))
        .expect("mpv should accept playback after filling the byte cap");
    let mut history = Vec::new();
    let mut resume_evidence = CacheCapResumeEvidence::default();
    let mut observed_underrun = false;
    let mut steady_playback_started = false;
    let mut observed_mid_play_cache_pause = false;
    let mut source_released = false;
    let playback_deadline = Instant::now() + CACHE_CAP_TIMEOUT;
    // Crossing the cache-end value captured while mpv was idle proves that
    // playback consumed beyond the media initially held under the byte cap.
    let must_advance_past_seconds = capped_cache_end_seconds + 1.0;
    loop {
        let previous_len = history.len();
        drain_transport_history(&mut player, &mut observed, &mut history);
        let cache_updates = drain_cache_updates(&mut player, &mut observed_cache);
        observed_underrun |= cache_updates
            .iter()
            .any(|update| update.underrun == Some(true));
        for sample in &history[previous_len..] {
            let update = &sample.update;
            let state = &sample.state_after;
            steady_playback_started |= state.phase == Some(PlayerTransportPhase::Playing)
                && state
                    .position_seconds
                    .is_some_and(|position| position > position_before_start + 0.05);
            if steady_playback_started {
                observed_mid_play_cache_pause |= state.paused_for_cache == Some(true)
                    || state.phase == Some(PlayerTransportPhase::Rebuffering);
            }
            resume_evidence.observe(update, capped_forward_bytes);
        }
        if !source_released
            && resume_evidence.drain_observed
            && resume_evidence.resumed_idle_observed
        {
            server.release_held_transmissions();
            resume_evidence.mark_source_released();
            source_released = true;
        }
        if source_released
            && resume_evidence.fresh_positive_input_observed
            && observed
                .position_seconds
                .is_some_and(|position| position > must_advance_past_seconds)
        {
            break;
        }
        assert!(
            Instant::now() < playback_deadline,
            "mpv did not drain below half its cap, release the held source, and refill beyond the release floor; cap={capped_forward_bytes}, target={must_advance_past_seconds:.3}, source_released={source_released}, evidence={resume_evidence:?}, latest={observed:?}, history_tail={:?}",
            history.iter().rev().take(12).collect::<Vec<_>>()
        );
        sleep(POLL_INTERVAL);
    }

    assert!(
        resume_evidence.drain_observed,
        "forward bytes never drained below the cap"
    );
    assert!(
        resume_evidence.resumed_idle_observed,
        "demuxer input did not leave its capped idle state after the cache drained"
    );
    assert!(
        resume_evidence.fresh_positive_input_observed,
        "no positive input-rate cache refill grew beyond the post-drain release floor"
    );
    assert!(!observed_underrun, "decoder cache underrun was observed");
    assert!(
        !observed_mid_play_cache_pause,
        "playback rebuffered after advancement began while draining/resuming the cache cap"
    );
    assert!(
        server.wait_for_requests(1, Duration::from_secs(3)),
        "real mpv should fetch the deterministic cache-cap route"
    );
    drop(player);
    drop(process);
}

#[test]
fn cache_cap_resume_evidence_requires_fresh_input_after_drain() {
    const CAPPED_FORWARD_BYTES: u64 = 512;
    let mut combined = CacheCapResumeEvidence::default();
    combined.observe(
        &PlayerTransportTelemetryUpdate {
            buffered_ahead_bytes: Some(CAPPED_FORWARD_BYTES / 2),
            demuxer_cache_idle: Some(false),
            input_rate_bytes_per_second: Some(10_000),
            ..PlayerTransportTelemetryUpdate::default()
        },
        CAPPED_FORWARD_BYTES,
    );
    assert!(combined.drain_observed);
    assert!(combined.resumed_idle_observed);
    assert!(
        !combined.fresh_positive_input_observed,
        "a positive input sample observed before source release must not count as resume evidence"
    );

    let mut evidence = CacheCapResumeEvidence::default();

    evidence.observe(
        &PlayerTransportTelemetryUpdate {
            demuxer_cache_idle: Some(false),
            input_rate_bytes_per_second: Some(10_000),
            ..PlayerTransportTelemetryUpdate::default()
        },
        CAPPED_FORWARD_BYTES,
    );
    assert!(evidence.resumed_idle_observed);
    assert!(!evidence.drain_observed);
    assert!(!evidence.fresh_positive_input_observed);

    evidence.observe(
        &PlayerTransportTelemetryUpdate {
            buffered_ahead_bytes: Some(CAPPED_FORWARD_BYTES / 2),
            ..PlayerTransportTelemetryUpdate::default()
        },
        CAPPED_FORWARD_BYTES,
    );
    assert!(evidence.drain_observed);
    assert!(!evidence.fresh_positive_input_observed);

    evidence.mark_source_released();
    let release_floor = evidence
        .release_forward_bytes
        .expect("the drain observation should establish a release floor");
    evidence.observe(
        &PlayerTransportTelemetryUpdate {
            input_rate_bytes_per_second: Some(10_000),
            ..PlayerTransportTelemetryUpdate::default()
        },
        CAPPED_FORWARD_BYTES,
    );
    assert!(
        !evidence.fresh_positive_input_observed,
        "a positive rate without a post-release cache increase must not satisfy resume evidence"
    );

    evidence.observe(
        &PlayerTransportTelemetryUpdate {
            buffered_ahead_bytes: Some(release_floor + 1),
            ..PlayerTransportTelemetryUpdate::default()
        },
        CAPPED_FORWARD_BYTES,
    );
    assert!(
        !evidence.fresh_positive_input_observed,
        "a post-release cache increase without a positive input rate must not count"
    );

    evidence.observe(
        &PlayerTransportTelemetryUpdate {
            buffered_ahead_bytes: Some(release_floor + 2),
            input_rate_bytes_per_second: Some(10_000),
            ..PlayerTransportTelemetryUpdate::default()
        },
        CAPPED_FORWARD_BYTES,
    );
    assert!(evidence.fresh_positive_input_observed);
}

#[test]
#[ignore = "scheduled integration test; requires the mpv binary"]
fn real_mpv_clients_keep_seek_recovery_bounded_during_an_http_stall() {
    let media = pcm_wav(30);
    let server = FaultInjectingHttpServer::start(BTreeMap::from([
        (
            "/healthy.wav".to_owned(),
            HttpMediaFixture::static_bytes("audio/wav", media.clone()).with_faults(
                NetworkFaultProfile {
                    // Keep this route comfortably above the PCM consumption
                    // rate so any post-start recovery command is a genuine
                    // isolation failure, not an independent healthy-route
                    // underflow.
                    bytes_per_second: Some(1_000_000),
                    ..NetworkFaultProfile::default()
                },
            ),
        ),
        (
            "/stalling.wav".to_owned(),
            HttpMediaFixture::static_bytes("audio/wav", media).with_faults(NetworkFaultProfile {
                bytes_per_second: Some(220_000),
                burst_stalls: vec![BurstStall {
                    // Delay the injected fault until both clients have
                    // completed startup and their ordinary convergence
                    // episodes have closed.
                    after_body_bytes: 1_000_000,
                    duration: Duration::from_secs(4),
                }],
                ..NetworkFaultProfile::default()
            }),
        ),
    ]))
    .expect("fault-injecting HTTP server should start");

    let mut healthy = RealMpvClient::start(0, &server.url("/healthy.wav"));
    let mut stalling = RealMpvClient::start(1, &server.url("/stalling.wav"));
    let startup = Instant::now();
    while startup.elapsed() < SEMANTICS_TIMEOUT
        && (!healthy.post_start_baseline_ready() || !stalling.post_start_baseline_ready())
    {
        healthy.poll();
        stalling.poll();
        sleep(POLL_INTERVAL);
    }
    assert!(
        healthy.post_start_baseline_ready() && stalling.post_start_baseline_ready(),
        "clients did not establish a post-start transport/recovery-decision baseline before the injected stall; healthy={:?} healthy_recovery={:?} healthy_metrics={:?} stalling={:?} stalling_recovery={:?} stalling_metrics={:?}",
        healthy.latest_transport,
        healthy.coordinator.recovery_episode(),
        healthy.coordinator.metrics(),
        stalling.latest_transport,
        stalling.coordinator.recovery_episode(),
        stalling.coordinator.metrics(),
    );
    let healthy_startup_position_commands = healthy.begin_steady_state_observation_window();
    let stalling_startup_position_commands = stalling.begin_steady_state_observation_window();

    let started = Instant::now();
    while started.elapsed() < TEST_DURATION {
        healthy.poll();
        stalling.poll();
        if stalling.recovery_observed()
            && stalling
                .seconds_since_last_cache_pause()
                .is_some_and(|value| value >= 3.0)
        {
            break;
        }
        sleep(POLL_INTERVAL);
    }

    assert!(
        healthy.started_revisions.contains(&1),
        "healthy mpv client never produced an observation-backed start acknowledgment: {:?}",
        healthy.coordinator.metrics()
    );
    assert!(
        stalling.started_revisions.contains(&1),
        "stalling mpv client never produced an observation-backed start acknowledgment: {:?}",
        stalling.coordinator.metrics()
    );
    assert!(
        stalling.coordinator.metrics().buffer_episode_count >= 1,
        "the deterministic stall did not reach mpv's buffering telemetry: {:?}",
        stalling.coordinator.metrics()
    );
    assert!(
        stalling
            .position_commands_by_episode
            .values()
            .all(|count| *count <= 1),
        "one recovery episode emitted multiple seeks: {:?}",
        stalling.position_commands_by_episode
    );
    assert_eq!(
        healthy.position_commands_by_episode.values().sum::<usize>(),
        0,
        "a healthy peer emitted a recovery seek after both clients reached steady state; startup commands are excluded because current mpv may legitimately seek while releasing cache-pause-initial (healthy_startup={healthy_startup_position_commands:?}, stalling_startup={stalling_startup_position_commands:?}, healthy_post_start={:?}, healthy_transport={:?}, healthy_metrics={:?}, healthy_commands={:?})",
        healthy.position_commands_by_episode,
        healthy.latest_transport,
        healthy.coordinator.metrics(),
        healthy.executed_commands,
    );
    drop(healthy);
    drop(stalling);
    assert!(
        server.wait_for_requests(2, Duration::from_secs(1)),
        "both mpv clients should have fetched their independent media routes"
    );

    verify_authoritative_pause_latches_during_an_http_stall();
}

fn verify_barrier_start_then_ordinary_pause_with_real_mpv(media: &TemporaryMedia) {
    const PREPARE_REVISION: u64 = 1;
    const COMMIT_REVISION: u64 = 2;
    const ORDINARY_PAUSE_REVISION: u64 = 3;
    const PREPARE_TARGET_SECONDS: f64 = 1.0;
    const ORDINARY_PAUSE_TARGET_SECONDS: f64 = 3.0;

    let mut client = RealMpvClient::start_with_desired(
        10_001,
        &media.path_string(),
        MediaTransportKind::LocalFile,
        DesiredRoomPlayback {
            media_generation: 0,
            state_revision: PREPARE_REVISION,
            paused: true,
            anchor_position_seconds: PREPARE_TARGET_SECONDS,
            anchor_observed_at_seconds: 0.0,
            force_seek: true,
        },
        // The startup barrier owns this alignment. It is not a new ordinary
        // mid-play seek and must not create an unbuffered-seek episode.
        DesiredRoomPlaybackUpdateKind::Ordinary,
    );
    wait_for_real_mpv_client(
        &mut client,
        "barrier prepare revision applied while ReadyPaused",
        SEMANTICS_TIMEOUT,
        |state| {
            state.applied_revisions.contains(&PREPARE_REVISION)
                && state.latest_transport.phase == Some(PlayerTransportPhase::ReadyPaused)
                && state.latest_transport.logical_pause == Some(true)
                && state.latest_transport.core_idle == Some(true)
                && state
                    .latest_transport
                    .position_seconds
                    .is_some_and(|position| (position - PREPARE_TARGET_SECONDS).abs() <= 0.5)
        },
    );

    client.set_desired(COMMIT_REVISION, false, PREPARE_TARGET_SECONDS, false);
    wait_for_real_mpv_client(
        &mut client,
        "barrier commit reached observation-backed Started and completion eligibility",
        SEMANTICS_TIMEOUT,
        |state| {
            state.applied_revisions.contains(&COMMIT_REVISION)
                && state.started_revisions.contains(&COMMIT_REVISION)
                && state.latest_transport.logical_pause == Some(false)
                && state
                    .latest_transport
                    .position_seconds
                    .is_some_and(|position| position > PREPARE_TARGET_SECONDS + 0.01)
        },
    );
    // In a one-participant barrier, the observation-backed Started action is
    // the evidence the session/server layer uses to publish Complete. The
    // next desired state deliberately has no barrier metadata: it represents
    // the later ordinary server-authoritative room pause.
    let ordinary_pause_commands_start = client.executed_commands.len();
    client.set_desired(
        ORDINARY_PAUSE_REVISION,
        true,
        ORDINARY_PAUSE_TARGET_SECONDS,
        true,
    );
    wait_for_real_mpv_client(
        &mut client,
        "ordinary pause after completed barrier",
        SEMANTICS_TIMEOUT,
        |state| {
            state.applied_revisions.contains(&ORDINARY_PAUSE_REVISION)
                && state.latest_transport.phase == Some(PlayerTransportPhase::ReadyPaused)
                && state.latest_transport.logical_pause == Some(true)
                && state.latest_transport.core_idle == Some(true)
                && state
                    .latest_transport
                    .position_seconds
                    .is_some_and(|position| (position - ORDINARY_PAUSE_TARGET_SECONDS).abs() <= 0.5)
        },
    );

    let ordinary_pause_commands = &client.executed_commands[ordinary_pause_commands_start..];
    let seek_index = ordinary_pause_commands
        .iter()
        .position(|command| {
            matches!(
                command,
                CoordinatorPlayerCommand::SetPosition(position)
                    if (*position - ORDINARY_PAUSE_TARGET_SECONDS).abs() <= 0.5
            )
        })
        .expect("ordinary remote pause must reconcile the room position");
    let pause_index = ordinary_pause_commands
        .iter()
        .position(|command| matches!(command, CoordinatorPlayerCommand::SetPaused(true)))
        .expect("ordinary remote pause must pause the player");
    assert!(
        seek_index < pause_index,
        "ordinary remote pause must apply the room position before pausing: \
         {ordinary_pause_commands:?}"
    );
}

fn verify_one_bounded_rebuffer_episode_with_real_mpv() {
    let server = FaultInjectingHttpServer::start(BTreeMap::from([(
        "/pr-stall.wav".to_owned(),
        HttpMediaFixture::static_bytes("audio/wav", pcm_wav(16)).with_faults(NetworkFaultProfile {
            bytes_per_second: Some(150_000),
            burst_stalls: vec![BurstStall {
                after_body_bytes: 240_000,
                duration: Duration::from_secs(2),
            }],
            ..NetworkFaultProfile::default()
        }),
    )]))
    .expect("short PR fault server should start");
    let mut client = RealMpvClient::start(10_002, &server.url("/pr-stall.wav"));

    wait_for_real_mpv_client(
        &mut client,
        "one deterministic real-mpv rebuffer episode",
        PR_STALL_TIMEOUT,
        |state| {
            let metrics = state.coordinator.metrics();
            state.started_revisions.contains(&1)
                && state.observed_rebuffer
                && metrics.buffer_episode_count >= 1
                && state.latest_transport.paused_for_cache == Some(false)
                && metrics.hard_seek_count
                    + metrics.gentle_catchup_count
                    + metrics.degraded_recovery_count
                    >= 1
        },
    );

    assert_eq!(
        client.coordinator.metrics().buffer_episode_count,
        1,
        "the single injected burst stall should create one recovery episode: {:?}",
        client.coordinator.metrics()
    );
    assert!(
        client
            .position_commands_by_episode
            .values()
            .all(|count| *count <= 1),
        "the PR rebuffer episode emitted more than one recovery seek: {:?}",
        client.position_commands_by_episode
    );
    assert!(
        client.coordinator.metrics().hard_seek_count
            + client.coordinator.metrics().gentle_catchup_count
            >= 1,
        "the observed rebuffer must enter a bounded recovery strategy: {:?}",
        client.coordinator.metrics()
    );
    drop(client);
    assert!(
        server.wait_for_requests(1, Duration::from_secs(3)),
        "real mpv should fetch the short deterministic stall route"
    );
}

fn verify_mid_play_unbuffered_seek_is_bounded_with_real_mpv() {
    const SEEK_REVISION: u64 = 2;
    const SEEK_TARGET_SECONDS: f64 = 22.0;
    const ADVANCED_ROOM_TARGET_SECONDS: f64 = 24.0;

    let server = FaultInjectingHttpServer::start(BTreeMap::from([(
        "/unbuffered-seek.wav".to_owned(),
        HttpMediaFixture::static_bytes("audio/wav", pcm_wav(30)).with_faults(NetworkFaultProfile {
            // The bounded cache cannot reach the target during startup,
            // and the delayed range response keeps the preparation
            // episode observable before mpv can refill at the target.
            range_response_delay: Duration::from_millis(1_200),
            bytes_per_second: Some(180_000),
            ..NetworkFaultProfile::default()
        }),
    )]))
    .expect("unbuffered-seek HTTP server should start");
    let mut client = RealMpvClient::start(10_004, &server.url("/unbuffered-seek.wav"));

    wait_for_real_mpv_client(
        &mut client,
        "playing baseline with the explicit seek target outside mpv's cache",
        SEMANTICS_TIMEOUT,
        |state| {
            state.post_start_baseline_ready()
                && state
                    .latest_transport
                    .seekable_ranges
                    .as_ref()
                    .is_some_and(|ranges| {
                        !ranges.is_empty()
                            && ranges.iter().all(|range| {
                                SEEK_TARGET_SECONDS < range.start_seconds
                                    || SEEK_TARGET_SECONDS > range.end_seconds
                            })
                    })
        },
    );

    let command_start = client.executed_commands.len();
    client.set_desired(SEEK_REVISION, false, SEEK_TARGET_SECONDS, true);

    let initial = client
        .coordinator
        .seek_preparation_snapshot()
        .expect("an out-of-cache explicit VOD seek should enter preparation");
    assert_eq!(initial.availability, SeekTargetAvailability::FetchRequired);
    assert_eq!(initial.frozen_target_seconds, SEEK_TARGET_SECONDS);
    let preparation_id = initial.id;
    let primary_seek_count = client.executed_commands[command_start..]
        .iter()
        .filter(|command| {
            matches!(
                command,
                CoordinatorPlayerCommand::SetPosition(position)
                    if (*position - SEEK_TARGET_SECONDS).abs() <= 0.5
            )
        })
        .count();
    assert_eq!(
        primary_seek_count,
        1,
        "the explicit out-of-cache seek must issue exactly one primary seek: {:?}",
        &client.executed_commands[command_start..]
    );

    // A normal canonical update may advance while this client is fetching.
    // It must update convergence intent without replacing the frozen target
    // or issuing another primary seek.
    client.set_desired(SEEK_REVISION, false, ADVANCED_ROOM_TARGET_SECONDS, false);
    let retained = client
        .coordinator
        .seek_preparation_snapshot()
        .expect("ordinary room advancement must retain the active preparation");
    assert_eq!(retained.id, preparation_id);
    assert_eq!(retained.frozen_target_seconds, SEEK_TARGET_SECONDS);
    assert!(
        retained.latest_room_position_seconds >= ADVANCED_ROOM_TARGET_SECONDS,
        "the preparation should retain the latest convergence target: {retained:?}"
    );
    assert_eq!(
        client.executed_commands[command_start..]
            .iter()
            .filter(|command| matches!(command, CoordinatorPlayerCommand::SetPosition(_)))
            .count(),
        1,
        "advancing room time must not restart the unbuffered seek: {:?}",
        &client.executed_commands[command_start..]
    );

    wait_for_real_mpv_client(
        &mut client,
        "bounded unbuffered seek preparation and recovery",
        PR_STALL_TIMEOUT,
        |state| {
            state.coordinator.last_seek_preparation_terminal_outcome()
                == Some(SeekPreparationTerminalOutcome::Ready)
                && state.started_revisions.contains(&SEEK_REVISION)
                && state.latest_transport.paused_for_cache == Some(false)
                && state.latest_transport.seeking == Some(false)
        },
    );

    let terminal = client
        .coordinator
        .last_seek_preparation_terminal_snapshot()
        .expect("completed preparation should retain its terminal snapshot");
    assert_eq!(terminal.id, preparation_id);
    assert_eq!(terminal.frozen_target_seconds, SEEK_TARGET_SECONDS);
    assert_eq!(
        terminal.terminal_outcome,
        Some(SeekPreparationTerminalOutcome::Ready)
    );
    let position_commands = client.executed_commands[command_start..]
        .iter()
        .filter(|command| matches!(command, CoordinatorPlayerCommand::SetPosition(_)))
        .count();
    assert!(
        position_commands <= 2,
        "one primary seek and at most one recovery alignment are allowed: {:?}",
        &client.executed_commands[command_start..]
    );
    assert!(
        client
            .position_commands_by_episode
            .values()
            .all(|count| *count <= 1),
        "post-fetch recovery exceeded its one-seek budget: {:?}",
        client.position_commands_by_episode
    );

    drop(client);
    assert!(
        server.wait_for_requests(2, Duration::from_secs(3)),
        "mpv should issue a second HTTP request for the uncached target"
    );
    assert!(
        server.requests().iter().any(|request| {
            request.path == "/unbuffered-seek.wav"
                && request.status_code == 206
                && request.range_start.is_some_and(|start| start > 1_000_000)
        }),
        "the explicit seek should fetch a nonzero range near the distant target: {:?}",
        server.requests()
    );
}

fn verify_authoritative_pause_latches_during_an_http_stall() {
    const PAUSE_REVISION: u64 = 2;
    let server = FaultInjectingHttpServer::start(BTreeMap::from([(
        "/pause-stall.wav".to_owned(),
        HttpMediaFixture::static_bytes("audio/wav", pcm_wav(20)).with_faults(NetworkFaultProfile {
            bytes_per_second: Some(115_000),
            burst_stalls: vec![BurstStall {
                after_body_bytes: 360_000,
                duration: Duration::from_secs(4),
            }],
            ..NetworkFaultProfile::default()
        }),
    )]))
    .expect("authoritative-pause fault server should start");
    let mut client = RealMpvClient::start(10_003, &server.url("/pause-stall.wav"));

    wait_for_real_mpv_client(
        &mut client,
        "injected HTTP stall after observation-backed start",
        TEST_DURATION,
        |state| {
            state.started_revisions.contains(&1)
                && state.latest_transport.paused_for_cache == Some(true)
                && state.latest_transport.logical_pause != Some(true)
        },
    );

    let pause_request_position = client
        .latest_transport
        .position_seconds
        .expect("the injected stall should retain a transport position");
    let pause_target = pause_request_position + 0.5;
    let command_start = client.executed_commands.len();
    let history_start = client.transport_history.len();
    client.set_desired(PAUSE_REVISION, true, pause_target, true);

    let commands_during_stall = &client.executed_commands[command_start..];
    assert!(
        matches!(
            commands_during_stall.first(),
            Some(CoordinatorPlayerCommand::SetPaused(true))
        ),
        "authoritative pause must be dispatched synchronously while cache-paused: \
         {commands_during_stall:?}"
    );
    assert!(
        !commands_during_stall
            .iter()
            .any(|command| matches!(command, CoordinatorPlayerCommand::SetPosition(_))),
        "the forced position correction must remain deferred during cache pause: \
         {commands_during_stall:?}"
    );

    wait_for_real_mpv_client(
        &mut client,
        "cache release into latched logical pause and deferred seek completion",
        TEST_DURATION,
        |state| {
            state.applied_revisions.contains(&PAUSE_REVISION)
                && state.latest_transport.paused_for_cache == Some(false)
                && state.latest_transport.logical_pause == Some(true)
                && state
                    .latest_transport
                    .position_seconds
                    .is_some_and(|position| (position - pause_target).abs() <= 0.5)
        },
    );

    let post_request_history = &client.transport_history[history_start..];
    let cache_release_index = post_request_history
        .iter()
        .position(|state| state.paused_for_cache == Some(false))
        .expect("the injected HTTP stall should release its cache pause");
    let logical_pause_index = post_request_history
        .iter()
        .position(|state| state.logical_pause == Some(true))
        .expect("cache release should expose the latched logical pause");
    assert!(
        logical_pause_index >= cache_release_index,
        "logical pause must not be inferred from cache pause"
    );
    let release_position = post_request_history[cache_release_index]
        .position_seconds
        .unwrap_or(pause_request_position);
    let maximum_position_before_logical_pause = post_request_history
        [cache_release_index..logical_pause_index]
        .iter()
        .filter_map(|state| state.position_seconds)
        .fold(release_position, f64::max);
    assert!(
        maximum_position_before_logical_pause - release_position <= 0.01,
        "mpv advanced after cache release before logical pause was established: \
         release={release_position}, maximum={maximum_position_before_logical_pause}, \
         history={:?}",
        &post_request_history[cache_release_index..=logical_pause_index]
    );

    let completed_commands = &client.executed_commands[command_start..];
    let pause_index = completed_commands
        .iter()
        .position(|command| matches!(command, CoordinatorPlayerCommand::SetPaused(true)))
        .expect("authoritative pause command should remain recorded");
    let seek_index = completed_commands
        .iter()
        .position(|command| matches!(command, CoordinatorPlayerCommand::SetPosition(_)))
        .expect("forced seek should execute after cache release");
    assert!(
        pause_index < seek_index,
        "pause must latch before the deferred seek: {completed_commands:?}"
    );
    drop(client);
    assert!(
        server.wait_for_requests(1, Duration::from_secs(3)),
        "real mpv should fetch the authoritative-pause stall route"
    );
}

fn wait_for_real_mpv_client(
    client: &mut RealMpvClient,
    description: &str,
    timeout: Duration,
    predicate: impl Fn(&RealMpvClient) -> bool,
) {
    let deadline = Instant::now() + timeout;
    loop {
        client.poll();
        if predicate(client) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{description} timed out; latest transport={:?}, applied={:?}, started={:?}, \
             commands={:?}, metrics={:?}",
            client.latest_transport,
            client.applied_revisions,
            client.started_revisions,
            client.executed_commands,
            client.coordinator.metrics()
        );
        sleep(POLL_INTERVAL);
    }
}

fn pause_and_wait(
    player: &mut impl PlayerAdapter,
    observed: &mut PlayerTransportTelemetryUpdate,
    description: &str,
) {
    let command_id = player
        .execute_tracked(PlayerCommand::SetPaused(true))
        .unwrap_or_else(|error| panic!("{description}: mpv rejected pause: {error}"));
    wait_for_completed_command(player, observed, command_id, description);
    wait_for_observation(player, observed, description, |state| {
        state.phase == Some(PlayerTransportPhase::ReadyPaused)
            && state.logical_pause == Some(true)
            && state.paused_for_cache == Some(false)
            && state.core_idle == Some(true)
    });
}

fn seek_and_wait(
    player: &mut impl PlayerAdapter,
    observed: &mut PlayerTransportTelemetryUpdate,
    target_seconds: f64,
    description: &str,
) {
    let command_id = player
        .execute_tracked(PlayerCommand::SetPosition(target_seconds))
        .unwrap_or_else(|error| panic!("{description}: mpv rejected seek: {error}"));
    wait_for_completed_command(player, observed, command_id, description);
    wait_for_observation(player, observed, description, |state| {
        state.seeking == Some(false)
            && state
                .position_seconds
                .is_some_and(|position| (position - target_seconds).abs() <= 0.5)
    });
}

fn wait_for_completed_command(
    player: &mut impl PlayerAdapter,
    observed: &mut PlayerTransportTelemetryUpdate,
    command_id: PlayerCommandId,
    description: &str,
) {
    let deadline = Instant::now() + SEMANTICS_TIMEOUT;
    let mut accepted = false;
    loop {
        drain_transport_updates(player, observed);
        while let Some(progress) = player.take_command_progress() {
            if progress.command_id != command_id {
                continue;
            }
            match progress.state {
                PlayerCommandProgressState::Accepted => accepted = true,
                PlayerCommandProgressState::Finished(PlayerCommandResult::Completed) => {
                    drain_transport_updates(player, observed);
                    return;
                }
                PlayerCommandProgressState::Finished(result) => {
                    panic!(
                        "{description}: tracked command {command_id:?} failed with {result:?}; \
                         latest transport observation: {observed:?}"
                    );
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "{description}: tracked command {command_id:?} timed out (accepted={accepted}); \
             latest transport observation: {observed:?}"
        );
        sleep(POLL_INTERVAL);
    }
}

fn wait_for_observation(
    player: &mut impl PlayerAdapter,
    observed: &mut PlayerTransportTelemetryUpdate,
    description: &str,
    predicate: impl Fn(&PlayerTransportTelemetryUpdate) -> bool,
) {
    let deadline = Instant::now() + SEMANTICS_TIMEOUT;
    loop {
        drain_transport_updates(player, observed);
        if predicate(observed) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{description}: expected transport state was not observed: {observed:?}"
        );
        sleep(POLL_INTERVAL);
    }
}

fn drain_transport_updates(
    player: &mut impl PlayerAdapter,
    observed: &mut PlayerTransportTelemetryUpdate,
) {
    // Production GUI/CLI polling also asks for local-file metadata. That IPC
    // round trip pumps mpv's asynchronous event stream into the adapter.
    let _ = player.take_local_file_update();
    while let Some(update) = player.take_transport_telemetry_update() {
        if std::env::var_os("SOROTTE_MPV_INTEGRATION_DEBUG").is_some() {
            eprintln!("mpv transport update: {update:?}");
        }
        merge_current_generation(observed, update);
    }
}

fn drain_cache_updates(
    player: &mut impl PlayerAdapter,
    observed: &mut PlayerCacheTelemetryUpdate,
) -> Vec<PlayerCacheTelemetryUpdate> {
    let mut updates = Vec::new();
    while let Some(update) = player.take_cache_telemetry_update() {
        if std::env::var_os("SOROTTE_MPV_INTEGRATION_DEBUG").is_some() {
            eprintln!("mpv cache telemetry update: {update:?}");
        }
        if update.media_generation.is_some() && observed.media_generation != update.media_generation
        {
            *observed = PlayerCacheTelemetryUpdate::default();
        }
        *observed = update.clone();
        updates.push(update);
    }
    updates
}

#[derive(Clone, Debug)]
struct TransportHistorySample {
    update: PlayerTransportTelemetryUpdate,
    state_after: PlayerTransportTelemetryUpdate,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CacheCapResumeEvidence {
    drain_observed: bool,
    resumed_idle_observed: bool,
    drained_forward_bytes: Option<u64>,
    release_forward_bytes: Option<u64>,
    source_released: bool,
    fresh_positive_input_observed: bool,
}

impl CacheCapResumeEvidence {
    fn observe(&mut self, update: &PlayerTransportTelemetryUpdate, capped_forward_bytes: u64) {
        if let Some(bytes) = update.buffered_ahead_bytes
            && bytes <= capped_forward_bytes / 2
        {
            self.drain_observed = true;
            self.drained_forward_bytes = Some(
                self.drained_forward_bytes
                    .map_or(bytes, |previous| previous.min(bytes)),
            );
        }
        if update.demuxer_cache_idle == Some(false) {
            self.resumed_idle_observed = true;
        }
        if self.source_released
            && update.buffered_ahead_bytes.is_some_and(|bytes| {
                self.release_forward_bytes
                    .is_some_and(|release_bytes| bytes > release_bytes)
            })
            && update
                .input_rate_bytes_per_second
                .is_some_and(|bytes_per_second| bytes_per_second > 0)
        {
            self.fresh_positive_input_observed = true;
        }
    }

    fn mark_source_released(&mut self) {
        assert!(
            self.drain_observed && self.resumed_idle_observed,
            "the deterministic source must remain held until the cache visibly drains and leaves its capped idle state"
        );
        self.release_forward_bytes = self.drained_forward_bytes;
        self.source_released = true;
    }
}

fn drain_transport_history(
    player: &mut impl PlayerAdapter,
    observed: &mut PlayerTransportTelemetryUpdate,
    history: &mut Vec<TransportHistorySample>,
) {
    let _ = player.take_local_file_update();
    while let Some(update) = player.take_transport_telemetry_update() {
        if std::env::var_os("SOROTTE_MPV_INTEGRATION_DEBUG").is_some() {
            eprintln!("mpv cache-cap transport update: {update:?}");
        }
        merge_current_generation(observed, update.clone());
        history.push(TransportHistorySample {
            update,
            state_after: observed.clone(),
        });
    }
}

fn merge_current_generation(
    observed: &mut PlayerTransportTelemetryUpdate,
    update: PlayerTransportTelemetryUpdate,
) {
    if update.media_generation.is_some() && observed.media_generation != update.media_generation {
        *observed = PlayerTransportTelemetryUpdate::default();
    }
    observed.merge_from(update);
}

struct TemporaryMedia {
    path: PathBuf,
}

impl TemporaryMedia {
    fn wav(duration_seconds: u32) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "sorotte-mpv-semantics-{}-{unique}.wav",
            std::process::id()
        ));
        std::fs::write(&path, pcm_wav(duration_seconds))
            .expect("deterministic local WAV fixture should be written");
        Self { path }
    }

    fn path_string(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }
}

impl Drop for TemporaryMedia {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

struct RealMpvClient {
    _process: MpvProcess,
    player: MpvAdapter,
    coordinator: PlaybackCoordinator,
    coordinator_generation: u64,
    adapter_generation: Option<u64>,
    player_commands: HashMap<PlayerCommandId, CoordinatorCommandId>,
    position_commands_by_episode: BTreeMap<u64, usize>,
    executed_commands: Vec<CoordinatorPlayerCommand>,
    applied_revisions: BTreeSet<u64>,
    started_revisions: BTreeSet<u64>,
    latest_transport: PlayerTransportTelemetryUpdate,
    transport_history: Vec<PlayerTransportTelemetryUpdate>,
    observed_rebuffer: bool,
    last_cache_pause_at: Option<f64>,
    clock_started: Instant,
}

impl RealMpvClient {
    fn start(index: usize, url: &str) -> Self {
        Self::start_with_desired(
            index,
            url,
            MediaTransportKind::NetworkVod,
            DesiredRoomPlayback {
                media_generation: 0,
                state_revision: 1,
                paused: false,
                anchor_position_seconds: 0.0,
                anchor_observed_at_seconds: 0.0,
                force_seek: false,
            },
            DesiredRoomPlaybackUpdateKind::Ordinary,
        )
    }

    fn start_with_desired(
        index: usize,
        target: &str,
        transport_kind: MediaTransportKind,
        mut desired: DesiredRoomPlayback,
        desired_update_kind: DesiredRoomPlaybackUpdateKind,
    ) -> Self {
        let (process, player) = MpvProcess::start(index);
        let mut player = player.into_inner();
        // Exercise Sorotte's network `loadfile` options against the minimum
        // supported real mpv binary used by the required CI harness.
        player.configure_network_media_options([
            ("cache", "yes"),
            ("cache-pause", "yes"),
            ("cache-pause-initial", "yes"),
            ("cache-pause-wait", "0.3"),
            ("cache-secs", "1"),
            ("demuxer-max-bytes", "524288"),
            ("demuxer-max-back-bytes", "65536"),
        ]);
        player
            .execute(PlayerCommand::SetPaused(true))
            .expect("coordinator real-mpv fixture should begin intentionally paused");
        player
            .execute_tracked(PlayerCommand::OpenFile(target.to_owned()))
            .expect("mpv should accept the deterministic HTTP fixture");

        let config = PlaybackCoordinatorConfig {
            negligible_lag_seconds: 0.25,
            hard_seek_threshold_seconds: 1.5,
            maximum_catchup_rate: 1.03,
            maximum_hard_seeks_per_episode: 1,
            stability_interval_seconds: 2.0,
            command_timeout_seconds: 8.0,
            ..PlaybackCoordinatorConfig::default()
        };
        let mut coordinator = PlaybackCoordinator::new(config);
        let coordinator_generation = coordinator
            .prepare_media(
                LogicalMediaId::new(format!("real-mpv-fixture-{index}"))
                    .expect("generated logical media ID should be valid"),
                transport_kind,
                0.0,
            )
            .media_generation;
        desired.media_generation = coordinator_generation;
        coordinator.update_desired_room_state_with_kind(desired, desired_update_kind);

        Self {
            _process: process,
            player,
            coordinator,
            coordinator_generation,
            adapter_generation: None,
            player_commands: HashMap::new(),
            position_commands_by_episode: BTreeMap::new(),
            executed_commands: Vec::new(),
            applied_revisions: BTreeSet::new(),
            started_revisions: BTreeSet::new(),
            latest_transport: PlayerTransportTelemetryUpdate::default(),
            transport_history: Vec::new(),
            observed_rebuffer: false,
            last_cache_pause_at: None,
            clock_started: Instant::now(),
        }
    }

    fn poll(&mut self) {
        // Match the production polling loop so asynchronous mpv events are
        // read while no tracked command is actively awaiting an IPC reply.
        let _ = self.player.take_local_file_update();
        while let Some(update) = self.player.take_transport_telemetry_update() {
            if std::env::var_os("SOROTTE_MPV_INTEGRATION_DEBUG").is_some() {
                eprintln!("coordinator mpv transport update: {update:?}");
            }
            if let Some(generation) = update.media_generation {
                let generation = generation.get();
                if self.adapter_generation.is_none() {
                    self.adapter_generation = Some(generation);
                }
                if self.adapter_generation != Some(generation) {
                    continue;
                }
            }
            if update.paused_for_cache == Some(true)
                || update.phase == Some(PlayerTransportPhase::Rebuffering)
            {
                self.observed_rebuffer = true;
                if update.paused_for_cache == Some(true) {
                    self.last_cache_pause_at = update
                        .observed_at
                        .map(|timestamp| timestamp.elapsed_since_adapter_start().as_secs_f64());
                }
            }
            let observation = self.coordinator_observation(update.clone());
            merge_current_generation(&mut self.latest_transport, update);
            self.transport_history.push(self.latest_transport.clone());
            if let Some(observation) = observation {
                let actions = self.coordinator.observe(observation);
                self.execute_actions(actions);
            }
        }

        while let Some(progress) = self.player.take_command_progress() {
            let Some(coordinator_id) = self.player_commands.get(&progress.command_id).copied()
            else {
                continue;
            };
            if let PlayerCommandProgressState::Finished(result) = progress.state {
                self.player_commands.remove(&progress.command_id);
                if !matches!(
                    result,
                    PlayerCommandResult::Completed | PlayerCommandResult::Superseded
                ) {
                    self.coordinator
                        .command_failed(coordinator_id, self.now_seconds());
                }
            }
        }

        let actions = self.coordinator.tick(self.now_seconds());
        self.execute_actions(actions);
    }

    fn coordinator_observation(
        &self,
        update: PlayerTransportTelemetryUpdate,
    ) -> Option<PlayerTransportObservation> {
        let observed_at_seconds = update
            .observed_at?
            .elapsed_since_adapter_start()
            .as_secs_f64();
        Some(PlayerTransportObservation {
            media_generation: self.coordinator_generation,
            observed_at_seconds,
            phase: update.phase,
            position_seconds: update.position_seconds,
            playback_rate: update.playback_rate,
            logical_pause: update.logical_pause,
            paused_for_cache: update.paused_for_cache,
            seeking: update.seeking,
            seekable: update.seekable,
            timeline_kind: update.timeline_kind,
            seekable_ranges: update.seekable_ranges,
            known_live_seekable_window: update.known_live_seekable_window,
            core_idle: update.core_idle,
            playback_restart_sequence: update.playback_restart_sequence,
            cache_buffering_percent: update.cache_buffering_percent,
            buffered_ahead_seconds: update.buffered_ahead_seconds,
            input_rate_bytes_per_second: update.input_rate_bytes_per_second,
        })
    }

    fn execute_actions(&mut self, actions: Vec<PlaybackCoordinatorAction>) {
        for action in actions {
            match action {
                PlaybackCoordinatorAction::Execute {
                    command_id,
                    command,
                } => self.execute_command(command_id, command),
                PlaybackCoordinatorAction::Started { state_revision, .. } => {
                    self.started_revisions.insert(state_revision);
                }
                PlaybackCoordinatorAction::RevisionApplied { state_revision, .. } => {
                    self.applied_revisions.insert(state_revision);
                }
                PlaybackCoordinatorAction::RequestRoomPause { .. }
                | PlaybackCoordinatorAction::Degraded { .. }
                | PlaybackCoordinatorAction::CommandTimedOut { .. } => {}
            }
        }
    }

    fn execute_command(
        &mut self,
        coordinator_id: CoordinatorCommandId,
        command: CoordinatorPlayerCommand,
    ) {
        self.executed_commands.push(command);
        let player_command = match command {
            CoordinatorPlayerCommand::SetPaused(paused) => PlayerCommand::SetPaused(paused),
            CoordinatorPlayerCommand::Play(intent) => PlayerCommand::Play(intent),
            CoordinatorPlayerCommand::SetPosition(position) => {
                if let Some(episode) = self.coordinator.recovery_episode() {
                    *self
                        .position_commands_by_episode
                        .entry(episode.id)
                        .or_default() += 1;
                }
                PlayerCommand::SetPosition(position)
            }
            CoordinatorPlayerCommand::SetPlaybackRate(rate) => {
                if self
                    .player
                    .execute(PlayerCommand::SetPlaybackRate(rate))
                    .is_ok()
                {
                    self.coordinator.command_accepted(coordinator_id);
                } else {
                    self.coordinator
                        .command_failed(coordinator_id, self.now_seconds());
                }
                return;
            }
        };

        match self.player.execute_tracked(player_command) {
            Ok(player_id) => {
                self.player_commands.insert(player_id, coordinator_id);
                self.coordinator.command_accepted(coordinator_id);
            }
            Err(_) => {
                self.coordinator
                    .command_failed(coordinator_id, self.now_seconds());
            }
        }
    }

    fn recovery_observed(&self) -> bool {
        self.observed_rebuffer && !self.position_commands_by_episode.is_empty()
    }

    fn post_start_baseline_ready(&self) -> bool {
        let recovery_decision_observed =
            self.coordinator.recovery_episode().is_none_or(|episode| {
                episode.catchup_deadline_seconds.is_some()
                    || episode.hard_seek_attempts > 0
                    || episode.stable_since_seconds.is_some()
                    || episode.degraded
            });
        self.started_revisions.contains(&1)
            && recovery_decision_observed
            && self.latest_transport.phase == Some(PlayerTransportPhase::Playing)
            && self.latest_transport.logical_pause == Some(false)
            && self.latest_transport.paused_for_cache == Some(false)
            && self.latest_transport.seeking == Some(false)
    }

    fn begin_steady_state_observation_window(&mut self) -> BTreeMap<u64, usize> {
        self.observed_rebuffer = false;
        self.last_cache_pause_at = None;
        std::mem::take(&mut self.position_commands_by_episode)
    }

    fn seconds_since_last_cache_pause(&self) -> Option<f64> {
        self.last_cache_pause_at
            .map(|last| (self.now_seconds() - last).max(0.0))
    }

    fn now_seconds(&self) -> f64 {
        self.clock_started.elapsed().as_secs_f64()
    }

    fn set_desired(
        &mut self,
        state_revision: u64,
        paused: bool,
        position_seconds: f64,
        force_seek: bool,
    ) {
        let update_kind = if force_seek {
            DesiredRoomPlaybackUpdateKind::ExplicitSeek
        } else {
            DesiredRoomPlaybackUpdateKind::Ordinary
        };
        let mut actions = self.coordinator.update_desired_room_state_with_kind(
            DesiredRoomPlayback {
                media_generation: self.coordinator_generation,
                state_revision,
                paused,
                anchor_position_seconds: position_seconds,
                anchor_observed_at_seconds: self.now_seconds(),
                force_seek,
            },
            update_kind,
        );
        if let Some(observation) = self.coordinator_observation(self.latest_transport.clone()) {
            actions.extend(self.coordinator.observe(observation));
        }
        self.execute_actions(actions);
    }
}

struct MpvProcess {
    child: Child,
    socket: PathBuf,
}

impl MpvProcess {
    fn start(index: usize) -> (Self, ConnectedMpvPlayer) {
        Self::start_with_cache_options(
            index,
            &[
                ("cache", "yes"),
                ("cache-pause", "yes"),
                ("cache-pause-initial", "yes"),
                ("cache-pause-wait", "0.3"),
                ("cache-secs", "1"),
                ("demuxer-max-bytes", "524288"),
                ("demuxer-max-back-bytes", "65536"),
            ],
        )
    }

    fn start_with_cache_options(
        index: usize,
        cache_options: &[(&str, &str)],
    ) -> (Self, ConnectedMpvPlayer) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after Unix epoch")
            .as_nanos();
        let socket = std::env::temp_dir().join(format!(
            "sorotte-mpv-rebuffer-{}-{index}-{unique}.sock",
            std::process::id()
        ));
        let mpv = std::env::var_os("SOROTTE_MPV_INTEGRATION_BIN").unwrap_or_else(|| "mpv".into());
        let stderr = if std::env::var_os("SOROTTE_MPV_INTEGRATION_DEBUG").is_some() {
            Stdio::inherit()
        } else {
            Stdio::null()
        };
        let mut command = Command::new(mpv);
        command
            .arg("--no-config")
            .arg("--idle=yes")
            .arg("--pause=yes")
            .arg("--force-window=no")
            .arg("--video=no")
            .arg("--audio-display=no")
            .arg("--ao=null");
        for (name, value) in cache_options {
            command.arg(format!("--{name}={value}"));
        }
        let child = command
            .arg(format!("--input-ipc-server={}", socket.display()))
            .stdout(Stdio::null())
            .stderr(stderr)
            .spawn()
            .expect("scheduled rebuffer test requires an mpv binary");
        let process = Self { child, socket };

        let started = Instant::now();
        let player = loop {
            match ConnectedMpvPlayer::connect(&process.socket) {
                Ok(player) => break player,
                Err(error) if started.elapsed() < Duration::from_secs(5) => {
                    let _ = error;
                    sleep(Duration::from_millis(25));
                }
                Err(error) => panic!(
                    "mpv JSON IPC socket did not become ready at {}: {error}",
                    process.socket.display()
                ),
            }
        };
        (process, player)
    }
}

impl Drop for MpvProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.socket);
    }
}

fn pcm_wav(duration_seconds: u32) -> Vec<u8> {
    const SAMPLE_RATE: u32 = 48_000;
    const CHANNELS: u16 = 1;
    const BITS_PER_SAMPLE: u16 = 16;
    let bytes_per_sample = u32::from(BITS_PER_SAMPLE / 8) * u32::from(CHANNELS);
    let data_bytes = SAMPLE_RATE
        .saturating_mul(duration_seconds)
        .saturating_mul(bytes_per_sample);
    let mut wav = Vec::with_capacity(44 + data_bytes as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36_u32.saturating_add(data_bytes)).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&CHANNELS.to_le_bytes());
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&(SAMPLE_RATE.saturating_mul(bytes_per_sample)).to_le_bytes());
    wav.extend_from_slice(&(bytes_per_sample as u16).to_le_bytes());
    wav.extend_from_slice(&BITS_PER_SAMPLE.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_bytes.to_le_bytes());
    wav.resize(44 + data_bytes as usize, 0);
    wav
}
