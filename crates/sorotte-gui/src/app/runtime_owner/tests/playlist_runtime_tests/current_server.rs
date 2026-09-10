use super::*;
use crate::app::runtime_stack::{
    GuiOutboundProtocolDeliveryResult, GuiQueuedSessionTransportHandle, GuiSessionTransportDriver,
};
use sorotte_player_api::{LocalFileUpdate, PlayerError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[cfg(windows)]
mod two_players;

#[derive(Default)]
struct CurrentServerDriver {
    server: sorotte_server::ServerRuntime,
    hold_responses: Arc<AtomicBool>,
    pending_responses: Vec<String>,
    statuses: Arc<Mutex<Vec<serde_json::Value>>>,
    remote_commands: Arc<Mutex<Vec<String>>>,
    observer: Option<Arc<Mutex<sorotte_client_core::ClientSession>>>,
    observer_ready_scope: Option<(u64, u64, Option<u64>)>,
    observer_report_sequence: u64,
    observer_started_revision: Option<(u64, u64)>,
}

impl CurrentServerDriver {
    #[cfg(windows)]
    fn with_observer() -> Self {
        let mut driver = Self {
            observer: Some(Arc::new(Mutex::new(
                sorotte_client_core::ClientSession::default(),
            ))),
            ..Self::default()
        };
        // The reporting client and observer both use the current readiness
        // protocol, as in the reported all-current-version room.
        let modern = true;
        let hello = serde_json::json!({"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.7.5","features":{"uiMode":"GUI","featureList":true,"sharedPlaylists":true,"sorotteParticipantStatusV1":true,"readiness":modern,"sorotteReadinessV2":modern,"sorottePlaybackBarrierV1":modern,"sorottePlexPlaylistUris":true}}}).to_string();
        let responses = driver.server.handle_line_fanout("bob", &hello).unwrap();
        driver.route(responses);
        if std::env::var_os("SOROTTE_TEST_CONTROLLED_ROOM").is_some() {
            let responses = driver
                .server
                .handle_line_fanout(
                    "bob",
                    r#"{"Set":{"controllerAuth":{"room":"room1","password":"AB-123-456"}}}"#,
                )
                .unwrap();
            let room = responses
                .iter()
                .find_map(|response| {
                    match sorotte_protocol::decode_message_line(&response.line).unwrap() {
                        sorotte_protocol::ProtocolMessage::Set(set) => {
                            set.set.new_controlled_room.and_then(|room| room.room_name)
                        }
                        _ => None,
                    }
                })
                .unwrap();
            let responses = driver
                .server
                .handle_line_fanout(
                    "bob",
                    &serde_json::json!({"Set":{"room":{"name":room}}}).to_string(),
                )
                .unwrap();
            driver.route(responses);
            let responses = driver.server.handle_line_fanout("bob", &serde_json::json!({"Set":{"controllerAuth":{"room":room,"password":"AB-123-456"}}}).to_string()).unwrap();
            driver.route(responses);
        }
        driver
    }

    fn route(&mut self, responses: Vec<sorotte_server::DirectedOutboundLine>) {
        let mut acknowledgements = Vec::new();
        for response in responses {
            let message: serde_json::Value = serde_json::from_str(&response.line).unwrap();
            if response.client_id == "alice" {
                if let Some(status) =
                    message.pointer("/State/sorotteParticipantStatusV1/snapshot/participants/alice")
                {
                    self.statuses.lock().unwrap().push(status.clone());
                }
                self.pending_responses.push(response.line);
            } else if response.client_id == "bob" {
                if let Some(observer) = &self.observer {
                    observer
                        .lock()
                        .unwrap()
                        .apply_message_json(&response.line)
                        .unwrap();
                }
                if let Some(state) = message.get("State") {
                    let mut reply = serde_json::Map::new();
                    if let Some(counter) = state.pointer("/ignoringOnTheFly/server") {
                        reply.insert(
                            "ignoringOnTheFly".into(),
                            serde_json::json!({"server": counter}),
                        );
                    }
                    if let Some(ping) = state.pointer("/ping/latencyCalculation") {
                        reply.insert(
                            "ping".into(),
                            serde_json::json!({"latencyCalculation": ping}),
                        );
                    }
                    if !reply.is_empty() {
                        acknowledgements.push(serde_json::json!({"State":reply}).to_string());
                    }
                }
            }
        }
        for reply in acknowledgements {
            let responses = self.server.handle_line_fanout("bob", &reply).unwrap();
            assert!(
                responses.is_empty(),
                "observer acknowledgement must not generate control"
            );
        }
    }
}

