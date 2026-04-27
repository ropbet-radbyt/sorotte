use std::time::Duration;

use anyhow::anyhow;
use serde_json::Value;
use syncplay_client_app::app_boundary::{
    commands::{
        LocalInputCommandPlanningContext, PlannedLocalInputDispatch, parse_local_input_command,
        plan_local_input_command_legacy_compatible, plan_local_input_dispatch_legacy_compatible,
        render_local_input_display_lines_legacy_compatible as shared_render_local_input_display_lines_legacy_compatible,
    },
    diagnostics::ReconnectCorrectionDiagnosticsState,
    notifications::FileDifferenceNotificationState,
    session::{
        ClientNetworkLoopAttemptDisposition, ClientNetworkLoopAttemptExecutionPlan,
        ClientNetworkLoopAttemptPlan, ClientNetworkLoopEventPlan,
        ClientNetworkLoopExecutionOutcome, ClientNetworkLoopReconnectExhaustedErrorAction,
        ClientNetworkLoopReconnectExhaustedErrorKind, ClientNetworkLoopStartupPlan,
        ClientNetworkLoopStartupPlanInputs, ConnectedSessionBranchPlan,
        ConnectedSessionDiagnosticsPlan, ConnectedSessionDrainAction, ConnectedSessionDrainPlan,
        ConnectedSessionEventExecutionPlan, ConnectedSessionInboundApplyPlan,
        ConnectedSessionInboundPostApplyAction, ConnectedSessionInboundPostApplyPlan,
        ConnectedSessionOuterLoopExitKind as ConnectedSessionExit, ConnectedSessionProtocolPlan,
        ConnectedSessionRuntimeStepAction, ConnectedSessionRuntimeStepPlan,
        ConnectedSessionSharedExecutionInputs, ConnectedSessionStartupPlaylistDisposition,
        client_network_loop_attempt_disposition_for_execution_plan_legacy_compatible,
        client_network_loop_attempt_execution_plan_for_connect_failure_legacy_compatible,
        client_network_loop_attempt_execution_plan_for_connected_session_exit_legacy_compatible,
        client_network_loop_execution_outcome_legacy_compatible,
        client_network_loop_reconnect_exhausted_error_action_legacy_compatible,
        client_network_loop_startup_plan_legacy_compatible,
        client_reconnect_backoff_plan_legacy_compatible,
        connected_session_autoplay_tick_event_execution_plan_legacy_compatible,
        connected_session_drain_actions_legacy_compatible,
        connected_session_inbound_message_event_execution_plan_legacy_compatible,
        connected_session_inbound_post_apply_actions_legacy_compatible,
        connected_session_local_input_event_execution_plan_legacy_compatible,
        connected_session_runtime_step_actions_legacy_compatible,
    },
    state::StoredClientSettingsMvp,
};
use syncplay_client_core::{
    AUTOPLAY_TICK_INTERVAL_SECONDS, AutoplayCountdownNotification, ClientRuntime,
    QueuedRuntimeControl, SYNCPLAY_COMPAT_VERSION_LEGACY, legacy_server_password_token,
};
use syncplay_player_mpv::MpvAdapter;
use syncplay_protocol::{
    HelloPayload, ProtocolMessage, StatePayload, decode_message_line, decode_message_lines,
    encode_message_line,
};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::Instant;

use crate::client_args::LegacyClientArgOverrides;
use crate::client_config::{
    ClientLoopConfig, client_hello_features_legacy_compatible, derive_runtime_loop_inputs,
    shared_playlists_enabled_cli_legacy_compatible,
};
use crate::diagnostics_config::{ClientLoopDiagnosticsConfig, client_loop_diagnostics_config};
use crate::env_support::{env_flag_enabled, env_flag_override, env_trimmed};
use crate::language_support::current_legacy_runtime_language_tag_legacy_compatible;
use crate::local_runtime_actions::{
    PLAYER_CHAT_INPUT_POLL_INTERVAL_MS, drain_player_chat_input_legacy_compatible,
    publish_pending_local_file_updates, run_planned_local_runtime_action_legacy_compatible,
};
use crate::mpv_startup::{
    ManagedMpvProcessGuard,
    apply_legacy_startup_file_to_attached_player_if_explicit_mpv_ipc_legacy_compatible,
    create_client_runtime_with_managed_mpv_support,
};
use crate::notifications::{
    emit_autoplay_countdown_notification, emit_file_difference_notification,
    emit_reconnect_correction_diagnostic, flush_autoplay_notifications_legacy_compatible,
    flush_chat_notifications_legacy_compatible,
    flush_controller_auth_notifications_legacy_compatible,
    flush_file_difference_notifications_legacy_compatible,
    flush_player_playback_telemetry_diagnostics, flush_reconnect_correction_diagnostics_to_sink,
    flush_reconnect_notifications_legacy_compatible,
    flush_user_change_notifications_legacy_compatible,
};
use crate::protocol_io::{flush_runtime_protocol_lines, write_protocol_line};
use crate::startup_playlist::emit_startup_playlist_load_from_file_legacy_compatible;
use crate::stdin_input::{recv_local_input_line, spawn_local_input_receiver_legacy_compatible};

mod connected_session;
mod network_loop;

use self::connected_session::{
    ConnectedSessionLaunchContext,
    run_connected_client_session_with_legacy_startup_overrides_and_diagnostics,
};

#[cfg(test)]
pub(super) use self::connected_session::{
    run_connected_client_session, run_connected_client_session_with_legacy_startup_overrides,
};
#[cfg(test)]
pub(super) use self::network_loop::run_client_network_loop;
pub(super) use self::network_loop::run_client_network_loop_with_legacy_startup_overrides_and_stored_settings;
