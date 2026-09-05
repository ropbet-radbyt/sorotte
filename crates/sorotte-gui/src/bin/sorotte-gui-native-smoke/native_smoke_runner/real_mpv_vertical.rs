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
const REAL_MPV_MEDIA_FAILURE_KIND: &str = "sorotte-gui-real-mpv-media-failure-recovery";
const REAL_MPV_MEDIA_FAILURE_ROUTE: &str = "/hard-media-failure.au";
const REAL_MPV_HTTP_FAULT_DURATION_SECONDS: u32 = 45;
const REAL_MPV_HTTP_FAULT_MINIMUM_PREFIX_BYTES: usize = 720_000;
const REAL_MPV_HTTP_FAULT_BYTES_PER_SECOND: usize = 350_000;
const REAL_MPV_HTTP_STALL_KIND: &str = "sorotte-gui-real-mpv-stalled-http";
const REAL_MPV_HTTP_STALL_ROUTE: &str = "/generated-stall.au";
const REAL_MPV_HTTP_STALL_DURATION_SECONDS: u32 = 45;
const REAL_MPV_HTTP_STALL_PREFIX_BYTES: usize = 720_000;
const REAL_MPV_HTTP_STALL_BYTES_PER_SECOND: usize = 350_000;
const REAL_MPV_HTTP_STALL_MINIMUM_DURATION: Duration = Duration::from_secs(25);
const REAL_MPV_HTTP_STALL_MAXIMUM_RECOVERY_WAIT: Duration = Duration::from_secs(50);
const REAL_MPV_HTTP_STALL_REQUEST_DEADLINE: Duration = Duration::from_secs(3);
const REAL_MPV_HTTP_STALL_PREFIX_DEADLINE: Duration = Duration::from_secs(6);
const REAL_MPV_HTTP_STALL_COMPLETE_RESPONSE_DEADLINE: Duration = Duration::from_secs(5);
const REAL_MPV_HTTP_STALL_SOCKET_POLL: Duration = Duration::from_millis(250);
const REAL_MPV_HTTP_STALL_AU_HEADER_BYTES: usize = 24;
const REAL_MPV_HTTP_STALL_PCM_BYTES_PER_SECOND: usize = 48_000 * 2;
const REAL_MPV_HTTP_STALL_POSITION_TOLERANCE_SECONDS: f64 = 0.25;
const PLAY_CONTROL_AUTOMATION_ID: &str = "main-window:control:play";
const PAUSE_CONTROL_AUTOMATION_ID: &str = "main-window:control:pause";
// The room-intent surface may omit attribution while a local causal intent is
// waiting for canonical and physical confirmation to converge. The strict
// loopback exchange independently proves the authenticated request and server
// echo, so accessibility owns only the user-visible room level here.
const PAUSED_ROOM_INTENT_PREFIX: &str = "Room intent: PAUSED";
const PLAYING_ROOM_INTENT_PREFIX: &str = "Room intent: PLAYING";

fn real_mpv_http_stall_prefix_playable_seconds() -> f64 {
    REAL_MPV_HTTP_STALL_PREFIX_BYTES.saturating_sub(REAL_MPV_HTTP_STALL_AU_HEADER_BYTES) as f64
        / REAL_MPV_HTTP_STALL_PCM_BYTES_PER_SECOND as f64
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RealMpvVerticalOptions {
    binary_path: PathBuf,
    mpv_path: PathBuf,
    artifact_dir: PathBuf,
    timeout: Duration,
    exercise_recovery: bool,
    exercise_http_fault: bool,
    exercise_http_stall: bool,
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
    shared_lifecycle_path: String,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    media_failure: Option<MediaFailureRecoveryEvidence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    http_stall: Option<HttpStallEvidence>,
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
    content_length_header: Option<usize>,
    transfer_encoding: Option<String>,
    transmitted_body_bytes: usize,
    framing_fault_injected: bool,
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
    minimum_body_bytes_before_fault: usize,
    request_count: usize,
    premature_disconnect_count: usize,
    complete_response_count: usize,
    requests: Vec<HttpRequestEvidence>,
    initial_file_loaded_index: Option<usize>,
    pre_fault_progress_index: Option<usize>,
    fault_triggered_after_progress: bool,
    premature_eof_index: Option<usize>,
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
    premature_eof_position_seconds: Option<f64>,
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
            fault: "first-response-malformed-chunk-after-observed-progress-and-playable-prefix-once",
            recovery_mode: "same-generation-automatic-network-stream-reload",
            listener_endpoint,
            listener_ipv4_loopback: true,
            media_url,
            route: REAL_MPV_HTTP_FAULT_ROUTE,
            generated_media_bytes: generated_media.len(),
            generated_media_sha256: hex_sha256(generated_media),
            duration_seconds: REAL_MPV_HTTP_FAULT_DURATION_SECONDS,
            minimum_body_bytes_before_fault: REAL_MPV_HTTP_FAULT_MINIMUM_PREFIX_BYTES,
            request_count: 0,
            premature_disconnect_count: 0,
            complete_response_count: 0,
            requests: Vec::new(),
            initial_file_loaded_index: None,
            pre_fault_progress_index: None,
            fault_triggered_after_progress: false,
            premature_eof_index: None,
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
            premature_eof_position_seconds: None,
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
                request.method == "GET"
                    && !request.disconnected_early
                    && !request.framing_fault_injected
                    && request.content_length_header == Some(request.transmitted_body_bytes)
            })
            .count();
        self.requests = requests;
    }
}

#[derive(Debug, Clone, Serialize)]
struct MediaFailureRecoveryEvidence {
    schema_version: u32,
    kind: &'static str,
    result: String,
    failure_mode: &'static str,
    recovery_mode: &'static str,
    listener_endpoint: String,
    listener_ipv4_loopback: bool,
    media_url: String,
    route: &'static str,
    request_count: usize,
    requests: Vec<HttpRequestEvidence>,
    failure_end_file_index: Option<usize>,
    failure_reason: Option<String>,
    media_fail_event_id: Option<String>,
    media_fail_emitter: Option<String>,
    media_fail_process_role: Option<String>,
    restored_file_loaded_index: Option<usize>,
    media_playable_event_id: Option<String>,
    media_playable_emitter: Option<String>,
    media_playable_process_role: Option<String>,
    initial_pid: u32,
    failure_pid: Option<u32>,
    recovered_pid: Option<u32>,
    parent_pid: u32,
    process_image_path: String,
    process_sha256: String,
    initial_ipc_endpoint: String,
    failure_ipc_endpoint: Option<String>,
    recovered_ipc_endpoint: Option<String>,
    same_process_identity: bool,
    same_ipc_endpoint: bool,
    restored_media_path: String,
    restored_media_sha256: String,
    manual_retry_invoked: bool,
    evidence_retained_before_cleanup: bool,
    server_thread_released: bool,
    socket_released: bool,
    owned_mpv_terminated_after_gui_exit: bool,
    error: Option<String>,
}

struct MediaFailureRecoveryInit<'a> {
    listener_endpoint: String,
    media_url: String,
    initial_pid: u32,
    parent_pid: u32,
    process_image_path: &'a Path,
    process_sha256: String,
    initial_ipc_endpoint: String,
    restored_media_path: &'a Path,
    restored_media_sha256: String,
}

impl MediaFailureRecoveryEvidence {
    fn new(init: MediaFailureRecoveryInit<'_>) -> Self {
        Self {
            schema_version: REAL_MPV_SCHEMA_VERSION,
            kind: REAL_MPV_MEDIA_FAILURE_KIND,
            result: "running".to_owned(),
            failure_mode: "authoritative-loopback-http-404",
            recovery_mode: "authoritative-local-media-restore",
            listener_endpoint: init.listener_endpoint,
            listener_ipv4_loopback: true,
            media_url: init.media_url,
            route: REAL_MPV_MEDIA_FAILURE_ROUTE,
            request_count: 0,
            requests: Vec::new(),
            failure_end_file_index: None,
            failure_reason: None,
            media_fail_event_id: None,
            media_fail_emitter: None,
            media_fail_process_role: None,
            restored_file_loaded_index: None,
            media_playable_event_id: None,
            media_playable_emitter: None,
            media_playable_process_role: None,
            initial_pid: init.initial_pid,
            failure_pid: None,
            recovered_pid: None,
            parent_pid: init.parent_pid,
            process_image_path: init.process_image_path.display().to_string(),
            process_sha256: init.process_sha256,
            initial_ipc_endpoint: init.initial_ipc_endpoint,
            failure_ipc_endpoint: None,
            recovered_ipc_endpoint: None,
            same_process_identity: false,
            same_ipc_endpoint: false,
            restored_media_path: init.restored_media_path.display().to_string(),
            restored_media_sha256: init.restored_media_sha256,
            manual_retry_invoked: false,
            evidence_retained_before_cleanup: false,
            server_thread_released: false,
            socket_released: false,
            owned_mpv_terminated_after_gui_exit: false,
            error: None,
        }
    }

    fn record_requests(&mut self, requests: Vec<HttpRequestEvidence>) {
        self.request_count = requests.len();
        self.requests = requests;
    }
}

#[derive(Debug, Clone, Serialize)]
struct HttpStallRequestEvidence {
    ordinal: usize,
    method: String,
    path: String,
    peer_endpoint: String,
    peer_ipv4_loopback: bool,
    range_header: Option<String>,
    status_code: u16,
    content_length_header: Option<usize>,
    transfer_encoding: Option<String>,
    transmitted_body_bytes: usize,
    stall_injected: bool,
    stalled_for_ms: Option<u128>,
    server_response_retained_at_recovery_get: bool,
    connection_released: bool,
    response_completed: bool,
    write_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct HttpStallEvidence {
    schema_version: u32,
    kind: &'static str,
    result: String,
    schedule: &'static str,
    expected_outcome: &'static str,
    listener_endpoint: String,
    listener_ipv4_loopback: bool,
    media_url: String,
    route: &'static str,
    generated_media_bytes: usize,
    generated_media_sha256: String,
    duration_seconds: u32,
    prefix_body_bytes: usize,
    prefix_bytes_per_second: usize,
    expected_prefix_playable_seconds: f64,
    cache_stall_position_tolerance_seconds: f64,
    minimum_stall_duration_ms: u128,
    maximum_recovery_wait_ms: u128,
    request_count: usize,
    stalled_response_count: usize,
    complete_response_count: usize,
    requests: Vec<HttpStallRequestEvidence>,
    initial_file_loaded_index: Option<usize>,
    pre_stall_progress_index: Option<usize>,
    cache_stall_index: Option<usize>,
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
    pre_stall_position_seconds: Option<f64>,
    cache_stall_position_seconds: Option<f64>,
    recovered_position_seconds: Option<f64>,
    eof_observations_before_recovery: usize,
    end_file_observations_before_recovery: usize,
    manual_retry_invoked: bool,
    foreign_pid_observations_after_stall: usize,
    evidence_retained_before_cleanup: bool,
    server_thread_released: bool,
    socket_released: bool,
    owned_mpv_terminated_after_gui_exit: bool,
    error: Option<String>,
}

impl HttpStallEvidence {
    fn new(listener_endpoint: String, media_url: String, generated_media: &[u8]) -> Self {
        Self {
            schema_version: REAL_MPV_SCHEMA_VERSION,
            kind: REAL_MPV_HTTP_STALL_KIND,
            result: "running".to_owned(),
            schedule: "first-response-valid-prefix-then-open-byte-silence",
            expected_outcome: "one-bounded-same-generation-reload-after-sustained-cache-pause",
            listener_endpoint,
            listener_ipv4_loopback: true,
            media_url,
            route: REAL_MPV_HTTP_STALL_ROUTE,
            generated_media_bytes: generated_media.len(),
            generated_media_sha256: hex_sha256(generated_media),
            duration_seconds: REAL_MPV_HTTP_STALL_DURATION_SECONDS,
            prefix_body_bytes: REAL_MPV_HTTP_STALL_PREFIX_BYTES,
            prefix_bytes_per_second: REAL_MPV_HTTP_STALL_BYTES_PER_SECOND,
            expected_prefix_playable_seconds: real_mpv_http_stall_prefix_playable_seconds(),
            cache_stall_position_tolerance_seconds: REAL_MPV_HTTP_STALL_POSITION_TOLERANCE_SECONDS,
            minimum_stall_duration_ms: REAL_MPV_HTTP_STALL_MINIMUM_DURATION.as_millis(),
            maximum_recovery_wait_ms: REAL_MPV_HTTP_STALL_MAXIMUM_RECOVERY_WAIT.as_millis(),
            request_count: 0,
            stalled_response_count: 0,
            complete_response_count: 0,
            requests: Vec::new(),
            initial_file_loaded_index: None,
            pre_stall_progress_index: None,
            cache_stall_index: None,
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
            pre_stall_position_seconds: None,
            cache_stall_position_seconds: None,
            recovered_position_seconds: None,
            eof_observations_before_recovery: 0,
            end_file_observations_before_recovery: 0,
            manual_retry_invoked: false,
            foreign_pid_observations_after_stall: 0,
            evidence_retained_before_cleanup: false,
            server_thread_released: false,
            socket_released: false,
            owned_mpv_terminated_after_gui_exit: false,
            error: None,
        }
    }

    fn record_requests(&mut self, requests: Vec<HttpStallRequestEvidence>) {
        self.request_count = requests.len();
        self.stalled_response_count = requests
            .iter()
            .filter(|request| request.stall_injected)
            .count();
        self.complete_response_count = requests
            .iter()
            .filter(|request| request.method == "GET" && request.response_completed)
            .count();
        self.requests = requests;
    }
}

#[derive(Default)]
struct StalledHttpSharedState {
    requests: Vec<HttpStallRequestEvidence>,
    stall_started_at: Option<Instant>,
    recovery_request_seen_at: Option<Instant>,
    recovery_request_server_response_retained: Option<bool>,
}

struct StalledLoopbackHttpServer {
    address: SocketAddr,
    shutdown: Arc<AtomicBool>,
    state: Arc<Mutex<StalledHttpSharedState>>,
    join_handle: Option<thread::JoinHandle<Result<(), String>>>,
}

impl StalledLoopbackHttpServer {
    fn start(generated_media: Vec<u8>) -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|error| format!("failed to bind stalled HTTP loopback listener: {error}"))?;
        listener.set_nonblocking(true).map_err(|error| {
            format!("failed to make stalled HTTP listener nonblocking: {error}")
        })?;
        let address = listener
            .local_addr()
            .map_err(|error| format!("failed to inspect stalled HTTP listener: {error}"))?;
        if !address.is_ipv4() || !address.ip().is_loopback() || address.port() == 0 {
            return Err(format!(
                "stalled HTTP listener {address} was not strict nonzero IPv4 loopback"
            ));
        }

