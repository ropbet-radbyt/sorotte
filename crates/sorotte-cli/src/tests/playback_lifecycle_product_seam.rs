use super::*;
use sorotte_protocol::{
    MixedReadinessPolicy, ParticipantPlaybackPhase, ParticipantPlayerConnection,
    ParticipantStatusAvailability, ParticipantStatusSnapshotMode, PlaybackBarrierPolicy,
    PlaylistChangePayload, PlaylistIndexPayload, SetPayload, UserReadinessIntent,
};
use sorotte_server::{
    ServerActorHandle, ServerRuntime, run_server_network_loops_and_shutdown_actor,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;

type LifecycleClientResult = anyhow::Result<(ClientApplication<MpvAdapter>, ConnectedSessionExit)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FaultProxyCommand {
    CloseAndHold,
    Resume,
    Shutdown,
}

async fn run_lifecycle_fault_proxy(
    listener: TcpListener,
    upstream_address: std::net::SocketAddr,
    mut commands: UnboundedReceiver<FaultProxyCommand>,
    downstream_accepted: UnboundedSender<u64>,
    upstream_connected: UnboundedSender<u64>,
) -> anyhow::Result<()> {
    let mut connection_generation = 0_u64;
    let mut hold_upstream = false;
    loop {
        let (mut downstream, _) = loop {
            tokio::select! {
                command = commands.recv() => match command {
                    Some(FaultProxyCommand::CloseAndHold) => hold_upstream = true,
                    Some(FaultProxyCommand::Resume) => hold_upstream = false,
                    Some(FaultProxyCommand::Shutdown) | None => return Ok(()),
                },
                accepted = listener.accept() => break accepted?,
            }
        };
        connection_generation = connection_generation.saturating_add(1);
        downstream_accepted
            .send(connection_generation)
            .map_err(|_| anyhow::anyhow!("fault proxy acceptance observer dropped"))?;

        while hold_upstream {
            match commands.recv().await {
                Some(FaultProxyCommand::Resume) => hold_upstream = false,
                Some(FaultProxyCommand::CloseAndHold) => {}
                Some(FaultProxyCommand::Shutdown) | None => return Ok(()),
            }
        }

        let mut upstream = TcpStream::connect(upstream_address).await?;
        upstream_connected
            .send(connection_generation)
            .map_err(|_| anyhow::anyhow!("fault proxy upstream observer dropped"))?;
        let relay = tokio::io::copy_bidirectional(&mut downstream, &mut upstream);
        tokio::pin!(relay);
        loop {
            tokio::select! {
                relay_result = &mut relay => {
                    if let Err(error) = relay_result
                        && !matches!(
                            error.kind(),
                            std::io::ErrorKind::ConnectionReset
                                | std::io::ErrorKind::ConnectionAborted
                                | std::io::ErrorKind::BrokenPipe
                                | std::io::ErrorKind::UnexpectedEof
                                | std::io::ErrorKind::NotConnected
                        )
                    {
                        return Err(error.into());
                    }
                    // A reset is a normal terminal observation for this
                    // fault-injection relay: the test deliberately severs an
                    // established stream and later shuts the proxy down.
                    break;
                }
                command = commands.recv() => match command {
                    Some(FaultProxyCommand::CloseAndHold) => {
                        hold_upstream = true;
                        break;
                    }
                    Some(FaultProxyCommand::Resume) => {}
                    Some(FaultProxyCommand::Shutdown) | None => return Ok(()),
                },
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct LifecycleClientOptions {
    initially_paused: bool,
    max_connected_runtime_seconds: f64,
    readiness_supported: bool,
    ready_at_start: bool,
    local_can_control: bool,
    coordinated_start_policy: Option<PlaybackBarrierPolicy>,
}

impl LifecycleClientOptions {
    const fn legacy_compatible(initially_paused: bool) -> Self {
        Self {
            initially_paused,
            max_connected_runtime_seconds: 2.5,
            readiness_supported: false,
            ready_at_start: false,
            local_can_control: true,
            coordinated_start_policy: None,
        }
    }

    const fn with_duration(mut self, max_connected_runtime_seconds: f64) -> Self {
        self.max_connected_runtime_seconds = max_connected_runtime_seconds;
        self
    }
}

fn launch_lifecycle_client(
    address: std::net::SocketAddr,
    username: &'static str,
    initially_paused: bool,
) -> (
    UnboundedSender<String>,
    Arc<AtomicBool>,
    JoinHandle<LifecycleClientResult>,
) {
    launch_lifecycle_client_with_options(
        address,
        username,
        LifecycleClientOptions::legacy_compatible(initially_paused),
    )
}

fn launch_lifecycle_client_for_duration(
    address: std::net::SocketAddr,
    username: &'static str,
    initially_paused: bool,
    max_connected_runtime_seconds: f64,
) -> (
    UnboundedSender<String>,
    Arc<AtomicBool>,
    JoinHandle<LifecycleClientResult>,
) {
    launch_lifecycle_client_with_options(
        address,
        username,
        LifecycleClientOptions::legacy_compatible(initially_paused)
            .with_duration(max_connected_runtime_seconds),
    )
}

fn launch_lifecycle_client_with_options(
    address: std::net::SocketAddr,
    username: &'static str,
    options: LifecycleClientOptions,
) -> (
    UnboundedSender<String>,
    Arc<AtomicBool>,
    JoinHandle<LifecycleClientResult>,
) {
    let (command_tx, mut command_rx) = unbounded_channel::<String>();
    let natural_eof_trigger = Arc::new(AtomicBool::new(false));
    let client_natural_eof_trigger = Arc::clone(&natural_eof_trigger);
    let task = tokio::spawn(async move {
        let config = ClientLoopConfig {
            host: address.ip().to_string(),
            port: address.port(),
            username: username.to_owned(),
            room: "lifecycle-room".to_owned(),
            max_retries: 0,
            max_connected_runtime_seconds: options.max_connected_runtime_seconds,
            readiness_supported_override: Some(options.readiness_supported),
            ready_at_start_override: Some(options.ready_at_start),
            local_can_control_override: Some(options.local_can_control),
            unpause_action_override: Some(UnpauseActionMode::Always),
            ..test_client_loop_config()
        };
        let mut runtime = create_client_runtime(&config);
        if let Some(policy) = options.coordinated_start_policy {
            runtime.set_playback_barrier_start_config(
                sorotte_client_core::PlaybackBarrierStartConfig {
                    policy: Some(policy),
                    ..sorotte_client_core::PlaybackBarrierStartConfig::default()
                },
            );
        }
        runtime.with_player_io(|player| {
            player.set_test_simulated_natural_eof_trigger(client_natural_eof_trigger)
        });
        let media_path = "lifecycle-media.mkv";
        runtime.player_mut().open_file(media_path)?;
        runtime.player_mut().set_position(0.0)?;
        runtime.player_mut().set_paused(options.initially_paused)?;
        runtime
            .session_mut()
            .apply_player_playback_telemetry_update(
                &PlayerPlaybackTelemetryUpdate::default()
                    .with_position_seconds(0.0)
                    .with_paused(options.initially_paused),
            );
        if options.coordinated_start_policy.is_some() {
            let logical_id = sorotte_client_core::logical_media_id_for_local_file_update(
                &sorotte_player_api::LocalFileUpdate::new(media_path),
            );
            runtime.prepare_playback_media_with_intent(
                logical_id,
                sorotte_client_core::MediaTransportKind::LocalFile,
                sorotte_protocol::MediaLoadIntent::NewPlayback,
                client_runtime_now_seconds(),
            );
        }
        let stream = TcpStream::connect(address).await?;
        let mut notification_sink = ignore_autoplay_notification;
        let mut file_difference_sink = ignore_file_difference_notification;
        let exit = run_connected_client_session(
            stream,
            &mut runtime,
            &config,
            None,
            Some(&mut command_rx),
            &mut notification_sink,
            &mut file_difference_sink,
        )
        .await?;
        Ok((runtime, exit))
    });
    (command_tx, natural_eof_trigger, task)
}

async fn wait_for_server_session(server: &ServerActorHandle, client_id: &str, username: &str) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if server
                .session(client_id)
                .await
                .expect("server actor should answer session probes")
                .is_some_and(|session| session.username == username)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{username} should complete the production server handshake"));
}

async fn connect_canonical_observer(
    address: std::net::SocketAddr,
) -> (tokio::io::Lines<BufReader<OwnedReadHalf>>, OwnedWriteHalf) {
    let stream = TcpStream::connect(address)
        .await
        .expect("canonical observer should connect");
    let (reader, mut writer) = stream.into_split();
    let hello = encode_message_line(&ProtocolMessage::hello_basic(
        "observer",
        "lifecycle-room",
        "1.7.5",
    ))
    .expect("observer Hello should encode");
    writer
        .write_all(format!("{hello}\r\n").as_bytes())
        .await
        .expect("observer Hello should write");
    writer.flush().await.expect("observer Hello should flush");

    let mut lines = BufReader::new(reader).lines();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let line = lines
                .next_line()
                .await
                .expect("observer should read the server handshake")
                .expect("server should remain connected during the handshake");
            if matches!(
                decode_message_line(&line).expect("server handshake line should decode"),
                ProtocolMessage::Hello(_)
            ) {
                return;
            }
        }
    })
    .await
    .expect("observer should receive the production server Hello");
    (lines, writer)
}

async fn wait_for_canonical_transport(
    lines: &mut tokio::io::Lines<BufReader<OwnedReadHalf>>,
    paused: bool,
) -> Result<PlaystatePayload, String> {
    wait_for_canonical_transport_by(lines, paused, "controller").await
}

async fn wait_for_canonical_transport_by(
    lines: &mut tokio::io::Lines<BufReader<OwnedReadHalf>>,
    paused: bool,
    expected_user: &str,
) -> Result<PlaystatePayload, String> {
    let mut observed_playstates = Vec::new();
    let result = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let line = lines
                .next_line()
                .await
                .expect("observer should read canonical server state")
                .expect("server should stay connected until canonical state is observed");
            let message = decode_message_line(&line).expect("canonical server line should decode");
            if let ProtocolMessage::State(state) = message
                && let Some(playstate) = state.state.playstate
            {
                observed_playstates.push(format!(
                    "paused={:?}, position={:?}, do_seek={:?}, set_by={:?}",
                    playstate.paused,
                    playstate.position,
                    playstate.do_seek,
                    playstate.set_by.as_deref()
                ));
                if playstate.paused == Some(paused)
                    && playstate.set_by.as_deref() == Some(expected_user)
                {
                    return playstate;
                }
            }
        }
    })
    .await;
    result.map_err(|_| {
        format!(
            "production server should canonically commit {expected_user} paused={paused}; observed server playstates: {observed_playstates:?}"
        )
    })
}

