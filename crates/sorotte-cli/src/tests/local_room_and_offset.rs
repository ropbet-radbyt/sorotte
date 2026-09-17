//! Regression coverage for room continuity and local timeline offsets.
use super::*;
use sorotte_server::ServerRuntime;

async fn room_probe_phase<T>(phase: &str, future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(2), future)
        .await
        .unwrap_or_else(|_| panic!("room probe timed out waiting for {phase}"))
}

async fn forward_server_reply(
    server: &mut ServerRuntime,
    client: &str,
    line: &str,
    writer: &mut OwnedWriteHalf,
) {
    let replies = server.handle_line(client, line).unwrap();
    room_probe_phase("server reply delivery", async {
        for reply in replies {
            writer.write_all(reply.as_bytes()).await.unwrap();
            writer.write_all(b"\n").await.unwrap();
        }
        writer.flush().await.unwrap();
    })
    .await;
}

async fn drain_final_room_probe_connection<R: tokio::io::AsyncBufRead + Unpin>(
    lines: &mut tokio::io::Lines<R>,
) -> usize {
    tokio::time::timeout(Duration::from_secs(3), async {
        let mut drained = 0;
        while lines.next_line().await.unwrap().is_some() {
            drained += 1;
        }
        drained
    })
    .await
    .expect("room probe timed out waiting for final client closure")
}

async fn cli_room_reconnect_probe(change_room: bool, reconnect: bool) -> (Vec<String>, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let config = ClientLoopConfig {
        room: "startup-room".to_owned(),
        max_connected_runtime_seconds: 2.0,
        readiness_supported_override: Some(false),
        ..test_client_loop_config_with_addr(addr)
    };
    let runtime = create_client_runtime(&config);
    let (input_tx, input_rx) = unbounded_channel();
    let server_future = async move {
        let mut server = ServerRuntime::new();
        server.set_readiness_enabled(false);
        let mut hello_rooms = Vec::new();
        for connection in 0..=usize::from(reconnect) {
            let client = format!("connection-{connection}");
            let (socket, _) = room_probe_phase("client connection", listener.accept())
                .await
                .unwrap();
            let (reader, mut writer) = socket.into_split();
            let mut lines = BufReader::new(reader).lines();
            let mut first = room_probe_phase("client greeting", lines.next_line())
                .await
                .unwrap()
                .unwrap();
            // Normal PreferTls negotiation against a plain local fixture.
            if serde_json::from_str::<Value>(&first)
                .unwrap()
                .get("TLS")
                .is_some()
            {
                room_probe_phase(
                    "TLS refusal delivery",
                    writer.write_all(b"{\"TLS\":{\"startTLS\":\"false\"}}\n"),
                )
                .await
                .unwrap();
                first = room_probe_phase("client Hello after TLS refusal", lines.next_line())
                    .await
                    .unwrap()
                    .unwrap();
            }
            let hello: Value = serde_json::from_str(&first).unwrap();
            hello_rooms.push(hello["Hello"]["room"]["name"].as_str().unwrap().to_owned());
            forward_server_reply(&mut server, &client, &first, &mut writer).await;
            let mut user_input_sent = false;
            let mut tick = tokio::time::interval(Duration::from_millis(20));
            let progress_deadline = tokio::time::sleep(Duration::from_secs(3));
            tokio::pin!(progress_deadline);
            let mut expected_progress = if connection == 0 {
                "first post-Hello heartbeat"
            } else {
                "final post-Hello heartbeat"
            };
            loop {
                let line = tokio::select! {
                    _ = &mut progress_deadline => {
                        panic!("room probe timed out waiting for {expected_progress}");
                    }
                    line = lines.next_line() => line.unwrap(),
                    _ = tick.tick() => {
                        let dispatch = server.collect_dispatch_at(client_runtime_now_seconds()).unwrap();
                        room_probe_phase("server heartbeat delivery", async {
                            for reply in dispatch.outbound_lines {
                                if reply.client_id == client {
                                    writer.write_all(reply.line.as_bytes()).await.unwrap();
                                    writer.write_all(b"\n").await.unwrap();
                                }
                            }
                        }).await;
                        continue;
                    }
                };
                let line = line.unwrap_or_else(|| {
                    panic!("room probe connection closed before {expected_progress}")
                });
                let decoded: Value = serde_json::from_str(&line).unwrap();
                let post_hello_heartbeat =
                    decoded["State"]["ping"]["clientLatencyCalculation"].is_number();
                if connection > 0 && post_hello_heartbeat {
                    // The final Hello has been consumed. Stop producing replies
                    // before the client's intended runtime-window exit; buffered
                    // requests can remain readable after it closes its socket.
                    drain_final_room_probe_connection(&mut lines).await;
                    break;
                }
                forward_server_reply(&mut server, &client, &line, &mut writer).await;
                if connection == 0 && !user_input_sent && post_hello_heartbeat {
                    // A response to a server-generated heartbeat proves the
                    // client consumed Hello before these ordinary user commands.
                    if change_room {
                        input_tx.send("room chosen-room".to_owned()).unwrap();
                    }
                    input_tx
                        .send("chat first-phase-complete".to_owned())
                        .unwrap();
                    user_input_sent = true;
                    expected_progress = "acceptance of queued user commands";
                    progress_deadline
                        .as_mut()
                        .reset(tokio::time::Instant::now() + Duration::from_secs(3));
                }
                if connection == 0 && decoded.get("Chat").is_some() {
                    let expected = if change_room {
                        "chosen-room"
                    } else {
                        "startup-room"
                    };
                    assert_eq!(
                        server.session(&client).unwrap().room,
                        expected,
                        "the user-selected room must be accepted before the disconnect"
                    );
                    if reconnect {
                        room_probe_phase("first connection closure", writer.shutdown())
                            .await
                            .unwrap();
                        server.handle_transport_disconnect_fanout(&client).unwrap();
                        break;
                    }
                    drain_final_room_probe_connection(&mut lines).await;
                    break;
                }
            }
        }
        hello_rooms
    };
    let client_future =
        run_client_network_loop_with_prepared_runtime_for_test(&config, runtime, Some(input_rx));
    let (hellos, runtime) = tokio::time::timeout(Duration::from_secs(15), async {
        tokio::join!(server_future, client_future)
    })
    .await
    .expect("bounded ordinary reconnect exchange");
    let runtime = runtime.expect("normal runtime-window exit after final connection");
    let final_room = runtime.session().room().unwrap().to_owned();
    (hellos, final_room)
}

