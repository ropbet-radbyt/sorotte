//! Opt-in real-player / TCP lifecycle experiment. No native input injection.
use super::*;
use crate::app::feature_slices::GuiRuntimeInput;
use crate::app::latency_review_probe as trace;
use crate::app::runtime_bridge::{GuiNativeRuntimePump, GuiQueuedRuntimeOwner};
use crate::app::runtime_queue::GuiThreadedRuntimeOwnerPump;
use sorotte_client_app::app_boundary::state::TlsPolicy;
use sorotte_player_mpv::{MpvAdapter, managed_process::ManagedMpvCommand};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Default, Clone, Debug, serde::Serialize)]
struct Observation {
    connected: bool,
    filename: Option<String>,
    placeholder: bool,
    paused: Option<bool>,
    position: Option<f64>,
    playlist_count: usize,
    mm_worker: bool,
    mm_decision: Option<String>,
    mm_nearest: Option<String>,
    mm_status: Option<String>,
    mm_candidate: Option<String>,
}
struct ObservedOwner {
    name: String,
    owner: GuiPersistedConfigRuntimeOwner,
    observation: Arc<Mutex<Observation>>,
}
impl GuiQueuedRuntimeOwner for ObservedOwner {
    fn input_changed(&mut self, handle: &GuiQueuedRuntimeBridgeHandle, input: &GuiRuntimeInput) {
        trace::client(&self.name);
        self.owner.input_changed(handle, input);
    }
    fn poll(&mut self, handle: &GuiQueuedRuntimeBridgeHandle) {
        trace::client(&self.name);
        self.owner.poll(handle);
        *self.observation.lock().unwrap() = Observation {
            connected: self
                .owner
                .session
                .as_ref()
                .is_some_and(|s| s.server_handshake_completed()),
            filename: self
                .owner
                .player_local_file
                .as_ref()
                .map(|f| f.name.clone()),
            placeholder: self.owner.player_local_file_placeholder,
            paused: self.owner.player_paused,
            position: self.owner.player_position_seconds,
            playlist_count: self
                .owner
                .runtime_state
                .as_ref()
                .map_or(0, |s| s.playlist.main_window.playlist.len()),
            mm_worker: self.owner.media_match_background_worker_rx.is_some(),
            mm_decision: self
                .owner
                .media_match_runtime_snapshot
                .current_decision
                .clone(),
            mm_nearest: self
                .owner
                .media_match_runtime_snapshot
                .nearest_match
                .clone(),
            mm_status: self
                .owner
                .media_match_runtime_snapshot
                .background_status
                .clone(),
            mm_candidate: self
                .owner
                .media_match_remote_lookup_result
                .as_ref()
                .and_then(|result| result.candidate_path.clone()),
        };
    }
}
struct Client {
    name: String,
    pump: GuiThreadedRuntimeOwnerPump,
    handle: GuiQueuedRuntimeBridgeHandle,
    state: SorotteGuiShellAppState,
    observation: Arc<Mutex<Observation>>,
    physical_log: PathBuf,
    physical_lines: usize,
    physical_filename: Option<String>,
    physical_paused: Option<bool>,
    physical_position: f64,
    target_seen_at: Option<Instant>,
}
impl Client {
    fn service_ui(&mut self) {
        trace::client(&self.name);
        let old_count = self.state.main_window.playlist.len();
        for action in self.handle.drain_actions() {
            if let GuiShellAction::PushTransientNotification { level, message } = &action {
                trace::mark(&format!("feedback:{level:?}:{message}"));
            }
            assert!(self.state.apply(action));
        }
        if old_count != self.state.main_window.playlist.len() {
            trace::mark(&format!(
                "ui.playlist-count:{}",
                self.state.main_window.playlist.len()
            ));
        }
        self.pump.pump(&self.state);
        if let Ok(text) = std::fs::read_to_string(&self.physical_log) {
            let lines: Vec<_> = text.lines().collect();
            for line in lines.iter().skip(self.physical_lines) {
                if let Some((kind, value)) = line.split_once('|') {
                    match kind {
                        "loaded" => {
                            self.physical_filename = Some(value.to_owned());
                            trace::mark(&format!("mpv.loaded:{value}"));
                        }
                        "pause" => {
                            self.physical_paused = Some(value == "true");
                            trace::mark(&format!("mpv.pause:{value}"));
                        }
                        "position" => {
                            let next = value.parse::<f64>().unwrap_or(0.0);
                            if (next - self.physical_position).abs() > 0.5 {
                                trace::mark(&format!("mpv.position:{next}"));
                            }
                            if (next - 10.0).abs() < 0.25 && self.target_seen_at.is_none() {
                                self.target_seen_at = Some(Instant::now());
                            }
                            self.physical_position = next;
                        }
                        _ => {}
                    }
                }
            }
            self.physical_lines = lines.len();
        }
    }
}
struct ProbeServer {
    stop: tokio::sync::watch::Sender<bool>,
    worker: Option<std::thread::JoinHandle<()>>,
    endpoint: String,
}
impl ProbeServer {
    fn start() -> Self {
        let (stop, rx) = tokio::sync::watch::channel(false);
        let (tx, ready) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let endpoint = listener.local_addr().unwrap().to_string();
                let actor = sorotte_server::ServerActorHandle::spawn(
                    sorotte_server::ServerRuntime::default(),
                );
                tx.send(endpoint).unwrap();
                sorotte_server::run_server_network_loop_until_shutdown(
                    listener,
                    actor.clone(),
                    None,
                    rx,
                )
                .await
                .unwrap();
                actor.shutdown().await.unwrap();
            });
        });
        Self {
            stop,
            worker: Some(worker),
            endpoint: ready.recv_timeout(Duration::from_secs(10)).unwrap(),
        }
    }
}
impl Drop for ProbeServer {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}
struct SaveTrace(PathBuf);
impl Drop for SaveTrace {
    fn drop(&mut self) {
        trace::save(&self.0);
    }
}

