use std::{collections::BTreeMap, fmt::Write as _, process::Stdio};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::*;

const REAL_MPV_SCHEMA_VERSION: u32 = 1;
const REAL_MPV_KIND: &str = "sorotte-gui-real-mpv-vertical";
const REAL_MPV_MEDIA_DURATION_SECONDS: u32 = 12;
const REAL_MPV_LOOPBACK_USERNAME: &str = "real-mpv-user";
const REAL_MPV_LOOPBACK_ROOM: &str = "real-mpv-room";
const REAL_MPV_SESSION_HELLO: &str = r#"{"Hello":{"username":"real-mpv-user","room":{"name":"real-mpv-room"},"version":"1.7.5","features":{"chat":true,"readiness":true,"sharedPlaylists":true}}}"#;
const REAL_MPV_SESSION_CAPABILITIES: &[&str] = &["chat", "readiness", "sharedPlaylists"];
const REAL_MPV_MENU_INTERACTIONS_KIND: &str = "sorotte-gui-real-mpv-menu-interactions";
const PLAY_CONTROL_AUTOMATION_ID: &str = "main-window:control:play";
const PAUSE_CONTROL_AUTOMATION_ID: &str = "main-window:control:pause";

#[derive(Debug, Clone, PartialEq, Eq)]
struct RealMpvVerticalOptions {
    binary_path: PathBuf,
    mpv_path: PathBuf,
    artifact_dir: PathBuf,
    timeout: Duration,
}

#[derive(Debug, Serialize)]
struct RealMpvVerticalState {
    schema_version: u32,
    kind: &'static str,
    result: String,
    stage: String,
    artifact_root: String,
    gui_binary: Option<String>,
    mpv_binary: Option<String>,
    gui_pid: Option<u32>,
    mpv_pid: Option<u32>,
    assertions: Vec<String>,
    error: Option<String>,
}

impl RealMpvVerticalState {
    fn new(artifact_root: &Path) -> Self {
        Self {
            schema_version: REAL_MPV_SCHEMA_VERSION,
            kind: REAL_MPV_KIND,
            result: "running".to_owned(),
            stage: "initialize".to_owned(),
            artifact_root: artifact_root.display().to_string(),
            gui_binary: None,
            mpv_binary: None,
            gui_pid: None,
            mpv_pid: None,
            assertions: Vec::new(),
            error: None,
        }
    }

    fn advance(
        &mut self,
        state_path: &Path,
        stage: &str,
        assertion: Option<&str>,
    ) -> Result<(), String> {
        self.stage = stage.to_owned();
        if let Some(assertion) = assertion {
            self.assertions.push(assertion.to_owned());
        }
        write_json_file(state_path, self)
    }
}

#[derive(Debug, Serialize)]
struct BinaryIdentity {
    path: String,
    bytes: u64,
    sha256: String,
}

#[derive(Debug, Serialize)]
struct MpvIdentity {
    path: String,
    bytes: u64,
    sha256: String,
    version: String,
    minimum_supported_version: &'static str,
    pid: u32,
    parent_pid: u32,
    process_image_path: String,
}

#[derive(Debug, Serialize)]
struct IsolationContract {
    artifact_root: String,
    config_path: String,
    appdata_root: String,
    media_path: String,
    observation_script_path: String,
    observation_path: String,
    mpv_log_path: String,
    lifecycle_path: String,
    session_exchange_path: String,
    menu_interactions_path: String,
    ipc_endpoint: String,
    session_endpoint: String,
    session_peer_endpoint: String,
    session_advertised_capabilities: Vec<&'static str>,
    network_mode: &'static str,
    media_source: &'static str,
    mpv_config: &'static str,
}

#[derive(Debug, Serialize)]
struct ArtifactIdentity {
    path: String,
    bytes: u64,
    sha256: String,
}

#[derive(Debug, Serialize)]
struct RealMpvVerticalReport {
    schema_version: u32,
    kind: &'static str,
    result: &'static str,
    capability: &'static str,
    gui: BinaryIdentity,
    mpv: MpvIdentity,
    isolation: IsolationContract,
    assertions: Vec<String>,
    artifacts: BTreeMap<String, ArtifactIdentity>,
    duration_ms: u128,
}

#[derive(Debug, Clone, Deserialize)]
struct MpvObservation {
    event: String,
    pid: Option<u32>,
    path: Option<String>,
    filename: Option<String>,
    duration: Option<f64>,
    pause: Option<bool>,
    ipc_endpoint: Option<String>,
}

#[derive(Debug)]
struct MpvPreflight {
    identity: BinaryIdentity,
    version: String,
}

#[derive(Debug, Serialize)]
struct SessionExchangeEvidence {
    schema_version: u32,
    kind: &'static str,
    result: String,
    bound_endpoint: String,
    connected_peer_endpoint: Option<String>,
    listener_ipv4_loopback: bool,
    peer_ipv4_loopback: Option<bool>,
    client_hello: Option<String>,
    server_hello: &'static str,
    advertised_capabilities: Vec<&'static str>,
    server_thread_released: bool,
    socket_released: bool,
    error: Option<String>,
}

impl SessionExchangeEvidence {
    fn new(bound_endpoint: String) -> Self {
        Self {
            schema_version: REAL_MPV_SCHEMA_VERSION,
            kind: "sorotte-gui-real-mpv-loopback-exchange",
            result: "running".to_owned(),
            bound_endpoint,
            connected_peer_endpoint: None,
            listener_ipv4_loopback: true,
            peer_ipv4_loopback: None,
            client_hello: None,
            server_hello: REAL_MPV_SESSION_HELLO,
            advertised_capabilities: REAL_MPV_SESSION_CAPABILITIES.to_vec(),
            server_thread_released: false,
            socket_released: false,
            error: None,
        }
    }
}

#[derive(Debug, Serialize)]
struct MenuSectionSnapshot {
    matching_nodes: usize,
    visible_nodes: usize,
    visible_enabled_nodes: usize,
    nodes: Vec<String>,
}