        let shutdown = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(StalledHttpSharedState::default()));
        let stalled_response_retained = Arc::new(AtomicBool::new(false));
        let thread_shutdown = Arc::clone(&shutdown);
        let thread_state = Arc::clone(&state);
        let thread_stalled_response_retained = Arc::clone(&stalled_response_retained);
        let generated_media = Arc::new(generated_media);
        let join_handle = thread::Builder::new()
            .name("sorotte-native-stalled-http".to_owned())
            .spawn(move || {
                let mut next_ordinal = 1_usize;
                let mut stall_spawned = false;
                let mut workers = Vec::new();
                while !thread_shutdown.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((mut stream, peer)) => {
                            if thread_shutdown.load(Ordering::Acquire) {
                                break;
                            }
                            let ordinal = next_ordinal;
                            next_ordinal = next_ordinal.saturating_add(1);
                            let request = read_stalled_http_request(
                                &mut stream,
                                peer,
                                ordinal,
                                &thread_shutdown,
                            )?;
                            if request.method == "GET" && !stall_spawned {
                                stall_spawned = true;
                                let worker_shutdown = Arc::clone(&thread_shutdown);
                                let worker_state = Arc::clone(&thread_state);
                                let worker_response_retained =
                                    Arc::clone(&thread_stalled_response_retained);
                                let worker_media = Arc::clone(&generated_media);
                                workers.push(
                                    thread::Builder::new()
                                        .name("sorotte-native-stalled-http-response".to_owned())
                                        .spawn(move || {
                                            serve_stalled_http_prefix(
                                                stream,
                                                request,
                                                worker_media.as_slice(),
                                                &worker_shutdown,
                                                &worker_state,
                                                &worker_response_retained,
                                            )
                                        })
                                        .map_err(|error| {
                                            format!(
                                                "failed to spawn stalled HTTP response thread: {error}"
                                            )
                                        })?,
                                );
                            } else {
                                if request.method == "GET" {
                                    note_stalled_http_recovery_request(
                                        &thread_state,
                                        &thread_stalled_response_retained,
                                    )?;
                                }
                                let record = serve_complete_stalled_http_response(
                                    stream,
                                    request,
                                    generated_media.as_slice(),
                                    &thread_shutdown,
                                )?;
                                record_stalled_http_request(&thread_state, record)?;
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                        }
                        Err(error) => {
                            return Err(format!("stalled HTTP listener accept failed: {error}"));
                        }
                    }
                }
                for worker in workers {
                    worker
                        .join()
                        .map_err(|_| "stalled HTTP response thread panicked".to_owned())??;
                }
                Ok(())
            })
            .map_err(|error| format!("failed to spawn stalled HTTP server thread: {error}"))?;
        Ok(Self {
            address,
            shutdown,
            state,
            join_handle: Some(join_handle),
        })
    }

    fn endpoint(&self) -> String {
        self.address.to_string()
    }

    fn url(&self) -> String {
        format!("http://{}{}", self.address, REAL_MPV_HTTP_STALL_ROUTE)
    }

    fn requests(&self) -> Result<Vec<HttpStallRequestEvidence>, String> {
        self.state
            .lock()
            .map(|state| state.requests.clone())
            .map_err(|_| "stalled HTTP request state was poisoned".to_owned())
    }

    fn stall_elapsed(&self) -> Result<Option<Duration>, String> {
        self.state
            .lock()
            .map(|state| {
                state
                    .stall_started_at
                    .map(|started| Instant::now().saturating_duration_since(started))
            })
            .map_err(|_| "stalled HTTP request state was poisoned".to_owned())
    }

    fn wait_for_media_gets(
        &self,
        expected: usize,
        timeout: Duration,
    ) -> Result<Vec<HttpStallRequestEvidence>, String> {
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
                    "timed out waiting for {expected} stalled HTTP media GETs; requests={requests:?}"
                ));
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn release(mut self) -> Result<Vec<HttpStallRequestEvidence>, String> {
        self.shutdown.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.address);
        if let Some(join_handle) = self.join_handle.take() {
            join_handle
                .join()
                .map_err(|_| "stalled HTTP server thread panicked".to_owned())??;
        }
        let requests = self.requests()?;
        let rebound = TcpListener::bind(self.address).map_err(|error| {
            format!(
                "stalled HTTP listener endpoint {} could not be rebound after release: {error}",
                self.address
            )
        })?;
        drop(rebound);
        Ok(requests)
    }
}

impl Drop for StalledLoopbackHttpServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.address);
        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
    }
}

fn read_stalled_http_request(
    stream: &mut TcpStream,
    peer: SocketAddr,
    ordinal: usize,
    shutdown: &AtomicBool,
) -> Result<HttpStallRequestEvidence, String> {
    if !peer.is_ipv4() || !peer.ip().is_loopback() || peer.port() == 0 {
        return Err(format!(
            "stalled HTTP peer {peer} was not strict nonzero IPv4 loopback"
        ));
    }
    stream
        .set_nonblocking(false)
        .map_err(|error| format!("failed making stalled HTTP connection blocking: {error}"))?;
    stream
        .set_read_timeout(Some(REAL_MPV_HTTP_STALL_SOCKET_POLL))
        .map_err(|error| format!("failed setting stalled HTTP read timeout: {error}"))?;
    stream
        .set_write_timeout(Some(REAL_MPV_HTTP_STALL_SOCKET_POLL))
        .map_err(|error| format!("failed setting stalled HTTP write timeout: {error}"))?;

    let mut request_bytes = Vec::new();
    let mut buffer = [0_u8; 1024];
    let deadline = Instant::now() + REAL_MPV_HTTP_STALL_REQUEST_DEADLINE;
    while !request_bytes.windows(4).any(|window| window == b"\r\n\r\n") {
        if shutdown.load(Ordering::Acquire) {
            return Err("stalled HTTP server released while reading request headers".to_owned());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "stalled HTTP request headers exceeded the {} ms absolute deadline",
                REAL_MPV_HTTP_STALL_REQUEST_DEADLINE.as_millis()
            ));
        }
        let read = match stream.read(&mut buffer) {
            Ok(read) => read,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(error) => {
                return Err(format!("failed reading stalled HTTP request: {error}"));
            }
        };
        if read == 0 {
            return Err("stalled HTTP peer closed before complete headers".to_owned());
        }
        request_bytes.extend_from_slice(&buffer[..read]);
        if request_bytes.len() > 16 * 1024 {
            return Err("stalled HTTP request headers exceeded 16 KiB".to_owned());
        }
    }
    let request = std::str::from_utf8(&request_bytes)
        .map_err(|error| format!("stalled HTTP request headers were not UTF-8: {error}"))?;
    let mut lines = request.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| "stalled HTTP request line was absent".to_owned())?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().unwrap_or_default().to_owned();
    let path = request_parts.next().unwrap_or_default().to_owned();
    let version = request_parts.next().unwrap_or_default();
    if !matches!(method.as_str(), "GET" | "HEAD")
        || path != REAL_MPV_HTTP_STALL_ROUTE
        || !matches!(version, "HTTP/1.0" | "HTTP/1.1")
    {
        return Err(format!(
            "stalled HTTP received unexpected request line {request_line:?}"
        ));
    }
    let range_header = lines.find_map(|line| {
        line.split_once(':').and_then(|(name, value)| {
            name.eq_ignore_ascii_case("range")
                .then(|| value.trim().to_owned())
        })
    });
    Ok(HttpStallRequestEvidence {
        ordinal,
        method,
        path,
        peer_endpoint: peer.to_string(),
        peer_ipv4_loopback: true,
        range_header,
        status_code: 200,
        content_length_header: None,
        transfer_encoding: None,
        transmitted_body_bytes: 0,
        stall_injected: false,
        stalled_for_ms: None,
        server_response_retained_at_recovery_get: false,
        connection_released: false,
        response_completed: false,
        write_error: None,
    })
}

fn record_stalled_http_request(
    state: &Mutex<StalledHttpSharedState>,
    record: HttpStallRequestEvidence,
) -> Result<(), String> {
    let mut state = state
        .lock()
        .map_err(|_| "stalled HTTP request state was poisoned".to_owned())?;
    state.requests.push(record);
    state.requests.sort_by_key(|request| request.ordinal);
    Ok(())
}

fn note_stalled_http_recovery_request(
    state: &Mutex<StalledHttpSharedState>,
    stalled_response_retained: &AtomicBool,
) -> Result<(), String> {
    let now = Instant::now();
    let mut state = state
        .lock()
        .map_err(|_| "stalled HTTP request state was poisoned".to_owned())?;
    if state.recovery_request_seen_at.is_some() {
        return Ok(());
    }
    state.recovery_request_seen_at = Some(now);
    state.recovery_request_server_response_retained =
        Some(stalled_response_retained.load(Ordering::Acquire));
    let stall_started_at = state.stall_started_at;
    if let Some(first) = state
        .requests
        .iter_mut()
        .find(|request| request.stall_injected)
    {
        first.stalled_for_ms =
            stall_started_at.map(|started| now.saturating_duration_since(started).as_millis());
        first.server_response_retained_at_recovery_get =
            stalled_response_retained.load(Ordering::Acquire);
    }
    Ok(())
}

fn write_stalled_http_bytes_before_deadline(
    stream: &mut TcpStream,
    bytes: &[u8],
    deadline: Instant,
    shutdown: &AtomicBool,
    label: &str,
) -> Result<usize, String> {
    let mut sent = 0;
    while sent < bytes.len() {
        if shutdown.load(Ordering::Acquire) {
            return Err(format!(
                "stalled HTTP server released while writing {label}"
            ));
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "stalled HTTP {label} exceeded its absolute deadline"
            ));
        }
        let next = (sent + 64 * 1024).min(bytes.len());
        match stream.write(&bytes[sent..next]) {
            Ok(0) => {
                return Err(format!("stalled HTTP {label} write made no progress"));
            }
            Ok(written) => sent += written,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(error) => {
                return Err(format!("failed writing stalled HTTP {label}: {error}"));
            }
        }
    }
    Ok(sent)
}

fn flush_stalled_http_before_deadline(
    stream: &mut TcpStream,
    deadline: Instant,
    shutdown: &AtomicBool,
    label: &str,
) -> Result<(), String> {
    loop {
        if shutdown.load(Ordering::Acquire) {
            return Err(format!(
                "stalled HTTP server released while flushing {label}"
            ));
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "stalled HTTP {label} flush exceeded its absolute deadline"
            ));
        }
        match stream.flush() {
            Ok(()) => return Ok(()),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(error) => {
                return Err(format!("failed flushing stalled HTTP {label}: {error}"));
            }
        }
    }
}

fn serve_stalled_http_prefix(
    mut stream: TcpStream,
    mut evidence: HttpStallRequestEvidence,
    generated_media: &[u8],
    shutdown: &AtomicBool,
    state: &Mutex<StalledHttpSharedState>,
    stalled_response_retained: &AtomicBool,
) -> Result<(), String> {
    let operation_deadline = Instant::now() + REAL_MPV_HTTP_STALL_PREFIX_DEADLINE;
    let headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: audio/basic\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        generated_media.len()
    );
    evidence.content_length_header = Some(generated_media.len());
    evidence.stall_injected = true;
    if let Err(error) = write_stalled_http_bytes_before_deadline(
        &mut stream,
        headers.as_bytes(),
        operation_deadline,
        shutdown,
        "stalled response headers",
    ) {
        evidence.write_error = Some(error);
    }
    let target = REAL_MPV_HTTP_STALL_PREFIX_BYTES.min(generated_media.len());
    let started = Instant::now();
    let mut sent = 0;
    while evidence.write_error.is_none() && sent < target && !shutdown.load(Ordering::Acquire) {
        if Instant::now() >= operation_deadline {
            evidence.write_error = Some(format!(
                "stalled HTTP prefix exceeded the {} ms absolute deadline",
                REAL_MPV_HTTP_STALL_PREFIX_DEADLINE.as_millis()
            ));
            break;
        }
        let next = (sent + 16 * 1024).min(target);
        match stream.write(&generated_media[sent..next]) {
            Ok(0) => {
                evidence.write_error =
                    Some("stalled HTTP prefix write made no progress".to_owned());
            }
            Ok(written) => sent += written,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => {
                evidence.write_error = Some(format!("failed writing stalled HTTP prefix: {error}"));
            }
        }
        let target_elapsed =
            Duration::from_secs_f64(sent as f64 / REAL_MPV_HTTP_STALL_BYTES_PER_SECOND as f64);
        if let Some(delay) = target_elapsed.checked_sub(started.elapsed()) {
            let remaining = operation_deadline.saturating_duration_since(Instant::now());
            thread::sleep(delay.min(remaining));
        }
    }
    if shutdown.load(Ordering::Acquire) && evidence.write_error.is_none() && sent < target {
        evidence.write_error =
            Some("stalled HTTP server released before the playable prefix completed".to_owned());
    }
    if evidence.write_error.is_none()
        && let Err(error) =
            flush_stalled_http_before_deadline(&mut stream, operation_deadline, shutdown, "prefix")
    {
        evidence.write_error = Some(error);
    }
    evidence.transmitted_body_bytes = sent;
    if evidence.write_error.is_none() && sent == target {
        let stall_started_at = Instant::now();
        stalled_response_retained.store(true, Ordering::Release);
        let mut locked = state
            .lock()
            .map_err(|_| "stalled HTTP request state was poisoned".to_owned())?;
        locked.stall_started_at = Some(stall_started_at);
        if let Some(recovery_seen_at) = locked.recovery_request_seen_at {
            evidence.stalled_for_ms = Some(
                recovery_seen_at
                    .saturating_duration_since(stall_started_at)
                    .as_millis(),
            );
            evidence.server_response_retained_at_recovery_get = recovery_seen_at
                >= stall_started_at
                && locked.recovery_request_server_response_retained == Some(true);
        }
        locked.requests.push(evidence.clone());
        locked.requests.sort_by_key(|request| request.ordinal);
        drop(locked);
        while !shutdown.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(10));
        }
    } else {
        evidence.connection_released = true;
        record_stalled_http_request(state, evidence.clone())?;
    }
    stalled_response_retained.store(false, Ordering::Release);
    let _ = stream.shutdown(Shutdown::Both);
    let mut locked = state
        .lock()
        .map_err(|_| "stalled HTTP request state was poisoned".to_owned())?;
    let stall_started_at = locked.stall_started_at;
    if let Some(first) = locked
        .requests
        .iter_mut()
        .find(|request| request.ordinal == evidence.ordinal)
    {
        if first.stalled_for_ms.is_none() {
            first.stalled_for_ms = stall_started_at.map(|started| {
                Instant::now()
                    .saturating_duration_since(started)
                    .as_millis()
            });
        }
        first.connection_released = true;
    }
    Ok(())
}

fn serve_complete_stalled_http_response(
    mut stream: TcpStream,
    mut evidence: HttpStallRequestEvidence,
    generated_media: &[u8],
    shutdown: &AtomicBool,
) -> Result<HttpStallRequestEvidence, String> {
    let operation_deadline = Instant::now() + REAL_MPV_HTTP_STALL_COMPLETE_RESPONSE_DEADLINE;
    let headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: audio/basic\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        generated_media.len()
    );
    evidence.content_length_header = Some(generated_media.len());
    if let Err(error) = write_stalled_http_bytes_before_deadline(
        &mut stream,
        headers.as_bytes(),
        operation_deadline,
        shutdown,
        "complete response headers",
    ) {
        evidence.write_error = Some(error);
    }
    let mut sent = 0;
    if evidence.method == "GET" {
        while evidence.write_error.is_none() && sent < generated_media.len() {
            if shutdown.load(Ordering::Acquire) {
                evidence.write_error =
                    Some("stalled HTTP server released during complete response".to_owned());
                break;
            }
            if Instant::now() >= operation_deadline {
                evidence.write_error = Some(format!(
                    "complete stalled HTTP response exceeded the {} ms absolute deadline",
                    REAL_MPV_HTTP_STALL_COMPLETE_RESPONSE_DEADLINE.as_millis()
                ));
                break;
            }
            let next = (sent + 64 * 1024).min(generated_media.len());
            match stream.write(&generated_media[sent..next]) {
                Ok(0) => {
                    evidence.write_error =
                        Some("complete stalled HTTP write made no progress".to_owned());
                }
                Ok(written) => sent += written,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(error) => {
                    evidence.write_error = Some(format!(
                        "failed writing complete stalled HTTP body: {error}"
                    ));
                }
            }
        }
        if evidence.write_error.is_none() && Instant::now() >= operation_deadline {
            evidence.write_error = Some(format!(
                "complete stalled HTTP flush exceeded the {} ms absolute deadline",
                REAL_MPV_HTTP_STALL_COMPLETE_RESPONSE_DEADLINE.as_millis()
            ));
        } else if evidence.write_error.is_none()
            && let Err(error) = flush_stalled_http_before_deadline(
                &mut stream,
                operation_deadline,
                shutdown,
                "complete response",
            )
        {
            evidence.write_error = Some(error);
        }
    }
    evidence.transmitted_body_bytes = sent;
    evidence.response_completed = evidence.write_error.is_none()
        && (evidence.method == "HEAD" || sent == generated_media.len());
    evidence.connection_released = true;
    let _ = stream.shutdown(Shutdown::Both);
    Ok(evidence)
}

