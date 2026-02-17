use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use syncplay_protocol::{
    HelloPayload, ProtocolError, ProtocolMessage, decode_message_line, encode_message_line,
    extract_hello_from_message,
};
use syncplay_server::{DirectedOutboundLine, ServerRuntime, ServerRuntimeError};

#[derive(Debug, Clone, PartialEq)]
pub struct PythonHandshakeTranscript {
    pub request_line: String,
    pub response_line: String,
    pub response_message: ProtocolMessage,
    pub response_hello: HelloPayload,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PythonProtocolStep {
    pub request_line: String,
    pub request_message: ProtocolMessage,
    pub response_lines: Vec<String>,
    pub response_messages: Vec<ProtocolMessage>,
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

#[derive(Debug, Clone, PartialEq)]
pub struct ServerRuntimeScenarioStep {
    pub client_id: String,
    pub request_line: String,
    pub advance_seconds: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerRuntimeScenarioEvent {
    pub client_id: String,
    pub request_line: String,
    pub outbound_lines: Vec<DirectedOutboundLine>,
}

const DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT: &str = "syncplay-rs-controlled-room-v1";
const LEGACY_SERVER_START_TIMEOUT: Duration = Duration::from_secs(6);
const LEGACY_SERVER_STEP_IDLE_WAIT: Duration = Duration::from_millis(60);
const LEGACY_SERVER_STEP_MIN_WAIT: Duration = Duration::from_millis(20);
const LEGACY_SERVER_STEP_MAX_WAIT: Duration = Duration::from_secs(2);
const LEGACY_COMPAT_MISSING_FEATURES_MARKER: &str = "__syncplay_rs_missing_features__";

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

pub fn protocol_fixture(name: &str) -> std::io::Result<String> {
    fs::read_to_string(fixture_path(name))
}

pub fn fixture_path(name: &str) -> PathBuf {
    protocol_fixture_dir().join(name)
}

pub fn protocol_fixture_dir() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("..");
    path.push("..");
    path.push("fixtures");
    path.push("protocol");
    path
}

pub fn fixture_decodes(name: &str) -> bool {
    let Ok(contents) = protocol_fixture(name) else {
        return false;
    };
    decode_message_line(&contents).is_ok()
}

pub fn decode_fixture(name: &str) -> Option<ProtocolMessage> {
    let contents = protocol_fixture(name).ok()?;
    decode_message_line(&contents).ok()
}

pub fn all_protocol_fixture_names() -> std::io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(protocol_fixture_dir())? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            names.push(entry.file_name().to_string_lossy().to_string());
        }
    }
    names.sort();
    Ok(names)
}

pub fn decode_protocol_file(path: &Path) -> bool {
    let Ok(contents) = fs::read_to_string(path) else {
        return false;
    };
    decode_message_line(&contents).is_ok()
}

pub fn scenario_fixture_path(name: &str) -> PathBuf {
    scenario_fixture_dir().join(name)
}

pub fn scenario_fixture_dir() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("..");
    path.push("..");
    path.push("fixtures");
    path.push("scenarios");
    path
}

pub fn load_server_runtime_scenario_fixture(
    name: &str,
) -> Result<Vec<ServerRuntimeScenarioStep>, InteropError> {
    let contents = fs::read_to_string(scenario_fixture_path(name))?;
    parse_server_runtime_scenario_steps(&contents)
}

pub fn parse_server_runtime_scenario_steps(
    json_lines: &str,
) -> Result<Vec<ServerRuntimeScenarioStep>, InteropError> {
    let mut steps = Vec::new();
    for (line_number, line) in json_lines.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let parsed: Value = serde_json::from_str(trimmed)?;
        let client_id = parsed
            .get("client")
            .and_then(Value::as_str)
            .filter(|client| !client.trim().is_empty())
            .ok_or_else(|| {
                InteropError::InvalidScenarioStep(format!(
                    "line {} is missing non-empty 'client' field",
                    line_number + 1
                ))
            })?;
        let request_value = parsed.get("message").ok_or_else(|| {
            InteropError::InvalidScenarioStep(format!(
                "line {} is missing 'message' field",
                line_number + 1
            ))
        })?;
        let advance_seconds = match parsed.get("advanceSeconds") {
            Some(Value::Number(number)) => number.as_f64().ok_or_else(|| {
                InteropError::InvalidScenarioStep(format!(
                    "line {} has non-finite 'advanceSeconds' value",
                    line_number + 1
                ))
            })?,
            Some(_) => {
                return Err(InteropError::InvalidScenarioStep(format!(
                    "line {} has non-numeric 'advanceSeconds' field",
                    line_number + 1
                )));
            }
            None => 0.0,
        };
        if !advance_seconds.is_finite() || advance_seconds < 0.0 {
            return Err(InteropError::InvalidScenarioStep(format!(
                "line {} has invalid 'advanceSeconds' value",
                line_number + 1
            )));
        }
        let request_line = serde_json::to_string(request_value)?;

        // Validate each scripted request decodes as a typed protocol message.
        let _ = decode_message_line(&request_line)?;

        steps.push(ServerRuntimeScenarioStep {
            client_id: client_id.to_owned(),
            request_line,
            advance_seconds,
        });
    }
    Ok(steps)
}

pub fn replay_server_runtime_scenario_steps(
    steps: &[ServerRuntimeScenarioStep],
) -> Result<Vec<ServerRuntimeScenarioEvent>, InteropError> {
    replay_server_runtime_scenario_steps_with_motd_template(steps, None)
}

pub fn replay_server_runtime_scenario_steps_with_motd_template(
    steps: &[ServerRuntimeScenarioStep],
    motd_template: Option<&str>,
) -> Result<Vec<ServerRuntimeScenarioEvent>, InteropError> {
    replay_server_runtime_scenario_steps_with_overrides(steps, motd_template, false)
}

pub fn replay_server_runtime_scenario_steps_with_overrides(
    steps: &[ServerRuntimeScenarioStep],
    motd_template: Option<&str>,
    persistent_rooms_enabled: bool,
) -> Result<Vec<ServerRuntimeScenarioEvent>, InteropError> {
    replay_server_runtime_scenario_steps_with_full_overrides(
        steps,
        motd_template,
        persistent_rooms_enabled,
        &[],
    )
}

fn replay_server_runtime_scenario_steps_with_full_overrides(
    steps: &[ServerRuntimeScenarioStep],
    motd_template: Option<&str>,
    persistent_rooms_enabled: bool,
    permanent_rooms: &[&str],
) -> Result<Vec<ServerRuntimeScenarioEvent>, InteropError> {
    let mut runtime = motd_template
        .map(str::trim)
        .filter(|template| !template.is_empty())
        .map_or_else(ServerRuntime::default, ServerRuntime::with_motd_template);
    runtime.set_persistent_rooms_enabled(persistent_rooms_enabled);
    let temporary_rooms_db_path = if persistent_rooms_enabled && !permanent_rooms.is_empty() {
        let path = create_temporary_legacy_rooms_db_file_path()?;
        runtime.set_persistent_rooms_db_path(Some(path.clone()))?;
        Some(path)
    } else {
        None
    };
    runtime.set_permanent_rooms(permanent_rooms.iter().copied().map(str::to_owned));
    runtime.set_time_now_override_seconds(Some(0.0));
    let result = (|| {
        let mut events = Vec::with_capacity(steps.len());
        for step in steps {
            let mut outbound_lines =
                runtime.advance_time_and_collect_fanout(step.advance_seconds)?;
            outbound_lines.extend(runtime.handle_line_fanout(&step.client_id, &step.request_line)?);
            events.push(ServerRuntimeScenarioEvent {
                client_id: step.client_id.clone(),
                request_line: step.request_line.clone(),
                outbound_lines,
            });
        }
        Ok(events)
    })();
    if let Some(path) = temporary_rooms_db_path {
        let _ = fs::remove_file(path);
    }
    result
}

pub fn replay_server_runtime_scenario_fixture(
    name: &str,
) -> Result<Vec<ServerRuntimeScenarioEvent>, InteropError> {
    let steps = load_server_runtime_scenario_fixture(name)?;
    replay_server_runtime_scenario_steps(&steps)
}

pub fn run_python_fanout_roundtrip(
    steps: &[ServerRuntimeScenarioStep],
) -> Result<Vec<ServerRuntimeScenarioEvent>, InteropError> {
    run_python_fanout_roundtrip_with_motd_template(steps, None)
}

pub fn run_python_fanout_roundtrip_with_tls_available(
    steps: &[ServerRuntimeScenarioStep],
    tls_available: bool,
) -> Result<Vec<ServerRuntimeScenarioEvent>, InteropError> {
    run_python_fanout_roundtrip_with_full_overrides(steps, None, false, &[], tls_available)
}

pub fn run_python_fanout_roundtrip_with_motd_template(
    steps: &[ServerRuntimeScenarioStep],
    motd_template: Option<&str>,
) -> Result<Vec<ServerRuntimeScenarioEvent>, InteropError> {
    run_python_fanout_roundtrip_with_overrides(steps, motd_template, false)
}

pub fn run_python_fanout_roundtrip_with_overrides(
    steps: &[ServerRuntimeScenarioStep],
    motd_template: Option<&str>,
    persistent_rooms_enabled: bool,
) -> Result<Vec<ServerRuntimeScenarioEvent>, InteropError> {
    run_python_fanout_roundtrip_with_full_overrides(
        steps,
        motd_template,
        persistent_rooms_enabled,
        &[],
        false,
    )
}

fn run_python_fanout_roundtrip_with_full_overrides(
    steps: &[ServerRuntimeScenarioStep],
    motd_template: Option<&str>,
    persistent_rooms_enabled: bool,
    permanent_rooms: &[&str],
    tls_available: bool,
) -> Result<Vec<ServerRuntimeScenarioEvent>, InteropError> {
    if steps.is_empty() {
        return Ok(Vec::new());
    }

    let payload = serde_json::to_vec(&json!({
        "events": steps
            .iter()
            .map(|step| json!({
                "client": step.client_id,
                "line": step.request_line,
                "advanceSeconds": step.advance_seconds,
            }))
            .collect::<Vec<_>>(),
    }))?;
    let stdout = run_python_probe_raw_with_overrides(
        &["--fanout-batch"],
        &payload,
        motd_template,
        persistent_rooms_enabled,
        permanent_rooms,
        tls_available,
    )?;
    let stdout_line =
        first_non_empty_stdout_line(&stdout).ok_or(InteropError::EmptyPythonResponse)?;
    let parsed: Value = serde_json::from_str(stdout_line)?;
    let output_sets = parsed
        .get("outputs")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            InteropError::InvalidPythonBatchResponse(
                "missing outputs array for fanout response".to_owned(),
            )
        })?;

    if output_sets.len() != steps.len() {
        return Err(InteropError::InvalidPythonBatchResponse(format!(
            "fanout response count mismatch: expected {}, got {}",
            steps.len(),
            output_sets.len()
        )));
    }

    let mut events = Vec::with_capacity(steps.len());
    for (index, output_set) in output_sets.iter().enumerate() {
        let output_values = output_set.as_array().ok_or_else(|| {
            InteropError::InvalidPythonBatchResponse(format!(
                "outputs[{index}] should be an array of directed outputs"
            ))
        })?;

        let mut outbound_lines = Vec::with_capacity(output_values.len());
        for output_value in output_values {
            let directed_client = output_value
                .get("client")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    InteropError::InvalidPythonBatchResponse(format!(
                        "outputs[{index}] entry is missing client field"
                    ))
                })?;
            let directed_message = output_value.get("message").ok_or_else(|| {
                InteropError::InvalidPythonBatchResponse(format!(
                    "outputs[{index}] entry is missing message field"
                ))
            })?;
            let line = serde_json::to_string(directed_message)?;
            let _ = decode_message_line(&line)?;
            outbound_lines.push(DirectedOutboundLine {
                client_id: directed_client.to_owned(),
                line,
            });
        }

        events.push(ServerRuntimeScenarioEvent {
            client_id: steps[index].client_id.clone(),
            request_line: steps[index].request_line.clone(),
            outbound_lines,
        });
    }

    Ok(events)
}

pub fn run_legacy_server_fanout_roundtrip(
    steps: &[ServerRuntimeScenarioStep],
) -> Result<Vec<ServerRuntimeScenarioEvent>, InteropError> {
    run_legacy_server_fanout_roundtrip_with_salt(steps, DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT)
}

pub fn run_legacy_server_fanout_roundtrip_with_salt(
    steps: &[ServerRuntimeScenarioStep],
    controlled_room_salt: &str,
) -> Result<Vec<ServerRuntimeScenarioEvent>, InteropError> {
    run_legacy_server_fanout_roundtrip_with_salt_and_motd_template(
        steps,
        controlled_room_salt,
        None,
    )
}

pub fn run_legacy_server_fanout_roundtrip_with_salt_and_motd_template(
    steps: &[ServerRuntimeScenarioStep],
    controlled_room_salt: &str,
    motd_template: Option<&str>,
) -> Result<Vec<ServerRuntimeScenarioEvent>, InteropError> {
    run_legacy_server_fanout_roundtrip_with_overrides(
        steps,
        controlled_room_salt,
        motd_template,
        false,
    )
}

pub fn run_legacy_server_fanout_roundtrip_with_overrides(
    steps: &[ServerRuntimeScenarioStep],
    controlled_room_salt: &str,
    motd_template: Option<&str>,
    persistent_rooms_enabled: bool,
) -> Result<Vec<ServerRuntimeScenarioEvent>, InteropError> {
    run_legacy_server_fanout_roundtrip_with_full_overrides(
        steps,
        controlled_room_salt,
        motd_template,
        persistent_rooms_enabled,
        &[],
    )
}

fn run_legacy_server_fanout_roundtrip_with_full_overrides(
    steps: &[ServerRuntimeScenarioStep],
    controlled_room_salt: &str,
    motd_template: Option<&str>,
    persistent_rooms_enabled: bool,
    permanent_rooms: &[&str],
) -> Result<Vec<ServerRuntimeScenarioEvent>, InteropError> {
    if steps.is_empty() {
        return Ok(Vec::new());
    }

    let legacy_checkout = legacy_syncplay_checkout_dir();
    if !legacy_checkout.is_dir() {
        return Err(InteropError::LegacySyncplayCheckoutMissing(legacy_checkout));
    }

    let legacy_server_entry = legacy_syncplay_server_entry_script_path();
    if !legacy_server_entry.is_file() {
        return Err(InteropError::LegacyServerEntryScriptMissing(
            legacy_server_entry,
        ));
    }

    let port = reserve_ephemeral_tcp_port()?;
    let python_bin = python_bin_from_env();
    let python_bin_display = python_bin.to_string_lossy().to_string();
    let motd_template_file_path = motd_template
        .map(str::trim)
        .filter(|template| !template.is_empty())
        .map(write_legacy_motd_template_file)
        .transpose()?;
    let persistent_rooms_db_path = if persistent_rooms_enabled {
        Some(create_temporary_legacy_rooms_db_file_path()?)
    } else {
        None
    };
    let permanent_rooms_file_path = if permanent_rooms.is_empty() {
        None
    } else {
        Some(create_temporary_legacy_permanent_rooms_file_path(
            permanent_rooms,
        )?)
    };
    let mut command = Command::new(&python_bin);
    command
        .arg(&legacy_server_entry)
        .arg("--port")
        .arg(port.to_string())
        .arg("--ipv4-only")
        .arg("--interface-ipv4")
        .arg("127.0.0.1")
        .arg("--salt")
        .arg(controlled_room_salt)
        .current_dir(legacy_checkout)
        .env("PYTHONUNBUFFERED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(motd_file_path) = motd_template_file_path.as_ref() {
        command.arg("--motd-file").arg(motd_file_path);
    }
    if let Some(rooms_db_path) = persistent_rooms_db_path.as_ref() {
        command.arg("--rooms-db-file").arg(rooms_db_path);
    }
    if let Some(permanent_rooms_path) = permanent_rooms_file_path.as_ref() {
        command
            .arg("--permanent-rooms-file")
            .arg(permanent_rooms_path);
    }
    let child_spawn = command.spawn();
    let mut child = match child_spawn {
        Ok(child) => child,
        Err(source) => {
            if let Some(motd_file_path) = motd_template_file_path {
                let _ = fs::remove_file(motd_file_path);
            }
            if let Some(rooms_db_path) = persistent_rooms_db_path {
                let _ = fs::remove_file(rooms_db_path);
            }
            if let Some(permanent_rooms_path) = permanent_rooms_file_path {
                let _ = fs::remove_file(permanent_rooms_path);
            }
            return Err(InteropError::PythonSpawn {
                python: python_bin_display,
                source,
            });
        }
    };

    let result = (|| {
        wait_for_legacy_server_startup(port, &mut child)?;

        let mut clients: BTreeMap<String, LegacyServerClientConnection> = BTreeMap::new();
        let mut events = Vec::with_capacity(steps.len());
        for step in steps {
            ensure_legacy_server_is_running(&mut child)?;
            if !clients.contains_key(&step.client_id) {
                let stream = connect_legacy_client_stream(port, &step.client_id)?;
                clients.insert(
                    step.client_id.clone(),
                    LegacyServerClientConnection {
                        stream,
                        pending_bytes: Vec::new(),
                    },
                );
            }

            if step.advance_seconds > 0.0 {
                thread::sleep(Duration::from_secs_f64(step.advance_seconds));
            }

            let stream = clients
                .get_mut(&step.client_id)
                .ok_or_else(|| InteropError::MissingLegacyClient(step.client_id.clone()))?;
            let legacy_request_line = prepare_legacy_server_request_line(&step.request_line)?;
            stream.stream.write_all(legacy_request_line.as_bytes())?;
            // Twisted LineReceiver defaults to CRLF framing.
            stream.stream.write_all(b"\r\n")?;
            stream.stream.flush()?;

            let outbound_lines = collect_legacy_server_step_outputs(&mut clients)?;
            events.push(ServerRuntimeScenarioEvent {
                client_id: step.client_id.clone(),
                request_line: step.request_line.clone(),
                outbound_lines,
            });
        }

        Ok(events)
    })();

    terminate_legacy_server_process(&mut child);
    if let Some(motd_file_path) = motd_template_file_path {
        let _ = fs::remove_file(motd_file_path);
    }
    if let Some(rooms_db_path) = persistent_rooms_db_path {
        let _ = fs::remove_file(rooms_db_path);
    }
    if let Some(permanent_rooms_path) = permanent_rooms_file_path {
        let _ = fs::remove_file(permanent_rooms_path);
    }
    result
}

pub fn capture_legacy_server_trace_fixture(
    scenario_name: &str,
    trace_fixture_name: &str,
) -> Result<(), InteropError> {
    capture_legacy_server_trace_fixture_with_salt(
        scenario_name,
        trace_fixture_name,
        DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT,
    )
}

pub fn capture_legacy_server_trace_fixture_with_salt(
    scenario_name: &str,
    trace_fixture_name: &str,
    controlled_room_salt: &str,
) -> Result<(), InteropError> {
    capture_legacy_server_trace_fixture_with_salt_and_motd_template(
        scenario_name,
        trace_fixture_name,
        controlled_room_salt,
        None,
    )
}

pub fn capture_legacy_server_trace_fixture_with_salt_and_motd_template(
    scenario_name: &str,
    trace_fixture_name: &str,
    controlled_room_salt: &str,
    motd_template: Option<&str>,
) -> Result<(), InteropError> {
    capture_legacy_server_trace_fixture_with_overrides(
        scenario_name,
        trace_fixture_name,
        controlled_room_salt,
        motd_template,
        false,
    )
}

pub fn capture_legacy_server_trace_fixture_with_overrides(
    scenario_name: &str,
    trace_fixture_name: &str,
    controlled_room_salt: &str,
    motd_template: Option<&str>,
    persistent_rooms_enabled: bool,
) -> Result<(), InteropError> {
    capture_legacy_server_trace_fixture_with_full_overrides(
        scenario_name,
        trace_fixture_name,
        controlled_room_salt,
        motd_template,
        persistent_rooms_enabled,
        &[],
    )
}

fn capture_legacy_server_trace_fixture_with_full_overrides(
    scenario_name: &str,
    trace_fixture_name: &str,
    controlled_room_salt: &str,
    motd_template: Option<&str>,
    persistent_rooms_enabled: bool,
    permanent_rooms: &[&str],
) -> Result<(), InteropError> {
    let steps = load_server_runtime_scenario_fixture(scenario_name)?;
    let events = run_legacy_server_fanout_roundtrip_with_full_overrides(
        &steps,
        controlled_room_salt,
        motd_template,
        persistent_rooms_enabled,
        permanent_rooms,
    )?;
    let trace_value = scenario_events_to_trace_fixture_value(scenario_name, &events)?;
    fs::write(
        scenario_fixture_path(trace_fixture_name),
        format!("{}\n", serde_json::to_string_pretty(&trace_value)?),
    )?;
    Ok(())
}

pub fn capture_python_trace_fixture(
    scenario_name: &str,
    trace_fixture_name: &str,
) -> Result<(), InteropError> {
    capture_python_trace_fixture_with_motd_template(scenario_name, trace_fixture_name, None)
}

pub fn capture_python_trace_fixture_with_motd_template(
    scenario_name: &str,
    trace_fixture_name: &str,
    motd_template: Option<&str>,
) -> Result<(), InteropError> {
    capture_python_trace_fixture_with_overrides(
        scenario_name,
        trace_fixture_name,
        motd_template,
        false,
    )
}

pub fn capture_python_trace_fixture_with_overrides(
    scenario_name: &str,
    trace_fixture_name: &str,
    motd_template: Option<&str>,
    persistent_rooms_enabled: bool,
) -> Result<(), InteropError> {
    capture_python_trace_fixture_with_full_overrides(
        scenario_name,
        trace_fixture_name,
        motd_template,
        persistent_rooms_enabled,
        &[],
    )
}

fn capture_python_trace_fixture_with_full_overrides(
    scenario_name: &str,
    trace_fixture_name: &str,
    motd_template: Option<&str>,
    persistent_rooms_enabled: bool,
    permanent_rooms: &[&str],
) -> Result<(), InteropError> {
    let steps = load_server_runtime_scenario_fixture(scenario_name)?;
    let events = run_python_fanout_roundtrip_with_full_overrides(
        &steps,
        motd_template,
        persistent_rooms_enabled,
        permanent_rooms,
        false,
    )?;
    let trace_value = scenario_events_to_trace_fixture_value(scenario_name, &events)?;
    fs::write(
        scenario_fixture_path(trace_fixture_name),
        format!("{}\n", serde_json::to_string_pretty(&trace_value)?),
    )?;
    Ok(())
}

fn scenario_events_to_trace_fixture_value(
    scenario_name: &str,
    events: &[ServerRuntimeScenarioEvent],
) -> Result<Value, InteropError> {
    let mut steps = Vec::with_capacity(events.len());
    for (index, event) in events.iter().enumerate() {
        let mut outputs = Vec::with_capacity(event.outbound_lines.len());
        for outbound in &event.outbound_lines {
            outputs.push(json!({
                "client": outbound.client_id,
                "message": serde_json::from_str::<Value>(&outbound.line)?,
            }));
        }
        steps.push(json!({
            "step": index + 1,
            "outputs": outputs,
        }));
    }
    Ok(json!({
        "scenario": scenario_name,
        "steps": steps,
    }))
}

fn prepare_legacy_server_request_line(request_line: &str) -> Result<String, InteropError> {
    let mut request_value: Value = serde_json::from_str(request_line)?;
    if let Some(hello) = request_value
        .get_mut("Hello")
        .and_then(Value::as_object_mut)
    {
        if !hello.contains_key("features") {
            hello.insert(
                "features".to_owned(),
                json!({ LEGACY_COMPAT_MISSING_FEATURES_MARKER: true }),
            );
        }
    }
    Ok(serde_json::to_string(&request_value)?)
}

fn reserve_ephemeral_tcp_port() -> Result<u16, InteropError> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

fn write_legacy_motd_template_file(template: &str) -> Result<PathBuf, InteropError> {
    let unique_suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let filename = format!(
        "syncplay-rs-motd-template-{}-{}.txt",
        std::process::id(),
        unique_suffix
    );
    let path = env::temp_dir().join(filename);
    fs::write(&path, template)?;
    Ok(path)
}

fn create_temporary_legacy_rooms_db_file_path() -> Result<PathBuf, InteropError> {
    let unique_suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let filename = format!(
        "syncplay-rs-persistent-rooms-{}-{}.sqlite3",
        std::process::id(),
        unique_suffix
    );
    let path = env::temp_dir().join(filename);
    fs::write(&path, b"")?;
    Ok(path)
}

fn create_temporary_legacy_permanent_rooms_file_path(
    permanent_rooms: &[&str],
) -> Result<PathBuf, InteropError> {
    let unique_suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let filename = format!(
        "syncplay-rs-permanent-rooms-{}-{}.txt",
        std::process::id(),
        unique_suffix
    );
    let path = env::temp_dir().join(filename);
    let contents = permanent_rooms.join("\n");
    fs::write(&path, contents)?;
    Ok(path)
}

fn wait_for_legacy_server_startup(port: u16, child: &mut Child) -> Result<(), InteropError> {
    let startup_deadline = Instant::now() + LEGACY_SERVER_START_TIMEOUT;
    while Instant::now() <= startup_deadline {
        if let Some(status) = child.try_wait()? {
            let (stdout, stderr) = collect_child_pipes(child);
            return Err(InteropError::LegacyServerExited {
                exit_code: status.code(),
                stdout,
                stderr,
            });
        }

        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return Ok(());
        }

        thread::sleep(Duration::from_millis(40));
    }

    Err(InteropError::LegacyServerStartTimeout {
        port,
        stdout: String::new(),
        stderr: String::new(),
    })
}

fn ensure_legacy_server_is_running(child: &mut Child) -> Result<(), InteropError> {
    if let Some(status) = child.try_wait()? {
        let (stdout, stderr) = collect_child_pipes(child);
        return Err(InteropError::LegacyServerExited {
            exit_code: status.code(),
            stdout,
            stderr,
        });
    }
    Ok(())
}

fn connect_legacy_client_stream(port: u16, client_id: &str) -> Result<TcpStream, InteropError> {
    let connect_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => {
                stream.set_nodelay(true).map_err(|source| {
                    InteropError::LegacyClientConnectionInit {
                        client_id: client_id.to_owned(),
                        source,
                    }
                })?;
                stream.set_nonblocking(true).map_err(|source| {
                    InteropError::LegacyClientConnectionInit {
                        client_id: client_id.to_owned(),
                        source,
                    }
                })?;
                return Ok(stream);
            }
            Err(source) => {
                if Instant::now() >= connect_deadline {
                    return Err(InteropError::LegacyClientConnectionInit {
                        client_id: client_id.to_owned(),
                        source,
                    });
                }
                thread::sleep(Duration::from_millis(25));
            }
        }
    }
}

fn collect_legacy_server_step_outputs(
    clients: &mut BTreeMap<String, LegacyServerClientConnection>,
) -> Result<Vec<DirectedOutboundLine>, InteropError> {
    let mut outputs = Vec::new();
    let step_start = Instant::now();
    let mut last_activity = Instant::now();
    loop {
        let mut saw_new_output = false;
        for (client_id, connection) in clients.iter_mut() {
            let lines = drain_legacy_client_lines(connection)?;
            if lines.is_empty() {
                continue;
            }
            saw_new_output = true;
            for line in lines {
                outputs.push(DirectedOutboundLine {
                    client_id: client_id.clone(),
                    line,
                });
            }
        }
        if saw_new_output {
            last_activity = Instant::now();
        }

        if step_start.elapsed() >= LEGACY_SERVER_STEP_MIN_WAIT
            && last_activity.elapsed() >= LEGACY_SERVER_STEP_IDLE_WAIT
        {
            break;
        }
        if step_start.elapsed() >= LEGACY_SERVER_STEP_MAX_WAIT {
            break;
        }

        thread::sleep(Duration::from_millis(5));
    }

    Ok(outputs)
}

fn drain_legacy_client_lines(
    connection: &mut LegacyServerClientConnection,
) -> Result<Vec<String>, InteropError> {
    let mut chunk = [0_u8; 4096];
    loop {
        match connection.stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => connection.pending_bytes.extend_from_slice(&chunk[..count]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(error) => return Err(InteropError::Io(error)),
        }
    }

    let mut lines = Vec::new();
    loop {
        let Some(newline_index) = connection
            .pending_bytes
            .iter()
            .position(|byte| *byte == b'\n')
        else {
            break;
        };

        let mut raw_line: Vec<u8> = connection.pending_bytes.drain(..=newline_index).collect();
        if raw_line.last().is_some_and(|byte| *byte == b'\n') {
            raw_line.pop();
        }
        if raw_line.last().is_some_and(|byte| *byte == b'\r') {
            raw_line.pop();
        }
        if raw_line.is_empty() {
            continue;
        }

        let line = String::from_utf8_lossy(&raw_line).trim().to_owned();
        if line.is_empty() {
            continue;
        }
        if decode_message_line(&line).is_err() {
            continue;
        }
        lines.push(line);
    }

    Ok(lines)
}

fn terminate_legacy_server_process(child: &mut Child) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = collect_child_pipes(child);
}

fn collect_child_pipes(child: &mut Child) -> (String, String) {
    let stdout = child
        .stdout
        .take()
        .map(read_process_pipe_to_string)
        .unwrap_or_default();
    let stderr = child
        .stderr
        .take()
        .map(read_process_pipe_to_string)
        .unwrap_or_default();
    (stdout, stderr)
}

fn read_process_pipe_to_string<R: Read>(mut reader: R) -> String {
    let mut buffer = Vec::new();
    let _ = reader.read_to_end(&mut buffer);
    String::from_utf8_lossy(&buffer).trim().to_owned()
}