#[derive(Debug, Serialize)]
struct MenuInteractionRecord {
    section_automation_id: String,
    action_automation_id: String,
    section_open_strategy: String,
    pre_fallback_snapshots: Vec<MenuSectionSnapshot>,
    opened_snapshot: Option<MenuSectionSnapshot>,
    leaf_delivery: &'static str,
    leaf_delivered: bool,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct MenuInteractionsEvidence {
    schema_version: u32,
    kind: &'static str,
    result: String,
    interactions: Vec<MenuInteractionRecord>,
    error: Option<String>,
}

impl MenuInteractionsEvidence {
    fn new() -> Self {
        Self {
            schema_version: REAL_MPV_SCHEMA_VERSION,
            kind: REAL_MPV_MENU_INTERACTIONS_KIND,
            result: "running".to_owned(),
            interactions: Vec::new(),
            error: None,
        }
    }
}

pub(crate) fn run_real_mpv_vertical_from_args(args: &[String]) -> Result<String, String> {
    let options = parse_real_mpv_vertical_options(args)?;
    fs::create_dir_all(&options.artifact_dir).map_err(|error| {
        format!(
            "failed to create real-mpv artifact root {}: {error}",
            options.artifact_dir.display()
        )
    })?;
    let artifact_root = fs::canonicalize(&options.artifact_dir).map_err(|error| {
        format!(
            "failed to resolve real-mpv artifact root {}: {error}",
            options.artifact_dir.display()
        )
    })?;
    let state_path = artifact_root.join("real-mpv-state.json");
    let session_exchange_path = artifact_root.join("session-exchange.json");
    let menu_interactions_path = artifact_root.join("menu-interactions.json");
    let mut state = RealMpvVerticalState::new(&artifact_root);
    write_json_file(&state_path, &state)?;
    let mut menu_interactions = MenuInteractionsEvidence::new();
    write_json_file(&menu_interactions_path, &menu_interactions)?;

    let started_at = Instant::now();
    let driver = PlatformNativeGuiDriver;
    let mut child: Option<Child> = None;
    let mut window = None;
    let mut verified_mpv_pid = None;
    let mut session_server: Option<MockSessionServer> = None;
    let mut session_exchange: Option<SessionExchangeEvidence> = None;

    let run_result = (|| -> Result<RealMpvVerticalReport, String> {
        #[cfg(not(target_os = "windows"))]
        {
            return Err(
                "the genuine native GUI-to-real-mpv vertical currently requires Windows UI Automation and Windows mpv IPC"
                    .to_owned(),
            );
        }

        let binary_path = resolve_binary_path(&options.binary_path)?;
        let mpv_path = fs::canonicalize(&options.mpv_path).map_err(|error| {
            format!(
                "failed to resolve required mpv binary {}: {error}",
                options.mpv_path.display()
            )
        })?;
        state.gui_binary = Some(binary_path.display().to_string());
        state.mpv_binary = Some(mpv_path.display().to_string());
        state.advance(&state_path, "preflight", None)?;

        let gui_identity = binary_identity(&binary_path)?;
        let mpv_preflight = preflight_supported_mpv(&mpv_path)?;
        state.advance(
            &state_path,
            "preflight-complete",
            Some("supported-mpv-version-and-digest"),
        )?;

        let config_path = artifact_root.join("sorotte-real-mpv.ini");
        let appdata_root = artifact_root.join("appdata");
        let media_path = artifact_root.join("generated-silence.wav");
        let observation_script_path = artifact_root.join("observe-real-mpv.lua");
        let observation_path = artifact_root.join("mpv-observation.jsonl");
        let mpv_log_path = artifact_root.join("mpv.log");
        let lifecycle_path = artifact_root.join("gui-lifecycle.jsonl");
        let success_screenshot_path = artifact_root.join("success-real-mpv.png");
        fs::create_dir_all(&appdata_root).map_err(|error| {
            format!(
                "failed to create isolated APPDATA root {}: {error}",
                appdata_root.display()
            )
        })?;
        fs::write(&media_path, pcm_wav_bytes(REAL_MPV_MEDIA_DURATION_SECONDS)).map_err(
            |error| {
                format!(
                    "failed to write generated local media {}: {error}",
                    media_path.display()
                )
            },
        )?;
        fs::write(
            &observation_script_path,
            real_mpv_observation_lua(&observation_path),
        )
        .map_err(|error| {
            format!(
                "failed to write real-mpv observation script {}: {error}",
                observation_script_path.display()
            )
        })?;
        for path in [&observation_path, &mpv_log_path, &lifecycle_path] {
            fs::write(path, []).map_err(|error| {
                format!(
                    "failed to initialize retained artifact {}: {error}",
                    path.display()
                )
            })?;
        }

        seed_real_mpv_config(
            &config_path,
            &mpv_path,
            &observation_script_path,
            &mpv_log_path,
        )?;
        state.advance(
            &state_path,
            "isolated-fixtures-ready",
            Some("isolated-config-and-generated-local-media"),
        )?;

        let server = start_phased_mock_session_server(&[REAL_MPV_SESSION_HELLO])?;
        let session_endpoint = server.address.clone();
        let session_port = server.port;
        session_server = Some(server);
        require_ipv4_loopback_endpoint(&session_endpoint, "bound real-mpv session listener")?;
        let exchange = SessionExchangeEvidence::new(session_endpoint.clone());
        write_json_file(&session_exchange_path, &exchange)?;
        session_exchange = Some(exchange);
        let launch = GuiLaunchConfig {
            config_path: &config_path,
            media_search_browse_path: &artifact_root,
            open_media_file_path: &media_path,
            public_servers_spec: "[]",
            network_mode: NativeNetworkMode::TcpLoopback {
                bootstrap: NativeTcpBootstrap::Environment(TcpSessionBootstrap {
                    host: "127.0.0.1",
                    port: session_port,
                    username: REAL_MPV_LOOPBACK_USERNAME,
                    room: REAL_MPV_LOOPBACK_ROOM,
                }),
            },
            attach_test_player: false,
            drop_file_paths_spec: None,
            drop_target: None,
        };
        let (launched_child, launched_window) = launch_sorotte_gui_with_retry_and_test_overrides(
            &driver,
            &binary_path,
            launch,
            options.timeout,
            GuiLaunchTestOverrides {
                appdata_root: Some(&appdata_root),
                explicit_config_path_with_appdata_root: true,
                lifecycle_observation_path: Some(&lifecycle_path),
                ..GuiLaunchTestOverrides::default()
            },
        )?;
        let gui_pid = launched_child.id();
        state.gui_pid = Some(gui_pid);
        child = Some(launched_child);
        window = Some(launched_window);
        state.advance(
            &state_path,
            "gui-window-ready",
            Some("actual-native-gui-window"),
        )?;

        let step_timeout = options.timeout.min(Duration::from_secs(12));
        wait_for_any_accessible_name(
            &driver,
            launched_window,
            &["view: setup", "view: room"],
            step_timeout,
        )?;
        let session_peer_endpoint = session_server
            .as_ref()
            .expect("real-mpv loopback server must remain live")
            .recv_peer(step_timeout, "real-mpv vertical")?;
        require_ipv4_loopback_endpoint(&session_peer_endpoint, "connected real-mpv session peer")?;
        let hello = session_server
            .as_ref()
            .expect("real-mpv loopback server must remain live")
            .recv_hello(step_timeout, "real-mpv vertical")?;
        if !hello.contains("\"Hello\"") {
            return Err(format!(
                "real-mpv loopback server did not receive an expected startup hello payload: {hello:?}"
            ));
        }
        let exchange = session_exchange
            .as_mut()
            .expect("real-mpv session exchange must be initialized");
        exchange.connected_peer_endpoint = Some(session_peer_endpoint.clone());
        exchange.peer_ipv4_loopback = Some(true);
        exchange.client_hello = Some(hello.trim_end().to_owned());
        write_json_file(&session_exchange_path, exchange)?;
        navigate_to_view_with_wait(
            &driver,
            launched_window,
            ROOM_SURFACE_AUTOMATION_ID,
            "view: room",
            step_timeout,
        )?;
        wait_for_main_window_user_row_name(
            &driver,
            launched_window,
            REAL_MPV_LOOPBACK_USERNAME,
            step_timeout,
        )?;
        state.advance(
            &state_path,
            "loopback-session-ready",
            Some("loopback-session-bound-to-local-gui"),
        )?;
        invoke_real_mpv_menu_action_with_evidence(
            &driver,
            launched_window,
            FILE_MENU_AUTOMATION_ID,
            OPEN_MEDIA_MENU_AUTOMATION_ID,
            step_timeout,
            &mut menu_interactions,
            &menu_interactions_path,
        )?;
        state.advance(
            &state_path,
            "open-media-invoked",
            Some("native-file-menu-open-media"),
        )?;

        wait_for_accessible_name(&driver, launched_window, "view: room", step_timeout)?;
        let (file_loaded_index, file_loaded) = wait_for_mpv_observation(
            &observation_path,
            0,
            step_timeout,
            "file-loaded for the generated local media",
            |observation| {
                observation.event == "file-loaded"
                    && observation.path.as_deref().is_some_and(|observed| {
                        observed_media_path_matches(Path::new(observed), &media_path)
                    })
            },
        )?;
        let mpv_pid = file_loaded.pid.ok_or_else(|| {
            "mpv file-loaded observation did not include its process ID".to_owned()
        })?;
        let expected_file_name = media_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "generated media file name was not valid UTF-8".to_owned())?;
        if file_loaded.filename.as_deref() != Some(expected_file_name) {
            return Err(format!(
                "real mpv reported filename {:?}; expected {expected_file_name:?}",
                file_loaded.filename
            ));
        }
        let observed_duration = file_loaded.duration.ok_or_else(|| {
            "mpv file-loaded observation did not include generated-media duration".to_owned()
        })?;
        if (observed_duration - f64::from(REAL_MPV_MEDIA_DURATION_SECONDS)).abs() > 0.05 {
            return Err(format!(
                "real mpv reported generated-media duration {observed_duration}; expected {}",
                REAL_MPV_MEDIA_DURATION_SECONDS
            ));
        }
        let parent_pid = process_parent_pid(mpv_pid)?;
        if parent_pid != gui_pid {
            return Err(format!(
                "real mpv PID {mpv_pid} was not owned by the launched GUI PID {gui_pid}; parent PID was {parent_pid}"
            ));
        }
        let process_image_path = process_image_path(mpv_pid)?;
        let process_identity = binary_identity(&process_image_path)?;
        if process_identity.sha256 != mpv_preflight.identity.sha256 {
            return Err(format!(
                "GUI-owned mpv process digest {} did not match preflight digest {}",
                process_identity.sha256, mpv_preflight.identity.sha256
            ));
        }
        verified_mpv_pid = Some(mpv_pid);
        state.mpv_pid = Some(mpv_pid);
        let ipc_endpoint = file_loaded
            .ipc_endpoint
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                "mpv file-loaded observation did not expose its managed IPC endpoint".to_owned()
            })?;
        let expected_ipc_prefix = format!(r"\\.\pipe\sorotte-gui-mpv-{gui_pid}-");
        if !ipc_endpoint.starts_with(&expected_ipc_prefix) {
            return Err(format!(
                "GUI-owned mpv IPC endpoint {ipc_endpoint:?} did not use the expected product-generated prefix {expected_ipc_prefix:?}"
            ));
        }
        state.advance(
            &state_path,
            "real-mpv-file-loaded",
            Some("gui-owned-exact-mpv-loaded-generated-media"),
        )?;

