use super::*;
use sorotte_player_mpv::{MpvAdapter, managed_process::ManagedMpvCommand};
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Default)]
struct SharedServer {
    server: sorotte_server::ServerRuntime,
    inboxes: HashMap<String, Vec<String>>,
}

impl SharedServer {
    fn route(&mut self, responses: Vec<sorotte_server::DirectedOutboundLine>) {
        for response in responses {
            self.inboxes
                .entry(response.client_id)
                .or_default()
                .push(response.line);
        }
    }
}

struct SharedDriver {
    username: String,
    server: Arc<Mutex<SharedServer>>,
}

impl GuiSessionTransportDriver for SharedDriver {
    fn pump(&mut self, transport: &GuiQueuedSessionTransportHandle) -> Result<(), String> {
        let mut shared = self.server.lock().unwrap();
        if let Some(delivery) = transport.take_outbound_protocol_delivery_for_driver() {
            let responses = shared
                .server
                .handle_line_fanout(&self.username, delivery.line())
                .map_err(|e| e.to_string())?;
            shared.route(responses);
            transport.publish_outbound_protocol_delivery_result(
                GuiOutboundProtocolDeliveryResult::FrameWritten {
                    token: delivery.token(),
                },
            );
        }
        let periodic = shared
            .server
            .collect_dispatch_at(f64::NAN)
            .map_err(|e| e.to_string())?;
        shared.route(periodic.outbound_lines);
        transport
            .push_inbound_protocol_lines(shared.inboxes.remove(&self.username).unwrap_or_default());
        Ok(())
    }
}

struct RealClient {
    owner: GuiPersistedConfigRuntimeOwner,
    handle: GuiQueuedRuntimeBridgeHandle,
    state: SorotteGuiShellAppState,
}

impl RealClient {
    fn pump(&mut self) {
        pump_and_apply_runtime_owner_actions(&mut self.owner, &self.handle, &mut self.state);
    }
}

