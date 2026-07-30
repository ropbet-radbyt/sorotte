//! Real loopback coverage for reconnect-playlist acknowledgement ownership.
//!
//! Each negative assertion stops at a causally later State/Ping reply, so the
//! fixture proves the preceding frames were drained without quiet-period sleeps.

use super::*;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::sync::mpsc::UnboundedReceiver;

const INITIAL_FILES: [&str; 2] = ["episode-1.mkv", "episode-2.mkv"];
const INITIAL_INDEX: i64 = 1;
const WIRE_WATCHDOG: Duration = Duration::from_secs(3);

#[derive(Debug, Default, PartialEq, Eq)]
struct WirePlaylistObservations {
    changes: Vec<Vec<String>>,
    indexes: Vec<i64>,
}

fn playlist_files(files: &[&str]) -> Vec<String> {
    files.iter().map(|file| (*file).to_owned()).collect()
}

fn config_for(addr: std::net::SocketAddr) -> ClientLoopConfig {
    ClientLoopConfig {
        host: addr.ip().to_string(),
        port: addr.port(),
        max_connected_runtime_seconds: WIRE_WATCHDOG.as_secs_f64(),
        ..test_client_loop_config()
    }
}

fn seeded_reconnecting_runtime() -> ClientApplication<MpvAdapter> {
    let config = test_client_loop_config();
    let mut runtime = create_client_runtime(&config);
    runtime
        .session_mut()
        .apply_message_json(
            r#"{"Hello":{"username":"cli-user","room":{"name":"cli-room"},"version":"1.7.5","features":{"sharedPlaylists":true}}}"#,
        )
        .expect("seed Hello should apply");
    runtime
        .session_mut()
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode-1.mkv","episode-2.mkv"],"user":"cli-user"}}}"#,
        )
        .expect("seed playlist should apply");
    runtime
        .session_mut()
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"cli-user"}}}"#)
        .expect("seed playlist index should apply");
    runtime.session_mut().reset_sync_state_for_reconnect();
    runtime
}

async fn accept_client(
    listener: TcpListener,
    context: &str,
) -> (
    tokio::io::Lines<BufReader<tokio::net::tcp::OwnedReadHalf>>,
    OwnedWriteHalf,
) {
    let (socket, _) = tokio::time::timeout(WIRE_WATCHDOG, listener.accept())
        .await
        .unwrap_or_else(|_| panic!("{context}: accept watchdog expired"))
        .unwrap_or_else(|error| panic!("{context}: accept should succeed: {error}"));
    let (reader, writer) = socket.into_split();
    let mut lines = BufReader::new(reader).lines();
    let hello = tokio::time::timeout(WIRE_WATCHDOG, lines.next_line())
        .await
        .unwrap_or_else(|_| panic!("{context}: client Hello watchdog expired"))
        .unwrap_or_else(|error| panic!("{context}: client Hello read should succeed: {error}"))
        .unwrap_or_else(|| panic!("{context}: client Hello should be present"));
    assert!(
        matches!(
            decode_message_line(&hello).expect("client Hello should decode"),
            ProtocolMessage::Hello(_)
        ),
        "{context}: first outbound frame should be the client Hello"
    );
    (lines, writer)
}

fn hello_json(shared_playlists: bool) -> String {
    format!(
        r#"{{"Hello":{{"username":"cli-user","room":{{"name":"cli-room"}},"version":"1.7.5","features":{{"sharedPlaylists":{shared_playlists}}}}}}}"#
    )
}

fn empty_playlist_json() -> &'static str {
    r#"{"Set":{"playlistChange":{"files":[]}}}"#
}

fn hello_and_empty_batch_json(shared_playlists: bool, empty_before_hello: bool) -> String {
    let hello = format!(
        r#""Hello":{{"username":"cli-user","room":{{"name":"cli-room"}},"version":"1.7.5","features":{{"sharedPlaylists":{shared_playlists}}}}}"#
    );
    let empty = r#""Set":{"playlistChange":{"files":[]}}"#;
    let (first, second) = if empty_before_hello {
        (empty, hello.as_str())
    } else {
        (hello.as_str(), empty)
    };
    format!("{{{first},{second}}}")
}