        wait_for_enabled_automation_id(
            &driver,
            launched_window,
            PLAY_CONTROL_AUTOMATION_ID,
            step_timeout,
        )?;
        wait_for_enabled_automation_id(
            &driver,
            launched_window,
            PAUSE_CONTROL_AUTOMATION_ID,
            step_timeout,
        )?;
        wait_for_accessible_name(&driver, launched_window, "Room state: paused", step_timeout)?;
        state.advance(
            &state_path,
            "gui-transport-ready",
            Some("gui-projected-real-mpv-transport-ready"),
        )?;

        let observations_before_play = read_mpv_observations(&observation_path)?.len();
        invoke_named_control_with_wait(
            &driver,
            launched_window,
            PLAY_CONTROL_AUTOMATION_ID,
            NativeControlKind::Button,
            step_timeout,
        )?;
        let (playing_index, _) = wait_for_mpv_observation(
            &observation_path,
            observations_before_play,
            step_timeout,
            "pause=false after the GUI Play action",
            |observation| observation.event == "pause" && observation.pause == Some(false),
        )?;
        state.advance(
            &state_path,
            "real-mpv-playing",
            Some("gui-play-command-observed-by-real-mpv"),
        )?;
        wait_for_accessible_name(
            &driver,
            launched_window,
            "Room state: playing",
            step_timeout,
        )?;
        state.advance(
            &state_path,
            "gui-playing-projected",
            Some("gui-projected-playing-after-real-mpv-observation"),
        )?;