async fn assert_canonical_transport_stays_paused_for(
    lines: &mut tokio::io::Lines<BufReader<OwnedReadHalf>>,
    duration: Duration,
) {
    let deadline = tokio::time::Instant::now() + duration;
    let mut observed_playstates = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return;
        }
        let line = match tokio::time::timeout(remaining, lines.next_line()).await {
            Err(_) => return,
            Ok(result) => result
                .expect("observer should read canonical server state")
                .expect("server should remain connected while the readiness gate holds"),
        };
        let message = decode_message_line(&line).expect("canonical server line should decode");
        if let ProtocolMessage::State(state) = message
            && let Some(playstate) = state.state.playstate
        {
            observed_playstates.push(format!(
                "paused={:?}, position={:?}, do_seek={:?}, set_by={:?}",
                playstate.paused,
                playstate.position,
                playstate.do_seek,
                playstate.set_by.as_deref()
            ));
            assert_ne!(
                playstate.paused,
                Some(false),
                "the canonical room unpaused before every required participant was ready: {observed_playstates:?}"
            );
        }
    }
}

async fn assert_canonical_transport_does_not_regress_for(
    lines: &mut tokio::io::Lines<BufReader<OwnedReadHalf>>,
    minimum_position_seconds: f64,
    duration: Duration,
) {
    let deadline = tokio::time::Instant::now() + duration;
    let mut observed_playstates = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return;
        }
        let line = match tokio::time::timeout(remaining, lines.next_line()).await {
            Err(_) => return,
            Ok(result) => result
                .expect("observer should read canonical server state")
                .expect("server should remain connected during reconnect convergence"),
        };
        let message = decode_message_line(&line).expect("canonical server line should decode");
        if let ProtocolMessage::State(state) = message
            && let Some(playstate) = state.state.playstate
        {
            observed_playstates.push(format!(
                "paused={:?}, position={:?}, do_seek={:?}, set_by={:?}",
                playstate.paused,
                playstate.position,
                playstate.do_seek,
                playstate.set_by.as_deref()
            ));
            assert_ne!(
                playstate.paused,
                Some(true),
                "reconnecting stale player evidence re-paused canonical playback: {observed_playstates:?}"
            );
            if let Some(position) = playstate.position {
                assert!(
                    position + 0.5 >= minimum_position_seconds,
                    "reconnecting stale player evidence rewound canonical playback below {minimum_position_seconds}: {observed_playstates:?}"
                );
            }
        }
    }
}