pub fn legacy_syncplay_checkout_dir() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("..");
    path.push("..");
    path.push("..");
    path.push("syncplay");
    path
}

pub fn legacy_syncplay_server_entry_script_path() -> PathBuf {
    legacy_syncplay_checkout_dir().join("syncplayServer.py")
}

pub fn python_handshake_probe_script_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("scripts");
    path.push("python_handshake_probe.py");
    path
}

pub fn default_rust_client_hello_for_interop() -> HelloPayload {
    HelloPayload::new("interop-client", "interop-room", "1.2.255")
        .with_realversion("syncplay-rs-dev")
        .with_features(json!({ "featureList": true }))
}

pub fn run_python_protocol_roundtrip(
    requests: &[ProtocolMessage],
) -> Result<PythonProtocolTranscript, InteropError> {
    if requests.is_empty() {
        return Ok(PythonProtocolTranscript::default());
    }

    let mut request_lines = Vec::with_capacity(requests.len());
    for request in requests {
        request_lines.push(encode_message_line(request)?);
    }

    let payload = serde_json::to_vec(&json!({ "inputs": &request_lines }))?;
    let stdout = run_python_probe_raw(&["--batch"], &payload)?;
    let stdout_line =
        first_non_empty_stdout_line(&stdout).ok_or(InteropError::EmptyPythonResponse)?;
    let parsed: Value = serde_json::from_str(stdout_line)?;
    let output_sets = parsed
        .get("outputs")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            InteropError::InvalidPythonBatchResponse("missing outputs array".to_owned())
        })?;

    if output_sets.len() != requests.len() {
        return Err(InteropError::InvalidPythonBatchResponse(format!(
            "response count mismatch: expected {}, got {}",
            requests.len(),
            output_sets.len()
        )));
    }

    let mut steps = Vec::with_capacity(requests.len());
    for (index, output_set) in output_sets.iter().enumerate() {
        let response_values = output_set.as_array().ok_or_else(|| {
            InteropError::InvalidPythonBatchResponse(format!(
                "outputs[{index}] should be an array of protocol messages"
            ))
        })?;

        let mut response_lines = Vec::with_capacity(response_values.len());
        let mut response_messages = Vec::with_capacity(response_values.len());
        for response_value in response_values {
            let response_line = serde_json::to_string(response_value)?;
            let response_message = decode_message_line(&response_line)?;
            response_lines.push(response_line);
            response_messages.push(response_message);
        }

        steps.push(PythonProtocolStep {
            request_line: request_lines[index].clone(),
            request_message: requests[index].clone(),
            response_lines,
            response_messages,
        });
    }

    Ok(PythonProtocolTranscript { steps })
}

pub fn run_python_handshake_roundtrip() -> Result<PythonHandshakeTranscript, InteropError> {
    run_python_handshake_roundtrip_with_hello(default_rust_client_hello_for_interop())
}

pub fn run_python_handshake_roundtrip_with_hello(
    hello: HelloPayload,
) -> Result<PythonHandshakeTranscript, InteropError> {
    let protocol_transcript = run_python_protocol_roundtrip(&[ProtocolMessage::hello(hello)])?;
    let first_step = protocol_transcript
        .steps
        .first()
        .ok_or(InteropError::EmptyPythonResponse)?;
    let response_line = first_step
        .response_lines
        .first()
        .ok_or(InteropError::EmptyPythonResponse)?
        .clone();
    let response_message = first_step
        .response_messages
        .first()
        .ok_or(InteropError::EmptyPythonResponse)?
        .clone();
    let response_hello = extract_hello_from_message(response_message.clone())?;

    Ok(PythonHandshakeTranscript {
        request_line: first_step.request_line.clone(),
        response_line,
        response_message,
        response_hello,
    })
}

pub fn run_python_same_filename_batch(pairs: &[(&str, &str)]) -> Result<Vec<bool>, InteropError> {
    if pairs.is_empty() {
        return Ok(Vec::new());
    }

    let pairs_payload = pairs
        .iter()
        .map(|(left, right)| json!([left, right]))
        .collect::<Vec<_>>();
    let payload = serde_json::to_vec(&json!({ "pairs": pairs_payload }))?;
    let stdout = run_python_probe_raw(&["--same-filename-batch"], &payload)?;
    let stdout_line =
        first_non_empty_stdout_line(&stdout).ok_or(InteropError::EmptyPythonResponse)?;
    let parsed: Value = serde_json::from_str(stdout_line)?;
    let outputs = parsed
        .get("outputs")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            InteropError::InvalidPythonBatchResponse(
                "missing outputs array for same-filename response".to_owned(),
            )
        })?;

    if outputs.len() != pairs.len() {
        return Err(InteropError::InvalidPythonBatchResponse(format!(
            "same-filename response count mismatch: expected {}, got {}",
            pairs.len(),
            outputs.len()
        )));
    }

    let mut results = Vec::with_capacity(outputs.len());
    for (index, output) in outputs.iter().enumerate() {
        let is_same = output.as_bool().ok_or_else(|| {
            InteropError::InvalidPythonBatchResponse(format!(
                "same-filename outputs[{index}] should be a boolean"
            ))
        })?;
        results.push(is_same);
    }
    Ok(results)
}

pub fn run_python_same_filesize_batch(pairs: &[(Value, Value)]) -> Result<Vec<bool>, InteropError> {
    if pairs.is_empty() {
        return Ok(Vec::new());
    }

    let pairs_payload = pairs
        .iter()
        .map(|(left, right)| json!([left, right]))
        .collect::<Vec<_>>();
    let payload = serde_json::to_vec(&json!({ "pairs": pairs_payload }))?;
    let stdout = run_python_probe_raw(&["--same-filesize-batch"], &payload)?;
    let stdout_line =
        first_non_empty_stdout_line(&stdout).ok_or(InteropError::EmptyPythonResponse)?;
    let parsed: Value = serde_json::from_str(stdout_line)?;
    let outputs = parsed
        .get("outputs")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            InteropError::InvalidPythonBatchResponse(
                "missing outputs array for same-filesize response".to_owned(),
            )
        })?;

    if outputs.len() != pairs.len() {
        return Err(InteropError::InvalidPythonBatchResponse(format!(
            "same-filesize response count mismatch: expected {}, got {}",
            pairs.len(),
            outputs.len()
        )));
    }

    let mut results = Vec::with_capacity(outputs.len());
    for (index, output) in outputs.iter().enumerate() {
        let is_same = output.as_bool().ok_or_else(|| {
            InteropError::InvalidPythonBatchResponse(format!(
                "same-filesize outputs[{index}] should be a boolean"
            ))
        })?;
        results.push(is_same);
    }
    Ok(results)
}

pub fn run_python_same_fileduration_batch(pairs: &[(f64, f64)]) -> Result<Vec<bool>, InteropError> {
    run_python_same_fileduration_batch_with_overrides(pairs, None, None)
}

pub fn run_python_same_fileduration_batch_with_overrides(
    pairs: &[(f64, f64)],
    show_duration_notification: Option<bool>,
    different_duration_threshold: Option<f64>,
) -> Result<Vec<bool>, InteropError> {
    if pairs.is_empty() {
        return Ok(Vec::new());
    }

    let pairs_payload = pairs
        .iter()
        .map(|(left, right)| json!([left, right]))
        .collect::<Vec<_>>();
    let mut payload_object = serde_json::Map::new();
    payload_object.insert("pairs".to_owned(), json!(pairs_payload));
    if let Some(show_duration_notification) = show_duration_notification {
        payload_object.insert(
            "showDurationNotification".to_owned(),
            json!(show_duration_notification),
        );
    }
    if let Some(different_duration_threshold) = different_duration_threshold {
        payload_object.insert(
            "differentDurationThreshold".to_owned(),
            json!(different_duration_threshold),
        );
    }
    let payload = serde_json::to_vec(&Value::Object(payload_object))?;
    let stdout = run_python_probe_raw(&["--same-fileduration-batch"], &payload)?;
    let stdout_line =
        first_non_empty_stdout_line(&stdout).ok_or(InteropError::EmptyPythonResponse)?;
    let parsed: Value = serde_json::from_str(stdout_line)?;
    let outputs = parsed
        .get("outputs")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            InteropError::InvalidPythonBatchResponse(
                "missing outputs array for same-fileduration response".to_owned(),
            )
        })?;

    if outputs.len() != pairs.len() {
        return Err(InteropError::InvalidPythonBatchResponse(format!(
            "same-fileduration response count mismatch: expected {}, got {}",
            pairs.len(),
            outputs.len()
        )));
    }

    let mut results = Vec::with_capacity(outputs.len());
    for (index, output) in outputs.iter().enumerate() {
        let is_same = output.as_bool().ok_or_else(|| {
            InteropError::InvalidPythonBatchResponse(format!(
                "same-fileduration outputs[{index}] should be a boolean"
            ))
        })?;
        results.push(is_same);
    }
    Ok(results)
}

pub fn run_python_privacy_file_payload_batch(
    cases: &[(Value, &str, &str)],
) -> Result<Vec<Value>, InteropError> {
    if cases.is_empty() {
        return Ok(Vec::new());
    }

    let cases_payload = cases
        .iter()
        .map(|(file, filename_privacy_mode, filesize_privacy_mode)| {
            json!({
                "file": file,
                "filenamePrivacyMode": filename_privacy_mode,
                "filesizePrivacyMode": filesize_privacy_mode,
            })
        })
        .collect::<Vec<_>>();
    let payload = serde_json::to_vec(&json!({ "cases": cases_payload }))?;
    let stdout = run_python_probe_raw(&["--privacy-file-payload-batch"], &payload)?;
    let stdout_line =
        first_non_empty_stdout_line(&stdout).ok_or(InteropError::EmptyPythonResponse)?;
    let parsed: Value = serde_json::from_str(stdout_line)?;
    let outputs = parsed
        .get("outputs")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            InteropError::InvalidPythonBatchResponse(
                "missing outputs array for privacy file payload response".to_owned(),
            )
        })?;

    if outputs.len() != cases.len() {
        return Err(InteropError::InvalidPythonBatchResponse(format!(
            "privacy file payload response count mismatch: expected {}, got {}",
            cases.len(),
            outputs.len()
        )));
    }

    outputs
        .iter()
        .enumerate()
        .map(|(index, output)| {
            if output.is_object() {
                Ok(output.clone())
            } else {
                Err(InteropError::InvalidPythonBatchResponse(format!(
                    "privacy file payload outputs[{index}] should be an object"
                )))
            }
        })
        .collect::<Result<Vec<_>, _>>()
}

pub fn run_python_legacy_client_set_file_contract_probe()
-> Result<LegacyClientSetFileContractProbe, InteropError> {
    let stdout = run_python_probe_raw(&["--client-set-file-contract"], b"")?;
    let stdout_line =
        first_non_empty_stdout_line(&stdout).ok_or(InteropError::EmptyPythonResponse)?;
    let parsed: Value = serde_json::from_str(stdout_line)?;

    let parse_string_array = |field_name: &str| -> Result<Vec<String>, InteropError> {
        let values = parsed
            .get(field_name)
            .and_then(Value::as_array)
            .ok_or_else(|| {
                InteropError::InvalidPythonBatchResponse(format!(
                    "missing {field_name} array for client set-file contract response"
                ))
            })?;
        values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                value.as_str().map(str::to_owned).ok_or_else(|| {
                    InteropError::InvalidPythonBatchResponse(format!(
                        "{field_name}[{index}] should be a string"
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()
    };

    let file_payload_ignored = parsed
        .get("filePayloadIgnored")
        .and_then(Value::as_bool)
        .ok_or_else(|| {
            InteropError::InvalidPythonBatchResponse(
                "missing filePayloadIgnored bool for client set-file contract response".to_owned(),
            )
        })?;
    let empty_payload_ignored = parsed
        .get("emptyPayloadIgnored")
        .and_then(Value::as_bool)
        .ok_or_else(|| {
            InteropError::InvalidPythonBatchResponse(
                "missing emptyPayloadIgnored bool for client set-file contract response".to_owned(),
            )
        })?;

    Ok(LegacyClientSetFileContractProbe {
        file_payload_ignored,
        empty_payload_ignored,
        file_payload_calls: parse_string_array("filePayloadCalls")?,
        empty_payload_calls: parse_string_array("emptyPayloadCalls")?,
    })
}

pub fn run_python_legacy_client_user_file_metadata_probe()
-> Result<LegacyClientUserFileMetadataProbe, InteropError> {
    let stdout = run_python_probe_raw(&["--client-user-file-metadata-contract"], b"")?;
    let stdout_line =
        first_non_empty_stdout_line(&stdout).ok_or(InteropError::EmptyPythonResponse)?;
    let parsed: Value = serde_json::from_str(stdout_line)?;

    let parse_snapshot_map =
        |field_name: &str| -> Result<BTreeMap<String, Option<Value>>, InteropError> {
            let values = parsed
                .get(field_name)
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    InteropError::InvalidPythonBatchResponse(format!(
                        "missing {field_name} object for client user file metadata response"
                    ))
                })?;

            values
                .iter()
                .map(|(username, value)| {
                    let file_value = if value.is_null() {
                        None
                    } else if value.is_object() {
                        Some(value.clone())
                    } else {
                        return Err(InteropError::InvalidPythonBatchResponse(format!(
                            "{field_name}.{username} should be an object or null"
                        )));
                    };
                    Ok((username.clone(), file_value))
                })
                .collect::<Result<BTreeMap<_, _>, _>>()
        };

    Ok(LegacyClientUserFileMetadataProbe {
        after_set_mixed: parse_snapshot_map("afterSetMixed")?,
        after_set_empty: parse_snapshot_map("afterSetEmpty")?,
        after_list_mixed: parse_snapshot_map("afterListMixed")?,
        after_list_clears: parse_snapshot_map("afterListClears")?,
    })
}

pub fn run_python_legacy_client_chat_send_contract_batch(
    cases: &[LegacyClientChatSendContractCase],
) -> Result<Vec<LegacyClientChatSendContractResult>, InteropError> {
    if cases.is_empty() {
        return Ok(Vec::new());
    }

    let cases_payload = cases
        .iter()
        .map(|case| {
            json!({
                "message": case.message,
                "chatSupported": case.chat_supported,
                "protocolLogged": case.protocol_logged,
                "serverVersion": case.server_version,
                "maxChatMessageLength": case.max_chat_message_length,
                "deriveServerFeatures": case.derive_server_features,
                "featureList": case.feature_list,
            })
        })
        .collect::<Vec<_>>();
    let payload = serde_json::to_vec(&json!({ "cases": cases_payload }))?;
    let stdout = run_python_probe_raw(&["--client-chat-send-contract-batch"], &payload)?;
    let stdout_line =
        first_non_empty_stdout_line(&stdout).ok_or(InteropError::EmptyPythonResponse)?;
    let parsed: Value = serde_json::from_str(stdout_line)?;
    let outputs = parsed
        .get("outputs")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            InteropError::InvalidPythonBatchResponse(
                "missing outputs array for client chat send contract response".to_owned(),
            )
        })?;

    if outputs.len() != cases.len() {
        return Err(InteropError::InvalidPythonBatchResponse(format!(
            "client chat send contract response count mismatch: expected {}, got {}",
            cases.len(),
            outputs.len()
        )));
    }

    let parse_string_array =
        |value: &Value, field_name: &str| -> Result<Vec<String>, InteropError> {
            let values = value
                .get(field_name)
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    InteropError::InvalidPythonBatchResponse(format!(
                        "missing {field_name} array for client chat send contract response"
                    ))
                })?;
            values
                .iter()
                .enumerate()
                .map(|(index, entry)| {
                    entry.as_str().map(str::to_owned).ok_or_else(|| {
                        InteropError::InvalidPythonBatchResponse(format!(
                            "{field_name}[{index}] should be a string"
                        ))
                    })
                })
                .collect::<Result<Vec<_>, _>>()
        };

    outputs
        .iter()
        .map(|output| {
            if !output.is_object() {
                return Err(InteropError::InvalidPythonBatchResponse(
                    "client chat send contract output should be an object".to_owned(),
                ));
            }
            Ok(LegacyClientChatSendContractResult {
                sent_messages: parse_string_array(output, "sentMessages")?,
                error_messages: parse_string_array(output, "errorMessages")?,
                debug_messages: parse_string_array(output, "debugMessages")?,
            })
        })
        .collect::<Result<Vec<_>, _>>()
}

fn run_python_probe_raw(extra_args: &[&str], stdin_payload: &[u8]) -> Result<String, InteropError> {
    run_python_probe_raw_with_overrides(extra_args, stdin_payload, None, false, &[], false)
}

fn run_python_probe_raw_with_overrides(
    extra_args: &[&str],
    stdin_payload: &[u8],
    motd_template: Option<&str>,
    persistent_rooms_enabled: bool,
    permanent_rooms: &[&str],
    tls_available: bool,
) -> Result<String, InteropError> {
    let legacy_checkout = legacy_syncplay_checkout_dir();
    if !legacy_checkout.is_dir() {
        return Err(InteropError::LegacySyncplayCheckoutMissing(legacy_checkout));
    }

    let probe_script = python_handshake_probe_script_path();
    if !probe_script.is_file() {
        return Err(InteropError::PythonHandshakeProbeMissing(probe_script));
    }

    let python_bin = python_bin_from_env();
    let python_bin_display = python_bin.to_string_lossy().to_string();
    let mut command = Command::new(&python_bin);
    command
        .arg(&probe_script)
        .arg(&legacy_checkout)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(template) = motd_template
        .map(str::trim)
        .filter(|template| !template.is_empty())
    {
        command.env("SYNCPLAY_PROBE_MOTD_TEMPLATE", template);
    }
    if persistent_rooms_enabled {
        command.env("SYNCPLAY_PROBE_PERSISTENT_ROOMS", "1");
    }
    if !permanent_rooms.is_empty() {
        command.env("SYNCPLAY_PROBE_PERMANENT_ROOMS", permanent_rooms.join("\n"));
    }
    if tls_available {
        command.env("SYNCPLAY_PROBE_TLS_AVAILABLE", "1");
    }
    for arg in extra_args {
        command.arg(arg);
    }

    let mut child = command
        .spawn()
        .map_err(|source| InteropError::PythonSpawn {
            python: python_bin_display,
            source,
        })?;

    let mut stdin = child.stdin.take().ok_or(InteropError::PythonStdinMissing)?;
    stdin
        .write_all(stdin_payload)
        .map_err(InteropError::PythonStdinWrite)?;
    drop(stdin);

    let output = child.wait_with_output().map_err(InteropError::PythonWait)?;
    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;
    if !output.status.success() {
        return Err(InteropError::PythonProbeFailed {
            exit_code: output.status.code(),
            stdout: stdout.trim().to_owned(),
            stderr: stderr.trim().to_owned(),
        });
    }

    Ok(stdout)
}

fn first_non_empty_stdout_line(stdout: &str) -> Option<&str> {
    stdout.lines().map(str::trim).find(|line| !line.is_empty())
}