fn playlist_json(files: &[&str], index: i64, user: &str) -> String {
    format!(
        "{}\n{}",
        serde_json::json!({
            "Set": {
                "playlistChange": {
                    "files": files,
                    "user": user,
                }
            }
        }),
        serde_json::json!({
            "Set": {
                "playlistIndex": {
                    "index": index,
                    "user": user,
                }
            }
        })
    )
}

fn ping_barrier_line(marker: f64) -> String {
    encode_message_line(&ProtocolMessage::state(
        StatePayload::new().with_ping(PingPayload::new().with_latency_calculation(marker)),
    ))
    .expect("barrier State should encode")
}

async fn write_frames_with_barrier(
    writer: &mut OwnedWriteHalf,
    frames: &str,
    marker: f64,
    context: &str,
) {
    writer
        .write_all(format!("{frames}\n{}\n", ping_barrier_line(marker)).as_bytes())
        .await
        .unwrap_or_else(|error| panic!("{context}: server frames should write: {error}"));
    writer
        .flush()
        .await
        .unwrap_or_else(|error| panic!("{context}: server frames should flush: {error}"));
}

async fn observe_until_ping_barrier(
    lines: &mut tokio::io::Lines<BufReader<tokio::net::tcp::OwnedReadHalf>>,
    marker: f64,
    context: &str,
) -> WirePlaylistObservations {
    let future = async {
        let mut observations = WirePlaylistObservations::default();
        loop {
            let line = lines
                .next_line()
                .await
                .unwrap_or_else(|error| panic!("{context}: outbound read should succeed: {error}"))
                .unwrap_or_else(|| panic!("{context}: client closed before the Ping barrier"));
            match decode_message_line(&line)
                .unwrap_or_else(|error| panic!("{context}: outbound frame should decode: {error}"))
            {
                ProtocolMessage::Set(payload) => {
                    if let Some(change) = payload.set.playlist_change {
                        observations.changes.push(change.files);
                    }
                    if let Some(index) = payload.set.playlist_index {
                        observations.indexes.push(index.index);
                    }
                }
                ProtocolMessage::State(payload)
                    if payload
                        .state
                        .ping
                        .as_ref()
                        .and_then(|ping| ping.latency_calculation)
                        == Some(marker) =>
                {
                    return observations;
                }
                _ => {}
            }
        }
    };
    tokio::time::timeout(WIRE_WATCHDOG, future)
        .await
        .unwrap_or_else(|_| panic!("{context}: Ping barrier watchdog expired"))
}

fn assert_single_playlist_restore(
    observations: &WirePlaylistObservations,
    expected_files: &[&str],
    expected_index: i64,
    context: &str,
) {
    assert_eq!(
        observations.changes,
        vec![playlist_files(expected_files)],
        "{context}: restore playlist writes"
    );
    assert_eq!(
        observations.indexes,
        vec![expected_index],
        "{context}: restore index writes"
    );
}

async fn finish_server_connection(
    writer: &mut OwnedWriteHalf,
    lines: &mut tokio::io::Lines<BufReader<tokio::net::tcp::OwnedReadHalf>>,
    context: &str,
) {
    writer
        .shutdown()
        .await
        .unwrap_or_else(|error| panic!("{context}: server write shutdown should succeed: {error}"));
    let _ = tokio::time::timeout(WIRE_WATCHDOG, async {
        while let Ok(Some(_)) = lines.next_line().await {}
    })
    .await;
}

async fn run_loopback_session(
    runtime: &mut ClientApplication<MpvAdapter>,
    addr: std::net::SocketAddr,
    local_input_rx: Option<&mut UnboundedReceiver<String>>,
    context: &str,
) {
    let config = config_for(addr);
    let stream = tokio::time::timeout(WIRE_WATCHDOG, TcpStream::connect(addr))
        .await
        .unwrap_or_else(|_| panic!("{context}: connect watchdog expired"))
        .unwrap_or_else(|error| panic!("{context}: client should connect: {error}"));
    let mut notification_sink = ignore_autoplay_notification;
    let mut file_difference_sink = ignore_file_difference_notification;
    let exit = tokio::time::timeout(
        WIRE_WATCHDOG,
        run_connected_client_session(
            stream,
            runtime,
            &config,
            None,
            local_input_rx,
            &mut notification_sink,
            &mut file_difference_sink,
        ),
    )
    .await
    .unwrap_or_else(|_| panic!("{context}: connected-session watchdog expired"))
    .unwrap_or_else(|error| panic!("{context}: connected session should run: {error:#}"));
    assert_eq!(
        exit,
        ConnectedSessionExit::TransportClosed,
        "{context}: server shutdown should be observed as transport close"
    );
}