        let observations_before_pause = read_mpv_observations(&observation_path)?.len();
        invoke_named_control_with_wait(
            &driver,
            launched_window,
            PAUSE_CONTROL_AUTOMATION_ID,
            NativeControlKind::Button,
            step_timeout,
        )?;
        let (paused_index, _) = wait_for_mpv_observation(
            &observation_path,
            observations_before_pause,
            step_timeout,
            "pause=true after the GUI Pause action",
            |observation| observation.event == "pause" && observation.pause == Some(true),
        )?;
        if !(file_loaded_index < playing_index && playing_index < paused_index) {
            return Err(format!(
                "mpv observation ordering was not file-loaded < playing < paused: {file_loaded_index}, {playing_index}, {paused_index}"
            ));
        }
        state.advance(
            &state_path,
            "real-mpv-paused",
            Some("gui-pause-command-observed-by-real-mpv"),
        )?;
        wait_for_accessible_name(&driver, launched_window, "Room state: paused", step_timeout)?;
        state.advance(
            &state_path,
            "gui-paused-projected",
            Some("gui-projected-paused-after-real-mpv-observation"),
        )?;

        driver
            .capture_window_png(launched_window, &success_screenshot_path)
            .map_err(|error| format!("failed to retain successful native screenshot: {error}"))?;
        state.advance(
            &state_path,
            "success-screenshot-retained",
            Some("native-success-screenshot"),
        )?;

        invoke_real_mpv_menu_action_with_evidence(
            &driver,
            launched_window,
            FILE_MENU_AUTOMATION_ID,
            EXIT_MENU_AUTOMATION_ID,
            step_timeout,
            &mut menu_interactions,
            &menu_interactions_path,
        )?;
        wait_for_process_exit(
            child
                .as_mut()
                .expect("launched GUI child must remain available"),
            step_timeout,
        )?;
        wait_for_lifecycle_events(
            &lifecycle_path,
            &[
                "exit-action-applied",
                "viewport-close-requested",
                "runtime-stop-requested",
                "runtime-worker-stopped",
                "app-drop-complete",
            ],
            step_timeout,
        )?;
        wait_for_process_termination(mpv_pid, step_timeout)?;
        let release_result = session_server
            .take()
            .expect("real-mpv loopback server must remain live until GUI exit")
            .release("real-mpv vertical");
        let exchange = session_exchange
            .as_mut()
            .expect("real-mpv session exchange must be initialized");
        match release_result {
            Ok(()) => {
                exchange.result = "released".to_owned();
                exchange.server_thread_released = true;
                exchange.socket_released = true;
                write_json_file(&session_exchange_path, exchange)?;
            }
            Err(error) => {
                exchange.result = "release-failed".to_owned();
                exchange.error = Some(redact_real_mpv_error(&error));
                write_json_file(&session_exchange_path, exchange)?;
                return Err(error);
            }
        }
        menu_interactions.result = "passed".to_owned();
        write_json_file(&menu_interactions_path, &menu_interactions)?;
        state.advance(&state_path, "complete", Some("gui-exit-reaped-owned-mpv"))?;
        state.result = "passed".to_owned();
        write_json_file(&state_path, &state)?;

        let artifacts = artifact_manifest(
            &artifact_root,
            &[
                ("config", &config_path),
                ("generated_media", &media_path),
                ("observation_script", &observation_script_path),
                ("mpv_observation", &observation_path),
                ("mpv_log", &mpv_log_path),
                ("gui_lifecycle", &lifecycle_path),
                ("session_exchange", &session_exchange_path),
                ("menu_interactions", &menu_interactions_path),
                ("success_screenshot", &success_screenshot_path),
                ("state", &state_path),
            ],
        )?;
        Ok(RealMpvVerticalReport {
            schema_version: REAL_MPV_SCHEMA_VERSION,
            kind: REAL_MPV_KIND,
            result: "passed",
            capability: "executed",
            gui: gui_identity,
            mpv: MpvIdentity {
                path: mpv_preflight.identity.path,
                bytes: mpv_preflight.identity.bytes,
                sha256: mpv_preflight.identity.sha256,
                version: mpv_preflight.version,
                minimum_supported_version: sorotte_player_mpv::MINIMUM_SUPPORTED_MPV_VERSION,
                pid: mpv_pid,
                parent_pid,
                process_image_path: process_image_path.display().to_string(),
            },
            isolation: IsolationContract {
                artifact_root: artifact_root.display().to_string(),
                config_path: config_path.display().to_string(),
                appdata_root: appdata_root.display().to_string(),
                media_path: media_path.display().to_string(),
                observation_script_path: observation_script_path.display().to_string(),
                observation_path: observation_path.display().to_string(),
                mpv_log_path: mpv_log_path.display().to_string(),
                lifecycle_path: lifecycle_path.display().to_string(),
                session_exchange_path: session_exchange_path.display().to_string(),
                menu_interactions_path: menu_interactions_path.display().to_string(),
                ipc_endpoint,
                session_endpoint,
                session_peer_endpoint,
                session_advertised_capabilities: REAL_MPV_SESSION_CAPABILITIES.to_vec(),
                network_mode: "os-assigned-ipv4-loopback-session",
                media_source: "generated-local-pcm-wav",
                mpv_config: "isolated --no-config",
            },
            assertions: state.assertions.clone(),
            artifacts,
            duration_ms: started_at.elapsed().as_millis(),
        })
    })();

    match run_result {
        Ok(report) => serde_json::to_string(&report)
            .map_err(|error| format!("failed to serialize real-mpv report: {error}")),
        Err(error) => {
            let mut error = error;
            if let (Some(gui_child), Some(gui_window)) = (child.as_mut(), window)
                && gui_child.try_wait().ok().flatten().is_none()
            {
                capture_native_failure_artifacts_at(
                    &driver,
                    gui_window,
                    &artifact_root,
                    "real-mpv-vertical",
                    &error,
                );
                let _ = driver.close_window(gui_window);
                if wait_for_process_exit(gui_child, Duration::from_secs(3)).is_err() {
                    let _ = gui_child.kill();
                    let _ = gui_child.wait();
                }
            }
            if let Some(mpv_pid) = verified_mpv_pid
                && process_is_running(mpv_pid)
            {
                let _ = terminate_test_process(mpv_pid);
            }
            if let Some(server) = session_server.take() {
                if let Some(exchange) = session_exchange.as_mut() {
                    if exchange.connected_peer_endpoint.is_none()
                        && let Ok(peer) = server
                            .recv_peer(Duration::from_millis(100), "real-mpv vertical cleanup")
                    {
                        exchange.connected_peer_endpoint = Some(peer);
                        exchange.peer_ipv4_loopback = Some(true);
                    }
                    if exchange.client_hello.is_none()
                        && let Ok(hello) = server
                            .recv_hello(Duration::from_millis(100), "real-mpv vertical cleanup")
                    {
                        exchange.client_hello = Some(hello.trim_end().to_owned());
                    }
                }
                match server.release("real-mpv vertical") {
                    Ok(()) => {
                        if let Some(exchange) = session_exchange.as_mut() {
                            exchange.server_thread_released = true;
                            exchange.socket_released = true;
                        }
                    }
                    Err(release_error) => {
                        error = format!("{error}; {release_error}");
                    }
                }
            }
            if let Some(exchange) = session_exchange.as_mut() {
                exchange.result = "failed".to_owned();
                exchange.error = Some(redact_real_mpv_error(&error));
                let _ = write_json_file(&session_exchange_path, exchange);
            }
            menu_interactions.result = "failed".to_owned();
            menu_interactions.error = Some(redact_real_mpv_error(&error));
            let _ = write_json_file(&menu_interactions_path, &menu_interactions);
            state.result = "failed".to_owned();
            state.stage = format!("{}-failed", state.stage);
            state.error = Some(redact_real_mpv_error(&error));
            let _ = write_json_file(&state_path, &state);
            Err(error)
        }
    }
}