fn copy_tree(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn wait_clients(
    clients: &mut [Client],
    timeout: Duration,
    predicate: impl Fn(&Client) -> bool,
    context: &str,
) -> Vec<f64> {
    let start = Instant::now();
    let mut times = vec![None; clients.len()];
    loop {
        for (index, client) in clients.iter_mut().enumerate() {
            client.service_ui();
            if times[index].is_none() && predicate(client) {
                times[index] = Some(start.elapsed().as_secs_f64() * 1000.0);
                trace::mark(&format!("complete:{context}"));
            }
        }
        if times.iter().all(Option::is_some) {
            return times.into_iter().map(Option::unwrap).collect();
        }
        assert!(
            start.elapsed() < timeout,
            "timeout: {context}; observations: {:?}; physical: {:?}",
            clients
                .iter()
                .map(|c| c.observation.lock().unwrap().clone())
                .collect::<Vec<_>>(),
            clients
                .iter()
                .map(|c| (
                    &c.name,
                    &c.physical_filename,
                    c.physical_paused,
                    c.physical_position
                ))
                .collect::<Vec<_>>()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn settle(clients: &mut [Client], duration: Duration) {
    let start = Instant::now();
    while start.elapsed() < duration {
        for client in clients.iter_mut() {
            client.service_ui();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn action(
    clients: &mut [Client],
    request: GuiRuntimeRequest,
    label: &str,
    predicate: impl Fn(&Client) -> bool,
    rows: &mut Vec<serde_json::Value>,
) {
    trace::client("controller");
    trace::mark(&format!("submit:{label}"));
    clients[0].handle.push_request(request);
    let times = wait_clients(clients, Duration::from_secs(45), predicate, label);
    let row = serde_json::json!({"action":label,"observed_ms":times});
    println!("LATENCY {row}");
    rows.push(row);
}

fn review_seek(clients: &mut [Client], label: &str, rows: &mut Vec<serde_json::Value>) {
    for client in clients.iter_mut() {
        client.target_seen_at = None;
    }
    trace::client("controller");
    trace::mark(&format!("submit:{label}"));
    let start = Instant::now();
    clients[0]
        .handle
        .push_request(GuiRuntimeRequest::SeekToPosition(10.0));
    settle(clients, Duration::from_secs(2));
    let row = serde_json::json!({"action":label,
        "first_target_observed_ms":clients.iter().map(|c|c.target_seen_at.map(|t|t.duration_since(start).as_secs_f64()*1000.0)).collect::<Vec<_>>(),
        "settled_at_target":clients.iter().map(|c|(c.physical_position-10.0).abs()<0.25).collect::<Vec<_>>(),
        "final_positions":clients.iter().map(|c|c.physical_position).collect::<Vec<_>>()});
    println!("LATENCY {row}");
    rows.push(row);
}

#[test]
#[ignore = "requires explicit generated media, mpv and optional tools; runs isolated real TCP and player measurements"]
fn latency_review_real_players() {
    let output =
        PathBuf::from(std::env::var_os("SOROTTE_LATENCY_OUTPUT").expect("explicit output"));
    std::fs::create_dir_all(&output).unwrap();
    trace::start();
    let _save = SaveTrace(output.join("stage-trace.json"));
    let executable = std::env::var_os("SOROTTE_TEST_MPV_BIN").expect("explicit mpv");
    let source =
        PathBuf::from(std::env::var_os("SOROTTE_TEST_MEDIA").expect("generated test media"));
    let tools = std::env::var_os("SOROTTE_TEST_MEDIA_TOOLS").map(PathBuf::from);
    let seed = std::env::var_os("SOROTTE_REVIEW_SEED_INDEX").map(PathBuf::from);
    let mm = std::env::var("SOROTTE_REVIEW_MM").unwrap_or_else(|_| "on".into()) == "on";
    let required_index = std::env::var_os("SOROTTE_REVIEW_REQUIRED_INDEX").is_some();
    let server = ProbeServer::start();
    let instance = tempfile::tempdir().unwrap();
    let mut processes = Vec::new();
    let mut clients = Vec::new();
    let mut media_paths = Vec::new();
    for (index, name) in ["host", "peer-mm", "peer-local"].into_iter().enumerate() {
        let enabled = mm && index != 2;
        let root = instance.path().join(name);
        let media = root.join("media");
        std::fs::create_dir_all(&media).unwrap();
        for n in 1..=3 {
            if required_index && index == 1 {
                if n == 1 {
                    std::fs::copy(&source, media.join("alternate-copy.mkv")).unwrap();
                }
            } else {
                std::fs::copy(&source, media.join(format!("episode-{n}.mkv"))).unwrap();
            }
        }
        if index == 0 {
            media_paths = (1..=3)
                .map(|n| {
                    media
                        .join(format!("episode-{n}.mkv"))
                        .to_string_lossy()
                        .into_owned()
                })
                .collect::<Vec<_>>();
        }
        if enabled {
            let bin = crate::app::media_match_support::managed_media_match_bin_dir(&root);
            std::fs::create_dir_all(&bin).unwrap();
            for tool in ["ffmpeg.exe", "ffprobe.exe"] {
                std::fs::copy(tools.as_ref().expect("tools").join(tool), bin.join(tool)).unwrap();
            }
            if let Some(seed) = &seed {
                copy_tree(seed, &root.join("cache/media-match"));
            }
        }
        let physical_log = output.join(format!("{name}-mpv.log"));
        let script = root.join("observe.lua");
        let lua = format!(
            r#"local out = [=[{}]=]
local function emit(k,v)
 local f=io.open(out,"a")
 if f then f:write(k.."|"..tostring(v).."\n"); f:close() end
end
mp.register_event("file-loaded",function() emit("loaded",mp.get_property("filename")) end)
mp.observe_property("pause","bool",function(_,v) emit("pause",v) end)
mp.observe_property("time-pos","number",function(_,v) if v then emit("position",v) end end)
local last_command = ""
mp.add_periodic_timer(0.01,function()
 local f=io.open(out..".command","r")
 if not f then return end
 local value=f:read("*a"); f:close()
 if value ~= last_command then
  last_command=value
  mp.set_property_bool("pause",value == "pause")
 end
end)
"#,
            physical_log.to_string_lossy().replace('\\', "/")
        );
        std::fs::write(&script, lua).unwrap();
        let endpoint = format!(r"\\.\pipe\sorotte-latency-{}-{name}", std::process::id());
        processes.push(
            ManagedMpvCommand::new(&executable)
                .args([
                    "--no-config",
                    "--no-terminal",
                    "--idle=yes",
                    "--force-window=no",
                    "--ao=null",
                    "--vo=null",
                    "--pause=yes",
                    "--keep-open=yes",
                ])
                .args([
                    format!("--input-ipc-server={endpoint}"),
                    format!("--script={}", script.display()),
                ])
                .spawn(None)
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        let player = loop {
            match MpvAdapter::with_json_ipc(&endpoint) {
                Ok(player) => break player,
                Err(error) => {
                    assert!(Instant::now() < deadline, "{error}");
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        };
        let settings = StoredClientSettings {
            username: Some(name.into()),
            room: Some("latency-room".into()),
            player_path: Some(executable.to_string_lossy().into_owned()),
            shared_playlist_enabled: Some(true),
            media_search_directories: Some(vec![media.to_string_lossy().into_owned()]),
            media_matching_plugin_enabled: Some(enabled),
            media_match_fingerprinting_enabled: Some(enabled),
            media_match_wire_sharing_enabled: Some(enabled),
            media_match_background_warmup_enabled: Some(required_index && index == 1),
            stream_support_plugin_enabled: Some(false),
            plex_plugin_enabled: Some(false),
            plex_sync_enabled: Some(false),
            ready_at_start: Some(true),
            ..StoredClientSettings::default()
        };
        let state = SorotteGuiShellAppState::from_stored_settings(&settings);
        let mut owner =
            GuiPersistedConfigRuntimeOwner::with_config_path(Some(root.join("sorotte.json")))
                .with_client_core_chat_tcp_session_runtime(
                    name,
                    "latency-room",
                    &server.endpoint,
                    TlsPolicy::PreferTls,
                )
                .unwrap();
        owner.startup_saved_connect_attempted = true;
        owner.startup_remote_actions_attempted = true;
        owner.startup_public_server_hydration.completed = true;
        owner.startup_stream_helper_probe_completed = true;
        if enabled {
            owner.media_match_runtime_snapshot =
                crate::app::media_match_support::probe_media_match_runtime_snapshot_with_cancel(
                    Some(&root),
                    &state.media_match.settings,
                    None,
                );
        }
        owner
            .complete_mpv_attachment_after_core_configuration(
                player,
                None,
                &sorotte_player_mpv::SyncplayUiSettings::default(),
            )
            .unwrap();
        owner.report_external_player_availability(
            sorotte_client_core::ExternalPlayerAvailability::Connecting,
        );
        let observation = Arc::new(Mutex::new(Observation::default()));
        let handle = GuiQueuedRuntimeBridgeHandle::default();
        let pump = GuiThreadedRuntimeOwnerPump::new(
            handle.clone(),
            ObservedOwner {
                name: name.into(),
                owner,
                observation: observation.clone(),
            },
        )
        .unwrap();
        clients.push(Client {
            name: name.into(),
            pump,
            handle,
            state,
            observation,
            physical_log,
            physical_lines: 0,
            physical_filename: None,
            physical_paused: None,
            physical_position: 0.0,
            target_seen_at: None,
        });
    }
    wait_clients(
        &mut clients,
        Duration::from_secs(15),
        |c| c.observation.lock().unwrap().connected,
        "connected",
    );
    settle(&mut clients, Duration::from_millis(300));
    let mut rows = Vec::new();
    if required_index {
        trace::client("test");
        trace::mark("submit:required-index-first-add");
        let start = Instant::now();
        clients[0]
            .handle
            .push_request(GuiRuntimeRequest::OpenMediaFiles {
                paths: vec![media_paths[0].clone()],
                load_into_shared_playlist: true,
                playlist_insert_slot: None,
            });
        let mut row_seen = vec![None; clients.len()];
        let mut loaded = vec![None; clients.len()];
        let mut append_seen = vec![None; clients.len()];
        let mut append_start = None;
        let mut peer_was_waiting_at_append = false;
        loop {
            for (index, client) in clients.iter_mut().enumerate() {
                client.service_ui();
                if row_seen[index].is_none() && !client.state.main_window.playlist.is_empty() {
                    row_seen[index] = Some(start.elapsed().as_secs_f64() * 1000.0);
                }
                let expected = if index == 1 {
                    "alternate-copy.mkv"
                } else {
                    "episode-1.mkv"
                };
                if loaded[index].is_none() && client.physical_filename.as_deref() == Some(expected)
                {
                    loaded[index] = Some(start.elapsed().as_secs_f64() * 1000.0);
                }
                if let Some(append_at) = append_start
                    && append_seen[index].is_none()
                    && client.state.main_window.playlist.len() == 2
                {
                    append_seen[index] =
                        Some(Instant::now().duration_since(append_at).as_secs_f64() * 1000.0);
                }
            }
            if append_start.is_none()
                && clients
                    .iter()
                    .any(|client| client.observation.lock().unwrap().mm_worker)
                && row_seen.iter().all(Option::is_some)
            {
                peer_was_waiting_at_append = loaded[1].is_none();
                trace::client("test");
                trace::mark("submit:append-while-indexing");
                append_start = Some(Instant::now());
                clients[0]
                    .handle
                    .push_request(GuiRuntimeRequest::OpenMediaFiles {
                        paths: vec![media_paths[1].clone()],
                        load_into_shared_playlist: true,
                        playlist_insert_slot: Some(1),
                    });
            }
            if loaded.iter().all(Option::is_some) && append_seen.iter().all(Option::is_some) {
                break;
            }
            if start.elapsed() > Duration::from_secs(90) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let host_record = crate::app::media_match_support::media_match_record_for_path(
            &instance.path().join("host"),
            &media_paths[0],
            &sorotte_media_match::MediaExtractionSettings::sampled_fast_audio_index_v3(),
        );
        let peer_record = crate::app::media_match_support::media_match_record_for_path(
            &instance.path().join("peer-mm"),
            &instance
                .path()
                .join("peer-mm/media/alternate-copy.mkv")
                .to_string_lossy(),
            &sorotte_media_match::MediaExtractionSettings::sampled_fast_audio_index_v3(),
        );
        let direct_comparison =
            host_record
                .as_ref()
                .zip(peer_record.as_ref())
                .map(|(host, peer)| {
                    let wire = sorotte_media_match::media_match_wire_value_from_records(
                        std::slice::from_ref(host),
                    )
                    .unwrap();
                    let signature =
                        sorotte_media_match::media_match_wire_signature_from_value(&wire).unwrap();
                    format!(
                        "{:?}",
                        sorotte_media_match::decide_media_match_against_wire_signature(
                            peer,
                            &signature,
                            &Default::default()
                        )
                    )
                });
        let result = serde_json::json!({"scenario":"required-index", "playlist_row_ms":row_seen,
            "host_record_present":host_record.is_some(),"peer_record_present":peer_record.is_some(),"direct_comparison":direct_comparison,
            "physical_load_ms":loaded, "append_while_indexing_ms":append_seen,
            "peer_was_waiting_at_append":peer_was_waiting_at_append,
            "observations":clients.iter().map(|client| client.observation.lock().unwrap().clone()).collect::<Vec<_>>()});
        std::fs::write(
            output.join("measurements.json"),
            serde_json::to_vec_pretty(&result).unwrap(),
        )
        .unwrap();
        println!("LATENCY {result}");
        assert!(
            loaded.iter().all(Option::is_some),
            "Media Match did not resolve the differently named copy: {result}"
        );
        assert!(
            append_seen.iter().all(Option::is_some),
            "append did not propagate during indexing: {result}"
        );
        drop(clients);
        drop(processes);
        return;
    }

    action(
        &mut clients,
        GuiRuntimeRequest::OpenMediaFiles {
            paths: vec![media_paths[0].clone()],
            load_into_shared_playlist: true,
            playlist_insert_slot: None,
        },
        "add-first",
        |c| c.physical_filename.as_deref() == Some("episode-1.mkv"),
        &mut rows,
    );
    // Let the first fingerprint finish before the warm-cache/control sequence.
    settle(&mut clients, Duration::from_secs(5));
    action(
        &mut clients,
        GuiRuntimeRequest::OpenMediaFiles {
            paths: vec![media_paths[1].clone()],
            load_into_shared_playlist: true,
            playlist_insert_slot: Some(1),
        },
        "append-second",
        |c| c.state.main_window.playlist.len() == 2,
        &mut rows,
    );
    action(
        &mut clients,
        GuiRuntimeRequest::SetPlaylistIndex(1),
        "select-second",
        |c| c.physical_filename.as_deref() == Some("episode-2.mkv"),
        &mut rows,
    );
    action(
        &mut clients,
        GuiRuntimeRequest::OpenMediaFiles {
            paths: vec![media_paths[2].clone()],
            load_into_shared_playlist: true,
            playlist_insert_slot: Some(2),
        },
        "append-third-during-signature",
        |c| c.state.main_window.playlist.len() == 3,
        &mut rows,
    );
    settle(&mut clients, Duration::from_secs(5));
    for client in &clients {
        client
            .handle
            .push_request(GuiRuntimeRequest::SetLocalReady(true));
    }
    settle(&mut clients, Duration::from_millis(500));
    for trial in 0..3 {
        for client in &clients {
            client
                .handle
                .push_request(GuiRuntimeRequest::SetLocalReady(true));
        }
        settle(&mut clients, Duration::from_millis(600));
        action(
            &mut clients,
            GuiRuntimeRequest::SetPlaybackPaused(false),
            &format!("play-{trial}"),
            |c| c.physical_paused == Some(false),
            &mut rows,
        );
        settle(&mut clients, Duration::from_millis(200));
        action(
            &mut clients,
            GuiRuntimeRequest::SetPlaybackPaused(true),
            &format!("pause-{trial}"),
            |c| c.physical_paused == Some(true),
            &mut rows,
        );
        settle(&mut clients, Duration::from_millis(200));
    }
    review_seek(&mut clients, "seek-after-pause", &mut rows);
    review_seek(&mut clients, "seek-after-settle", &mut rows);
    for trial in 0..6 {
        for paused in [false, true] {
            for client in &clients {
                client
                    .handle
                    .push_request(GuiRuntimeRequest::SetLocalReady(true));
            }
            settle(&mut clients, Duration::from_millis(250));
            let action_name = if paused {
                "native-peer-pause"
            } else {
                "native-peer-play"
            };
            let label = format!("{action_name}-{trial}");
            trace::client("controller");
            trace::mark(&format!("submit:{label}"));
            std::fs::write(
                format!("{}.command", clients[1].physical_log.display()),
                if paused { "pause" } else { "play" },
            )
            .unwrap();
            let started = Instant::now();
            let mut timings = vec![None; clients.len()];
            while started.elapsed() < Duration::from_millis(1500) {
                for (index, client) in clients.iter_mut().enumerate() {
                    client.service_ui();
                    if timings[index].is_none() && client.physical_paused == Some(paused) {
                        timings[index] = Some(started.elapsed().as_secs_f64() * 1000.0);
                    }
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            rows.push(serde_json::json!({"action":label,"ms":timings,
            "settled":clients.iter().map(|client| client.physical_paused == Some(paused)).collect::<Vec<_>>()}));
            settle(&mut clients, Duration::from_millis(250));
        }
    }
    action(
        &mut clients,
        GuiRuntimeRequest::DeletePlaylistIndex(0),
        "delete-after-select",
        |c| {
            c.state.main_window.playlist.len() == 2
                && c.state.main_window.playlist[0].label == "episode-2.mkv"
        },
        &mut rows,
    );
    settle(&mut clients, Duration::from_millis(300));
    std::fs::write(output.join("measurements.json"),serde_json::to_vec_pretty(&serde_json::json!({
        "mm":mm,"seeded":seed.is_some(),"clients":["host","peer-mm","peer-local"],
        "boundaries":"production GUI runtime threads and TCP server, independent real mpv Lua observations; no rendered GUI or WAN",
        "rows":rows
    })).unwrap()).unwrap();
    assert!(
        rows.iter().all(|row| {
            row.get("settled")
                .or_else(|| row.get("settled_at_target"))
                .and_then(serde_json::Value::as_array)
                .is_none_or(|outcomes| {
                    outcomes
                        .iter()
                        .all(|outcome| outcome.as_bool() == Some(true))
                })
        }),
        "all measured native controls and seeks must settle at the requested state"
    );
    drop(clients);
    for process in processes {
        process
            .terminate_until(Instant::now() + Duration::from_secs(5))
            .unwrap();
    }
}