impl GuiSessionTransportDriver for CurrentServerDriver {
    fn pump(&mut self, transport: &GuiQueuedSessionTransportHandle) -> Result<(), String> {
        if let Some(delivery) = transport.take_outbound_protocol_delivery_for_driver() {
            let responses = self
                .server
                .handle_line_fanout("alice", delivery.line())
                .map_err(|e| e.to_string())?;
            self.route(responses);
            transport.publish_outbound_protocol_delivery_result(
                GuiOutboundProtocolDeliveryResult::FrameWritten {
                    token: delivery.token(),
                },
            );
        }
        let periodic = self
            .server
            .collect_dispatch_at(f64::NAN)
            .map_err(|e| e.to_string())?;
        self.route(periodic.outbound_lines);
        if let Some(observer) = &self.observer {
            use sorotte_protocol::*;
            let scope = {
                let observer = observer.lock().unwrap();
                observer.readiness_snapshot().and_then(|snapshot| {
                    Some((
                        snapshot.participants.get("bob")?.membership_epoch,
                        snapshot.media_generation.unwrap_or(0),
                        observer
                            .playback_barrier_commit()
                            .map(|commit| commit.state_revision),
                    ))
                })
            };
            if let Some((epoch, generation, revision)) =
                scope.filter(|scope| self.observer_ready_scope != Some(*scope))
            {
                self.observer_ready_scope = scope;
                self.observer_report_sequence += 1;
                let intent = ReadinessIntentRequest::new(
                    "observer-ready",
                    self.observer_report_sequence,
                    epoch,
                    UserReadinessIntent::Ready,
                    UserReadinessMutationSource::DirectUser {
                        surface: DirectReadinessSurface::GuiButton,
                    },
                );
                let mut technical = TechnicalReadinessReport::new(
                    generation,
                    epoch,
                    self.observer_report_sequence,
                    TechnicalPlayabilityPhase::Playable,
                );
                technical.authoritative_playback_revision = revision;
                let messages = [
                    ProtocolMessage::set(
                        SetPayload::new()
                            .with_readiness_v2(ReadinessSetExtension::new().with_intent(intent)),
                    ),
                    ProtocolMessage::state(StatePayload::new().with_readiness_v2(
                        ReadinessStateExtension::new().with_technical(technical),
                    )),
                    ProtocolMessage::state(StatePayload::new().with_playback_barrier_v1(
                        PlaybackBarrierStateExtension::new().with_ready(
                            MediaReadyPayload::new(generation, true, true).with_seekable(true),
                        ),
                    )),
                ];
                for message in messages
                    .into_iter()
                    .take(if generation == 0 { 1 } else { 3 })
                {
                    let responses = self
                        .server
                        .handle_line_fanout("bob", &encode_message_line(&message).unwrap())
                        .unwrap();
                    self.route(responses);
                }
            }
        }
        let observer_commit = self
            .observer
            .as_ref()
            .and_then(|observer| observer.lock().unwrap().playback_barrier_commit().cloned());
        if let Some(commit) = observer_commit
            && self.observer_started_revision
                != Some((commit.media_generation, commit.state_revision))
        {
            use sorotte_protocol::*;
            self.observer_started_revision = Some((commit.media_generation, commit.state_revision));
            let started = StartedAckPayload::new(
                commit.media_generation,
                commit.state_revision,
                commit.anchor_position,
            );
            let line = encode_message_line(&ProtocolMessage::state(
                StatePayload::new().with_playback_barrier_v1(
                    PlaybackBarrierStateExtension::new().with_started(started),
                ),
            ))
            .unwrap();
            let responses = self.server.handle_line_fanout("bob", &line).unwrap();
            self.route(responses);
        }
        let commands = std::mem::take(&mut *self.remote_commands.lock().unwrap());
        for command in commands {
            if let Some(paused) = serde_json::from_str::<serde_json::Value>(&command)
                .unwrap()
                .pointer("/State/playstate/paused")
                .and_then(serde_json::Value::as_bool)
                && let Some((epoch, _, _)) = self.observer_ready_scope
            {
                use sorotte_protocol::*;
                self.observer_report_sequence += 1;
                let intent = ReadinessIntentRequest::new(
                    format!("observer-control-{}", self.observer_report_sequence),
                    self.observer_report_sequence,
                    epoch,
                    if paused {
                        UserReadinessIntent::NotReady
                    } else {
                        UserReadinessIntent::Ready
                    },
                    UserReadinessMutationSource::IndirectPlayer {
                        action: if paused {
                            PlayerReadinessAction::Pause
                        } else {
                            PlayerReadinessAction::Play
                        },
                        surface: PlayerInteractionSurface::NativePlayerControl,
                    },
                );
                let line = encode_message_line(&ProtocolMessage::set(
                    SetPayload::new()
                        .with_readiness_v2(ReadinessSetExtension::new().with_intent(intent)),
                ))
                .unwrap();
                let responses = self.server.handle_line_fanout("bob", &line).unwrap();
                self.route(responses);
            }
            let responses = self
                .server
                .handle_line_fanout("bob", &command)
                .map_err(|e| e.to_string())?;
            self.route(responses);
        }
        if !self.hold_responses.load(Ordering::SeqCst) {
            transport.push_inbound_protocol_lines(std::mem::take(&mut self.pending_responses));
        }
        Ok(())
    }
}