struct FaultingLoopbackHttpServer {
    address: SocketAddr,
    shutdown: Arc<AtomicBool>,
    fault_trigger: Arc<AtomicBool>,
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
        let fault_trigger = Arc::new(AtomicBool::new(false));
        let thread_fault_trigger = Arc::clone(&fault_trigger);
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
                                &thread_fault_trigger,
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
            fault_trigger,
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

    fn trigger_fault(&self) -> Result<(), String> {
        self.fault_trigger
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| "faulting HTTP disconnect trigger was applied more than once".to_owned())
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
    fault_trigger: &AtomicBool,
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
    let content_length_header = if inject_fault {
        None
    } else {
        Some(generated_media.len())
    };
    let transfer_encoding = inject_fault.then(|| "chunked".to_owned());
    let headers = if inject_fault {
        "HTTP/1.1 200 OK\r\nContent-Type: audio/basic\r\nTransfer-Encoding: chunked\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n"
            .to_owned()
    } else {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: audio/basic\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
            generated_media.len()
        )
    };
    let mut evidence = HttpRequestEvidence {
        ordinal,
        method: method.clone(),
        path,
        peer_endpoint: peer.to_string(),
        peer_ipv4_loopback: true,
        range_header,
        status_code: 200,
        content_length_header,
        transfer_encoding,
        transmitted_body_bytes: 0,
        framing_fault_injected: inject_fault,
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
        let minimum_prefix = REAL_MPV_HTTP_FAULT_MINIMUM_PREFIX_BYTES.min(generated_media.len());
        let started = Instant::now();
        let mut sent = 0;
        while sent < generated_media.len()
            && !shutdown.load(Ordering::Acquire)
            && (sent < minimum_prefix || !fault_trigger.load(Ordering::Acquire))
        {
            let next = (sent + 16 * 1024).min(generated_media.len());
            let chunk = &generated_media[sent..next];
            let chunk_header = format!("{:x}\r\n", chunk.len());
            if let Err(error) = stream
                .write_all(chunk_header.as_bytes())
                .and_then(|()| stream.write_all(chunk))
                .and_then(|()| stream.write_all(b"\r\n"))
            {
                evidence.write_error = Some(format!(
                    "failed writing faulting HTTP chunked body: {error}"
                ));
                break;
            }
            sent = next;
            let target_elapsed =
                Duration::from_secs_f64(sent as f64 / REAL_MPV_HTTP_FAULT_BYTES_PER_SECOND as f64);
            if let Some(delay) = target_elapsed.checked_sub(started.elapsed()) {
                thread::sleep(delay);
            }
        }
        while evidence.write_error.is_none()
            && !fault_trigger.load(Ordering::Acquire)
            && !shutdown.load(Ordering::Acquire)
        {
            thread::sleep(Duration::from_millis(2));
        }
        if evidence.write_error.is_none() && !shutdown.load(Ordering::Acquire) {
            if let Err(error) = stream.write_all(b"not-a-chunk-size\r\n") {
                evidence.write_error = Some(format!(
                    "failed writing malformed HTTP chunk boundary: {error}"
                ));
            } else if let Err(error) = stream.flush() {
                evidence.write_error = Some(format!(
                    "failed flushing malformed HTTP chunk boundary: {error}"
                ));
            }
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
    evidence.disconnected_early = method == "GET"
        && (evidence.framing_fault_injected || transmitted_body_bytes < generated_media.len());
    Ok(evidence)
}

struct HardFailureLoopbackHttpServer {
    address: SocketAddr,
    shutdown: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<HttpRequestEvidence>>>,
    join_handle: Option<thread::JoinHandle<Result<(), String>>>,
}

impl HardFailureLoopbackHttpServer {
    fn start() -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| {
            format!("failed to bind hard-failure HTTP loopback listener: {error}")
        })?;
        listener.set_nonblocking(true).map_err(|error| {
            format!("failed to make hard-failure HTTP listener nonblocking: {error}")
        })?;
        let address = listener
            .local_addr()
            .map_err(|error| format!("failed to inspect hard-failure HTTP listener: {error}"))?;
        if !address.is_ipv4() || !address.ip().is_loopback() || address.port() == 0 {
            return Err(format!(
                "hard-failure HTTP listener {address} was not strict nonzero IPv4 loopback"
            ));
        }

        let shutdown = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let thread_shutdown = Arc::clone(&shutdown);
        let thread_requests = Arc::clone(&requests);
        let join_handle = thread::Builder::new()
            .name("sorotte-native-hard-failure-http".to_owned())
            .spawn(move || {
                while !thread_shutdown.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, peer)) => {
                            if thread_shutdown.load(Ordering::Acquire) {
                                break;
                            }
                            let ordinal = thread_requests
                                .lock()
                                .map_err(|_| {
                                    "hard-failure HTTP request log was poisoned".to_owned()
                                })?
                                .len()
                                .saturating_add(1);
                            let record =
                                handle_hard_failure_http_connection(stream, peer, ordinal)?;
                            thread_requests
                                .lock()
                                .map_err(|_| {
                                    "hard-failure HTTP request log was poisoned".to_owned()
                                })?
                                .push(record);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                        }
                        Err(error) => {
                            return Err(format!(
                                "hard-failure HTTP listener accept failed: {error}"
                            ));
                        }
                    }
                }
                Ok(())
            })
            .map_err(|error| format!("failed to spawn hard-failure HTTP server thread: {error}"))?;
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
        format!("http://{}{}", self.address, REAL_MPV_MEDIA_FAILURE_ROUTE)
    }

    fn requests(&self) -> Result<Vec<HttpRequestEvidence>, String> {
        self.requests
            .lock()
            .map(|requests| requests.clone())
            .map_err(|_| "hard-failure HTTP request log was poisoned".to_owned())
    }

    fn wait_for_media_get(&self, timeout: Duration) -> Result<Vec<HttpRequestEvidence>, String> {
        let deadline = Instant::now() + timeout;
        loop {
            let requests = self.requests()?;
            if requests.iter().any(|request| request.method == "GET") {
                return Ok(requests);
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "timed out waiting for hard-failure HTTP media GET; requests={requests:?}"
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
                .map_err(|_| "hard-failure HTTP server thread panicked".to_owned())??;
        }
        let requests = self.requests()?;
        let rebound = TcpListener::bind(self.address).map_err(|error| {
            format!(
                "hard-failure HTTP listener endpoint {} could not be rebound after release: {error}",
                self.address
            )
        })?;
        drop(rebound);
        Ok(requests)
    }
}

impl Drop for HardFailureLoopbackHttpServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.address);
        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
    }
}

fn handle_hard_failure_http_connection(
    mut stream: TcpStream,
    peer: SocketAddr,
    ordinal: usize,
) -> Result<HttpRequestEvidence, String> {
    if !peer.is_ipv4() || !peer.ip().is_loopback() || peer.port() == 0 {
        return Err(format!(
            "hard-failure HTTP peer {peer} was not strict nonzero IPv4 loopback"
        ));
    }
    stream
        .set_nonblocking(false)
        .map_err(|error| format!("failed setting hard-failure HTTP blocking mode: {error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|error| format!("failed setting hard-failure HTTP read timeout: {error}"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .map_err(|error| format!("failed setting hard-failure HTTP write timeout: {error}"))?;

    let mut request_bytes = Vec::new();
    let mut buffer = [0_u8; 1024];
    while !request_bytes.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream
            .read(&mut buffer)
            .map_err(|error| format!("failed reading hard-failure HTTP request: {error}"))?;
        if read == 0 {
            return Err("hard-failure HTTP peer closed before complete headers".to_owned());
        }
        request_bytes.extend_from_slice(&buffer[..read]);
        if request_bytes.len() > 16 * 1024 {
            return Err("hard-failure HTTP request headers exceeded 16 KiB".to_owned());
        }
    }
    let request = std::str::from_utf8(&request_bytes)
        .map_err(|error| format!("hard-failure HTTP request headers were not UTF-8: {error}"))?;
    let mut lines = request.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| "hard-failure HTTP request line was absent".to_owned())?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().unwrap_or_default().to_owned();
    let path = request_parts.next().unwrap_or_default().to_owned();
    let version = request_parts.next().unwrap_or_default();
    if !matches!(method.as_str(), "GET" | "HEAD")
        || path != REAL_MPV_MEDIA_FAILURE_ROUTE
        || !matches!(version, "HTTP/1.0" | "HTTP/1.1")
        || request_parts.next().is_some()
    {
        return Err(format!(
            "hard-failure HTTP received unexpected request line {request_line:?}"
        ));
    }
    let range_header = lines.find_map(|line| {
        line.split_once(':').and_then(|(name, value)| {
            name.eq_ignore_ascii_case("range")
                .then(|| value.trim().to_owned())
        })
    });
    let mut evidence = HttpRequestEvidence {
        ordinal,
        method,
        path,
        peer_endpoint: peer.to_string(),
        peer_ipv4_loopback: true,
        range_header,
        status_code: 404,
        content_length_header: Some(0),
        transfer_encoding: None,
        transmitted_body_bytes: 0,
        framing_fault_injected: false,
        disconnected_early: false,
        write_error: None,
    };
    if let Err(error) = stream
        .write_all(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        )
        .and_then(|()| stream.flush())
    {
        evidence.write_error = Some(format!(
            "failed writing hard-failure HTTP 404 response: {error}"
        ));
    }
    let _ = stream.shutdown(Shutdown::Both);
    Ok(evidence)
}

#[derive(Debug, Clone, Serialize)]
struct MissingMediaEvidence {
    path: String,
    event_id: String,
    emitter: String,
    process_role: String,
    initial_pid: u32,
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
    missing_media: Option<MissingMediaEvidence>,
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
            missing_media: None,
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
    paused_for_cache: Option<bool>,
    eof_reached: Option<bool>,
    ipc_endpoint: Option<String>,
    reason: Option<String>,
}

#[derive(Debug)]
struct MpvPreflight {
    identity: BinaryIdentity,
    version: String,
}

#[derive(Debug, Serialize)]
struct PlaystateExchangeEvidence {
    action: &'static str,
    mutation_kind: &'static str,
    expected_paused: bool,
    request: String,
    authoritative_echo: String,
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
    initial_authoritative_playstate: Option<String>,
    playstate_exchanges: Vec<PlaystateExchangeEvidence>,
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
            initial_authoritative_playstate: None,
            playstate_exchanges: Vec::new(),
            server_thread_released: false,
            socket_released: false,
            error: None,
        }
    }
}