#[tokio::test]
async fn loopback_reconnect_rearms_unacknowledged_restore_and_matching_echo_retires_ownership() {
    let mut runtime = seeded_reconnecting_runtime();

    let first_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("first listener should bind");
    let first_addr = first_listener
        .local_addr()
        .expect("first listener should expose its address");
    let first_server = tokio::spawn(async move {
        let (mut lines, mut writer) =
            accept_client(first_listener, "first unacknowledged generation").await;
        write_frames_with_barrier(
            &mut writer,
            &format!("{}\n{}", hello_json(true), empty_playlist_json()),
            1_001.0,
            "first unacknowledged generation",
        )
        .await;
        let observed =
            observe_until_ping_barrier(&mut lines, 1_001.0, "first unacknowledged generation")
                .await;
        assert_single_playlist_restore(
            &observed,
            &INITIAL_FILES,
            INITIAL_INDEX,
            "first unacknowledged generation",
        );
        finish_server_connection(&mut writer, &mut lines, "first unacknowledged generation").await;
    });
    run_loopback_session(
        &mut runtime,
        first_addr,
        None,
        "first unacknowledged generation",
    )
    .await;
    first_server
        .await
        .expect("first unacknowledged server should not panic");

    runtime.session_mut().reset_sync_state_for_reconnect();
    let second_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("second listener should bind");
    let second_addr = second_listener
        .local_addr()
        .expect("second listener should expose its address");
    let (local_input_tx, mut local_input_rx) = unbounded_channel::<String>();
    let second_server = tokio::spawn(async move {
        let context = "re-armed generation";
        let (mut lines, mut writer) = accept_client(second_listener, context).await;
        write_frames_with_barrier(
            &mut writer,
            &format!("{}\n{}", hello_json(true), empty_playlist_json()),
            1_002.0,
            context,
        )
        .await;
        let rearmed = observe_until_ping_barrier(&mut lines, 1_002.0, context).await;
        assert_single_playlist_restore(&rearmed, &INITIAL_FILES, INITIAL_INDEX, context);

        write_frames_with_barrier(
            &mut writer,
            &playlist_json(&INITIAL_FILES, INITIAL_INDEX, "cli-user"),
            1_003.0,
            "matching echo",
        )
        .await;
        let after_echo = observe_until_ping_barrier(&mut lines, 1_003.0, "matching echo").await;
        assert_eq!(
            after_echo,
            WirePlaylistObservations::default(),
            "a matching server echo should not manufacture another playlist mutation"
        );

        local_input_tx
            .send("queue episode-3.mkv".to_owned())
            .expect("new local playlist command should queue after the echo barrier");
        let local_change = tokio::time::timeout(WIRE_WATCHDOG, async {
            loop {
                let line = lines
                    .next_line()
                    .await
                    .expect("new local playlist read should succeed")
                    .expect("client should remain connected for the new local playlist");
                if let ProtocolMessage::Set(payload) =
                    decode_message_line(&line).expect("new local playlist frame should decode")
                    && let Some(change) = payload.set.playlist_change
                {
                    break change.files;
                }
            }
        })
        .await
        .expect("new local playlist watchdog should not expire");
        assert_eq!(
            local_change,
            playlist_files(&["episode-1.mkv", "episode-2.mkv", "episode-3.mkv"]),
            "new local ownership should supersede the acknowledged restore"
        );
        finish_server_connection(&mut writer, &mut lines, context).await;
    });
    run_loopback_session(
        &mut runtime,
        second_addr,
        Some(&mut local_input_rx),
        "re-armed generation",
    )
    .await;
    second_server
        .await
        .expect("re-armed server should not panic");

    runtime.session_mut().reset_sync_state_for_reconnect();
    let third_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("third listener should bind");
    let third_addr = third_listener
        .local_addr()
        .expect("third listener should expose its address");
    let third_server = tokio::spawn(async move {
        let context = "post-acknowledgement generation";
        let (mut lines, mut writer) = accept_client(third_listener, context).await;
        write_frames_with_barrier(
            &mut writer,
            &format!("{}\n{}", hello_json(true), empty_playlist_json()),
            1_004.0,
            context,
        )
        .await;
        let observed = observe_until_ping_barrier(&mut lines, 1_004.0, context).await;
        assert_single_playlist_restore(
            &observed,
            &["episode-1.mkv", "episode-2.mkv", "episode-3.mkv"],
            INITIAL_INDEX,
            context,
        );
        finish_server_connection(&mut writer, &mut lines, context).await;
    });
    run_loopback_session(
        &mut runtime,
        third_addr,
        None,
        "post-acknowledgement generation",
    )
    .await;
    third_server
        .await
        .expect("post-acknowledgement server should not panic");
}