fn parse_real_mpv_vertical_options(args: &[String]) -> Result<RealMpvVerticalOptions, String> {
    let mut binary_path = None;
    let mut mpv_path = None;
    let mut artifact_dir = None;
    let mut timeout = Duration::from_secs(30);
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--real-mpv-vertical" | "--json" => index += 1,
            "--binary" => {
                binary_path = Some(PathBuf::from(required_value(args, index, "--binary")?));
                index += 2;
            }
            "--mpv" => {
                mpv_path = Some(PathBuf::from(required_value(args, index, "--mpv")?));
                index += 2;
            }
            "--artifact-dir" => {
                artifact_dir = Some(PathBuf::from(required_value(
                    args,
                    index,
                    "--artifact-dir",
                )?));
                index += 2;
            }
            "--timeout-ms" => {
                timeout = parse_timeout_ms(required_value(args, index, "--timeout-ms")?)?;
                index += 2;
            }
            argument => {
                return Err(format!(
                    "unknown real-mpv vertical argument {argument:?}; expected --binary, --mpv, --artifact-dir, and optional --timeout-ms"
                ));
            }
        }
    }
    Ok(RealMpvVerticalOptions {
        binary_path: binary_path
            .ok_or_else(|| "--real-mpv-vertical requires --binary PATH".to_owned())?,
        mpv_path: mpv_path.ok_or_else(|| "--real-mpv-vertical requires --mpv PATH".to_owned())?,
        artifact_dir: artifact_dir
            .ok_or_else(|| "--real-mpv-vertical requires --artifact-dir PATH".to_owned())?,
        timeout,
    })
}

fn required_value<'a>(args: &'a [String], index: usize, option: &str) -> Result<&'a str, String> {
    args.get(index + 1)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("{option} requires a non-empty value"))
}

fn preflight_supported_mpv(path: &Path) -> Result<MpvPreflight, String> {
    if !path.is_file() {
        return Err(format!(
            "required real mpv binary does not exist: {}",
            path.display()
        ));
    }
    let output = Command::new(path)
        .arg("--no-config")
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("failed to query mpv version at {}: {error}", path.display()))?;
    if !output.status.success() {
        return Err(format!(
            "mpv version query failed at {} with status {}",
            path.display(),
            output.status
        ));
    }
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let version = combined
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("mpv v"))
        .ok_or_else(|| {
            format!(
                "mpv version query at {} did not emit an 'mpv v...' identity",
                path.display()
            )
        })?
        .to_owned();
    let observed = parse_mpv_version_core(&version)?;
    let minimum = parse_mpv_version_core(&format!(
        "mpv v{}",
        sorotte_player_mpv::MINIMUM_SUPPORTED_MPV_VERSION
    ))?;
    if observed < minimum {
        return Err(format!(
            "mpv {observed:?} is below Sorotte's supported minimum {minimum:?}"
        ));
    }
    Ok(MpvPreflight {
        identity: binary_identity(path)?,
        version,
    })
}

fn parse_mpv_version_core(value: &str) -> Result<(u64, u64, u64), String> {
    let version = value
        .split_whitespace()
        .find_map(|token| token.strip_prefix('v'))
        .ok_or_else(|| format!("could not find an mpv vVERSION token in {value:?}"))?;
    let numeric = version
        .chars()
        .take_while(|character| character.is_ascii_digit() || *character == '.')
        .collect::<String>();
    let mut components = numeric.split('.');
    let major = components
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| format!("invalid mpv major version in {value:?}"))?;
    let minor = components
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| format!("invalid mpv minor version in {value:?}"))?;
    let patch = components
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| format!("invalid mpv patch version in {value:?}"))?;
    Ok((major, minor, patch))
}

fn seed_real_mpv_config(
    config_path: &Path,
    mpv_path: &Path,
    observation_script_path: &Path,
    mpv_log_path: &Path,
) -> Result<(), String> {
    let player_path = mpv_path.display().to_string();
    let extra_args = vec![
        "--no-config".to_owned(),
        "--force-window=no".to_owned(),
        "--video=no".to_owned(),
        "--audio-display=no".to_owned(),
        "--ao=null".to_owned(),
        format!("--script={}", observation_script_path.display()),
        format!("--log-file={}", mpv_log_path.display()),
        "--msg-level=all=v".to_owned(),
    ];
    let settings = StoredClientSettingsMvp {
        player_path: Some(player_path.clone()),
        per_player_arguments: Some(BTreeMap::from([(player_path, extra_args)])),
        shared_playlist_enabled: Some(false),
        show_osd: Some(false),
        chat_input_enabled: Some(false),
        chat_output_enabled: Some(false),
        check_for_updates_automatically: Some(false),
        ..StoredClientSettingsMvp::default()
    };
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(config_path, &settings).map_err(|error| {
        format!(
            "failed to write isolated real-mpv config {}: {error}",
            config_path.display()
        )
    })
}

