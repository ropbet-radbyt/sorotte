#![cfg(unix)]

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sorotte_client_core::{
    CoordinatorCommandId, CoordinatorPlayerCommand, DesiredRoomPlayback, LogicalMediaId,
    MediaTransportKind, PlaybackCoordinator, PlaybackCoordinatorAction, PlaybackCoordinatorConfig,
    PlayerTransportObservation,
};
use sorotte_player_api::{
    PlayerAdapter, PlayerCommand, PlayerCommandId, PlayerCommandProgressState, PlayerCommandResult,
    PlayerTransportTelemetryUpdate,
};
use sorotte_player_mpv::ConnectedMpvPlayer;
use sorotte_sim::{BurstStall, FaultInjectingHttpServer, HttpMediaFixture, NetworkFaultProfile};

const TEST_DURATION: Duration = Duration::from_secs(20);
const POLL_INTERVAL: Duration = Duration::from_millis(20);

#[test]
#[ignore = "scheduled integration test; requires the mpv binary"]
fn real_mpv_clients_keep_seek_recovery_bounded_during_an_http_stall() {
    let media = pcm_wav(30);
    let server = FaultInjectingHttpServer::start(BTreeMap::from([
        (
            "/healthy.wav".to_owned(),
            HttpMediaFixture::static_bytes("audio/wav", media.clone()).with_faults(
                NetworkFaultProfile {
                    bytes_per_second: Some(180_000),
                    ..NetworkFaultProfile::default()
                },
            ),
        ),
        (
            "/stalling.wav".to_owned(),
            HttpMediaFixture::static_bytes("audio/wav", media).with_faults(NetworkFaultProfile {
                bytes_per_second: Some(115_000),
                burst_stalls: vec![BurstStall {
                    after_body_bytes: 360_000,
                    duration: Duration::from_secs(4),
                }],
                ..NetworkFaultProfile::default()
            }),
        ),
    ]))
    .expect("fault-injecting HTTP server should start");

    let mut healthy = RealMpvClient::start(0, &server.url("/healthy.wav"));
    let mut stalling = RealMpvClient::start(1, &server.url("/stalling.wav"));
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
        "a healthy peer should not inherit another client's recovery commands"
    );
    assert!(
        server.wait_for_requests(2, Duration::from_secs(1)),
        "both mpv clients should have fetched their independent media routes"
    );
}

struct RealMpvClient {
    _process: MpvProcess,
    player: ConnectedMpvPlayer,
    coordinator: PlaybackCoordinator,
    coordinator_generation: u64,
    adapter_generation: Option<u64>,
    player_commands: HashMap<PlayerCommandId, CoordinatorCommandId>,
    position_commands_by_episode: BTreeMap<u64, usize>,
    started_revisions: BTreeSet<u64>,
    observed_rebuffer: bool,
    last_cache_pause_at: Option<f64>,
    clock_started: Instant,
}

impl RealMpvClient {
    fn start(index: usize, url: &str) -> Self {
        let (process, mut player) = MpvProcess::start(index);
        player
            .execute_tracked(PlayerCommand::OpenFile(url.to_owned()))
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
                MediaTransportKind::NetworkVod,
                0.0,
            )
            .media_generation;
        coordinator.update_desired_room_state(DesiredRoomPlayback {
            media_generation: coordinator_generation,
            state_revision: 1,
            paused: false,
            anchor_position_seconds: 0.0,
            anchor_observed_at_seconds: 0.0,
            force_seek: false,
        });

        Self {
            _process: process,
            player,
            coordinator,
            coordinator_generation,
            adapter_generation: None,
            player_commands: HashMap::new(),
            position_commands_by_episode: BTreeMap::new(),
            started_revisions: BTreeSet::new(),
            observed_rebuffer: false,
            last_cache_pause_at: None,
            clock_started: Instant::now(),
        }
    }

    fn poll(&mut self) {
        while let Some(update) = self.player.take_transport_telemetry_update() {
            if let Some(generation) = update.media_generation {
                let generation = generation.get();
                if self.adapter_generation.is_none() {
                    self.adapter_generation = Some(generation);
                }
                if self.adapter_generation != Some(generation) {
                    continue;
                }
            }
            if update.paused_for_cache == Some(true) {
                self.observed_rebuffer = true;
                self.last_cache_pause_at = update
                    .observed_at
                    .map(|timestamp| timestamp.elapsed_since_adapter_start().as_secs_f64());
            }
            if let Some(observation) = self.coordinator_observation(update) {
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
            seekable_ranges: update.seekable_ranges,
            core_idle: update.core_idle,
            playback_restart_sequence: update.playback_restart_sequence,
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
                PlaybackCoordinatorAction::RevisionApplied { .. }
                | PlaybackCoordinatorAction::RequestRoomPause { .. }
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
        let player_command = match command {
            CoordinatorPlayerCommand::SetPaused(paused) => PlayerCommand::SetPaused(paused),
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

    fn seconds_since_last_cache_pause(&self) -> Option<f64> {
        self.last_cache_pause_at
            .map(|last| (self.now_seconds() - last).max(0.0))
    }

    fn now_seconds(&self) -> f64 {
        self.clock_started.elapsed().as_secs_f64()
    }
}

struct MpvProcess {
    child: Child,
    socket: PathBuf,
}

impl MpvProcess {
    fn start(index: usize) -> (Self, ConnectedMpvPlayer) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after Unix epoch")
            .as_nanos();
        let socket = std::env::temp_dir().join(format!(
            "sorotte-mpv-rebuffer-{}-{index}-{unique}.sock",
            std::process::id()
        ));
        let mpv = std::env::var_os("SOROTTE_MPV_INTEGRATION_BIN").unwrap_or_else(|| "mpv".into());
        let child = Command::new(mpv)
            .arg("--no-config")
            .arg("--idle=yes")
            .arg("--pause=yes")
            .arg("--force-window=no")
            .arg("--video=no")
            .arg("--audio-display=no")
            .arg("--ao=null")
            .arg("--cache=yes")
            .arg("--cache-pause=yes")
            .arg("--cache-pause-initial=yes")
            .arg("--cache-pause-wait=0.3")
            .arg("--cache-secs=1")
            .arg("--demuxer-max-bytes=524288")
            .arg("--demuxer-max-back-bytes=65536")
            .arg(format!("--input-ipc-server={}", socket.display()))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
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