#[tokio::test]
async fn loopback_divergent_authority_supersedes_emitted_restore_across_reconnect() {
    let mut runtime = seeded_reconnecting_runtime();
    let divergent_files = ["remote-1.mkv", "remote-2.mkv", "remote-3.mkv"];
    let divergent_index = 2;

    let first_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("authority listener should bind");
    let first_addr = first_listener
        .local_addr()
        .expect("authority listener should expose its address");
    let first_server = tokio::spawn(async move {
        let context = "authority supersession generation";
        let (mut lines, mut writer) = accept_client(first_listener, context).await;
        write_frames_with_barrier(
            &mut writer,
            &format!("{}\n{}", hello_json(true), empty_playlist_json()),
            2_001.0,
            context,
        )
        .await;
        let restore = observe_until_ping_barrier(&mut lines, 2_001.0, context).await;
        assert_single_playlist_restore(&restore, &INITIAL_FILES, INITIAL_INDEX, context);

        write_frames_with_barrier(
            &mut writer,
            &playlist_json(&divergent_files, divergent_index, "remote-user"),
            2_002.0,
            context,
        )
        .await;
        let after_authority = observe_until_ping_barrier(&mut lines, 2_002.0, context).await;
        assert_eq!(
            after_authority,
            WirePlaylistObservations::default(),
            "remote authority should apply without a local playlist echo"
        );
        finish_server_connection(&mut writer, &mut lines, context).await;
    });
    run_loopback_session(
        &mut runtime,
        first_addr,
        None,
        "authority supersession generation",
    )
    .await;
    first_server
        .await
        .expect("authority supersession server should not panic");

    let current = runtime
        .session()
        .current_room_playlist()
        .expect("remote authority should remain current before reconnect");
    assert_eq!(current.files, playlist_files(&divergent_files));
    assert_eq!(current.index, Some(divergent_index));

    runtime.session_mut().reset_sync_state_for_reconnect();
    let second_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("post-authority listener should bind");
    let second_addr = second_listener
        .local_addr()
        .expect("post-authority listener should expose its address");
    let second_server = tokio::spawn(async move {
        let context = "post-authority generation";
        let (mut lines, mut writer) = accept_client(second_listener, context).await;
        write_frames_with_barrier(
            &mut writer,
            &format!("{}\n{}", hello_json(true), empty_playlist_json()),
            2_003.0,
            context,
        )
        .await;
        let observed = observe_until_ping_barrier(&mut lines, 2_003.0, context).await;
        assert_single_playlist_restore(&observed, &divergent_files, divergent_index, context);
        finish_server_connection(&mut writer, &mut lines, context).await;
    });
    run_loopback_session(&mut runtime, second_addr, None, "post-authority generation").await;
    second_server
        .await
        .expect("post-authority server should not panic");
}

