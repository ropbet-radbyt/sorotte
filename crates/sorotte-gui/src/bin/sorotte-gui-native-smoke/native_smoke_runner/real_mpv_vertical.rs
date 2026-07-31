use std::{
    collections::BTreeMap,
    fmt::Write as _,
    io::Read as _,
    net::{Shutdown, SocketAddr, TcpStream},
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

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
const REAL_MPV_RECOVERY_KIND: &str = "sorotte-gui-real-mpv-owned-process-recovery";
const REAL_MPV_HTTP_FAULT_KIND: &str = "sorotte-gui-real-mpv-faulting-http-recovery";
const REAL_MPV_HTTP_FAULT_ROUTE: &str = "/generated-fault.au";
const REAL_MPV_HTTP_FAULT_DURATION_SECONDS: u32 = 45;
const REAL_MPV_HTTP_FAULT_DISCONNECT_AFTER_BYTES: usize = 720_000;
const REAL_MPV_HTTP_FAULT_BYTES_PER_SECOND: usize = 350_000;
const PLAY_CONTROL_AUTOMATION_ID: &str = "main-window:control:play";
const PAUSE_CONTROL_AUTOMATION_ID: &str = "main-window:control:pause";

#[derive(Debug, Clone, PartialEq, Eq)]
struct RealMpvVerticalOptions {
    binary_path: PathBuf,
    mpv_path: PathBuf,
    artifact_dir: PathBuf,
    timeout: Duration,
    exercise_recovery: bool,
    exercise_http_fault: bool,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    recovered_mpv_pid: Option<u32>,
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
            recovered_mpv_pid: None,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    media_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    http_endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    http_evidence_path: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    recovered_mpv: Option<MpvIdentity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recovery: Option<MpvRecoveryEvidence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    http_fault: Option<HttpFaultRecoveryEvidence>,
    isolation: IsolationContract,
    assertions: Vec<String>,
    artifacts: BTreeMap<String, ArtifactIdentity>,
    duration_ms: u128,
}

#[derive(Debug, Clone, Serialize)]
struct HttpRequestEvidence {
    ordinal: usize,
    method: String,
    path: String,
    peer_endpoint: String,
    peer_ipv4_loopback: bool,
    range_header: Option<String>,
    status_code: u16,
    advertised_body_bytes: usize,
    transmitted_body_bytes: usize,
    disconnected_early: bool,
    write_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct HttpFaultRecoveryEvidence {
    schema_version: u32,
    kind: &'static str,
    result: String,
    fault: &'static str,
    recovery_mode: &'static str,
    listener_endpoint: String,
    listener_ipv4_loopback: bool,
    media_url: String,
    route: &'static str,
    generated_media_bytes: usize,
    generated_media_sha256: String,
    duration_seconds: u32,
    disconnect_after_body_bytes: usize,
    request_count: usize,
    premature_disconnect_count: usize,
    complete_response_count: usize,
    requests: Vec<HttpRequestEvidence>,
    initial_file_loaded_index: Option<usize>,
    pre_fault_progress_index: Option<usize>,
    end_file_index: Option<usize>,
    recovered_file_loaded_index: Option<usize>,
    recovered_progress_index: Option<usize>,
    recovered_paused_index: Option<usize>,
    initial_pid: Option<u32>,
    recovered_pid: Option<u32>,
    parent_pid: Option<u32>,
    process_image_path: Option<String>,
    process_sha256: Option<String>,
    initial_ipc_endpoint: Option<String>,
    recovered_ipc_endpoint: Option<String>,
    stable_process_identity: bool,
    stable_ipc_endpoint: bool,
    stable_media_url: bool,
    stable_duration: bool,
    pre_fault_position_seconds: Option<f64>,
    recovered_position_seconds: Option<f64>,
    manual_retry_invoked: bool,
    foreign_pid_observations_after_fault: usize,
    evidence_retained_before_cleanup: bool,
    server_thread_released: bool,
    socket_released: bool,
    owned_mpv_terminated_after_gui_exit: bool,
    error: Option<String>,
}

impl HttpFaultRecoveryEvidence {
    fn new(listener_endpoint: String, media_url: String, generated_media: &[u8]) -> Self {
        Self {
            schema_version: REAL_MPV_SCHEMA_VERSION,
            kind: REAL_MPV_HTTP_FAULT_KIND,
            result: "running".to_owned(),
            fault: "first-response-content-length-is-shorter-than-declared-au-media-once",
            recovery_mode: "same-generation-automatic-network-stream-reload",
            listener_endpoint,
            listener_ipv4_loopback: true,
            media_url,
            route: REAL_MPV_HTTP_FAULT_ROUTE,
            generated_media_bytes: generated_media.len(),
            generated_media_sha256: hex_sha256(generated_media),
            duration_seconds: REAL_MPV_HTTP_FAULT_DURATION_SECONDS,
            disconnect_after_body_bytes: REAL_MPV_HTTP_FAULT_DISCONNECT_AFTER_BYTES,
            request_count: 0,
            premature_disconnect_count: 0,
            complete_response_count: 0,
            requests: Vec::new(),
            initial_file_loaded_index: None,
            pre_fault_progress_index: None,
            end_file_index: None,
            recovered_file_loaded_index: None,
            recovered_progress_index: None,
            recovered_paused_index: None,
            initial_pid: None,
            recovered_pid: None,
            parent_pid: None,
            process_image_path: None,
            process_sha256: None,
            initial_ipc_endpoint: None,
            recovered_ipc_endpoint: None,
            stable_process_identity: false,
            stable_ipc_endpoint: false,
            stable_media_url: false,
            stable_duration: false,
            pre_fault_position_seconds: None,
            recovered_position_seconds: None,
            manual_retry_invoked: false,
            foreign_pid_observations_after_fault: 0,
            evidence_retained_before_cleanup: false,
            server_thread_released: false,
            socket_released: false,
            owned_mpv_terminated_after_gui_exit: false,
            error: None,
        }
    }

    fn record_requests(&mut self, requests: Vec<HttpRequestEvidence>) {
        self.request_count = requests.len();
        self.premature_disconnect_count = requests
            .iter()
            .filter(|request| request.disconnected_early)
            .count();
        self.complete_response_count = requests
            .iter()
            .filter(|request| {
                !request.disconnected_early
                    && request.transmitted_body_bytes == request.advertised_body_bytes
            })
            .count();
        self.requests = requests;
    }
}

struct FaultingLoopbackHttpServer {
    address: SocketAddr,
    shutdown: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<HttpRequestEvidence>>>,
    join_handle: Option<thread::JoinHandle<Result<(), String>>>,
}

impl FaultingLoopbackHttpServer {
    fn start(generated_media: Vec<u8>) -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|error| format!("failed to bind faulting HTTP loopback listener: {error}"))?;
        listener.set_nonblocking(true).map_err(|error| {
            format!("failed to make faulting HTTP listener nonblocking: {error}")
        })?;
        let address = listener
            .local_addr()
            .map_err(|error| format!("failed to inspect faulting HTTP listener: {error}"))?;
        if !address.is_ipv4() || !address.ip().is_loopback() || address.port() == 0 {
            return Err(format!(
                "faulting HTTP listener {address} was not strict nonzero IPv4 loopback"
            ));
        }

        let shutdown = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let thread_shutdown = Arc::clone(&shutdown);
        let thread_requests = Arc::clone(&requests);
        let fault_injected = Arc::new(AtomicBool::new(false));
        let thread_fault_injected = Arc::clone(&fault_injected);
        let join_handle = thread::Builder::new()
            .name("sorotte-native-fault-http".to_owned())
            .spawn(move || {
                while !thread_shutdown.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, peer)) => {
                            if thread_shutdown.load(Ordering::Acquire) {
                                break;
                            }
                            let ordinal = thread_requests
                                .lock()
                                .map_err(|_| "faulting HTTP request log was poisoned".to_owned())?
                                .len()
                                .saturating_add(1);
                            let record = handle_faulting_http_connection(
                                stream,
                                peer,
                                ordinal,
                                &generated_media,
                                &thread_shutdown,
                                &thread_fault_injected,
                            )?;
                            thread_requests
                                .lock()
                                .map_err(|_| "faulting HTTP request log was poisoned".to_owned())?
                                .push(record);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                        }
                        Err(error) => {
                            return Err(format!("faulting HTTP listener accept failed: {error}"));
                        }
                    }
                }
                Ok(())
            })
            .map_err(|error| format!("failed to spawn faulting HTTP server thread: {error}"))?;
        Ok(Self {
            address,
            shutdown,
            requests,
            join_handle: Some(join_handle),
        })
    }

    fn endpoint(&self) -> String {
        self.address.to_string()
    }

    fn url(&self) -> String {
        format!("http://{}{}", self.address, REAL_MPV_HTTP_FAULT_ROUTE)
    }

    fn requests(&self) -> Result<Vec<HttpRequestEvidence>, String> {
        self.requests
            .lock()
            .map(|requests| requests.clone())
            .map_err(|_| "faulting HTTP request log was poisoned".to_owned())
    }

    fn wait_for_media_gets(
        &self,
        expected: usize,
        timeout: Duration,
    ) -> Result<Vec<HttpRequestEvidence>, String> {
        let deadline = Instant::now() + timeout;
        loop {
            let requests = self.requests()?;
            if requests
                .iter()
                .filter(|request| request.method == "GET")
                .count()
                >= expected
            {
                return Ok(requests);
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "timed out waiting for {expected} faulting HTTP media GETs; requests={requests:?}"
                ));
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn release(mut self) -> Result<Vec<HttpRequestEvidence>, String> {
        self.shutdown.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.address);
        if let Some(join_handle) = self.join_handle.take() {
            join_handle
                .join()
                .map_err(|_| "faulting HTTP server thread panicked".to_owned())??;
        }
        let requests = self.requests()?;
        let rebound = TcpListener::bind(self.address).map_err(|error| {
            format!(
                "faulting HTTP listener endpoint {} could not be rebound after release: {error}",
                self.address
            )
        })?;
        drop(rebound);
        Ok(requests)
    }
}

impl Drop for FaultingLoopbackHttpServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.address);
        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
    }
}

fn handle_faulting_http_connection(
    mut stream: TcpStream,
    peer: SocketAddr,
    ordinal: usize,
    generated_media: &[u8],
    shutdown: &AtomicBool,
    fault_injected: &AtomicBool,
) -> Result<HttpRequestEvidence, String> {
    if !peer.is_ipv4() || !peer.ip().is_loopback() || peer.port() == 0 {
        return Err(format!(
            "faulting HTTP peer {peer} was not strict nonzero IPv4 loopback"
        ));
    }
    stream
        .set_nonblocking(false)
        .map_err(|error| format!("failed making faulting HTTP connection blocking: {error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|error| format!("failed setting faulting HTTP read timeout: {error}"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|error| format!("failed setting faulting HTTP write timeout: {error}"))?;

    let mut request_bytes = Vec::new();
    let mut buffer = [0_u8; 1024];
    while !request_bytes.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream
            .read(&mut buffer)
            .map_err(|error| format!("failed reading faulting HTTP request: {error}"))?;
        if read == 0 {
            return Err("faulting HTTP peer closed before complete headers".to_owned());
        }
        request_bytes.extend_from_slice(&buffer[..read]);
        if request_bytes.len() > 16 * 1024 {
            return Err("faulting HTTP request headers exceeded 16 KiB".to_owned());
        }
    }
    let request = std::str::from_utf8(&request_bytes)
        .map_err(|error| format!("faulting HTTP request headers were not UTF-8: {error}"))?;
    let mut lines = request.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| "faulting HTTP request line was absent".to_owned())?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().unwrap_or_default().to_owned();
    let path = request_parts.next().unwrap_or_default().to_owned();
    let version = request_parts.next().unwrap_or_default();
    if !matches!(method.as_str(), "GET" | "HEAD")
        || path != REAL_MPV_HTTP_FAULT_ROUTE
        || !matches!(version, "HTTP/1.0" | "HTTP/1.1")
    {
        return Err(format!(
            "faulting HTTP received unexpected request line {request_line:?}"
        ));
    }
    let range_header = lines.find_map(|line| {
        line.split_once(':').and_then(|(name, value)| {
            name.eq_ignore_ascii_case("range")
                .then(|| value.trim().to_owned())
        })
    });

    let inject_fault = method == "GET"
        && fault_injected
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok();
    let advertised_body_bytes = if inject_fault {
        REAL_MPV_HTTP_FAULT_DISCONNECT_AFTER_BYTES.min(generated_media.len())
    } else {
        generated_media.len()
    };
    let headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: audio/basic\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        advertised_body_bytes
    );
    let mut evidence = HttpRequestEvidence {
        ordinal,
        method: method.clone(),
        path,
        peer_endpoint: peer.to_string(),
        peer_ipv4_loopback: true,
        range_header,
        status_code: 200,
        advertised_body_bytes,
        transmitted_body_bytes: 0,
        disconnected_early: false,
        write_error: None,
    };
    if let Err(error) = stream.write_all(headers.as_bytes()) {
        evidence.write_error = Some(format!(
            "failed writing faulting HTTP response headers: {error}"
        ));
        return Ok(evidence);
    }

    let transmitted_body_bytes = if method == "HEAD" {
        0
    } else if inject_fault {
        let target = advertised_body_bytes;
        let started = Instant::now();
        let mut sent = 0;
        while sent < target && !shutdown.load(Ordering::Acquire) {
            let next = (sent + 16 * 1024).min(target);
            match stream.write(&generated_media[sent..next]) {
                Ok(0) => {
                    evidence.write_error =
                        Some("faulting HTTP short-body write made no progress".to_owned());
                    break;
                }
                Ok(written) => sent += written,
                Err(error) => {
                    evidence.write_error =
                        Some(format!("failed writing faulting HTTP short body: {error}"));
                    break;
                }
            }
            let target_elapsed =
                Duration::from_secs_f64(sent as f64 / REAL_MPV_HTTP_FAULT_BYTES_PER_SECOND as f64);
            if let Some(delay) = target_elapsed.checked_sub(started.elapsed()) {
                thread::sleep(delay);
            }
        }
        if evidence.write_error.is_none()
            && let Err(error) = stream.flush()
        {
            evidence.write_error =
                Some(format!("failed flushing faulting HTTP short body: {error}"));
        }
        let _ = stream.shutdown(Shutdown::Both);
        sent
    } else {
        let mut sent = 0;
        while sent < generated_media.len() {
            match stream.write(&generated_media[sent..]) {
                Ok(0) => {
                    evidence.write_error =
                        Some("recovered HTTP full-body write made no progress".to_owned());
                    break;
                }
                Ok(written) => sent += written,
                Err(error) => {
                    evidence.write_error =
                        Some(format!("failed writing recovered HTTP full body: {error}"));
                    break;
                }
            }
        }
        if evidence.write_error.is_none()
            && let Err(error) = stream.flush()
        {
            evidence.write_error =
                Some(format!("failed flushing recovered HTTP full body: {error}"));
        }
        sent
    };
    evidence.transmitted_body_bytes = transmitted_body_bytes;
    evidence.disconnected_early = method == "GET" && transmitted_body_bytes < generated_media.len();
    Ok(evidence)
}

#[derive(Debug, Clone, Serialize)]
struct MpvRecoveryEvidence {
    schema_version: u32,
    kind: &'static str,
    result: String,
    fault: &'static str,
    recovery_mode: &'static str,
    automatic_relaunch_timeout_ms: u128,
    initial_pid: u32,
    initial_parent_pid: u32,
    initial_process_image_path: String,
    initial_sha256: String,
    initial_ipc_endpoint: String,
    initial_process_terminated: bool,
    automatic_relaunch_observation_index: Option<usize>,
    automatic_relaunch_observation_event: &'static str,
    gui_room_remained_active: bool,
    manual_retry_invoked: bool,
    recovered_pid: Option<u32>,
    recovered_parent_pid: Option<u32>,
    recovered_process_image_path: Option<String>,
    recovered_sha256: Option<String>,
    recovered_ipc_endpoint: Option<String>,
    distinct_pid: bool,
    distinct_ipc_endpoint: bool,
    post_termination_observation_index: Option<usize>,
    recovered_file_loaded_index: Option<usize>,
    recovered_playing_index: Option<usize>,
    recovered_paused_index: Option<usize>,
    initial_process_still_terminated_after_recovery: bool,
    initial_process_still_terminated_after_gui_exit: bool,
    recovered_process_terminated_after_gui_exit: bool,
    error: Option<String>,
}