fn python_bin_from_env() -> OsString {
    env::var_os("SYNCPLAY_PYTHON_BIN").unwrap_or_else(|| OsString::from("python"))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        fs,
        io::{Cursor, Read, Write},
        net::TcpStream,
        path::{Path, PathBuf},
        process::{self, Command, Stdio},
        sync::Arc,
        thread,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    use rustls::{
        ClientConfig, ClientConnection, RootCertStore, StreamOwned, pki_types::ServerName,
    };
    use serde_json::{Value, json};

    use super::{
        InteropError, LEGACY_COMPAT_MISSING_FEATURES_MARKER, LegacyClientChatSendContractCase,
        ServerRuntimeScenarioStep, all_protocol_fixture_names, capture_legacy_server_trace_fixture,
        capture_legacy_server_trace_fixture_with_full_overrides,
        capture_legacy_server_trace_fixture_with_overrides,
        capture_legacy_server_trace_fixture_with_salt_and_motd_template,
        capture_python_trace_fixture, capture_python_trace_fixture_with_full_overrides,
        capture_python_trace_fixture_with_motd_template,
        capture_python_trace_fixture_with_overrides, decode_fixture, decode_protocol_file,
        default_rust_client_hello_for_interop, fixture_decodes,
        load_server_runtime_scenario_fixture, replay_server_runtime_scenario_fixture,
        replay_server_runtime_scenario_steps,
        replay_server_runtime_scenario_steps_with_full_overrides,
        replay_server_runtime_scenario_steps_with_motd_template,
        replay_server_runtime_scenario_steps_with_overrides, run_legacy_server_fanout_roundtrip,
        run_legacy_server_fanout_roundtrip_with_full_overrides, run_python_fanout_roundtrip,
        run_python_fanout_roundtrip_with_full_overrides,
        run_python_fanout_roundtrip_with_tls_available, run_python_handshake_roundtrip,
        run_python_legacy_client_chat_send_contract_batch,
        run_python_legacy_client_set_file_contract_probe,
        run_python_legacy_client_user_file_metadata_probe, run_python_privacy_file_payload_batch,
        run_python_protocol_roundtrip, run_python_same_fileduration_batch,
        run_python_same_fileduration_batch_with_overrides, run_python_same_filename_batch,
        run_python_same_filesize_batch, scenario_fixture_path,
    };
    use syncplay_client_core::{ClientRuntimeAction, ClientSession, PrivacyMode};
    use syncplay_protocol::{
        ListPayload, PlaystatePayload, ProtocolMessage, ReadyPayload, RoomRef, SetPayload,
        StatePayload, decode_message_line, encode_message_line, extract_hello_from_message,
    };
    use syncplay_server::ServerRuntime;

    #[derive(Clone, Copy)]
    struct MessageNormalizationOptions {
        normalize_hello_motd: bool,
        normalize_hello_features: bool,
        normalize_set_user_event_features: bool,
        normalize_set_user_features: bool,
        normalize_list_features: bool,
        normalize_list_position: bool,
        normalize_list_file: bool,
        normalize_list_is_ready_when_false_or_null: bool,
        normalize_ping_latency_calculation: bool,
        normalize_ping_client_latency_calculation: bool,
        normalize_ping_client_rtt: bool,
        normalize_ping_server_rtt: bool,
    }

    impl Default for MessageNormalizationOptions {
        fn default() -> Self {
            Self {
                normalize_hello_motd: true,
                normalize_hello_features: true,
                normalize_set_user_event_features: true,
                normalize_set_user_features: true,
                normalize_list_features: true,
                normalize_list_position: true,
                normalize_list_file: true,
                normalize_list_is_ready_when_false_or_null: true,
                normalize_ping_latency_calculation: true,
                normalize_ping_client_latency_calculation: true,
                normalize_ping_client_rtt: true,
                normalize_ping_server_rtt: true,
            }
        }
    }

    const MOTD_TEMPLATE_SCENARIO: &str = "server_runtime_motd_template.jsonl";
    const MOTD_TEMPLATE_OUTDATED_SCENARIO: &str =
        "server_runtime_motd_template_outdated_client.jsonl";
    const MOTD_TEMPLATE_RUNTIME_AND_PROBE: &str = "Compat MOTD latest={latest_version}";
    const MOTD_TEMPLATE_LEGACY_FILE: &str = "Compat MOTD latest=$version";
    const MOTD_TEMPLATE_OUTDATED_EXPECTED: &str = "You are using Syncplay 1.2.255 but a newer version is available from https://syncplay.pl\nCompat MOTD latest=1.7.5";
    const PERSISTENT_ROOMS_NOTICE_SCENARIO: &str = "server_runtime_persistent_rooms_notice.jsonl";
    const PERSISTENT_ROOMS_LIFECYCLE_SCENARIO: &str =
        "server_runtime_persistent_rooms_lifecycle.jsonl";
    const PERSISTENT_ROOMS_TIMEOUT_LIST_UPDATES_SCENARIO: &str =
        "server_runtime_persistent_rooms_timeout_list_updates.jsonl";
    const PERMANENT_ROOMS_FILE_SCENARIO: &str = "server_runtime_permanent_rooms_file.jsonl";
    const PERMANENT_ROOMS_FILE_LIST: &[&str] = &["permanent-room"];
    const PERSISTENT_ROOMS_NOTICE: &str = "NOTICE: This server uses persistent rooms, which means that the playlist information is stored between playback sessions. If you want to create a room where information is not saved then put -temp at the end of the room name.";
    const TEST_TLS_CERT_PEM: &str = include_str!("../../../fixtures/tls/test_cert.pem");
    const TEST_TLS_CHAIN_PEM: &str = include_str!("../../../fixtures/tls/test_chain.pem");
    const TEST_TLS_PRIVATE_KEY_PEM: &str = include_str!("../../../fixtures/tls/test_privkey.pem");

    fn temporary_tls_directory_path(label: &str) -> PathBuf {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "syncplay-rs-compat-{label}-{}-{unique_suffix}",
            process::id()
        ))
    }

    fn write_valid_tls_bundle(path: &Path) {
        fs::write(path.join("privkey.pem"), TEST_TLS_PRIVATE_KEY_PEM)
            .expect("valid private key fixture should write");
        fs::write(path.join("cert.pem"), TEST_TLS_CERT_PEM)
            .expect("valid certificate fixture should write");
        fs::write(path.join("chain.pem"), TEST_TLS_CHAIN_PEM)
            .expect("valid chain fixture should write");
    }

    fn normalization_options_for_runtime_trace_scenario(
        _scenario_name: &str,
    ) -> MessageNormalizationOptions {
        MessageNormalizationOptions {
            normalize_hello_motd: false,
            normalize_hello_features: false,
            normalize_set_user_event_features: false,
            normalize_set_user_features: false,
            normalize_list_features: false,
            normalize_list_position: false,
            normalize_list_file: false,
            normalize_list_is_ready_when_false_or_null: false,
            normalize_ping_latency_calculation: false,
            normalize_ping_client_latency_calculation: false,
            normalize_ping_client_rtt: false,
            normalize_ping_server_rtt: false,
        }
    }

    fn normalization_options_for_runtime_python_scenario(
        _scenario_name: &str,
    ) -> MessageNormalizationOptions {
        MessageNormalizationOptions {
            normalize_hello_motd: false,
            normalize_hello_features: false,
            normalize_set_user_event_features: false,
            normalize_set_user_features: false,
            normalize_list_features: false,
            normalize_list_position: false,
            normalize_list_file: false,
            normalize_list_is_ready_when_false_or_null: false,
            normalize_ping_latency_calculation: false,
            normalize_ping_client_latency_calculation: false,
            normalize_ping_client_rtt: false,
            normalize_ping_server_rtt: false,
        }
    }

    fn normalization_options_for_legacy_scenario(
        _scenario_name: &str,
    ) -> MessageNormalizationOptions {
        MessageNormalizationOptions {
            normalize_hello_motd: false,
            normalize_hello_features: false,
            normalize_set_user_event_features: false,
            normalize_set_user_features: false,
            normalize_list_features: false,
            normalize_list_position: false,
            normalize_list_file: false,
            normalize_list_is_ready_when_false_or_null: false,
            normalize_ping_latency_calculation: false,
            normalize_ping_client_latency_calculation: false,
            normalize_ping_client_rtt: false,
            normalize_ping_server_rtt: false,
        }
    }

    #[derive(Clone, Copy)]
    enum LegacyTimingSide {
        Legacy,
        Runtime,
    }

    #[derive(Default)]
    struct LegacyTimingCanonicalizer {
        legacy_latency_origin: Option<f64>,
        runtime_latency_origin: Option<f64>,
        legacy_server_rtt_nonzero_origin: Option<f64>,
        runtime_server_rtt_nonzero_origin: Option<f64>,
    }

    impl LegacyTimingCanonicalizer {
        fn canonicalize_message(&mut self, message: &mut Value, side: LegacyTimingSide) {
            let Some(state_payload) = message.get_mut("State").and_then(Value::as_object_mut)
            else {
                return;
            };
            let Some(ping) = state_payload.get_mut("ping").and_then(Value::as_object_mut) else {
                return;
            };
            let Some(latency_value) = ping.get_mut("latencyCalculation") else {
                return;
            };
            let Some(latency) = latency_value.as_f64() else {
                return;
            };
            if !latency.is_finite() {
                return;
            }

            let origin_slot = match side {
                LegacyTimingSide::Legacy => &mut self.legacy_latency_origin,
                LegacyTimingSide::Runtime => &mut self.runtime_latency_origin,
            };
            let origin = *origin_slot.get_or_insert(latency);
            let canonical_latency = (latency - origin).round();
            let canonical_latency = if canonical_latency == -0.0 {
                0.0
            } else {
                canonical_latency
            };
            *latency_value = Value::from(canonical_latency);

            let Some(server_rtt_value) = ping.get_mut("serverRtt") else {
                return;
            };
            let Some(server_rtt) = server_rtt_value.as_f64() else {
                return;
            };
            if !server_rtt.is_finite() {
                return;
            }
            if server_rtt.abs() <= f64::EPSILON {
                *server_rtt_value = Value::from(0.0);
                return;
            }

            let rtt_origin_slot = match side {
                LegacyTimingSide::Legacy => &mut self.legacy_server_rtt_nonzero_origin,
                LegacyTimingSide::Runtime => &mut self.runtime_server_rtt_nonzero_origin,
            };
            let rtt_origin = *rtt_origin_slot.get_or_insert(server_rtt);
            let canonical_server_rtt = (server_rtt - rtt_origin).round();
            let canonical_server_rtt = if canonical_server_rtt == -0.0 {
                0.0
            } else {
                canonical_server_rtt
            };
            *server_rtt_value = Value::from(canonical_server_rtt);
        }
    }

    fn is_legacy_default_user_features(features: &serde_json::Map<String, Value>) -> bool {
        features.get("chat") == Some(&Value::Bool(false))
            && features.get("featureList") == Some(&Value::Bool(false))
            && features.get("managedRooms") == Some(&Value::Bool(false))
            && features.get("persistentRooms") == Some(&Value::Bool(false))
            && features.get("readiness") == Some(&Value::Bool(false))
            && features.get("sharedPlaylists") == Some(&Value::Bool(false))
            && features.get("uiMode") == Some(&Value::String("Unknown".to_owned()))
    }

    fn is_legacy_compat_missing_features_marker(features: &serde_json::Map<String, Value>) -> bool {
        features.get(LEGACY_COMPAT_MISSING_FEATURES_MARKER) == Some(&Value::Bool(true))
    }

    fn canonicalize_user_features_field(object: &mut serde_json::Map<String, Value>, field: &str) {
        let canonicalize_to_default = match object.get(field) {
            None => true,
            Some(Value::Null) => true,
            Some(Value::Object(features)) => {
                is_legacy_default_user_features(features)
                    || is_legacy_compat_missing_features_marker(features)
            }
            _ => false,
        };
        if canonicalize_to_default {
            object.insert(
                field.to_owned(),
                Value::String("__default_user_features__".to_owned()),
            );
        }
    }

    fn canonicalize_legacy_set_user_features(message: &mut Value) {
        let Some(set_payload) = message.get_mut("Set").and_then(Value::as_object_mut) else {
            return;
        };
        let Some(users) = set_payload.get_mut("user").and_then(Value::as_object_mut) else {
            return;
        };
        for user_payload in users.values_mut() {
            let Some(user_object) = user_payload.as_object_mut() else {
                continue;
            };
            if let Some(event) = user_object.get_mut("event").and_then(Value::as_object_mut) {
                canonicalize_user_features_field(event, "features");
            }
            canonicalize_user_features_field(user_object, "features");
        }
    }

    fn canonicalize_user_is_ready_field(object: &mut serde_json::Map<String, Value>, field: &str) {
        let canonicalize_to_not_ready = matches!(
            object.get(field),
            None | Some(Value::Null) | Some(Value::Bool(false))
        );
        if canonicalize_to_not_ready {
            object.insert(field.to_owned(), Value::String("__not_ready__".to_owned()));
        }
    }

    fn canonicalize_legacy_list_fields(message: &mut Value) {
        let Some(list_payload) = message.get_mut("List").and_then(Value::as_object_mut) else {
            return;
        };
        for room_users in list_payload.values_mut() {
            let Some(room_users) = room_users.as_object_mut() else {
                continue;
            };
            for user_payload in room_users.values_mut() {
                let Some(user_object) = user_payload.as_object_mut() else {
                    continue;
                };
                canonicalize_user_features_field(user_object, "features");
                canonicalize_user_is_ready_field(user_object, "isReady");
            }
        }
    }

    fn canonicalize_legacy_hello_fields(message: &mut Value) {
        const SHARED_HELLO_FEATURE_KEYS: &[&str] = &[
            "chat",
            "isolateRooms",
            "managedRooms",
            "persistentRooms",
            "readiness",
            "setOthersReadiness",
        ];

        let Some(hello_payload) = message.get_mut("Hello").and_then(Value::as_object_mut) else {
            return;
        };

        let canonical_motd = match hello_payload.get("motd") {
            None | Some(Value::Null) => Some(String::new()),
            Some(Value::String(motd)) if motd.trim().is_empty() => Some(String::new()),
            _ => None,
        };
        if let Some(motd) = canonical_motd {
            hello_payload.insert("motd".to_owned(), Value::String(motd));
        }

        if let Some(features_value) = hello_payload.get_mut("features") {
            let Some(features) = features_value.as_object() else {
                return;
            };

            let mut canonical_features = serde_json::Map::new();
            for key in SHARED_HELLO_FEATURE_KEYS {
                if let Some(value) = features.get(*key) {
                    canonical_features.insert((*key).to_owned(), value.clone());
                }
            }
            *features_value = Value::Object(canonical_features);
        }
    }

    fn normalize_cross_impl_message(value: Value) -> Value {
        normalize_cross_impl_message_with_options(value, MessageNormalizationOptions::default())
    }

    fn normalize_cross_impl_message_with_options(
        mut value: Value,
        options: MessageNormalizationOptions,
    ) -> Value {
        if let Some(hello) = value.get_mut("Hello").and_then(Value::as_object_mut) {
            // Rust runtime and Python probe intentionally report different server version strings.
            hello.insert(
                "realversion".to_owned(),
                Value::String("__normalized__".to_owned()),
            );
            if options.normalize_hello_motd && hello.contains_key("motd") {
                hello.insert(
                    "motd".to_owned(),
                    Value::String("__normalized_motd__".to_owned()),
                );
            }
            if options.normalize_hello_features && hello.contains_key("features") {
                hello.insert(
                    "features".to_owned(),
                    Value::String("__normalized_features__".to_owned()),
                );
            }
        }
        if let Some(set_payload) = value.get_mut("Set").and_then(Value::as_object_mut) {
            if let Some(users) = set_payload.get_mut("user").and_then(Value::as_object_mut) {
                for user_payload in users.values_mut() {
                    let Some(user_object) = user_payload.as_object_mut() else {
                        continue;
                    };
                    if let Some(event) = user_object.get_mut("event").and_then(Value::as_object_mut)
                    {
                        if options.normalize_set_user_event_features {
                            event.remove("features");
                        }
                    }
                    if options.normalize_set_user_features {
                        user_object.remove("features");
                    }
                }
            }
        }
        if let Some(list_payload) = value.get_mut("List").and_then(Value::as_object_mut) {
            for room_users in list_payload.values_mut() {
                let Some(room_users) = room_users.as_object_mut() else {
                    continue;
                };
                for user_payload in room_users.values_mut() {
                    let Some(user_object) = user_payload.as_object_mut() else {
                        continue;
                    };
                    if options.normalize_list_features {
                        user_object.remove("features");
                    }
                    if let Some(position_value) = user_object.get_mut("position") {
                        if let Some(position) = position_value.as_f64() {
                            let rounded_position = (position * 1000.0).round() / 1000.0;
                            *position_value = Value::from(rounded_position);
                        }
                    }
                    if options.normalize_list_position {
                        user_object.remove("position");
                    }
                    if options.normalize_list_file {
                        user_object.remove("file");
                    }
                    if options.normalize_list_is_ready_when_false_or_null
                        && user_object.get("isReady").is_some_and(|is_ready| {
                            is_ready.is_null() || is_ready == &Value::Bool(false)
                        })
                    {
                        user_object.remove("isReady");
                    }
                }
            }
        }
        if let Some(state_payload) = value.get_mut("State").and_then(Value::as_object_mut) {
            if let Some(playstate) = state_payload
                .get_mut("playstate")
                .and_then(Value::as_object_mut)
            {
                if let Some(position_value) = playstate.get_mut("position") {
                    if let Some(position) = position_value.as_f64() {
                        let rounded_position = (position * 1000.0).round() / 1000.0;
                        *position_value = Value::from(rounded_position);
                    }
                }
            }
            if let Some(ping) = state_payload.get_mut("ping").and_then(Value::as_object_mut) {
                if let Some(latency_value) = ping.get_mut("latencyCalculation") {
                    if let Some(latency) = latency_value.as_f64() {
                        let rounded_latency = (latency * 1000.0).round() / 1000.0;
                        *latency_value = Value::from(rounded_latency);
                    }
                }
                if options.normalize_ping_latency_calculation
                    && ping.contains_key("latencyCalculation")
                {
                    ping.insert(
                        "latencyCalculation".to_owned(),
                        Value::String("__normalized_latency__".to_owned()),
                    );
                }
                if let Some(client_latency_value) = ping.get_mut("clientLatencyCalculation") {
                    if let Some(client_latency) = client_latency_value.as_f64() {
                        let canonical_client_latency =
                            if options.normalize_ping_client_latency_calculation {
                                (client_latency * 1000.0).round() / 1000.0
                            } else {
                                // Legacy server mutates this field slightly in-flight; compare at
                                // a stable tenth-second granularity when preserving the value.
                                (client_latency * 10.0).trunc() / 10.0
                            };
                        *client_latency_value = Value::from(canonical_client_latency);
                    }
                }
                if options.normalize_ping_client_latency_calculation
                    && ping.contains_key("clientLatencyCalculation")
                {
                    ping.insert(
                        "clientLatencyCalculation".to_owned(),
                        Value::String("__normalized_client_latency__".to_owned()),
                    );
                }
                if let Some(client_rtt_value) = ping.get_mut("clientRtt") {
                    if let Some(client_rtt) = client_rtt_value.as_f64() {
                        let rounded_client_rtt = (client_rtt * 1000.0).round() / 1000.0;
                        *client_rtt_value = Value::from(rounded_client_rtt);
                    }
                }
                if options.normalize_ping_client_rtt && ping.contains_key("clientRtt") {
                    ping.insert(
                        "clientRtt".to_owned(),
                        Value::String("__normalized_client_rtt__".to_owned()),
                    );
                }
                if let Some(server_rtt_value) = ping.get_mut("serverRtt") {
                    if let Some(server_rtt) = server_rtt_value.as_f64() {
                        let rounded_server_rtt = (server_rtt * 1000.0).round() / 1000.0;
                        *server_rtt_value = Value::from(rounded_server_rtt);
                    }
                }
                if options.normalize_ping_server_rtt && ping.contains_key("serverRtt") {
                    ping.insert(
                        "serverRtt".to_owned(),
                        Value::String("__normalized_server_rtt__".to_owned()),
                    );
                }
            }
        }
        strip_null_object_fields(&mut value);
        value
    }

    fn strip_null_object_fields(value: &mut Value) {
        match value {
            Value::Object(object) => {
                object.retain(|_, field_value| !field_value.is_null());
                for field_value in object.values_mut() {
                    strip_null_object_fields(field_value);
                }
            }
            Value::Array(values) => {
                for field_value in values {
                    strip_null_object_fields(field_value);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn normalize_cross_impl_message_treats_null_fields_as_absent() {
        let with_null = json!({
            "Set": {
                "playlistChange": {
                    "files": [],
                    "user": null
                }
            }
        });
        let without_null = json!({
            "Set": {
                "playlistChange": {
                    "files": []
                }
            }
        });

        assert_eq!(
            normalize_cross_impl_message(with_null),
            normalize_cross_impl_message(without_null)
        );
    }

    #[test]
    fn normalize_cross_impl_message_normalizes_state_ping_timing_fields() {
        let first = json!({
            "State": {
                "playstate": {
                    "position": 12.500001,
                    "paused": false,
                    "doSeek": false,
                    "setBy": "alice"
                },
                "ping": {
                    "latencyCalculation": 100.1,
                    "serverRtt": 0.0
                },
                "ignoringOnTheFly": {
                    "server": 1
                }
            }
        });
        let second = json!({
            "State": {
                "playstate": {
                    "position": 12.500499,
                    "paused": false,
                    "doSeek": false,
                    "setBy": "alice"
                },
                "ping": {
                    "latencyCalculation": 999.9,
                    "serverRtt": 0.25
                },
                "ignoringOnTheFly": {
                    "server": 1
                }
            }
        });

        assert_eq!(
            normalize_cross_impl_message(first),
            normalize_cross_impl_message(second)
        );
    }

    #[test]
    fn normalize_cross_impl_message_normalizes_set_user_features_by_default() {
        let first = json!({
            "Set": {
                "user": {
                    "bob": {
                        "event": {
                            "joined": true,
                            "features": {
                                "chat": false
                            }
                        }
                    }
                }
            }
        });
        let second = json!({
            "Set": {
                "user": {
                    "bob": {
                        "event": {
                            "joined": true
                        }
                    }
                }
            }
        });

        assert_eq!(
            normalize_cross_impl_message(first),
            normalize_cross_impl_message(second)
        );
    }

    #[test]
    fn normalize_cross_impl_message_can_preserve_set_user_features() {
        let first = json!({
            "Set": {
                "user": {
                    "bob": {
                        "event": {
                            "joined": true,
                            "features": {
                                "chat": false
                            }
                        }
                    }
                }
            }
        });
        let second = json!({
            "Set": {
                "user": {
                    "bob": {
                        "event": {
                            "joined": true
                        }
                    }
                }
            }
        });

        let options = MessageNormalizationOptions {
            normalize_set_user_event_features: false,
            normalize_set_user_features: false,
            ..MessageNormalizationOptions::default()
        };
        assert_ne!(
            normalize_cross_impl_message_with_options(first, options),
            normalize_cross_impl_message_with_options(second, options)
        );
    }

    #[test]
    fn normalize_cross_impl_message_canonicalizes_list_position_number_types() {
        let with_integer_position = json!({
            "List": {
                "room1": {
                    "alice": {
                        "controller": false,
                        "position": 0,
                        "file": {},
                        "isReady": null
                    }
                }
            }
        });
        let with_float_position = json!({
            "List": {
                "room1": {
                    "alice": {
                        "controller": false,
                        "position": 0.0,
                        "file": {},
                        "isReady": null
                    }
                }
            }
        });

        let options = MessageNormalizationOptions {
            normalize_list_features: false,
            normalize_list_position: false,
            normalize_list_file: false,
            normalize_list_is_ready_when_false_or_null: false,
            ..MessageNormalizationOptions::default()
        };
        assert_eq!(
            normalize_cross_impl_message_with_options(with_integer_position, options),
            normalize_cross_impl_message_with_options(with_float_position, options)
        );
    }

    #[test]
    fn normalize_cross_impl_message_can_preserve_state_ping_server_rtt() {
        let first = json!({
            "State": {
                "playstate": {
                    "position": 12.500001,
                    "paused": false,
                    "doSeek": false,
                    "setBy": "alice"
                },
                "ping": {
                    "latencyCalculation": 100.1,
                    "serverRtt": 0.0
                },
                "ignoringOnTheFly": {
                    "server": 1
                }
            }
        });
        let second = json!({
            "State": {
                "playstate": {
                    "position": 12.500499,
                    "paused": false,
                    "doSeek": false,
                    "setBy": "alice"
                },
                "ping": {
                    "latencyCalculation": 999.9,
                    "serverRtt": 0.25
                },
                "ignoringOnTheFly": {
                    "server": 1
                }
            }
        });

        let options = MessageNormalizationOptions {
            normalize_ping_latency_calculation: true,
            normalize_ping_client_latency_calculation: true,
            normalize_ping_client_rtt: true,
            normalize_ping_server_rtt: false,
            ..MessageNormalizationOptions::default()
        };
        assert_ne!(
            normalize_cross_impl_message_with_options(first, options),
            normalize_cross_impl_message_with_options(second, options)
        );
    }

    #[test]
    fn normalize_cross_impl_message_can_preserve_state_ping_client_timing_fields() {
        let first = json!({
            "State": {
                "playstate": {
                    "position": 2.0,
                    "paused": false,
                    "doSeek": true,
                    "setBy": "alice"
                },
                "ping": {
                    "latencyCalculation": 100.1,
                    "clientLatencyCalculation": 124.1,
                    "clientRtt": 1.0,
                    "serverRtt": 0.0
                }
            }
        });
        let second = json!({
            "State": {
                "playstate": {
                    "position": 2.0,
                    "paused": false,
                    "doSeek": true,
                    "setBy": "alice"
                },
                "ping": {
                    "latencyCalculation": 100.1,
                    "clientLatencyCalculation": 126.1,
                    "clientRtt": 2.0,
                    "serverRtt": 0.0
                }
            }
        });

        let options = MessageNormalizationOptions {
            normalize_ping_latency_calculation: false,
            normalize_ping_client_latency_calculation: false,
            normalize_ping_client_rtt: false,
            normalize_ping_server_rtt: false,
            ..MessageNormalizationOptions::default()
        };
        assert_ne!(
            normalize_cross_impl_message_with_options(first, options),
            normalize_cross_impl_message_with_options(second, options)
        );
    }

    #[test]
    fn normalize_cross_impl_message_can_preserve_state_ping_latency_calculation() {
        let first = json!({
            "State": {
                "playstate": {
                    "position": 2.0,
                    "paused": false,
                    "doSeek": true,
                    "setBy": "alice"
                },
                "ping": {
                    "latencyCalculation": 100.1,
                    "serverRtt": 0.0
                }
            }
        });
        let second = json!({
            "State": {
                "playstate": {
                    "position": 2.0,
                    "paused": false,
                    "doSeek": true,
                    "setBy": "alice"
                },
                "ping": {
                    "latencyCalculation": 101.6,
                    "serverRtt": 0.0
                }
            }
        });

        let options = MessageNormalizationOptions {
            normalize_ping_latency_calculation: false,
            normalize_ping_client_latency_calculation: false,
            normalize_ping_client_rtt: false,
            normalize_ping_server_rtt: false,
            ..MessageNormalizationOptions::default()
        };
        assert_ne!(
            normalize_cross_impl_message_with_options(first, options),
            normalize_cross_impl_message_with_options(second, options)
        );
    }

    #[test]
    fn legacy_timing_canonicalizer_aligns_latency_from_independent_clock_origins() {
        let mut canonicalizer = LegacyTimingCanonicalizer::default();
        let mut legacy_message = json!({
            "State": {
                "ping": {
                    "latencyCalculation": 1770633211.852
                }
            }
        });
        let mut runtime_message = json!({
            "State": {
                "ping": {
                    "latencyCalculation": 0.0
                }
            }
        });
        canonicalizer.canonicalize_message(&mut legacy_message, LegacyTimingSide::Legacy);
        canonicalizer.canonicalize_message(&mut runtime_message, LegacyTimingSide::Runtime);
        assert_eq!(legacy_message, runtime_message);

        let mut legacy_next = json!({
            "State": {
                "ping": {
                    "latencyCalculation": 1770633212.852
                }
            }
        });
        let mut runtime_next = json!({
            "State": {
                "ping": {
                    "latencyCalculation": 1.0
                }
            }
        });
        canonicalizer.canonicalize_message(&mut legacy_next, LegacyTimingSide::Legacy);
        canonicalizer.canonicalize_message(&mut runtime_next, LegacyTimingSide::Runtime);
        assert_eq!(legacy_next, runtime_next);
    }

    #[test]
    fn legacy_timing_canonicalizer_aligns_server_rtt_from_independent_nonzero_origins() {
        let mut canonicalizer = LegacyTimingCanonicalizer::default();
        let mut legacy_message = json!({
            "State": {
                "ping": {
                    "latencyCalculation": 1770633211.852,
                    "serverRtt": 1770632677.433682
                }
            }
        });
        let mut runtime_message = json!({
            "State": {
                "ping": {
                    "latencyCalculation": 0.0,
                    "serverRtt": 0.0
                }
            }
        });
        canonicalizer.canonicalize_message(&mut legacy_message, LegacyTimingSide::Legacy);
        canonicalizer.canonicalize_message(&mut runtime_message, LegacyTimingSide::Runtime);
        assert_eq!(
            legacy_message
                .pointer("/State/ping/serverRtt")
                .and_then(Value::as_f64),
            Some(0.0)
        );
        assert_eq!(
            runtime_message
                .pointer("/State/ping/serverRtt")
                .and_then(Value::as_f64),
            Some(0.0)
        );
    }

    #[test]
    fn canonicalize_legacy_set_user_features_aligns_missing_and_default_shapes() {
        let mut legacy_message = json!({
            "Set": {
                "user": {
                    "bob": {
                        "event": {
                            "joined": true,
                            "features": {
                                "chat": false,
                                "featureList": false,
                                "managedRooms": false,
                                "persistentRooms": false,
                                "readiness": false,
                                "sharedPlaylists": false,
                                "uiMode": "Unknown"
                            }
                        }
                    }
                }
            }
        });
        let mut runtime_message = json!({
            "Set": {
                "user": {
                    "bob": {
                        "event": {
                            "joined": true
                        }
                    }
                }
            }
        });
        canonicalize_legacy_set_user_features(&mut legacy_message);
        canonicalize_legacy_set_user_features(&mut runtime_message);
        assert_eq!(legacy_message, runtime_message);
    }

    #[test]
    fn canonicalize_legacy_set_user_features_aligns_compat_missing_features_marker() {
        let mut legacy_message = json!({
            "Set": {
                "user": {
                    "charlie": {
                        "event": {
                            "joined": true,
                            "features": {}
                        },
                        "features": {}
                    }
                }
            }
        });
        let mut runtime_message = json!({
            "Set": {
                "user": {
                    "charlie": {
                        "event": {
                            "joined": true
                        }
                    }
                }
            }
        });
        legacy_message
            .pointer_mut("/Set/user/charlie/event/features")
            .and_then(Value::as_object_mut)
            .expect("event features object should exist")
            .insert(
                LEGACY_COMPAT_MISSING_FEATURES_MARKER.to_owned(),
                Value::Bool(true),
            );
        legacy_message
            .pointer_mut("/Set/user/charlie/features")
            .and_then(Value::as_object_mut)
            .expect("user features object should exist")
            .insert(
                LEGACY_COMPAT_MISSING_FEATURES_MARKER.to_owned(),
                Value::Bool(true),
            );
        canonicalize_legacy_set_user_features(&mut legacy_message);
        canonicalize_legacy_set_user_features(&mut runtime_message);
        assert_eq!(legacy_message, runtime_message);
    }

    #[test]
    fn canonicalize_legacy_list_fields_aligns_feature_and_not_ready_shapes() {
        let mut legacy_message = json!({
            "List": {
                "room1": {
                    "bob": {
                        "controller": false,
                        "features": {
                            "chat": false,
                            "featureList": false,
                            "managedRooms": false,
                            "persistentRooms": false,
                            "readiness": false,
                            "sharedPlaylists": false,
                            "uiMode": "Unknown"
                        },
                        "isReady": null,
                        "file": {},
                        "position": 0
                    }
                }
            }
        });
        let mut runtime_message = json!({
            "List": {
                "room1": {
                    "bob": {
                        "controller": false,
                        "isReady": false,
                        "file": {},
                        "position": 0.0
                    }
                }
            }
        });
        let options = MessageNormalizationOptions {
            normalize_list_features: false,
            normalize_list_position: false,
            normalize_list_file: false,
            normalize_list_is_ready_when_false_or_null: false,
            ..MessageNormalizationOptions::default()
        };
        legacy_message = normalize_cross_impl_message_with_options(legacy_message, options);
        runtime_message = normalize_cross_impl_message_with_options(runtime_message, options);
        canonicalize_legacy_list_fields(&mut legacy_message);
        canonicalize_legacy_list_fields(&mut runtime_message);
        assert_eq!(legacy_message, runtime_message);
    }

    #[test]
    fn canonicalize_legacy_list_fields_aligns_compat_missing_features_marker() {
        let mut legacy_message = json!({
            "List": {
                "room1": {
                    "charlie": {
                        "controller": false,
                        "features": {},
                        "file": {},
                        "position": 0.0
                    }
                }
            }
        });
        let mut runtime_message = json!({
            "List": {
                "room1": {
                    "charlie": {
                        "controller": false,
                        "file": {},
                        "position": 0.0
                    }
                }
            }
        });
        legacy_message
            .pointer_mut("/List/room1/charlie/features")
            .and_then(Value::as_object_mut)
            .expect("list user features object should exist")
            .insert(
                LEGACY_COMPAT_MISSING_FEATURES_MARKER.to_owned(),
                Value::Bool(true),
            );
        canonicalize_legacy_list_fields(&mut legacy_message);
        canonicalize_legacy_list_fields(&mut runtime_message);
        assert_eq!(legacy_message, runtime_message);
    }

    #[test]
    fn canonicalize_legacy_hello_fields_aligns_shared_capabilities() {
        let mut legacy_message = json!({
            "Hello": {
                "features": {
                    "chat": true,
                    "isolateRooms": false,
                    "managedRooms": true,
                    "maxChatMessageLength": 150,
                    "maxFilenameLength": 250,
                    "maxRoomNameLength": 35,
                    "maxUsernameLength": 16,
                    "persistentRooms": false,
                    "readiness": true,
                    "setOthersReadiness": true
                }
            }
        });
        let mut runtime_message = json!({
            "Hello": {
                "features": {
                    "chat": true,
                    "featureList": true,
                    "isolateRooms": false,
                    "managedRooms": true,
                    "persistentRooms": false,
                    "readiness": true,
                    "setOthersReadiness": true,
                    "uiMode": "UNKNOWN"
                }
            }
        });
        canonicalize_legacy_hello_fields(&mut legacy_message);
        canonicalize_legacy_hello_fields(&mut runtime_message);
        assert_eq!(legacy_message, runtime_message);
    }

    #[test]
    fn canonicalize_legacy_hello_fields_aligns_empty_motd_shapes() {
        let mut legacy_message = json!({
            "Hello": {
                "motd": null
            }
        });
        let mut runtime_message = json!({
            "Hello": {
                "motd": ""
            }
        });
        canonicalize_legacy_hello_fields(&mut legacy_message);
        canonicalize_legacy_hello_fields(&mut runtime_message);
        assert_eq!(legacy_message, runtime_message);
    }

    #[test]
    fn canonicalize_legacy_hello_fields_preserves_non_default_motd() {
        let mut first_message = json!({
            "Hello": {
                "motd": "Welcome to a custom room."
            }
        });
        let mut second_message = json!({
            "Hello": {
                "motd": "Different custom room text."
            }
        });
        canonicalize_legacy_hello_fields(&mut first_message);
        canonicalize_legacy_hello_fields(&mut second_message);
        assert_ne!(first_message, second_message);
    }

    fn assert_runtime_matches_captured_trace(trace_fixture_name: &str) {
        assert_runtime_matches_captured_trace_with_overrides(trace_fixture_name, None, false);
    }

    fn assert_runtime_matches_captured_trace_with_motd_template(
        trace_fixture_name: &str,
        runtime_motd_template: Option<&str>,
    ) {
        assert_runtime_matches_captured_trace_with_overrides(
            trace_fixture_name,
            runtime_motd_template,
            false,
        );
    }

    fn assert_runtime_matches_captured_trace_with_overrides(
        trace_fixture_name: &str,
        runtime_motd_template: Option<&str>,
        runtime_persistent_rooms_enabled: bool,
    ) {
        assert_runtime_matches_captured_trace_with_full_overrides(
            trace_fixture_name,
            runtime_motd_template,
            runtime_persistent_rooms_enabled,
            &[],
        );
    }

    fn assert_runtime_matches_captured_trace_with_full_overrides(
        trace_fixture_name: &str,
        runtime_motd_template: Option<&str>,
        runtime_persistent_rooms_enabled: bool,
        runtime_permanent_rooms: &[&str],
    ) {
        let expected_path = scenario_fixture_path(trace_fixture_name);
        let expected_value: Value = serde_json::from_str(
            &std::fs::read_to_string(&expected_path)
                .expect("expected parity trace fixture should be readable"),
        )
        .expect("expected parity trace fixture should be valid JSON");

        let scenario_name = expected_value
            .get("scenario")
            .and_then(Value::as_str)
            .expect("expected trace fixture should contain scenario field");
        let normalization_options = normalization_options_for_runtime_trace_scenario(scenario_name);
        let steps = load_server_runtime_scenario_fixture(scenario_name)
            .expect("scenario fixture should be readable for runtime trace comparison");
        let events = replay_server_runtime_scenario_steps_with_full_overrides(
            &steps,
            runtime_motd_template,
            runtime_persistent_rooms_enabled,
            runtime_permanent_rooms,
        )
        .expect("scenario should replay through server runtime");

        let expected_steps = expected_value
            .get("steps")
            .and_then(Value::as_array)
            .expect("expected trace fixture should contain steps array");
        assert_eq!(
            events.len(),
            expected_steps.len(),
            "scenario step count mismatch for captured trace fixture '{trace_fixture_name}'"
        );

        for expected_step in expected_steps {
            let step_number = expected_step
                .get("step")
                .and_then(Value::as_u64)
                .expect("expected step should contain numeric step")
                as usize;
            let expected_outputs = expected_step
                .get("outputs")
                .and_then(Value::as_array)
                .expect("expected step should contain outputs array");
            let actual_event = events
                .get(step_number - 1)
                .expect("expected step index should exist in replay output");

            assert_eq!(
                actual_event.outbound_lines.len(),
                expected_outputs.len(),
                "mismatch in outbound count at scenario step {step_number}"
            );

            for (index, expected_output) in expected_outputs.iter().enumerate() {
                let expected_client = expected_output
                    .get("client")
                    .and_then(Value::as_str)
                    .expect("expected output should contain client");
                let expected_message = expected_output
                    .get("message")
                    .expect("expected output should contain message");

                let actual_output = &actual_event.outbound_lines[index];
                let actual_message: Value = normalize_cross_impl_message_with_options(
                    serde_json::from_str(&actual_output.line)
                        .expect("actual outbound line should decode to JSON value"),
                    normalization_options,
                );
                let expected_message = normalize_cross_impl_message_with_options(
                    expected_message.clone(),
                    normalization_options,
                );

                assert_eq!(
                    actual_output.client_id, expected_client,
                    "mismatch in routed client at step {step_number} output {index}"
                );
                assert_eq!(
                    actual_message, expected_message,
                    "mismatch in message shape/order at step {step_number} output {index}"
                );
            }
        }
    }

    fn assert_python_fanout_matches_server_runtime_for_scenario(
        scenario_name: &str,
    ) -> Result<(), InteropError> {
        assert_python_fanout_matches_server_runtime_for_scenario_with_overrides(
            scenario_name,
            None,
            None,
            false,
            false,
        )
    }

    fn assert_python_fanout_matches_server_runtime_for_scenario_with_motd_template(
        scenario_name: &str,
        runtime_motd_template: Option<&str>,
        probe_motd_template: Option<&str>,
    ) -> Result<(), InteropError> {
        assert_python_fanout_matches_server_runtime_for_scenario_with_overrides(
            scenario_name,
            runtime_motd_template,
            probe_motd_template,
            false,
            false,
        )
    }

    fn assert_python_fanout_matches_server_runtime_for_scenario_with_overrides(
        scenario_name: &str,
        runtime_motd_template: Option<&str>,
        probe_motd_template: Option<&str>,
        runtime_persistent_rooms_enabled: bool,
        probe_persistent_rooms_enabled: bool,
    ) -> Result<(), InteropError> {
        assert_python_fanout_matches_server_runtime_for_scenario_with_full_overrides(
            scenario_name,
            runtime_motd_template,
            probe_motd_template,
            runtime_persistent_rooms_enabled,
            probe_persistent_rooms_enabled,
            &[],
            &[],
        )
    }

    fn assert_python_fanout_matches_server_runtime_for_scenario_with_full_overrides(
        scenario_name: &str,
        runtime_motd_template: Option<&str>,
        probe_motd_template: Option<&str>,
        runtime_persistent_rooms_enabled: bool,
        probe_persistent_rooms_enabled: bool,
        runtime_permanent_rooms: &[&str],
        probe_permanent_rooms: &[&str],
    ) -> Result<(), InteropError> {
        let normalization_options =
            normalization_options_for_runtime_python_scenario(scenario_name);
        let steps = load_server_runtime_scenario_fixture(scenario_name)?;
        let rust_events = replay_server_runtime_scenario_steps_with_full_overrides(
            &steps,
            runtime_motd_template,
            runtime_persistent_rooms_enabled,
            runtime_permanent_rooms,
        )?;
        let python_events = run_python_fanout_roundtrip_with_full_overrides(
            &steps,
            probe_motd_template,
            probe_persistent_rooms_enabled,
            probe_permanent_rooms,
            false,
        )?;

        assert_eq!(python_events.len(), rust_events.len());
        for (index, (python_event, rust_event)) in
            python_events.iter().zip(rust_events.iter()).enumerate()
        {
            assert_eq!(
                python_event.client_id, rust_event.client_id,
                "request client mismatch at step {index}"
            );
            assert_eq!(
                python_event.request_line, rust_event.request_line,
                "request line mismatch at step {index}"
            );
            assert_eq!(
                python_event.outbound_lines.len(),
                rust_event.outbound_lines.len(),
                "outbound count mismatch at step {index}"
            );

            for (output_index, (python_output, rust_output)) in python_event
                .outbound_lines
                .iter()
                .zip(rust_event.outbound_lines.iter())
                .enumerate()
            {
                let python_value = normalize_cross_impl_message_with_options(
                    serde_json::from_str(&python_output.line)
                        .expect("python outbound line should decode as JSON"),
                    normalization_options,
                );
                let rust_value = normalize_cross_impl_message_with_options(
                    serde_json::from_str(&rust_output.line)
                        .expect("rust outbound line should decode as JSON"),
                    normalization_options,
                );
                assert_eq!(
                    python_output.client_id, rust_output.client_id,
                    "outbound client mismatch at step {index} output {output_index}"
                );
                assert_eq!(
                    python_value, rust_value,
                    "outbound message mismatch at step {index} output {output_index}"
                );
            }
        }

        Ok(())
    }

    fn assert_legacy_server_fanout_matches_server_runtime_for_scenario(
        scenario_name: &str,
    ) -> Result<(), InteropError> {
        assert_legacy_server_fanout_matches_server_runtime_for_scenario_with_overrides(
            scenario_name,
            None,
            None,
            false,
            false,
        )
    }

    fn assert_legacy_server_fanout_matches_server_runtime_for_scenario_with_motd_template(
        scenario_name: &str,
        runtime_motd_template: Option<&str>,
        legacy_motd_template: Option<&str>,
    ) -> Result<(), InteropError> {
        assert_legacy_server_fanout_matches_server_runtime_for_scenario_with_overrides(
            scenario_name,
            runtime_motd_template,
            legacy_motd_template,
            false,
            false,
        )
    }

    fn assert_legacy_server_fanout_matches_server_runtime_for_scenario_with_overrides(
        scenario_name: &str,
        runtime_motd_template: Option<&str>,
        legacy_motd_template: Option<&str>,
        runtime_persistent_rooms_enabled: bool,
        legacy_persistent_rooms_enabled: bool,
    ) -> Result<(), InteropError> {
        assert_legacy_server_fanout_matches_server_runtime_for_scenario_with_full_overrides(
            scenario_name,
            runtime_motd_template,
            legacy_motd_template,
            runtime_persistent_rooms_enabled,
            legacy_persistent_rooms_enabled,
            &[],
            &[],
        )
    }

    fn assert_legacy_server_fanout_matches_server_runtime_for_scenario_with_full_overrides(
        scenario_name: &str,
        runtime_motd_template: Option<&str>,
        legacy_motd_template: Option<&str>,
        runtime_persistent_rooms_enabled: bool,
        legacy_persistent_rooms_enabled: bool,
        runtime_permanent_rooms: &[&str],
        legacy_permanent_rooms: &[&str],
    ) -> Result<(), InteropError> {
        let normalization_options = normalization_options_for_legacy_scenario(scenario_name);
        let mut timing_canonicalizer = LegacyTimingCanonicalizer::default();
        let steps = load_server_runtime_scenario_fixture(scenario_name)?;
        let rust_events = replay_server_runtime_scenario_steps_with_full_overrides(
            &steps,
            runtime_motd_template,
            runtime_persistent_rooms_enabled,
            runtime_permanent_rooms,
        )?;
        let legacy_events = run_legacy_server_fanout_roundtrip_with_full_overrides(
            &steps,
            super::DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT,
            legacy_motd_template,
            legacy_persistent_rooms_enabled,
            legacy_permanent_rooms,
        )?;

        assert_eq!(legacy_events.len(), rust_events.len());
        for (index, (legacy_event, rust_event)) in
            legacy_events.iter().zip(rust_events.iter()).enumerate()
        {
            let mut legacy_outputs: Vec<(String, Value)> = Vec::new();
            for output in &legacy_event.outbound_lines {
                let include_output = decode_message_line(&output.line)
                    .ok()
                    .is_some_and(|message| !is_background_idle_state_message(&message));
                if !include_output {
                    continue;
                }

                let mut normalized = normalize_cross_impl_message_with_options(
                    serde_json::from_str(&output.line)
                        .expect("legacy outbound line should decode as JSON"),
                    normalization_options,
                );
                timing_canonicalizer
                    .canonicalize_message(&mut normalized, LegacyTimingSide::Legacy);
                canonicalize_legacy_hello_fields(&mut normalized);
                canonicalize_legacy_set_user_features(&mut normalized);
                canonicalize_legacy_list_fields(&mut normalized);
                legacy_outputs.push((output.client_id.clone(), normalized));
            }

            let mut rust_outputs: Vec<(String, Value)> = Vec::new();
            for output in &rust_event.outbound_lines {
                let include_output = decode_message_line(&output.line)
                    .ok()
                    .is_some_and(|message| !is_background_idle_state_message(&message));
                if !include_output {
                    continue;
                }

                let mut normalized = normalize_cross_impl_message_with_options(
                    serde_json::from_str(&output.line)
                        .expect("rust outbound line should decode as JSON"),
                    normalization_options,
                );
                timing_canonicalizer
                    .canonicalize_message(&mut normalized, LegacyTimingSide::Runtime);
                canonicalize_legacy_hello_fields(&mut normalized);
                canonicalize_legacy_set_user_features(&mut normalized);
                canonicalize_legacy_list_fields(&mut normalized);
                rust_outputs.push((output.client_id.clone(), normalized));
            }
            assert_eq!(
                legacy_event.client_id, rust_event.client_id,
                "request client mismatch at step {index}"
            );
            assert_eq!(
                legacy_event.request_line, rust_event.request_line,
                "request line mismatch at step {index}"
            );
            if legacy_outputs.len() != rust_outputs.len() {
                panic!(
                    "outbound count mismatch at step {index}: legacy={} rust={}; legacy_outputs={legacy_outputs:?}; rust_outputs={rust_outputs:?}",
                    legacy_outputs.len(),
                    rust_outputs.len()
                );
            }

            let mut unmatched_rust_outputs = rust_outputs.clone();
            for (output_index, legacy_output) in legacy_outputs.iter().enumerate() {
                if let Some(rust_index) = unmatched_rust_outputs
                    .iter()
                    .position(|rust_output| rust_output == legacy_output)
                {
                    unmatched_rust_outputs.remove(rust_index);
                    continue;
                }

                panic!(
                    "legacy output had no Rust match at step {index} output {output_index}: {legacy_output:?}"
                );
            }

            if !unmatched_rust_outputs.is_empty() {
                panic!(
                    "Rust produced unmatched outputs at step {index}: {unmatched_rust_outputs:?}"
                );
            }
        }

        Ok(())
    }

    fn is_background_idle_state_message(message: &ProtocolMessage) -> bool {
        match message {
            ProtocolMessage::State(payload) => {
                payload.state.playstate.as_ref().is_some_and(|playstate| {
                    playstate.paused == Some(true)
                        && playstate.do_seek != Some(true)
                        && playstate
                            .position
                            .is_some_and(|position| position.abs() <= 0.01)
                })
            }
            _ => false,
        }
    }

    fn legacy_server_prerequisites_missing(error: &InteropError) -> bool {
        match error {
            InteropError::LegacySyncplayCheckoutMissing(_) | InteropError::PythonSpawn { .. } => {
                true
            }
            InteropError::LegacyServerExited { stderr, .. }
            | InteropError::LegacyServerStartTimeout { stderr, .. } => {
                let lowered = stderr.to_ascii_lowercase();
                lowered.contains("no module named 'twisted'")
                    || lowered.contains("unable import twisted")
                    || lowered.contains("unable to import twisted")
            }
            _ => false,
        }
    }

    fn legacy_tls_fixture_directory() -> PathBuf {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("..");
        path.push("..");
        path.push("fixtures");
        path.push("tls");
        path
    }

    fn read_next_protocol_line_from_pending(pending_bytes: &mut Vec<u8>) -> Option<String> {
        loop {
            let newline_index = pending_bytes.iter().position(|byte| *byte == b'\n')?;
            let mut raw_line: Vec<u8> = pending_bytes.drain(..=newline_index).collect();
            if raw_line.last().is_some_and(|byte| *byte == b'\n') {
                raw_line.pop();
            }
            if raw_line.last().is_some_and(|byte| *byte == b'\r') {
                raw_line.pop();
            }
            if raw_line.is_empty() {
                continue;
            }

            let line = String::from_utf8_lossy(&raw_line).trim().to_owned();
            if line.is_empty() {
                continue;
            }
            if decode_message_line(&line).is_ok() {
                return Some(line);
            }
        }
    }

    fn read_plaintext_legacy_protocol_line_with_timeout(
        connection: &mut super::LegacyServerClientConnection,
        timeout: Duration,
    ) -> Result<String, InteropError> {
        let deadline = Instant::now() + timeout;
        let mut chunk = [0_u8; 4096];
        loop {
            if let Some(line) = read_next_protocol_line_from_pending(&mut connection.pending_bytes)
            {
                return Ok(line);
            }

            match connection.stream.read(&mut chunk) {
                Ok(0) => {}
                Ok(count) => connection.pending_bytes.extend_from_slice(&chunk[..count]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(InteropError::Io(error)),
            }

            if Instant::now() >= deadline {
                return Err(InteropError::InvalidPythonBatchResponse(
                    "timed out waiting for legacy plaintext protocol line".to_owned(),
                ));
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn read_tls_protocol_line_with_timeout(
        stream: &mut StreamOwned<ClientConnection, TcpStream>,
        timeout: Duration,
    ) -> Result<String, InteropError> {
        let deadline = Instant::now() + timeout;
        let mut pending_bytes = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            if let Some(line) = read_next_protocol_line_from_pending(&mut pending_bytes) {
                return Ok(line);
            }

            match stream.read(&mut chunk) {
                Ok(0) => {}
                Ok(count) => pending_bytes.extend_from_slice(&chunk[..count]),
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.kind() == std::io::ErrorKind::TimedOut => {}
                Err(error) => return Err(InteropError::Io(error)),
            }

            if Instant::now() >= deadline {
                return Err(InteropError::InvalidPythonBatchResponse(
                    "timed out waiting for legacy TLS protocol line".to_owned(),
                ));
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn open_legacy_tls_client_stream(
        stream: TcpStream,
        tls_cert_path: &Path,
    ) -> Result<StreamOwned<ClientConnection, TcpStream>, InteropError> {
        let cert_pem_path = tls_cert_path.join("cert.pem");
        let cert_pem = fs::read(&cert_pem_path)?;
        let certs = rustls_pemfile::certs(&mut Cursor::new(cert_pem))
            .collect::<Result<Vec<_>, _>>()
            .map_err(InteropError::Io)?;
        if certs.is_empty() {
            return Err(InteropError::InvalidPythonBatchResponse(format!(
                "legacy TLS cert bundle contains no certificates at {}",
                cert_pem_path.display()
            )));
        }

        let mut roots = RootCertStore::empty();
        for cert in certs {
            roots.add(cert).map_err(|error| {
                InteropError::InvalidPythonBatchResponse(format!(
                    "failed to add legacy TLS root certificate: {error}"
                ))
            })?;
        }

        let config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let server_name = ServerName::try_from("localhost").map_err(|error| {
            InteropError::InvalidPythonBatchResponse(format!(
                "invalid legacy TLS server name: {error}"
            ))
        })?;
        let connection =
            ClientConnection::new(Arc::new(config), server_name.to_owned()).map_err(|error| {
                InteropError::InvalidPythonBatchResponse(format!(
                    "failed to initialize legacy TLS client connection: {error}"
                ))
            })?;

        Ok(StreamOwned::new(connection, stream))
    }

    fn run_legacy_server_tls_upgrade_roundtrip_with_cert_path(
        tls_cert_path: &Path,
    ) -> Result<(String, String), InteropError> {
        let legacy_checkout = super::legacy_syncplay_checkout_dir();
        if !legacy_checkout.is_dir() {
            return Err(InteropError::LegacySyncplayCheckoutMissing(legacy_checkout));
        }

        let legacy_server_entry = super::legacy_syncplay_server_entry_script_path();
        if !legacy_server_entry.is_file() {
            return Err(InteropError::LegacyServerEntryScriptMissing(
                legacy_server_entry,
            ));
        }

        let port = super::reserve_ephemeral_tcp_port()?;
        let python_bin = super::python_bin_from_env();
        let python_bin_display = python_bin.to_string_lossy().to_string();
        let mut command = Command::new(&python_bin);
        command
            .arg(&legacy_server_entry)
            .arg("--port")
            .arg(port.to_string())
            .arg("--ipv4-only")
            .arg("--interface-ipv4")
            .arg("127.0.0.1")
            .arg("--salt")
            .arg(super::DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT)
            .arg("--tls")
            .arg(tls_cert_path)
            .current_dir(legacy_checkout)
            .env("PYTHONUNBUFFERED", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|source| InteropError::PythonSpawn {
                python: python_bin_display,
                source,
            })?;

        let result = (|| {
            super::wait_for_legacy_server_startup(port, &mut child)?;
            super::ensure_legacy_server_is_running(&mut child)?;

            let stream = super::connect_legacy_client_stream(port, "legacy-tls-client")?;
            let mut connection = super::LegacyServerClientConnection {
                stream,
                pending_bytes: Vec::new(),
            };
            let request_line =
                super::prepare_legacy_server_request_line(r#"{"TLS":{"startTLS":"send"}}"#)?;
            connection.stream.write_all(request_line.as_bytes())?;
            connection.stream.write_all(b"\r\n")?;
            connection.stream.flush()?;

            let tls_response_line = read_plaintext_legacy_protocol_line_with_timeout(
                &mut connection,
                Duration::from_secs(2),
            )?;
            let tls_message = decode_message_line(&tls_response_line)?;
            let ProtocolMessage::Tls(tls_payload) = tls_message else {
                return Err(InteropError::InvalidPythonBatchResponse(format!(
                    "expected legacy TLS response before upgrade, got: {tls_response_line}"
                )));
            };
            if tls_payload.tls.start_tls != "true" {
                return Err(InteropError::InvalidPythonBatchResponse(format!(
                    "legacy TLS upgrade denied by server response: {tls_response_line}"
                )));
            }

            connection.pending_bytes.clear();
            connection.stream.set_nonblocking(false)?;
            connection
                .stream
                .set_read_timeout(Some(Duration::from_secs(3)))?;
            connection
                .stream
                .set_write_timeout(Some(Duration::from_secs(3)))?;
            let mut tls_stream = open_legacy_tls_client_stream(connection.stream, tls_cert_path)?;

            let hello_line = encode_message_line(&ProtocolMessage::hello(
                default_rust_client_hello_for_interop(),
            ))?;
            tls_stream.write_all(hello_line.as_bytes())?;
            tls_stream.write_all(b"\r\n")?;
            tls_stream.flush()?;
            let hello_response_line =
                read_tls_protocol_line_with_timeout(&mut tls_stream, Duration::from_secs(3))?;
            let hello_message = decode_message_line(&hello_response_line)?;
            let _ = extract_hello_from_message(hello_message)?;

            Ok((tls_response_line, hello_response_line))
        })();

        super::terminate_legacy_server_process(&mut child);
        result
    }

    fn run_legacy_server_tls_logged_client_send_denied_roundtrip_with_cert_path(
        tls_cert_path: &Path,
    ) -> Result<String, InteropError> {
        let legacy_checkout = super::legacy_syncplay_checkout_dir();
        if !legacy_checkout.is_dir() {
            return Err(InteropError::LegacySyncplayCheckoutMissing(legacy_checkout));
        }

        let legacy_server_entry = super::legacy_syncplay_server_entry_script_path();
        if !legacy_server_entry.is_file() {
            return Err(InteropError::LegacyServerEntryScriptMissing(
                legacy_server_entry,
            ));
        }

        let port = super::reserve_ephemeral_tcp_port()?;
        let python_bin = super::python_bin_from_env();
        let python_bin_display = python_bin.to_string_lossy().to_string();
        let mut command = Command::new(&python_bin);
        command
            .arg(&legacy_server_entry)
            .arg("--port")
            .arg(port.to_string())
            .arg("--ipv4-only")
            .arg("--interface-ipv4")
            .arg("127.0.0.1")
            .arg("--salt")
            .arg(super::DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT)
            .arg("--tls")
            .arg(tls_cert_path)
            .current_dir(legacy_checkout)
            .env("PYTHONUNBUFFERED", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|source| InteropError::PythonSpawn {
                python: python_bin_display,
                source,
            })?;

        let result = (|| {
            super::wait_for_legacy_server_startup(port, &mut child)?;
            super::ensure_legacy_server_is_running(&mut child)?;

            // First verify TLS is actually available for unlogged clients in this legacy setup.
            let probe_stream = super::connect_legacy_client_stream(port, "legacy-tls-probe")?;
            let mut probe_connection = super::LegacyServerClientConnection {
                stream: probe_stream,
                pending_bytes: Vec::new(),
            };
            let tls_request_line =
                super::prepare_legacy_server_request_line(r#"{"TLS":{"startTLS":"send"}}"#)?;
            probe_connection
                .stream
                .write_all(tls_request_line.as_bytes())?;
            probe_connection.stream.write_all(b"\r\n")?;
            probe_connection.stream.flush()?;
            let probe_response_line = read_plaintext_legacy_protocol_line_with_timeout(
                &mut probe_connection,
                Duration::from_secs(2),
            )?;
            let probe_message = decode_message_line(&probe_response_line)?;
            let ProtocolMessage::Tls(probe_payload) = probe_message else {
                return Err(InteropError::InvalidPythonBatchResponse(format!(
                    "expected legacy TLS probe response, got: {probe_response_line}"
                )));
            };
            if probe_payload.tls.start_tls != "true" {
                return Err(InteropError::InvalidPythonBatchResponse(format!(
                    "legacy tls availability probe returned non-true response: {probe_response_line}"
                )));
            }

            let stream = super::connect_legacy_client_stream(port, "legacy-tls-logged-client")?;
            let mut connection = super::LegacyServerClientConnection {
                stream,
                pending_bytes: Vec::new(),
            };
            let hello_line = encode_message_line(&ProtocolMessage::hello(
                default_rust_client_hello_for_interop(),
            ))?;
            connection.stream.write_all(hello_line.as_bytes())?;
            connection.stream.write_all(b"\r\n")?;
            connection.stream.flush()?;

            let mut saw_hello = false;
            for _ in 0..8 {
                let line = read_plaintext_legacy_protocol_line_with_timeout(
                    &mut connection,
                    Duration::from_secs(2),
                )?;
                let message = decode_message_line(&line)?;
                if matches!(message, ProtocolMessage::Hello(_)) {
                    saw_hello = true;
                    break;
                }
            }
            if !saw_hello {
                return Err(InteropError::InvalidPythonBatchResponse(
                    "timed out waiting for legacy hello response before logged TLS probe"
                        .to_owned(),
                ));
            }

            connection.stream.write_all(tls_request_line.as_bytes())?;
            connection.stream.write_all(b"\r\n")?;
            connection.stream.flush()?;
            let logged_tls_response_line = read_plaintext_legacy_protocol_line_with_timeout(
                &mut connection,
                Duration::from_secs(2),
            )?;
            let logged_tls_message = decode_message_line(&logged_tls_response_line)?;
            let ProtocolMessage::Tls(logged_tls_payload) = logged_tls_message else {
                return Err(InteropError::InvalidPythonBatchResponse(format!(
                    "expected legacy logged TLS response, got: {logged_tls_response_line}"
                )));
            };
            if logged_tls_payload.tls.start_tls != "false" {
                return Err(InteropError::InvalidPythonBatchResponse(format!(
                    "legacy tls send was not denied for logged client: {logged_tls_response_line}"
                )));
            }

            Ok(logged_tls_response_line)
        })();

        super::terminate_legacy_server_process(&mut child);
        result
    }

    fn legacy_server_tls_prerequisites_missing(error: &InteropError) -> bool {
        if legacy_server_prerequisites_missing(error) {
            return true;
        }
        match error {
            InteropError::LegacyServerExited { stdout, stderr, .. } => {
                let lowered = format!("{stdout}\n{stderr}").to_ascii_lowercase();
                lowered.contains("no module named 'openssl'")
                    || lowered.contains("unable import openssl")
                    || lowered.contains("unable to import openssl")
                    || lowered.contains("error while loading the tls certificates")
                    || lowered.contains("tls support is not enabled")
            }
            InteropError::InvalidPythonBatchResponse(message) => {
                let lowered = message.to_ascii_lowercase();
                lowered.contains("legacy tls upgrade denied by server response")
                    || lowered.contains("legacy tls availability probe returned non-true response")
            }
            _ => false,
        }
    }

    fn legacy_client_protocol_prerequisites_missing(error: &InteropError) -> bool {
        match error {
            InteropError::LegacySyncplayCheckoutMissing(_) | InteropError::PythonSpawn { .. } => {
                true
            }
            InteropError::PythonProbeFailed { stdout, stderr, .. } => {
                let lowered = format!("{stdout}\n{stderr}").to_ascii_lowercase();
                lowered.contains("legacy-client-protocol-import-failed")
                    || lowered.contains("legacy-client-chat-import-failed")
                    || lowered.contains("no module named 'twisted'")
                    || lowered.contains("unable import twisted")
                    || lowered.contains("unable to import twisted")
            }
            _ => false,
        }
    }

    fn rust_file_payload_for_user(session: &ClientSession, username: &str) -> Option<Value> {
        match session.user_has_file(username) {
            Some(true) => {
                let mut file = serde_json::Map::new();
                if let Some(name) = session.user_file_name(username) {
                    file.insert("name".to_owned(), json!(name));
                }
                if let Some(size) = session.user_file_size(username) {
                    file.insert("size".to_owned(), size.clone());
                }
                if let Some(duration) = session.user_file_duration(username) {
                    file.insert("duration".to_owned(), duration.clone());
                }
                Some(Value::Object(file))
            }
            Some(false) => None,
            None => None,
        }
    }

    fn rust_user_file_snapshot(
        session: &ClientSession,
        usernames: &[&str],
    ) -> BTreeMap<String, Option<Value>> {
        let mut snapshot = BTreeMap::new();
        for username in usernames {
            if session.user_room(username).is_none() {
                continue;
            }
            snapshot.insert(
                (*username).to_owned(),
                rust_file_payload_for_user(session, username),
            );
        }
        snapshot
    }

    fn legacy_server_parity_assertions_enabled() -> bool {
        std::env::var("SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY")
            .ok()
            .is_some_and(|value| {
                value == "1"
                    || value.eq_ignore_ascii_case("true")
                    || value.eq_ignore_ascii_case("yes")
            })
    }

    fn legacy_tls_parity_prerequisites_strict_enabled() -> bool {
        std::env::var("SYNCPLAY_REQUIRE_LEGACY_TLS_PARITY")
            .ok()
            .is_some_and(|value| {
                value == "1"
                    || value.eq_ignore_ascii_case("true")
                    || value.eq_ignore_ascii_case("yes")
            })
    }

    #[test]
    fn protocol_hello_fixture_decodes() {
        assert!(fixture_decodes("hello_minimal.json"));
    }

    #[test]
    fn all_protocol_fixtures_decode() {
        let fixtures = all_protocol_fixture_names().expect("fixture names should be available");
        assert!(!fixtures.is_empty());

        for fixture in fixtures {
            assert!(
                fixture_decodes(&fixture),
                "expected fixture {fixture} to decode as protocol message"
            );
        }
    }

    #[test]
    fn fixture_decode_returns_typed_message() {
        let message = decode_fixture("tls_send.json").expect("tls fixture should decode");
        assert!(matches!(message, ProtocolMessage::Tls(_)));
    }

    #[test]
    fn decode_protocol_file_works_for_existing_fixture() {
        let path = super::fixture_path("error_message.json");
        assert!(decode_protocol_file(path.as_path()));
    }

    #[test]
    fn python_interop_roundtrip_returns_server_hello() {
        let transcript = match run_python_handshake_roundtrip() {
            Ok(transcript) => transcript,
            Err(InteropError::LegacySyncplayCheckoutMissing(_))
            | Err(InteropError::PythonSpawn { .. }) => {
                eprintln!(
                    "python interop handshake test skipped due to missing local prerequisites"
                );
                return;
            }
            Err(err) => panic!("python interop handshake should succeed, got: {err}"),
        };

        assert_eq!(transcript.response_hello.username, "interop-client");
        assert_eq!(transcript.response_hello.room.name, "interop-room");
        assert_eq!(transcript.response_hello.version, "syncplay-rs-dev");
        assert_eq!(
            transcript.response_hello.realversion.as_deref(),
            Some("1.7.5")
        );
    }

    #[test]
    fn python_interop_sequence_supports_list_set_and_state() {
        let requests = vec![
            ProtocolMessage::hello(default_rust_client_hello_for_interop()),
            ProtocolMessage::list_request(),
            ProtocolMessage::set(SetPayload::new().with_room(RoomRef::new("interop-room-2"))),
            ProtocolMessage::list_request(),
            ProtocolMessage::set(
                SetPayload::new().with_ready(
                    ReadyPayload::new(true)
                        .with_manually_initiated(true)
                        .with_username("interop-client"),
                ),
            ),
            ProtocolMessage::state(
                StatePayload::new().with_playstate(
                    PlaystatePayload::new()
                        .with_position(42.0)
                        .with_paused(false)
                        .with_do_seek(false),
                ),
            ),
        ];

        let transcript = match run_python_protocol_roundtrip(&requests) {
            Ok(transcript) => transcript,
            Err(InteropError::LegacySyncplayCheckoutMissing(_))
            | Err(InteropError::PythonSpawn { .. }) => {
                eprintln!(
                    "python interop sequence test skipped due to missing local prerequisites"
                );
                return;
            }
            Err(err) => panic!("python interop sequence should succeed, got: {err}"),
        };

        assert_eq!(transcript.steps.len(), requests.len());

        let hello = extract_hello_from_message(
            transcript.steps[0]
                .response_messages
                .first()
                .expect("hello step should return one message")
                .clone(),
        )
        .expect("first response should be hello");
        assert_eq!(hello.room.name, "interop-room");

        match transcript.steps[1]
            .response_messages
            .first()
            .expect("list response should be present")
        {
            ProtocolMessage::List(payload) => match &payload.list {
                ListPayload::Rooms(rooms) => {
                    assert!(rooms.contains_key("interop-room"));
                    let room = rooms.get("interop-room").expect("room should exist");
                    assert!(room.contains_key("interop-client"));
                }
                other => panic!("expected list room snapshot, got {other:?}"),
            },
            other => panic!("expected list response, got {}", other.kind()),
        }

        match transcript.steps[2]
            .response_messages
            .first()
            .expect("set room response should be present")
        {
            ProtocolMessage::Set(payload) => {
                let room = payload
                    .set
                    .room
                    .as_ref()
                    .expect("set room payload should exist");
                assert_eq!(room.name, "interop-room-2");
            }
            other => panic!("expected set response, got {}", other.kind()),
        }

        match transcript.steps[3]
            .response_messages
            .first()
            .expect("second list response should be present")
        {
            ProtocolMessage::List(payload) => match &payload.list {
                ListPayload::Rooms(rooms) => {
                    assert!(rooms.contains_key("interop-room-2"));
                    let room = rooms.get("interop-room-2").expect("room should exist");
                    assert!(room.contains_key("interop-client"));
                }
                other => panic!("expected list room snapshot, got {other:?}"),
            },
            other => panic!("expected list response, got {}", other.kind()),
        }

        match transcript.steps[4]
            .response_messages
            .first()
            .expect("set ready response should be present")
        {
            ProtocolMessage::Set(payload) => {
                let ready = payload
                    .set
                    .ready
                    .as_ref()
                    .expect("ready payload should be present");
                assert_eq!(ready.username.as_deref(), Some("interop-client"));
                assert!(ready.is_ready);
            }
            other => panic!("expected set response, got {}", other.kind()),
        }

        assert!(
            transcript.steps[5].response_messages.is_empty(),
            "state message should be accepted without an immediate response"
        );
    }

    #[test]
    fn legacy_python_same_filename_matches_client_core_on_edge_cases() {
        let pairs = [
            ("**Hidden filename**", "anything.mkv"),
            (
                "https://example.invalid/media/Movie%20Name.mkv",
                "Movie Name.mkv",
            ),
            ("Movie Name.mkv", "a9858cb4803c"),
            ("movie-a.mkv", "movie-b.mkv"),
        ];

        let legacy_results = match run_python_same_filename_batch(&pairs) {
            Ok(results) => results,
            Err(InteropError::LegacySyncplayCheckoutMissing(_))
            | Err(InteropError::PythonSpawn { .. }) => {
                eprintln!(
                    "legacy same-filename parity test skipped due to missing local prerequisites"
                );
                return;
            }
            Err(err) => panic!("legacy same-filename probe should succeed, got: {err}"),
        };

        assert_eq!(legacy_results.len(), pairs.len());
        for ((left, right), legacy_result) in pairs.iter().zip(legacy_results) {
            let rust_result = ClientSession::same_filename_legacy_compatible(left, right);
            assert_eq!(
                rust_result, legacy_result,
                "same-filename mismatch for pair ({left:?}, {right:?})"
            );
        }
    }

    #[test]
    fn legacy_python_same_filesize_matches_client_core_on_edge_cases() {
        let pairs = vec![
            (json!(0), json!(123456789)),
            (json!(123456789), json!("15e2b0d3c338")),
            (json!(123456789), json!(123456789)),
            (json!(123456789), json!(987654321)),
            (json!("0"), json!(123456789)),
            (json!("ABCDEF"), json!("abcdef")),
        ];

        let legacy_results = match run_python_same_filesize_batch(&pairs) {
            Ok(results) => results,
            Err(InteropError::LegacySyncplayCheckoutMissing(_))
            | Err(InteropError::PythonSpawn { .. }) => {
                eprintln!(
                    "legacy same-filesize parity test skipped due to missing local prerequisites"
                );
                return;
            }
            Err(err) => panic!("legacy same-filesize probe should succeed, got: {err}"),
        };

        assert_eq!(legacy_results.len(), pairs.len());
        for ((left, right), legacy_result) in pairs.iter().zip(legacy_results) {
            let rust_result = ClientSession::same_filesize_legacy_compatible(left, right);
            assert_eq!(
                rust_result, legacy_result,
                "same-filesize mismatch for pair ({left:?}, {right:?})"
            );
        }
    }

    #[test]
    fn legacy_python_same_fileduration_matches_client_core_on_edge_cases() {
        let pairs = vec![
            (10.49, 12.49),
            (10.49, 13.49),
            (1.5, 4.5),
            (100.0, 100.0),
            (-1.5, 1.5),
        ];

        let legacy_results = match run_python_same_fileduration_batch(&pairs) {
            Ok(results) => results,
            Err(InteropError::LegacySyncplayCheckoutMissing(_))
            | Err(InteropError::PythonSpawn { .. }) => {
                eprintln!(
                    "legacy same-fileduration parity test skipped due to missing local prerequisites"
                );
                return;
            }
            Err(err) => panic!("legacy same-fileduration probe should succeed, got: {err}"),
        };

        assert_eq!(legacy_results.len(), pairs.len());
        for ((left, right), legacy_result) in pairs.iter().zip(legacy_results) {
            let rust_result = ClientSession::same_fileduration_legacy_compatible(*left, *right);
            assert_eq!(
                rust_result, legacy_result,
                "same-fileduration mismatch for pair ({left:?}, {right:?})"
            );
        }
    }

    #[test]
    fn legacy_python_same_fileduration_with_config_overrides_matches_client_core_on_edge_cases() {
        let pairs = vec![(10.49, 12.49), (10.49, 13.49), (1.5, 4.5)];
        let scenarios = [
            ("duration-notifications-disabled", Some(false), None),
            ("tight-threshold", Some(true), Some(1.0)),
            ("wide-threshold", Some(true), Some(3.5)),
        ];

        for (scenario_name, show_duration_notification, different_duration_threshold) in scenarios {
            let legacy_results = match run_python_same_fileduration_batch_with_overrides(
                &pairs,
                show_duration_notification,
                different_duration_threshold,
            ) {
                Ok(results) => results,
                Err(InteropError::LegacySyncplayCheckoutMissing(_))
                | Err(InteropError::PythonSpawn { .. }) => {
                    eprintln!(
                        "legacy same-fileduration override parity test skipped due to missing local prerequisites"
                    );
                    return;
                }
                Err(err) => panic!(
                    "legacy same-fileduration override probe should succeed for '{scenario_name}', got: {err}"
                ),
            };

            assert_eq!(legacy_results.len(), pairs.len());
            for ((left, right), legacy_result) in pairs.iter().zip(legacy_results) {
                let rust_result = ClientSession::same_fileduration_legacy_compatible_with_overrides(
                    *left,
                    *right,
                    show_duration_notification.unwrap_or(true),
                    different_duration_threshold.unwrap_or(2.5),
                );
                assert_eq!(
                    rust_result, legacy_result,
                    "same-fileduration override mismatch for scenario '{scenario_name}' pair ({left:?}, {right:?})"
                );
            }
        }
    }

    #[test]
    fn legacy_python_privacy_file_payload_batch_matches_client_core_behavior() {
        let cases = vec![
            (
                json!({
                    "name": "https://example.invalid/media/Movie Name.mkv",
                    "size": 123456789,
                    "duration": 95.5,
                    "path": "C:/media/movie.mkv",
                    "extra": "keep-me"
                }),
                "SendRaw",
                "SendRaw",
            ),
            (
                json!({
                    "name": "https://example.invalid/media/Movie Name.mkv",
                    "size": 123456789,
                    "duration": 95.5,
                    "path": "C:/media/movie.mkv",
                    "extra": "keep-me"
                }),
                "SendHashed",
                "SendHashed",
            ),
            (
                json!({
                    "name": "movie.mkv",
                    "size": 123456789,
                    "duration": 95.5,
                    "path": "C:/media/movie.mkv"
                }),
                "DoNotSend",
                "DoNotSend",
            ),
        ];

        let legacy_results = match run_python_privacy_file_payload_batch(&cases) {
            Ok(results) => results,
            Err(InteropError::LegacySyncplayCheckoutMissing(_))
            | Err(InteropError::PythonSpawn { .. }) => {
                eprintln!(
                    "legacy privacy file payload parity test skipped due to missing local prerequisites"
                );
                return;
            }
            Err(err) => panic!("legacy privacy file payload probe should succeed, got: {err}"),
        };

        assert_eq!(legacy_results.len(), cases.len());
        for ((file_payload, filename_privacy_mode, filesize_privacy_mode), legacy_result) in
            cases.iter().zip(legacy_results)
        {
            let mut session = ClientSession::default();
            session
                .apply_message_json(
                    r#"{"Hello":{"username":"interop-client","room":{"name":"room1"},"version":"1.2.255"}}"#,
                )
                .expect("hello should apply");

            let filename_mode = PrivacyMode::from_legacy_name(filename_privacy_mode)
                .expect("filename privacy mode should map to Rust mode");
            let filesize_mode = PrivacyMode::from_legacy_name(filesize_privacy_mode)
                .expect("filesize privacy mode should map to Rust mode");
            let actions = session.runtime_actions_for_local_file_publish_legacy_compatible(
                file_payload,
                filename_mode,
                filesize_mode,
            );
            assert_eq!(
                actions.len(),
                1,
                "local file publish should emit one action"
            );
            let ClientRuntimeAction::SetFile { file_payload } = &actions[0] else {
                panic!("local file publish should emit SetFile action");
            };
            assert_eq!(
                file_payload, &legacy_result,
                "privacy file payload mismatch for modes ({filename_privacy_mode}, {filesize_privacy_mode})"
            );
        }
    }

    #[test]
    fn legacy_client_chat_send_contract_matches_client_core_behavior() {
        let cases = vec![
            LegacyClientChatSendContractCase {
                message: "hello room".to_owned(),
                protocol_logged: true,
                server_version: "1.7.5".to_owned(),
                chat_supported: Some(true),
                max_chat_message_length: Some(150),
                derive_server_features: false,
                feature_list: None,
            },
            LegacyClientChatSendContractCase {
                message: "hello\nroom\r!".to_owned(),
                protocol_logged: true,
                server_version: "1.7.5".to_owned(),
                chat_supported: Some(true),
                max_chat_message_length: Some(5),
                derive_server_features: false,
                feature_list: None,
            },
            LegacyClientChatSendContractCase {
                message: "chat disabled".to_owned(),
                protocol_logged: true,
                server_version: "1.7.5".to_owned(),
                chat_supported: Some(false),
                max_chat_message_length: Some(150),
                derive_server_features: false,
                feature_list: None,
            },
            LegacyClientChatSendContractCase {
                message: "legacy fallback disabled".to_owned(),
                protocol_logged: true,
                server_version: "1.4.9".to_owned(),
                chat_supported: None,
                max_chat_message_length: None,
                derive_server_features: true,
                feature_list: None,
            },
            LegacyClientChatSendContractCase {
                message: "x".repeat(60),
                protocol_logged: true,
                server_version: "1.7.5".to_owned(),
                chat_supported: None,
                max_chat_message_length: None,
                derive_server_features: true,
                feature_list: None,
            },
            LegacyClientChatSendContractCase {
                message: "1234567890".to_owned(),
                protocol_logged: true,
                server_version: "1.7.5".to_owned(),
                chat_supported: None,
                max_chat_message_length: None,
                derive_server_features: true,
                feature_list: Some(json!({"maxChatMessageLength": 7})),
            },
            LegacyClientChatSendContractCase {
                message: "disconnected transport".to_owned(),
                protocol_logged: false,
                server_version: "1.7.5".to_owned(),
                chat_supported: Some(true),
                max_chat_message_length: Some(150),
                derive_server_features: false,
                feature_list: None,
            },
            LegacyClientChatSendContractCase {
                message: "feature-list disabled".to_owned(),
                protocol_logged: true,
                server_version: "1.7.5".to_owned(),
                chat_supported: None,
                max_chat_message_length: None,
                derive_server_features: true,
                feature_list: Some(json!({"chat": false})),
            },
        ];

        let legacy_results = match run_python_legacy_client_chat_send_contract_batch(&cases) {
            Ok(results) => results,
            Err(err) if legacy_client_protocol_prerequisites_missing(&err) => {
                eprintln!(
                    "legacy client chat-send contract test skipped due to missing local prerequisites: {err}"
                );
                return;
            }
            Err(err) => panic!("legacy client chat-send contract probe should succeed, got: {err}"),
        };

        assert_eq!(legacy_results.len(), cases.len());
        for (case, legacy_result) in cases.iter().zip(legacy_results.iter()) {
            let mut session = ClientSession::default();
            let mut features = case
                .feature_list
                .clone()
                .map(|value| {
                    value.as_object().cloned().unwrap_or_else(|| {
                        panic!("feature_list should be an object when present: {value:?}")
                    })
                })
                .unwrap_or_default();
            if let Some(chat_supported) = case.chat_supported {
                features
                    .entry("chat".to_owned())
                    .or_insert(Value::Bool(chat_supported));
            }
            if let Some(max_chat_message_length) = case.max_chat_message_length {
                features
                    .entry("maxChatMessageLength".to_owned())
                    .or_insert(json!(max_chat_message_length));
            }
            let hello_line = if features.is_empty() {
                json!({
                    "Hello": {
                        "username": "interop-client",
                        "room": {"name": "room1"},
                        "version": case.server_version,
                    }
                })
            } else {
                json!({
                    "Hello": {
                        "username": "interop-client",
                        "room": {"name": "room1"},
                        "version": case.server_version,
                        "features": Value::Object(features),
                    }
                })
            }
            .to_string();
            session
                .apply_message_json(&hello_line)
                .expect("hello should apply");
            if !case.protocol_logged {
                let _ = session.handle_disconnect(0.0);
            }

            let rust_messages = session
                .runtime_actions_for_outbound_chat_message(case.message.clone())
                .into_iter()
                .filter_map(|action| match action {
                    ClientRuntimeAction::SendChat { message } => Some(message),
                    _ => None,
                })
                .collect::<Vec<_>>();

            assert_eq!(
                rust_messages, legacy_result.sent_messages,
                "outbound chat mismatch for case: {case:?}",
            );
            if session.server_chat_supported() == Some(false) {
                assert!(
                    !legacy_result.error_messages.is_empty(),
                    "legacy client should emit a not-supported error when chat is disabled: {case:?}"
                );
            }
        }
    }

    #[test]
    fn legacy_client_set_file_contract_matches_client_core_behavior() {
        let legacy_contract = match run_python_legacy_client_set_file_contract_probe() {
            Ok(contract) => contract,
            Err(err) if legacy_client_protocol_prerequisites_missing(&err) => {
                eprintln!(
                    "legacy client set-file contract test skipped due to missing local prerequisites: {err}"
                );
                return;
            }
            Err(err) => panic!("legacy client set-file contract probe should succeed, got: {err}"),
        };

        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        session
            .apply_message_json(
                r#"{"Set":{"file":{"name":"movie.mkv","duration":95.5,"size":123456789}}}"#,
            )
            .expect("set file should parse");
        let rust_file_payload_ignored = session.user_has_file("alice") == Some(false)
            && session.user_file_name("alice").is_none();

        session
            .apply_message_json(r#"{"Set":{"file":{}}}"#)
            .expect("empty set file should parse");
        let rust_empty_payload_ignored = session.user_has_file("alice") == Some(false)
            && session.user_file_name("alice").is_none();

        assert_eq!(
            rust_file_payload_ignored, legacy_contract.file_payload_ignored,
            "Rust top-level Set.file handling diverges from legacy client contract"
        );
        assert_eq!(
            rust_empty_payload_ignored, legacy_contract.empty_payload_ignored,
            "Rust empty top-level Set.file handling diverges from legacy client contract"
        );
        assert!(
            legacy_contract.file_payload_calls.is_empty(),
            "legacy probe expected zero calls for top-level file payload, got: {:?}",
            legacy_contract.file_payload_calls
        );
        assert!(
            legacy_contract.empty_payload_calls.is_empty(),
            "legacy probe expected zero calls for top-level empty file payload, got: {:?}",
            legacy_contract.empty_payload_calls
        );
    }

    #[test]
    fn legacy_client_user_file_metadata_contract_matches_client_core_behavior() {
        let legacy_probe = match run_python_legacy_client_user_file_metadata_probe() {
            Ok(probe) => probe,
            Err(err) if legacy_client_protocol_prerequisites_missing(&err) => {
                eprintln!(
                    "legacy client user-file metadata test skipped due to missing local prerequisites: {err}"
                );
                return;
            }
            Err(err) => panic!("legacy client user-file metadata probe should succeed, got: {err}"),
        };

        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"interop-client","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        session
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"file":{"name":"**Hidden filename**","size":"15e2b0d3c338","duration":95}},"bob":{"room":{"name":"room1"},"file":{"name":"movie.mkv","size":123456789,"duration":95.5}}}}}"#,
            )
            .expect("set mixed user metadata should apply");
        let after_set_mixed = rust_user_file_snapshot(&session, &["alice", "bob", "charlie"]);

        session
            .apply_message_json(r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{}}}}}"#)
            .expect("set empty file payload should apply");
        let after_set_empty = rust_user_file_snapshot(&session, &["alice", "bob", "charlie"]);

        session
            .apply_message_json(
                r#"{"List":{"room1":{"alice":{"file":{"name":"**Hidden filename**","size":"15e2b0d3c338","duration":95},"controller":false,"isReady":true,"features":{}},"bob":{"file":{"name":"movie.mkv","size":123456789,"duration":95.5},"controller":false,"isReady":false,"features":{}}},"room2":{"charlie":{"file":{"name":"a9858cb4803c","size":"15e2b0d3c338","duration":95.0},"controller":true,"isReady":true,"features":{}}}}}"#,
            )
            .expect("list mixed metadata payload should apply");
        let after_list_mixed = rust_user_file_snapshot(&session, &["alice", "bob", "charlie"]);

        session
            .apply_message_json(
                r#"{"List":{"room1":{"alice":{"file":{"name":"**Hidden filename**","size":"15e2b0d3c338","duration":95},"controller":false,"isReady":true,"features":{}},"bob":{"file":{},"controller":false,"isReady":false,"features":{}}},"room2":{"charlie":{"file":{"name":"a9858cb4803c","size":"15e2b0d3c338","duration":95.0},"controller":true,"isReady":true,"features":{}}}}}"#,
            )
            .expect("list empty file payload should apply");
        let after_list_clears = rust_user_file_snapshot(&session, &["alice", "bob", "charlie"]);

        assert_eq!(
            after_set_mixed, legacy_probe.after_set_mixed,
            "Rust Set.user mixed file metadata snapshot diverges from legacy client behavior"
        );
        assert_eq!(
            after_set_empty, legacy_probe.after_set_empty,
            "Rust Set.user empty file payload semantics diverge from legacy client behavior"
        );
        assert_eq!(
            after_list_mixed, legacy_probe.after_list_mixed,
            "Rust List mixed file metadata snapshot diverges from legacy client behavior"
        );
        assert_eq!(
            after_list_clears, legacy_probe.after_list_clears,
            "Rust List empty file payload semantics diverge from legacy client behavior"
        );
    }

    #[test]
    fn scripted_server_runtime_scenario_replays_and_fanout_decodes() {
        let events = replay_server_runtime_scenario_fixture("server_runtime_fanout.jsonl")
            .expect("scenario fixture should replay through server runtime");
        assert_eq!(events.len(), 7);

        let mut saw_bob_join_to_alice = false;
        let mut saw_bob_room2_update_to_alice = false;
        let mut saw_ready_broadcast_to_bob = false;
        let mut saw_state_echo_to_bob = false;

        for event in &events {
            for outbound in &event.outbound_lines {
                let message = decode_message_line(&outbound.line)
                    .expect("fanout output line should decode as protocol message");
                match message {
                    ProtocolMessage::Set(payload) => {
                        if let Some(user_map) = payload.set.user.as_ref() {
                            if outbound.client_id == "client-1"
                                && user_map
                                    .get("bob")
                                    .and_then(|u| u.event.as_ref())
                                    .and_then(|event| event.get("joined"))
                                    == Some(&json!(true))
                            {
                                saw_bob_join_to_alice = true;
                            }
                            if outbound.client_id == "client-1"
                                && user_map
                                    .get("bob")
                                    .and_then(|u| u.room.as_ref())
                                    .map(|room| room.name.as_str())
                                    == Some("room2")
                            {
                                saw_bob_room2_update_to_alice = true;
                            }
                        }
                        if let Some(ready) = payload.set.ready.as_ref() {
                            if outbound.client_id == "client-2"
                                && ready.username.as_deref() == Some("alice")
                                && ready.is_ready
                            {
                                saw_ready_broadcast_to_bob = true;
                            }
                        }
                    }
                    ProtocolMessage::State(payload) => {
                        if outbound.client_id == "client-2"
                            && payload.state.playstate.as_ref().is_some_and(|playstate| {
                                playstate.set_by.as_deref() == Some("bob")
                                    && playstate.position == Some(10.0)
                                    && playstate.paused == Some(false)
                                    && playstate.do_seek == Some(false)
                            })
                        {
                            saw_state_echo_to_bob = true;
                        }
                    }
                    ProtocolMessage::Hello(_)
                    | ProtocolMessage::List(_)
                    | ProtocolMessage::Chat(_)
                    | ProtocolMessage::Error(_)
                    | ProtocolMessage::Tls(_) => {}
                }
            }
        }

        assert!(
            saw_bob_join_to_alice,
            "scenario should include bob join fanout to alice"
        );
        assert!(
            saw_bob_room2_update_to_alice,
            "scenario should include bob room2 user-update fanout to alice"
        );
        assert!(
            saw_ready_broadcast_to_bob,
            "scenario should include alice ready-state fanout to bob"
        );
        assert!(
            saw_state_echo_to_bob,
            "scenario should include bob state reflection after moving rooms"
        );
    }

    #[test]
    fn scripted_server_runtime_state_propagation_scenario_replays_state_fanout() {
        let events =
            replay_server_runtime_scenario_fixture("server_runtime_state_propagation.jsonl")
                .expect("state propagation scenario fixture should replay through server runtime");
        assert_eq!(events.len(), 5);

        let state_event = events.get(2).expect("step 3 state event should be present");
        assert_eq!(state_event.client_id, "client-1");
        assert_eq!(state_event.outbound_lines.len(), 2);
        for outbound in &state_event.outbound_lines {
            let message =
                decode_message_line(&outbound.line).expect("state fanout line should decode");
            match message {
                ProtocolMessage::State(payload) => {
                    let playstate = payload
                        .state
                        .playstate
                        .as_ref()
                        .expect("state fanout should include playstate");
                    assert_eq!(playstate.set_by.as_deref(), Some("alice"));
                    assert_eq!(playstate.position, Some(12.5));
                    assert_eq!(playstate.paused, Some(false));
                    assert_eq!(playstate.do_seek, Some(false));
                    assert!(
                        payload
                            .state
                            .ping
                            .as_ref()
                            .is_some_and(|ping| ping.latency_calculation.is_some()),
                        "state fanout should include ping metadata"
                    );
                    assert!(
                        payload
                            .state
                            .ignoring_on_the_fly
                            .as_ref()
                            .is_some_and(|ignore| ignore.server == Some(1)),
                        "state fanout should include ignoringOnTheFly server counter"
                    );
                }
                other => panic!("expected state response at step 3, got {}", other.kind()),
            }
        }

        let unchanged_state_event = events
            .get(3)
            .expect("step 4 unchanged-playstate event should be present");
        assert_eq!(unchanged_state_event.client_id, "client-1");
        assert!(
            unchanged_state_event.outbound_lines.is_empty(),
            "playstate updates without seek/pause transitions should not produce immediate fanout"
        );

        let ping_only_event = events
            .get(4)
            .expect("step 5 ping-only state event should be present");
        assert_eq!(ping_only_event.client_id, "client-1");
        assert!(
            ping_only_event.outbound_lines.is_empty(),
            "ping-only state updates should not produce immediate fanout"
        );
    }

    #[test]
    fn scripted_server_runtime_state_metadata_forwarding_scenario_replays_sender_passthrough() {
        let events = replay_server_runtime_scenario_fixture(
            "server_runtime_state_metadata_forwarding.jsonl",
        )
        .expect("state metadata forwarding scenario fixture should replay");
        assert_eq!(events.len(), 6);

        let first_forced_event = events
            .get(2)
            .expect("step 3 first forced state event should be present");
        assert_eq!(first_forced_event.outbound_lines.len(), 2);
        for outbound in &first_forced_event.outbound_lines {
            let message = decode_message_line(&outbound.line)
                .expect("step 3 state fanout line should decode");
            let ProtocolMessage::State(payload) = message else {
                panic!("step 3 outputs should be state updates");
            };
            let ping = payload
                .state
                .ping
                .as_ref()
                .expect("step 3 state update should include ping");
            let ignore = payload
                .state
                .ignoring_on_the_fly
                .as_ref()
                .expect("step 3 state update should include ignore counters");
            if outbound.client_id == "client-1" {
                assert_eq!(ping.client_latency_calculation, Some(124.1));
                assert_eq!(ignore.client, Some(4));
            } else {
                assert_eq!(ping.client_latency_calculation, None);
                assert_eq!(ignore.client, None);
            }
        }

        let second_forced_event = events
            .get(3)
            .expect("step 4 second forced state event should be present");
        assert_eq!(second_forced_event.outbound_lines.len(), 2);
        for outbound in &second_forced_event.outbound_lines {
            let message = decode_message_line(&outbound.line)
                .expect("step 4 state fanout line should decode");
            let ProtocolMessage::State(payload) = message else {
                panic!("step 4 outputs should be state updates");
            };
            assert_eq!(
                payload
                    .state
                    .ping
                    .as_ref()
                    .and_then(|ping| ping.client_latency_calculation),
                None,
                "client latency passthrough should be consumed after first forced send"
            );
            assert_eq!(
                payload
                    .state
                    .ignoring_on_the_fly
                    .as_ref()
                    .and_then(|ignore| ignore.client),
                None,
                "client ignore passthrough should be consumed after first forced send"
            );
        }

        let ping_only_event = events
            .get(4)
            .expect("step 5 ping-only metadata event should be present");
        assert!(
            ping_only_event.outbound_lines.is_empty(),
            "step 5 ping-only metadata should not produce immediate fanout"
        );

        let final_forced_event = events
            .get(5)
            .expect("step 6 forced pause-change event should be present");
        assert_eq!(final_forced_event.outbound_lines.len(), 2);
        let sender_output = final_forced_event
            .outbound_lines
            .iter()
            .find(|output| output.client_id == "client-1")
            .expect("step 6 should include sender-directed forced state output");
        let sender_message =
            decode_message_line(&sender_output.line).expect("step 6 sender output should decode");
        let ProtocolMessage::State(payload) = sender_message else {
            panic!("step 6 sender output should be state update");
        };
        assert_eq!(
            payload
                .state
                .ping
                .as_ref()
                .and_then(|ping| ping.client_latency_calculation),
            Some(126.1),
            "queued ping metadata should be forwarded on next forced update"
        );
        assert_eq!(
            payload
                .state
                .ignoring_on_the_fly
                .as_ref()
                .and_then(|ignore| ignore.client),
            Some(8),
            "queued client ignore counter should be forwarded on next forced update"
        );
    }

    #[test]
    fn python_fanout_roundtrip_matches_runtime_on_state_ping_forward_delay_metrics() {
        let steps = vec![
            ServerRuntimeScenarioStep {
                client_id: "client-1".to_owned(),
                request_line: r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#
                    .to_owned(),
                advance_seconds: 0.0,
            },
            ServerRuntimeScenarioStep {
                client_id: "client-2".to_owned(),
                request_line: r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.2.255"}}"#
                    .to_owned(),
                advance_seconds: 0.0,
            },
            ServerRuntimeScenarioStep {
                client_id: "client-1".to_owned(),
                request_line: r#"{"State":{"playstate":{"position":5.0,"paused":false,"doSeek":true},"ping":{"latencyCalculation":-10.0,"clientRtt":2.0}}}"#
                    .to_owned(),
                advance_seconds: 0.0,
            },
        ];
        let rust_events = replay_server_runtime_scenario_steps(&steps)
            .expect("state ping-forward-delay scenario should replay through runtime");
        let python_events = match run_python_fanout_roundtrip(&steps) {
            Ok(events) => events,
            Err(InteropError::LegacySyncplayCheckoutMissing(_))
            | Err(InteropError::PythonSpawn { .. }) => {
                eprintln!("python fanout interop test skipped due to missing local prerequisites");
                return;
            }
            Err(err) => {
                panic!("python fanout interop for ping-forward-delay scenario failed: {err}")
            }
        };

        let rust_state_event = rust_events
            .get(2)
            .expect("runtime state step should exist")
            .outbound_lines
            .iter()
            .find(|line| line.client_id == "client-1")
            .expect("runtime sender output should exist");
        let python_state_event = python_events
            .get(2)
            .expect("python state step should exist")
            .outbound_lines
            .iter()
            .find(|line| line.client_id == "client-1")
            .expect("python sender output should exist");

        let rust_message = decode_message_line(&rust_state_event.line)
            .expect("runtime sender output should decode");
        let python_message = decode_message_line(&python_state_event.line)
            .expect("python sender output should decode");
        let ProtocolMessage::State(rust_payload) = rust_message else {
            panic!("runtime sender output should be state");
        };
        let ProtocolMessage::State(python_payload) = python_message else {
            panic!("python sender output should be state");
        };

        let rust_position = rust_payload
            .state
            .playstate
            .as_ref()
            .and_then(|playstate| playstate.position)
            .expect("runtime state should include playstate position");
        let python_position = python_payload
            .state
            .playstate
            .as_ref()
            .and_then(|playstate| playstate.position)
            .expect("python state should include playstate position");
        assert!(
            (rust_position - 18.0).abs() <= 0.000_001,
            "runtime should apply forward delay to position"
        );
        assert!(
            (python_position - 18.0).abs() <= 0.000_001,
            "python probe should apply forward delay to position"
        );

        let rust_server_rtt = rust_payload
            .state
            .ping
            .as_ref()
            .and_then(|ping| ping.server_rtt)
            .expect("runtime state should include serverRtt");
        let python_server_rtt = python_payload
            .state
            .ping
            .as_ref()
            .and_then(|ping| ping.server_rtt)
            .expect("python state should include serverRtt");
        assert!(
            (rust_server_rtt - 10.0).abs() <= 0.000_001,
            "runtime sender serverRtt should reflect ping RTT update"
        );
        assert!(
            (python_server_rtt - 10.0).abs() <= 0.000_001,
            "python sender serverRtt should reflect ping RTT update"
        );
    }

    #[test]
    fn scripted_server_runtime_state_latency_metrics_scenario_applies_forward_delay_and_sender_rtt()
    {
        let events =
            replay_server_runtime_scenario_fixture("server_runtime_state_latency_metrics.jsonl")
                .expect("state latency-metrics scenario fixture should replay through runtime");
        assert_eq!(events.len(), 3);

        let state_event = events.get(2).expect("step 3 state event should be present");
        assert_eq!(state_event.client_id, "client-1");
        assert_eq!(state_event.outbound_lines.len(), 2);

        let mut saw_sender = false;
        let mut saw_peer = false;

        for outbound in &state_event.outbound_lines {
            let message =
                decode_message_line(&outbound.line).expect("step 3 outbound line should decode");
            let ProtocolMessage::State(payload) = message else {
                panic!("step 3 outputs should be state updates");
            };
            let playstate = payload
                .state
                .playstate
                .as_ref()
                .expect("step 3 state update should include playstate");
            assert_eq!(playstate.set_by.as_deref(), Some("alice"));
            assert_eq!(playstate.paused, Some(false));
            assert_eq!(playstate.do_seek, Some(true));
            assert!(
                (playstate
                    .position
                    .expect("state update should include position")
                    - 18.0)
                    .abs()
                    <= 0.000_001,
                "forward delay should be applied to shared position"
            );

            let ping = payload
                .state
                .ping
                .as_ref()
                .expect("step 3 state update should include ping");
            let server_rtt = ping
                .server_rtt
                .expect("state update should include serverRtt");

            if outbound.client_id == "client-1" {
                saw_sender = true;
                assert!(
                    (server_rtt - 10.0).abs() <= 0.000_001,
                    "sender-directed update should include derived non-zero serverRtt"
                );
            } else if outbound.client_id == "client-2" {
                saw_peer = true;
                assert_eq!(
                    server_rtt, 0.0,
                    "peer-directed update should retain default serverRtt"
                );
            } else {
                panic!("unexpected outbound recipient '{}'", outbound.client_id);
            }
        }

        assert!(
            saw_sender,
            "step 3 should include sender-directed state update"
        );
        assert!(saw_peer, "step 3 should include peer-directed state update");
    }

    #[test]
    fn scripted_server_runtime_state_periodic_timeout_scenario_emits_periodic_and_drops_stale_client()
     {
        let events =
            replay_server_runtime_scenario_fixture("server_runtime_state_periodic_timeout.jsonl")
                .expect("periodic-timeout scenario fixture should replay through server runtime");
        assert_eq!(events.len(), 5);

        let periodic_event = events
            .get(2)
            .expect("step 3 periodic-state event should be present");
        assert!(
            periodic_event.outbound_lines.iter().any(|line| {
                line.client_id == "client-1"
                    && decode_message_line(&line.line)
                        .ok()
                        .is_some_and(|message| matches!(message, ProtocolMessage::State(_)))
            }),
            "step 3 should include periodic state updates for stale client"
        );
        assert!(
            periodic_event.outbound_lines.iter().any(|line| {
                line.client_id == "client-2"
                    && decode_message_line(&line.line)
                        .ok()
                        .is_some_and(|message| matches!(message, ProtocolMessage::State(_)))
            }),
            "step 3 should include periodic state updates for active client"
        );

        let timeout_event = events
            .get(3)
            .expect("step 4 timeout event should be present");
        assert!(
            timeout_event.outbound_lines.iter().any(|line| {
                if line.client_id != "client-2" {
                    return false;
                }
                decode_message_line(&line.line).ok().is_some_and(|message| {
                    matches!(
                        message,
                        ProtocolMessage::Set(payload)
                            if payload
                                .set
                                .user
                                .as_ref()
                                .and_then(|users| users.get("alice"))
                                .and_then(|user| user.event.as_ref())
                                .and_then(|event| event.get("left"))
                                == Some(&json!(true))
                    )
                })
            }),
            "step 4 should notify active peers when stale client is disconnected"
        );

        let list_event = events.get(4).expect("step 5 list event should be present");
        let list_message = decode_message_line(
            &list_event
                .outbound_lines
                .first()
                .expect("step 5 should include list response")
                .line,
        )
        .expect("step 5 list output should decode");
        let ProtocolMessage::List(payload) = list_message else {
            panic!("step 5 output should be list response");
        };
        let ListPayload::Rooms(rooms) = payload.list else {
            panic!("list response should include room entries");
        };
        let room_users = rooms
            .get("room1")
            .expect("room1 should still exist for active user");
        assert!(
            room_users.contains_key("bob"),
            "active user should remain present after timeout handling"
        );
        assert!(
            !room_users.contains_key("alice"),
            "stale disconnected user should be removed from room list"
        );
    }

    #[test]
    fn scripted_server_runtime_username_conflict_scenario_resolves_names_legacy_style() {
        let events =
            replay_server_runtime_scenario_fixture("server_runtime_username_conflict.jsonl")
                .expect("username conflict scenario fixture should replay through server runtime");
        assert_eq!(events.len(), 4);

        let second_hello_event = events
            .get(1)
            .expect("step 2 second hello event should be present");
        let second_hello_response = second_hello_event
            .outbound_lines
            .iter()
            .filter(|line| line.client_id == "client-2")
            .find_map(|line| {
                decode_message_line(&line.line)
                    .ok()
                    .and_then(|message| extract_hello_from_message(message).ok())
            })
            .expect("step 2 should include hello response for client-2");
        assert_eq!(second_hello_response.username, "alice_");

        let third_hello_event = events
            .get(2)
            .expect("step 3 third hello event should be present");
        let third_hello_response = third_hello_event
            .outbound_lines
            .iter()
            .filter(|line| line.client_id == "client-3")
            .find_map(|line| {
                decode_message_line(&line.line)
                    .ok()
                    .and_then(|message| extract_hello_from_message(message).ok())
            })
            .expect("step 3 should include hello response for client-3");
        assert_eq!(third_hello_response.username, "alice__");

        let list_event = events.get(3).expect("step 4 list event should be present");
        let list_response = decode_message_line(
            &list_event
                .outbound_lines
                .first()
                .expect("step 4 should include one list response")
                .line,
        )
        .expect("step 4 list output should decode");
        match list_response {
            ProtocolMessage::List(payload) => match payload.list {
                ListPayload::Rooms(rooms) => {
                    let room = rooms.get("room1").expect("room1 should be listed");
                    assert!(room.contains_key("alice"));
                    assert!(room.contains_key("alice_"));
                    assert!(room.contains_key("alice__"));
                }
                other => panic!("expected list room snapshot at step 4, got {other:?}"),
            },
            other => panic!("expected list response at step 4, got {}", other.kind()),
        }
    }

    #[test]
    fn scripted_server_runtime_motd_template_scenario_applies_custom_template() {
        let steps = load_server_runtime_scenario_fixture(MOTD_TEMPLATE_SCENARIO)
            .expect("motd-template scenario fixture should load");
        let events = replay_server_runtime_scenario_steps_with_motd_template(
            &steps,
            Some(MOTD_TEMPLATE_RUNTIME_AND_PROBE),
        )
        .expect("motd-template scenario should replay through server runtime");
        assert_eq!(events.len(), 1);

        let hello_response = events[0]
            .outbound_lines
            .iter()
            .filter(|line| line.client_id == "client-1")
            .find_map(|line| {
                decode_message_line(&line.line)
                    .ok()
                    .and_then(|message| extract_hello_from_message(message).ok())
            })
            .expect("scenario should include hello response for client-1");
        let motd = hello_response
            .extra
            .get("motd")
            .and_then(Value::as_str)
            .expect("hello response should include motd");
        assert!(
            motd.starts_with("Compat MOTD latest="),
            "motd template output should include latest-version prefix"
        );
        assert!(
            !motd.contains("{latest_version}"),
            "motd template placeholder should be rendered"
        );
    }

    #[test]
    fn scripted_server_runtime_motd_template_outdated_client_scenario_prepends_upgrade_warning() {
        let steps = load_server_runtime_scenario_fixture(MOTD_TEMPLATE_OUTDATED_SCENARIO)
            .expect("motd-template outdated-client scenario fixture should load");
        let events = replay_server_runtime_scenario_steps_with_motd_template(
            &steps,
            Some(MOTD_TEMPLATE_RUNTIME_AND_PROBE),
        )
        .expect("motd-template outdated-client scenario should replay through server runtime");
        assert_eq!(events.len(), 1);

        let hello_response = events[0]
            .outbound_lines
            .iter()
            .filter(|line| line.client_id == "client-1")
            .find_map(|line| {
                decode_message_line(&line.line)
                    .ok()
                    .and_then(|message| extract_hello_from_message(message).ok())
            })
            .expect("scenario should include hello response for client-1");
        let motd = hello_response
            .extra
            .get("motd")
            .and_then(Value::as_str)
            .expect("hello response should include motd");
        assert_eq!(motd, MOTD_TEMPLATE_OUTDATED_EXPECTED);
    }

    #[test]
    fn scripted_server_runtime_persistent_rooms_notice_scenario_emits_notice_and_feature() {
        let steps = load_server_runtime_scenario_fixture(PERSISTENT_ROOMS_NOTICE_SCENARIO)
            .expect("persistent-rooms notice scenario fixture should load");
        let events = replay_server_runtime_scenario_steps_with_overrides(&steps, None, true)
            .expect("persistent-rooms notice scenario should replay through server runtime");
        assert_eq!(events.len(), 1);

        let hello_response = events[0]
            .outbound_lines
            .iter()
            .filter(|line| line.client_id == "client-1")
            .find_map(|line| {
                decode_message_line(&line.line)
                    .ok()
                    .and_then(|message| extract_hello_from_message(message).ok())
            })
            .expect("scenario should include hello response for client-1");
        let persistent_rooms = hello_response
            .features
            .as_ref()
            .and_then(Value::as_object)
            .and_then(|features| features.get("persistentRooms"))
            .and_then(Value::as_bool);
        assert_eq!(persistent_rooms, Some(true));
        let motd = hello_response
            .extra
            .get("motd")
            .and_then(Value::as_str)
            .expect("hello response should include motd");
        assert_eq!(motd, PERSISTENT_ROOMS_NOTICE);
    }

    #[test]
    fn scripted_server_runtime_persistent_rooms_lifecycle_scenario_replays_saved_room_state() {
        let steps = load_server_runtime_scenario_fixture(PERSISTENT_ROOMS_LIFECYCLE_SCENARIO)
            .expect("persistent-rooms lifecycle scenario fixture should load");
        let events = replay_server_runtime_scenario_steps_with_overrides(&steps, None, true)
            .expect("persistent-rooms lifecycle scenario should replay through server runtime");
        assert_eq!(events.len(), 7);

        let rejoin_event = events
            .get(5)
            .expect("step 6 should exist for rejoin snapshot assertions");
        let mut saw_playlist_snapshot = false;
        let mut saw_playlist_index_snapshot = false;
        for outbound in &rejoin_event.outbound_lines {
            if outbound.client_id != "client-2" {
                continue;
            }
            let message = decode_message_line(&outbound.line)
                .expect("rejoin event output should decode as protocol message");
            if let ProtocolMessage::Set(payload) = message {
                if payload
                    .set
                    .playlist_change
                    .as_ref()
                    .is_some_and(|playlist_change| {
                        playlist_change.files == vec!["episode1.mkv", "episode2.mkv"]
                            && playlist_change.user.as_deref() == Some("alice")
                    })
                {
                    saw_playlist_snapshot = true;
                }
                if payload
                    .set
                    .playlist_index
                    .as_ref()
                    .is_some_and(|playlist_index| {
                        playlist_index.index == 1 && playlist_index.user.as_deref() == Some("alice")
                    })
                {
                    saw_playlist_index_snapshot = true;
                }
            }
        }
        assert!(
            saw_playlist_snapshot,
            "rejoin should include persisted playlist snapshot"
        );
        assert!(
            saw_playlist_index_snapshot,
            "rejoin should include persisted playlist index snapshot"
        );

        let periodic_event = events
            .get(6)
            .expect("step 7 should exist for periodic-state assertion");
        let periodic_state = periodic_event
            .outbound_lines
            .iter()
            .find_map(|outbound| {
                if outbound.client_id != "client-2" {
                    return None;
                }
                decode_message_line(&outbound.line)
                    .ok()
                    .and_then(|message| {
                        if let ProtocolMessage::State(payload) = message {
                            payload.state.playstate
                        } else {
                            None
                        }
                    })
            })
            .expect("step 7 should include periodic state for rejoined client");
        assert_eq!(periodic_state.position, Some(42.0));
        assert_eq!(periodic_state.paused, Some(true));
    }

    #[test]
    fn scripted_server_runtime_permanent_rooms_file_scenario_retains_room_and_gui_dummy_entry() {
        let steps = load_server_runtime_scenario_fixture(PERMANENT_ROOMS_FILE_SCENARIO)
            .expect("permanent-rooms-file scenario fixture should load");
        let events = replay_server_runtime_scenario_steps_with_full_overrides(
            &steps,
            None,
            true,
            PERMANENT_ROOMS_FILE_LIST,
        )
        .expect("permanent-rooms-file scenario should replay through server runtime");
        assert_eq!(events.len(), 9);

        let list_event = events
            .get(7)
            .expect("step 8 should exist for GUI list assertions");
        let list_message = list_event
            .outbound_lines
            .iter()
            .find(|line| line.client_id == "client-2")
            .and_then(|line| decode_message_line(&line.line).ok())
            .expect("step 7 should include list response for GUI client");
        match list_message {
            ProtocolMessage::List(payload) => match payload.list {
                ListPayload::Rooms(rooms) => {
                    let dummy_room = rooms
                        .get("permanent-room")
                        .expect("GUI list should include empty permanent room");
                    let (dummy_username, dummy_entry) = dummy_room
                        .iter()
                        .next()
                        .expect("dummy entry should be present");
                    assert_eq!(dummy_username, " ");
                    assert_eq!(dummy_entry.features.as_ref(), Some(&json!([])));
                    assert_eq!(dummy_entry.is_ready, Some(true));
                }
                other => panic!("expected list room snapshot at step 8, got {other:?}"),
            },
            other => panic!("expected list response at step 8, got {}", other.kind()),
        }

        let rejoin_event = events
            .get(8)
            .expect("step 9 should exist for permanent-room snapshot assertions");
        let mut saw_playlist_snapshot = false;
        let mut saw_playlist_index_snapshot = false;
        for outbound in &rejoin_event.outbound_lines {
            if outbound.client_id != "client-3" {
                continue;
            }
            let message = decode_message_line(&outbound.line)
                .expect("step 8 output should decode as protocol message");
            if let ProtocolMessage::Set(payload) = message {
                if payload
                    .set
                    .playlist_change
                    .as_ref()
                    .is_some_and(|playlist_change| playlist_change.files.is_empty())
                {
                    saw_playlist_snapshot = true;
                }
                if payload
                    .set
                    .playlist_index
                    .as_ref()
                    .is_some_and(|playlist_index| {
                        playlist_index.index == 0 && playlist_index.user.as_deref() == Some("alice")
                    })
                {
                    saw_playlist_index_snapshot = true;
                }
            }
        }
        assert!(
            saw_playlist_snapshot,
            "rejoin should include empty playlist snapshot for permanent room"
        );
        assert!(
            saw_playlist_index_snapshot,
            "rejoin should include retained playlist index for permanent room"
        );
    }

    #[test]
    fn scripted_server_runtime_persistent_rooms_timeout_list_updates_scenario_is_ui_mode_scoped() {
        let steps =
            load_server_runtime_scenario_fixture(PERSISTENT_ROOMS_TIMEOUT_LIST_UPDATES_SCENARIO)
                .expect("persistent timeout-list-updates scenario fixture should load");
        let events = replay_server_runtime_scenario_steps_with_overrides(&steps, None, true)
            .expect(
                "persistent timeout-list-updates scenario should replay through server runtime",
            );
        assert_eq!(events.len(), 7);

        let timeout_event = events
            .get(5)
            .expect("step 6 should exist for timeout list-update assertions");
        let mut saw_timeout_left_for_bob = false;
        let mut list_to_client_1 = false;
        let mut list_to_client_3 = false;
        for outbound in &timeout_event.outbound_lines {
            let message = decode_message_line(&outbound.line)
                .expect("step 6 output should decode as protocol message");
            match message {
                ProtocolMessage::Set(payload) => {
                    if payload
                        .set
                        .user
                        .as_ref()
                        .and_then(|users| users.get("bob"))
                        .and_then(|user| user.event.as_ref())
                        .and_then(|event| event.get("left"))
                        .and_then(Value::as_bool)
                        == Some(true)
                    {
                        saw_timeout_left_for_bob = true;
                    }
                }
                ProtocolMessage::List(_) => {
                    if outbound.client_id == "client-1" {
                        list_to_client_1 = true;
                    }
                    if outbound.client_id == "client-3" {
                        list_to_client_3 = true;
                    }
                }
                _ => {}
            }
        }
        assert!(
            saw_timeout_left_for_bob,
            "step 6 should include timeout left event for bob"
        );
        assert!(
            list_to_client_1,
            "step 6 should include persistent list update for client that advertises uiMode"
        );
        assert!(
            !list_to_client_3,
            "step 6 should skip persistent list update for client that omits uiMode"
        );

        let final_list_event = events
            .get(6)
            .expect("step 7 should exist for post-timeout list assertions");
        let list_message = final_list_event
            .outbound_lines
            .iter()
            .find(|line| line.client_id == "client-1")
            .and_then(|line| decode_message_line(&line.line).ok())
            .expect("step 7 should include list response for client-1");
        match list_message {
            ProtocolMessage::List(payload) => match payload.list {
                ListPayload::Rooms(rooms) => {
                    let room = rooms.get("room1").expect("room1 should be listed");
                    assert!(room.contains_key("alice"));
                    assert!(room.contains_key("charlie"));
                    assert!(!room.contains_key("bob"));
                }
                other => panic!("expected list room snapshot at step 7, got {other:?}"),
            },
            other => panic!("expected list response at step 7, got {}", other.kind()),
        }
    }

    #[test]
    fn scripted_server_runtime_playlist_controller_scenario_replays_and_fanout_decodes() {
        let events =
            replay_server_runtime_scenario_fixture("server_runtime_playlist_controller.jsonl")
                .expect(
                    "playlist/controller scenario fixture should replay through server runtime",
                );
        assert_eq!(events.len(), 7);

        let mut saw_playlist_change_broadcast = false;
        let mut saw_playlist_index_broadcast = false;
        let mut saw_controller_auth_broadcast = false;
        let mut saw_new_controlled_room_ignored = false;
        let mut saw_list_snapshot_with_both_users = false;

        for (step_index, event) in events.iter().enumerate() {
            if step_index == 5 {
                assert!(
                    event.outbound_lines.is_empty(),
                    "newControlledRoom client input should currently be ignored by runtime"
                );
                saw_new_controlled_room_ignored = true;
            }

            for outbound in &event.outbound_lines {
                let message = decode_message_line(&outbound.line)
                    .expect("fanout output line should decode as protocol message");
                match message {
                    ProtocolMessage::Set(payload) => {
                        if let Some(playlist_change) = payload.set.playlist_change.as_ref() {
                            if playlist_change.user.as_deref() == Some("alice")
                                && playlist_change.files == vec!["episode1.mkv", "episode2.mkv"]
                            {
                                saw_playlist_change_broadcast = true;
                            }
                        }

                        if let Some(playlist_index) = payload.set.playlist_index.as_ref() {
                            if playlist_index.user.as_deref() == Some("bob")
                                && playlist_index.index == 1
                            {
                                saw_playlist_index_broadcast = true;
                            }
                        }

                        if let Some(controller_auth) = payload.set.controller_auth.as_ref() {
                            if controller_auth.user.as_deref() == Some("alice")
                                && controller_auth.room.as_deref() == Some("room1")
                                && controller_auth.success == Some(false)
                            {
                                saw_controller_auth_broadcast = true;
                            }
                        }
                    }
                    ProtocolMessage::List(payload) => {
                        if let ListPayload::Rooms(rooms) = payload.list {
                            let room = rooms.get("room1");
                            if room.is_some_and(|users| {
                                users.contains_key("alice") && users.contains_key("bob")
                            }) {
                                saw_list_snapshot_with_both_users = true;
                            }
                        }
                    }
                    ProtocolMessage::Hello(_)
                    | ProtocolMessage::State(_)
                    | ProtocolMessage::Chat(_)
                    | ProtocolMessage::Error(_)
                    | ProtocolMessage::Tls(_) => {}
                }
            }
        }

        assert!(
            saw_playlist_change_broadcast,
            "scenario should include playlistChange broadcast"
        );
        assert!(
            saw_playlist_index_broadcast,
            "scenario should include playlistIndex broadcast"
        );
        assert!(
            saw_controller_auth_broadcast,
            "scenario should include controllerAuth broadcast"
        );
        assert!(
            saw_new_controlled_room_ignored,
            "scenario should include ignored newControlledRoom client input"
        );
        assert!(
            saw_list_snapshot_with_both_users,
            "scenario should include list snapshot with both users in room1"
        );
    }

    #[test]
    fn scripted_server_runtime_cross_room_ready_list_scenario_validates_list_snapshots() {
        let events =
            replay_server_runtime_scenario_fixture("server_runtime_cross_room_ready_list.jsonl")
                .expect("cross-room list scenario fixture should replay through server runtime");
        assert_eq!(events.len(), 8);

        let pre_move_list_event = events
            .get(5)
            .expect("step 6 list request event should be present");
        assert_eq!(pre_move_list_event.client_id, "client-1");
        assert_eq!(pre_move_list_event.outbound_lines.len(), 1);
        assert_eq!(pre_move_list_event.outbound_lines[0].client_id, "client-1");
        let pre_move_list = decode_message_line(&pre_move_list_event.outbound_lines[0].line)
            .expect("step 6 list response should decode");
        match pre_move_list {
            ProtocolMessage::List(payload) => match payload.list {
                ListPayload::Rooms(rooms) => {
                    let room1 = rooms.get("room1").expect("room1 should be present");
                    let room2 = rooms.get("room2").expect("room2 should be present");
                    assert!(
                        room1
                            .get("alice")
                            .and_then(|entry| entry.is_ready)
                            .expect("alice should be in room1 with ready state")
                    );
                    assert_eq!(
                        room1.get("alice").and_then(|entry| entry.file.as_ref()),
                        Some(&json!({})),
                        "legacy list snapshots keep empty file objects for no-file users"
                    );
                    assert!(
                        room1
                            .get("carol")
                            .and_then(|entry| entry.is_ready)
                            .expect("carol should be in room1 with ready state")
                    );
                    assert_eq!(
                        room1.get("carol").and_then(|entry| entry.file.as_ref()),
                        Some(&json!({})),
                        "legacy list snapshots keep empty file objects for no-file users"
                    );
                    assert!(
                        !room2
                            .get("bob")
                            .and_then(|entry| entry.is_ready)
                            .expect("bob should be in room2 with ready state")
                    );
                    assert_eq!(
                        room2.get("bob").and_then(|entry| entry.file.as_ref()),
                        Some(&json!({})),
                        "legacy list snapshots keep empty file objects for no-file users"
                    );
                }
                other => panic!("expected list room snapshot at step 6, got {other:?}"),
            },
            other => panic!("expected list response at step 6, got {}", other.kind()),
        }

        let post_move_list_event = events
            .get(7)
            .expect("step 8 list request event should be present");
        assert_eq!(post_move_list_event.client_id, "client-3");
        assert_eq!(post_move_list_event.outbound_lines.len(), 1);
        assert_eq!(post_move_list_event.outbound_lines[0].client_id, "client-3");
        let post_move_list = decode_message_line(&post_move_list_event.outbound_lines[0].line)
            .expect("step 8 list response should decode");
        match post_move_list {
            ProtocolMessage::List(payload) => match payload.list {
                ListPayload::Rooms(rooms) => {
                    assert!(
                        !rooms.contains_key("room2"),
                        "room2 should be absent after bob moved to room1"
                    );
                    let room1 = rooms.get("room1").expect("room1 should be present");
                    assert!(
                        room1
                            .get("alice")
                            .and_then(|entry| entry.is_ready)
                            .expect("alice should be in room1 with ready state")
                    );
                    assert_eq!(
                        room1.get("alice").and_then(|entry| entry.file.as_ref()),
                        Some(&json!({})),
                        "legacy list snapshots keep empty file objects for no-file users"
                    );
                    assert!(
                        !room1
                            .get("bob")
                            .and_then(|entry| entry.is_ready)
                            .expect("bob should be in room1 with ready state")
                    );
                    assert_eq!(
                        room1.get("bob").and_then(|entry| entry.file.as_ref()),
                        Some(&json!({})),
                        "legacy list snapshots keep empty file objects for no-file users"
                    );
                    assert!(
                        room1
                            .get("carol")
                            .and_then(|entry| entry.is_ready)
                            .expect("carol should be in room1 with ready state")
                    );
                    assert_eq!(
                        room1.get("carol").and_then(|entry| entry.file.as_ref()),
                        Some(&json!({})),
                        "legacy list snapshots keep empty file objects for no-file users"
                    );
                }
                other => panic!("expected list room snapshot at step 8, got {other:?}"),
            },
            other => panic!("expected list response at step 8, got {}", other.kind()),
        }
    }

    #[test]
    fn scripted_server_runtime_controlled_room_permissions_scenario_validates_auth_and_playlist_corrections()
     {
        let events = replay_server_runtime_scenario_fixture(
            "server_runtime_controlled_room_permissions.jsonl",
        )
        .expect("controlled-room scenario fixture should replay through server runtime");
        assert_eq!(events.len(), 11);

        let create_room_auth_event = events
            .get(2)
            .expect("step 3 controllerAuth event should be present");
        assert_eq!(create_room_auth_event.outbound_lines.len(), 1);
        assert_eq!(
            create_room_auth_event.outbound_lines[0].client_id,
            "client-1"
        );
        let create_room_message =
            decode_message_line(&create_room_auth_event.outbound_lines[0].line)
                .expect("step 3 response should decode");
        match create_room_message {
            ProtocolMessage::Set(payload) => {
                let new_controlled_room = payload
                    .set
                    .new_controlled_room
                    .as_ref()
                    .expect("step 3 should include newControlledRoom payload");
                assert_eq!(
                    new_controlled_room.room_name.as_deref(),
                    Some("+room1:CB39A19549E8")
                );
                assert_eq!(new_controlled_room.password.as_deref(), Some("AB-123-456"));
            }
            other => panic!("expected set response at step 3, got {}", other.kind()),
        }

        let bob_playlist_attempt_event = events
            .get(5)
            .expect("step 6 bob playlist attempt event should be present");
        assert_eq!(bob_playlist_attempt_event.outbound_lines.len(), 1);
        assert!(
            bob_playlist_attempt_event
                .outbound_lines
                .iter()
                .all(|line| line.client_id == "client-2"),
            "non-controller correction should be directed only to sender"
        );
        let bob_correction_messages: Vec<_> = bob_playlist_attempt_event
            .outbound_lines
            .iter()
            .map(|line| decode_message_line(&line.line).expect("correction line should decode"))
            .collect();
        assert!(
            bob_correction_messages.iter().any(|message| match message {
                ProtocolMessage::Set(payload) =>
                    payload
                        .set
                        .playlist_change
                        .as_ref()
                        .is_some_and(|playlist| {
                            playlist.files.is_empty()
                                && playlist.user.as_deref() == Some("+room1:CB39A19549E8")
                        }),
                _ => false,
            }),
            "step 6 should include playlistChange correction for controlled room state"
        );
        let controller_auth_success_event = events
            .get(6)
            .expect("step 7 controllerAuth success event should be present");
        assert_eq!(controller_auth_success_event.outbound_lines.len(), 2);
        assert!(
            controller_auth_success_event
                .outbound_lines
                .iter()
                .any(|line| line.client_id == "client-1")
                && controller_auth_success_event
                    .outbound_lines
                    .iter()
                    .any(|line| line.client_id == "client-2"),
            "controller auth success should be broadcast to all clients"
        );
        for line in &controller_auth_success_event.outbound_lines {
            let message = decode_message_line(&line.line)
                .expect("step 7 controller auth response should decode");
            match message {
                ProtocolMessage::Set(payload) => {
                    let auth = payload
                        .set
                        .controller_auth
                        .as_ref()
                        .expect("step 7 response should include controllerAuth");
                    assert_eq!(auth.user.as_deref(), Some("alice"));
                    assert_eq!(auth.room.as_deref(), Some("+room1:CB39A19549E8"));
                    assert_eq!(auth.success, Some(true));
                }
                other => panic!("expected set response at step 7, got {}", other.kind()),
            }
        }

        let list_event = events
            .get(10)
            .expect("step 11 list event should be present");
        assert_eq!(list_event.outbound_lines.len(), 1);
        assert_eq!(list_event.outbound_lines[0].client_id, "client-2");
        let list_message = decode_message_line(&list_event.outbound_lines[0].line)
            .expect("step 11 list response should decode");
        match list_message {
            ProtocolMessage::List(payload) => match payload.list {
                ListPayload::Rooms(rooms) => {
                    let room = rooms
                        .get("+room1:CB39A19549E8")
                        .expect("controlled room should be present in list");
                    assert!(
                        room.get("alice")
                            .and_then(|entry| entry.controller)
                            .expect("alice should be listed in controlled room")
                    );
                    assert!(
                        !room
                            .get("bob")
                            .and_then(|entry| entry.controller)
                            .expect("bob should be listed in controlled room")
                    );
                }
                other => panic!("expected list room snapshot at step 11, got {other:?}"),
            },
            other => panic!("expected list response at step 11, got {}", other.kind()),
        }
    }

    #[test]
    fn scripted_server_runtime_controlled_room_invalid_password_scenario_validates_failures() {
        let events = replay_server_runtime_scenario_fixture(
            "server_runtime_controlled_room_invalid_password.jsonl",
        )
        .expect("controlled-room invalid-password scenario fixture should replay");
        assert_eq!(events.len(), 9);

        let invalid_plain_room_auth = events
            .get(2)
            .expect("step 3 invalid plain-room auth event should exist");
        assert_eq!(invalid_plain_room_auth.outbound_lines.len(), 2);
        for line in &invalid_plain_room_auth.outbound_lines {
            let message = decode_message_line(&line.line)
                .expect("step 3 invalid plain-room auth output should decode");
            match message {
                ProtocolMessage::Set(payload) => {
                    let auth = payload
                        .set
                        .controller_auth
                        .as_ref()
                        .expect("step 3 should include controllerAuth");
                    assert_eq!(auth.user.as_deref(), Some("alice"));
                    assert_eq!(auth.room.as_deref(), Some("room1"));
                    assert_eq!(auth.success, Some(false));
                }
                other => panic!("expected set response at step 3, got {}", other.kind()),
            }
        }

        let create_controlled_room = events
            .get(3)
            .expect("step 4 controlled-room creation response should exist");
        assert_eq!(create_controlled_room.outbound_lines.len(), 1);
        assert_eq!(
            create_controlled_room.outbound_lines[0].client_id,
            "client-1"
        );
        let create_controlled_room_message =
            decode_message_line(&create_controlled_room.outbound_lines[0].line)
                .expect("step 4 controlled-room creation output should decode");
        match create_controlled_room_message {
            ProtocolMessage::Set(payload) => {
                let new_room = payload
                    .set
                    .new_controlled_room
                    .as_ref()
                    .expect("step 4 should include newControlledRoom");
                assert_eq!(new_room.room_name.as_deref(), Some("+room1:CB39A19549E8"));
                assert_eq!(new_room.password.as_deref(), Some("AB-123-456"));
            }
            other => panic!("expected set response at step 4, got {}", other.kind()),
        }

        let invalid_controlled_room_auth = events
            .get(6)
            .expect("step 7 invalid controlled-room auth should exist");
        assert_eq!(invalid_controlled_room_auth.outbound_lines.len(), 2);
        for line in &invalid_controlled_room_auth.outbound_lines {
            let message = decode_message_line(&line.line)
                .expect("step 7 invalid controlled-room auth output should decode");
            match message {
                ProtocolMessage::Set(payload) => {
                    let auth = payload
                        .set
                        .controller_auth
                        .as_ref()
                        .expect("step 7 should include controllerAuth");
                    assert_eq!(auth.user.as_deref(), Some("bob"));
                    assert_eq!(auth.room.as_deref(), Some("+room1:CB39A19549E8"));
                    assert_eq!(auth.success, Some(false));
                }
                other => panic!("expected set response at step 7, got {}", other.kind()),
            }
        }

        let wrong_but_valid_format_password = events
            .get(7)
            .expect("step 8 wrong valid-format password auth should exist");
        assert_eq!(wrong_but_valid_format_password.outbound_lines.len(), 2);
        for line in &wrong_but_valid_format_password.outbound_lines {
            let message = decode_message_line(&line.line)
                .expect("step 8 wrong valid-format auth output should decode");
            match message {
                ProtocolMessage::Set(payload) => {
                    let auth = payload
                        .set
                        .controller_auth
                        .as_ref()
                        .expect("step 8 should include controllerAuth");
                    assert_eq!(auth.user.as_deref(), Some("bob"));
                    assert_eq!(auth.room.as_deref(), Some("+room1:CB39A19549E8"));
                    assert_eq!(auth.success, Some(false));
                }
                other => panic!("expected set response at step 8, got {}", other.kind()),
            }
        }

        let list_event = events.get(8).expect("step 9 list response should exist");
        assert_eq!(list_event.outbound_lines.len(), 1);
        assert_eq!(list_event.outbound_lines[0].client_id, "client-2");
        let list_message = decode_message_line(&list_event.outbound_lines[0].line)
            .expect("step 9 list response should decode");
        match list_message {
            ProtocolMessage::List(payload) => match payload.list {
                ListPayload::Rooms(rooms) => {
                    let room = rooms
                        .get("+room1:CB39A19549E8")
                        .expect("controlled room should be listed");
                    assert!(
                        !room
                            .get("alice")
                            .and_then(|entry| entry.controller)
                            .expect("alice should be listed")
                    );
                    assert!(
                        !room
                            .get("bob")
                            .and_then(|entry| entry.controller)
                            .expect("bob should be listed")
                    );
                }
                other => panic!("expected list room snapshot at step 9, got {other:?}"),
            },
            other => panic!("expected list response at step 9, got {}", other.kind()),
        }
    }

    #[test]
    fn scripted_server_runtime_controlled_room_state_forced_correction_scenario_validates_forced_pair()
     {
        let events = replay_server_runtime_scenario_fixture(
            "server_runtime_controlled_room_state_forced_correction.jsonl",
        )
        .expect("controlled-room forced-correction scenario should replay");
        assert_eq!(events.len(), 8);

        let forced_correction_event = events
            .get(7)
            .expect("step 8 non-controller state correction should exist");
        assert_eq!(forced_correction_event.client_id, "client-2");
        assert_eq!(forced_correction_event.outbound_lines.len(), 2);
        assert!(
            forced_correction_event
                .outbound_lines
                .iter()
                .all(|line| line.client_id == "client-2"),
            "forced correction should be directed only to non-controller sender"
        );

        let first_message = decode_message_line(&forced_correction_event.outbound_lines[0].line)
            .expect("first forced correction message should decode");
        match first_message {
            ProtocolMessage::State(payload) => {
                let playstate = payload
                    .state
                    .playstate
                    .as_ref()
                    .expect("first correction should include playstate");
                assert_eq!(playstate.position, Some(0.0));
                assert_eq!(playstate.paused, Some(false));
                assert_eq!(playstate.do_seek, Some(false));
                assert_eq!(playstate.set_by.as_deref(), Some("bob"));
                assert_eq!(
                    payload
                        .state
                        .ignoring_on_the_fly
                        .as_ref()
                        .and_then(|ignore| ignore.server),
                    Some(1),
                    "first correction should include server ignore counter 1"
                );
            }
            other => panic!(
                "expected state response at step 8 output 0, got {}",
                other.kind()
            ),
        }

        let second_message = decode_message_line(&forced_correction_event.outbound_lines[1].line)
            .expect("second forced correction message should decode");
        match second_message {
            ProtocolMessage::State(payload) => {
                let playstate = payload
                    .state
                    .playstate
                    .as_ref()
                    .expect("second correction should include playstate");
                assert_eq!(playstate.position, Some(0.0));
                assert_eq!(playstate.paused, Some(true));
                assert_eq!(playstate.do_seek, Some(true));
                assert_eq!(playstate.set_by, None);
                assert_eq!(
                    payload
                        .state
                        .ignoring_on_the_fly
                        .as_ref()
                        .and_then(|ignore| ignore.server),
                    Some(2),
                    "second correction should include server ignore counter 2"
                );
            }
            other => panic!(
                "expected state response at step 8 output 1, got {}",
                other.kind()
            ),
        }
    }

    #[test]
    fn server_runtime_fanout_matches_captured_python_trace_shape() {
        assert_runtime_matches_captured_trace("server_runtime_fanout.python_trace.json");
    }

    #[test]
    fn server_runtime_playlist_controller_matches_captured_python_trace_shape() {
        assert_runtime_matches_captured_trace(
            "server_runtime_playlist_controller.python_trace.json",
        );
    }

    #[test]
    fn server_runtime_controlled_room_permissions_matches_captured_python_trace_shape() {
        assert_runtime_matches_captured_trace(
            "server_runtime_controlled_room_permissions.python_trace.json",
        );
    }

    #[test]
    fn server_runtime_controlled_room_invalid_password_matches_captured_python_trace_shape() {
        assert_runtime_matches_captured_trace(
            "server_runtime_controlled_room_invalid_password.python_trace.json",
        );
    }

    #[test]
    fn server_runtime_controlled_room_state_forced_correction_matches_captured_python_trace_shape()
    {
        assert_runtime_matches_captured_trace(
            "server_runtime_controlled_room_state_forced_correction.python_trace.json",
        );
    }

    #[test]
    fn server_runtime_state_metadata_forwarding_matches_captured_python_trace_shape() {
        assert_runtime_matches_captured_trace(
            "server_runtime_state_metadata_forwarding.python_trace.json",
        );
    }

    #[test]
    fn server_runtime_state_propagation_matches_captured_python_trace_shape() {
        assert_runtime_matches_captured_trace("server_runtime_state_propagation.python_trace.json");
    }

    #[test]
    fn server_runtime_state_periodic_timeout_matches_captured_python_trace_shape() {
        assert_runtime_matches_captured_trace(
            "server_runtime_state_periodic_timeout.python_trace.json",
        );
    }

    #[test]
    fn server_runtime_state_latency_metrics_matches_captured_python_trace_shape() {
        assert_runtime_matches_captured_trace(
            "server_runtime_state_latency_metrics.python_trace.json",
        );
    }

    #[test]
    fn server_runtime_username_conflict_matches_captured_python_trace_shape() {
        assert_runtime_matches_captured_trace("server_runtime_username_conflict.python_trace.json");
    }

    #[test]
    fn server_runtime_motd_template_matches_captured_python_trace_shape() {
        assert_runtime_matches_captured_trace_with_motd_template(
            "server_runtime_motd_template.python_trace.json",
            Some(MOTD_TEMPLATE_RUNTIME_AND_PROBE),
        );
    }

    #[test]
    fn server_runtime_motd_template_outdated_client_matches_captured_python_trace_shape() {
        assert_runtime_matches_captured_trace_with_motd_template(
            "server_runtime_motd_template_outdated_client.python_trace.json",
            Some(MOTD_TEMPLATE_RUNTIME_AND_PROBE),
        );
    }

    #[test]
    fn server_runtime_persistent_rooms_notice_matches_captured_python_trace_shape() {
        assert_runtime_matches_captured_trace_with_overrides(
            "server_runtime_persistent_rooms_notice.python_trace.json",
            None,
            true,
        );
    }

    #[test]
    fn server_runtime_persistent_rooms_lifecycle_matches_captured_python_trace_shape() {
        assert_runtime_matches_captured_trace_with_overrides(
            "server_runtime_persistent_rooms_lifecycle.python_trace.json",
            None,
            true,
        );
    }

    #[test]
    fn server_runtime_permanent_rooms_file_matches_captured_python_trace_shape() {
        assert_runtime_matches_captured_trace_with_full_overrides(
            "server_runtime_permanent_rooms_file.python_trace.json",
            None,
            true,
            PERMANENT_ROOMS_FILE_LIST,
        );
    }

    #[test]
    fn server_runtime_persistent_rooms_timeout_list_updates_matches_captured_python_trace_shape() {
        assert_runtime_matches_captured_trace_with_overrides(
            "server_runtime_persistent_rooms_timeout_list_updates.python_trace.json",
            None,
            true,
        );
    }

    #[test]
    fn python_fanout_roundtrip_matches_server_runtime_on_fanout_scenario() {
        match assert_python_fanout_matches_server_runtime_for_scenario(
            "server_runtime_fanout.jsonl",
        ) {
            Ok(()) => {}
            Err(InteropError::LegacySyncplayCheckoutMissing(_))
            | Err(InteropError::PythonSpawn { .. }) => {
                eprintln!("python fanout interop test skipped due to missing local prerequisites");
            }
            Err(err) => panic!("python fanout interop should succeed, got: {err}"),
        }
    }

    #[test]
    fn python_fanout_roundtrip_matches_server_runtime_on_tls_send_available_scenario() {
        let cert_path = temporary_tls_directory_path("tls-fanout-available");
        let _ = fs::remove_dir_all(&cert_path);
        fs::create_dir_all(&cert_path).expect("tls cert temp directory should be creatable");
        write_valid_tls_bundle(&cert_path);

        let steps = vec![ServerRuntimeScenarioStep {
            client_id: "client-1".to_owned(),
            request_line: r#"{"TLS":{"startTLS":"send"}}"#.to_owned(),
            advance_seconds: 0.0,
        }];

        let rust_events = {
            let mut runtime = ServerRuntime::new();
            runtime.set_tls_cert_path(Some(cert_path.clone()));
            runtime.set_time_now_override_seconds(Some(0.0));

            let mut events = Vec::new();
            for step in &steps {
                let mut outbound_lines = runtime
                    .advance_time_and_collect_fanout(step.advance_seconds)
                    .expect("runtime fanout tick should encode");
                outbound_lines.extend(
                    runtime
                        .handle_line_fanout(&step.client_id, &step.request_line)
                        .expect("runtime step should succeed"),
                );
                events.push(super::ServerRuntimeScenarioEvent {
                    client_id: step.client_id.clone(),
                    request_line: step.request_line.clone(),
                    outbound_lines,
                });
            }
            events
        };

        let python_events = match run_python_fanout_roundtrip_with_tls_available(&steps, true) {
            Ok(events) => events,
            Err(InteropError::LegacySyncplayCheckoutMissing(_))
            | Err(InteropError::PythonSpawn { .. }) => {
                let _ = fs::remove_dir_all(&cert_path);
                eprintln!("python fanout interop test skipped due to missing local prerequisites");
                return;
            }
            Err(err) => panic!("python tls fanout interop should succeed, got: {err}"),
        };

        assert_eq!(python_events.len(), rust_events.len());
        for (python_event, rust_event) in python_events.iter().zip(rust_events.iter()) {
            assert_eq!(python_event.client_id, rust_event.client_id);
            assert_eq!(python_event.request_line, rust_event.request_line);
            assert_eq!(
                python_event.outbound_lines.len(),
                rust_event.outbound_lines.len()
            );
            for (python_output, rust_output) in python_event
                .outbound_lines
                .iter()
                .zip(rust_event.outbound_lines.iter())
            {
                assert_eq!(python_output.client_id, rust_output.client_id);
                let python_value = normalize_cross_impl_message(
                    serde_json::from_str::<Value>(&python_output.line)
                        .expect("python output line should decode"),
                );
                let rust_value = normalize_cross_impl_message(
                    serde_json::from_str::<Value>(&rust_output.line)
                        .expect("runtime output line should decode"),
                );
                assert_eq!(python_value, rust_value);
            }
        }

        fs::remove_dir_all(&cert_path).expect("tls cert temp directory should be removable");
    }

    #[test]
    fn legacy_server_live_tls_upgrade_roundtrip_supports_post_upgrade_hello_over_same_socket() {
        if !legacy_server_parity_assertions_enabled() {
            eprintln!(
                "legacy server TLS parity assertion skipped; set SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
            );
            return;
        }

        let tls_cert_path = legacy_tls_fixture_directory();
        match run_legacy_server_tls_upgrade_roundtrip_with_cert_path(&tls_cert_path) {
            Ok((tls_response_line, hello_response_line)) => {
                let tls_message = decode_message_line(&tls_response_line)
                    .expect("legacy TLS response should decode");
                match tls_message {
                    ProtocolMessage::Tls(payload) => {
                        assert_eq!(payload.tls.start_tls, "true");
                    }
                    other => panic!(
                        "expected legacy TLS response before upgrade, got {}",
                        other.kind()
                    ),
                }

                let hello_message = decode_message_line(&hello_response_line)
                    .expect("post-upgrade legacy hello response should decode");
                let hello = extract_hello_from_message(hello_message)
                    .expect("post-upgrade legacy response should be hello");
                assert_eq!(hello.username, "interop-client");
                assert_eq!(hello.room.name, "interop-room");
            }
            Err(err) if legacy_server_tls_prerequisites_missing(&err) => {
                if legacy_tls_parity_prerequisites_strict_enabled() {
                    panic!(
                        "legacy live TLS roundtrip prerequisites should be satisfied in strict mode, got: {err}"
                    );
                }
                eprintln!(
                    "legacy live TLS roundtrip test skipped due to missing prerequisites: {err}"
                );
            }
            Err(err) => {
                panic!("legacy live TLS roundtrip should succeed over upgraded socket, got: {err}")
            }
        }
    }

    #[test]
    fn legacy_server_live_tls_send_is_denied_for_logged_client() {
        if !legacy_server_parity_assertions_enabled() {
            eprintln!(
                "legacy server TLS parity assertion skipped; set SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
            );
            return;
        }

        let tls_cert_path = legacy_tls_fixture_directory();
        match run_legacy_server_tls_logged_client_send_denied_roundtrip_with_cert_path(
            &tls_cert_path,
        ) {
            Ok(tls_response_line) => {
                let tls_message = decode_message_line(&tls_response_line)
                    .expect("legacy logged tls response should decode");
                match tls_message {
                    ProtocolMessage::Tls(payload) => {
                        assert_eq!(payload.tls.start_tls, "false");
                    }
                    other => panic!(
                        "expected legacy TLS response for logged client probe, got {}",
                        other.kind()
                    ),
                }
            }
            Err(err) if legacy_server_tls_prerequisites_missing(&err) => {
                if legacy_tls_parity_prerequisites_strict_enabled() {
                    panic!(
                        "legacy logged-client TLS denial prerequisites should be satisfied in strict mode, got: {err}"
                    );
                }
                eprintln!(
                    "legacy logged-client TLS denial test skipped due to missing prerequisites: {err}"
                );
            }
            Err(err) => panic!(
                "legacy logged-client TLS denial behavior should succeed with startTLS=false, got: {err}"
            ),
        }
    }

    #[test]
    fn python_fanout_roundtrip_matches_server_runtime_on_playlist_controller_scenario() {
        match assert_python_fanout_matches_server_runtime_for_scenario(
            "server_runtime_playlist_controller.jsonl",
        ) {
            Ok(()) => {}
            Err(InteropError::LegacySyncplayCheckoutMissing(_))
            | Err(InteropError::PythonSpawn { .. }) => {
                eprintln!("python fanout interop test skipped due to missing local prerequisites");
            }
            Err(err) => panic!(
                "python fanout interop for playlist/controller scenario should succeed, got: {err}"
            ),
        }
    }

    #[test]
    fn python_fanout_roundtrip_matches_server_runtime_on_cross_room_ready_list_scenario() {
        match assert_python_fanout_matches_server_runtime_for_scenario(
            "server_runtime_cross_room_ready_list.jsonl",
        ) {
            Ok(()) => {}
            Err(InteropError::LegacySyncplayCheckoutMissing(_))
            | Err(InteropError::PythonSpawn { .. }) => {
                eprintln!("python fanout interop test skipped due to missing local prerequisites");
            }
            Err(err) => panic!(
                "python fanout interop for cross-room ready/list scenario should succeed, got: {err}"
            ),
        }
    }

    #[test]
    fn python_fanout_roundtrip_matches_server_runtime_on_controlled_room_permissions_scenario() {
        match assert_python_fanout_matches_server_runtime_for_scenario(
            "server_runtime_controlled_room_permissions.jsonl",
        ) {
            Ok(()) => {}
            Err(InteropError::LegacySyncplayCheckoutMissing(_))
            | Err(InteropError::PythonSpawn { .. }) => {
                eprintln!("python fanout interop test skipped due to missing local prerequisites");
            }
            Err(err) => panic!(
                "python fanout interop for controlled-room permissions scenario should succeed, got: {err}"
            ),
        }
    }

    #[test]
    fn python_fanout_roundtrip_matches_server_runtime_on_controlled_room_invalid_password_scenario()
    {
        match assert_python_fanout_matches_server_runtime_for_scenario(
            "server_runtime_controlled_room_invalid_password.jsonl",
        ) {
            Ok(()) => {}
            Err(InteropError::LegacySyncplayCheckoutMissing(_))
            | Err(InteropError::PythonSpawn { .. }) => {
                eprintln!("python fanout interop test skipped due to missing local prerequisites");
            }
            Err(err) => panic!(
                "python fanout interop for controlled-room invalid-password scenario should succeed, got: {err}"
            ),
        }
    }

    #[test]
    fn python_fanout_roundtrip_matches_server_runtime_on_controlled_room_state_forced_correction_scenario()
     {
        match assert_python_fanout_matches_server_runtime_for_scenario(
            "server_runtime_controlled_room_state_forced_correction.jsonl",
        ) {
            Ok(()) => {}
            Err(InteropError::LegacySyncplayCheckoutMissing(_))
            | Err(InteropError::PythonSpawn { .. }) => {
                eprintln!("python fanout interop test skipped due to missing local prerequisites");
            }
            Err(err) => panic!(
                "python fanout interop for controlled-room forced-correction scenario should succeed, got: {err}"
            ),
        }
    }

    #[test]
    fn python_fanout_roundtrip_matches_server_runtime_on_state_propagation_scenario() {
        match assert_python_fanout_matches_server_runtime_for_scenario(
            "server_runtime_state_propagation.jsonl",
        ) {
            Ok(()) => {}
            Err(InteropError::LegacySyncplayCheckoutMissing(_))
            | Err(InteropError::PythonSpawn { .. }) => {
                eprintln!("python fanout interop test skipped due to missing local prerequisites");
            }
            Err(err) => panic!(
                "python fanout interop for state propagation scenario should succeed, got: {err}"
            ),
        }
    }

    #[test]
    fn python_fanout_roundtrip_matches_server_runtime_on_state_metadata_forwarding_scenario() {
        match assert_python_fanout_matches_server_runtime_for_scenario(
            "server_runtime_state_metadata_forwarding.jsonl",
        ) {
            Ok(()) => {}
            Err(InteropError::LegacySyncplayCheckoutMissing(_))
            | Err(InteropError::PythonSpawn { .. }) => {
                eprintln!("python fanout interop test skipped due to missing local prerequisites");
            }
            Err(err) => panic!(
                "python fanout interop for state metadata forwarding scenario should succeed, got: {err}"
            ),
        }
    }

    #[test]
    fn python_fanout_roundtrip_matches_server_runtime_on_state_periodic_timeout_scenario() {
        match assert_python_fanout_matches_server_runtime_for_scenario(
            "server_runtime_state_periodic_timeout.jsonl",
        ) {
            Ok(()) => {}
            Err(InteropError::LegacySyncplayCheckoutMissing(_))
            | Err(InteropError::PythonSpawn { .. }) => {
                eprintln!("python fanout interop test skipped due to missing local prerequisites");
            }
            Err(err) => panic!(
                "python fanout interop for state periodic-timeout scenario should succeed, got: {err}"
            ),
        }
    }

    #[test]
    fn python_fanout_roundtrip_matches_server_runtime_on_state_latency_metrics_scenario() {
        match assert_python_fanout_matches_server_runtime_for_scenario(
            "server_runtime_state_latency_metrics.jsonl",
        ) {
            Ok(()) => {}
            Err(InteropError::LegacySyncplayCheckoutMissing(_))
            | Err(InteropError::PythonSpawn { .. }) => {
                eprintln!("python fanout interop test skipped due to missing local prerequisites");
            }
            Err(err) => panic!(
                "python fanout interop for state latency-metrics scenario should succeed, got: {err}"
            ),
        }
    }

    #[test]
    fn python_fanout_roundtrip_matches_server_runtime_on_username_conflict_scenario() {
        match assert_python_fanout_matches_server_runtime_for_scenario(
            "server_runtime_username_conflict.jsonl",
        ) {
            Ok(()) => {}
            Err(InteropError::LegacySyncplayCheckoutMissing(_))
            | Err(InteropError::PythonSpawn { .. }) => {
                eprintln!("python fanout interop test skipped due to missing local prerequisites");
            }
            Err(err) => panic!(
                "python fanout interop for username conflict scenario should succeed, got: {err}"
            ),
        }
    }

    #[test]
    fn python_fanout_roundtrip_matches_server_runtime_on_motd_template_scenario() {
        match assert_python_fanout_matches_server_runtime_for_scenario_with_motd_template(
            MOTD_TEMPLATE_SCENARIO,
            Some(MOTD_TEMPLATE_RUNTIME_AND_PROBE),
            Some(MOTD_TEMPLATE_RUNTIME_AND_PROBE),
        ) {
            Ok(()) => {}
            Err(InteropError::LegacySyncplayCheckoutMissing(_))
            | Err(InteropError::PythonSpawn { .. }) => {
                eprintln!("python fanout interop test skipped due to missing local prerequisites");
            }
            Err(err) => panic!(
                "python fanout interop for motd-template scenario should succeed, got: {err}"
            ),
        }
    }

    #[test]
    fn python_fanout_roundtrip_matches_server_runtime_on_motd_template_outdated_client_scenario() {
        match assert_python_fanout_matches_server_runtime_for_scenario_with_motd_template(
            MOTD_TEMPLATE_OUTDATED_SCENARIO,
            Some(MOTD_TEMPLATE_RUNTIME_AND_PROBE),
            Some(MOTD_TEMPLATE_RUNTIME_AND_PROBE),
        ) {
            Ok(()) => {}
            Err(InteropError::LegacySyncplayCheckoutMissing(_))
            | Err(InteropError::PythonSpawn { .. }) => {
                eprintln!("python fanout interop test skipped due to missing local prerequisites");
            }
            Err(err) => panic!(
                "python fanout interop for motd-template outdated-client scenario should succeed, got: {err}"
            ),
        }
    }

    #[test]
    fn python_fanout_roundtrip_matches_server_runtime_on_persistent_rooms_notice_scenario() {
        match assert_python_fanout_matches_server_runtime_for_scenario_with_overrides(
            PERSISTENT_ROOMS_NOTICE_SCENARIO,
            None,
            None,
            true,
            true,
        ) {
            Ok(()) => {}
            Err(InteropError::LegacySyncplayCheckoutMissing(_))
            | Err(InteropError::PythonSpawn { .. }) => {
                eprintln!("python fanout interop test skipped due to missing local prerequisites");
            }
            Err(err) => panic!(
                "python fanout interop for persistent-rooms notice scenario should succeed, got: {err}"
            ),
        }
    }

    #[test]
    fn python_fanout_roundtrip_matches_server_runtime_on_persistent_rooms_lifecycle_scenario() {
        match assert_python_fanout_matches_server_runtime_for_scenario_with_overrides(
            PERSISTENT_ROOMS_LIFECYCLE_SCENARIO,
            None,
            None,
            true,
            true,
        ) {
            Ok(()) => {}
            Err(InteropError::LegacySyncplayCheckoutMissing(_))
            | Err(InteropError::PythonSpawn { .. }) => {
                eprintln!("python fanout interop test skipped due to missing local prerequisites");
            }
            Err(err) => panic!(
                "python fanout interop for persistent-rooms lifecycle scenario should succeed, got: {err}"
            ),
        }
    }

    #[test]
    fn python_fanout_roundtrip_matches_server_runtime_on_permanent_rooms_file_scenario() {
        match assert_python_fanout_matches_server_runtime_for_scenario_with_full_overrides(
            PERMANENT_ROOMS_FILE_SCENARIO,
            None,
            None,
            true,
            true,
            PERMANENT_ROOMS_FILE_LIST,
            PERMANENT_ROOMS_FILE_LIST,
        ) {
            Ok(()) => {}
            Err(InteropError::LegacySyncplayCheckoutMissing(_))
            | Err(InteropError::PythonSpawn { .. }) => {
                eprintln!("python fanout interop test skipped due to missing local prerequisites");
            }
            Err(err) => panic!(
                "python fanout interop for permanent-rooms-file scenario should succeed, got: {err}"
            ),
        }
    }

    #[test]
    fn python_fanout_roundtrip_matches_server_runtime_on_persistent_rooms_timeout_list_updates_scenario()
     {
        match assert_python_fanout_matches_server_runtime_for_scenario_with_overrides(
            PERSISTENT_ROOMS_TIMEOUT_LIST_UPDATES_SCENARIO,
            None,
            None,
            true,
            true,
        ) {
            Ok(()) => {}
            Err(InteropError::LegacySyncplayCheckoutMissing(_))
            | Err(InteropError::PythonSpawn { .. }) => {
                eprintln!("python fanout interop test skipped due to missing local prerequisites");
            }
            Err(err) => panic!(
                "python fanout interop for persistent timeout-list-updates scenario should succeed, got: {err}"
            ),
        }
    }

    #[test]
    fn legacy_server_state_propagation_matches_runtime_core_behavior() {
        let steps = load_server_runtime_scenario_fixture("server_runtime_state_propagation.jsonl")
            .expect("state propagation scenario fixture should load");
        let rust_events = replay_server_runtime_scenario_steps(&steps)
            .expect("state propagation scenario should replay through server runtime");
        let legacy_events = match run_legacy_server_fanout_roundtrip(&steps) {
            Ok(events) => events,
            Err(err) if legacy_server_prerequisites_missing(&err) => {
                eprintln!(
                    "legacy state propagation test skipped due to missing prerequisites: {err}"
                );
                return;
            }
            Err(err) => panic!(
                "legacy state propagation roundtrip should succeed for probe scenario, got: {err}"
            ),
        };

        let rust_state_event = rust_events
            .get(2)
            .expect("step 3 state event should exist for runtime replay");
        let legacy_state_event = legacy_events
            .get(2)
            .expect("step 3 state event should exist for legacy replay");

        let mut rust_state_summaries: Vec<(String, String, bool, bool, f64)> = rust_state_event
            .outbound_lines
            .iter()
            .filter_map(|outbound| {
                let message = decode_message_line(&outbound.line).ok()?;
                if is_background_idle_state_message(&message) {
                    return None;
                }
                let ProtocolMessage::State(payload) = message else {
                    return None;
                };
                let playstate = payload.state.playstate?;
                let ping = payload.state.ping?;
                assert!(
                    ping.latency_calculation.is_some(),
                    "runtime state update should include latencyCalculation"
                );
                assert_eq!(
                    ping.server_rtt,
                    Some(0.0),
                    "runtime state update should include serverRtt=0"
                );
                Some((
                    outbound.client_id.clone(),
                    playstate.set_by.unwrap_or_default(),
                    playstate.paused.unwrap_or_default(),
                    playstate.do_seek.unwrap_or_default(),
                    playstate.position.unwrap_or_default(),
                ))
            })
            .collect();
        rust_state_summaries.sort_by(|left, right| left.0.cmp(&right.0));

        let mut legacy_state_summaries: Vec<(String, String, bool, bool, f64)> = legacy_state_event
            .outbound_lines
            .iter()
            .filter_map(|outbound| {
                let message = decode_message_line(&outbound.line).ok()?;
                if is_background_idle_state_message(&message) {
                    return None;
                }
                let ProtocolMessage::State(payload) = message else {
                    return None;
                };
                let playstate = payload.state.playstate?;
                let ping = payload.state.ping?;
                assert!(
                    ping.latency_calculation.is_some(),
                    "legacy state update should include latencyCalculation"
                );
                assert_eq!(
                    ping.server_rtt,
                    Some(0.0),
                    "legacy state update should include serverRtt=0"
                );
                Some((
                    outbound.client_id.clone(),
                    playstate.set_by.unwrap_or_default(),
                    playstate.paused.unwrap_or_default(),
                    playstate.do_seek.unwrap_or_default(),
                    playstate.position.unwrap_or_default(),
                ))
            })
            .collect();
        legacy_state_summaries.sort_by(|left, right| left.0.cmp(&right.0));

        assert_eq!(
            rust_state_summaries.len(),
            2,
            "runtime step 3 should broadcast state to sender and room peer"
        );
        assert_eq!(
            legacy_state_summaries.len(),
            2,
            "legacy step 3 should broadcast state to sender and room peer"
        );

        let expected_recipients = vec!["client-1".to_owned(), "client-2".to_owned()];
        assert_eq!(
            rust_state_summaries
                .iter()
                .map(|summary| summary.0.clone())
                .collect::<Vec<_>>(),
            expected_recipients
        );
        assert_eq!(
            legacy_state_summaries
                .iter()
                .map(|summary| summary.0.clone())
                .collect::<Vec<_>>(),
            expected_recipients
        );
        for (_, set_by, paused, do_seek, position) in rust_state_summaries {
            assert_eq!(set_by, "alice");
            assert!(!paused);
            assert!(!do_seek);
            assert_eq!(position, 12.5);
        }
        for (_, set_by, paused, do_seek, position) in legacy_state_summaries {
            assert_eq!(set_by, "alice");
            assert!(!paused);
            assert!(!do_seek);
            assert!(
                (position - 12.5).abs() <= 0.01,
                "legacy playstate position should stay near requested position"
            );
        }

        let rust_unchanged_state_event = rust_events
            .get(3)
            .expect("step 4 unchanged-playstate event should exist for runtime replay");
        let legacy_unchanged_state_event = legacy_events
            .get(3)
            .expect("step 4 unchanged-playstate event should exist for legacy replay");
        assert!(
            rust_unchanged_state_event.outbound_lines.is_empty(),
            "runtime unchanged playstate update should produce no immediate outbound lines"
        );
        assert!(
            legacy_unchanged_state_event.outbound_lines.is_empty(),
            "legacy unchanged playstate update should produce no immediate outbound lines"
        );

        let rust_ping_only_event = rust_events
            .get(4)
            .expect("step 5 ping-only event should exist for runtime replay");
        let legacy_ping_only_event = legacy_events
            .get(4)
            .expect("step 5 ping-only event should exist for legacy replay");
        assert!(
            rust_ping_only_event.outbound_lines.is_empty(),
            "runtime ping-only state update should produce no immediate outbound lines"
        );
        assert!(
            legacy_ping_only_event.outbound_lines.is_empty(),
            "legacy ping-only state update should produce no immediate outbound lines"
        );
    }

    #[test]
    fn legacy_server_state_latency_metrics_matches_runtime_core_behavior() {
        let steps =
            load_server_runtime_scenario_fixture("server_runtime_state_latency_metrics.jsonl")
                .expect("state latency-metrics scenario fixture should load");
        let rust_events = replay_server_runtime_scenario_steps(&steps)
            .expect("state latency-metrics scenario should replay through server runtime");
        let legacy_events = match run_legacy_server_fanout_roundtrip(&steps) {
            Ok(events) => events,
            Err(err) if legacy_server_prerequisites_missing(&err) => {
                eprintln!(
                    "legacy state latency-metrics test skipped due to missing prerequisites: {err}"
                );
                return;
            }
            Err(err) => panic!(
                "legacy state latency-metrics roundtrip should succeed for probe scenario, got: {err}"
            ),
        };

        let rust_state_event = rust_events
            .get(2)
            .expect("step 3 state event should exist for runtime replay");
        let legacy_state_event = legacy_events
            .get(2)
            .expect("step 3 state event should exist for legacy replay");

        let parse_summary = |line: &str| -> Option<(String, bool, bool, f64, f64)> {
            let message = decode_message_line(line).ok()?;
            if is_background_idle_state_message(&message) {
                return None;
            }
            let ProtocolMessage::State(payload) = message else {
                return None;
            };
            let playstate = payload.state.playstate?;
            let ping = payload.state.ping?;
            Some((
                playstate.set_by.unwrap_or_default(),
                playstate.paused.unwrap_or_default(),
                playstate.do_seek.unwrap_or_default(),
                playstate.position.unwrap_or_default(),
                ping.server_rtt.unwrap_or_default(),
            ))
        };

        let rust_sender = rust_state_event
            .outbound_lines
            .iter()
            .find_map(|outbound| {
                if outbound.client_id != "client-1" {
                    return None;
                }
                parse_summary(&outbound.line)
            })
            .expect("runtime replay should include sender-directed state output");
        let rust_peer = rust_state_event
            .outbound_lines
            .iter()
            .find_map(|outbound| {
                if outbound.client_id != "client-2" {
                    return None;
                }
                parse_summary(&outbound.line)
            })
            .expect("runtime replay should include peer-directed state output");

        let legacy_sender = legacy_state_event
            .outbound_lines
            .iter()
            .find_map(|outbound| {
                if outbound.client_id != "client-1" {
                    return None;
                }
                parse_summary(&outbound.line)
            })
            .expect("legacy replay should include sender-directed state output");
        let legacy_peer = legacy_state_event
            .outbound_lines
            .iter()
            .find_map(|outbound| {
                if outbound.client_id != "client-2" {
                    return None;
                }
                parse_summary(&outbound.line)
            })
            .expect("legacy replay should include peer-directed state output");

        for (set_by, paused, do_seek, position, server_rtt) in [&rust_sender, &rust_peer] {
            assert_eq!(set_by, "alice");
            assert!(!paused);
            assert!(*do_seek);
            assert!(
                (*position - 18.0).abs() <= 0.000_001,
                "runtime should apply forward delay to shared position"
            );
            assert!(
                *server_rtt >= 0.0,
                "runtime state updates should include non-negative serverRtt"
            );
        }
        assert!(
            (rust_sender.4 - 10.0).abs() <= 0.000_001,
            "runtime sender-directed state should include derived non-zero serverRtt"
        );
        assert_eq!(
            rust_peer.4, 0.0,
            "runtime peer-directed state should include default serverRtt"
        );

        for (set_by, paused, do_seek, _position, server_rtt) in [&legacy_sender, &legacy_peer] {
            assert_eq!(set_by, "alice");
            assert!(!paused);
            assert!(*do_seek);
            assert!(
                *server_rtt >= 0.0,
                "legacy state updates should include non-negative serverRtt"
            );
        }
        assert!(
            legacy_sender.3 > 18.0 && legacy_peer.3 > 18.0,
            "legacy forward-delay position should remain positive and above base position"
        );
        assert!(
            (legacy_sender.3 - legacy_peer.3).abs() <= 0.01,
            "legacy sender and peer should receive equivalent forwarded positions"
        );
        assert!(
            legacy_sender.4 >= legacy_peer.4,
            "legacy sender-directed serverRtt should not be lower than peer default"
        );
    }

    #[test]
    fn legacy_server_fanout_roundtrip_matches_server_runtime_on_username_conflict_scenario() {
        if !legacy_server_parity_assertions_enabled() {
            eprintln!(
                "legacy server parity assertion skipped; set SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
            );
            return;
        }
        match assert_legacy_server_fanout_matches_server_runtime_for_scenario(
            "server_runtime_username_conflict.jsonl",
        ) {
            Ok(()) => {}
            Err(err) if legacy_server_prerequisites_missing(&err) => {
                eprintln!(
                    "legacy server fanout interop test skipped due to missing prerequisites: {err}"
                );
            }
            Err(err) => panic!(
                "legacy server fanout interop for username conflict scenario should succeed, got: {err}"
            ),
        }
    }

    #[test]
    fn legacy_server_fanout_roundtrip_matches_server_runtime_on_motd_template_scenario() {
        if !legacy_server_parity_assertions_enabled() {
            eprintln!(
                "legacy server parity assertion skipped; set SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
            );
            return;
        }
        match assert_legacy_server_fanout_matches_server_runtime_for_scenario_with_motd_template(
            MOTD_TEMPLATE_SCENARIO,
            Some(MOTD_TEMPLATE_RUNTIME_AND_PROBE),
            Some(MOTD_TEMPLATE_LEGACY_FILE),
        ) {
            Ok(()) => {}
            Err(err) if legacy_server_prerequisites_missing(&err) => {
                eprintln!(
                    "legacy server fanout interop test skipped due to missing prerequisites: {err}"
                );
            }
            Err(err) => panic!(
                "legacy server fanout interop for motd-template scenario should succeed, got: {err}"
            ),
        }
    }

    #[test]
    fn legacy_server_fanout_roundtrip_matches_server_runtime_on_motd_template_outdated_client_scenario()
     {
        if !legacy_server_parity_assertions_enabled() {
            eprintln!(
                "legacy server parity assertion skipped; set SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
            );
            return;
        }
        match assert_legacy_server_fanout_matches_server_runtime_for_scenario_with_motd_template(
            MOTD_TEMPLATE_OUTDATED_SCENARIO,
            Some(MOTD_TEMPLATE_RUNTIME_AND_PROBE),
            Some(MOTD_TEMPLATE_LEGACY_FILE),
        ) {
            Ok(()) => {}
            Err(err) if legacy_server_prerequisites_missing(&err) => {
                eprintln!(
                    "legacy server fanout interop test skipped due to missing prerequisites: {err}"
                );
            }
            Err(err) => panic!(
                "legacy server fanout interop for motd-template outdated-client scenario should succeed, got: {err}"
            ),
        }
    }

    #[test]
    fn legacy_server_fanout_roundtrip_matches_server_runtime_on_persistent_rooms_notice_scenario() {
        if !legacy_server_parity_assertions_enabled() {
            eprintln!(
                "legacy server parity assertion skipped; set SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
            );
            return;
        }
        match assert_legacy_server_fanout_matches_server_runtime_for_scenario_with_overrides(
            PERSISTENT_ROOMS_NOTICE_SCENARIO,
            None,
            None,
            true,
            true,
        ) {
            Ok(()) => {}
            Err(err) if legacy_server_prerequisites_missing(&err) => {
                eprintln!(
                    "legacy server fanout interop test skipped due to missing prerequisites: {err}"
                );
            }
            Err(err) => panic!(
                "legacy server fanout interop for persistent-rooms notice scenario should succeed, got: {err}"
            ),
        }
    }

    #[test]
    fn legacy_server_fanout_roundtrip_matches_server_runtime_on_persistent_rooms_lifecycle_scenario()
     {
        if !legacy_server_parity_assertions_enabled() {
            eprintln!(
                "legacy server parity assertion skipped; set SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
            );
            return;
        }
        match assert_legacy_server_fanout_matches_server_runtime_for_scenario_with_overrides(
            PERSISTENT_ROOMS_LIFECYCLE_SCENARIO,
            None,
            None,
            true,
            true,
        ) {
            Ok(()) => {}
            Err(err) if legacy_server_prerequisites_missing(&err) => {
                eprintln!(
                    "legacy server fanout interop test skipped due to missing prerequisites: {err}"
                );
            }
            Err(err) => panic!(
                "legacy server fanout interop for persistent-rooms lifecycle scenario should succeed, got: {err}"
            ),
        }
    }

    #[test]
    fn legacy_server_fanout_roundtrip_matches_server_runtime_on_permanent_rooms_file_scenario() {
        if !legacy_server_parity_assertions_enabled() {
            eprintln!(
                "legacy server parity assertion skipped; set SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
            );
            return;
        }
        match assert_legacy_server_fanout_matches_server_runtime_for_scenario_with_full_overrides(
            PERMANENT_ROOMS_FILE_SCENARIO,
            None,
            None,
            true,
            true,
            PERMANENT_ROOMS_FILE_LIST,
            PERMANENT_ROOMS_FILE_LIST,
        ) {
            Ok(()) => {}
            Err(err) if legacy_server_prerequisites_missing(&err) => {
                eprintln!(
                    "legacy server fanout interop test skipped due to missing prerequisites: {err}"
                );
            }
            Err(err) => panic!(
                "legacy server fanout interop for permanent-rooms-file scenario should succeed, got: {err}"
            ),
        }
    }

    #[test]
    fn legacy_server_fanout_roundtrip_matches_server_runtime_on_persistent_rooms_timeout_list_updates_scenario()
     {
        if !legacy_server_parity_assertions_enabled() {
            eprintln!(
                "legacy server parity assertion skipped; set SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
            );
            return;
        }
        match assert_legacy_server_fanout_matches_server_runtime_for_scenario_with_overrides(
            PERSISTENT_ROOMS_TIMEOUT_LIST_UPDATES_SCENARIO,
            None,
            None,
            true,
            true,
        ) {
            Ok(()) => {}
            Err(err) if legacy_server_prerequisites_missing(&err) => {
                eprintln!(
                    "legacy server fanout interop test skipped due to missing prerequisites: {err}"
                );
            }
            Err(err) => panic!(
                "legacy server fanout interop for persistent timeout-list-updates scenario should succeed, got: {err}"
            ),
        }
    }

    #[test]
    fn legacy_server_fanout_roundtrip_matches_server_runtime_on_controlled_room_permissions_scenario()
     {
        if !legacy_server_parity_assertions_enabled() {
            eprintln!(
                "legacy server parity assertion skipped; set SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
            );
            return;
        }
        match assert_legacy_server_fanout_matches_server_runtime_for_scenario(
            "server_runtime_controlled_room_permissions.jsonl",
        ) {
            Ok(()) => {}
            Err(err) if legacy_server_prerequisites_missing(&err) => {
                eprintln!(
                    "legacy server fanout interop test skipped due to missing prerequisites: {err}"
                );
            }
            Err(err) => panic!(
                "legacy server fanout interop for controlled-room permissions scenario should succeed, got: {err}"
            ),
        }
    }

    #[test]
    fn legacy_server_fanout_roundtrip_matches_server_runtime_on_controlled_room_invalid_password_scenario()
     {
        if !legacy_server_parity_assertions_enabled() {
            eprintln!(
                "legacy server parity assertion skipped; set SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
            );
            return;
        }
        match assert_legacy_server_fanout_matches_server_runtime_for_scenario(
            "server_runtime_controlled_room_invalid_password.jsonl",
        ) {
            Ok(()) => {}
            Err(err) if legacy_server_prerequisites_missing(&err) => {
                eprintln!(
                    "legacy server fanout interop test skipped due to missing prerequisites: {err}"
                );
            }
            Err(err) => panic!(
                "legacy server fanout interop for controlled-room invalid-password scenario should succeed, got: {err}"
            ),
        }
    }

    #[test]
    fn legacy_server_fanout_roundtrip_matches_server_runtime_on_controlled_room_state_forced_correction_scenario()
     {
        if !legacy_server_parity_assertions_enabled() {
            eprintln!(
                "legacy server parity assertion skipped; set SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
            );
            return;
        }
        match assert_legacy_server_fanout_matches_server_runtime_for_scenario(
            "server_runtime_controlled_room_state_forced_correction.jsonl",
        ) {
            Ok(()) => {}
            Err(err) if legacy_server_prerequisites_missing(&err) => {
                eprintln!(
                    "legacy server fanout interop test skipped due to missing prerequisites: {err}"
                );
            }
            Err(err) => panic!(
                "legacy server fanout interop for controlled-room forced-correction scenario should succeed, got: {err}"
            ),
        }
    }

    #[test]
    fn legacy_server_fanout_roundtrip_matches_server_runtime_on_state_metadata_forwarding_scenario()
    {
        if !legacy_server_parity_assertions_enabled() {
            eprintln!(
                "legacy server parity assertion skipped; set SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
            );
            return;
        }
        match assert_legacy_server_fanout_matches_server_runtime_for_scenario(
            "server_runtime_state_metadata_forwarding.jsonl",
        ) {
            Ok(()) => {}
            Err(err) if legacy_server_prerequisites_missing(&err) => {
                eprintln!(
                    "legacy server fanout interop test skipped due to missing prerequisites: {err}"
                );
            }
            Err(err) => panic!(
                "legacy server fanout interop for state metadata forwarding scenario should succeed, got: {err}"
            ),
        }
    }

    #[test]
    fn legacy_server_fanout_roundtrip_matches_server_runtime_on_state_periodic_timeout_scenario() {
        if !legacy_server_parity_assertions_enabled() {
            eprintln!(
                "legacy server parity assertion skipped; set SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY=1 to enable"
            );
            return;
        }
        match assert_legacy_server_fanout_matches_server_runtime_for_scenario(
            "server_runtime_state_periodic_timeout.jsonl",
        ) {
            Ok(()) => {}
            Err(err) if legacy_server_prerequisites_missing(&err) => {
                eprintln!(
                    "legacy server fanout interop test skipped due to missing prerequisites: {err}"
                );
            }
            Err(err) => panic!(
                "legacy server fanout interop for state periodic-timeout scenario should succeed, got: {err}"
            ),
        }
    }

    #[test]
    #[ignore = "requires Twisted and writes fixture files from a live legacy server session"]
    fn capture_legacy_server_state_latency_metrics_trace_fixture() {
        capture_legacy_server_trace_fixture(
            "server_runtime_state_latency_metrics.jsonl",
            "server_runtime_state_latency_metrics.legacy_trace.json",
        )
        .expect("state latency-metrics legacy trace capture should succeed");
    }

    #[test]
    #[ignore = "writes python fanout trace fixtures from current probe behavior"]
    fn capture_python_state_latency_metrics_trace_fixture() {
        capture_python_trace_fixture(
            "server_runtime_state_latency_metrics.jsonl",
            "server_runtime_state_latency_metrics.python_trace.json",
        )
        .expect("state latency-metrics python trace capture should succeed");
    }

    #[test]
    #[ignore = "writes persistent-room lifecycle python/legacy trace fixtures"]
    fn capture_persistent_rooms_lifecycle_trace_fixtures() {
        capture_python_trace_fixture_with_overrides(
            PERSISTENT_ROOMS_LIFECYCLE_SCENARIO,
            "server_runtime_persistent_rooms_lifecycle.python_trace.json",
            None,
            true,
        )
        .expect("persistent-rooms lifecycle python trace capture should succeed");
        capture_legacy_server_trace_fixture_with_overrides(
            PERSISTENT_ROOMS_LIFECYCLE_SCENARIO,
            "server_runtime_persistent_rooms_lifecycle.legacy_trace.json",
            super::DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT,
            None,
            true,
        )
        .expect("persistent-rooms lifecycle legacy trace capture should succeed");
        capture_legacy_server_trace_fixture_with_overrides(
            PERSISTENT_ROOMS_TIMEOUT_LIST_UPDATES_SCENARIO,
            "server_runtime_persistent_rooms_timeout_list_updates.legacy_trace.json",
            super::DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT,
            None,
            true,
        )
        .expect("persistent timeout-list-updates legacy trace capture should succeed");
        capture_legacy_server_trace_fixture_with_full_overrides(
            PERMANENT_ROOMS_FILE_SCENARIO,
            "server_runtime_permanent_rooms_file.legacy_trace.json",
            super::DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT,
            None,
            true,
            PERMANENT_ROOMS_FILE_LIST,
        )
        .expect("permanent-rooms-file legacy trace capture should succeed");
    }

    #[test]
    #[ignore = "writes permanent-rooms-file python/legacy trace fixtures"]
    fn capture_permanent_rooms_file_trace_fixtures() {
        capture_python_trace_fixture_with_full_overrides(
            PERMANENT_ROOMS_FILE_SCENARIO,
            "server_runtime_permanent_rooms_file.python_trace.json",
            None,
            true,
            PERMANENT_ROOMS_FILE_LIST,
        )
        .expect("permanent-rooms-file python trace capture should succeed");
        capture_legacy_server_trace_fixture_with_full_overrides(
            PERMANENT_ROOMS_FILE_SCENARIO,
            "server_runtime_permanent_rooms_file.legacy_trace.json",
            super::DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT,
            None,
            true,
            PERMANENT_ROOMS_FILE_LIST,
        )
        .expect("permanent-rooms-file legacy trace capture should succeed");
    }

    #[test]
    #[ignore = "requires Twisted and writes fixture files from a live legacy server session"]
    fn capture_legacy_server_controlled_room_trace_fixtures() {
        capture_legacy_server_trace_fixture(
            "server_runtime_controlled_room_permissions.jsonl",
            "server_runtime_controlled_room_permissions.legacy_trace.json",
        )
        .expect("controlled-room permissions legacy trace capture should succeed");
        capture_legacy_server_trace_fixture(
            "server_runtime_controlled_room_invalid_password.jsonl",
            "server_runtime_controlled_room_invalid_password.legacy_trace.json",
        )
        .expect("controlled-room invalid-password legacy trace capture should succeed");
        capture_legacy_server_trace_fixture(
            "server_runtime_controlled_room_state_forced_correction.jsonl",
            "server_runtime_controlled_room_state_forced_correction.legacy_trace.json",
        )
        .expect("controlled-room forced-correction legacy trace capture should succeed");
        capture_legacy_server_trace_fixture(
            "server_runtime_state_propagation.jsonl",
            "server_runtime_state_propagation.legacy_trace.json",
        )
        .expect("state propagation legacy trace capture should succeed");
        capture_legacy_server_trace_fixture(
            "server_runtime_state_metadata_forwarding.jsonl",
            "server_runtime_state_metadata_forwarding.legacy_trace.json",
        )
        .expect("state metadata forwarding legacy trace capture should succeed");
        capture_legacy_server_trace_fixture(
            "server_runtime_state_periodic_timeout.jsonl",
            "server_runtime_state_periodic_timeout.legacy_trace.json",
        )
        .expect("state periodic-timeout legacy trace capture should succeed");
        capture_legacy_server_trace_fixture(
            "server_runtime_state_latency_metrics.jsonl",
            "server_runtime_state_latency_metrics.legacy_trace.json",
        )
        .expect("state latency-metrics legacy trace capture should succeed");
        capture_legacy_server_trace_fixture(
            "server_runtime_username_conflict.jsonl",
            "server_runtime_username_conflict.legacy_trace.json",
        )
        .expect("username conflict legacy trace capture should succeed");
        capture_legacy_server_trace_fixture_with_salt_and_motd_template(
            MOTD_TEMPLATE_SCENARIO,
            "server_runtime_motd_template.legacy_trace.json",
            super::DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT,
            Some(MOTD_TEMPLATE_LEGACY_FILE),
        )
        .expect("motd-template legacy trace capture should succeed");
        capture_legacy_server_trace_fixture_with_salt_and_motd_template(
            MOTD_TEMPLATE_OUTDATED_SCENARIO,
            "server_runtime_motd_template_outdated_client.legacy_trace.json",
            super::DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT,
            Some(MOTD_TEMPLATE_LEGACY_FILE),
        )
        .expect("motd-template outdated-client legacy trace capture should succeed");
        capture_legacy_server_trace_fixture_with_overrides(
            PERSISTENT_ROOMS_NOTICE_SCENARIO,
            "server_runtime_persistent_rooms_notice.legacy_trace.json",
            super::DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT,
            None,
            true,
        )
        .expect("persistent-rooms notice legacy trace capture should succeed");
        capture_legacy_server_trace_fixture_with_overrides(
            PERSISTENT_ROOMS_LIFECYCLE_SCENARIO,
            "server_runtime_persistent_rooms_lifecycle.legacy_trace.json",
            super::DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT,
            None,
            true,
        )
        .expect("persistent-rooms lifecycle legacy trace capture should succeed");
    }

    #[test]
    #[ignore = "writes persistent timeout-list-updates python/legacy trace fixtures"]
    fn capture_persistent_rooms_timeout_list_updates_trace_fixtures() {
        capture_python_trace_fixture_with_overrides(
            PERSISTENT_ROOMS_TIMEOUT_LIST_UPDATES_SCENARIO,
            "server_runtime_persistent_rooms_timeout_list_updates.python_trace.json",
            None,
            true,
        )
        .expect("persistent timeout-list-updates python trace capture should succeed");
        capture_legacy_server_trace_fixture_with_overrides(
            PERSISTENT_ROOMS_TIMEOUT_LIST_UPDATES_SCENARIO,
            "server_runtime_persistent_rooms_timeout_list_updates.legacy_trace.json",
            super::DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT,
            None,
            true,
        )
        .expect("persistent timeout-list-updates legacy trace capture should succeed");
    }

    #[test]
    #[ignore = "writes python fanout trace fixtures from current probe behavior"]
    fn capture_python_fanout_trace_fixtures() {
        capture_python_trace_fixture(
            "server_runtime_fanout.jsonl",
            "server_runtime_fanout.python_trace.json",
        )
        .expect("fanout python trace capture should succeed");
        capture_python_trace_fixture(
            "server_runtime_playlist_controller.jsonl",
            "server_runtime_playlist_controller.python_trace.json",
        )
        .expect("playlist/controller python trace capture should succeed");
        capture_python_trace_fixture(
            "server_runtime_cross_room_ready_list.jsonl",
            "server_runtime_cross_room_ready_list.python_trace.json",
        )
        .expect("cross-room ready/list python trace capture should succeed");
        capture_python_trace_fixture(
            "server_runtime_controlled_room_permissions.jsonl",
            "server_runtime_controlled_room_permissions.python_trace.json",
        )
        .expect("controlled-room permissions python trace capture should succeed");
        capture_python_trace_fixture(
            "server_runtime_controlled_room_invalid_password.jsonl",
            "server_runtime_controlled_room_invalid_password.python_trace.json",
        )
        .expect("controlled-room invalid-password python trace capture should succeed");
        capture_python_trace_fixture(
            "server_runtime_controlled_room_state_forced_correction.jsonl",
            "server_runtime_controlled_room_state_forced_correction.python_trace.json",
        )
        .expect("controlled-room forced-correction python trace capture should succeed");
        capture_python_trace_fixture(
            "server_runtime_state_propagation.jsonl",
            "server_runtime_state_propagation.python_trace.json",
        )
        .expect("state propagation python trace capture should succeed");
        capture_python_trace_fixture(
            "server_runtime_state_metadata_forwarding.jsonl",
            "server_runtime_state_metadata_forwarding.python_trace.json",
        )
        .expect("state metadata forwarding python trace capture should succeed");
        capture_python_trace_fixture(
            "server_runtime_state_periodic_timeout.jsonl",
            "server_runtime_state_periodic_timeout.python_trace.json",
        )
        .expect("state periodic-timeout python trace capture should succeed");
        capture_python_trace_fixture(
            "server_runtime_state_latency_metrics.jsonl",
            "server_runtime_state_latency_metrics.python_trace.json",
        )
        .expect("state latency-metrics python trace capture should succeed");
        capture_python_trace_fixture(
            "server_runtime_username_conflict.jsonl",
            "server_runtime_username_conflict.python_trace.json",
        )
        .expect("username conflict python trace capture should succeed");
        capture_python_trace_fixture_with_motd_template(
            MOTD_TEMPLATE_SCENARIO,
            "server_runtime_motd_template.python_trace.json",
            Some(MOTD_TEMPLATE_RUNTIME_AND_PROBE),
        )
        .expect("motd-template python trace capture should succeed");
        capture_python_trace_fixture_with_motd_template(
            MOTD_TEMPLATE_OUTDATED_SCENARIO,
            "server_runtime_motd_template_outdated_client.python_trace.json",
            Some(MOTD_TEMPLATE_RUNTIME_AND_PROBE),
        )
        .expect("motd-template outdated-client python trace capture should succeed");
        capture_python_trace_fixture_with_overrides(
            PERSISTENT_ROOMS_NOTICE_SCENARIO,
            "server_runtime_persistent_rooms_notice.python_trace.json",
            None,
            true,
        )
        .expect("persistent-rooms notice python trace capture should succeed");
        capture_python_trace_fixture_with_overrides(
            PERSISTENT_ROOMS_LIFECYCLE_SCENARIO,
            "server_runtime_persistent_rooms_lifecycle.python_trace.json",
            None,
            true,
        )
        .expect("persistent-rooms lifecycle python trace capture should succeed");
        capture_python_trace_fixture_with_overrides(
            PERSISTENT_ROOMS_TIMEOUT_LIST_UPDATES_SCENARIO,
            "server_runtime_persistent_rooms_timeout_list_updates.python_trace.json",
            None,
            true,
        )
        .expect("persistent timeout-list-updates python trace capture should succeed");
        capture_python_trace_fixture_with_full_overrides(
            PERMANENT_ROOMS_FILE_SCENARIO,
            "server_runtime_permanent_rooms_file.python_trace.json",
            None,
            true,
            PERMANENT_ROOMS_FILE_LIST,
        )
        .expect("permanent-rooms-file python trace capture should succeed");
    }
}
