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

use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned, pki_types::ServerName};
use serde_json::{Value, json};

use super::legacy_server::run_legacy_server_fanout_roundtrip_with_full_overrides;
use super::scenario_replay::{
    replay_server_runtime_scenario_steps_with_full_overrides,
    run_python_fanout_roundtrip_with_full_overrides,
};
#[cfg(feature = "trace-capture")]
use super::trace_capture::{
    capture_legacy_server_trace_fixture_with_full_overrides,
    capture_python_trace_fixture_with_full_overrides,
};
use super::{
    DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT, InteropError,
    LEGACY_COMPAT_MISSING_FEATURES_MARKER, LegacyClientChatSendContractCase,
    LegacyServerClientConnection, ServerRuntimeScenarioEvent, ServerRuntimeScenarioStep,
    all_protocol_fixture_names, connect_legacy_client_stream, decode_fixture, decode_protocol_file,
    default_rust_client_hello_for_interop, default_rust_client_hello_for_legacy_live_tls,
    ensure_legacy_server_is_running, ensure_legacy_syncplay_checkout_available, fixture_decodes,
    fixture_path, legacy_syncplay_checkout_dir, legacy_syncplay_server_entry_script_path,
    load_server_runtime_scenario_fixture, prepare_legacy_server_request_line, python_bin_from_env,
    replay_server_runtime_scenario_fixture, replay_server_runtime_scenario_steps,
    replay_server_runtime_scenario_steps_with_motd_template,
    replay_server_runtime_scenario_steps_with_overrides, reserve_ephemeral_tcp_port,
    run_legacy_server_fanout_roundtrip, run_python_fanout_roundtrip,
    run_python_fanout_roundtrip_with_tls_available, run_python_handshake_roundtrip,
    run_python_legacy_client_chat_send_contract_batch,
    run_python_legacy_client_set_file_contract_probe,
    run_python_legacy_client_user_file_metadata_probe, run_python_privacy_file_payload_batch,
    run_python_protocol_roundtrip, run_python_same_fileduration_batch,
    run_python_same_fileduration_batch_with_overrides, run_python_same_filename_batch,
    run_python_same_filesize_batch, scenario_fixture_path, terminate_legacy_server_process,
    wait_for_legacy_server_startup,
};
#[cfg(feature = "trace-capture")]
use super::{
    capture_legacy_server_trace_fixture, capture_legacy_server_trace_fixture_with_overrides,
    capture_legacy_server_trace_fixture_with_salt_and_motd_template, capture_python_trace_fixture,
    capture_python_trace_fixture_with_motd_template, capture_python_trace_fixture_with_overrides,
};
use sorotte_client_core::{ClientRuntimeAction, ClientSession, PrivacyMode};
use sorotte_protocol::{
    ChatPayload, ListPayload, PlaystatePayload, ProtocolMessage, ReadyPayload, RoomRef, SetPayload,
    StatePayload, decode_message_line, encode_message_line, extract_hello_from_message,
};
use sorotte_server::ServerRuntime;

mod normalization_support;
mod scenario_constants;
mod tls_fixture_support;
use self::normalization_support::*;
use self::scenario_constants::*;
use self::tls_fixture_support::*;
mod assertions;
use self::assertions::*;
mod chat_fanout_tests;
mod controlled_room_fanout_tests;
mod fixture_tests;
mod legacy_client_contract_tests;
mod legacy_tls_tests;
mod normalization_tests;
mod playlist_fanout_tests;
mod python_protocol_tests;
mod rooms_motd_fanout_tests;
mod scenario_replay_tests;
mod state_fanout_tests;
mod trace_shape_tests;