async fn wait_for_canonical_seek(
    lines: &mut tokio::io::Lines<BufReader<OwnedReadHalf>>,
    expected_position_seconds: f64,
) -> Result<PlaystatePayload, String> {
    wait_for_canonical_seek_by(lines, expected_position_seconds, "controller").await
}

async fn wait_for_canonical_seek_by(
    lines: &mut tokio::io::Lines<BufReader<OwnedReadHalf>>,
    expected_position_seconds: f64,
    expected_user: &str,
) -> Result<PlaystatePayload, String> {
    let mut observed_playstates = Vec::new();
    let result = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let line = lines
                .next_line()
                .await
                .expect("observer should read canonical server state")
                .expect("server should stay connected until canonical seek is observed");
            let message = decode_message_line(&line).expect("canonical server line should decode");
            if let ProtocolMessage::State(state) = message
                && let Some(playstate) = state.state.playstate
            {
                observed_playstates.push(format!(
                    "paused={:?}, position={:?}, do_seek={:?}, set_by={:?}",
                    playstate.paused,
                    playstate.position,
                    playstate.do_seek,
                    playstate.set_by.as_deref()
                ));
                if playstate
                    .position
                    .is_some_and(|position| (position - expected_position_seconds).abs() <= 0.05)
                    && playstate.set_by.as_deref() == Some(expected_user)
                {
                    return playstate;
                }
            }
        }
    })
    .await;
    result.map_err(|_| {
        format!(
            "production server should canonically commit {expected_user} seek={expected_position_seconds}; observed server playstates: {observed_playstates:?}"
        )
    })
}

async fn write_observer_protocol_message(writer: &mut OwnedWriteHalf, message: ProtocolMessage) {
    let line = encode_message_line(&message).expect("observer protocol message should encode");
    writer
        .write_all(format!("{line}\r\n").as_bytes())
        .await
        .expect("observer protocol message should write");
    writer
        .flush()
        .await
        .expect("observer protocol message should flush");
}

async fn acknowledge_observer_server_counter(
    server: &ServerActorHandle,
    client_id: &str,
    writer: &mut OwnedWriteHalf,
) {
    let counter = server
        .server_ignoring_counter(client_id)
        .await
        .expect("server actor should expose the raw peer state counter");
    if counter > 0 {
        write_observer_protocol_message(
            writer,
            ProtocolMessage::state(
                StatePayload::new()
                    .with_ignoring_on_the_fly(IgnoringOnTheFlyPayload::new().with_server(counter)),
            ),
        )
        .await;
    }
}

async fn wait_for_canonical_playlist_index(
    lines: &mut tokio::io::Lines<BufReader<OwnedReadHalf>>,
    expected_index: i64,
    expected_user: &str,
) -> Result<(), String> {
    let mut observed = Vec::new();
    let result = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let line = lines
                .next_line()
                .await
                .expect("observer should read canonical playlist state")
                .expect("server should stay connected until playlist state is observed");
            let message = decode_message_line(&line).expect("canonical server line should decode");
            if let ProtocolMessage::Set(set) = message
                && let Some(index) = set.set.playlist_index
            {
                observed.push(format!(
                    "index={}, user={:?}",
                    index.index,
                    index.user.as_deref()
                ));
                if index.index == expected_index && index.user.as_deref() == Some(expected_user) {
                    return;
                }
            }
        }
    })
    .await;
    result.map_err(|_| {
        format!(
            "production server should canonically commit playlist index={expected_index} by {expected_user}; observed playlist indices: {observed:?}"
        )
    })
}