struct ObservedFilePlayer {
    current: Arc<Mutex<Option<std::path::PathBuf>>>,
    update: Option<LocalFileUpdate>,
}

impl PlayerAdapter for ObservedFilePlayer {
    fn name(&self) -> &'static str {
        "observed-file"
    }
    fn open_file(&mut self, path: &str) -> Result<(), PlayerError> {
        // The player may receive the expanded spelling of a Windows 8.3 path.
        // Observe the actual fixture identity, not the caller's path spelling.
        *self.current.lock().unwrap() = Some(std::fs::canonicalize(path).unwrap());
        self.update = Some(
            LocalFileUpdate::new(
                std::path::Path::new(path)
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap(),
            )
            .with_path(path.to_owned())
            .with_duration_seconds(300.0),
        );
        Ok(())
    }
    fn set_position(&mut self, _position: f64) -> Result<(), PlayerError> {
        Ok(())
    }
    fn set_paused(&mut self, _paused: bool) -> Result<(), PlayerError> {
        Ok(())
    }
    fn set_playback_rate(&mut self, _rate: f64) -> Result<(), PlayerError> {
        Ok(())
    }
    fn take_local_file_update(&mut self) -> Option<LocalFileUpdate> {
        self.update.take()
    }
}

#[test]
fn current_server_local_file_append_select_then_edit_keeps_player_and_playlist_in_sync() {
    let media = tempfile::tempdir().unwrap();
    let first = media
        .path()
        .join("episode1.mkv")
        .to_string_lossy()
        .into_owned();
    let second = media
        .path()
        .join("episode2.mkv")
        .to_string_lossy()
        .into_owned();
    std::fs::write(&first, b"first").unwrap();
    std::fs::write(&second, b"second").unwrap();
    let first_identity = std::fs::canonicalize(&first).unwrap();
    let second_identity = std::fs::canonicalize(&second).unwrap();
    let (owner, _) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", "room1")
        .unwrap();
    let mut owner = owner.with_session_transport_driver(Box::new(CurrentServerDriver::default()));
    let current = Arc::new(Mutex::new(None));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(ObservedFilePlayer {
        current: current.clone(),
        update: None,
    })));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".into()),
        room: Some("room1".into()),
        player_path: Some("mpv".into()),
        shared_playlist_enabled: Some(true),
        only_switch_to_trusted_domains: Some(false),
        trusted_domains: Some(Vec::new()),
        ..StoredClientSettingsMvp::default()
    });
    for _ in 0..12 {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    }
    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![first.clone()],
        load_into_shared_playlist: true,
        playlist_insert_slot: None,
    });
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(3),
        |_| current.lock().unwrap().as_deref() == Some(first_identity.as_path()),
        "first local file should open through current server",
    );
    for _ in 0..12 {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    }
    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: vec![second.clone()],
        load_into_shared_playlist: true,
        playlist_insert_slot: Some(1),
    });
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(3),
        |s| s.main_window.playlist.len() == 2,
        "append second file",
    );
    for _ in 0..12 {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    }
    assert_eq!(
        current.lock().unwrap().as_deref(),
        Some(first_identity.as_path()),
        "append preserves playback"
    );
    handle.push_request(GuiRuntimeRequest::SetPlaylistIndex(1));
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(3),
        |_| current.lock().unwrap().as_deref() == Some(second_identity.as_path()),
        "select second file must reach local player",
    );
    assert_eq!(
        current.lock().unwrap().as_deref(),
        Some(second_identity.as_path())
    );
    handle.push_request(GuiRuntimeRequest::DeletePlaylistIndex(0));
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(3),
        |s| s.main_window.playlist.len() == 1 && s.main_window.playlist[0].label == "episode2.mkv",
        "edit after selection remains synchronized",
    );
}

