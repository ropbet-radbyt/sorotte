use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use sorotte_protocol::{
    HelloPayload, ProtocolError, ProtocolMessage, decode_message_line, encode_message_line,
    extract_hello_from_message,
};
use sorotte_server::{
    DirectedOutboundLine, ServerOutboundDelivery, ServerRuntime, ServerRuntimeError,
};

#[derive(Clone, PartialEq)]
pub struct PythonHandshakeTranscript {
    pub request_line: String,
    pub response_line: String,
    pub response_message: ProtocolMessage,
    pub response_hello: HelloPayload,
}

impl std::fmt::Debug for PythonHandshakeTranscript {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PythonHandshakeTranscript")
            .field("request_line_bytes", &self.request_line.len())
            .field("response_line_bytes", &self.response_line.len())
            .field("response_message", &self.response_message)
            .field("response_hello", &self.response_hello)
            .finish()
    }
}

#[derive(Clone, PartialEq)]
pub struct PythonProtocolStep {
    pub request_line: String,
    pub request_message: ProtocolMessage,
    pub response_lines: Vec<String>,
    pub response_messages: Vec<ProtocolMessage>,
}

impl std::fmt::Debug for PythonProtocolStep {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let response_line_bytes = self
            .response_lines
            .iter()
            .map(String::len)
            .collect::<Vec<_>>();
        formatter
            .debug_struct("PythonProtocolStep")
            .field("request_line_bytes", &self.request_line.len())
            .field("request_message", &self.request_message)
            .field("response_line_bytes", &response_line_bytes)
            .field("response_messages", &self.response_messages)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PythonProtocolTranscript {
    pub steps: Vec<PythonProtocolStep>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyClientSetFileContractProbe {
    pub file_payload_ignored: bool,
    pub empty_payload_ignored: bool,
    pub file_payload_calls: Vec<String>,
    pub empty_payload_calls: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegacyClientUserFileMetadataProbe {
    pub after_set_mixed: BTreeMap<String, Option<Value>>,
    pub after_set_empty: BTreeMap<String, Option<Value>>,
    pub after_list_mixed: BTreeMap<String, Option<Value>>,
    pub after_list_clears: BTreeMap<String, Option<Value>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyPythonPeerChatMessage {
    pub sender: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyPythonPeerSnapshot {
    pub username: String,
    pub room: String,
    pub local_ready: Option<bool>,
    pub local_file_name: Option<String>,
    pub local_controller: Option<bool>,
    pub observed_users: BTreeMap<String, Option<bool>>,
    pub observed_user_file_names: BTreeMap<String, Option<String>>,
    pub observed_user_controllers: BTreeMap<String, Option<bool>>,
    pub playlist: Vec<String>,
    pub playlist_index: Option<usize>,
    pub chat_messages: Vec<LegacyPythonPeerChatMessage>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegacyClientChatSendContractCase {
    pub message: String,
    pub protocol_logged: bool,
    pub server_version: String,
    pub chat_supported: Option<bool>,
    pub max_chat_message_length: Option<usize>,
    pub derive_server_features: bool,
    pub feature_list: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyClientChatSendContractResult {
    pub sent_messages: Vec<String>,
    pub error_messages: Vec<String>,
    pub debug_messages: Vec<String>,
}

#[derive(Clone, PartialEq)]
pub struct ServerRuntimeScenarioStep {
    pub client_id: String,
    pub request_line: String,
    pub advance_seconds: f64,
}

impl std::fmt::Debug for ServerRuntimeScenarioStep {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServerRuntimeScenarioStep")
            .field("client_id", &self.client_id)
            .field("request_line_bytes", &self.request_line.len())
            .field("advance_seconds", &self.advance_seconds)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ServerRuntimeScenarioEvent {
    pub client_id: String,
    pub request_line: String,
    pub outbound_lines: Vec<DirectedOutboundLine>,
}

impl std::fmt::Debug for ServerRuntimeScenarioEvent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServerRuntimeScenarioEvent")
            .field("client_id", &self.client_id)
            .field("request_line_bytes", &self.request_line.len())
            .field("outbound_lines", &self.outbound_lines)
            .finish()
    }
}

// This value is part of controlled-room hash compatibility; keep it byte-stable.
const DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT: &str = "syncplay-rs-controlled-room-v1";
const LEGACY_SERVER_START_TIMEOUT: Duration = Duration::from_secs(6);
const LEGACY_SERVER_STEP_IDLE_WAIT: Duration = Duration::from_millis(60);
const LEGACY_SERVER_STEP_MIN_WAIT: Duration = Duration::from_millis(20);
const LEGACY_SERVER_STEP_MAX_WAIT: Duration = Duration::from_secs(2);
const LEGACY_COMPAT_MISSING_FEATURES_MARKER: &str = "__syncplay_rs_missing_features__";
const LEGACY_SYNCPLAY_UPSTREAM_REPO: &str = "https://github.com/Syncplay/syncplay.git";
const LEGACY_SYNCPLAY_UPSTREAM_REF: &str = "v1.7.5";

static LEGACY_SYNCPLAY_BOOTSTRAP_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug)]
struct LegacyServerClientConnection {
    stream: TcpStream,
    pending_bytes: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum InteropError {
    #[error("legacy syncplay checkout not found at {0}")]
    LegacySyncplayCheckoutMissing(PathBuf),
    #[error("python handshake probe script not found at {0}")]
    PythonHandshakeProbeMissing(PathBuf),
    #[error("python live peer probe script not found at {0}")]
    PythonLivePeerProbeMissing(PathBuf),
    #[error("legacy syncplay server entry script not found at {0}")]
    LegacyServerEntryScriptMissing(PathBuf),
    #[error("failed to spawn python process '{python}': {source}")]
    PythonSpawn {
        python: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to open python process stdin")]
    PythonStdinMissing,
    #[error("failed to write request to python process stdin: {0}")]
    PythonStdinWrite(#[source] std::io::Error),
    #[error("failed while waiting for python process output: {0}")]
    PythonWait(#[source] std::io::Error),
    #[error("python output was not valid UTF-8: {0}")]
    PythonOutputUtf8(#[from] std::string::FromUtf8Error),
    #[error(
        "python probe failed (exit code: {exit_code:?}, stdout: '{stdout}', stderr: '{stderr}')"
    )]
    PythonProbeFailed {
        exit_code: Option<i32>,
        stdout: String,
        stderr: String,
    },
    #[error("python probe returned an empty response")]
    EmptyPythonResponse,
    #[error("invalid python batch response: {0}")]
    InvalidPythonBatchResponse(String),
    #[error(
        "legacy server process exited before becoming reachable (exit code: {exit_code:?}, stdout: '{stdout}', stderr: '{stderr}')"
    )]
    LegacyServerExited {
        exit_code: Option<i32>,
        stdout: String,
        stderr: String,
    },
    #[error(
        "legacy server did not accept connections on port {port} before timeout (stdout: '{stdout}', stderr: '{stderr}')"
    )]
    LegacyServerStartTimeout {
        port: u16,
        stdout: String,
        stderr: String,
    },
    #[error(
        "python live peer process exited before reporting a successful connection (exit code: {exit_code:?}, stdout: '{stdout}', stderr: '{stderr}')"
    )]
    PythonLivePeerExited {
        exit_code: Option<i32>,
        stdout: String,
        stderr: String,
    },
    #[error(
        "python live peer process did not report a successful connection before timeout (stdout: '{stdout}', stderr: '{stderr}')"
    )]
    PythonLivePeerStartTimeout { stdout: String, stderr: String },
    #[error("failed to initialize legacy client stream for '{client_id}': {source}")]
    LegacyClientConnectionInit {
        client_id: String,
        #[source]
        source: std::io::Error,
    },
    #[error("missing legacy client stream for '{0}'")]
    MissingLegacyClient(String),
    #[error("invalid server runtime scenario line: {0}")]
    InvalidScenarioStep(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error(transparent)]
    ServerRuntime(#[from] ServerRuntimeError),
}