fn real_mpv_observation_lua(observation_path: &Path) -> String {
    let output_path = lua_long_string(&observation_path.display().to_string());
    format!(
        r#"local utils = require "mp.utils"
local output_path = {output_path}

local function emit(event)
    local record = {{
        event = event,
        pid = utils.getpid(),
        path = mp.get_property_native("path"),
        filename = mp.get_property_native("filename"),
        duration = mp.get_property_native("duration"),
        pause = mp.get_property_native("pause"),
        ipc_endpoint = mp.get_property_native("input-ipc-server"),
    }}
    local handle, open_error = io.open(output_path, "a")
    if handle == nil then
        mp.msg.error("sorotte real-mpv observation open failed: " .. tostring(open_error))
        return
    end
    handle:write(utils.format_json(record), "\n")
    handle:flush()
    handle:close()
end

mp.register_event("file-loaded", function() emit("file-loaded") end)
mp.observe_property("pause", "bool", function(_, value)
    if value ~= nil then
        emit("pause")
    end
end)
"#
    )
}

fn lua_long_string(value: &str) -> String {
    for equals_count in 0..=16 {
        let equals = "=".repeat(equals_count);
        let closing = format!("]{equals}]");
        if !value.contains(&closing) {
            return format!("[{equals}[{value}]{equals}]");
        }
    }
    panic!("path could not be represented as a bounded Lua long string");
}

fn pcm_wav_bytes(duration_seconds: u32) -> Vec<u8> {
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

fn read_mpv_observations(path: &Path) -> Result<Vec<MpvObservation>, String> {
    let contents = fs::read_to_string(path).map_err(|error| {
        format!(
            "failed to read real-mpv observation {}: {error}",
            path.display()
        )
    })?;
    contents
        .lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
        .map(|(index, line)| {
            serde_json::from_str(line).map_err(|error| {
                format!(
                    "real-mpv observation {} line {} was invalid JSON: {error}",
                    path.display(),
                    index + 1
                )
            })
        })
        .collect()
}

fn wait_for_mpv_observation<F>(
    path: &Path,
    start_index: usize,
    timeout: Duration,
    description: &str,
    mut predicate: F,
) -> Result<(usize, MpvObservation), String>
where
    F: FnMut(&MpvObservation) -> bool,
{
    let deadline = Instant::now() + timeout;
    loop {
        let observations = read_mpv_observations(path)?;
        if let Some((offset, observation)) = observations
            .iter()
            .enumerate()
            .skip(start_index)
            .find(|(_, observation)| predicate(observation))
        {
            return Ok((offset, observation.clone()));
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for {description}; observations={:?}",
                observations
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn observed_media_path_matches(observed: &Path, expected: &Path) -> bool {
    let observed = fs::canonicalize(observed).unwrap_or_else(|_| observed.to_path_buf());
    let expected = fs::canonicalize(expected).unwrap_or_else(|_| expected.to_path_buf());
    if cfg!(windows) {
        observed
            .to_string_lossy()
            .eq_ignore_ascii_case(&expected.to_string_lossy())
    } else {
        observed == expected
    }
}

fn require_ipv4_loopback_endpoint(value: &str, label: &str) -> Result<(), String> {
    let address = value
        .parse::<std::net::SocketAddr>()
        .map_err(|error| format!("{label} {value:?} was not a socket endpoint: {error}"))?;
    if !address.is_ipv4() || !address.ip().is_loopback() || address.port() == 0 {
        return Err(format!(
            "{label} {address} was not a nonzero IPv4 loopback endpoint"
        ));
    }
    Ok(())
}

fn menu_action_snapshot<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    action_automation_id: &str,
) -> Result<MenuSectionSnapshot, String> {
    let matching = driver
        .accessibility_nodes(window)?
        .into_iter()
        .filter(|node| node.automation_id == action_automation_id)
        .collect::<Vec<_>>();
    let visible_nodes = matching
        .iter()
        .filter(|node| !node.offscreen && node.bounds.is_some())
        .count();
    let visible_enabled_nodes = matching
        .iter()
        .filter(|node| node.enabled && !node.offscreen && node.bounds.is_some())
        .count();
    let nodes = matching
        .iter()
        .map(|node| {
            format!(
                "name={:?}, automation_id={:?}, enabled={}, offscreen={}, bounds={:?}",
                node.name, node.automation_id, node.enabled, node.offscreen, node.bounds
            )
        })
        .collect();
    Ok(MenuSectionSnapshot {
        matching_nodes: matching.len(),
        visible_nodes,
        visible_enabled_nodes,
        nodes,
    })
}

fn record_menu_interaction_error(
    evidence: &mut MenuInteractionsEvidence,
    index: usize,
    evidence_path: &Path,
    error: String,
) -> Result<(), String> {
    evidence.interactions[index].error = Some(redact_real_mpv_error(&error));
    write_json_file(evidence_path, evidence).map_err(|write_error| {
        format!("{error}; additionally failed to retain menu evidence: {write_error}")
    })?;
    Err(error)
}

fn invoke_real_mpv_menu_action_with_evidence<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    section_automation_id: &str,
    action_automation_id: &str,
    timeout: Duration,
    evidence: &mut MenuInteractionsEvidence,
    evidence_path: &Path,
) -> Result<(), String> {
    let index = evidence.interactions.len();
    evidence.interactions.push(MenuInteractionRecord {
        section_automation_id: section_automation_id.to_owned(),
        action_automation_id: action_automation_id.to_owned(),
        section_open_strategy: "physical-section-open-pending".to_owned(),
        pre_fallback_snapshots: Vec::new(),
        opened_snapshot: None,
        leaf_delivery: "single-exact-physical-click-no-retry",
        leaf_delivered: false,
        error: None,
    });
    write_json_file(evidence_path, evidence)?;

    let physical_result = invoke_menu_action_by_id_with_wait(
        driver,
        window,
        section_automation_id,
        action_automation_id,
        timeout,
    );
    match physical_result {
        Ok(()) => {
            let record = &mut evidence.interactions[index];
            record.section_open_strategy = "physical-section-open".to_owned();
            record.leaf_delivered = true;
            write_json_file(evidence_path, evidence)?;
            return Ok(());
        }
        Err(error)
            if !error.starts_with("timed out waiting for one physical click on menu section") =>
        {
            return record_menu_interaction_error(evidence, index, evidence_path, error);
        }
        Err(_) => {}
    }

    for snapshot_index in 0..2 {
        let snapshot = match menu_action_snapshot(driver, window, action_automation_id) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return record_menu_interaction_error(
                    evidence,
                    index,
                    evidence_path,
                    format!(
                        "failed pre-fallback menu snapshot {} for {action_automation_id:?}: {error}",
                        snapshot_index + 1
                    ),
                );
            }
        };
        let visible_nodes = snapshot.visible_nodes;
        evidence.interactions[index]
            .pre_fallback_snapshots
            .push(snapshot);
        write_json_file(evidence_path, evidence)?;
        if visible_nodes != 0 {
            return record_menu_interaction_error(
                evidence,
                index,
                evidence_path,
                format!(
                    "physical menu-section acknowledgement timed out, but {action_automation_id:?} became visible before fallback; refusing a second section delivery"
                ),
            );
        }
        if snapshot_index == 0 {
            thread::sleep(Duration::from_millis(100));
        }
    }

    evidence.interactions[index].section_open_strategy =
        "uia-section-open-after-two-hidden-snapshots".to_owned();
    write_json_file(evidence_path, evidence)?;
    if let Err(error) =
        driver.invoke_named_control(window, section_automation_id, NativeControlKind::Any)
    {
        return record_menu_interaction_error(
            evidence,
            index,
            evidence_path,
            format!(
                "failed the bounded UIA section-open fallback for {section_automation_id:?}: {error}"
            ),
        );
    }

    let deadline = Instant::now() + timeout;
    loop {
        let snapshot = match menu_action_snapshot(driver, window, action_automation_id) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return record_menu_interaction_error(
                    evidence,
                    index,
                    evidence_path,
                    format!(
                        "failed to inspect {action_automation_id:?} after UIA section open: {error}"
                    ),
                );
            }
        };
        if snapshot.visible_enabled_nodes == 1 {
            evidence.interactions[index].opened_snapshot = Some(snapshot);
            write_json_file(evidence_path, evidence)?;
            break;
        }
        if snapshot.visible_nodes > 1 || Instant::now() >= deadline {
            evidence.interactions[index].opened_snapshot = Some(snapshot);
            write_json_file(evidence_path, evidence)?;
            return record_menu_interaction_error(
                evidence,
                index,
                evidence_path,
                format!(
                    "UIA section-open fallback did not expose exactly one enabled {action_automation_id:?}"
                ),
            );
        }
        thread::sleep(Duration::from_millis(50));
    }

    match driver.click_named_control(window, action_automation_id, NativeControlKind::Any) {
        Ok(()) => {
            evidence.interactions[index].leaf_delivered = true;
            write_json_file(evidence_path, evidence)?;
            Ok(())
        }
        Err(error) => record_menu_interaction_error(
            evidence,
            index,
            evidence_path,
            format!(
                "menu leaf {action_automation_id:?} was exposed by the bounded section fallback, but its single exact physical click failed: {error}"
            ),
        ),
    }
}