#[tokio::test]
async fn final_room_probe_phase_drains_requests_from_an_already_closed_peer() {
    let mut server = ServerRuntime::new();
    server
        .handle_line(
            "fixture",
            r#"{"Hello":{"username":"alice","room":{"name":"room"},"version":"1.7.5"}}"#,
        )
        .unwrap();
    let request = b"{\"List\":null}\n";

    // A duplex transport makes the old fixture's race deterministic: the
    // request remains readable after the peer has closed both directions.
    let (mut peer, fixture) = tokio::io::duplex(128);
    peer.write_all(request).await.unwrap();
    drop(peer);
    let (reader, mut writer) = tokio::io::split(fixture);
    let mut lines = BufReader::new(reader).lines();
    let pending = lines.next_line().await.unwrap().unwrap();
    let replies = server.handle_line("fixture", &pending).unwrap();
    assert!(!replies.is_empty());
    let old_reply_error = writer.write_all(replies[0].as_bytes()).await.unwrap_err();
    assert_eq!(old_reply_error.kind(), std::io::ErrorKind::BrokenPipe);

    let (mut peer, fixture) = tokio::io::duplex(128);
    peer.write_all(request).await.unwrap();
    drop(peer);
    let mut lines = BufReader::new(fixture).lines();
    assert_eq!(drain_final_room_probe_connection(&mut lines).await, 1);
}

#[tokio::test]
async fn reconnect_keeps_user_selected_room() {
    let (hellos, final_room) = cli_room_reconnect_probe(true, true).await;
    assert_eq!(
        hellos,
        ["startup-room", "chosen-room"],
        "automatic reconnect must retain the room already selected and accepted in this session"
    );
    assert_eq!(final_room, "chosen-room");
}

#[tokio::test]
async fn reconnect_preserves_unchanged_room() {
    let (hellos, final_room) = cli_room_reconnect_probe(false, true).await;
    assert_eq!(hellos, ["startup-room", "startup-room"]);
    assert_eq!(final_room, "startup-room");
}

#[tokio::test]
async fn room_switch_without_reconnect_preserves_selected_room() {
    let (hellos, final_room) = cli_room_reconnect_probe(true, false).await;
    assert_eq!(hellos, ["startup-room"]);
    assert_eq!(final_room, "chosen-room");
}