fn record_authoritative_playstate_exchange(
    server: &MockSessionServer,
    evidence: &mut SessionExchangeEvidence,
    evidence_path: &Path,
    timeout: Duration,
    action: &'static str,
    expected_paused: bool,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(format!(
                "timed out waiting for {action} after authenticated interleaved seek exchanges"
            ));
        }
        let (request, authoritative_echo) = server.recv_playstate_exchange(remaining, action)?;
        let request_json: serde_json::Value = serde_json::from_str(&request)
            .map_err(|error| format!("{action} client playstate was invalid JSON: {error}"))?;
        let echo_json: serde_json::Value =
            serde_json::from_str(&authoritative_echo).map_err(|error| {
                format!("{action} authoritative playstate was invalid JSON: {error}")
            })?;
        let request_playstate = request_json
            .pointer("/State/playstate")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| format!("{action} client frame omitted State.playstate"))?;
        let echo_playstate = echo_json
            .pointer("/State/playstate")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| format!("{action} server echo omitted State.playstate"))?;
        let request_position = request_playstate
            .get("position")
            .and_then(serde_json::Value::as_f64)
            .filter(|position| position.is_finite() && *position >= 0.0)
            .ok_or_else(|| format!("{action} client frame omitted a valid position"))?;
        let observed_paused = request_playstate
            .get("paused")
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| format!("{action} client frame omitted a boolean paused state"))?;
        let observed_do_seek = request_playstate
            .get("doSeek")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if request_playstate.contains_key("setBy") {
            return Err(format!(
                "{action} client playstate improperly claimed server attribution"
            ));
        }
        let expected_counter_ack = request_json
            .pointer("/State/ignoringOnTheFly/client")
            .filter(|counter| counter.as_u64().is_some_and(|counter| counter != 0))
            .map(|counter| serde_json::json!({"client": counter}));
        if echo_json.pointer("/State/ignoringOnTheFly") != expected_counter_ack.as_ref() {
            return Err(format!(
                "{action} authoritative echo did not acknowledge the exact client counter"
            ));
        }
        if echo_playstate
            .get("paused")
            .and_then(serde_json::Value::as_bool)
            != Some(observed_paused)
            || echo_playstate
                .get("doSeek")
                .and_then(serde_json::Value::as_bool)
                != Some(observed_do_seek)
            || echo_playstate
                .get("setBy")
                .and_then(serde_json::Value::as_str)
                != Some(REAL_MPV_LOOPBACK_USERNAME)
            || echo_playstate
                .get("position")
                .and_then(serde_json::Value::as_f64)
                .is_none_or(|position| (position - request_position).abs() > f64::EPSILON)
            || echo_playstate.get("sorotteTransportRevision")
                != request_playstate.get("sorotteTransportRevision")
        {
            return Err(format!(
                "{action} authoritative echo did not preserve and authenticate the client mutation"
            ));
        }

        let mutation_kind = if observed_do_seek { "seek" } else { "pause" };
        evidence
            .playstate_exchanges
            .push(PlaystateExchangeEvidence {
                action,
                mutation_kind,
                expected_paused: observed_paused,
                request,
                authoritative_echo,
            });
        write_json_file(evidence_path, evidence)?;

        if observed_do_seek {
            continue;
        }
        if observed_paused != expected_paused {
            return Err(format!(
                "{action} client pause mutation had paused={observed_paused}, expected {expected_paused}"
            ));
        }
        return Ok(());
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
    let media_failure_path = artifact_root.join("hard-media-failure.json");
    let http_stall_path = artifact_root.join("stalled-http.json");
    let mut state = RealMpvVerticalState::new(&artifact_root);
    write_json_file(&state_path, &state)?;
    let mut menu_interactions = MenuInteractionsEvidence::new();
    write_json_file(&menu_interactions_path, &menu_interactions)?;

    let started_at = Instant::now();
    let driver = PlatformNativeGuiDriver::default();
    let mut child: Option<Child> = None;
    let mut window = None;
    let mut verified_mpv_pids = Vec::new();
    let mut session_server: Option<MockSessionServer> = None;
    let mut session_exchange: Option<SessionExchangeEvidence> = None;
    let mut recovery_evidence: Option<MpvRecoveryEvidence> = None;
    let mut fault_http_server: Option<FaultingLoopbackHttpServer> = None;
    let mut http_fault_evidence: Option<HttpFaultRecoveryEvidence> = None;
    let mut hard_failure_http_server: Option<HardFailureLoopbackHttpServer> = None;
    let mut media_failure_evidence: Option<MediaFailureRecoveryEvidence> = None;
    let mut stalled_http_server: Option<StalledLoopbackHttpServer> = None;
    let mut http_stall_evidence: Option<HttpStallEvidence> = None;

    let run_result = (|| -> Result<RealMpvVerticalReport, String> {
        require_real_mpv_vertical_platform()?;
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

        let exercise_http = options.exercise_http_fault || options.exercise_http_stall;
        let config_path = artifact_root.join("sorotte-real-mpv.ini");
        let appdata_root = artifact_root.join("appdata");
        let media_path = artifact_root.join(if exercise_http {
            "generated-silence.au"
        } else {
            "generated-silence.wav"
        });
        let observation_script_path = artifact_root.join("observe-real-mpv.lua");
        let observation_path = artifact_root.join("mpv-observation.jsonl");
        let mpv_log_path = artifact_root.join("mpv.log");
        let lifecycle_path = artifact_root.join("gui-lifecycle.jsonl");
        let shared_lifecycle_path = artifact_root.join("shared-lifecycle-evidence.jsonl");
        let shared_lifecycle_run_id = format!("gui-real-mpv-{}", unique_suffix());
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
        } else if options.exercise_http_stall {
            REAL_MPV_HTTP_STALL_DURATION_SECONDS
        } else {
            REAL_MPV_MEDIA_DURATION_SECONDS
        };
        let generated_media = if exercise_http {
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
        let mut additional_trusted_media_urls = Vec::new();
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
            let hard_failure_server = HardFailureLoopbackHttpServer::start()?;
            let hard_failure_endpoint = hard_failure_server.endpoint();
            require_ipv4_loopback_endpoint(
                &hard_failure_endpoint,
                "hard media-failure HTTP listener",
            )?;
            additional_trusted_media_urls.push(hard_failure_server.url());
            hard_failure_http_server = Some(hard_failure_server);
            PathBuf::from(url)
        } else if options.exercise_http_stall {
            let server = StalledLoopbackHttpServer::start(generated_media.clone())?;
            let endpoint = server.endpoint();
            require_ipv4_loopback_endpoint(&endpoint, "stalled HTTP listener")?;
            let url = server.url();
            let evidence = HttpStallEvidence::new(endpoint, url.clone(), &generated_media);
            write_json_file(&http_stall_path, &evidence)?;
            stalled_http_server = Some(server);
            http_stall_evidence = Some(evidence);
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
            media_url
                .iter()
                .cloned()
                .chain(additional_trusted_media_urls)
                .collect(),
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
        } else if options.exercise_http_stall {
            state.advance(
                &state_path,
                "stalled-http-ready",
                Some("strict-loopback-stalled-http-ready"),
            )?;
        }

        let expected_playlist_target = match media_url.as_ref() {
            Some(media_url) => media_url.clone(),
            None => media_path
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
                .ok_or_else(|| {
                    "generated local media did not have a valid UTF-8 playlist entry".to_owned()
                })?,
        };
        let server = start_playlist_echo_mock_session_server(
            REAL_MPV_SESSION_HELLO,
            expected_playlist_target,
            REAL_MPV_LOOPBACK_USERNAME,
        )?;
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
                shared_lifecycle_evidence_path: Some(&shared_lifecycle_path),
                shared_lifecycle_run_id: Some(&shared_lifecycle_run_id),
                shared_lifecycle_emitter: Some("gui-real-mpv"),
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
        let (
            playlist_change_request,
            playlist_change_echo,
            playlist_index_request,
            playlist_index_echo,
            initial_authoritative_playstate,
        ) = session_server
            .as_ref()
            .expect("real-mpv session server must remain live")
            .recv_playlist_exchange(step_timeout, "real-mpv canonical playlist echo")?;
        let exchange = session_exchange
            .as_mut()
            .expect("real-mpv session exchange must remain initialized");
        exchange.playlist_change_request = Some(playlist_change_request);
        exchange.playlist_change_echo = Some(playlist_change_echo);
        exchange.playlist_index_request = Some(playlist_index_request);
        exchange.playlist_index_echo = Some(playlist_index_echo);
        let initial_playstate_json: serde_json::Value =
            serde_json::from_str(&initial_authoritative_playstate).map_err(|error| {
                format!("initial authoritative playstate was invalid JSON: {error}")
            })?;
        if initial_playstate_json
            != serde_json::json!({
                "State": {
                    "playstate": {
                        "position": 0.0,
                        "paused": true,
                        "doSeek": false,
                        "setBy": REAL_MPV_LOOPBACK_USERNAME,
                    }
                }
            })
        {
            return Err("initial authoritative paused playstate drifted".to_owned());
        }
        exchange.initial_authoritative_playstate = Some(initial_authoritative_playstate);
        write_json_file(&session_exchange_path, exchange)?;

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
        } else if options.exercise_http_stall {
            REAL_MPV_HTTP_STALL_ROUTE
                .rsplit('/')
                .next()
                .expect("stalled HTTP route has a file component")
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
        if let Some(evidence) = http_stall_evidence.as_mut() {
            evidence.initial_file_loaded_index = Some(file_loaded_index);
            evidence.initial_pid = Some(mpv_pid);
            evidence.parent_pid = Some(parent_pid);
            evidence.process_image_path = Some(initial_process_image_path.display().to_string());
            evidence.process_sha256 = Some(process_identity.sha256.clone());
            evidence.initial_ipc_endpoint = Some(ipc_endpoint.clone());
            write_json_file(&http_stall_path, evidence)?;
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
        wait_for_accessible_name_prefix(
            &driver,
            launched_window,
            PAUSED_ROOM_INTENT_PREFIX,
            step_timeout,
        )?;
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
        record_authoritative_playstate_exchange(
            session_server
                .as_ref()
                .expect("real-mpv session server must remain live"),
            session_exchange
                .as_mut()
                .expect("real-mpv session exchange must remain initialized"),
            &session_exchange_path,
            step_timeout,
            "GUI Play canonical transport",
            false,
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
        wait_for_accessible_name_prefix(
            &driver,
            launched_window,
            PLAYING_ROOM_INTENT_PREFIX,
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
            fault_http_server
                .as_ref()
                .expect("faulting HTTP server must remain live")
                .trigger_fault()?;
            {
                let evidence = http_fault_evidence
                    .as_mut()
                    .expect("faulting HTTP evidence must be initialized");
                evidence.pre_fault_progress_index = Some(pre_fault_progress_index);
                evidence.pre_fault_position_seconds = Some(pre_fault_position);
                evidence.fault_triggered_after_progress = true;
                write_json_file(&http_fault_path, evidence)?;
            }
            let (premature_eof_index, premature_eof) = wait_for_mpv_observation(
                &observation_path,
                pre_fault_progress_index,
                fault_timeout,
                "causal keep-open eof-reached=true after the malformed chunked HTTP response",
                |observation| {
                    observation.event == "eof-reached"
                        && observation.pid == Some(mpv_pid)
                        && observation.eof_reached == Some(true)
                        && observation.ipc_endpoint.as_deref() == Some(&ipc_endpoint)
                },
            )?;
            let premature_eof_position = premature_eof.position.ok_or_else(|| {
                "premature keep-open EOF observation omitted its position".to_owned()
            })?;
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
                || first_request.transmitted_body_bytes < REAL_MPV_HTTP_FAULT_MINIMUM_PREFIX_BYTES
                || first_request.transmitted_body_bytes >= generated_media.len()
                || first_request.content_length_header.is_some()
                || first_request.transfer_encoding.as_deref() != Some("chunked")
                || !first_request.framing_fault_injected
                || first_request.write_error.is_some()
            {
                return Err(format!(
                    "first faulting HTTP response was not the exact malformed chunked response: {first_request:?}"
                ));
            }
            if premature_eof.path.as_deref() != media_url.as_deref()
                || premature_eof.duration.is_none_or(|duration| {
                    (duration - f64::from(media_duration_seconds)).abs() > 0.05
                })
                || premature_eof_position < pre_fault_position
                || f64::from(media_duration_seconds) - premature_eof_position <= 15.0
            {
                return Err(format!(
                    "premature keep-open EOF identity or remaining duration drifted: {premature_eof:?}"
                ));
            }
            state.advance(
                &state_path,
                "malformed-http-premature-eof-observed",
                Some("one-malformed-http-premature-eof-observed"),
            )?;

            let (recovered_file_loaded_index, recovered_file_loaded) = wait_for_mpv_observation(
                &observation_path,
                premature_eof_index.saturating_add(1),
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
            wait_for_accessible_name_prefix(
                &driver,
                launched_window,
                PLAYING_ROOM_INTENT_PREFIX,
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
            record_authoritative_playstate_exchange(
                session_server
                    .as_ref()
                    .expect("real-mpv session server must remain live"),
                session_exchange
                    .as_mut()
                    .expect("real-mpv session exchange must remain initialized"),
                &session_exchange_path,
                step_timeout,
                "GUI Pause after HTTP fault canonical transport",
                true,
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
                && pre_fault_progress_index < premature_eof_index
                && premature_eof_index < recovered_file_loaded_index
                && recovered_file_loaded_index < recovered_progress_index
                && recovered_progress_index < recovered_paused_index)
            {
                return Err(format!(
                    "faulting HTTP observation ordering drifted: {file_loaded_index}, {playing_index}, {pre_fault_progress_index}, {premature_eof_index}, {recovered_file_loaded_index}, {recovered_progress_index}, {recovered_paused_index}"
                ));
            }
            state.advance(
                &state_path,
                "real-mpv-paused",
                Some("gui-pause-command-observed-by-real-mpv"),
            )?;
            wait_for_accessible_name_prefix(
                &driver,
                launched_window,
                PAUSED_ROOM_INTENT_PREFIX,
                step_timeout,
            )?;
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
                .skip(premature_eof_index)
                .take(
                    recovered_paused_index
                        .saturating_sub(premature_eof_index)
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
            evidence.premature_eof_index = Some(premature_eof_index);
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
            evidence.premature_eof_position_seconds = Some(premature_eof_position);
            evidence.recovered_position_seconds = Some(recovered_position);
            evidence.foreign_pid_observations_after_fault = foreign_observations;
            evidence.evidence_retained_before_cleanup = true;
            write_json_file(&http_fault_path, evidence)?;
            state.advance(
                &state_path,
                "faulting-http-evidence-retained",
                Some("fault-evidence-retained-before-cleanup"),
            )?;

            let hard_failure_server = hard_failure_http_server
                .as_ref()
                .expect("hard-failure HTTP server must be preflighted and trusted");
            let hard_failure_endpoint = hard_failure_server.endpoint();
            require_ipv4_loopback_endpoint(
                &hard_failure_endpoint,
                "hard media-failure HTTP listener",
            )?;
            let hard_failure_url = hard_failure_server.url();
            let evidence = MediaFailureRecoveryEvidence::new(MediaFailureRecoveryInit {
                listener_endpoint: hard_failure_endpoint,
                media_url: hard_failure_url.clone(),
                initial_pid: mpv_pid,
                parent_pid,
                process_image_path: &initial_process_image_path,
                process_sha256: mpv_preflight.identity.sha256.clone(),
                initial_ipc_endpoint: ipc_endpoint.clone(),
                restored_media_path: &media_path,
                restored_media_sha256: hex_sha256(&generated_media),
            });
            write_json_file(&media_failure_path, &evidence)?;
            media_failure_evidence = Some(evidence);

            let observations_before_hard_failure = read_mpv_observations(&observation_path)?.len();
            let lifecycle_before_hard_failure =
                wait_for_lifecycle_snapshot(&shared_lifecycle_path, step_timeout)?.len();
            let mock_session = session_server
                .as_ref()
                .expect("real-mpv session server must remain live");
            mock_session.send_authoritative_line(
                serde_json::json!({
                    "Set": {
                        "playlistChange": {
                            "files": [&hard_failure_url],
                            "user": "remote-controller",
                        }
                    }
                })
                .to_string(),
                "hard media-failure playlist",
            )?;
            mock_session.send_authoritative_line(
                serde_json::json!({
                    "Set": {
                        "playlistIndex": {
                            "index": 0,
                            "user": "remote-controller",
                        }
                    }
                })
                .to_string(),
                "hard media-failure playlist selection",
            )?;
            mock_session.send_authoritative_line(
                serde_json::json!({
                    "State": {
                        "playstate": {
                            "position": 0.0,
                            "paused": true,
                            "doSeek": false,
                            "setBy": "remote-controller",
                        }
                    }
                })
                .to_string(),
                "hard media-failure paused transport",
            )?;

            let initial_hard_failure_requests = hard_failure_http_server
                .as_ref()
                .expect("hard-failure HTTP server must remain live")
                .wait_for_media_get(fault_timeout)?;
            validate_hard_failure_http_request_accounting(&initial_hard_failure_requests)?;
            let (failure_end_file_index, failure_end_file) = wait_for_mpv_observation(
                &observation_path,
                observations_before_hard_failure,
                fault_timeout,
                "same-process end-file error after authoritative HTTP 404 media load",
                |observation| {
                    observation.event == "end-file"
                        && observation.reason.as_deref() == Some("error")
                        && observation.pid == Some(mpv_pid)
                        && observation.ipc_endpoint.as_deref() == Some(&ipc_endpoint)
                },
            )?;
            let (media_fail_index, media_fail_record) = wait_for_lifecycle_transition(
                &shared_lifecycle_path,
                lifecycle_before_hard_failure,
                "MEDIA-FAIL-001",
                fault_timeout,
            )?;
            let media_fail_event_id =
                required_lifecycle_string(&media_fail_record, "event_id", "MEDIA-FAIL-001")?;
            let media_fail_emitter =
                required_lifecycle_string(&media_fail_record, "emitter", "MEDIA-FAIL-001")?;
            let media_fail_process_role =
                required_lifecycle_string(&media_fail_record, "process_role", "MEDIA-FAIL-001")?;
            if media_fail_emitter != "gui-real-mpv" || media_fail_process_role != "client" {
                return Err(format!(
                    "hard media-failure lifecycle attribution drifted: emitter={media_fail_emitter:?}, process_role={media_fail_process_role:?}"
                ));
            }
            if !process_is_running(mpv_pid)
                || process_parent_pid(mpv_pid)? != gui_pid
                || binary_identity(&process_image_path(mpv_pid)?)?.sha256
                    != mpv_preflight.identity.sha256
            {
                return Err(format!(
                    "GUI-owned mpv identity changed across the hard media-failure boundary for PID {mpv_pid}"
                ));
            }
            {
                let evidence = media_failure_evidence
                    .as_mut()
                    .expect("hard media-failure evidence must be initialized");
                evidence.record_requests(initial_hard_failure_requests);
                evidence.failure_end_file_index = Some(failure_end_file_index);
                evidence.failure_reason = failure_end_file.reason.clone();
                evidence.media_fail_event_id = Some(media_fail_event_id);
                evidence.media_fail_emitter = Some(media_fail_emitter);
                evidence.media_fail_process_role = Some(media_fail_process_role);
                evidence.failure_pid = failure_end_file.pid;
                evidence.failure_ipc_endpoint = failure_end_file.ipc_endpoint.clone();
                write_json_file(&media_failure_path, evidence)?;
            }
            state.advance(
                &state_path,
                "authoritative-http-404-media-failure-observed",
                Some("authoritative-http-404-produced-media-failure"),
            )?;

            let restored_media = media_path.display().to_string();
            mock_session.send_authoritative_line(
                serde_json::json!({
                    "Set": {
                        "playlistChange": {
                            "files": [&restored_media],
                            "user": "remote-controller",
                        }
                    }
                })
                .to_string(),
                "hard media-failure recovery playlist",
            )?;
            mock_session.send_authoritative_line(
                serde_json::json!({
                    "Set": {
                        "playlistIndex": {
                            "index": 0,
                            "user": "remote-controller",
                        }
                    }
                })
                .to_string(),
                "hard media-failure recovery selection",
            )?;
            mock_session.send_authoritative_line(
                serde_json::json!({
                    "State": {
                        "playstate": {
                            "position": 0.0,
                            "paused": true,
                            "doSeek": false,
                            "setBy": "remote-controller",
                        }
                    }
                })
                .to_string(),
                "hard media-failure recovery transport",
            )?;
            let (restored_file_loaded_index, restored_file_loaded) = wait_for_mpv_observation(
                &observation_path,
                failure_end_file_index.saturating_add(1),
                fault_timeout,
                "same-process file-loaded after authoritative local-media restore",
                |observation| {
                    observation.event == "file-loaded"
                        && observation.pid == Some(mpv_pid)
                        && observation.ipc_endpoint.as_deref() == Some(&ipc_endpoint)
                        && observation.path.as_deref().is_some_and(|path| {
                            observed_media_path_matches(Path::new(path), &media_path)
                        })
                },
            )?;
            let (_, media_playable_record) = wait_for_lifecycle_transition(
                &shared_lifecycle_path,
                media_fail_index.saturating_add(1),
                "MEDIA-PLAYABLE-001",
                fault_timeout,
            )?;
            let media_playable_event_id = required_lifecycle_string(
                &media_playable_record,
                "event_id",
                "MEDIA-PLAYABLE-001",
            )?;
            let media_playable_emitter =
                required_lifecycle_string(&media_playable_record, "emitter", "MEDIA-PLAYABLE-001")?;
            let media_playable_process_role = required_lifecycle_string(
                &media_playable_record,
                "process_role",
                "MEDIA-PLAYABLE-001",
            )?;
            if media_playable_emitter != "gui-real-mpv"
                || media_playable_process_role != "client"
                || restored_file_loaded.pid != Some(mpv_pid)
                || restored_file_loaded.ipc_endpoint.as_deref() != Some(&ipc_endpoint)
                || !process_is_running(mpv_pid)
                || process_parent_pid(mpv_pid)? != gui_pid
                || binary_identity(&process_image_path(mpv_pid)?)?.sha256
                    != mpv_preflight.identity.sha256
            {
                return Err(format!(
                    "same-process hard media-failure recovery identity or lifecycle attribution drifted: emitter={media_playable_emitter:?}, process_role={media_playable_process_role:?}, observation={restored_file_loaded:?}"
                ));
            }
            state.advance(
                &state_path,
                "hard-media-failure-recovered",
                Some("same-owned-mpv-recovered-from-hard-media-failure"),
            )?;

            let requests = hard_failure_http_server
                .take()
                .expect("hard-failure HTTP server must remain live until recovery")
                .release()?;
            validate_hard_failure_http_request_accounting(&requests)?;
            let evidence = media_failure_evidence
                .as_mut()
                .expect("hard media-failure evidence must be initialized");
            evidence.record_requests(requests);
            evidence.restored_file_loaded_index = Some(restored_file_loaded_index);
            evidence.media_playable_event_id = Some(media_playable_event_id);
            evidence.media_playable_emitter = Some(media_playable_emitter);
            evidence.media_playable_process_role = Some(media_playable_process_role);
            evidence.recovered_pid = restored_file_loaded.pid;
            evidence.recovered_ipc_endpoint = restored_file_loaded.ipc_endpoint;
            evidence.same_process_identity =
                evidence.failure_pid == Some(mpv_pid) && evidence.recovered_pid == Some(mpv_pid);
            evidence.same_ipc_endpoint = evidence.failure_ipc_endpoint.as_deref()
                == Some(&ipc_endpoint)
                && evidence.recovered_ipc_endpoint.as_deref() == Some(&ipc_endpoint);
            evidence.evidence_retained_before_cleanup = true;
            evidence.server_thread_released = true;
            evidence.socket_released = true;
            write_json_file(&media_failure_path, evidence)?;
            state.advance(
                &state_path,
                "hard-media-failure-evidence-retained",
                Some("hard-media-failure-evidence-retained"),
            )?;
        } else if options.exercise_http_stall {
            let stall_timeout = REAL_MPV_HTTP_STALL_MAXIMUM_RECOVERY_WAIT;
            let (pre_stall_progress_index, pre_stall_progress) = wait_for_mpv_observation(
                &observation_path,
                playing_index,
                step_timeout,
                "positive time-pos before the controlled HTTP stall",
                |observation| {
                    observation.event == "time-pos"
                        && observation.pid == Some(mpv_pid)
                        && observation.ipc_endpoint.as_deref() == Some(&ipc_endpoint)
                        && observation.path.as_deref() == media_url.as_deref()
                        && observation.position.is_some_and(|position| position >= 0.5)
                },
            )?;
            let pre_stall_position = pre_stall_progress
                .position
                .ok_or_else(|| "pre-stall time-pos observation omitted its position".to_owned())?;
            {
                let evidence = http_stall_evidence
                    .as_mut()
                    .expect("stalled HTTP evidence must be initialized");
                evidence.pre_stall_progress_index = Some(pre_stall_progress_index);
                evidence.pre_stall_position_seconds = Some(pre_stall_position);
                write_json_file(&http_stall_path, evidence)?;
            }
            let first_requests = stalled_http_server
                .as_ref()
                .expect("stalled HTTP server must remain live")
                .wait_for_media_gets(1, step_timeout)?;
            {
                let evidence = http_stall_evidence
                    .as_mut()
                    .expect("stalled HTTP evidence must be initialized");
                evidence.record_requests(first_requests.clone());
                write_json_file(&http_stall_path, evidence)?;
            }
            let first_request = first_requests
                .iter()
                .find(|request| request.method == "GET")
                .ok_or_else(|| "stalled HTTP request evidence was empty".to_owned())?;
            if first_request.status_code != 200
                || first_request.range_header.as_deref() != Some("bytes=0-")
                || first_request.content_length_header != Some(generated_media.len())
                || first_request.transfer_encoding.is_some()
                || first_request.transmitted_body_bytes != REAL_MPV_HTTP_STALL_PREFIX_BYTES
                || !first_request.stall_injected
                || first_request.stalled_for_ms.is_some()
                || first_request.server_response_retained_at_recovery_get
                || first_request.connection_released
                || first_request.response_completed
                || first_request.write_error.is_some()
            {
                return Err(format!(
                    "first stalled HTTP response was not the exact open byte-silent prefix: {first_request:?}"
                ));
            }
            let (cache_stall_index, cache_stall) = wait_for_mpv_observation(
                &observation_path,
                pre_stall_progress_index,
                step_timeout,
                "paused-for-cache=true after the valid HTTP prefix stopped advancing",
                |observation| {
                    observation.event == "paused-for-cache"
                        && observation.pid == Some(mpv_pid)
                        && observation.paused_for_cache == Some(true)
                        && observation.ipc_endpoint.as_deref() == Some(&ipc_endpoint)
                        && observation.path.as_deref() == media_url.as_deref()
                },
            )?;
            let cache_stall_position = cache_stall.position.ok_or_else(|| {
                "cache-stall observation omitted its retained playback position".to_owned()
            })?;
            {
                let evidence = http_stall_evidence
                    .as_mut()
                    .expect("stalled HTTP evidence must be initialized");
                evidence.cache_stall_index = Some(cache_stall_index);
                evidence.cache_stall_position_seconds = Some(cache_stall_position);
                write_json_file(&http_stall_path, evidence)?;
            }
            let expected_prefix_position = real_mpv_http_stall_prefix_playable_seconds();
            if cache_stall
                .duration
                .is_none_or(|duration| (duration - f64::from(media_duration_seconds)).abs() > 0.05)
                || cache_stall_position < pre_stall_position
                || (cache_stall_position - expected_prefix_position).abs()
                    > REAL_MPV_HTTP_STALL_POSITION_TOLERANCE_SECONDS
                || cache_stall.eof_reached == Some(true)
            {
                return Err(format!(
                    "cache-stall identity, prefix-bound position, duration, or EOF state drifted (expected {expected_prefix_position:.6} +/- {:.3} seconds): {cache_stall:?}",
                    REAL_MPV_HTTP_STALL_POSITION_TOLERANCE_SECONDS
                ));
            }
            state.advance(
                &state_path,
                "stalled-http-cache-pause-observed",
                Some("sustained-valid-http-cache-stall-observed"),
            )?;

            let stall_elapsed = stalled_http_server
                .as_ref()
                .expect("stalled HTTP server must remain live")
                .stall_elapsed()?
                .ok_or_else(|| {
                    "stalled HTTP server did not retain its prefix-completion boundary".to_owned()
                })?;
            let recovery_timeout = stall_timeout.checked_sub(stall_elapsed).ok_or_else(|| {
                format!(
                    "stalled HTTP cache pause arrived after the {} ms prefix-to-recovery deadline (elapsed {} ms)",
                    stall_timeout.as_millis(),
                    stall_elapsed.as_millis()
                )
            })?;
            let (recovered_file_loaded_index, recovered_file_loaded) = wait_for_mpv_observation(
                &observation_path,
                cache_stall_index.saturating_add(1),
                recovery_timeout,
                "same-process file-loaded after bounded cache-stall recovery",
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
                "recovered stalled-HTTP file-loaded observation omitted duration".to_owned()
            })?;
            if recovered_file_loaded.filename.as_deref() != Some(expected_file_name)
                || (recovered_duration - f64::from(media_duration_seconds)).abs() > 0.05
            {
                return Err(format!(
                    "recovered stalled-HTTP media identity drifted: filename={:?}, duration={recovered_duration}",
                    recovered_file_loaded.filename
                ));
            }
            {
                let evidence = http_stall_evidence
                    .as_mut()
                    .expect("stalled HTTP evidence must be initialized");
                evidence.recovered_file_loaded_index = Some(recovered_file_loaded_index);
                evidence.recovered_pid = Some(mpv_pid);
                evidence.recovered_ipc_endpoint = Some(ipc_endpoint.clone());
                write_json_file(&http_stall_path, evidence)?;
            }
            let requests = stalled_http_server
                .as_ref()
                .expect("stalled HTTP server must remain live")
                .wait_for_media_gets(2, step_timeout)?;
            {
                let evidence = http_stall_evidence
                    .as_mut()
                    .expect("stalled HTTP evidence must be initialized");
                evidence.record_requests(requests.clone());
                write_json_file(&http_stall_path, evidence)?;
            }
            validate_stalled_http_request_accounting(&requests, generated_media.len(), false)?;
            if !process_is_running(mpv_pid)
                || process_parent_pid(mpv_pid)? != gui_pid
                || binary_identity(&process_image_path(mpv_pid)?)?.sha256
                    != mpv_preflight.identity.sha256
            {
                return Err(format!(
                    "GUI-owned mpv identity changed across the stalled HTTP boundary for PID {mpv_pid}"
                ));
            }
            {
                let evidence = http_stall_evidence
                    .as_mut()
                    .expect("stalled HTTP evidence must be initialized");
                evidence.stable_process_identity = true;
                evidence.stable_ipc_endpoint = true;
                evidence.stable_media_url = true;
                evidence.stable_duration = true;
                write_json_file(&http_stall_path, evidence)?;
            }
            let observations_through_recovery = read_mpv_observations(&observation_path)?;
            let causal_observations = observations_through_recovery
                .iter()
                .skip(file_loaded_index)
                .take(
                    recovered_file_loaded_index
                        .saturating_sub(file_loaded_index)
                        .saturating_add(1),
                );
            let eof_observations_before_recovery = causal_observations
                .clone()
                .filter(|observation| {
                    observation.event == "eof-reached" && observation.eof_reached == Some(true)
                })
                .count();
            let end_file_observations = observations_through_recovery
                .iter()
                .enumerate()
                .skip(cache_stall_index.saturating_add(1))
                .take(
                    recovered_file_loaded_index
                        .saturating_sub(cache_stall_index)
                        .saturating_sub(1),
                )
                .filter(|(_, observation)| observation.event == "end-file")
                .collect::<Vec<_>>();
            let end_file_observations_before_recovery = end_file_observations.len();
            let recovery_lifecycle_observations = observations_through_recovery
                .iter()
                .enumerate()
                .skip(cache_stall_index.saturating_add(1))
                .take(recovered_file_loaded_index.saturating_sub(cache_stall_index))
                .filter(|(_, observation)| {
                    matches!(observation.event.as_str(), "end-file" | "file-loaded")
                })
                .collect::<Vec<_>>();
            {
                let evidence = http_stall_evidence
                    .as_mut()
                    .expect("stalled HTTP evidence must be initialized");
                evidence.eof_observations_before_recovery = eof_observations_before_recovery;
                evidence.end_file_observations_before_recovery =
                    end_file_observations_before_recovery;
                write_json_file(&http_stall_path, evidence)?;
            }
            if eof_observations_before_recovery != 0 {
                return Err(format!(
                    "stalled HTTP recovery observed {eof_observations_before_recovery} EOF events despite an open valid response"
                ));
            }
            if end_file_observations_before_recovery != 1
                || recovery_lifecycle_observations.len() != 2
                || recovery_lifecycle_observations[0].1.event != "end-file"
                || recovery_lifecycle_observations[0].1.pid != Some(mpv_pid)
                || recovery_lifecycle_observations[0].1.ipc_endpoint.as_deref()
                    != Some(&ipc_endpoint)
                || recovery_lifecycle_observations[0].1.reason.as_deref() != Some("stop")
                || recovery_lifecycle_observations[1].0 != recovered_file_loaded_index
                || recovery_lifecycle_observations[1].1.event != "file-loaded"
                || recovery_lifecycle_observations[1].1.pid != Some(mpv_pid)
                || recovery_lifecycle_observations[1].1.ipc_endpoint.as_deref()
                    != Some(&ipc_endpoint)
            {
                return Err(format!(
                    "stalled HTTP recovery contained an unidentified or intervening lifecycle row instead of exactly one same-process end-file stop followed by the recovered file-loaded: {recovery_lifecycle_observations:?}"
                ));
            }
            state.advance(
                &state_path,
                "stalled-http-reloaded",
                Some("same-owned-mpv-reloaded-after-bounded-cache-stall"),
            )?;

            let required_recovered_position = cache_stall_position + 0.5;
            let (recovered_progress_index, recovered_progress) = wait_for_mpv_observation(
                &observation_path,
                recovered_file_loaded_index,
                step_timeout,
                "post-recovery playback progress beyond the cache-stall position",
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
            let recovered_position = recovered_progress.position.ok_or_else(|| {
                "recovered stalled-HTTP time-pos observation omitted its position".to_owned()
            })?;
            {
                let evidence = http_stall_evidence
                    .as_mut()
                    .expect("stalled HTTP evidence must be initialized");
                evidence.recovered_progress_index = Some(recovered_progress_index);
                evidence.recovered_position_seconds = Some(recovered_position);
                write_json_file(&http_stall_path, evidence)?;
            }
            wait_for_accessible_name_prefix(
                &driver,
                launched_window,
                PLAYING_ROOM_INTENT_PREFIX,
                step_timeout,
            )?;
            state.advance(
                &state_path,
                "stalled-http-progress-recovered",
                Some("recovered-playback-advanced-past-stall"),
            )?;

            let observations_before_pause = read_mpv_observations(&observation_path)?.len();
            invoke_named_control_with_wait(
                &driver,
                launched_window,
                PAUSE_CONTROL_AUTOMATION_ID,
                NativeControlKind::Button,
                step_timeout,
            )?;
            record_authoritative_playstate_exchange(
                session_server
                    .as_ref()
                    .expect("real-mpv session server must remain live"),
                session_exchange
                    .as_mut()
                    .expect("real-mpv session exchange must remain initialized"),
                &session_exchange_path,
                step_timeout,
                "GUI Pause after HTTP stall canonical transport",
                true,
            )?;
            let (recovered_paused_index, _) = wait_for_mpv_observation(
                &observation_path,
                observations_before_pause,
                step_timeout,
                "pause=true after recovered stalled-HTTP playback",
                |observation| {
                    observation.event == "pause"
                        && observation.pid == Some(mpv_pid)
                        && observation.pause == Some(true)
                        && observation.ipc_endpoint.as_deref() == Some(&ipc_endpoint)
                        && observation.path.as_deref() == media_url.as_deref()
                },
            )?;
            {
                let evidence = http_stall_evidence
                    .as_mut()
                    .expect("stalled HTTP evidence must be initialized");
                evidence.recovered_paused_index = Some(recovered_paused_index);
                write_json_file(&http_stall_path, evidence)?;
            }
            if !(file_loaded_index < playing_index
                && playing_index < pre_stall_progress_index
                && pre_stall_progress_index < cache_stall_index
                && cache_stall_index < recovered_file_loaded_index
                && recovered_file_loaded_index < recovered_progress_index
                && recovered_progress_index < recovered_paused_index)
            {
                return Err(format!(
                    "stalled HTTP observation ordering drifted: {file_loaded_index}, {playing_index}, {pre_stall_progress_index}, {cache_stall_index}, {recovered_file_loaded_index}, {recovered_progress_index}, {recovered_paused_index}"
                ));
            }
            state.advance(
                &state_path,
                "real-mpv-paused",
                Some("gui-pause-command-observed-by-real-mpv"),
            )?;
            wait_for_accessible_name_prefix(
                &driver,
                launched_window,
                PAUSED_ROOM_INTENT_PREFIX,
                step_timeout,
            )?;
            state.advance(
                &state_path,
                "gui-paused-projected",
                Some("gui-projected-paused-after-real-mpv-observation"),
            )?;

            let observations = read_mpv_observations(&observation_path)?;
            let foreign_observations = observations
                .iter()
                .skip(cache_stall_index)
                .take(
                    recovered_paused_index
                        .saturating_sub(cache_stall_index)
                        .saturating_add(1),
                )
                .filter(|observation| {
                    observation.pid != Some(mpv_pid)
                        || observation.ipc_endpoint.as_deref() != Some(&ipc_endpoint)
                })
                .count();
            {
                let evidence = http_stall_evidence
                    .as_mut()
                    .expect("stalled HTTP evidence must be initialized");
                evidence.foreign_pid_observations_after_stall = foreign_observations;
                write_json_file(&http_stall_path, evidence)?;
            }
            if foreign_observations != 0 {
                return Err(format!(
                    "unidentified, stale, or foreign mpv generation emitted {foreign_observations} observations after the HTTP stall boundary"
                ));
            }
            let evidence = http_stall_evidence
                .as_mut()
                .expect("stalled HTTP evidence must be initialized");
            evidence.record_requests(requests);
            evidence.pre_stall_progress_index = Some(pre_stall_progress_index);
            evidence.cache_stall_index = Some(cache_stall_index);
            evidence.recovered_file_loaded_index = Some(recovered_file_loaded_index);
            evidence.recovered_progress_index = Some(recovered_progress_index);
            evidence.recovered_paused_index = Some(recovered_paused_index);
            evidence.recovered_pid = Some(mpv_pid);
            evidence.recovered_ipc_endpoint = Some(ipc_endpoint.clone());
            evidence.stable_process_identity = true;
            evidence.stable_ipc_endpoint = true;
            evidence.stable_media_url = true;
            evidence.stable_duration = true;
            evidence.pre_stall_position_seconds = Some(pre_stall_position);
            evidence.cache_stall_position_seconds = Some(cache_stall_position);
            evidence.recovered_position_seconds = Some(recovered_position);
            evidence.eof_observations_before_recovery = eof_observations_before_recovery;
            evidence.foreign_pid_observations_after_stall = foreign_observations;
            evidence.evidence_retained_before_cleanup = true;
            write_json_file(&http_stall_path, evidence)?;
            state.advance(
                &state_path,
                "stalled-http-evidence-retained",
                Some("stall-evidence-retained-before-cleanup"),
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
            record_authoritative_playstate_exchange(
                session_server
                    .as_ref()
                    .expect("real-mpv session server must remain live"),
                session_exchange
                    .as_mut()
                    .expect("real-mpv session exchange must remain initialized"),
                &session_exchange_path,
                step_timeout,
                "GUI Pause canonical transport",
                true,
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
            wait_for_accessible_name_prefix(
                &driver,
                launched_window,
                PAUSED_ROOM_INTENT_PREFIX,
                step_timeout,
            )?;
            state.advance(
                &state_path,
                "gui-paused-projected",
                Some("gui-projected-paused-after-real-mpv-observation"),
            )?;
        }

        let mut active_mpv_pid = mpv_pid;
        let mut recovered_mpv_identity = None;
        if options.exercise_recovery {
            let missing_media = exercise_missing_media_resolution(
                session_server
                    .as_ref()
                    .expect("session server remains live"),
                &shared_lifecycle_path,
                &artifact_root.join("missing-generated-media.wav"),
                &media_path,
                mpv_pid,
                step_timeout,
            )?;
            let recovery = recovery_evidence.as_mut().expect("recovery is initialized");
            recovery.missing_media = Some(missing_media);
            write_json_file(&recovery_path, recovery)?;
            state.advance(
                &state_path,
                "missing-playlist-target-observed",
                Some("missing-playlist-target-is-reported"),
            )?;
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

            // Process relaunch is automatic; reopening the selected media is
            // a separate user action. A load can also arrive during ownership
            // attestation, so retain observations from the relaunch boundary.
            invoke_real_mpv_menu_action_with_evidence(
                &driver,
                launched_window,
                FILE_MENU_AUTOMATION_ID,
                OPEN_MEDIA_MENU_AUTOMATION_ID,
                step_timeout,
                &mut menu_interactions,
                &menu_interactions_path,
            )?;
            let (recovered_file_loaded_index, recovered_file_loaded) =
                wait_for_replacement_media_loaded(
                    &observation_path,
                    automatic_relaunch_observation_index,
                    recovered_mpv_pid,
                    &recovered_ipc_endpoint,
                    &media_path,
                    step_timeout,
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
            wait_for_accessible_name_prefix(
                &driver,
                launched_window,
                PAUSED_ROOM_INTENT_PREFIX,
                step_timeout,
            )?;
            let observations_before_recovered_play =
                read_mpv_observations(&observation_path)?.len();
            invoke_named_control_with_wait(
                &driver,
                launched_window,
                PLAY_CONTROL_AUTOMATION_ID,
                NativeControlKind::Button,
                step_timeout,
            )?;
            record_authoritative_playstate_exchange(
                session_server
                    .as_ref()
                    .expect("real-mpv session server must remain live"),
                session_exchange
                    .as_mut()
                    .expect("real-mpv session exchange must remain initialized"),
                &session_exchange_path,
                step_timeout,
                "GUI Play on replacement mpv canonical transport",
                false,
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
            wait_for_accessible_name_prefix(
                &driver,
                launched_window,
                PLAYING_ROOM_INTENT_PREFIX,
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
            record_authoritative_playstate_exchange(
                session_server
                    .as_ref()
                    .expect("real-mpv session server must remain live"),
                session_exchange
                    .as_mut()
                    .expect("real-mpv session exchange must remain initialized"),
                &session_exchange_path,
                step_timeout,
                "GUI Pause on replacement mpv canonical transport",
                true,
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
            wait_for_accessible_name_prefix(
                &driver,
                launched_window,
                PAUSED_ROOM_INTENT_PREFIX,
                step_timeout,
            )?;
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
        wait_for_lifecycle_event_suffix(
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
            let media_failure = media_failure_evidence
                .as_mut()
                .expect("hard media-failure evidence must be complete");
            media_failure.owned_mpv_terminated_after_gui_exit = !process_is_running(mpv_pid);
            if !media_failure.owned_mpv_terminated_after_gui_exit {
                return Err(format!(
                    "GUI-owned mpv PID {mpv_pid} remained alive after hard media-failure GUI exit"
                ));
            }
            media_failure.result = "passed".to_owned();
            write_json_file(&media_failure_path, media_failure)?;
        }
        if options.exercise_http_stall {
            let requests = stalled_http_server
                .take()
                .expect("stalled HTTP server must remain live until GUI exit")
                .release()?;
            let evidence = http_stall_evidence
                .as_mut()
                .expect("stalled HTTP evidence must be complete");
            evidence.record_requests(requests.clone());
            evidence.server_thread_released = true;
            evidence.socket_released = true;
            evidence.owned_mpv_terminated_after_gui_exit = !process_is_running(mpv_pid);
            write_json_file(&http_stall_path, evidence)?;
            validate_stalled_http_request_accounting(&requests, generated_media.len(), true)?;
            if !evidence.owned_mpv_terminated_after_gui_exit {
                return Err(format!(
                    "GUI-owned mpv PID {mpv_pid} remained alive after stalled HTTP GUI exit"
                ));
            }
            evidence.result = "passed".to_owned();
            write_json_file(&http_stall_path, evidence)?;
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
            "gui-exit-reaped-owned-mpv-and-released-fault-servers"
        } else if options.exercise_http_stall {
            "gui-exit-reaped-owned-mpv-and-released-stall-server"
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
            ("shared_lifecycle", shared_lifecycle_path.as_path()),
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
            artifact_files.extend([
                ("faulting_http_recovery", http_fault_path.as_path()),
                ("hard_media_failure", media_failure_path.as_path()),
            ]);
        }
        if options.exercise_http_stall {
            artifact_files.push(("stalled_http", http_stall_path.as_path()));
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
            media_failure: media_failure_evidence.clone(),
            http_stall: http_stall_evidence.clone(),
            isolation: IsolationContract {
                artifact_root: artifact_root.display().to_string(),
                config_path: config_path.display().to_string(),
                appdata_root: appdata_root.display().to_string(),
                media_path: media_path.display().to_string(),
                observation_script_path: observation_script_path.display().to_string(),
                observation_path: observation_path.display().to_string(),
                mpv_log_path: mpv_log_path.display().to_string(),
                lifecycle_path: lifecycle_path.display().to_string(),
                shared_lifecycle_path: shared_lifecycle_path.display().to_string(),
                session_exchange_path: session_exchange_path.display().to_string(),
                menu_interactions_path: menu_interactions_path.display().to_string(),
                ipc_endpoint,
                session_endpoint,
                session_peer_endpoint,
                session_advertised_capabilities: REAL_MPV_SESSION_CAPABILITIES.to_vec(),
                network_mode: if exercise_http {
                    "os-assigned-ipv4-loopback-session-and-http"
                } else {
                    "os-assigned-ipv4-loopback-session"
                },
                media_source: if options.exercise_http_fault {
                    "generated-pcm-au-over-faulting-loopback-http"
                } else if options.exercise_http_stall {
                    "generated-pcm-au-over-stalled-loopback-http"
                } else {
                    "generated-local-pcm-wav"
                },
                mpv_config: "isolated --no-config",
                media_url: media_url.clone(),
                http_endpoint: http_fault_evidence
                    .as_ref()
                    .map(|evidence| evidence.listener_endpoint.clone())
                    .or_else(|| {
                        http_stall_evidence
                            .as_ref()
                            .map(|evidence| evidence.listener_endpoint.clone())
                    }),
                http_evidence_path: if options.exercise_http_fault {
                    Some(http_fault_path.display().to_string())
                } else if options.exercise_http_stall {
                    Some(http_stall_path.display().to_string())
                } else {
                    None
                },
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
            if let (Some(server), Some(evidence)) = (
                hard_failure_http_server.as_ref(),
                media_failure_evidence.as_mut(),
            ) && let Ok(requests) = server.requests()
            {
                evidence.record_requests(requests);
                let _ = write_json_file(&media_failure_path, evidence);
            }
            if let (Some(server), Some(evidence)) =
                (stalled_http_server.as_ref(), http_stall_evidence.as_mut())
                && let Ok(requests) = server.requests()
            {
                evidence.record_requests(requests);
                let _ = write_json_file(&http_stall_path, evidence);
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
            if let Some(server) = hard_failure_http_server.take() {
                match server.release() {
                    Ok(requests) => {
                        if let Some(evidence) = media_failure_evidence.as_mut() {
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
            if let Some(server) = stalled_http_server.take() {
                match server.release() {
                    Ok(requests) => {
                        if let Some(evidence) = http_stall_evidence.as_mut() {
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
            if let Some(evidence) = media_failure_evidence.as_mut() {
                evidence.result = "failed".to_owned();
                evidence.owned_mpv_terminated_after_gui_exit =
                    !process_is_running(evidence.initial_pid);
                evidence.error = Some(redact_real_mpv_error(&error));
                let _ = write_json_file(&media_failure_path, evidence);
            }
            if let Some(evidence) = http_stall_evidence.as_mut() {
                evidence.result = "failed".to_owned();
                evidence.owned_mpv_terminated_after_gui_exit = evidence
                    .initial_pid
                    .is_some_and(|pid| !process_is_running(pid));
                evidence.error = Some(redact_real_mpv_error(&error));
                let _ = write_json_file(&http_stall_path, evidence);
            }
            state.result = "failed".to_owned();
            state.stage = format!("{}-failed", state.stage);
            state.error = Some(redact_real_mpv_error(&error));
            let _ = write_json_file(&state_path, &state);
            Err(error)
        }
    }
}

fn require_real_mpv_vertical_platform() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err(
            "the genuine native GUI-to-real-mpv vertical currently requires Windows UI Automation and Windows mpv IPC"
                .to_owned(),
        )
    }
}

fn parse_real_mpv_vertical_options(args: &[String]) -> Result<RealMpvVerticalOptions, String> {
    let mut binary_path = None;
    let mut mpv_path = None;
    let mut artifact_dir = None;
    let mut timeout = Duration::from_secs(30);
    let mut exercise_recovery = false;
    let mut exercise_http_fault = false;
    let mut exercise_http_stall = false;
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
            "--exercise-stalled-http" => {
                exercise_http_stall = true;
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
                    "unknown real-mpv vertical argument {argument:?}; expected --binary, --mpv, --artifact-dir, optional --timeout-ms, optional --exercise-owned-mpv-recovery, optional --exercise-faulting-http-recovery, and optional --exercise-stalled-http"
                ));
            }
        }
    }
    if usize::from(exercise_recovery)
        + usize::from(exercise_http_fault)
        + usize::from(exercise_http_stall)
        > 1
    {
        return Err(
            "--exercise-owned-mpv-recovery, --exercise-faulting-http-recovery, and --exercise-stalled-http are mutually exclusive"
                .to_owned(),
        );
    }
    if exercise_http_stall && timeout < REAL_MPV_HTTP_STALL_MAXIMUM_RECOVERY_WAIT {
        return Err(format!(
            "--exercise-stalled-http requires --timeout-ms of at least {}",
            REAL_MPV_HTTP_STALL_MAXIMUM_RECOVERY_WAIT.as_millis()
        ));
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
        exercise_http_stall,
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
    trusted_domains: Vec<String>,
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
        // Every real-mpv mode exercises the GUI's playlist-backed media-open
        // workflow. Keep the configured client capability aligned with the
        // canonical playlist exchange that the fixture requires.
        shared_playlist_enabled: Some(true),
        show_osd: Some(false),
        chat_input_enabled: Some(false),
        chat_output_enabled: Some(false),
        check_for_updates_automatically: Some(false),
        only_switch_to_trusted_domains: (!trusted_domains.is_empty()).then_some(true),
        trusted_domains: (!trusted_domains.is_empty()).then_some(trusted_domains),
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
        paused_for_cache = event_data and event_data.paused_for_cache or nil,
        eof_reached = event_data and event_data.eof_reached or nil,
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
mp.observe_property("paused-for-cache", "bool", function(_, value)
    if value ~= nil then
        emit("paused-for-cache", {{ paused_for_cache = value }})
    end
end)
mp.observe_property("eof-reached", "bool", function(_, value)
    if value ~= nil then
        emit("eof-reached", {{ eof_reached = value }})
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

fn read_lifecycle_records(path: &Path) -> Result<Vec<serde_json::Value>, String> {
    let contents = fs::read_to_string(path).map_err(|error| {
        format!(
            "failed to read shared lifecycle evidence {}: {error}",
            path.display()
        )
    })?;
    if !contents.is_empty() && !contents.ends_with('\n') {
        return Err(format!(
            "shared lifecycle evidence {} ended with an incomplete record",
            path.display()
        ));
    }
    contents
        .lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
        .map(|(index, line)| {
            let value: serde_json::Value = serde_json::from_str(line).map_err(|error| {
                format!(
                    "shared lifecycle evidence {} line {} was invalid JSON: {error}",
                    path.display(),
                    index + 1
                )
            })?;
            if !value.is_object() {
                return Err(format!(
                    "shared lifecycle evidence {} line {} was not an object",
                    path.display(),
                    index + 1
                ));
            }
            Ok(value)
        })
        .collect()
}

fn wait_for_lifecycle_snapshot(
    path: &Path,
    timeout: Duration,
) -> Result<Vec<serde_json::Value>, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match read_lifecycle_records(path) {
            Ok(records) => return Ok(records),
            Err(error) => {
                if Instant::now() >= deadline {
                    return Err(error);
                }
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn wait_for_lifecycle_transition(
    path: &Path,
    start_index: usize,
    transition: &str,
    timeout: Duration,
) -> Result<(usize, serde_json::Value), String> {
    let deadline = Instant::now() + timeout;
    let mut last_records = Vec::new();
    loop {
        let last_error = match read_lifecycle_records(path) {
            Ok(records) => {
                if let Some((index, record)) =
                    records
                        .iter()
                        .enumerate()
                        .skip(start_index)
                        .find(|(_, record)| {
                            record
                                .get("record_type")
                                .and_then(serde_json::Value::as_str)
                                == Some("transition")
                                && record.get("transition").and_then(serde_json::Value::as_str)
                                    == Some(transition)
                        })
                {
                    return Ok((index, record.clone()));
                }
                last_records = records;
                None
            }
            Err(error) => Some(error),
        };
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for lifecycle transition {transition:?} after record {start_index}; last_error={last_error:?}; records={last_records:?}"
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn required_lifecycle_string(
    record: &serde_json::Value,
    field: &str,
    transition: &str,
) -> Result<String, String> {
    record
        .get(field)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            format!("lifecycle transition {transition} omitted nonempty string field {field}")
        })
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

fn send_selected_media(
    server: &MockSessionServer,
    target: &Path,
    phase: &str,
) -> Result<(), String> {
    for message in [
        serde_json::json!({"Set":{"playlistChange":{"files":[target],"user":"remote-controller"}}}),
        serde_json::json!({"Set":{"playlistIndex":{"index":0,"user":"remote-controller"}}}),
        serde_json::json!({"State":{"playstate":{"position":0.0,"paused":true,"doSeek":false,"setBy":"remote-controller"}}}),
    ] {
        server.send_authoritative_line(message.to_string(), phase)?;
    }
    Ok(())
}

fn exercise_missing_media_resolution(
    server: &MockSessionServer,
    lifecycle_path: &Path,
    missing_path: &Path,
    restored_path: &Path,
    initial_pid: u32,
    timeout: Duration,
) -> Result<MissingMediaEvidence, String> {
    if missing_path
        .try_exists()
        .map_err(|error| error.to_string())?
    {
        return Err("missing-media fixture target unexpectedly exists".to_owned());
    }
    let boundary = wait_for_lifecycle_snapshot(lifecycle_path, timeout)?.len();
    send_selected_media(server, missing_path, "missing-media resolution selection")?;
    let (missing_index, record) =
        wait_for_lifecycle_transition(lifecycle_path, boundary, "MEDIA-MISSING-001", timeout)?;
    let event_id = required_lifecycle_string(&record, "event_id", "MEDIA-MISSING-001")?;
    let emitter = required_lifecycle_string(&record, "emitter", "MEDIA-MISSING-001")?;
    let process_role = required_lifecycle_string(&record, "process_role", "MEDIA-MISSING-001")?;
    if emitter != "gui-real-mpv" || process_role != "client" {
        return Err("missing-media transition was not emitted by the GUI resolver".to_owned());
    }
    send_selected_media(
        server,
        restored_path,
        "restore media before owned-player loss",
    )?;
    wait_for_lifecycle_transition(
        lifecycle_path,
        missing_index.saturating_add(1),
        "MEDIA-RESOLVE-001",
        timeout,
    )?;
    Ok(MissingMediaEvidence {
        path: missing_path.display().to_string(),
        event_id,
        emitter,
        process_role,
        initial_pid,
    })
}

fn wait_for_replacement_media_loaded(
    observation_path: &Path,
    relaunch_index: usize,
    recovered_pid: u32,
    recovered_ipc_endpoint: &str,
    media_path: &Path,
    timeout: Duration,
) -> Result<(usize, MpvObservation), String> {
    wait_for_mpv_observation(
        observation_path,
        relaunch_index.saturating_add(1),
        timeout,
        "file-loaded for generated local media from the replacement real mpv",
        |observation| {
            observation.event == "file-loaded"
                && observation.pid == Some(recovered_pid)
                && observation.path.as_deref().is_some_and(|observed| {
                    observed_media_path_matches(Path::new(observed), media_path)
                })
                && observation.ipc_endpoint.as_deref() == Some(recovered_ipc_endpoint)
        },
    )
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

fn validate_hard_failure_http_request_accounting(
    requests: &[HttpRequestEvidence],
) -> Result<(), String> {
    if !requests.iter().any(|request| request.method == "GET") {
        return Err(format!(
            "hard-failure HTTP request accounting omitted a media GET: {requests:?}"
        ));
    }
    for (index, request) in requests.iter().enumerate() {
        if request.ordinal != index + 1
            || !matches!(request.method.as_str(), "GET" | "HEAD")
            || request.path != REAL_MPV_MEDIA_FAILURE_ROUTE
            || !request.peer_ipv4_loopback
            || request.status_code != 404
            || request.content_length_header != Some(0)
            || request.transfer_encoding.is_some()
            || request.transmitted_body_bytes != 0
            || request.framing_fault_injected
            || request.disconnected_early
            || request.write_error.is_some()
        {
            return Err(format!(
                "hard-failure HTTP request accounting drifted at row {index}: {request:?}"
            ));
        }
        require_ipv4_loopback_endpoint(&request.peer_endpoint, "hard-failure HTTP connected peer")?;
    }
    Ok(())
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
                || request.content_length_header != Some(generated_media_bytes)
                || request.transfer_encoding.is_some()
                || request.transmitted_body_bytes != 0
                || request.framing_fault_injected
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
            "faulting HTTP expected exactly two media GETs (one malformed chunked, one complete); requests={requests:?}"
        ));
    }
    let short = media_gets[0];
    if !short.disconnected_early
        || short.content_length_header.is_some()
        || short.transfer_encoding.as_deref() != Some("chunked")
        || short.transmitted_body_bytes < REAL_MPV_HTTP_FAULT_MINIMUM_PREFIX_BYTES
        || !short.framing_fault_injected
        || short.transmitted_body_bytes >= generated_media_bytes
    {
        return Err(format!(
            "faulting HTTP first media GET was not the exact malformed chunked response: {short:?}"
        ));
    }
    let complete = media_gets[1];
    if complete.disconnected_early
        || complete.content_length_header != Some(generated_media_bytes)
        || complete.transfer_encoding.is_some()
        || complete.framing_fault_injected
        || complete.transmitted_body_bytes != generated_media_bytes
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

fn validate_stalled_http_request_accounting(
    requests: &[HttpStallRequestEvidence],
    generated_media_bytes: usize,
    require_stalled_connection_released: bool,
) -> Result<(), String> {
    if requests.is_empty() {
        return Err("stalled HTTP request accounting was empty".to_owned());
    }
    for (index, request) in requests.iter().enumerate() {
        if request.ordinal != index + 1
            || request.path != REAL_MPV_HTTP_STALL_ROUTE
            || !request.peer_ipv4_loopback
            || request.status_code != 200
            || request.write_error.is_some()
        {
            return Err(format!(
                "stalled HTTP request accounting drifted at row {index}: {request:?}"
            ));
        }
        require_ipv4_loopback_endpoint(&request.peer_endpoint, "stalled HTTP connected peer")?;
        if request.method == "HEAD" {
            if request.content_length_header != Some(generated_media_bytes)
                || request.transfer_encoding.is_some()
                || request.transmitted_body_bytes != 0
                || request.stall_injected
                || request.stalled_for_ms.is_some()
                || request.server_response_retained_at_recovery_get
                || !request.connection_released
                || !request.response_completed
            {
                return Err(format!(
                    "stalled HTTP HEAD probe unexpectedly carried a body or stall: {request:?}"
                ));
            }
        } else if request.method == "GET" {
            if request.range_header.as_deref() != Some("bytes=0-") {
                return Err(format!(
                    "stalled HTTP media GET did not use the exact byte-zero non-seekable contract: {request:?}"
                ));
            }
        } else {
            return Err(format!(
                "stalled HTTP request method was not explicitly accounted: {request:?}"
            ));
        }
    }
    let media_gets = requests
        .iter()
        .filter(|request| request.method == "GET")
        .collect::<Vec<_>>();
    if media_gets.len() != 2 {
        return Err(format!(
            "stalled HTTP expected exactly one open stalled GET and one complete recovery GET; requests={requests:?}"
        ));
    }
    let stalled = media_gets[0];
    let stalled_for_ms = stalled.stalled_for_ms.ok_or_else(|| {
        format!("stalled HTTP first GET did not record the recovery-request boundary: {stalled:?}")
    })?;
    if stalled.content_length_header != Some(generated_media_bytes)
        || stalled.transfer_encoding.is_some()
        || stalled.transmitted_body_bytes != REAL_MPV_HTTP_STALL_PREFIX_BYTES
        || !stalled.stall_injected
        || stalled_for_ms < REAL_MPV_HTTP_STALL_MINIMUM_DURATION.as_millis()
        || stalled_for_ms > REAL_MPV_HTTP_STALL_MAXIMUM_RECOVERY_WAIT.as_millis()
        || !stalled.server_response_retained_at_recovery_get
        || stalled.connection_released != require_stalled_connection_released
        || stalled.response_completed
    {
        return Err(format!(
            "stalled HTTP first media GET did not retain the exact bounded open byte-silent response: {stalled:?}"
        ));
    }
    let complete = media_gets[1];
    if complete.content_length_header != Some(generated_media_bytes)
        || complete.transfer_encoding.is_some()
        || complete.transmitted_body_bytes != generated_media_bytes
        || complete.stall_injected
        || complete.stalled_for_ms.is_some()
        || complete.server_response_retained_at_recovery_get
        || !complete.connection_released
        || !complete.response_completed
    {
        return Err(format!(
            "stalled HTTP recovery GET was not a complete response: {complete:?}"
        ));
    }
    if requests
        .iter()
        .filter(|request| request.stall_injected)
        .count()
        != 1
    {
        return Err(format!(
            "stalled HTTP did not retain exactly one stalled response: {requests:?}"
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
    fn real_mpv_platform_preflight_matches_the_compiled_target() {
        let result = require_real_mpv_vertical_platform();
        #[cfg(target_os = "windows")]
        assert!(result.is_ok());
        #[cfg(not(target_os = "windows"))]
        assert_eq!(
            result.expect_err("non-Windows targets must fail before launch"),
            "the genuine native GUI-to-real-mpv vertical currently requires Windows UI Automation and Windows mpv IPC"
        );
    }

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
        assert!(!parsed.exercise_http_stall);

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
        let mut stalled_http_args = args.clone();
        let timeout_index = stalled_http_args
            .iter()
            .position(|value| value == "1234")
            .expect("timeout argument");
        stalled_http_args[timeout_index] = "50000".to_owned();
        stalled_http_args.push("--exercise-stalled-http".to_owned());
        assert!(
            parse_real_mpv_vertical_options(&stalled_http_args)
                .expect("stalled HTTP options should parse")
                .exercise_http_stall
        );
        let mut too_short_stalled_http_args = args.clone();
        too_short_stalled_http_args.push("--exercise-stalled-http".to_owned());
        assert!(parse_real_mpv_vertical_options(&too_short_stalled_http_args).is_err());

        let mut conflicting_args = recovery_args.clone();
        conflicting_args.push("--exercise-faulting-http-recovery".to_owned());
        assert!(parse_real_mpv_vertical_options(&conflicting_args).is_err());
        let mut recovery_stall_conflict = recovery_args;
        let timeout_index = recovery_stall_conflict
            .iter()
            .position(|value| value == "1234")
            .expect("timeout argument");
        recovery_stall_conflict[timeout_index] = "50000".to_owned();
        recovery_stall_conflict.push("--exercise-stalled-http".to_owned());
        assert!(parse_real_mpv_vertical_options(&recovery_stall_conflict).is_err());
        let mut fault_stall_conflict = http_fault_args;
        let timeout_index = fault_stall_conflict
            .iter()
            .position(|value| value == "1234")
            .expect("timeout argument");
        fault_stall_conflict[timeout_index] = "50000".to_owned();
        fault_stall_conflict.push("--exercise-stalled-http".to_owned());
        assert!(parse_real_mpv_vertical_options(&fault_stall_conflict).is_err());

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
        assert!(script.contains(r#"mp.observe_property("paused-for-cache""#));
        assert!(script.contains("paused_for_cache = value"));
        assert!(script.contains(r#"mp.observe_property("eof-reached""#));
        assert!(script.contains("eof_reached = value"));
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
    fn replacement_media_already_observed_during_attestation_is_retained() {
        let root = std::env::temp_dir().join(format!(
            "sorotte-replacement-observation-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        fs::create_dir_all(&root).unwrap();
        let media = root.join("media.wav");
        fs::write(&media, b"fixture").unwrap();
        let path = root.join("observations.jsonl");
        let loaded = serde_json::json!({
            "event": "file-loaded", "pid": 72, "path": media,
            "ipc_endpoint": "replacement"
        });
        let mut rows = vec![
            loaded.clone(), // A matching record before the relaunch is stale.
            serde_json::json!({"event":"pause","pid":72,"pause":true,"ipc_endpoint":"replacement"}),
            serde_json::json!({"event":"file-loaded","pid":71,"path":media,"ipc_endpoint":"replacement"}),
            serde_json::json!({"event":"file-loaded","pid":72,"path":media,"ipc_endpoint":"old-endpoint"}),
            serde_json::json!({"event":"file-loaded","pid":72,"path":root.join("other.wav"),"ipc_endpoint":"replacement"}),
            loaded,
        ];
        let write_rows = |rows: &[serde_json::Value]| {
            fs::write(
                &path,
                rows.iter()
                    .map(|row| format!("{row}\n"))
                    .collect::<String>(),
            )
            .unwrap();
        };
        write_rows(&rows);
        // All events arrived before ownership and screenshot checks finished.
        let (index, _) =
            wait_for_replacement_media_loaded(&path, 1, 72, "replacement", &media, Duration::ZERO)
                .unwrap();
        assert_eq!(index, 5);
        rows.pop();
        write_rows(&rows);
        assert!(
            wait_for_replacement_media_loaded(&path, 1, 72, "replacement", &media, Duration::ZERO)
                .is_err()
        );
        fs::remove_dir_all(root).unwrap();
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
        let mut initial_playstate = String::new();
        reader
            .read_line(&mut initial_playstate)
            .expect("read initial authoritative playstate");
        let recorded = server
            .recv_playlist_exchange(Duration::from_secs(2), "unit playlist echo")
            .expect("record playlist exchange");
        assert_eq!(recorded.0, playlist_request);
        assert_eq!(recorded.1, playlist_change_echo.trim());
        assert_eq!(recorded.2, playlist_index_request);
        assert_eq!(recorded.3, playlist_index_echo.trim());
        assert_eq!(recorded.4, initial_playstate.trim());
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
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&recorded.4)
                .expect("valid initial playstate"),
            serde_json::json!({
                "State": {
                    "playstate": {
                        "position": 0.0,
                        "paused": true,
                        "doSeek": false,
                        "setBy": REAL_MPV_LOOPBACK_USERNAME,
                    }
                }
            })
        );

        for (counter, playstate) in [
            (
                1,
                Some(serde_json::json!({"position": 0.0, "paused": true, "doSeek": true})),
            ),
            (
                2,
                Some(serde_json::json!({"position": 0.0, "paused": true})),
            ),
            (3, None),
        ] {
            let mut request =
                serde_json::json!({"State": {"ignoringOnTheFly": {"client": counter}}});
            if let Some(playstate) = playstate {
                request["State"]["playstate"] = playstate;
            }
            writeln!(stream, "{request}").expect("write client seek acknowledgement request");
            let mut echo = String::new();
            reader
                .read_line(&mut echo)
                .expect("read client counter acknowledgement");
            let echo: serde_json::Value = serde_json::from_str(&echo).unwrap();
            assert_eq!(
                echo.pointer("/State/ignoringOnTheFly"),
                Some(&serde_json::json!({"client": counter}))
            );
            if counter == 1 {
                let (_, recorded_echo) = server
                    .recv_playstate_exchange(Duration::from_secs(2), "seek counter")
                    .unwrap();
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(&recorded_echo).unwrap(),
                    echo
                );
                assert_eq!(
                    echo.pointer("/State/playstate/doSeek"),
                    Some(&serde_json::json!(true))
                );
            } else {
                assert_eq!(
                    echo,
                    serde_json::json!({"State": {"ignoringOnTheFly": {"client": counter}}})
                );
            }
        }

        for (label, position, paused) in [("play", 1.25, false), ("pause", 2.0, true)] {
            let request = serde_json::json!({
                "State": {
                    "playstate": {
                        "position": position,
                        "paused": paused,
                        "doSeek": false,
                    }
                }
            })
            .to_string();
            writeln!(stream, "{request}")
                .unwrap_or_else(|error| panic!("write {label} playstate: {error}"));
            let mut echo = String::new();
            reader
                .read_line(&mut echo)
                .unwrap_or_else(|error| panic!("read {label} playstate echo: {error}"));
            let exchange = server
                .recv_playstate_exchange(Duration::from_secs(2), label)
                .unwrap_or_else(|error| panic!("record {label} playstate exchange: {error}"));
            assert_eq!(exchange.0, request);
            assert_eq!(exchange.1, echo.trim());
            let echo_json: serde_json::Value =
                serde_json::from_str(&exchange.1).expect("valid authoritative playstate echo");
            assert_eq!(
                echo_json.pointer("/State/playstate/paused"),
                Some(&serde_json::json!(paused))
            );
            assert_eq!(
                echo_json.pointer("/State/playstate/setBy"),
                Some(&serde_json::json!(REAL_MPV_LOOPBACK_USERNAME))
            );
        }
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
    fn playlist_echo_server_preserves_a_fragmented_frame_across_read_timeout() {
        let media_url = "http://127.0.0.1:49152/generated-fault.au";
        let (server, _, mut stream, _) = connect_playlist_echo_test_server(media_url);
        let frame = serde_json::json!({
            "State": {
                "ping": {
                    "clientLatencyCalculation": 1.0,
                    "clientRtt": 0.0,
                    "serverRtt": 0.0,
                }
            }
        })
        .to_string();
        let split = frame.len() / 2;
        stream
            .write_all(&frame.as_bytes()[..split])
            .expect("write first heartbeat fragment");
        thread::sleep(Duration::from_millis(150));
        stream
            .write_all(&frame.as_bytes()[split..])
            .expect("write second heartbeat fragment");
        stream
            .write_all(b"\n")
            .expect("terminate fragmented heartbeat");

        let receive_error = server
            .recv_playlist_exchange(Duration::from_secs(2), "fragmented-frame rejection")
            .expect_err("widened fragmented heartbeat must not produce playlist evidence");
        assert!(receive_error.contains("closed"));
        let release_error = server
            .release("fragmented-frame rejection")
            .expect_err("widened fragmented heartbeat must fail the fixture");
        assert!(
            release_error.contains(
                "unexpected client frame before playlistChange (redacted shape: State.ping)"
            ),
            "fragmented frame lost its semantic rejection reason: {release_error}"
        );
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
    fn hard_failure_http_connection_waits_for_headers_on_a_nonblocking_socket() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, peer) = listener.accept().unwrap();
        // Windows accepts inherit the listener's nonblocking mode. Set it
        // explicitly so this regression also exercises that state on Unix.
        stream.set_nonblocking(true).unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            started_tx.send(()).unwrap();
            handle_hard_failure_http_connection(stream, peer, 1)
        });
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        thread::sleep(Duration::from_millis(50));
        client
            .write_all(
                format!("GET {REAL_MPV_MEDIA_FAILURE_ROUTE} HTTP/1.1\r\nHost: localhost\r\n\r\n")
                    .as_bytes(),
            )
            .unwrap();
        let evidence = server.join().unwrap().unwrap();
        assert_eq!(evidence.status_code, 404);
        assert_eq!(evidence.method, "GET");
    }

    #[test]
    fn hard_failure_http_server_returns_only_retained_strict_404_evidence() {
        let server = HardFailureLoopbackHttpServer::start().expect("start hard-failure fixture");
        let endpoint = server.endpoint();
        require_ipv4_loopback_endpoint(&endpoint, "unit hard-failure HTTP listener")
            .expect("strict listener");
        let mut stream = TcpStream::connect(&endpoint).expect("connect hard-failure fixture");
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .expect("set hard-failure response timeout");
        stream
            .write_all(
                format!(
                    "GET {REAL_MPV_MEDIA_FAILURE_ROUTE} HTTP/1.1\r\nHost: {endpoint}\r\nRange: bytes=0-\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .expect("write hard-failure GET");
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .expect("read hard-failure response");
        assert_eq!(
            response,
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n"
        );
        let requests = server
            .wait_for_media_get(Duration::from_secs(3))
            .expect("hard-failure GET evidence");
        validate_hard_failure_http_request_accounting(&requests)
            .expect("strict hard-failure request accounting");
        let requests = server.release().expect("release hard-failure fixture");
        validate_hard_failure_http_request_accounting(&requests)
            .expect("retained hard-failure request accounting");
    }

    #[test]
    fn stalled_http_server_keeps_first_get_open_while_serving_complete_recovery_get() {
        fn write_get(stream: &mut TcpStream, endpoint: &str) {
            stream
                .write_all(
                    format!(
                        "GET {REAL_MPV_HTTP_STALL_ROUTE} HTTP/1.1\r\nHost: {endpoint}\r\nRange: bytes=0-\r\nConnection: close\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .expect("write stalled HTTP GET");
        }

        let generated_media = pcm_au_bytes(REAL_MPV_HTTP_STALL_DURATION_SECONDS);
        let server =
            StalledLoopbackHttpServer::start(generated_media.clone()).expect("start fixture");
        let endpoint = server.endpoint();
        require_ipv4_loopback_endpoint(&endpoint, "unit stalled HTTP listener")
            .expect("strict listener");

        let mut stalled_stream =
            TcpStream::connect(&endpoint).expect("connect first stalled loopback request");
        stalled_stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("set stalled response timeout");
        write_get(&mut stalled_stream, &endpoint);
        let mut first_response = Vec::new();
        let mut buffer = [0_u8; 16 * 1024];
        let header_end = loop {
            let read = stalled_stream
                .read(&mut buffer)
                .expect("read stalled HTTP response prefix");
            assert_ne!(read, 0, "stalled response closed before exact prefix");
            first_response.extend_from_slice(&buffer[..read]);
            if let Some(index) = first_response
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
            {
                let header_end = index + 4;
                if first_response.len() - header_end >= REAL_MPV_HTTP_STALL_PREFIX_BYTES {
                    break header_end;
                }
            }
        };
        let expected_content_length = format!("Content-Length: {}", generated_media.len());
        assert!(
            first_response[..header_end]
                .windows(expected_content_length.len())
                .any(|window| window == expected_content_length.as_bytes()),
            "stalled response must declare the complete generated body"
        );
        assert_eq!(
            first_response.len() - header_end,
            REAL_MPV_HTTP_STALL_PREFIX_BYTES,
            "stalled response must stop at the deterministic playable prefix"
        );
        stalled_stream
            .set_read_timeout(Some(Duration::from_millis(100)))
            .expect("set byte-silence probe timeout");
        let no_byte_result = stalled_stream.read(&mut buffer[..1]);
        assert!(
            matches!(
                no_byte_result,
                Err(ref error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    )
            ),
            "first response must remain open without another byte: {no_byte_result:?}"
        );

        let mut recovery_stream =
            TcpStream::connect(&endpoint).expect("connect complete recovery request");
        recovery_stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("set recovery response timeout");
        write_get(&mut recovery_stream, &endpoint);
        let mut recovery_response = Vec::new();
        recovery_stream
            .read_to_end(&mut recovery_response)
            .expect("read complete recovery response");
        let recovery_header_end = recovery_response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|index| index + 4)
            .expect("recovery response header boundary");
        assert_eq!(
            recovery_response.len() - recovery_header_end,
            generated_media.len(),
            "recovery response must carry the complete generated body"
        );

        let requests = server
            .wait_for_media_gets(2, Duration::from_secs(5))
            .expect("two media GETs");
        assert_eq!(requests.len(), 2);
        assert!(requests[0].stall_injected);
        assert!(requests[0].server_response_retained_at_recovery_get);
        assert!(!requests[0].connection_released);
        assert!(!requests[0].response_completed);
        assert!(requests[1].response_completed);
        assert!(
            validate_stalled_http_request_accounting(&requests, generated_media.len(), false)
                .is_err(),
            "the fast unit fixture must prove the independent 25-second lower bound is enforced"
        );
        let mut bounded_requests = requests;
        bounded_requests[0].stalled_for_ms = Some(REAL_MPV_HTTP_STALL_MINIMUM_DURATION.as_millis());
        validate_stalled_http_request_accounting(&bounded_requests, generated_media.len(), false)
            .expect("otherwise exact in-flight stalled request accounting");

        let mut released_requests = server.release().expect("release stalled HTTP server");
        assert!(released_requests[0].connection_released);
        released_requests[0].stalled_for_ms =
            Some(REAL_MPV_HTTP_STALL_MINIMUM_DURATION.as_millis());
        validate_stalled_http_request_accounting(&released_requests, generated_media.len(), true)
            .expect("exact final stalled request accounting");
        drop(stalled_stream);
        let rebound =
            TcpListener::bind(&endpoint).expect("stalled HTTP listener socket should be released");
        drop(rebound);
    }

    #[test]
    fn faulting_http_server_accounts_one_malformed_chunked_get_and_one_complete_get() {
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

        server
            .trigger_fault()
            .expect("unit fixture should explicitly release its first fault");
        let short = request(&endpoint, "GET", Some("bytes=0-"));
        assert!(
            short.windows(15).any(|window| window == b"HTTP/1.1 200 OK"),
            "byte-zero GET must receive a non-seekable chunked response"
        );
        assert!(
            short
                .windows(b"Transfer-Encoding: chunked".len())
                .any(|window| window == b"Transfer-Encoding: chunked"),
            "first GET must use explicit chunked transfer framing"
        );
        assert!(
            !short
                .windows(b"Content-Length:".len())
                .any(|window| window == b"Content-Length:"),
            "the malformed chunked response must not also declare a content length"
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
        assert!(
            short.ends_with(b"\r\nnot-a-chunk-size\r\n"),
            "first GET must end with the exact malformed chunk-size boundary"
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
        assert_eq!(requests[1].content_length_header, None);
        assert_eq!(requests[1].transfer_encoding.as_deref(), Some("chunked"));
        assert!(requests[1].framing_fault_injected);
        assert!(
            requests[1].transmitted_body_bytes >= REAL_MPV_HTTP_FAULT_MINIMUM_PREFIX_BYTES
                && requests[1].transmitted_body_bytes
                    < REAL_MPV_HTTP_FAULT_MINIMUM_PREFIX_BYTES + 16 * 1024,
            "pre-triggered unit fault must stop at the first chunk boundary after the playable minimum: {:?}",
            requests[1]
        );
        assert_eq!(requests[2].range_header.as_deref(), Some("bytes=0-"));
        assert_eq!(
            requests[2].content_length_header,
            Some(generated_media.len())
        );
        assert_eq!(requests[2].transfer_encoding, None);
        assert!(!requests[2].framing_fault_injected);
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
            request.transmitted_body_bytes < REAL_MPV_HTTP_FAULT_MINIMUM_PREFIX_BYTES,
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