fn wait_for_enabled_automation_id<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    automation_id: &str,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let nodes = driver.accessibility_nodes(window)?;
        if nodes
            .iter()
            .any(|node| node.automation_id == automation_id && node.enabled)
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for enabled native control {automation_id:?}"
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn binary_identity(path: &Path) -> Result<BinaryIdentity, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("failed to read binary identity {}: {error}", path.display()))?;
    Ok(BinaryIdentity {
        path: path.display().to_string(),
        bytes: bytes.len() as u64,
        sha256: hex_sha256(&bytes),
    })
}

fn artifact_manifest(
    artifact_root: &Path,
    files: &[(&str, &Path)],
) -> Result<BTreeMap<String, ArtifactIdentity>, String> {
    let mut manifest = BTreeMap::new();
    for (label, path) in files {
        if !path.starts_with(artifact_root) {
            return Err(format!(
                "retained artifact {} escaped the isolated root {}",
                path.display(),
                artifact_root.display()
            ));
        }
        let bytes = fs::read(path)
            .map_err(|error| format!("failed to hash artifact {}: {error}", path.display()))?;
        let relative_path = path
            .strip_prefix(artifact_root)
            .expect("artifact prefix was checked")
            .display()
            .to_string();
        manifest.insert(
            (*label).to_owned(),
            ArtifactIdentity {
                path: relative_path,
                bytes: bytes.len() as u64,
                sha256: hex_sha256(&bytes),
            },
        );
    }
    Ok(manifest)
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut rendered = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(rendered, "{byte:02x}").expect("writing to a String cannot fail");
    }
    rendered
}

fn write_json_file(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let mut json = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("failed to serialize {}: {error}", path.display()))?;
    json.push(b'\n');
    fs::write(path, json).map_err(|error| format!("failed to write {}: {error}", path.display()))
}

fn redact_real_mpv_error(error: &str) -> String {
    if sorotte_secret::text_may_contain_credentials(error) {
        sorotte_secret::REDACTED_SECRET.to_owned()
    } else {
        error.to_owned()
    }
}

#[cfg(target_os = "windows")]
fn process_parent_pid(pid: u32) -> Result<u32, String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
    };

    // SAFETY: The returned snapshot handle is checked and closed exactly once. PROCESSENTRY32W
    // owns its fixed buffers, and the ToolHelp calls receive a correctly sized mutable record.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(format!(
            "failed to snapshot processes while attesting mpv PID {pid}: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: PROCESSENTRY32W is a plain Windows API record that permits zero initialization;
    // dwSize is populated before the record is passed to ToolHelp.
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut found = None;
    // SAFETY: snapshot and entry remain valid for the complete bounded iteration.
    let mut has_entry = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while has_entry {
        if entry.th32ProcessID == pid {
            found = Some(entry.th32ParentProcessID);
            break;
        }
        // SAFETY: snapshot and entry remain valid until CloseHandle below.
        has_entry = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    // SAFETY: snapshot is a live ToolHelp handle and has not previously been closed.
    unsafe {
        CloseHandle(snapshot);
    }
    found.ok_or_else(|| format!("GUI-owned mpv PID {pid} was absent from the process snapshot"))
}

#[cfg(not(target_os = "windows"))]
fn process_parent_pid(_pid: u32) -> Result<u32, String> {
    Err("real-mpv parent process attestation requires Windows".to_owned())
}

#[cfg(target_os = "windows")]
fn process_image_path(pid: u32) -> Result<PathBuf, String> {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
        },
    };

    // SAFETY: OpenProcess requests query access only. The returned handle is checked, used by one
    // bounded image-path query, and closed exactly once.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return Err(format!(
            "failed to open GUI-owned mpv PID {pid} for image attestation: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut buffer = vec![0_u16; 32_768];
    let mut length = buffer.len() as u32;
    // SAFETY: buffer is writable for `length` UTF-16 units, and `length` remains live for the
    // duration of the call.
    let queried =
        unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut length) };
    // SAFETY: handle is a live process handle and has not previously been closed.
    unsafe {
        CloseHandle(handle);
    }
    if queried == 0 {
        return Err(format!(
            "failed to query GUI-owned mpv PID {pid} image path: {}",
            std::io::Error::last_os_error()
        ));
    }
    buffer.truncate(length as usize);
    Ok(PathBuf::from(String::from_utf16_lossy(&buffer)))
}