fn cli_offset_probe(command: &str) -> (f64, f64, Vec<Value>) {
    use sorotte_client_app::app_boundary::commands::{
        LocalInputCommandPlanningContext, PlannedLocalInputDispatch, plan_local_input_dispatch,
    };
    let config = ClientLoopConfig {
        local_can_control_override: Some(true),
        ..test_client_loop_config()
    };
    let mut runtime = create_client_runtime(&config);
    let mut server = ServerRuntime::new();
    let now = client_runtime_now_seconds();
    server.set_time_now_override_seconds(Some(now));
    server.set_readiness_enabled(false);
    let hello = r#"{"Hello":{"username":"cli-user","room":{"name":"cli-room"},"version":"1.7.5","features":{"chat":true}}}"#;
    for line in server.handle_line("cli", hello).unwrap() {
        runtime
            .apply_protocol_line(&line, now, true, false, false)
            .unwrap();
    }
    server
        .handle_line(
            "cli",
            r#"{"Set":{"file":{"name":"clip.mkv","duration":240}}}"#,
        )
        .unwrap();
    runtime.player_mut().open_file("clip.mkv").unwrap();
    runtime.player_mut().set_position(0.0).unwrap();
    runtime.player_mut().set_paused(true).unwrap();
    for directed in server
        .collect_dispatch_at(now + 2.0)
        .unwrap()
        .outbound_lines
    {
        if directed.client_id == "cli" {
            runtime
                .apply_protocol_line(&directed.line, now + 2.0, true, false, false)
                .unwrap();
        }
    }
    // Establish the ordinary paused baseline and deliver all setup work first.
    while let Some(pending) = runtime.pending_protocol_line().unwrap() {
        let line = pending.line().to_owned();
        runtime.acknowledge_protocol_line(pending.lease()).unwrap();
        for reply in server.handle_line("cli", &line).unwrap() {
            runtime
                .apply_protocol_line(&reply, now + 2.0, true, false, false)
                .unwrap();
        }
    }
    assert_eq!(
        runtime.session().current_room_playstate().unwrap().position,
        Some(0.0)
    );
    let dispatch = plan_local_input_dispatch(
        parse_local_input_command(command).unwrap(),
        &LocalInputCommandPlanningContext {
            current_room: Some("cli-room"),
            configured_room: "cli-room",
        },
        true,
    );
    let PlannedLocalInputDispatch::Run(action) = dispatch else {
        panic!("ordinary input must run");
    };
    let mut user_offset = 0.0;
    assert!(run_planned_local_runtime_action(&mut runtime, &mut user_offset, now, action).unwrap());
    let mut wire = Vec::new();
    while let Some(pending) = runtime.pending_protocol_line().unwrap() {
        let line = pending.line().to_owned();
        wire.push(serde_json::from_str::<Value>(&line).unwrap());
        runtime.acknowledge_protocol_line(pending.lease()).unwrap();
        for reply in server.handle_line("cli", &line).unwrap() {
            runtime
                .apply_protocol_line(&reply, now + 2.0, true, false, false)
                .unwrap();
        }
    }
    let canonical_position = server
        .collect_dispatch_at(now + 4.0)
        .unwrap()
        .outbound_lines
        .into_iter()
        .filter(|line| line.client_id == "cli")
        .filter_map(|line| match decode_message_line(&line.line).unwrap() {
            ProtocolMessage::State(state) => state.state.playstate.and_then(|state| state.position),
            _ => None,
        })
        .next_back()
        .expect("periodic server state must expose canonical playback");
    let physical_position = runtime.player().position_seconds();
    (physical_position, canonical_position, wire)
}

#[test]
fn local_offset_does_not_seek_the_room() {
    let (physical, canonical, _) = cli_offset_probe("offset 5");
    assert_eq!(physical, 5.0);
    assert_eq!(
        canonical, 0.0,
        "the documented local offset must not change everyone else's canonical room position"
    );
}

#[test]
fn explicit_seek_changes_room() {
    let (physical, canonical, _) = cli_offset_probe("seek 5");
    assert_eq!(physical, 5.0);
    assert_eq!(canonical, 5.0);
}

#[test]
fn zero_offset_keeps_room_position() {
    let (physical, canonical, _) = cli_offset_probe("offset 0");
    assert_eq!(physical, 0.0);
    assert_eq!(canonical, 0.0);
}

#[test]
fn relative_offset_uses_the_current_room_clock() {
    use sorotte_client_app::app_boundary::commands::{
        LocalOffsetCommand, PlannedLocalRuntimeAction,
    };
    let mut application = create_client_runtime(&test_client_loop_config());
    application.player_mut().open_file("clip.mkv").unwrap();
    application.player_mut().set_position(40.0).unwrap();
    application.player_mut().set_paused(false).unwrap();
    application
        .apply_protocol_line(
            r#"{"Hello":{"username":"alice","room":{"name":"room"},"version":"1.7.5"}}"#,
            100.0,
            true,
            false,
            false,
        )
        .unwrap();
    application.apply_protocol_line(
        r#"{"State":{"playstate":{"position":40.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
        100.0, true, false, false,
    ).unwrap();
    let mut offset = 0.0;
    run_planned_local_runtime_action(
        &mut application,
        &mut offset,
        105.0,
        PlannedLocalRuntimeAction::SetUserOffset(
            LocalOffsetCommand::RelativeFromCurrentPositionMinus(10.0),
        ),
    )
    .unwrap();
    assert_eq!(
        offset, 35.0,
        "the room advances from 40 to 45 before this command"
    );
    assert_eq!(application.player().position_seconds(), 80.0);
}