#[tokio::test]
async fn loopback_capability_disable_clears_restore_and_does_not_resurrect_it() {
    let mut runtime = seeded_reconnecting_runtime();

    let first_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("pre-disable listener should bind");
    let first_addr = first_listener
        .local_addr()
        .expect("pre-disable listener should expose its address");
    let first_server = tokio::spawn(async move {
        let context = "pre-disable pending acknowledgement";
        let (mut lines, mut writer) = accept_client(first_listener, context).await;
        write_frames_with_barrier(
            &mut writer,
            &format!("{}\n{}", hello_json(true), empty_playlist_json()),
            3_001.0,
            context,
        )
        .await;
        let observed = observe_until_ping_barrier(&mut lines, 3_001.0, context).await;
        assert_single_playlist_restore(&observed, &INITIAL_FILES, INITIAL_INDEX, context);
        finish_server_connection(&mut writer, &mut lines, context).await;
    });
    run_loopback_session(
        &mut runtime,
        first_addr,
        None,
        "pre-disable pending acknowledgement",
    )
    .await;
    first_server
        .await
        .expect("pre-disable server should not panic");

    runtime.session_mut().reset_sync_state_for_reconnect();
    let disabled_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("disabled listener should bind");
    let disabled_addr = disabled_listener
        .local_addr()
        .expect("disabled listener should expose its address");
    let disabled_server = tokio::spawn(async move {
        let context = "capability-disabled generation";
        let (mut lines, mut writer) = accept_client(disabled_listener, context).await;
        let empty_then_hello = hello_and_empty_batch_json(false, true);
        write_frames_with_barrier(&mut writer, &empty_then_hello, 3_002.0, context).await;
        let observed = observe_until_ping_barrier(&mut lines, 3_002.0, context).await;
        assert_eq!(
            observed,
            WirePlaylistObservations::default(),
            "a capability-disabled generation must not write a playlist restore"
        );
        finish_server_connection(&mut writer, &mut lines, context).await;
    });
    run_loopback_session(
        &mut runtime,
        disabled_addr,
        None,
        "capability-disabled generation",
    )
    .await;
    disabled_server
        .await
        .expect("capability-disabled server should not panic");
    assert!(
        !runtime.session().server_shared_playlists_supported(),
        "disabled server capability should apply through the connected-session boundary"
    );

    runtime.session_mut().reset_sync_state_for_reconnect();
    let reenabled_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("re-enabled listener should bind");
    let reenabled_addr = reenabled_listener
        .local_addr()
        .expect("re-enabled listener should expose its address");
    let reenabled_server = tokio::spawn(async move {
        let context = "re-enabled generation";
        let (mut lines, mut writer) = accept_client(reenabled_listener, context).await;
        let hello_then_empty = hello_and_empty_batch_json(true, false);
        write_frames_with_barrier(&mut writer, &hello_then_empty, 3_003.0, context).await;
        let observed = observe_until_ping_barrier(&mut lines, 3_003.0, context).await;
        assert_eq!(
            observed,
            WirePlaylistObservations::default(),
            "re-enabling shared playlists must not resurrect capability-cleared ownership"
        );
        finish_server_connection(&mut writer, &mut lines, context).await;
    });
    run_loopback_session(&mut runtime, reenabled_addr, None, "re-enabled generation").await;
    reenabled_server
        .await
        .expect("re-enabled server should not panic");
}

#[tokio::test]
async fn loopback_batched_hello_and_empty_snapshot_restore_in_either_order() {
    for empty_before_hello in [false, true] {
        let mut runtime = seeded_reconnecting_runtime();
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("ordering listener should bind");
        let addr = listener
            .local_addr()
            .expect("ordering listener should expose its address");
        let server = tokio::spawn(async move {
            let context = if empty_before_hello {
                "empty-before-Hello batch"
            } else {
                "Hello-before-empty batch"
            };
            let (mut lines, mut writer) = accept_client(listener, context).await;
            let frames = hello_and_empty_batch_json(true, empty_before_hello);
            let marker = if empty_before_hello { 4_002.0 } else { 4_001.0 };
            write_frames_with_barrier(&mut writer, &frames, marker, context).await;
            let observed = observe_until_ping_barrier(&mut lines, marker, context).await;
            assert_single_playlist_restore(&observed, &INITIAL_FILES, INITIAL_INDEX, context);
            finish_server_connection(&mut writer, &mut lines, context).await;
        });
        let context = if empty_before_hello {
            "empty-before-Hello batch"
        } else {
            "Hello-before-empty batch"
        };
        run_loopback_session(&mut runtime, addr, None, context).await;
        server.await.expect("ordering server should not panic");
    }
}