impl MpvRecoveryEvidence {
    fn new(
        initial_pid: u32,
        initial_parent_pid: u32,
        initial_process_image_path: &Path,
        initial_sha256: &str,
        initial_ipc_endpoint: &str,
        automatic_relaunch_timeout: Duration,
    ) -> Self {
        Self {
            schema_version: REAL_MPV_SCHEMA_VERSION,
            kind: REAL_MPV_RECOVERY_KIND,
            result: "running".to_owned(),
            fault: "terminate-exact-attested-gui-owned-mpv",
            recovery_mode: "active-session-automatic-managed-mpv-relaunch",
            automatic_relaunch_timeout_ms: automatic_relaunch_timeout.as_millis(),
            initial_pid,
            initial_parent_pid,
            initial_process_image_path: initial_process_image_path.display().to_string(),
            initial_sha256: initial_sha256.to_owned(),
            initial_ipc_endpoint: initial_ipc_endpoint.to_owned(),
            initial_process_terminated: false,
            automatic_relaunch_observation_index: None,
            automatic_relaunch_observation_event: "pause",
            gui_room_remained_active: false,
            manual_retry_invoked: false,
            recovered_pid: None,
            recovered_parent_pid: None,
            recovered_process_image_path: None,
            recovered_sha256: None,
            recovered_ipc_endpoint: None,
            distinct_pid: false,
            distinct_ipc_endpoint: false,
            post_termination_observation_index: None,
            recovered_file_loaded_index: None,
            recovered_playing_index: None,
            recovered_paused_index: None,
            initial_process_still_terminated_after_recovery: false,
            initial_process_still_terminated_after_gui_exit: false,
            recovered_process_terminated_after_gui_exit: false,
            error: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct MpvObservation {
    event: String,
    pid: Option<u32>,
    path: Option<String>,
    filename: Option<String>,
    duration: Option<f64>,
    position: Option<f64>,
    pause: Option<bool>,
    ipc_endpoint: Option<String>,
    reason: Option<String>,
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
    playlist_change_request: Option<String>,
    playlist_change_echo: Option<String>,
    playlist_index_request: Option<String>,
    playlist_index_echo: Option<String>,
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
            playlist_change_request: None,
            playlist_change_echo: None,
            playlist_index_request: None,
            playlist_index_echo: None,
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
    let recovery_path = artifact_root.join("owned-mpv-recovery.json");
    let http_fault_path = artifact_root.join("faulting-http-recovery.json");
    let mut state = RealMpvVerticalState::new(&artifact_root);
    write_json_file(&state_path, &state)?;
    let mut menu_interactions = MenuInteractionsEvidence::new();
    write_json_file(&menu_interactions_path, &menu_interactions)?;

    let started_at = Instant::now();
    let driver = PlatformNativeGuiDriver;
    let mut child: Option<Child> = None;
    let mut window = None;
    let mut verified_mpv_pids = Vec::new();
    let mut session_server: Option<MockSessionServer> = None;
    let mut session_exchange: Option<SessionExchangeEvidence> = None;
    let mut recovery_evidence: Option<MpvRecoveryEvidence> = None;
    let mut fault_http_server: Option<FaultingLoopbackHttpServer> = None;
    let mut http_fault_evidence: Option<HttpFaultRecoveryEvidence> = None;

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
        let media_path = artifact_root.join(if options.exercise_http_fault {
            "generated-silence.au"
        } else {
            "generated-silence.wav"
        });
        let observation_script_path = artifact_root.join("observe-real-mpv.lua");
        let observation_path = artifact_root.join("mpv-observation.jsonl");
        let mpv_log_path = artifact_root.join("mpv.log");
        let lifecycle_path = artifact_root.join("gui-lifecycle.jsonl");
        let automatic_relaunch_screenshot_path =
            artifact_root.join("owned-mpv-automatic-relaunch.png");
        let recovery_screenshot_path = artifact_root.join("owned-mpv-recovered.png");
        let success_screenshot_path = artifact_root.join("success-real-mpv.png");
        fs::create_dir_all(&appdata_root).map_err(|error| {
            format!(
                "failed to create isolated APPDATA root {}: {error}",
                appdata_root.display()
            )
        })?;
        let media_duration_seconds = if options.exercise_http_fault {
            REAL_MPV_HTTP_FAULT_DURATION_SECONDS
        } else {
            REAL_MPV_MEDIA_DURATION_SECONDS
        };
        let generated_media = if options.exercise_http_fault {
            pcm_au_bytes(media_duration_seconds)
        } else {
            pcm_wav_bytes(media_duration_seconds)
        };
        fs::write(&media_path, &generated_media).map_err(|error| {
            format!(
                "failed to write generated local media {}: {error}",
                media_path.display()
            )
        })?;
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

        let mut media_url = None;
        let media_open_target = if options.exercise_http_fault {
            let server = FaultingLoopbackHttpServer::start(generated_media.clone())?;
            let endpoint = server.endpoint();
            require_ipv4_loopback_endpoint(&endpoint, "faulting HTTP listener")?;
            let url = server.url();
            let evidence = HttpFaultRecoveryEvidence::new(endpoint, url.clone(), &generated_media);
            write_json_file(&http_fault_path, &evidence)?;
            fault_http_server = Some(server);
            http_fault_evidence = Some(evidence);
            media_url = Some(url.clone());
            PathBuf::from(url)
        } else {
            media_path.clone()
        };
        seed_real_mpv_config(
            &config_path,
            &mpv_path,
            &observation_script_path,
            &mpv_log_path,
            media_url.clone(),
        )?;
        state.advance(
            &state_path,
            "isolated-fixtures-ready",
            Some("isolated-config-and-generated-local-media"),
        )?;
        if options.exercise_http_fault {
            state.advance(
                &state_path,
                "faulting-http-ready",
                Some("strict-loopback-faulting-http-ready"),
            )?;
        }

        let server = if let Some(media_url) = media_url.as_ref() {
            start_playlist_echo_mock_session_server(
                REAL_MPV_SESSION_HELLO,
                media_url.clone(),
                REAL_MPV_LOOPBACK_USERNAME,
            )?
        } else {
            start_phased_mock_session_server(&[REAL_MPV_SESSION_HELLO])?
        };
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
            open_media_file_path: &media_open_target,
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
        if options.exercise_http_fault {
            let (
                playlist_change_request,
                playlist_change_echo,
                playlist_index_request,
                playlist_index_echo,
            ) = session_server
                .as_ref()
                .expect("faulting HTTP session server must remain live")
                .recv_playlist_exchange(step_timeout, "real-mpv faulting HTTP")?;
            let exchange = session_exchange
                .as_mut()
                .expect("real-mpv session exchange must remain initialized");
            exchange.playlist_change_request = Some(playlist_change_request);
            exchange.playlist_change_echo = Some(playlist_change_echo);
            exchange.playlist_index_request = Some(playlist_index_request);
            exchange.playlist_index_echo = Some(playlist_index_echo);
            write_json_file(&session_exchange_path, exchange)?;
        }

        wait_for_accessible_name(&driver, launched_window, "view: room", step_timeout)?;
        let (file_loaded_index, file_loaded) = wait_for_mpv_observation(
            &observation_path,
            0,
            step_timeout,
            "file-loaded for the generated local media",
            |observation| {
                observation.event == "file-loaded"
                    && observation.path.as_deref().is_some_and(|observed| {
                        observed_media_target_matches(observed, &media_path, media_url.as_deref())
                    })
            },
        )?;
        let mpv_pid = file_loaded.pid.ok_or_else(|| {
            "mpv file-loaded observation did not include its process ID".to_owned()
        })?;
        let expected_file_name = if options.exercise_http_fault {
            REAL_MPV_HTTP_FAULT_ROUTE
                .rsplit('/')
                .next()
                .expect("faulting HTTP route has a file component")
        } else {
            media_path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| "generated media file name was not valid UTF-8".to_owned())?
        };
        if file_loaded.filename.as_deref() != Some(expected_file_name) {
            return Err(format!(
                "real mpv reported filename {:?}; expected {expected_file_name:?}",
                file_loaded.filename
            ));
        }
        let observed_duration = file_loaded.duration.ok_or_else(|| {
            "mpv file-loaded observation did not include generated-media duration".to_owned()
        })?;
        if (observed_duration - f64::from(media_duration_seconds)).abs() > 0.05 {
            return Err(format!(
                "real mpv reported generated-media duration {observed_duration}; expected {}",
                media_duration_seconds
            ));
        }
        let parent_pid = process_parent_pid(mpv_pid)?;
        if parent_pid != gui_pid {
            return Err(format!(
                "real mpv PID {mpv_pid} was not owned by the launched GUI PID {gui_pid}; parent PID was {parent_pid}"
            ));
        }
        let initial_process_image_path = process_image_path(mpv_pid)?;
        let process_identity = binary_identity(&initial_process_image_path)?;
        if process_identity.sha256 != mpv_preflight.identity.sha256 {
            return Err(format!(
                "GUI-owned mpv process digest {} did not match preflight digest {}",
                process_identity.sha256, mpv_preflight.identity.sha256
            ));
        }
        verified_mpv_pids.push(mpv_pid);
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
        if options.exercise_recovery {
            let recovery = MpvRecoveryEvidence::new(
                mpv_pid,
                parent_pid,
                &initial_process_image_path,
                &process_identity.sha256,
                &ipc_endpoint,
                step_timeout,
            );
            write_json_file(&recovery_path, &recovery)?;
            recovery_evidence = Some(recovery);
        }
        if let Some(evidence) = http_fault_evidence.as_mut() {
            evidence.initial_file_loaded_index = Some(file_loaded_index);
            evidence.initial_pid = Some(mpv_pid);
            evidence.parent_pid = Some(parent_pid);
            evidence.process_image_path = Some(initial_process_image_path.display().to_string());
            evidence.process_sha256 = Some(process_identity.sha256.clone());
            evidence.initial_ipc_endpoint = Some(ipc_endpoint.clone());
            write_json_file(&http_fault_path, evidence)?;
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

        if options.exercise_http_fault {
            let fault_timeout = options.timeout.min(Duration::from_secs(30));
            let (pre_fault_progress_index, pre_fault_progress) = wait_for_mpv_observation(
                &observation_path,
                playing_index,
                fault_timeout,
                "positive time-pos before the controlled HTTP disconnect",
                |observation| {
                    observation.event == "time-pos"
                        && observation.pid == Some(mpv_pid)
                        && observation.ipc_endpoint.as_deref() == Some(&ipc_endpoint)
                        && observation.path.as_deref() == media_url.as_deref()
                        && observation.position.is_some_and(|position| position >= 0.5)
                },
            )?;
            let pre_fault_position = pre_fault_progress
                .position
                .ok_or_else(|| "pre-fault time-pos observation omitted its position".to_owned())?;
            let (end_file_index, end_file) = wait_for_mpv_observation(
                &observation_path,
                pre_fault_progress_index,
                fault_timeout,
                "causal end-file:eof after the controlled short HTTP response",
                |observation| {
                    observation.event == "end-file"
                        && observation.pid == Some(mpv_pid)
                        && observation.reason.as_deref() == Some("eof")
                        && observation.ipc_endpoint.as_deref() == Some(&ipc_endpoint)
                },
            )?;
            let first_requests = fault_http_server
                .as_ref()
                .expect("faulting HTTP server must remain live")
                .wait_for_media_gets(1, step_timeout)?;
            let first_request = first_requests
                .iter()
                .find(|request| request.method == "GET")
                .ok_or_else(|| "faulting HTTP request evidence was empty".to_owned())?;
            if !first_request.disconnected_early
                || first_request.status_code != 200
                || first_request.range_header.as_deref() != Some("bytes=0-")
                || first_request.transmitted_body_bytes
                    != REAL_MPV_HTTP_FAULT_DISCONNECT_AFTER_BYTES
                || first_request.advertised_body_bytes != REAL_MPV_HTTP_FAULT_DISCONNECT_AFTER_BYTES
                || first_request.write_error.is_some()
            {
                return Err(format!(
                    "first faulting HTTP response was not the exact controlled short response: {first_request:?}"
                ));
            }
            if end_file.path.as_deref().is_some_and(|observed| {
                !observed_media_target_matches(observed, &media_path, media_url.as_deref())
            }) {
                return Err(format!(
                    "terminal HTTP observation drifted to an unexpected path: {:?}",
                    end_file.path
                ));
            }
            state.advance(
                &state_path,
                "premature-http-eof-observed",
                Some("one-premature-http-eof-observed"),
            )?;

            let (recovered_file_loaded_index, recovered_file_loaded) = wait_for_mpv_observation(
                &observation_path,
                end_file_index.saturating_add(1),
                fault_timeout,
                "same-process file-loaded after automatic HTTP recovery",
                |observation| {
                    observation.event == "file-loaded"
                        && observation.pid == Some(mpv_pid)
                        && observation.ipc_endpoint.as_deref() == Some(&ipc_endpoint)
                        && observation.path.as_deref().is_some_and(|observed| {
                            observed_media_target_matches(
                                observed,
                                &media_path,
                                media_url.as_deref(),
                            )
                        })
                },
            )?;
            let recovered_duration = recovered_file_loaded.duration.ok_or_else(|| {
                "recovered HTTP file-loaded observation omitted duration".to_owned()
            })?;
            if recovered_file_loaded.filename.as_deref() != Some(expected_file_name)
                || (recovered_duration - f64::from(media_duration_seconds)).abs() > 0.05
            {
                return Err(format!(
                    "recovered HTTP media identity drifted: filename={:?}, duration={recovered_duration}",
                    recovered_file_loaded.filename
                ));
            }
            if !process_is_running(mpv_pid)
                || process_parent_pid(mpv_pid)? != gui_pid
                || binary_identity(&process_image_path(mpv_pid)?)?.sha256
                    != mpv_preflight.identity.sha256
            {
                return Err(format!(
                    "GUI-owned mpv identity changed across the HTTP fault boundary for PID {mpv_pid}"
                ));
            }
            state.advance(
                &state_path,
                "faulting-http-reloaded",
                Some("same-owned-mpv-reloaded-stable-network-media"),
            )?;

            let required_recovered_position = pre_fault_position + 0.5;
            let (recovered_progress_index, recovered_progress) = wait_for_mpv_observation(
                &observation_path,
                recovered_file_loaded_index,
                fault_timeout,
                "post-recovery playback progress beyond the pre-fault position",
                |observation| {
                    observation.event == "time-pos"
                        && observation.pid == Some(mpv_pid)
                        && observation.ipc_endpoint.as_deref() == Some(&ipc_endpoint)
                        && observation.path.as_deref() == media_url.as_deref()
                        && observation
                            .position
                            .is_some_and(|position| position >= required_recovered_position)
                },
            )?;
            let recovered_position = recovered_progress
                .position
                .ok_or_else(|| "recovered time-pos observation omitted its position".to_owned())?;
            wait_for_accessible_name(
                &driver,
                launched_window,
                "Room state: playing",
                step_timeout,
            )?;
            state.advance(
                &state_path,
                "faulting-http-progress-recovered",
                Some("recovered-playback-advanced-past-fault"),
            )?;

            let observations_before_pause = read_mpv_observations(&observation_path)?.len();
            invoke_named_control_with_wait(
                &driver,
                launched_window,
                PAUSE_CONTROL_AUTOMATION_ID,
                NativeControlKind::Button,
                step_timeout,
            )?;
            let (recovered_paused_index, _) = wait_for_mpv_observation(
                &observation_path,
                observations_before_pause,
                step_timeout,
                "pause=true after recovered HTTP playback",
                |observation| {
                    observation.event == "pause"
                        && observation.pid == Some(mpv_pid)
                        && observation.pause == Some(true)
                        && observation.ipc_endpoint.as_deref() == Some(&ipc_endpoint)
                        && observation.path.as_deref() == media_url.as_deref()
                },
            )?;
            if !(file_loaded_index < playing_index
                && playing_index < pre_fault_progress_index
                && pre_fault_progress_index < end_file_index
                && end_file_index < recovered_file_loaded_index
                && recovered_file_loaded_index < recovered_progress_index
                && recovered_progress_index < recovered_paused_index)
            {
                return Err(format!(
                    "faulting HTTP observation ordering drifted: {file_loaded_index}, {playing_index}, {pre_fault_progress_index}, {end_file_index}, {recovered_file_loaded_index}, {recovered_progress_index}, {recovered_paused_index}"
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

            let requests = fault_http_server
                .as_ref()
                .expect("faulting HTTP server must remain live")
                .wait_for_media_gets(2, step_timeout)?;
            validate_faulting_http_request_accounting(&requests, generated_media.len())?;
            let observations = read_mpv_observations(&observation_path)?;
            let foreign_observations = observations
                .iter()
                .skip(end_file_index)
                .take(
                    recovered_paused_index
                        .saturating_sub(end_file_index)
                        .saturating_add(1),
                )
                .filter(|observation| {
                    observation.pid.is_some_and(|pid| pid != mpv_pid)
                        || observation
                            .ipc_endpoint
                            .as_deref()
                            .is_some_and(|endpoint| endpoint != ipc_endpoint)
                })
                .count();
            if foreign_observations != 0 {
                return Err(format!(
                    "stale or foreign mpv generation emitted {foreign_observations} observations after the HTTP fault boundary"
                ));
            }
            let evidence = http_fault_evidence
                .as_mut()
                .expect("faulting HTTP evidence must be initialized");
            evidence.record_requests(requests);
            evidence.pre_fault_progress_index = Some(pre_fault_progress_index);
            evidence.end_file_index = Some(end_file_index);
            evidence.recovered_file_loaded_index = Some(recovered_file_loaded_index);
            evidence.recovered_progress_index = Some(recovered_progress_index);
            evidence.recovered_paused_index = Some(recovered_paused_index);
            evidence.recovered_pid = Some(mpv_pid);
            evidence.recovered_ipc_endpoint = Some(ipc_endpoint.clone());
            evidence.stable_process_identity = true;
            evidence.stable_ipc_endpoint = true;
            evidence.stable_media_url = true;
            evidence.stable_duration = true;
            evidence.pre_fault_position_seconds = Some(pre_fault_position);
            evidence.recovered_position_seconds = Some(recovered_position);
            evidence.foreign_pid_observations_after_fault = foreign_observations;
            evidence.evidence_retained_before_cleanup = true;
            write_json_file(&http_fault_path, evidence)?;
            state.advance(
                &state_path,
                "faulting-http-evidence-retained",
                Some("fault-evidence-retained-before-cleanup"),
            )?;
        } else {
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
        }

        let mut active_mpv_pid = mpv_pid;
        let mut recovered_mpv_identity = None;
        if options.exercise_recovery {
            terminate_test_process(mpv_pid)?;
            wait_for_process_termination(mpv_pid, step_timeout)?;
            if process_is_running(mpv_pid) {
                return Err(format!(
                    "initial mpv PID {mpv_pid} remained alive before automatic replacement"
                ));
            }
            let post_termination_observation_index =
                read_mpv_observations(&observation_path)?.len();
            {
                let recovery = recovery_evidence
                    .as_mut()
                    .expect("real-mpv recovery evidence must be initialized");
                recovery.initial_process_terminated = true;
                recovery.post_termination_observation_index =
                    Some(post_termination_observation_index);
                write_json_file(&recovery_path, recovery)?;
            }
            state.advance(
                &state_path,
                "owned-mpv-terminated",
                Some("exact-attested-owned-mpv-terminated"),
            )?;

            let (automatic_relaunch_observation_index, recovered_started) =
                wait_for_automatic_replacement_observation(
                    &observation_path,
                    post_termination_observation_index,
                    mpv_pid,
                    &expected_ipc_prefix,
                    &ipc_endpoint,
                    step_timeout,
                )?;
            let recovered_mpv_pid = recovered_started.pid.ok_or_else(|| {
                "replacement mpv pause observation did not include its process ID".to_owned()
            })?;
            let recovered_ipc_endpoint = recovered_started
                .ipc_endpoint
                .clone()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    "replacement mpv observation did not expose its managed IPC endpoint".to_owned()
                })?;
            if recovered_mpv_pid == mpv_pid {
                return Err(format!(
                    "Automatic mpv recovery reused terminated PID {mpv_pid}; expected a replacement process"
                ));
            }
            if recovered_ipc_endpoint == ipc_endpoint {
                return Err(format!(
                    "Automatic mpv recovery reused terminated IPC endpoint {ipc_endpoint:?}; expected a fresh endpoint"
                ));
            }
            if !recovered_ipc_endpoint.starts_with(&expected_ipc_prefix) {
                return Err(format!(
                    "replacement mpv IPC endpoint {recovered_ipc_endpoint:?} did not use the expected product-generated prefix {expected_ipc_prefix:?}"
                ));
            }
            let recovered_parent_pid = process_parent_pid(recovered_mpv_pid)?;
            if recovered_parent_pid != gui_pid {
                return Err(format!(
                    "replacement real mpv PID {recovered_mpv_pid} was not owned by GUI PID {gui_pid}; parent PID was {recovered_parent_pid}"
                ));
            }
            let recovered_process_image_path = process_image_path(recovered_mpv_pid)?;
            let recovered_process_identity = binary_identity(&recovered_process_image_path)?;
            if recovered_process_identity.sha256 != mpv_preflight.identity.sha256 {
                return Err(format!(
                    "replacement GUI-owned mpv process digest {} did not match preflight digest {}",
                    recovered_process_identity.sha256, mpv_preflight.identity.sha256
                ));
            }
            verified_mpv_pids.push(recovered_mpv_pid);
            active_mpv_pid = recovered_mpv_pid;
            state.recovered_mpv_pid = Some(recovered_mpv_pid);
            if process_is_running(mpv_pid) {
                return Err(format!(
                    "initial mpv PID {mpv_pid} resumed before automatic replacement attestation"
                ));
            }
            {
                let recovery = recovery_evidence
                    .as_mut()
                    .expect("real-mpv recovery evidence must be initialized");
                recovery.automatic_relaunch_observation_index =
                    Some(automatic_relaunch_observation_index);
                recovery.recovered_pid = Some(recovered_mpv_pid);
                recovery.recovered_parent_pid = Some(recovered_parent_pid);
                recovery.recovered_process_image_path =
                    Some(recovered_process_image_path.display().to_string());
                recovery.recovered_sha256 = Some(recovered_process_identity.sha256.clone());
                recovery.recovered_ipc_endpoint = Some(recovered_ipc_endpoint.clone());
                recovery.distinct_pid = true;
                recovery.distinct_ipc_endpoint = true;
                write_json_file(&recovery_path, recovery)?;
            }
            state.advance(
                &state_path,
                "automatic-replacement-owned-mpv-ready",
                Some("automatic-relaunch-distinct-owned-exact-mpv"),
            )?;
            wait_for_accessible_name(&driver, launched_window, "view: room", step_timeout)?;
            driver
                .capture_window_png(launched_window, &automatic_relaunch_screenshot_path)
                .map_err(|error| {
                    format!("failed to retain active-room automatic-relaunch screenshot: {error}")
                })?;
            {
                let recovery = recovery_evidence
                    .as_mut()
                    .expect("real-mpv recovery evidence must be initialized");
                recovery.gui_room_remained_active = true;
                write_json_file(&recovery_path, recovery)?;
            }
            state.advance(
                &state_path,
                "automatic-relaunch-room-active",
                Some("gui-remained-on-active-room-during-automatic-relaunch"),
            )?;

            let observations_before_recovered_open =
                read_mpv_observations(&observation_path)?.len();
            invoke_real_mpv_menu_action_with_evidence(
                &driver,
                launched_window,
                FILE_MENU_AUTOMATION_ID,
                OPEN_MEDIA_MENU_AUTOMATION_ID,
                step_timeout,
                &mut menu_interactions,
                &menu_interactions_path,
            )?;
            let (recovered_file_loaded_index, recovered_file_loaded) = wait_for_mpv_observation(
                &observation_path,
                observations_before_recovered_open,
                step_timeout,
                "file-loaded for generated local media from the replacement real mpv",
                |observation| {
                    observation.event == "file-loaded"
                        && observation.pid == Some(recovered_mpv_pid)
                        && observation.path.as_deref().is_some_and(|observed| {
                            observed_media_path_matches(Path::new(observed), &media_path)
                        })
                        && observation.ipc_endpoint.as_deref() == Some(&recovered_ipc_endpoint)
                },
            )?;
            if recovered_file_loaded.filename.as_deref() != Some(expected_file_name) {
                return Err(format!(
                    "replacement real mpv reported filename {:?}; expected {expected_file_name:?}",
                    recovered_file_loaded.filename
                ));
            }
            let recovered_duration = recovered_file_loaded.duration.ok_or_else(|| {
                "replacement mpv file-loaded observation omitted generated-media duration"
                    .to_owned()
            })?;
            if (recovered_duration - f64::from(REAL_MPV_MEDIA_DURATION_SECONDS)).abs() > 0.05 {
                return Err(format!(
                    "replacement real mpv reported generated-media duration {recovered_duration}; expected {}",
                    REAL_MPV_MEDIA_DURATION_SECONDS
                ));
            }
            state.advance(
                &state_path,
                "replacement-real-mpv-file-loaded",
                Some("replacement-mpv-loaded-generated-media"),
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
            let observations_before_recovered_play =
                read_mpv_observations(&observation_path)?.len();
            invoke_named_control_with_wait(
                &driver,
                launched_window,
                PLAY_CONTROL_AUTOMATION_ID,
                NativeControlKind::Button,
                step_timeout,
            )?;
            let (recovered_playing_index, _) = wait_for_mpv_observation(
                &observation_path,
                observations_before_recovered_play,
                step_timeout,
                "pause=false after GUI Play on replacement real mpv",
                |observation| {
                    observation.event == "pause"
                        && observation.pid == Some(recovered_mpv_pid)
                        && observation.pause == Some(false)
                },
            )?;
            wait_for_accessible_name(
                &driver,
                launched_window,
                "Room state: playing",
                step_timeout,
            )?;
            state.advance(
                &state_path,
                "replacement-real-mpv-playing",
                Some("gui-play-command-observed-by-replacement-mpv"),
            )?;

            let observations_before_recovered_pause =
                read_mpv_observations(&observation_path)?.len();
            invoke_named_control_with_wait(
                &driver,
                launched_window,
                PAUSE_CONTROL_AUTOMATION_ID,
                NativeControlKind::Button,
                step_timeout,
            )?;
            let (recovered_paused_index, _) = wait_for_mpv_observation(
                &observation_path,
                observations_before_recovered_pause,
                step_timeout,
                "pause=true after GUI Pause on replacement real mpv",
                |observation| {
                    observation.event == "pause"
                        && observation.pid == Some(recovered_mpv_pid)
                        && observation.pause == Some(true)
                },
            )?;
            wait_for_accessible_name(&driver, launched_window, "Room state: paused", step_timeout)?;
            state.advance(
                &state_path,
                "replacement-real-mpv-paused",
                Some("gui-pause-command-observed-by-replacement-mpv"),
            )?;
            if !(recovered_file_loaded_index < recovered_playing_index
                && recovered_playing_index < recovered_paused_index)
            {
                return Err(format!(
                    "replacement mpv ordering was not file-loaded < playing < paused: {recovered_file_loaded_index}, {recovered_playing_index}, {recovered_paused_index}"
                ));
            }
            if process_is_running(mpv_pid) {
                return Err(format!(
                    "terminated initial mpv PID {mpv_pid} was running after replacement recovery"
                ));
            }
            let observations_after_termination = read_mpv_observations(&observation_path)?;
            if observations_after_termination
                .iter()
                .skip(post_termination_observation_index)
                .any(|observation| observation.pid == Some(mpv_pid))
            {
                return Err(format!(
                    "terminated initial mpv PID {mpv_pid} emitted an observation after the recovery boundary"
                ));
            }
            {
                let recovery = recovery_evidence
                    .as_mut()
                    .expect("real-mpv recovery evidence must be initialized");
                recovery.recovered_file_loaded_index = Some(recovered_file_loaded_index);
                recovery.recovered_playing_index = Some(recovered_playing_index);
                recovery.recovered_paused_index = Some(recovered_paused_index);
                recovery.initial_process_still_terminated_after_recovery = true;
                recovery.result = "passed".to_owned();
                write_json_file(&recovery_path, recovery)?;
            }
            driver
                .capture_window_png(launched_window, &recovery_screenshot_path)
                .map_err(|error| {
                    format!("failed to retain successful owned-mpv recovery screenshot: {error}")
                })?;
            state.advance(
                &state_path,
                "owned-mpv-recovery-complete",
                Some("replacement-transport-recovered-with-old-mpv-fenced"),
            )?;
            recovered_mpv_identity = Some(MpvIdentity {
                path: mpv_preflight.identity.path.clone(),
                bytes: mpv_preflight.identity.bytes,
                sha256: mpv_preflight.identity.sha256.clone(),
                version: mpv_preflight.version.clone(),
                minimum_supported_version: sorotte_player_mpv::MINIMUM_SUPPORTED_MPV_VERSION,
                pid: recovered_mpv_pid,
                parent_pid: recovered_parent_pid,
                process_image_path: recovered_process_image_path.display().to_string(),
            });
        }

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
        for verified_pid in verified_mpv_pids.iter().copied() {
            wait_for_process_termination(verified_pid, step_timeout)?;
        }
        if options.exercise_recovery {
            let recovery = recovery_evidence
                .as_mut()
                .expect("real-mpv recovery evidence must be complete");
            recovery.initial_process_still_terminated_after_gui_exit = !process_is_running(mpv_pid);
            recovery.recovered_process_terminated_after_gui_exit =
                !process_is_running(active_mpv_pid);
            write_json_file(&recovery_path, recovery)?;
        }
        if options.exercise_http_fault {
            let requests = fault_http_server
                .take()
                .expect("faulting HTTP server must remain live until GUI exit")
                .release()?;
            validate_faulting_http_request_accounting(&requests, generated_media.len())?;
            let evidence = http_fault_evidence
                .as_mut()
                .expect("faulting HTTP evidence must be complete");
            evidence.record_requests(requests);
            evidence.server_thread_released = true;
            evidence.socket_released = true;
            evidence.owned_mpv_terminated_after_gui_exit = !process_is_running(mpv_pid);
            if !evidence.owned_mpv_terminated_after_gui_exit {
                return Err(format!(
                    "GUI-owned mpv PID {mpv_pid} remained alive after faulting HTTP GUI exit"
                ));
            }
            evidence.result = "passed".to_owned();
            write_json_file(&http_fault_path, evidence)?;
        }
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
        let exit_assertion = if options.exercise_recovery {
            "gui-exit-reaped-replacement-owned-mpv"
        } else if options.exercise_http_fault {
            "gui-exit-reaped-owned-mpv-and-released-fault-server"
        } else {
            "gui-exit-reaped-owned-mpv"
        };
        state.advance(&state_path, "complete", Some(exit_assertion))?;
        state.result = "passed".to_owned();
        write_json_file(&state_path, &state)?;

        let mut artifact_files = vec![
            ("config", config_path.as_path()),
            ("generated_media", media_path.as_path()),
            ("observation_script", observation_script_path.as_path()),
            ("mpv_observation", observation_path.as_path()),
            ("mpv_log", mpv_log_path.as_path()),
            ("gui_lifecycle", lifecycle_path.as_path()),
            ("session_exchange", session_exchange_path.as_path()),
            ("menu_interactions", menu_interactions_path.as_path()),
            ("success_screenshot", success_screenshot_path.as_path()),
            ("state", state_path.as_path()),
        ];
        if options.exercise_recovery {
            artifact_files.extend([
                ("owned_mpv_recovery", recovery_path.as_path()),
                (
                    "automatic_relaunch_screenshot",
                    automatic_relaunch_screenshot_path.as_path(),
                ),
                ("recovery_screenshot", recovery_screenshot_path.as_path()),
            ]);
        }
        if options.exercise_http_fault {
            artifact_files.push(("faulting_http_recovery", http_fault_path.as_path()));
        }
        let artifacts = artifact_manifest(&artifact_root, &artifact_files)?;
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
                process_image_path: initial_process_image_path.display().to_string(),
            },
            recovered_mpv: recovered_mpv_identity,
            recovery: recovery_evidence.clone(),
            http_fault: http_fault_evidence.clone(),
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
                network_mode: if options.exercise_http_fault {
                    "os-assigned-ipv4-loopback-session-and-http"
                } else {
                    "os-assigned-ipv4-loopback-session"
                },
                media_source: if options.exercise_http_fault {
                    "generated-pcm-au-over-faulting-loopback-http"
                } else {
                    "generated-local-pcm-wav"
                },
                mpv_config: "isolated --no-config",
                media_url: media_url.clone(),
                http_endpoint: http_fault_evidence
                    .as_ref()
                    .map(|evidence| evidence.listener_endpoint.clone()),
                http_evidence_path: options
                    .exercise_http_fault
                    .then(|| http_fault_path.display().to_string()),
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
            if let (Some(server), Some(evidence)) =
                (fault_http_server.as_ref(), http_fault_evidence.as_mut())
                && let Ok(requests) = server.requests()
            {
                evidence.record_requests(requests);
                let _ = write_json_file(&http_fault_path, evidence);
            }
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
            for mpv_pid in verified_mpv_pids.iter().copied() {
                if process_is_running(mpv_pid) {
                    let _ = terminate_test_process(mpv_pid);
                }
            }
            if let Some(server) = fault_http_server.take() {
                match server.release() {
                    Ok(requests) => {
                        if let Some(evidence) = http_fault_evidence.as_mut() {
                            evidence.record_requests(requests);
                            evidence.server_thread_released = true;
                            evidence.socket_released = true;
                        }
                    }
                    Err(release_error) => {
                        error = format!("{error}; {release_error}");
                    }
                }
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
            if let Some(recovery) = recovery_evidence.as_mut() {
                recovery.result = "failed".to_owned();
                recovery.error = Some(redact_real_mpv_error(&error));
                let _ = write_json_file(&recovery_path, recovery);
            }
            if let Some(evidence) = http_fault_evidence.as_mut() {
                evidence.result = "failed".to_owned();
                evidence.owned_mpv_terminated_after_gui_exit = evidence
                    .initial_pid
                    .is_some_and(|pid| !process_is_running(pid));
                evidence.error = Some(redact_real_mpv_error(&error));
                let _ = write_json_file(&http_fault_path, evidence);
            }
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
    let mut exercise_recovery = false;
    let mut exercise_http_fault = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--real-mpv-vertical" | "--json" => index += 1,
            "--exercise-owned-mpv-recovery" => {
                exercise_recovery = true;
                index += 1;
            }
            "--exercise-faulting-http-recovery" => {
                exercise_http_fault = true;
                index += 1;
            }
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
                    "unknown real-mpv vertical argument {argument:?}; expected --binary, --mpv, --artifact-dir, optional --timeout-ms, optional --exercise-owned-mpv-recovery, and optional --exercise-faulting-http-recovery"
                ));
            }
        }
    }
    if exercise_recovery && exercise_http_fault {
        return Err(
            "--exercise-owned-mpv-recovery and --exercise-faulting-http-recovery are mutually exclusive"
                .to_owned(),
        );
    }
    Ok(RealMpvVerticalOptions {
        binary_path: binary_path
            .ok_or_else(|| "--real-mpv-vertical requires --binary PATH".to_owned())?,
        mpv_path: mpv_path.ok_or_else(|| "--real-mpv-vertical requires --mpv PATH".to_owned())?,
        artifact_dir: artifact_dir
            .ok_or_else(|| "--real-mpv-vertical requires --artifact-dir PATH".to_owned())?,
        timeout,
        exercise_recovery,
        exercise_http_fault,
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
    trusted_domain: Option<String>,
) -> Result<(), String> {
    let exercise_faulting_http_recovery = trusted_domain.is_some();
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
        shared_playlist_enabled: Some(exercise_faulting_http_recovery),
        show_osd: Some(false),
        chat_input_enabled: Some(false),
        chat_output_enabled: Some(false),
        check_for_updates_automatically: Some(false),
        only_switch_to_trusted_domains: trusted_domain.as_ref().map(|_| true),
        trusted_domains: trusted_domain.map(|domain| vec![domain]),
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

local function emit(event, event_data)
    local record = {{
        event = event,
        pid = utils.getpid(),
        path = mp.get_property_native("path"),
        filename = mp.get_property_native("filename"),
        duration = mp.get_property_native("duration"),
        position = mp.get_property_native("time-pos"),
        pause = mp.get_property_native("pause"),
        ipc_endpoint = mp.get_property_native("input-ipc-server"),
        reason = event_data and event_data.reason or nil,
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
mp.register_event("end-file", function(event_data) emit("end-file", event_data) end)
mp.observe_property("pause", "bool", function(_, value)
    if value ~= nil then
        emit("pause")
    end
end)
mp.observe_property("time-pos", "number", function(_, value)
    if value ~= nil then
        emit("time-pos")
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

fn pcm_au_bytes(duration_seconds: u32) -> Vec<u8> {
    const HEADER_BYTES: u32 = 24;
    const SAMPLE_RATE: u32 = 48_000;
    const CHANNELS: u32 = 1;
    const ENCODING_LINEAR_PCM_16_BIT: u32 = 3;
    const BYTES_PER_SAMPLE: u32 = 2;
    let data_bytes = SAMPLE_RATE
        .saturating_mul(duration_seconds)
        .saturating_mul(CHANNELS)
        .saturating_mul(BYTES_PER_SAMPLE);
    let mut au = Vec::with_capacity(HEADER_BYTES as usize + data_bytes as usize);
    au.extend_from_slice(b".snd");
    au.extend_from_slice(&HEADER_BYTES.to_be_bytes());
    au.extend_from_slice(&data_bytes.to_be_bytes());
    au.extend_from_slice(&ENCODING_LINEAR_PCM_16_BIT.to_be_bytes());
    au.extend_from_slice(&SAMPLE_RATE.to_be_bytes());
    au.extend_from_slice(&CHANNELS.to_be_bytes());
    au.resize(HEADER_BYTES as usize + data_bytes as usize, 0);
    au
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

fn wait_for_automatic_replacement_observation(
    path: &Path,
    start_index: usize,
    terminated_pid: u32,
    expected_ipc_prefix: &str,
    terminated_ipc_endpoint: &str,
    timeout: Duration,
) -> Result<(usize, MpvObservation), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if process_is_running(terminated_pid) {
            return Err(format!(
                "terminated initial mpv PID {terminated_pid} became live while waiting for automatic replacement"
            ));
        }
        let observations = read_mpv_observations(path)?;
        if observations
            .iter()
            .skip(start_index)
            .any(|observation| observation.pid == Some(terminated_pid))
        {
            return Err(format!(
                "terminated initial mpv PID {terminated_pid} emitted an observation while waiting for automatic replacement"
            ));
        }
        if let Some((index, observation)) =
            observations
                .iter()
                .enumerate()
                .skip(start_index)
                .find(|(_, observation)| {
                    observation.event == "pause"
                        && observation.pid.is_some_and(|pid| pid != terminated_pid)
                        && observation.ipc_endpoint.as_deref().is_some_and(|endpoint| {
                            endpoint.starts_with(expected_ipc_prefix)
                                && endpoint != terminated_ipc_endpoint
                        })
                })
        {
            return Ok((index, observation.clone()));
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for bounded automatic active-session replacement mpv; observations={observations:?}"
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

fn observed_media_target_matches(
    observed: &str,
    expected_local_path: &Path,
    expected_url: Option<&str>,
) -> bool {
    expected_url.map_or_else(
        || observed_media_path_matches(Path::new(observed), expected_local_path),
        |expected_url| observed == expected_url,
    )
}

fn validate_faulting_http_request_accounting(
    requests: &[HttpRequestEvidence],
    generated_media_bytes: usize,
) -> Result<(), String> {
    if requests.is_empty() {
        return Err("faulting HTTP request accounting was empty".to_owned());
    }
    for (index, request) in requests.iter().enumerate() {
        if request.ordinal != index + 1
            || request.path != REAL_MPV_HTTP_FAULT_ROUTE
            || !request.peer_ipv4_loopback
            || request.write_error.is_some()
        {
            return Err(format!(
                "faulting HTTP request accounting drifted at row {index}: {request:?}"
            ));
        }
        require_ipv4_loopback_endpoint(&request.peer_endpoint, "faulting HTTP connected peer")?;
        if request.method == "HEAD" {
            if request.status_code != 200
                || request.advertised_body_bytes != generated_media_bytes
                || request.transmitted_body_bytes != 0
                || request.disconnected_early
            {
                return Err(format!(
                    "faulting HTTP HEAD probe unexpectedly carried a body or fault: {request:?}"
                ));
            }
        } else if request.method == "GET" {
            if request.status_code != 200 || request.range_header.as_deref() != Some("bytes=0-") {
                return Err(format!(
                    "faulting HTTP media GET did not use the exact byte-zero non-seekable contract: {request:?}"
                ));
            }
        } else {
            return Err(format!(
                "faulting HTTP request method was not explicitly accounted: {request:?}"
            ));
        }
    }
    let media_gets = requests
        .iter()
        .filter(|request| request.method == "GET")
        .collect::<Vec<_>>();
    if media_gets.len() != 2 {
        return Err(format!(
            "faulting HTTP expected exactly two media GETs (one short, one complete); requests={requests:?}"
        ));
    }
    let short = media_gets[0];
    if !short.disconnected_early
        || short.advertised_body_bytes != REAL_MPV_HTTP_FAULT_DISCONNECT_AFTER_BYTES
        || short.transmitted_body_bytes != REAL_MPV_HTTP_FAULT_DISCONNECT_AFTER_BYTES
        || short.advertised_body_bytes >= generated_media_bytes
    {
        return Err(format!(
            "faulting HTTP first media GET was not the exact one-shot short response: {short:?}"
        ));
    }
    let complete = media_gets[1];
    if complete.disconnected_early
        || complete.advertised_body_bytes != generated_media_bytes
        || complete.transmitted_body_bytes != complete.advertised_body_bytes
    {
        return Err(format!(
            "faulting HTTP recovery GET was not a complete response: {complete:?}"
        ));
    }
    if requests
        .iter()
        .filter(|request| request.disconnected_early)
        .count()
        != 1
    {
        return Err(format!(
            "faulting HTTP did not retain exactly one premature response: {requests:?}"
        ));
    }
    Ok(())
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
        assert!(!parsed.exercise_recovery);
        assert!(!parsed.exercise_http_fault);

        let mut recovery_args = args.clone();
        recovery_args.push("--exercise-owned-mpv-recovery".to_owned());
        assert!(
            parse_real_mpv_vertical_options(&recovery_args)
                .expect("recovery options should parse")
                .exercise_recovery
        );
        let mut http_fault_args = args.clone();
        http_fault_args.push("--exercise-faulting-http-recovery".to_owned());
        assert!(
            parse_real_mpv_vertical_options(&http_fault_args)
                .expect("faulting HTTP options should parse")
                .exercise_http_fault
        );
        let mut conflicting_args = recovery_args;
        conflicting_args.push("--exercise-faulting-http-recovery".to_owned());
        assert!(parse_real_mpv_vertical_options(&conflicting_args).is_err());

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
    fn generated_au_has_exact_pcm_header_and_duration() {
        let au = pcm_au_bytes(3);
        assert_eq!(&au[..4], b".snd");
        assert_eq!(
            u32::from_be_bytes(au[4..8].try_into().expect("data offset")),
            24
        );
        let data_bytes = u32::from_be_bytes(au[8..12].try_into().expect("data length"));
        assert_eq!(data_bytes, 48_000 * 3 * 2);
        assert_eq!(
            u32::from_be_bytes(au[12..16].try_into().expect("encoding")),
            3
        );
        assert_eq!(
            u32::from_be_bytes(au[16..20].try_into().expect("sample rate")),
            48_000
        );
        assert_eq!(
            u32::from_be_bytes(au[20..24].try_into().expect("channels")),
            1
        );
        assert_eq!(au.len(), 24 + data_bytes as usize);
        assert_eq!(data_bytes / (48_000 * 2), 3);
    }

    #[test]
    fn lua_observer_uses_safe_long_string_and_required_real_state() {
        let path = Path::new(r"C:\isolated\contains]]\observation.jsonl");
        let script = real_mpv_observation_lua(path);
        assert!(script.contains(r"C:\isolated\contains]]\observation.jsonl"));
        assert!(script.contains("utils.getpid()"));
        assert!(script.contains(r#"mp.register_event("file-loaded""#));
        assert!(script.contains(r#"mp.register_event("end-file""#));
        assert!(script.contains(r#"mp.observe_property("pause""#));
        assert!(script.contains(r#"mp.observe_property("time-pos""#));
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

    #[test]
    fn mock_session_release_proves_and_leaves_exact_endpoint_rebindable() {
        let server =
            start_phased_mock_session_server(&[]).expect("start releasable mock session fixture");
        let endpoint = server.address.clone();

        server
            .release("exact endpoint rebind")
            .expect("release must join and prove exact endpoint reuse");
        let rebound =
            TcpListener::bind(&endpoint).expect("released exact endpoint should remain rebindable");
        assert_eq!(
            rebound
                .local_addr()
                .expect("read rebound endpoint")
                .to_string(),
            endpoint
        );
        drop(rebound);
    }

    fn connect_playlist_echo_test_server(
        media_url: &str,
    ) -> (MockSessionServer, String, TcpStream, BufReader<TcpStream>) {
        let server = start_playlist_echo_mock_session_server(
            REAL_MPV_SESSION_HELLO,
            media_url.to_owned(),
            REAL_MPV_LOOPBACK_USERNAME,
        )
        .expect("start playlist-echo fixture");
        let endpoint = server.address.clone();
        let mut stream = TcpStream::connect(&endpoint).expect("connect playlist-echo fixture");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("set playlist-echo read timeout");
        let reader_stream = stream.try_clone().expect("clone playlist-echo stream");
        let mut reader = BufReader::new(reader_stream);
        stream
            .write_all(b"{\"Hello\":{\"username\":\"real-mpv-user\"}}\n")
            .expect("write client Hello");
        let mut server_hello = String::new();
        reader
            .read_line(&mut server_hello)
            .expect("read server Hello");
        assert_eq!(server_hello.trim(), REAL_MPV_SESSION_HELLO);
        (server, endpoint, stream, reader)
    }

    fn assert_playlist_echo_test_frame_rejected(
        label: &str,
        frame: serde_json::Value,
        expected_reason: &str,
    ) {
        let media_url = "http://127.0.0.1:49152/generated-fault.au";
        let (server, _, mut stream, _) = connect_playlist_echo_test_server(media_url);
        writeln!(stream, "{frame}")
            .unwrap_or_else(|error| panic!("{label} should write the rejected frame: {error}"));
        let receive_error = server
            .recv_playlist_exchange(Duration::from_secs(2), label)
            .expect_err("rejected frame must not produce playlist evidence");
        assert!(
            receive_error.contains("closed"),
            "{label} should close the fixture evidence sender: {receive_error}"
        );
        let release_error = server
            .release(label)
            .expect_err("rejected frame must fail the fixture");
        assert!(
            release_error.contains(expected_reason),
            "{label} had an unexpected rejection reason: {release_error}"
        );
    }

    #[test]
    fn playlist_echo_server_requires_and_records_exact_generated_url_exchange() {
        let media_url = "http://127.0.0.1:49152/generated-fault.au";
        let (server, endpoint, mut stream, mut reader) =
            connect_playlist_echo_test_server(media_url);
        stream
            .write_all(
                b"{\"Set\":{\"ready\":{\"isReady\":false,\"manuallyInitiated\":false}}}\n{\"State\":{\"ping\":{\"clientLatencyCalculation\":1.0,\"clientRtt\":0.0}}}\n{\"List\":null}\n{\"State\":{\"ping\":{}}}\n",
            )
            .expect("write exact known startup frames");
        let playlist_request = serde_json::json!({
            "Set": {
                "playlistChange": {
                    "files": [media_url],
                }
            }
        })
        .to_string();
        writeln!(stream, "{playlist_request}").expect("write exact playlist request");

        let mut playlist_change_echo = String::new();
        reader
            .read_line(&mut playlist_change_echo)
            .expect("read playlist change echo");
        let playlist_index_request = serde_json::json!({
            "Set": {
                "playlistIndex": {
                    "index": 0,
                }
            }
        })
        .to_string();
        writeln!(stream, "{playlist_index_request}").expect("write exact playlist index request");
        let mut playlist_index_echo = String::new();
        reader
            .read_line(&mut playlist_index_echo)
            .expect("read playlist index echo");
        let recorded = server
            .recv_playlist_exchange(Duration::from_secs(2), "unit playlist echo")
            .expect("record playlist exchange");
        assert_eq!(recorded.0, playlist_request);
        assert_eq!(recorded.1, playlist_change_echo.trim());
        assert_eq!(recorded.2, playlist_index_request);
        assert_eq!(recorded.3, playlist_index_echo.trim());
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&recorded.1)
                .expect("valid playlist change echo")
                .pointer("/Set/playlistChange/files/0")
                .and_then(serde_json::Value::as_str),
            Some(media_url)
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&recorded.2)
                .expect("valid playlist index request")
                .pointer("/Set/playlistIndex/index")
                .and_then(serde_json::Value::as_u64),
            Some(0)
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&recorded.3)
                .expect("valid playlist index echo")
                .pointer("/Set/playlistIndex/index")
                .and_then(serde_json::Value::as_u64),
            Some(0)
        );
        server
            .release("unit playlist echo")
            .expect("release fixture");
        let rebound =
            TcpListener::bind(&endpoint).expect("playlist-echo listener socket should be released");
        drop(rebound);
    }

    #[test]
    fn playlist_echo_server_rejects_arbitrary_valid_json_before_playlist_change() {
        let media_url = "http://127.0.0.1:49152/generated-fault.au";
        let (server, _, mut stream, _) = connect_playlist_echo_test_server(media_url);
        stream
            .write_all(b"{\"Future\":{\"accepted\":false}}\n")
            .expect("write arbitrary valid JSON");
        let receive_error = server
            .recv_playlist_exchange(Duration::from_secs(2), "arbitrary-frame rejection")
            .expect_err("arbitrary JSON must not produce playlist evidence");
        assert!(
            receive_error.contains("closed"),
            "fixture thread should close its evidence sender after rejection: {receive_error}"
        );
        let release_error = server
            .release("arbitrary-frame rejection")
            .expect_err("arbitrary valid JSON must fail the fixture");
        assert!(
            release_error.contains(
                "unexpected client frame before playlistChange (redacted shape: other-top-level)"
            ),
            "unexpected rejection reason: {release_error}"
        );
    }

    #[test]
    fn playlist_echo_server_rejects_widened_or_malformed_pre_media_heartbeats() {
        let cases = [
            (
                "extra heartbeat field",
                serde_json::json!({
                    "State": {
                        "ping": {
                            "clientLatencyCalculation": 1.0,
                            "clientRtt": 0.0,
                            "serverRtt": 0.0,
                        }
                    }
                }),
            ),
            (
                "missing heartbeat field",
                serde_json::json!({
                    "State": {
                        "ping": {
                            "clientLatencyCalculation": 1.0,
                        }
                    }
                }),
            ),
            (
                "wrong heartbeat field type",
                serde_json::json!({
                    "State": {
                        "ping": {
                            "clientLatencyCalculation": "not-a-number",
                            "clientRtt": 0.0,
                        }
                    }
                }),
            ),
        ];
        for (label, frame) in cases {
            assert_playlist_echo_test_frame_rejected(
                label,
                frame,
                "unexpected client frame before playlistChange (redacted shape: State.ping)",
            );
        }
    }

    #[test]
    fn playlist_echo_server_rejects_default_ready_publish_with_extra_fields() {
        let media_url = "http://127.0.0.1:49152/generated-fault.au";
        let (server, _, mut stream, _) = connect_playlist_echo_test_server(media_url);
        stream
            .write_all(
                b"{\"Set\":{\"ready\":{\"isReady\":false,\"manuallyInitiated\":false,\"unexpected\":true}}}\n",
            )
            .expect("write extra-field default ready publication");
        let receive_error = server
            .recv_playlist_exchange(Duration::from_secs(2), "ready extra-field rejection")
            .expect_err("extra-field default Ready must not produce playlist evidence");
        assert!(
            receive_error.contains("closed"),
            "fixture thread should close its evidence sender after rejection: {receive_error}"
        );
        let release_error = server
            .release("ready extra-field rejection")
            .expect_err("extra-field default Ready must fail the fixture");
        assert!(
            release_error.contains(
                "unexpected client frame before playlistChange (redacted shape: Set.ready)"
            ),
            "unexpected rejection reason: {release_error}"
        );
    }

    #[test]
    fn playlist_echo_server_rejects_playlist_change_with_extra_fields() {
        let media_url = "http://127.0.0.1:49152/generated-fault.au";
        let (server, _, mut stream, _) = connect_playlist_echo_test_server(media_url);
        let request = serde_json::json!({
            "Set": {
                "playlistChange": {
                    "files": [media_url],
                    "unexpected": true,
                }
            }
        });
        writeln!(stream, "{request}").expect("write extra-field playlist request");
        let receive_error = server
            .recv_playlist_exchange(Duration::from_secs(2), "extra-field rejection")
            .expect_err("extra-field playlistChange must not produce playlist evidence");
        assert!(
            receive_error.contains("closed"),
            "fixture thread should close its evidence sender after rejection: {receive_error}"
        );
        let release_error = server
            .release("extra-field rejection")
            .expect_err("extra-field playlistChange must fail the fixture");
        assert!(
            release_error.contains("did not match the exact closed request schema"),
            "unexpected rejection reason: {release_error}"
        );
    }

    #[test]
    fn playlist_echo_server_requires_exact_playlist_index_request_before_echo() {
        let media_url = "http://127.0.0.1:49152/generated-fault.au";
        let (server, _, mut stream, mut reader) = connect_playlist_echo_test_server(media_url);
        let playlist_request = serde_json::json!({
            "Set": {
                "playlistChange": {
                    "files": [media_url],
                }
            }
        });
        writeln!(stream, "{playlist_request}").expect("write exact playlist request");
        let mut playlist_change_echo = String::new();
        reader
            .read_line(&mut playlist_change_echo)
            .expect("read playlist change echo");
        let invalid_index_request = serde_json::json!({
            "Set": {
                "playlistIndex": {
                    "index": 0,
                    "unexpected": true,
                }
            }
        });
        writeln!(stream, "{invalid_index_request}")
            .expect("write extra-field playlist index request");

        let receive_error = server
            .recv_playlist_exchange(Duration::from_secs(2), "playlistIndex rejection")
            .expect_err("extra-field playlistIndex must not produce playlist evidence");
        assert!(
            receive_error.contains("closed"),
            "fixture thread should close its evidence sender after rejection: {receive_error}"
        );
        let release_error = server
            .release("playlistIndex rejection")
            .expect_err("extra-field playlistIndex must fail the fixture");
        assert!(
            release_error.contains("playlistIndex did not match the exact closed request schema"),
            "unexpected rejection reason: {release_error}"
        );
    }

    #[test]
    fn faulting_http_server_accounts_head_range_one_short_get_and_one_complete_get() {
        fn request(endpoint: &str, method: &str, range: Option<&str>) -> Vec<u8> {
            let mut stream = TcpStream::connect(endpoint).expect("connect loopback fixture");
            let range = range.map_or_else(String::new, |value| format!("Range: {value}\r\n"));
            stream
                .write_all(
                    format!(
                        "{method} {REAL_MPV_HTTP_FAULT_ROUTE} HTTP/1.1\r\nHost: {endpoint}\r\n{range}Connection: close\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .expect("write loopback request");
            let mut response = Vec::new();
            stream
                .read_to_end(&mut response)
                .expect("read loopback response");
            response
        }

        let generated_media = pcm_au_bytes(REAL_MPV_HTTP_FAULT_DURATION_SECONDS);
        let server =
            FaultingLoopbackHttpServer::start(generated_media.clone()).expect("start fixture");
        let endpoint = server.endpoint();
        require_ipv4_loopback_endpoint(&endpoint, "unit faulting HTTP listener")
            .expect("strict listener");

        let head = request(&endpoint, "HEAD", None);
        assert!(head.windows(15).any(|window| window == b"HTTP/1.1 200 OK"));
        assert!(
            head.ends_with(b"\r\n\r\n"),
            "HEAD response must not carry a body"
        );

        let short = request(&endpoint, "GET", Some("bytes=0-"));
        assert!(
            short.windows(15).any(|window| window == b"HTTP/1.1 200 OK"),
            "byte-zero GET must receive a non-seekable finite response"
        );
        let short_content_length = format!(
            "Content-Length: {}",
            REAL_MPV_HTTP_FAULT_DISCONNECT_AFTER_BYTES
        );
        assert!(
            short
                .windows(short_content_length.len())
                .any(|window| window == short_content_length.as_bytes()),
            "first GET must declare the exact clean short-body boundary"
        );
        assert!(
            !short
                .windows(b"Accept-Ranges:".len())
                .any(|window| window == b"Accept-Ranges:"),
            "non-seekable response must not advertise byte-range support"
        );
        assert!(
            !short
                .windows(b"Content-Range:".len())
                .any(|window| window == b"Content-Range:"),
            "non-seekable response must not advertise partial-content semantics"
        );
        let short_body = short
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|index| short.len() - index - 4)
            .expect("short response header boundary");
        assert_eq!(
            short_body, REAL_MPV_HTTP_FAULT_DISCONNECT_AFTER_BYTES,
            "first GET must close at the exact one-shot boundary"
        );

        let complete = request(&endpoint, "GET", Some("bytes=0-"));
        let complete_content_length = format!("Content-Length: {}", generated_media.len());
        assert!(
            complete
                .windows(complete_content_length.len())
                .any(|window| window == complete_content_length.as_bytes()),
            "recovery GET must declare the complete generated AU body"
        );
        let complete_body = complete
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|index| complete.len() - index - 4)
            .expect("complete response header boundary");
        assert_eq!(complete_body, generated_media.len());

        let requests = server
            .wait_for_media_gets(2, Duration::from_secs(8))
            .expect("two media GETs");
        validate_faulting_http_request_accounting(&requests, generated_media.len())
            .expect("strict request accounting");
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].method, "HEAD");
        assert_eq!(requests[1].range_header.as_deref(), Some("bytes=0-"));
        assert_eq!(requests[2].range_header.as_deref(), Some("bytes=0-"));
        server.release().expect("release faulting HTTP server");
        let rebound =
            TcpListener::bind(&endpoint).expect("faulting HTTP listener socket should be released");
        drop(rebound);
    }

    #[test]
    fn faulting_http_server_preserves_partial_get_evidence_when_client_aborts() {
        let generated_media = pcm_au_bytes(REAL_MPV_HTTP_FAULT_DURATION_SECONDS);
        let server =
            FaultingLoopbackHttpServer::start(generated_media.clone()).expect("start fixture");
        let endpoint = server.endpoint();
        let mut stream = TcpStream::connect(&endpoint).expect("connect loopback fixture");
        stream
            .write_all(
                format!(
                    "GET {REAL_MPV_HTTP_FAULT_ROUTE} HTTP/1.1\r\nHost: {endpoint}\r\nRange: bytes=0-\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .expect("write loopback request");
        let mut response = Vec::new();
        let mut buffer = [0_u8; 1024];
        while !response.windows(4).any(|window| window == b"\r\n\r\n") {
            let read = stream
                .read(&mut buffer)
                .expect("read loopback response headers");
            assert_ne!(read, 0, "fixture closed before response headers");
            response.extend_from_slice(&buffer[..read]);
        }
        stream
            .shutdown(Shutdown::Both)
            .expect("abort loopback response");
        drop(stream);

        let requests = server
            .wait_for_media_gets(1, Duration::from_secs(8))
            .expect("partial GET must remain accounted");
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.method, "GET");
        assert!(
            request.transmitted_body_bytes < REAL_MPV_HTTP_FAULT_DISCONNECT_AFTER_BYTES,
            "client abort must stop before the controlled short-response boundary: {request:?}"
        );
        assert!(request.disconnected_early);
        assert!(
            request.write_error.is_some(),
            "the partial request must retain the write failure that ended transmission: {request:?}"
        );
        validate_faulting_http_request_accounting(&requests, generated_media.len())
            .expect_err("partial transmission must still fail the strict campaign contract");
        server
            .release()
            .expect("partial response must not poison fixture release");
        let rebound =
            TcpListener::bind(&endpoint).expect("faulting HTTP listener socket should be released");
        drop(rebound);
    }
}