pub struct LegacyServerPythonPeerHarness {
    host: String,
    address: String,
    port: u16,
    room: String,
    peer_username: String,
    server_child: Child,
    peer_child: Option<Child>,
    peer_stdin: Option<ChildStdin>,
    peer_status_rx: Option<mpsc::Receiver<String>>,
    peer_stdout_lines: Arc<Mutex<Vec<String>>>,
    peer_stderr_lines: Arc<Mutex<Vec<String>>>,
    next_peer_request_id: u64,
}

mod fixtures;
mod legacy_process;
mod legacy_server;
mod python_peer;
mod python_probe;
mod scenario_replay;
#[cfg(feature = "trace-capture")]
mod trace_capture;

pub use self::fixtures::{
    all_protocol_fixture_names, decode_fixture, decode_protocol_file, fixture_decodes,
    fixture_path, load_server_runtime_scenario_fixture, parse_server_runtime_scenario_steps,
    protocol_fixture, protocol_fixture_dir, scenario_fixture_dir, scenario_fixture_path,
};
pub use self::legacy_process::{
    interop_prerequisites_missing, legacy_syncplay_checkout_dir,
    legacy_syncplay_server_entry_script_path, python_handshake_probe_script_path,
    python_live_peer_probe_script_path,
};
pub use self::legacy_server::{
    run_legacy_server_fanout_roundtrip, run_legacy_server_fanout_roundtrip_with_overrides,
    run_legacy_server_fanout_roundtrip_with_salt,
    run_legacy_server_fanout_roundtrip_with_salt_and_motd_template,
};
pub use self::python_probe::{
    default_rust_client_hello_for_interop, run_python_handshake_roundtrip,
    run_python_handshake_roundtrip_with_hello, run_python_legacy_client_chat_send_contract_batch,
    run_python_legacy_client_set_file_contract_probe,
    run_python_legacy_client_user_file_metadata_probe, run_python_privacy_file_payload_batch,
    run_python_protocol_roundtrip, run_python_same_fileduration_batch,
    run_python_same_fileduration_batch_with_overrides, run_python_same_filename_batch,
    run_python_same_filesize_batch,
};
pub use self::scenario_replay::{
    replay_server_runtime_scenario_fixture, replay_server_runtime_scenario_steps,
    replay_server_runtime_scenario_steps_with_motd_template,
    replay_server_runtime_scenario_steps_with_overrides, run_python_fanout_roundtrip,
    run_python_fanout_roundtrip_with_motd_template, run_python_fanout_roundtrip_with_overrides,
    run_python_fanout_roundtrip_with_tls_available,
};
#[cfg(feature = "trace-capture")]
pub use self::trace_capture::{
    capture_legacy_server_trace_fixture, capture_legacy_server_trace_fixture_with_overrides,
    capture_legacy_server_trace_fixture_with_salt,
    capture_legacy_server_trace_fixture_with_salt_and_motd_template, capture_python_trace_fixture,
    capture_python_trace_fixture_with_motd_template, capture_python_trace_fixture_with_overrides,
};

pub(crate) use self::legacy_process::*;
#[cfg(test)]
pub(crate) use self::python_probe::default_rust_client_hello_for_legacy_live_tls;
pub(crate) use self::python_probe::{
    first_non_empty_stdout_line, python_bin_from_env, run_python_probe_raw_with_overrides,
};

#[cfg(test)]
mod tests;