#[test]
#[ignore = "requires SOROTTE_TEST_MPV_BIN and SOROTTE_TEST_REAL_PLEX; read-only direct Plex reproduction"]
fn real_mpv_two_plex_clients_resume_loaded_stream() {
    let config: serde_json::Value = serde_json::from_str(
        &std::env::var("SOROTTE_TEST_REAL_PLEX").expect("explicit private Plex test configuration"),
    )
    .unwrap();
    let executable =
        std::env::var_os("SOROTTE_TEST_MPV_BIN").expect("explicit mpv test executable");
    let instance = tempfile::tempdir().unwrap();
    let server = Arc::new(Mutex::new(SharedServer::default()));
    let mut processes = Vec::new();
    let mut clients = Vec::new();
    for username in ["alice", "bob"] {
        let endpoint = format!(
            r"\\.\pipe\sorotte-two-plex-{}-{}-{username}",
            std::process::id(),
            instance.path().file_name().unwrap().to_string_lossy()
        );
        processes.push(
            ManagedMpvCommand::new(&executable)
                .args([
                    "--no-config",
                    "--no-terminal",
                    "--idle=yes",
                    "--force-window=no",
                    "--ao=null",
                    if std::env::var_os("SOROTTE_TEST_RENDER_VIDEO").is_some() {
                        "--vo=gpu-next"
                    } else {
                        "--vo=null"
                    },
                    "--pause=yes",
                    "--keep-open=yes",
                ])
                .args([format!("--input-ipc-server={endpoint}")])
                .spawn(None)
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        let player = loop {
            match MpvAdapter::with_json_ipc(&endpoint) {
                Ok(player) => break player,
                Err(error) => {
                    assert!(Instant::now() < deadline, "{error}");
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        };
        let (owner, _) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
            .with_client_core_chat_session_runtime(username, "room1")
            .unwrap();
        let mut owner = owner.with_session_transport_driver(Box::new(SharedDriver {
            username: username.into(),
            server: server.clone(),
        }));
        owner
            .complete_mpv_attachment_after_core_configuration(
                player,
                None,
                &sorotte_player_mpv::LegacySyncplayUiSettings::default(),
            )
            .unwrap();
        owner.report_external_player_availability(
            sorotte_client_core::ExternalPlayerAvailability::Connecting,
        );
        let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            username: Some(username.into()),
            room: Some("room1".into()),
            player_path: Some("mpv".into()),
            shared_playlist_enabled: Some(true),
            plex_streaming_enabled: Some(true),
            plex_sync_enabled: Some(false),
            streaming_start_policy: Some("wait-all".into()),
            streaming_room_buffering_policy: Some("pause-eligible".into()),
            streaming_read_ahead_seconds: Some(1500.0),
            streaming_memory_cache_mebibytes: Some(600),
            plex_user_token: Some(config["token"].as_str().unwrap().into()),
            plex_selected_server_id: Some(config["server_id"].as_str().unwrap().into()),
            plex_selected_server_url: Some(config["server_url"].as_str().unwrap().into()),
            plex_selected_server_token: Some(config["token"].as_str().unwrap().into()),
            ..StoredClientSettingsMvp::default()
        });
        clients.push(RealClient {
            owner,
            handle: GuiQueuedRuntimeBridgeHandle::default(),
            state,
        });
    }
    let pump = |clients: &mut Vec<RealClient>| {
        for client in clients {
            client.pump();
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    for _ in 0..20 {
        pump(&mut clients);
    }
    let playlist_uri = sorotte_plex::PlexPlaylistUri {
        machine_identifier: config["server_id"].as_str().unwrap().into(),
        rating_key: config["rating_key"].as_str().unwrap().into(),
        title: Some("Plex reproduction".into()),
        file_name: Some("episode.mkv".into()),
        duration_millis: Some((config["duration_seconds"].as_f64().unwrap() * 1000.0) as u64),
        size_bytes: config["size_bytes"].as_u64(),
        media_type: Some(sorotte_plex::PlexMediaType::Episode),
    };
    clients[0]
        .handle
        .push_request(GuiRuntimeRequest::OpenMediaFiles {
            paths: vec![playlist_uri.to_string()],
            load_into_shared_playlist: true,
            playlist_insert_slot: None,
        });
    let deadline = Instant::now() + Duration::from_secs(35);
    while Instant::now() < deadline {
        pump(&mut clients);
        if clients.iter().all(|client| {
            !client.owner.player_local_file_placeholder
                && client.owner.player_local_file.is_some()
                && client
                    .owner
                    .session
                    .as_ref()
                    .unwrap()
                    .playback_coordination_snapshot()
                    .is_some_and(|snapshot| {
                        snapshot.diagnostic
                            == sorotte_client_core::PlaybackDiagnostic::ReadyWaitingForRoom
                    })
        }) {
            break;
        }
    }
    for (index, client) in clients.iter().enumerate() {
        assert!(
            !client.owner.player_local_file_placeholder && client.owner.player_local_file.is_some(),
            "client {index} must physically load the shared stream"
        );
        client
            .handle
            .push_request(GuiRuntimeRequest::SetLocalReady(true));
    }
    for _ in 0..20 {
        pump(&mut clients);
    }
    clients[0]
        .handle
        .push_request(GuiRuntimeRequest::SetPlaybackPaused(false));
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        pump(&mut clients);
        if clients.iter().all(|client| {
            client.owner.player_paused == Some(false)
                && client
                    .owner
                    .player_position_seconds
                    .is_some_and(|p| p > 2.0)
        }) {
            break;
        }
    }
    for (index, client) in clients.iter().enumerate() {
        assert!(
            client.owner.player_paused == Some(false)
                && client
                    .owner
                    .player_position_seconds
                    .is_some_and(|p| p > 2.0),
            "client {index} must play before measuring resume: {:?}",
            client
                .owner
                .session
                .as_ref()
                .unwrap()
                .playback_coordination_snapshot()
        );
    }
    for initiator in [0, 1, 0, 1] {
        clients[initiator]
            .handle
            .push_request(GuiRuntimeRequest::SetPlaybackPaused(true));
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            pump(&mut clients);
        }
        for client in &clients {
            assert_eq!(client.owner.player_paused, Some(true));
        }
        let positions: Vec<_> = clients
            .iter()
            .map(|client| client.owner.player_position_seconds.unwrap())
            .collect();
        let mut latencies = [None, None];
        let resumed_at = Instant::now();
        clients[initiator]
            .handle
            .push_request(GuiRuntimeRequest::SetPlaybackPaused(false));
        while resumed_at.elapsed() < Duration::from_secs(35) {
            pump(&mut clients);
            for (index, client) in clients.iter().enumerate() {
                if latencies[index].is_none()
                    && client.owner.player_paused == Some(false)
                    && client
                        .owner
                        .player_position_seconds
                        .is_some_and(|p| p > positions[index] + 0.05)
                {
                    latencies[index] = Some(resumed_at.elapsed());
                }
            }
            if latencies.iter().all(Option::is_some) {
                break;
            }
        }
        eprintln!("two real Plex clients, initiator {initiator}, resume latencies {latencies:?}");
        for (index, latency) in latencies.into_iter().enumerate() {
            assert!(
                latency.is_some_and(|elapsed| elapsed < Duration::from_secs(1)),
                "client {index} resumed late: {latency:?}; coordination {:?}",
                clients[index]
                    .owner
                    .session
                    .as_ref()
                    .unwrap()
                    .playback_coordination_snapshot()
            );
        }
    }
}