#[cfg(not(target_os = "windows"))]
fn process_image_path(_pid: u32) -> Result<PathBuf, String> {
    Err("real-mpv image attestation requires Windows".to_owned())
}

#[cfg(target_os = "windows")]
fn process_is_running(pid: u32) -> bool {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, WAIT_TIMEOUT},
        System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            WaitForSingleObject,
        },
    };

    // SAFETY: The query/synchronize handle is checked, polled without mutation, and closed once.
    let handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            pid,
        )
    };
    if handle.is_null() {
        return false;
    }
    // SAFETY: handle is live until the matching CloseHandle below.
    let running = unsafe { WaitForSingleObject(handle, 0) == WAIT_TIMEOUT };
    // SAFETY: handle has not previously been closed.
    unsafe {
        CloseHandle(handle);
    }
    running
}

#[cfg(not(target_os = "windows"))]
fn process_is_running(_pid: u32) -> bool {
    false
}

fn wait_for_process_termination(pid: u32, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while process_is_running(pid) {
        if Instant::now() >= deadline {
            return Err(format!(
                "GUI-owned real mpv PID {pid} remained alive after the GUI exited"
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn terminate_test_process(pid: u32) -> Result<(), String> {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{
            OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess,
            WaitForSingleObject,
        },
    };

    // SAFETY: The PID was attested as the exact GUI-owned mpv process. The returned handle is
    // checked, used only to terminate/wait that process, and closed exactly once.
    let handle = unsafe { OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        return Ok(());
    }
    // SAFETY: handle grants terminate access to the exact test-owned process.
    let terminated = unsafe { TerminateProcess(handle, 1) };
    if terminated != 0 {
        // SAFETY: handle remains live for the bounded wait.
        unsafe {
            WaitForSingleObject(handle, 5_000);
        }
    }
    // SAFETY: handle has not previously been closed.
    unsafe {
        CloseHandle(handle);
    }
    if terminated == 0 {
        Err(format!(
            "failed to terminate test-owned mpv PID {pid}: {}",
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
fn terminate_test_process(_pid: u32) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_mpv_options_require_explicit_paths_and_positive_timeout() {
        let args = [
            "--real-mpv-vertical",
            "--binary",
            "gui.exe",
            "--mpv",
            "mpv.exe",
            "--artifact-dir",
            "artifacts",
            "--timeout-ms",
            "1234",
            "--json",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
        let parsed = parse_real_mpv_vertical_options(&args).expect("options should parse");
        assert_eq!(parsed.binary_path, PathBuf::from("gui.exe"));
        assert_eq!(parsed.mpv_path, PathBuf::from("mpv.exe"));
        assert_eq!(parsed.artifact_dir, PathBuf::from("artifacts"));
        assert_eq!(parsed.timeout, Duration::from_millis(1234));

        assert!(parse_real_mpv_vertical_options(&["--real-mpv-vertical".to_owned()]).is_err());
        let mut zero = args;
        let timeout_index = zero
            .iter()
            .position(|value| value == "1234")
            .expect("timeout argument");
        zero[timeout_index] = "0".to_owned();
        assert!(parse_real_mpv_vertical_options(&zero).is_err());
    }

    #[test]
    fn generated_wav_has_exact_pcm_header_and_duration() {
        let wav = pcm_wav_bytes(3);
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(wav.len(), 44 + (48_000 * 3 * 2));
        assert_eq!(
            u32::from_le_bytes(wav[40..44].try_into().expect("data length")),
            48_000 * 3 * 2
        );
    }

    #[test]
    fn lua_observer_uses_safe_long_string_and_required_real_state() {
        let path = Path::new(r"C:\isolated\contains]]\observation.jsonl");
        let script = real_mpv_observation_lua(path);
        assert!(script.contains(r"C:\isolated\contains]]\observation.jsonl"));
        assert!(script.contains("utils.getpid()"));
        assert!(script.contains(r#"mp.register_event("file-loaded""#));
        assert!(script.contains(r#"mp.observe_property("pause""#));
        assert!(script.contains(r#"mp.get_property_native("input-ipc-server")"#));
    }

    #[test]
    fn mpv_version_parser_accepts_supported_snapshot_suffixes() {
        assert_eq!(
            parse_mpv_version_core("mpv v0.41.0-877-ge5486b96d Copyright"),
            Ok((0, 41, 0))
        );
        assert_eq!(
            parse_mpv_version_core("mpv v1.2.3 Copyright"),
            Ok((1, 2, 3))
        );
        assert!(parse_mpv_version_core("not-mpv").is_err());
    }

    #[test]
    fn observation_reader_rejects_malformed_evidence_and_preserves_order() {
        let root = std::env::temp_dir().join(format!(
            "sorotte-real-mpv-observation-unit-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        fs::create_dir_all(&root).expect("test root");
        let path = root.join("observations.jsonl");
        fs::write(
            &path,
            concat!(
                "{\"event\":\"file-loaded\",\"pid\":7,\"path\":\"x.wav\",\"pause\":true}\n",
                "{\"event\":\"pause\",\"pid\":7,\"pause\":false}\n",
                "{\"event\":\"pause\",\"pid\":7,\"pause\":true}\n"
            ),
        )
        .expect("observations");
        let observations = read_mpv_observations(&path).expect("valid observations");
        assert_eq!(observations.len(), 3);
        assert_eq!(observations[0].event, "file-loaded");
        assert_eq!(observations[1].pause, Some(false));
        assert_eq!(observations[2].pause, Some(true));

        fs::write(&path, "{invalid}\n").expect("malformed observations");
        assert!(read_mpv_observations(&path).is_err());
        fs::remove_dir_all(root).expect("remove test root");
    }

    #[test]
    fn real_mpv_session_endpoints_are_strict_ipv4_loopback_only() {
        assert!(require_ipv4_loopback_endpoint("127.0.0.1:49152", "fixture").is_ok());
        assert!(require_ipv4_loopback_endpoint("[::1]:49152", "fixture").is_err());
        assert!(require_ipv4_loopback_endpoint("192.0.2.1:49152", "fixture").is_err());
        assert!(require_ipv4_loopback_endpoint("127.0.0.1:0", "fixture").is_err());
        assert!(require_ipv4_loopback_endpoint("not-an-endpoint", "fixture").is_err());
    }
}