#[cfg(windows)]
#[test]
#[ignore = "requires SOROTTE_TEST_MPV_BIN; headless real-player reproduction"]
fn real_mpv_current_server_local_file_append_select_then_edit() {
    real_mpv_current_server_repro(true, false, false);
}

#[cfg(windows)]
#[test]
#[ignore = "requires SOROTTE_TEST_MPV_BIN; headless real-player reproduction"]
fn real_mpv_current_server_playing_participant_status() {
    real_mpv_current_server_repro(false, false, false);
}

#[cfg(windows)]
#[test]
#[ignore = "requires SOROTTE_TEST_MPV_BIN; headless real-player reproduction"]
fn real_mpv_current_server_direct_http_resume_latency() {
    real_mpv_current_server_repro(false, true, false);
}

#[cfg(windows)]
#[test]
#[ignore = "requires SOROTTE_TEST_MPV_BIN; headless real-player reproduction"]
fn real_mpv_current_server_direct_plex_resume_latency() {
    real_mpv_current_server_repro(false, true, true);
}

#[cfg(windows)]
fn real_mpv_current_server_repro(pipelined_playlist: bool, network: bool, plex: bool) {
    use sorotte_player_mpv::{MpvAdapter, managed_process::ManagedMpvCommand};
    use std::time::{Duration, Instant};
    let media = tempfile::tempdir().unwrap();
    let real_plex = plex
        .then(|| std::env::var("SOROTTE_TEST_REAL_PLEX").ok())
        .flatten()
        .map(|value| serde_json::from_str::<serde_json::Value>(&value).unwrap());
    let duration_seconds = real_plex
        .as_ref()
        .and_then(|value| value["duration_seconds"].as_f64())
        .unwrap_or(120.0);
    let media_size = real_plex
        .as_ref()
        .and_then(|value| value["size_bytes"].as_u64())
        .unwrap_or(1_920_044);
    let first = media
        .path()
        .join("episode1.wav")
        .to_string_lossy()
        .into_owned();
    let second = media
        .path()
        .join("episode2.wav")
        .to_string_lossy()
        .into_owned();
    let data_len = 8000_u32 * 120 * 2;
    let mut wav = b"RIFF".to_vec();
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&8000_u32.to_le_bytes());
    wav.extend_from_slice(&16000_u32.to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.resize(44 + data_len as usize, 0);
    std::fs::write(&first, &wav).unwrap();
    std::fs::write(&second, &wav).unwrap();
    let http = network.then(|| DirectHttpFixture::new(wav));
    let first = http.as_ref().map_or(first, |http| {
        format!("http://{}/episode1.wav", http.address)
    });
    let stream_target = plex.then(|| {
        let playlist_uri = sorotte_plex::PlexPlaylistUri {
            machine_identifier: real_plex
                .as_ref()
                .and_then(|value| value["server_id"].as_str())
                .unwrap_or("fixture-machine")
                .into(),
            rating_key: real_plex
                .as_ref()
                .and_then(|value| value["rating_key"].as_str())
                .unwrap_or("1")
                .into(),
            title: Some("Episode 1".into()),
            file_name: Some("episode1.wav".into()),
            duration_millis: Some((duration_seconds * 1000.0) as u64),
            size_bytes: Some(media_size),
            media_type: Some(sorotte_plex::PlexMediaType::Episode),
        };
        sorotte_plex::PlexStreamTarget {
            logical_file: LocalFileUpdate::new("episode1.wav")
                .with_path(playlist_uri.to_string())
                .with_duration_seconds(duration_seconds)
                .with_size_bytes(media_size),
            playlist_uri,
            matched_item: sorotte_plex::PlexMatchedItem {
                rating_key: "1".into(),
                title: "Episode 1".into(),
                media_type: sorotte_plex::PlexMediaType::Episode,
                duration_millis: Some((duration_seconds * 1000.0) as u64),
            },
            playback_url: sorotte_plex::SecretPlexPlaybackUrl::new(
                real_plex
                    .as_ref()
                    .and_then(|value| value["url"].as_str())
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("{first}?X-Plex-Token=public-test-token")),
            ),
        }
    });
    let first = stream_target
        .as_ref()
        .map_or(first, |target| target.playlist_uri.to_string());
    let endpoint = format!(
        r"\\.\pipe\sorotte-playlist-repro-{}-{}",
        std::process::id(),
        media.path().file_name().unwrap().to_string_lossy()
    );
    let video_output = if std::env::var_os("SOROTTE_TEST_RENDER_VIDEO").is_some() {
        "--vo=gpu-next"
    } else {
        "--vo=null"
    };
    let _process =
        ManagedMpvCommand::new(std::env::var_os("SOROTTE_TEST_MPV_BIN").expect("real mpv path"))
            .args([
                "--no-config",
                "--no-terminal",
                "--idle=yes",
                "--force-window=no",
                "--ao=null",
                video_output,
                "--pause=yes",
                "--keep-open=yes",
            ])
            .args([format!("--input-ipc-server={endpoint}")])
            .spawn(None)
            .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut player = loop {
        match MpvAdapter::with_json_ipc(&endpoint) {
            Ok(player) => break player,
            Err(error) => {
                assert!(Instant::now() < deadline, "{error}");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    player.configure_network_media_options([
        ("cache", "auto"),
        ("cache-pause", "yes"),
        ("cache-pause-initial", "yes"),
        ("cache-pause-wait", "5"),
        ("cache-secs", "1500"),
        ("demuxer-max-bytes", "600MiB"),
    ]);
    let hold_responses = Arc::new(AtomicBool::new(false));
    let statuses = Arc::new(Mutex::new(Vec::new()));
    let remote_commands = Arc::new(Mutex::new(Vec::new()));
    let server = CurrentServerDriver::with_observer();
    let observer = server.observer.clone().unwrap();
    let room = observer.lock().unwrap().room().unwrap().to_owned();
    let room_input = if room.starts_with('+') {
        format!("{room}:AB-123-456")
    } else {
        room
    };
    let (owner, _) = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_session_runtime("alice", &room_input)
        .unwrap();
    let mut owner = owner.with_session_transport_driver(Box::new(CurrentServerDriver {
        hold_responses: hold_responses.clone(),
        statuses: statuses.clone(),
        remote_commands: remote_commands.clone(),
        ..server
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
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".into()),
        room: Some(room_input.clone()),
        player_path: Some("mpv".into()),
        shared_playlist_enabled: Some(true),
        only_switch_to_trusted_domains: Some(false),
        trusted_domains: Some(Vec::new()),
        ..StoredClientSettingsMvp::default()
    });
    if let Some(config) = &real_plex {
        state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            username: Some("alice".into()),
            room: Some(room_input.clone()),
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
    }
    for _ in 0..12 {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    }
    remote_commands
        .lock()
        .unwrap()
        .push(r#"{"List":null}"#.into());
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    if real_plex.is_some() {
        handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
            paths: vec![first.clone()],
            load_into_shared_playlist: true,
            playlist_insert_slot: None,
        });
    } else if let Some(target) = stream_target {
        owner
            .session
            .as_mut()
            .unwrap()
            .replace_playlist(vec![first.clone()], Some(0))
            .unwrap();
        owner
            .open_plex_stream_target_through_attached_player_result_impl(&first, target, true)
            .unwrap()
            .unwrap();
    } else {
        handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
            paths: vec![first.clone()],
            load_into_shared_playlist: true,
            playlist_insert_slot: None,
        });
    }
    for stage in 0..2 {
        let expected = if network && !plex {
            first.as_str()
        } else if stage == 0 {
            "episode1.wav"
        } else {
            "episode2.wav"
        };
        let deadline =
            Instant::now() + Duration::from_secs(if real_plex.is_some() { 20 } else { 6 });
        while Instant::now() < deadline {
            pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
            if real_plex.is_some()
                && !owner.player_local_file_placeholder
                && owner
                    .session
                    .as_ref()
                    .unwrap()
                    .playback_coordination_snapshot()
                    .is_some_and(|snapshot| {
                        snapshot.transport_telemetry_observed
                            && snapshot.diagnostic
                                == sorotte_client_core::PlaybackDiagnostic::ReadyWaitingForRoom
                    })
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        eprintln!(
            "stage {stage}: local={:?}, placeholder={}, coordination={:?}, pending={}, reset={}",
            owner.player_local_file,
            owner.player_local_file_placeholder,
            owner
                .session
                .as_ref()
                .unwrap()
                .playback_coordination_snapshot(),
            owner.pending_shared_playlist_open.is_some(),
            owner
                .session
                .as_ref()
                .unwrap()
                .has_pending_playlist_index_reset_intent()
        );
        assert_eq!(
            owner
                .player_local_file
                .as_ref()
                .map(|file| file.name.as_str()),
            Some(expected)
        );
        assert!(
            !owner.player_local_file_placeholder,
            "physical player must confirm selection"
        );
        if stage == 0 {
            if !pipelined_playlist {
                if std::env::var_os("SOROTTE_TEST_START_GATE").is_some() {
                    use sorotte_protocol::*;
                    let logical_id = sorotte_client_core::logical_media_id_for_local_file_update(
                        owner.player_local_file.as_ref().unwrap(),
                    );
                    let prepare = PrepareMediaPayload::request(
                        1,
                        logical_id.as_str(),
                        0.0,
                        PlaybackBarrierPolicy::AllEligible,
                        MediaLoadIntent::NewPlayback,
                    )
                    .with_request_id("fixture-start");
                    let policy =
                        RoomBufferingPolicyPayload::new(0, RoomBufferingPolicy::PauseAnyEligible)
                            .with_request_nonce(1)
                            .with_request_id("fixture-start");
                    remote_commands.lock().unwrap().push(
                        encode_message_line(&ProtocolMessage::set(
                            SetPayload::new().with_playback_barrier_v1(
                                PlaybackBarrierSetExtension::new()
                                    .with_prepare(prepare)
                                    .with_buffering_policy(policy),
                            ),
                        ))
                        .unwrap(),
                    );
                    handle.push_request(GuiRuntimeRequest::SetLocalReady(true));
                    let deadline = Instant::now() + Duration::from_secs(10);
                    while Instant::now() < deadline {
                        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    eprintln!(
                        "coordinated room: {:?}",
                        observer.lock().unwrap().readiness_snapshot()
                    );
                    assert!(
                        observer.lock().unwrap().playback_barrier_commit().is_some(),
                        "coordinated fixture must commit before pause/resume"
                    );
                    assert_eq!(
                        observer
                            .lock()
                            .unwrap()
                            .playback_barrier_status()
                            .unwrap()
                            .phase,
                        PlaybackBarrierPhase::Complete,
                        "both clients must have acknowledged playback before testing resume"
                    );
                }
                owner.player.as_mut().unwrap().set_paused(false).unwrap();
                let deadline = Instant::now() + Duration::from_millis(2500);
                while Instant::now() < deadline {
                    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
                    std::thread::sleep(Duration::from_millis(20));
                }
                let snapshot = owner
                    .session
                    .as_ref()
                    .unwrap()
                    .playback_coordination_snapshot()
                    .unwrap();
                eprintln!("playing snapshot: {snapshot:?}");
                assert!(
                    owner.player_position_seconds.unwrap_or_default() > 1.5,
                    "real player must be advancing"
                );
                let statuses = statuses.lock().unwrap();
                let status = statuses.last().expect("server must broadcast status");
                assert_eq!(status["availability"], "fresh");
                assert_eq!(
                    status["phase"], "playing",
                    "physical playback cannot be reported as Loading during policy recovery: {snapshot:?}"
                );
                drop(statuses);
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs_f64();
                let peer_status = observer
                    .lock()
                    .unwrap()
                    .user_participant_status_at("alice", now)
                    .expect("second client must receive the participant report");
                assert_eq!(
                    peer_status.status.phase,
                    Some(sorotte_protocol::ParticipantPlaybackPhase::Playing)
                );
                assert_eq!(
                    peer_status.status.availability,
                    sorotte_protocol::ParticipantStatusAvailability::Fresh
                );
                let (_watch_sender, watch_receiver) = std::sync::mpsc::channel();
                if std::env::var_os("SOROTTE_TEST_SLOW_WATCH_SYNC").is_some() {
                    owner.plex_sync_engine = None;
                    owner.plex_sync_rx = Some(watch_receiver);
                }
                for iteration in 0..3 {
                    let position = owner.player_position_seconds.unwrap();
                    if std::env::var_os("SOROTTE_TEST_LOCAL_CONTROLS").is_some() {
                        handle.push_request(GuiRuntimeRequest::SetPlaybackPaused(true));
                    } else {
                        remote_commands.lock().unwrap().push(serde_json::json!({"State":{"playstate":{"position":position,"paused":true,"doSeek":false}}}).to_string());
                    }
                    let deadline = Instant::now()
                        + Duration::from_secs(
                            std::env::var("SOROTTE_TEST_PAUSE_SECONDS")
                                .ok()
                                .and_then(|value| value.parse().ok())
                                .unwrap_or(2),
                        );
                    while Instant::now() < deadline {
                        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    assert_eq!(owner.player_paused, Some(true));
                    let position = owner.player_position_seconds.unwrap();
                    let resume_position = position
                        + std::env::var("SOROTTE_TEST_RESUME_OFFSET")
                            .ok()
                            .and_then(|value| value.parse::<f64>().ok())
                            .unwrap_or(0.0);
                    let resumed_at = Instant::now();
                    if std::env::var_os("SOROTTE_TEST_LOCAL_CONTROLS").is_some() {
                        handle.push_request(GuiRuntimeRequest::SetPlaybackPaused(false));
                    } else {
                        remote_commands.lock().unwrap().push(serde_json::json!({"State":{"playstate":{"position":resume_position,"paused":false,"doSeek":false}}}).to_string());
                    }
                    let deadline = resumed_at + Duration::from_secs(35);
                    while Instant::now() < deadline {
                        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
                        if owner.player_paused == Some(false)
                            && owner
                                .player_position_seconds
                                .is_some_and(|current| current > position + 0.05)
                        {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    let latency = resumed_at.elapsed();
                    eprintln!(
                        "resume latency network={network} plex={plex} iteration={iteration}: {latency:?}"
                    );
                    assert!(
                        latency < Duration::from_secs(1),
                        "loaded stream resume must be prompt: {latency:?}"
                    );
                }
                return;
            }
            hold_responses.store(true, Ordering::SeqCst);
            handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
                paths: vec![second.clone()],
                load_into_shared_playlist: true,
                playlist_insert_slot: Some(1),
            });
            pump_and_apply_runtime_owner_actions_until(
                &mut owner,
                &handle,
                &mut state,
                Duration::from_secs(3),
                |s| s.main_window.playlist.len() == 2,
                "append appears before server reply",
            );
            handle.push_request(GuiRuntimeRequest::SetPlaylistIndex(1));
            for _ in 0..12 {
                pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
            }
            hold_responses.store(false, Ordering::SeqCst);
        }
    }
    handle.push_request(GuiRuntimeRequest::DeletePlaylistIndex(0));
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        Duration::from_secs(3),
        |s| s.main_window.playlist.len() == 1 && s.main_window.playlist[0].label == "episode2.wav",
        "real player session accepts edits after second selection",
    );
}

#[cfg(windows)]
struct DirectHttpFixture {
    address: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[cfg(windows)]
impl DirectHttpFixture {
    fn new(body: Vec<u8>) -> Self {
        use std::io::{BufRead, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if worker_stop.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(mut stream) = stream else {
                    break;
                };
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(1)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(std::time::Duration::from_secs(1)))
                    .unwrap();
                let mut reader = std::io::BufReader::new(&stream);
                let mut request = String::new();
                let mut offset = 0;
                loop {
                    request.clear();
                    if reader.read_line(&mut request).unwrap_or_default() == 0 || request == "\r\n"
                    {
                        break;
                    }
                    if let Some(range) = request
                        .trim()
                        .to_ascii_lowercase()
                        .strip_prefix("range: bytes=")
                    {
                        offset = range
                            .split('-')
                            .next()
                            .unwrap()
                            .parse::<usize>()
                            .unwrap_or(0)
                            .min(body.len());
                    }
                }
                let header = if offset > 0 {
                    format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {offset}-{}/{}\r\n",
                        body.len() - 1,
                        body.len()
                    )
                } else {
                    "HTTP/1.1 200 OK\r\n".into()
                };
                let header = format!(
                    "{header}Content-Type: audio/wav\r\nAccept-Ranges: bytes\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len() - offset
                );
                if stream.write_all(header.as_bytes()).is_ok() {
                    let _ = stream.write_all(&body[offset..]);
                }
            }
        });
        Self {
            address,
            stop,
            thread: Some(thread),
        }
    }
}

#[cfg(windows)]
impl Drop for DirectHttpFixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = std::net::TcpStream::connect(self.address);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}