async fn assert_local_command_converges_through_production_server(
    command: &'static str,
    initial_paused: bool,
    expected_paused: bool,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("production server listener should bind");
    let address = listener
        .local_addr()
        .expect("production server listener should have an address");
    let mut server_runtime = ServerRuntime::new();
    server_runtime.set_readiness_enabled(false);
    let server = ServerActorHandle::spawn(server_runtime);
    let server_probe = server.clone();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let server_task = tokio::spawn(run_server_network_loops_and_shutdown_actor(
        vec![listener],
        server,
        None,
        shutdown_rx,
    ));

    let (controller_tx, _controller_eof, controller_task) =
        launch_lifecycle_client(address, "controller", initial_paused);
    wait_for_server_session(&server_probe, "client-1", "controller").await;
    let (mut observer_lines, _observer_writer) = connect_canonical_observer(address).await;

    controller_tx
        .send(command.to_owned())
        .expect("controller command should enter the connected-session input channel");
    let canonical = wait_for_canonical_transport(&mut observer_lines, expected_paused).await;

    // Join only after the controller's state is canonical. This makes the
    // second client a follower rather than a competing legacy authority, and
    // starting it opposite proves that the server state reaches its player.
    let (_follower_tx, _follower_eof, follower_task) =
        launch_lifecycle_client(address, "follower", !expected_paused);
    wait_for_server_session(&server_probe, "client-3", "follower").await;

    let (controller, controller_exit) =
        tokio::time::timeout(Duration::from_secs(4), controller_task)
            .await
            .expect("controller should finish its bounded connected session")
            .expect("controller task should join")
            .expect("controller connected session should succeed");
    let (follower, follower_exit) = tokio::time::timeout(Duration::from_secs(4), follower_task)
        .await
        .expect("follower should finish its bounded connected session")
        .expect("follower task should join")
        .expect("follower connected session should succeed");

    shutdown_tx
        .send(true)
        .expect("production server shutdown signal should send");
    tokio::time::timeout(Duration::from_secs(3), server_task)
        .await
        .expect("production server should shut down within its grace deadline")
        .expect("production server task should join")
        .expect("production server and actor should shut down cleanly");

    let canonical = canonical.unwrap_or_else(|diagnostic| {
        panic!(
            "{diagnostic}; final controller player paused={}, room={:?}, coordination={:?}; final follower player paused={}, room={:?}, coordination={:?}",
            controller.player().paused(),
            controller.session().current_room_playstate(),
            controller.playback_coordination_snapshot(),
            follower.player().paused(),
            follower.session().current_room_playstate(),
            follower.playback_coordination_snapshot()
        )
    });
    assert_eq!(canonical.paused, Some(expected_paused));
    assert_eq!(canonical.set_by.as_deref(), Some("controller"));
    for exit in [controller_exit, follower_exit] {
        assert_eq!(exit, ConnectedSessionExit::RuntimeWindowElapsed);
    }
    assert_eq!(
        controller.player().paused(),
        expected_paused,
        "controller player; controller room={:?}; follower player={}; follower room={:?}",
        controller.session().current_room_playstate(),
        follower.player().paused(),
        follower.session().current_room_playstate()
    );
    assert_eq!(
        follower.player().paused(),
        expected_paused,
        "follower player; controller room={:?}; follower room={:?}; controller coordination={:?}; follower coordination={:?}",
        controller.session().current_room_playstate(),
        follower.session().current_room_playstate(),
        controller.playback_coordination_snapshot(),
        follower.playback_coordination_snapshot(),
    );
    for (role, runtime) in [("controller", &controller), ("follower", &follower)] {
        let room = runtime
            .session()
            .current_room_playstate()
            .unwrap_or_else(|| panic!("{role} should retain canonical room state"));
        assert_eq!(
            room.paused,
            Some(expected_paused),
            "{role} paused state; final room projection: {room:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn legacy_local_pause_and_play_cross_the_server_authority_boundary_and_converge() {
    assert_local_command_converges_through_production_server("pause", false, true).await;
    assert_local_command_converges_through_production_server("play", true, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn readiness_gate_holds_for_a_delayed_member_and_includes_a_late_joiner_before_commit() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("production server listener should bind");
    let address = listener
        .local_addr()
        .expect("production server listener should have an address");
    let mut server_runtime = ServerRuntime::new();
    server_runtime.set_readiness_enabled(true);
    // The raw observer is test instrumentation rather than a playback
    // participant. Every actual client in this scenario supports both V2
    // readiness and the playback barrier and remains in the required cohort.
    server_runtime.set_mixed_readiness_policy(MixedReadinessPolicy::ExcludeLegacy);
    let server = ServerActorHandle::spawn(server_runtime);
    let server_probe = server.clone();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let server_task = tokio::spawn(run_server_network_loops_and_shutdown_actor(
        vec![listener],
        server,
        None,
        shutdown_rx,
    ));

    let delayed_options = LifecycleClientOptions {
        initially_paused: true,
        max_connected_runtime_seconds: 6.0,
        readiness_supported: true,
        ready_at_start: false,
        local_can_control: true,
        coordinated_start_policy: None,
    };
    let (delayed_tx, _delayed_eof, delayed_task) =
        launch_lifecycle_client_with_options(address, "delayed", delayed_options);
    wait_for_server_session(&server_probe, "client-1", "delayed").await;
    let (mut observer_lines, _observer_writer) = connect_canonical_observer(address).await;

    let controller_options = LifecycleClientOptions {
        initially_paused: true,
        max_connected_runtime_seconds: 5.5,
        readiness_supported: true,
        ready_at_start: true,
        local_can_control: true,
        coordinated_start_policy: Some(PlaybackBarrierPolicy::AllEligible),
    };
    let (_controller_tx, _controller_eof, controller_task) =
        launch_lifecycle_client_with_options(address, "controller", controller_options);
    wait_for_server_session(&server_probe, "client-3", "controller").await;
    wait_for_canonical_transport(&mut observer_lines, true)
        .await
        .unwrap_or_else(|diagnostic| panic!("{diagnostic}"));

    assert_canonical_transport_stays_paused_for(&mut observer_lines, Duration::from_millis(250))
        .await;

    // Join while the accepted generation is still Preparing. This player is
    // dynamically added to the required cohort, receives the complete media
    // and readiness snapshots, and must acknowledge current-generation
    // playability before the server may commit.
    let late_options = LifecycleClientOptions {
        initially_paused: true,
        max_connected_runtime_seconds: 3.0,
        readiness_supported: true,
        ready_at_start: true,
        local_can_control: true,
        coordinated_start_policy: None,
    };
    let (_late_tx, _late_eof, late_task) =
        launch_lifecycle_client_with_options(address, "late", late_options);
    wait_for_server_session(&server_probe, "client-4", "late").await;
    assert_canonical_transport_stays_paused_for(&mut observer_lines, Duration::from_millis(250))
        .await;

    delayed_tx.send("ready".to_owned()).expect(
        "the delayed participant's readiness intent should enter the production input channel",
    );
    let canonical = wait_for_canonical_transport(&mut observer_lines, false).await;

    let (late, late_exit) = tokio::time::timeout(Duration::from_secs(5), late_task)
        .await
        .expect("late participant should finish its bounded session")
        .expect("late participant task should join")
        .expect("late participant session should succeed");
    let (controller, controller_exit) =
        tokio::time::timeout(Duration::from_secs(5), controller_task)
            .await
            .expect("controller should finish its bounded session")
            .expect("controller task should join")
            .expect("controller session should succeed");
    let (delayed, delayed_exit) = tokio::time::timeout(Duration::from_secs(5), delayed_task)
        .await
        .expect("delayed participant should finish its bounded session")
        .expect("delayed participant task should join")
        .expect("delayed participant session should succeed");

    shutdown_tx
        .send(true)
        .expect("production server shutdown signal should send");
    tokio::time::timeout(Duration::from_secs(3), server_task)
        .await
        .expect("production server should shut down within its grace deadline")
        .expect("production server task should join")
        .expect("production server and actor should shut down cleanly");

    canonical.unwrap_or_else(|diagnostic| {
        panic!(
            "{diagnostic}; delayed player paused={}, room={:?}, readiness={:?}; controller player paused={}, room={:?}, readiness={:?}; late player paused={}, room={:?}, readiness={:?}",
            delayed.player().paused(),
            delayed.session().current_room_playstate(),
            delayed.session().readiness_snapshot(),
            controller.player().paused(),
            controller.session().current_room_playstate(),
            controller.session().readiness_snapshot(),
            late.player().paused(),
            late.session().current_room_playstate(),
            late.session().readiness_snapshot(),
        )
    });
    for exit in [delayed_exit, controller_exit, late_exit] {
        assert_eq!(exit, ConnectedSessionExit::RuntimeWindowElapsed);
    }
    for (role, runtime) in [
        ("delayed", &delayed),
        ("controller", &controller),
        ("late", &late),
    ] {
        assert!(
            !runtime.player().paused(),
            "{role} physical player did not apply the canonical start commit: room={:?}, coordination={:?}, readiness={:?}",
            runtime.session().current_room_playstate(),
            runtime.playback_coordination_snapshot(),
            runtime.session().readiness_snapshot(),
        );
        assert_eq!(
            runtime
                .session()
                .current_room_playstate()
                .and_then(|playstate| playstate.paused),
            Some(false),
            "{role} did not retain the canonical unpaused projection"
        );
    }
    let late_snapshot = late
        .session()
        .readiness_snapshot()
        .expect("late participant should receive a complete readiness snapshot");
    for username in ["delayed", "controller", "late"] {
        let participant = late_snapshot
            .participants
            .get(username)
            .unwrap_or_else(|| panic!("late snapshot should contain {username}"));
        assert_eq!(participant.user_intent, UserReadinessIntent::Ready);
        assert!(participant.room_ready, "{username} should be room-ready");
        assert!(
            participant.start_eligible,
            "{username} should be eligible for the committed generation"
        );
    }
    assert!(
        late.session().playback_barrier_commit().is_some(),
        "late participant should retain the committed start authority"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_reconnect_loop_catches_up_after_missing_pause_seek_and_start()
-> anyhow::Result<()> {
    const TARGET_SECONDS: f64 = 23.0;

    let server_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("production server listener should bind");
    let server_address = server_listener
        .local_addr()
        .expect("production server listener should have an address");
    let mut server_runtime = ServerRuntime::new();
    server_runtime.set_readiness_enabled(false);
    let server = ServerActorHandle::spawn(server_runtime);
    let server_probe = server.clone();
    let (server_shutdown_tx, server_shutdown_rx) = tokio::sync::watch::channel(false);
    let server_task = tokio::spawn(run_server_network_loops_and_shutdown_actor(
        vec![server_listener],
        server,
        None,
        server_shutdown_rx,
    ));

    let proxy_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("fault proxy listener should bind");
    let proxy_address = proxy_listener
        .local_addr()
        .expect("fault proxy should have an address");
    let (proxy_command_tx, proxy_command_rx) = unbounded_channel();
    let (downstream_accepted_tx, mut downstream_accepted_rx) = unbounded_channel();
    let (upstream_connected_tx, mut upstream_connected_rx) = unbounded_channel();
    let proxy_task = tokio::spawn(run_lifecycle_fault_proxy(
        proxy_listener,
        server_address,
        proxy_command_rx,
        downstream_accepted_tx,
        upstream_connected_tx,
    ));

    let (controller_tx, _controller_eof, controller_task) =
        launch_lifecycle_client_for_duration(server_address, "controller", true, 5.5);
    wait_for_server_session(&server_probe, "client-1", "controller").await;
    let (mut observer_lines, mut observer_writer) =
        connect_canonical_observer(server_address).await;
    controller_tx
        .send("play".to_owned())
        .expect("controller play should enter the production input channel");
    wait_for_canonical_transport(&mut observer_lines, false)
        .await
        .unwrap_or_else(|diagnostic| panic!("{diagnostic}"));

    let follower_config = ClientLoopConfig {
        host: proxy_address.ip().to_string(),
        port: proxy_address.port(),
        username: "reconnector".to_owned(),
        room: "lifecycle-room".to_owned(),
        max_retries: 3,
        max_connected_runtime_seconds: 2.5,
        readiness_supported_override: Some(false),
        local_can_control_override: Some(true),
        unpause_action_override: Some(UnpauseActionMode::Always),
        ..test_client_loop_config()
    };
    let mut follower_runtime = create_client_runtime(&follower_config);
    follower_runtime
        .player_mut()
        .open_file("lifecycle-media.mkv")?;
    follower_runtime.player_mut().set_position(0.0)?;
    follower_runtime.player_mut().set_paused(true)?;
    follower_runtime
        .session_mut()
        .apply_player_playback_telemetry_update(
            &PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(0.0)
                .with_paused(true),
        );
    let follower_task = tokio::spawn(async move {
        run_client_network_loop_with_prepared_runtime_for_test(
            &follower_config,
            follower_runtime,
            None,
        )
        .await
    });

    assert_eq!(
        tokio::time::timeout(Duration::from_secs(10), downstream_accepted_rx.recv())
            .await
            .expect("first proxy downstream connection should arrive"),
        Some(1)
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(10), upstream_connected_rx.recv())
            .await
            .expect("first proxy upstream connection should arrive"),
        Some(1)
    );
    wait_for_server_session(&server_probe, "client-3", "reconnector").await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    proxy_command_tx
        .send(FaultProxyCommand::CloseAndHold)
        .expect("fault proxy should accept the controlled cut");
    assert_eq!(
        // The retry delay itself is deterministic, but a full parallel
        // workspace run can starve this test runtime briefly. Keep the
        // assertion bounded without coupling it to an unloaded workstation.
        tokio::time::timeout(Duration::from_secs(10), downstream_accepted_rx.recv())
            .await
            .expect("the production retry loop should reconnect to the held proxy"),
        Some(2)
    );

    acknowledge_observer_server_counter(&server_probe, "client-2", &mut observer_writer).await;

    // The follower now has a live TCP connection to the proxy but no path to
    // the server. It therefore misses every delta below and can recover only
    // from authoritative reconnect snapshots.
    write_observer_protocol_message(
        &mut observer_writer,
        ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(TARGET_SECONDS)
                    .with_paused(false)
                    .with_do_seek(true),
            ),
        ),
    )
    .await;
    wait_for_canonical_seek_by(&mut observer_lines, TARGET_SECONDS, "observer")
        .await
        .unwrap_or_else(|diagnostic| panic!("{diagnostic}"));
    acknowledge_observer_server_counter(&server_probe, "client-2", &mut observer_writer).await;
    write_observer_protocol_message(
        &mut observer_writer,
        ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(TARGET_SECONDS)
                    .with_paused(true)
                    .with_do_seek(false),
            ),
        ),
    )
    .await;
    wait_for_canonical_transport_by(&mut observer_lines, true, "observer")
        .await
        .unwrap_or_else(|diagnostic| panic!("{diagnostic}"));
    acknowledge_observer_server_counter(&server_probe, "client-2", &mut observer_writer).await;
    write_observer_protocol_message(
        &mut observer_writer,
        ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(TARGET_SECONDS)
                    .with_paused(false)
                    .with_do_seek(false),
            ),
        ),
    )
    .await;
    wait_for_canonical_transport_by(&mut observer_lines, false, "observer")
        .await
        .unwrap_or_else(|diagnostic| panic!("{diagnostic}"));

    proxy_command_tx
        .send(FaultProxyCommand::Resume)
        .expect("fault proxy should resume the upstream path");
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), upstream_connected_rx.recv())
            .await
            .expect("second proxy upstream connection should arrive"),
        Some(2)
    );
    wait_for_server_session(&server_probe, "client-4", "reconnector").await;
    assert!(
        server_probe
            .session("client-3")
            .await
            .expect("server actor should answer retired-session probes")
            .is_none(),
        "the failed transport must not retain competing room membership"
    );
    assert_canonical_transport_does_not_regress_for(
        &mut observer_lines,
        TARGET_SECONDS,
        Duration::from_millis(350),
    )
    .await;

    let follower = tokio::time::timeout(Duration::from_secs(5), follower_task)
        .await
        .expect("reconnecting follower should reach its bounded normal exit")
        .expect("reconnecting follower task should join")
        .expect("production network retry loop should succeed");
    let (controller, controller_exit) =
        tokio::time::timeout(Duration::from_secs(4), controller_task)
            .await
            .expect("controller should finish its bounded session")
            .expect("controller task should join")
            .expect("controller session should succeed");

    proxy_command_tx
        .send(FaultProxyCommand::Shutdown)
        .expect("fault proxy should accept shutdown");
    tokio::time::timeout(Duration::from_secs(2), proxy_task)
        .await
        .expect("fault proxy should shut down")
        .expect("fault proxy task should join")
        .expect("fault proxy should exit cleanly");
    server_shutdown_tx
        .send(true)
        .expect("production server shutdown signal should send");
    tokio::time::timeout(Duration::from_secs(3), server_task)
        .await
        .expect("production server should shut down within its grace deadline")
        .expect("production server task should join")
        .expect("production server and actor should shut down cleanly");

    assert_eq!(controller_exit, ConnectedSessionExit::RuntimeWindowElapsed);
    assert!(
        !follower.player().paused(),
        "reconnected physical player missed the authoritative start: room={:?}, coordination={:?}",
        follower.session().current_room_playstate(),
        follower.playback_coordination_snapshot(),
    );
    let follower_position = follower.player().position_seconds();
    assert!(
        follower_position + 0.5 >= TARGET_SECONDS && follower_position <= TARGET_SECONDS + 5.0,
        "reconnected physical player did not catch up to the missed seek with plausible forward progress: player={}, room={:?}, coordination={:?}",
        follower_position,
        follower.session().current_room_playstate(),
        follower.playback_coordination_snapshot(),
    );
    assert_eq!(
        follower
            .session()
            .current_room_playstate()
            .and_then(|playstate| playstate.paused),
        Some(false),
        "the surviving client owner should retain canonical playing state"
    );
    assert_eq!(
        follower.player().current_path(),
        Some("lifecycle-media.mkv"),
        "reconnect must preserve the current physical media owner"
    );
    assert!(
        !controller.player().paused(),
        "the controller should remain on the same committed start"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_seek_crosses_server_authority_and_reaches_a_late_joining_player() {
    const TARGET_SECONDS: f64 = 18.0;

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("production server listener should bind");
    let address = listener
        .local_addr()
        .expect("production server listener should have an address");
    let mut server_runtime = ServerRuntime::new();
    server_runtime.set_readiness_enabled(false);
    let server = ServerActorHandle::spawn(server_runtime);
    let server_probe = server.clone();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let server_task = tokio::spawn(run_server_network_loops_and_shutdown_actor(
        vec![listener],
        server,
        None,
        shutdown_rx,
    ));

    let (controller_tx, _controller_eof, controller_task) =
        launch_lifecycle_client(address, "controller", true);
    wait_for_server_session(&server_probe, "client-1", "controller").await;
    let (mut observer_lines, _observer_writer) = connect_canonical_observer(address).await;

    controller_tx
        .send(format!("seek {TARGET_SECONDS}"))
        .expect("controller seek should enter the connected-session input channel");
    let canonical = wait_for_canonical_seek(&mut observer_lines, TARGET_SECONDS).await;

    let (_follower_tx, _follower_eof, follower_task) =
        launch_lifecycle_client(address, "follower", true);
    wait_for_server_session(&server_probe, "client-3", "follower").await;

    let (controller, controller_exit) =
        tokio::time::timeout(Duration::from_secs(4), controller_task)
            .await
            .expect("controller should finish its bounded connected session")
            .expect("controller task should join")
            .expect("controller connected session should succeed");
    let (follower, follower_exit) = tokio::time::timeout(Duration::from_secs(4), follower_task)
        .await
        .expect("follower should finish its bounded connected session")
        .expect("follower task should join")
        .expect("follower connected session should succeed");

    shutdown_tx
        .send(true)
        .expect("production server shutdown signal should send");
    tokio::time::timeout(Duration::from_secs(3), server_task)
        .await
        .expect("production server should shut down within its grace deadline")
        .expect("production server task should join")
        .expect("production server and actor should shut down cleanly");

    let canonical = canonical.unwrap_or_else(|diagnostic| {
        panic!(
            "{diagnostic}; final controller position={}, room={:?}, coordination={:?}; final follower position={}, room={:?}, coordination={:?}",
            controller.player().position_seconds(),
            controller.session().current_room_playstate(),
            controller.playback_coordination_snapshot(),
            follower.player().position_seconds(),
            follower.session().current_room_playstate(),
            follower.playback_coordination_snapshot()
        )
    });
    assert_eq!(canonical.set_by.as_deref(), Some("controller"));
    assert_eq!(
        canonical.do_seek,
        Some(false),
        "the server snapshot carries the committed seek position after normalizing the request edge"
    );
    for exit in [controller_exit, follower_exit] {
        assert_eq!(exit, ConnectedSessionExit::RuntimeWindowElapsed);
    }
    for (role, runtime) in [("controller", &controller), ("follower", &follower)] {
        assert!(
            (runtime.player().position_seconds() - TARGET_SECONDS).abs() <= 0.5,
            "{role} player did not converge to the canonical seek: player={}, room={:?}, coordination={:?}",
            runtime.player().position_seconds(),
            runtime.session().current_room_playstate(),
            runtime.playback_coordination_snapshot(),
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn canonical_playlist_selection_loads_the_same_item_in_existing_and_late_joining_players() {
    let playlist = vec![
        "lifecycle-media.mkv".to_owned(),
        "lifecycle-next.mkv".to_owned(),
    ];
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("production server listener should bind");
    let address = listener
        .local_addr()
        .expect("production server listener should have an address");
    let mut server_runtime = ServerRuntime::new();
    server_runtime.set_readiness_enabled(false);
    let server = ServerActorHandle::spawn(server_runtime);
    let server_probe = server.clone();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let server_task = tokio::spawn(run_server_network_loops_and_shutdown_actor(
        vec![listener],
        server,
        None,
        shutdown_rx,
    ));

    let (controller_tx, _controller_eof, controller_task) =
        launch_lifecycle_client(address, "controller", true);
    wait_for_server_session(&server_probe, "client-1", "controller").await;
    let (mut observer_lines, mut observer_writer) = connect_canonical_observer(address).await;
    write_observer_protocol_message(
        &mut observer_writer,
        ProtocolMessage::set(
            SetPayload::new().with_playlist_change(PlaylistChangePayload::new(playlist.clone())),
        ),
    )
    .await;
    write_observer_protocol_message(
        &mut observer_writer,
        ProtocolMessage::set(SetPayload::new().with_playlist_index(PlaylistIndexPayload::new(0))),
    )
    .await;
    wait_for_canonical_playlist_index(&mut observer_lines, 0, "observer")
        .await
        .unwrap_or_else(|diagnostic| panic!("{diagnostic}"));

    let (_follower_tx, _follower_eof, follower_task) =
        launch_lifecycle_client(address, "follower", true);
    wait_for_server_session(&server_probe, "client-3", "follower").await;
    controller_tx
        .send("next".to_owned())
        .expect("controller next should enter the connected-session input channel");
    let canonical = wait_for_canonical_playlist_index(&mut observer_lines, 1, "controller").await;
    // Join another player only after index 1 is canonical. The first playlist
    // index a client receives intentionally does not queue a rewind intent, so
    // this proves selected-media reconciliation is independent of that edge.
    let (_late_follower_tx, _late_follower_eof, late_follower_task) =
        launch_lifecycle_client(address, "late-follower", true);
    wait_for_server_session(&server_probe, "client-4", "late-follower").await;

    let (controller, controller_exit) =
        tokio::time::timeout(Duration::from_secs(4), controller_task)
            .await
            .expect("controller should finish its bounded connected session")
            .expect("controller task should join")
            .expect("controller connected session should succeed");
    let (follower, follower_exit) = tokio::time::timeout(Duration::from_secs(4), follower_task)
        .await
        .expect("follower should finish its bounded connected session")
        .expect("follower task should join")
        .expect("follower connected session should succeed");
    let (late_follower, late_follower_exit) =
        tokio::time::timeout(Duration::from_secs(4), late_follower_task)
            .await
            .expect("late follower should finish its bounded connected session")
            .expect("late follower task should join")
            .expect("late follower connected session should succeed");

    shutdown_tx
        .send(true)
        .expect("production server shutdown signal should send");
    tokio::time::timeout(Duration::from_secs(3), server_task)
        .await
        .expect("production server should shut down within its grace deadline")
        .expect("production server task should join")
        .expect("production server and actor should shut down cleanly");

    canonical.unwrap_or_else(|diagnostic| {
        panic!(
            "{diagnostic}; controller playlist={:?}, path={:?}; follower playlist={:?}, path={:?}; late follower playlist={:?}, path={:?}",
            controller.session().current_room_playlist(),
            controller.player().current_path(),
            follower.session().current_room_playlist(),
            follower.player().current_path(),
            late_follower.session().current_room_playlist(),
            late_follower.player().current_path(),
        )
    });
    for exit in [controller_exit, follower_exit, late_follower_exit] {
        assert_eq!(exit, ConnectedSessionExit::RuntimeWindowElapsed);
    }
    for (role, runtime) in [
        ("controller", &controller),
        ("follower", &follower),
        ("late follower", &late_follower),
    ] {
        let canonical_playlist = runtime
            .session()
            .current_room_playlist()
            .unwrap_or_else(|| panic!("{role} should retain canonical playlist authority"));
        assert_eq!(canonical_playlist.files, playlist, "{role} playlist files");
        assert_eq!(canonical_playlist.index, Some(1), "{role} playlist index");
        assert_eq!(
            runtime.player().current_path(),
            Some("lifecycle-next.mkv"),
            "{role} physical player selection"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn natural_eof_advances_canonical_playlist_and_loads_the_next_item_for_every_player() {
    let playlist = vec![
        "lifecycle-media.mkv".to_owned(),
        "lifecycle-next.mkv".to_owned(),
    ];
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("production server listener should bind");
    let address = listener
        .local_addr()
        .expect("production server listener should have an address");
    let mut server_runtime = ServerRuntime::new();
    server_runtime.set_readiness_enabled(false);
    let server = ServerActorHandle::spawn(server_runtime);
    let server_probe = server.clone();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let server_task = tokio::spawn(run_server_network_loops_and_shutdown_actor(
        vec![listener],
        server,
        None,
        shutdown_rx,
    ));

    let (_controller_tx, controller_eof, controller_task) =
        launch_lifecycle_client(address, "controller", false);
    wait_for_server_session(&server_probe, "client-1", "controller").await;
    let (mut observer_lines, mut observer_writer) = connect_canonical_observer(address).await;
    write_observer_protocol_message(
        &mut observer_writer,
        ProtocolMessage::set(
            SetPayload::new().with_playlist_change(PlaylistChangePayload::new(playlist.clone())),
        ),
    )
    .await;
    write_observer_protocol_message(
        &mut observer_writer,
        ProtocolMessage::set(SetPayload::new().with_playlist_index(PlaylistIndexPayload::new(0))),
    )
    .await;
    wait_for_canonical_playlist_index(&mut observer_lines, 0, "observer")
        .await
        .unwrap_or_else(|diagnostic| panic!("{diagnostic}"));

    let (_follower_tx, _follower_eof, follower_task) =
        launch_lifecycle_client(address, "follower", false);
    wait_for_server_session(&server_probe, "client-3", "follower").await;

    // The trigger is consumed by the adapter's ordinary maintenance cadence,
    // which produces the same correlated LogicalPlaybackTerminal event as a
    // real mpv end-file reason=eof observation.
    controller_eof.store(true, Ordering::SeqCst);
    wait_for_canonical_playlist_index(&mut observer_lines, 1, "controller")
        .await
        .unwrap_or_else(|diagnostic| panic!("{diagnostic}"));

    let (controller, controller_exit) =
        tokio::time::timeout(Duration::from_secs(4), controller_task)
            .await
            .expect("controller should finish its bounded connected session")
            .expect("controller task should join")
            .expect("controller connected session should succeed");
    let (follower, follower_exit) = tokio::time::timeout(Duration::from_secs(4), follower_task)
        .await
        .expect("follower should finish its bounded connected session")
        .expect("follower task should join")
        .expect("follower connected session should succeed");

    shutdown_tx
        .send(true)
        .expect("production server shutdown signal should send");
    tokio::time::timeout(Duration::from_secs(3), server_task)
        .await
        .expect("production server should shut down within its grace deadline")
        .expect("production server task should join")
        .expect("production server and actor should shut down cleanly");

    for exit in [controller_exit, follower_exit] {
        assert_eq!(exit, ConnectedSessionExit::RuntimeWindowElapsed);
    }
    for (role, runtime) in [("controller", &controller), ("follower", &follower)] {
        let canonical_playlist = runtime
            .session()
            .current_room_playlist()
            .unwrap_or_else(|| panic!("{role} should retain canonical playlist authority"));
        assert_eq!(canonical_playlist.files, playlist, "{role} playlist files");
        assert_eq!(canonical_playlist.index, Some(1), "{role} playlist index");
        assert_eq!(
            runtime.player().current_path(),
            Some("lifecycle-next.mkv"),
            "{role} physical player selection after natural EOF"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn participant_status_heartbeats_reach_a_late_client_and_withdraw_after_disconnect() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("production server listener should bind");
    let address = listener
        .local_addr()
        .expect("production server listener should have an address");
    let mut server_runtime = ServerRuntime::new();
    server_runtime.set_readiness_enabled(false);
    let server = ServerActorHandle::spawn(server_runtime);
    let server_probe = server.clone();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let server_task = tokio::spawn(run_server_network_loops_and_shutdown_actor(
        vec![listener],
        server,
        None,
        shutdown_rx,
    ));

    let (controller_tx, _controller_eof, controller_task) =
        launch_lifecycle_client_for_duration(address, "controller", true, 7.0);
    wait_for_server_session(&server_probe, "client-1", "controller").await;
    let (mut canonical_lines, canonical_writer) = connect_canonical_observer(address).await;
    controller_tx
        .send("play".to_owned())
        .expect("the explicit controller play should enter the production input channel");
    wait_for_canonical_transport(&mut canonical_lines, false)
        .await
        .unwrap_or_else(|diagnostic| panic!("{diagnostic}"));
    drop(canonical_writer);
    drop(canonical_lines);

    // This client joins after the reporting player is already active. Its
    // production protocol loop must obtain complete periodic snapshots; it
    // cannot depend on having observed the controller's initial transition.
    let (_observer_tx, _observer_eof, observer_task) =
        launch_lifecycle_client_for_duration(address, "status-observer", true, 2.5);
    wait_for_server_session(&server_probe, "client-3", "status-observer").await;
    // The session owns a 2.5 second observation window. Give a saturated
    // workspace runner enough scheduling margin while retaining a hard bound.
    let (observer, observer_exit) = tokio::time::timeout(Duration::from_secs(12), observer_task)
        .await
        .expect("status observer should finish its bounded session")
        .expect("status observer task should join")
        .expect("status observer session should succeed");
    assert_eq!(observer_exit, ConnectedSessionExit::RuntimeWindowElapsed);

    let observer_now = client_runtime_now_seconds();
    let controller_status = observer
        .session()
        .user_participant_status_at("controller", observer_now)
        .expect("late observer should receive the controller's complete status row");
    assert!(
        matches!(
            controller_status.freshness,
            sorotte_client_core::ClientParticipantStatusFreshness::Fresh
                | sorotte_client_core::ClientParticipantStatusFreshness::Delayed
        ),
        "a saturated test scheduler may age the final received snapshot, but current-epoch evidence must not already be stale: {:?}",
        controller_status.freshness,
    );
    assert_eq!(
        controller_status.status.availability,
        ParticipantStatusAvailability::Fresh
    );
    assert_eq!(
        controller_status.status.player_connection,
        Some(ParticipantPlayerConnection::Connected)
    );
    assert_eq!(
        controller_status.status.phase,
        Some(ParticipantPlaybackPhase::Playing)
    );
    assert_eq!(
        observer.session().participant_status_snapshot_mode(),
        ParticipantStatusSnapshotMode::Full
    );
    assert!(
        observer
            .session()
            .participant_status_snapshot_revision()
            .is_some_and(|revision| revision >= 3),
        "multiple complete heartbeat snapshots should advance the advisory snapshot revision: {:?}",
        observer.session().participant_status_snapshot_revision()
    );

    let (_controller, controller_exit) =
        tokio::time::timeout(Duration::from_secs(6), controller_task)
            .await
            .expect("controller should finish its bounded session")
            .expect("controller task should join")
            .expect("controller session should succeed");
    assert_eq!(controller_exit, ConnectedSessionExit::RuntimeWindowElapsed);

    // Join after the reporting connection has gone away. Membership and the
    // complete status snapshot must both omit the retired evidence epoch.
    let (_post_tx, _post_eof, post_disconnect_task) =
        launch_lifecycle_client_for_duration(address, "post-disconnect", true, 1.5);
    wait_for_server_session(&server_probe, "client-4", "post-disconnect").await;
    let (post_disconnect, post_disconnect_exit) =
        tokio::time::timeout(Duration::from_secs(3), post_disconnect_task)
            .await
            .expect("post-disconnect observer should finish")
            .expect("post-disconnect observer task should join")
            .expect("post-disconnect observer session should succeed");
    assert_eq!(
        post_disconnect_exit,
        ConnectedSessionExit::RuntimeWindowElapsed
    );
    assert_eq!(
        post_disconnect
            .session()
            .user_participant_status_v1_supported("controller"),
        None,
        "retired membership must not advertise a stale capability"
    );
    assert!(
        post_disconnect
            .session()
            .user_participant_status_at("controller", client_runtime_now_seconds())
            .is_none(),
        "retired participant status must be absent from a late complete snapshot"
    );

    shutdown_tx
        .send(true)
        .expect("production server shutdown signal should send");
    tokio::time::timeout(Duration::from_secs(3), server_task)
        .await
        .expect("production server should shut down within its grace deadline")
        .expect("production server task should join")
        .expect("production server and actor should shut down cleanly");
}

#[test]
fn loaded_simulated_mpv_events_survive_the_first_owner_clock_observation() {
    let config = ClientLoopConfig {
        username: "startup-owner".to_owned(),
        room: "lifecycle-room".to_owned(),
        readiness_supported_override: Some(false),
        ..test_client_loop_config()
    };
    let mut runtime = create_client_runtime(&config);
    runtime
        .player_mut()
        .open_file("lifecycle-startup-owner.mkv")
        .expect("simulated mpv should load before network ownership starts");
    runtime
        .player_mut()
        .set_position(4.0)
        .expect("simulated mpv should accept the initial position");
    runtime
        .player_mut()
        .set_paused(false)
        .expect("simulated mpv should accept the initial transport state");

    assert!(
        runtime
            .publish_pending_local_file_update_legacy_compatible(
                config.filename_privacy_mode,
                config.filesize_privacy_mode,
            )
            .expect("startup file publication should drain and publish ordered player events"),
        "the simulated load should publish one startup file identity"
    );
    assert!(
        runtime
            .playback_coordination_snapshot()
            .transport_telemetry_observed,
        "assigning the real logical file identity must retain the current physical generation's evidence"
    );

    let _ = runtime
        .synchronize_player_availability(10.0)
        .expect("the initial owner-clock observation should succeed");

    let snapshot = runtime.playback_coordination_snapshot();
    assert!(
        snapshot.transport_telemetry_observed,
        "current-epoch player evidence queued before the first owner clock must remain admissible: {snapshot:?}"
    );
}
