use crate::legacy_reconnect_diagnostics::ReconnectCorrectionDiagnosticsFormat;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectedSessionLoopEventKind {
    InboundMessage,
    AutoplayTick,
    LocalInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectedSessionDiagnosticsPlan {
    pub log_player_telemetry: bool,
    pub log_player_drift: bool,
    pub reconnect_correction_diagnostics_format: Option<ReconnectCorrectionDiagnosticsFormat>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectedSessionDrainPlan {
    pub flush_player_playback_diagnostics: bool,
    pub reconnect_correction_diagnostics_format: Option<ReconnectCorrectionDiagnosticsFormat>,
    pub flush_reconnect_notifications: bool,
    pub flush_controller_auth_notifications: bool,
    pub flush_chat_notifications: bool,
    pub flush_user_change_notifications: bool,
    pub flush_autoplay_notifications: bool,
    pub flush_file_difference_notifications: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectedSessionDrainAction {
    FlushPlayerPlaybackDiagnostics,
    FlushReconnectNotifications,
    FlushReconnectCorrectionDiagnostics(ReconnectCorrectionDiagnosticsFormat),
    FlushControllerAuthNotifications,
    FlushChatNotifications,
    FlushUserChangeNotifications,
    FlushAutoplayNotifications,
    FlushFileDifferenceNotifications,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectedSessionRuntimeStepPlan {
    pub run_room_pause_sync: bool,
    pub run_readiness_unpause_attempt: bool,
    pub run_update_autoplay_check: bool,
    pub run_tick_autoplay: bool,
    pub run_desync_correction: bool,
    pub run_reconnect_state_restore_validation: bool,
    pub run_state_sync_heartbeat: bool,
    pub publish_pending_local_file_updates: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectedSessionRuntimeStepAction {
    RunRoomPauseSync,
    RunReadinessUnpauseAttempt,
    RunUpdateAutoplayCheck,
    RunTickAutoplay,
    RunDesyncCorrection,
    RunReconnectStateRestoreValidation,
    RunStateSyncHeartbeat,
    PublishPendingLocalFileUpdates,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectedSessionInboundPostApplyPlan {
    pub consume_pending_ready_at_start: bool,
    pub consume_pending_chat_message_on_connect: bool,
    pub run_reconnect_transition: bool,
    pub run_controller_reidentify: bool,
    pub run_controller_auth_notifications: bool,
    pub run_chat_notifications: bool,
    pub run_user_change_notifications: bool,
    pub run_reconnect_state_restore: bool,
    pub run_reconnect_playlist_restore: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectedSessionInboundPostApplyAction {
    ConsumePendingReadyAtStart,
    ConsumePendingChatMessageOnConnect,
    RunReconnectTransition,
    RunControllerReidentify,
    RunControllerAuthNotifications,
    RunChatNotifications,
    RunUserChangeNotifications,
    RunReconnectStateRestore,
    RunReconnectPlaylistRestore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectedSessionInboundApplyPlan {
    pub reconcile_inbound_state: bool,
    pub apply_message_json_at: bool,
    pub outbound_state_sync_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectedSessionStartupPlaylistDisposition {
    LeavePending,
    EmitIfAvailable,
    DiscardIfPending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectedSessionProtocolPlan {
    pub flush_runtime_protocol_lines: bool,
    pub startup_playlist_disposition: ConnectedSessionStartupPlaylistDisposition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectedSessionBranchPlan {
    pub run_protocol_before_runtime_steps: bool,
    pub runtime_steps: ConnectedSessionRuntimeStepPlan,
    pub protocol: ConnectedSessionProtocolPlan,
    pub drain: ConnectedSessionDrainPlan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectedSessionEventPlanInputs {
    pub emitted_runtime_action: bool,
    pub inbound_is_server_hello: bool,
    pub has_pending_chat_message_on_connect: bool,
    pub shared_playlists_enabled: bool,
    pub diagnostics: ConnectedSessionDiagnosticsPlan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectedSessionEventPlan {
    pub inbound_post_apply: Option<ConnectedSessionInboundPostApplyPlan>,
    pub branch: ConnectedSessionBranchPlan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectedSessionEventExecutionPlanInputs {
    pub event: ConnectedSessionEventPlanInputs,
    pub inbound_message_is_state: bool,
    pub outbound_state_sync_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectedSessionEventExecutionPlan {
    pub inbound_apply: Option<ConnectedSessionInboundApplyPlan>,
    pub event: ConnectedSessionEventPlan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientNetworkLoopEventKind {
    ConnectFailed,
    ConnectedSessionTransportClosed,
    ConnectedSessionRuntimeWindowElapsed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientNetworkLoopEventPlan {
    pub return_success: bool,
    pub run_disconnect: bool,
    pub run_reconnect_backoff: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientNetworkLoopExecutionOutcome {
    ReturnSuccess,
    Continue,
    ReconnectExhausted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientNetworkLoopAttemptOutcomeKind {
    ConnectFailed,
    ConnectedSessionTransportClosed,
    ConnectedSessionRuntimeWindowElapsed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientNetworkLoopAttemptSource {
    ConnectFailed,
    ConnectedSession(ConnectedSessionOuterLoopExitKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectedSessionOuterLoopExitKind {
    TransportClosed,
    RuntimeWindowElapsed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientNetworkLoopReconnectExhaustedErrorKind {
    ConnectError,
    TransportClosedMessage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientNetworkLoopAttemptDisposition {
    ReturnSuccess,
    Continue,
    ReconnectExhausted(ClientNetworkLoopReconnectExhaustedErrorKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientNetworkLoopReconnectExhaustedErrorAction {
    UseConnectError,
    StaticMessage(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClientReconnectBackoffPlan {
    pub stop_retrying: bool,
    pub sleep_delay_seconds: Option<f64>,
    pub next_retries: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientNetworkLoopAttemptPlan {
    pub reset_retries_before_event: bool,
    pub event: ClientNetworkLoopEventPlan,
    pub reconnect_exhausted_error_kind: Option<ClientNetworkLoopReconnectExhaustedErrorKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientNetworkLoopAttemptExecutionPlan {
    pub source: ClientNetworkLoopAttemptSource,
    pub attempt_plan: ClientNetworkLoopAttemptPlan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientNetworkLoopStartupPlanInputs<'a> {
    pub endpoint_host: &'a str,
    pub endpoint_port: u16,
    pub stdin_enabled: bool,
    pub has_legacy_overrides: bool,
    pub chat_message_on_connect: Option<&'a str>,
    pub startup_playlist_file_on_connect: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientNetworkLoopStartupPlan {
    pub apply_legacy_explicit_mpv_ipc_startup: bool,
    pub spawn_local_input_receiver: bool,
    pub endpoint: String,
    pub chat_message_on_connect: Option<String>,
    pub startup_playlist_file_on_connect: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectedSessionSharedExecutionInputs {
    pub shared_playlists_enabled: bool,
    pub diagnostics: ConnectedSessionDiagnosticsPlan,
    pub outbound_state_sync_enabled: bool,
}

pub fn connected_session_inbound_post_apply_plan_legacy_compatible(
    inbound_is_server_hello: bool,
    has_pending_chat_message_on_connect: bool,
    shared_playlists_enabled: bool,
) -> ConnectedSessionInboundPostApplyPlan {
    ConnectedSessionInboundPostApplyPlan {
        consume_pending_ready_at_start: inbound_is_server_hello,
        consume_pending_chat_message_on_connect: has_pending_chat_message_on_connect,
        run_reconnect_transition: true,
        run_controller_reidentify: true,
        run_controller_auth_notifications: true,
        run_chat_notifications: true,
        run_user_change_notifications: true,
        run_reconnect_state_restore: true,
        run_reconnect_playlist_restore: shared_playlists_enabled,
    }
}

pub fn connected_session_inbound_post_apply_actions_legacy_compatible(
    plan: ConnectedSessionInboundPostApplyPlan,
) -> Vec<ConnectedSessionInboundPostApplyAction> {
    let mut actions = Vec::new();

    if plan.consume_pending_ready_at_start {
        actions.push(ConnectedSessionInboundPostApplyAction::ConsumePendingReadyAtStart);
    }
    if plan.consume_pending_chat_message_on_connect {
        actions.push(ConnectedSessionInboundPostApplyAction::ConsumePendingChatMessageOnConnect);
    }
    if plan.run_reconnect_transition {
        actions.push(ConnectedSessionInboundPostApplyAction::RunReconnectTransition);
    }
    if plan.run_controller_reidentify {
        actions.push(ConnectedSessionInboundPostApplyAction::RunControllerReidentify);
    }
    if plan.run_controller_auth_notifications {
        actions.push(ConnectedSessionInboundPostApplyAction::RunControllerAuthNotifications);
    }
    if plan.run_chat_notifications {
        actions.push(ConnectedSessionInboundPostApplyAction::RunChatNotifications);
    }
    if plan.run_user_change_notifications {
        actions.push(ConnectedSessionInboundPostApplyAction::RunUserChangeNotifications);
    }
    if plan.run_reconnect_state_restore {
        actions.push(ConnectedSessionInboundPostApplyAction::RunReconnectStateRestore);
    }
    if plan.run_reconnect_playlist_restore {
        actions.push(ConnectedSessionInboundPostApplyAction::RunReconnectPlaylistRestore);
    }

    actions
}

pub fn connected_session_inbound_apply_plan_legacy_compatible(
    inbound_message_is_state: bool,
    outbound_state_sync_enabled: bool,
) -> ConnectedSessionInboundApplyPlan {
    if inbound_message_is_state {
        ConnectedSessionInboundApplyPlan {
            reconcile_inbound_state: true,
            apply_message_json_at: false,
            outbound_state_sync_enabled: true,
        }
    } else {
        ConnectedSessionInboundApplyPlan {
            reconcile_inbound_state: false,
            apply_message_json_at: true,
            outbound_state_sync_enabled,
        }
    }
}

pub fn connected_session_protocol_plan_legacy_compatible(
    event_kind: ConnectedSessionLoopEventKind,
    emitted_runtime_action: bool,
    shared_playlists_enabled: bool,
) -> ConnectedSessionProtocolPlan {
    match event_kind {
        ConnectedSessionLoopEventKind::InboundMessage => ConnectedSessionProtocolPlan {
            flush_runtime_protocol_lines: true,
            startup_playlist_disposition: if shared_playlists_enabled {
                ConnectedSessionStartupPlaylistDisposition::EmitIfAvailable
            } else {
                ConnectedSessionStartupPlaylistDisposition::DiscardIfPending
            },
        },
        ConnectedSessionLoopEventKind::AutoplayTick => ConnectedSessionProtocolPlan {
            flush_runtime_protocol_lines: true,
            startup_playlist_disposition: ConnectedSessionStartupPlaylistDisposition::LeavePending,
        },
        ConnectedSessionLoopEventKind::LocalInput => ConnectedSessionProtocolPlan {
            flush_runtime_protocol_lines: emitted_runtime_action,
            startup_playlist_disposition: ConnectedSessionStartupPlaylistDisposition::LeavePending,
        },
    }
}

pub fn connected_session_branch_plan_legacy_compatible(
    event_kind: ConnectedSessionLoopEventKind,
    emitted_runtime_action: bool,
    shared_playlists_enabled: bool,
    diagnostics: ConnectedSessionDiagnosticsPlan,
) -> ConnectedSessionBranchPlan {
    ConnectedSessionBranchPlan {
        run_protocol_before_runtime_steps: matches!(
            event_kind,
            ConnectedSessionLoopEventKind::LocalInput
        ),
        runtime_steps: connected_session_runtime_step_plan_legacy_compatible(event_kind),
        protocol: connected_session_protocol_plan_legacy_compatible(
            event_kind,
            emitted_runtime_action,
            shared_playlists_enabled,
        ),
        drain: connected_session_drain_plan_legacy_compatible(event_kind, diagnostics),
    }
}

pub fn connected_session_event_plan_legacy_compatible(
    event_kind: ConnectedSessionLoopEventKind,
    inputs: ConnectedSessionEventPlanInputs,
) -> ConnectedSessionEventPlan {
    ConnectedSessionEventPlan {
        inbound_post_apply: match event_kind {
            ConnectedSessionLoopEventKind::InboundMessage => {
                Some(connected_session_inbound_post_apply_plan_legacy_compatible(
                    inputs.inbound_is_server_hello,
                    inputs.has_pending_chat_message_on_connect,
                    inputs.shared_playlists_enabled,
                ))
            }
            ConnectedSessionLoopEventKind::AutoplayTick
            | ConnectedSessionLoopEventKind::LocalInput => None,
        },
        branch: connected_session_branch_plan_legacy_compatible(
            event_kind,
            inputs.emitted_runtime_action,
            inputs.shared_playlists_enabled,
            inputs.diagnostics,
        ),
    }
}

pub fn connected_session_event_execution_plan_legacy_compatible(
    event_kind: ConnectedSessionLoopEventKind,
    inputs: ConnectedSessionEventExecutionPlanInputs,
) -> ConnectedSessionEventExecutionPlan {
    ConnectedSessionEventExecutionPlan {
        inbound_apply: match event_kind {
            ConnectedSessionLoopEventKind::InboundMessage => {
                Some(connected_session_inbound_apply_plan_legacy_compatible(
                    inputs.inbound_message_is_state,
                    inputs.outbound_state_sync_enabled,
                ))
            }
            ConnectedSessionLoopEventKind::AutoplayTick
            | ConnectedSessionLoopEventKind::LocalInput => None,
        },
        event: connected_session_event_plan_legacy_compatible(event_kind, inputs.event),
    }
}

pub fn connected_session_inbound_message_event_execution_plan_legacy_compatible(
    inbound_is_server_hello: bool,
    has_pending_chat_message_on_connect: bool,
    inbound_message_is_state: bool,
    shared: ConnectedSessionSharedExecutionInputs,
) -> ConnectedSessionEventExecutionPlan {
    connected_session_event_execution_plan_legacy_compatible(
        ConnectedSessionLoopEventKind::InboundMessage,
        ConnectedSessionEventExecutionPlanInputs {
            event: ConnectedSessionEventPlanInputs {
                emitted_runtime_action: false,
                inbound_is_server_hello,
                has_pending_chat_message_on_connect,
                shared_playlists_enabled: shared.shared_playlists_enabled,
                diagnostics: shared.diagnostics,
            },
            inbound_message_is_state,
            outbound_state_sync_enabled: shared.outbound_state_sync_enabled,
        },
    )
}

pub fn connected_session_autoplay_tick_event_execution_plan_legacy_compatible(
    shared: ConnectedSessionSharedExecutionInputs,
) -> ConnectedSessionEventExecutionPlan {
    connected_session_event_execution_plan_legacy_compatible(
        ConnectedSessionLoopEventKind::AutoplayTick,
        ConnectedSessionEventExecutionPlanInputs {
            event: ConnectedSessionEventPlanInputs {
                emitted_runtime_action: false,
                inbound_is_server_hello: false,
                has_pending_chat_message_on_connect: false,
                shared_playlists_enabled: shared.shared_playlists_enabled,
                diagnostics: shared.diagnostics,
            },
            inbound_message_is_state: false,
            outbound_state_sync_enabled: shared.outbound_state_sync_enabled,
        },
    )
}

pub fn connected_session_local_input_event_execution_plan_legacy_compatible(
    emitted_runtime_action: bool,
    shared: ConnectedSessionSharedExecutionInputs,
) -> ConnectedSessionEventExecutionPlan {
    connected_session_event_execution_plan_legacy_compatible(
        ConnectedSessionLoopEventKind::LocalInput,
        ConnectedSessionEventExecutionPlanInputs {
            event: ConnectedSessionEventPlanInputs {
                emitted_runtime_action,
                inbound_is_server_hello: false,
                has_pending_chat_message_on_connect: false,
                shared_playlists_enabled: shared.shared_playlists_enabled,
                diagnostics: shared.diagnostics,
            },
            inbound_message_is_state: false,
            outbound_state_sync_enabled: shared.outbound_state_sync_enabled,
        },
    )
}

pub fn client_network_loop_event_plan_legacy_compatible(
    event_kind: ClientNetworkLoopEventKind,
) -> ClientNetworkLoopEventPlan {
    match event_kind {
        ClientNetworkLoopEventKind::ConnectFailed => ClientNetworkLoopEventPlan {
            return_success: false,
            run_disconnect: false,
            run_reconnect_backoff: true,
        },
        ClientNetworkLoopEventKind::ConnectedSessionTransportClosed => ClientNetworkLoopEventPlan {
            return_success: false,
            run_disconnect: true,
            run_reconnect_backoff: true,
        },
        ClientNetworkLoopEventKind::ConnectedSessionRuntimeWindowElapsed => {
            ClientNetworkLoopEventPlan {
                return_success: true,
                run_disconnect: false,
                run_reconnect_backoff: false,
            }
        }
    }
}

pub fn client_network_loop_attempt_plan_legacy_compatible(
    outcome_kind: ClientNetworkLoopAttemptOutcomeKind,
) -> ClientNetworkLoopAttemptPlan {
    match outcome_kind {
        ClientNetworkLoopAttemptOutcomeKind::ConnectFailed => ClientNetworkLoopAttemptPlan {
            reset_retries_before_event: false,
            event: client_network_loop_event_plan_legacy_compatible(
                ClientNetworkLoopEventKind::ConnectFailed,
            ),
            reconnect_exhausted_error_kind: Some(
                ClientNetworkLoopReconnectExhaustedErrorKind::ConnectError,
            ),
        },
        ClientNetworkLoopAttemptOutcomeKind::ConnectedSessionTransportClosed => {
            ClientNetworkLoopAttemptPlan {
                reset_retries_before_event: true,
                event: client_network_loop_event_plan_legacy_compatible(
                    ClientNetworkLoopEventKind::ConnectedSessionTransportClosed,
                ),
                reconnect_exhausted_error_kind: Some(
                    ClientNetworkLoopReconnectExhaustedErrorKind::TransportClosedMessage,
                ),
            }
        }
        ClientNetworkLoopAttemptOutcomeKind::ConnectedSessionRuntimeWindowElapsed => {
            ClientNetworkLoopAttemptPlan {
                reset_retries_before_event: true,
                event: client_network_loop_event_plan_legacy_compatible(
                    ClientNetworkLoopEventKind::ConnectedSessionRuntimeWindowElapsed,
                ),
                reconnect_exhausted_error_kind: None,
            }
        }
    }
}

pub fn client_network_loop_attempt_plan_for_connect_failure_legacy_compatible()
-> ClientNetworkLoopAttemptPlan {
    client_network_loop_attempt_plan_legacy_compatible(
        ClientNetworkLoopAttemptOutcomeKind::ConnectFailed,
    )
}

pub fn client_network_loop_attempt_plan_for_connected_session_exit_legacy_compatible(
    exit_kind: ConnectedSessionOuterLoopExitKind,
) -> ClientNetworkLoopAttemptPlan {
    client_network_loop_attempt_plan_legacy_compatible(match exit_kind {
        ConnectedSessionOuterLoopExitKind::TransportClosed => {
            ClientNetworkLoopAttemptOutcomeKind::ConnectedSessionTransportClosed
        }
        ConnectedSessionOuterLoopExitKind::RuntimeWindowElapsed => {
            ClientNetworkLoopAttemptOutcomeKind::ConnectedSessionRuntimeWindowElapsed
        }
    })
}

pub fn client_network_loop_attempt_plan_for_source_legacy_compatible(
    source: ClientNetworkLoopAttemptSource,
) -> ClientNetworkLoopAttemptPlan {
    match source {
        ClientNetworkLoopAttemptSource::ConnectFailed => {
            client_network_loop_attempt_plan_for_connect_failure_legacy_compatible()
        }
        ClientNetworkLoopAttemptSource::ConnectedSession(exit_kind) => {
            client_network_loop_attempt_plan_for_connected_session_exit_legacy_compatible(exit_kind)
        }
    }
}

pub fn client_network_loop_attempt_execution_plan_for_source_legacy_compatible(
    source: ClientNetworkLoopAttemptSource,
) -> ClientNetworkLoopAttemptExecutionPlan {
    ClientNetworkLoopAttemptExecutionPlan {
        source,
        attempt_plan: client_network_loop_attempt_plan_for_source_legacy_compatible(source),
    }
}

pub fn client_network_loop_attempt_execution_plan_for_connect_failure_legacy_compatible()
-> ClientNetworkLoopAttemptExecutionPlan {
    client_network_loop_attempt_execution_plan_for_source_legacy_compatible(
        ClientNetworkLoopAttemptSource::ConnectFailed,
    )
}

pub fn client_network_loop_attempt_execution_plan_for_connected_session_exit_legacy_compatible(
    exit_kind: ConnectedSessionOuterLoopExitKind,
) -> ClientNetworkLoopAttemptExecutionPlan {
    client_network_loop_attempt_execution_plan_for_source_legacy_compatible(
        ClientNetworkLoopAttemptSource::ConnectedSession(exit_kind),
    )
}

pub fn client_network_loop_execution_outcome_legacy_compatible(
    plan: ClientNetworkLoopEventPlan,
    reconnect_exhausted: bool,
) -> ClientNetworkLoopExecutionOutcome {
    if plan.return_success {
        ClientNetworkLoopExecutionOutcome::ReturnSuccess
    } else if reconnect_exhausted {
        ClientNetworkLoopExecutionOutcome::ReconnectExhausted
    } else {
        ClientNetworkLoopExecutionOutcome::Continue
    }
}

pub fn client_network_loop_attempt_disposition_legacy_compatible(
    plan: ClientNetworkLoopAttemptPlan,
    outcome: ClientNetworkLoopExecutionOutcome,
) -> ClientNetworkLoopAttemptDisposition {
    match outcome {
        ClientNetworkLoopExecutionOutcome::ReturnSuccess => {
            ClientNetworkLoopAttemptDisposition::ReturnSuccess
        }
        ClientNetworkLoopExecutionOutcome::Continue => {
            ClientNetworkLoopAttemptDisposition::Continue
        }
        ClientNetworkLoopExecutionOutcome::ReconnectExhausted => {
            ClientNetworkLoopAttemptDisposition::ReconnectExhausted(
                plan.reconnect_exhausted_error_kind
                    .expect("reconnect exhaustion must map to a defined error policy"),
            )
        }
    }
}

pub fn client_network_loop_attempt_disposition_for_execution_plan_legacy_compatible(
    execution_plan: ClientNetworkLoopAttemptExecutionPlan,
    outcome: ClientNetworkLoopExecutionOutcome,
) -> ClientNetworkLoopAttemptDisposition {
    let derived_attempt_plan =
        client_network_loop_attempt_plan_for_source_legacy_compatible(execution_plan.source);
    debug_assert_eq!(execution_plan.attempt_plan, derived_attempt_plan);
    client_network_loop_attempt_disposition_legacy_compatible(derived_attempt_plan, outcome)
}

pub fn client_network_loop_reconnect_exhausted_error_action_legacy_compatible(
    kind: ClientNetworkLoopReconnectExhaustedErrorKind,
) -> ClientNetworkLoopReconnectExhaustedErrorAction {
    match kind {
        ClientNetworkLoopReconnectExhaustedErrorKind::ConnectError => {
            ClientNetworkLoopReconnectExhaustedErrorAction::UseConnectError
        }
        ClientNetworkLoopReconnectExhaustedErrorKind::TransportClosedMessage => {
            ClientNetworkLoopReconnectExhaustedErrorAction::StaticMessage(
                "server connection closed and reconnect retries were exhausted",
            )
        }
    }
}

pub fn client_reconnect_backoff_plan_legacy_compatible(
    current_retries: u32,
    stop_requested: bool,
    reconnect_delay_seconds: Option<f64>,
) -> ClientReconnectBackoffPlan {
    if stop_requested {
        ClientReconnectBackoffPlan {
            stop_retrying: true,
            sleep_delay_seconds: None,
            next_retries: current_retries,
        }
    } else {
        ClientReconnectBackoffPlan {
            stop_retrying: false,
            sleep_delay_seconds: Some(reconnect_delay_seconds.unwrap_or(0.1)),
            next_retries: current_retries.saturating_add(1),
        }
    }
}

pub fn client_network_loop_startup_plan_legacy_compatible(
    inputs: ClientNetworkLoopStartupPlanInputs<'_>,
) -> ClientNetworkLoopStartupPlan {
    ClientNetworkLoopStartupPlan {
        apply_legacy_explicit_mpv_ipc_startup: inputs.has_legacy_overrides,
        spawn_local_input_receiver: inputs.stdin_enabled,
        endpoint: format!("{}:{}", inputs.endpoint_host, inputs.endpoint_port),
        chat_message_on_connect: inputs.chat_message_on_connect.map(str::to_owned),
        startup_playlist_file_on_connect: inputs
            .startup_playlist_file_on_connect
            .map(str::to_owned),
    }
}

pub fn connected_session_runtime_step_plan_legacy_compatible(
    event_kind: ConnectedSessionLoopEventKind,
) -> ConnectedSessionRuntimeStepPlan {
    match event_kind {
        ConnectedSessionLoopEventKind::InboundMessage => ConnectedSessionRuntimeStepPlan {
            run_room_pause_sync: true,
            run_readiness_unpause_attempt: true,
            run_update_autoplay_check: false,
            run_tick_autoplay: false,
            run_desync_correction: true,
            run_reconnect_state_restore_validation: true,
            run_state_sync_heartbeat: false,
            publish_pending_local_file_updates: true,
        },
        ConnectedSessionLoopEventKind::AutoplayTick => ConnectedSessionRuntimeStepPlan {
            run_room_pause_sync: true,
            run_readiness_unpause_attempt: false,
            run_update_autoplay_check: true,
            run_tick_autoplay: true,
            run_desync_correction: true,
            run_reconnect_state_restore_validation: true,
            run_state_sync_heartbeat: true,
            publish_pending_local_file_updates: true,
        },
        ConnectedSessionLoopEventKind::LocalInput => ConnectedSessionRuntimeStepPlan {
            run_room_pause_sync: false,
            run_readiness_unpause_attempt: false,
            run_update_autoplay_check: false,
            run_tick_autoplay: false,
            run_desync_correction: false,
            run_reconnect_state_restore_validation: true,
            run_state_sync_heartbeat: false,
            publish_pending_local_file_updates: false,
        },
    }
}

pub fn connected_session_runtime_step_actions_legacy_compatible(
    plan: ConnectedSessionRuntimeStepPlan,
    outbound_state_sync_enabled: bool,
) -> Vec<ConnectedSessionRuntimeStepAction> {
    let mut actions = Vec::new();

    if plan.run_room_pause_sync {
        actions.push(ConnectedSessionRuntimeStepAction::RunRoomPauseSync);
    }
    if plan.run_readiness_unpause_attempt {
        actions.push(ConnectedSessionRuntimeStepAction::RunReadinessUnpauseAttempt);
    }
    if plan.run_update_autoplay_check {
        actions.push(ConnectedSessionRuntimeStepAction::RunUpdateAutoplayCheck);
    }
    if plan.run_tick_autoplay {
        actions.push(ConnectedSessionRuntimeStepAction::RunTickAutoplay);
    }
    if plan.run_desync_correction {
        actions.push(ConnectedSessionRuntimeStepAction::RunDesyncCorrection);
    }
    if plan.run_reconnect_state_restore_validation {
        actions.push(ConnectedSessionRuntimeStepAction::RunReconnectStateRestoreValidation);
    }
    if plan.run_state_sync_heartbeat && outbound_state_sync_enabled {
        actions.push(ConnectedSessionRuntimeStepAction::RunStateSyncHeartbeat);
    }
    if plan.publish_pending_local_file_updates {
        actions.push(ConnectedSessionRuntimeStepAction::PublishPendingLocalFileUpdates);
    }

    actions
}

pub fn connected_session_drain_plan_legacy_compatible(
    event_kind: ConnectedSessionLoopEventKind,
    diagnostics: ConnectedSessionDiagnosticsPlan,
) -> ConnectedSessionDrainPlan {
    let flush_player_playback_diagnostics =
        diagnostics.log_player_telemetry || diagnostics.log_player_drift;

    let (
        flush_controller_auth_notifications,
        flush_chat_notifications,
        flush_user_change_notifications,
        flush_autoplay_notifications,
        flush_file_difference_notifications,
    ) = match event_kind {
        ConnectedSessionLoopEventKind::InboundMessage => (true, true, true, true, true),
        ConnectedSessionLoopEventKind::AutoplayTick => (false, false, false, true, true),
        ConnectedSessionLoopEventKind::LocalInput => (false, false, false, false, false),
    };

    ConnectedSessionDrainPlan {
        flush_player_playback_diagnostics,
        reconnect_correction_diagnostics_format: diagnostics
            .reconnect_correction_diagnostics_format,
        flush_reconnect_notifications: true,
        flush_controller_auth_notifications,
        flush_chat_notifications,
        flush_user_change_notifications,
        flush_autoplay_notifications,
        flush_file_difference_notifications,
    }
}

pub fn connected_session_drain_actions_legacy_compatible(
    plan: ConnectedSessionDrainPlan,
) -> Vec<ConnectedSessionDrainAction> {
    let mut actions = Vec::new();

    if plan.flush_player_playback_diagnostics {
        actions.push(ConnectedSessionDrainAction::FlushPlayerPlaybackDiagnostics);
    }
    if plan.flush_reconnect_notifications {
        actions.push(ConnectedSessionDrainAction::FlushReconnectNotifications);
    }
    if let Some(format) = plan.reconnect_correction_diagnostics_format {
        actions.push(ConnectedSessionDrainAction::FlushReconnectCorrectionDiagnostics(format));
    }
    if plan.flush_controller_auth_notifications {
        actions.push(ConnectedSessionDrainAction::FlushControllerAuthNotifications);
    }
    if plan.flush_chat_notifications {
        actions.push(ConnectedSessionDrainAction::FlushChatNotifications);
    }
    if plan.flush_user_change_notifications {
        actions.push(ConnectedSessionDrainAction::FlushUserChangeNotifications);
    }
    if plan.flush_autoplay_notifications {
        actions.push(ConnectedSessionDrainAction::FlushAutoplayNotifications);
    }
    if plan.flush_file_difference_notifications {
        actions.push(ConnectedSessionDrainAction::FlushFileDifferenceNotifications);
    }

    actions
}

#[cfg(test)]
mod tests {
    use super::{
        ClientNetworkLoopAttemptDisposition, ClientNetworkLoopAttemptExecutionPlan,
        ClientNetworkLoopAttemptOutcomeKind, ClientNetworkLoopAttemptPlan,
        ClientNetworkLoopAttemptSource, ClientNetworkLoopEventKind, ClientNetworkLoopEventPlan,
        ClientNetworkLoopExecutionOutcome, ClientNetworkLoopReconnectExhaustedErrorAction,
        ClientNetworkLoopReconnectExhaustedErrorKind, ClientNetworkLoopStartupPlan,
        ClientNetworkLoopStartupPlanInputs, ClientReconnectBackoffPlan, ConnectedSessionBranchPlan,
        ConnectedSessionDiagnosticsPlan, ConnectedSessionDrainAction, ConnectedSessionDrainPlan,
        ConnectedSessionEventExecutionPlan, ConnectedSessionEventExecutionPlanInputs,
        ConnectedSessionEventPlan, ConnectedSessionEventPlanInputs,
        ConnectedSessionInboundApplyPlan, ConnectedSessionInboundPostApplyAction,
        ConnectedSessionInboundPostApplyPlan, ConnectedSessionLoopEventKind,
        ConnectedSessionOuterLoopExitKind, ConnectedSessionProtocolPlan,
        ConnectedSessionRuntimeStepAction, ConnectedSessionRuntimeStepPlan,
        ConnectedSessionSharedExecutionInputs, ConnectedSessionStartupPlaylistDisposition,
        client_network_loop_attempt_disposition_for_execution_plan_legacy_compatible,
        client_network_loop_attempt_disposition_legacy_compatible,
        client_network_loop_attempt_execution_plan_for_connect_failure_legacy_compatible,
        client_network_loop_attempt_execution_plan_for_connected_session_exit_legacy_compatible,
        client_network_loop_attempt_execution_plan_for_source_legacy_compatible,
        client_network_loop_attempt_plan_for_connect_failure_legacy_compatible,
        client_network_loop_attempt_plan_for_connected_session_exit_legacy_compatible,
        client_network_loop_attempt_plan_for_source_legacy_compatible,
        client_network_loop_attempt_plan_legacy_compatible,
        client_network_loop_event_plan_legacy_compatible,
        client_network_loop_execution_outcome_legacy_compatible,
        client_network_loop_reconnect_exhausted_error_action_legacy_compatible,
        client_network_loop_startup_plan_legacy_compatible,
        client_reconnect_backoff_plan_legacy_compatible,
        connected_session_autoplay_tick_event_execution_plan_legacy_compatible,
        connected_session_branch_plan_legacy_compatible,
        connected_session_drain_actions_legacy_compatible,
        connected_session_drain_plan_legacy_compatible,
        connected_session_event_execution_plan_legacy_compatible,
        connected_session_event_plan_legacy_compatible,
        connected_session_inbound_apply_plan_legacy_compatible,
        connected_session_inbound_message_event_execution_plan_legacy_compatible,
        connected_session_inbound_post_apply_actions_legacy_compatible,
        connected_session_inbound_post_apply_plan_legacy_compatible,
        connected_session_local_input_event_execution_plan_legacy_compatible,
        connected_session_protocol_plan_legacy_compatible,
        connected_session_runtime_step_actions_legacy_compatible,
        connected_session_runtime_step_plan_legacy_compatible,
    };
    use crate::legacy_reconnect_diagnostics::ReconnectCorrectionDiagnosticsFormat;

    #[test]
    fn connected_session_drain_plan_legacy_compatible_enables_inbound_flushes() {
        let plan = connected_session_drain_plan_legacy_compatible(
            ConnectedSessionLoopEventKind::InboundMessage,
            ConnectedSessionDiagnosticsPlan {
                log_player_telemetry: false,
                log_player_drift: true,
                reconnect_correction_diagnostics_format: Some(
                    ReconnectCorrectionDiagnosticsFormat::Text,
                ),
            },
        );
        assert!(plan.flush_player_playback_diagnostics);
        assert_eq!(
            plan.reconnect_correction_diagnostics_format,
            Some(ReconnectCorrectionDiagnosticsFormat::Text)
        );
        assert!(plan.flush_reconnect_notifications);
        assert!(plan.flush_controller_auth_notifications);
        assert!(plan.flush_chat_notifications);
        assert!(plan.flush_user_change_notifications);
        assert!(plan.flush_autoplay_notifications);
        assert!(plan.flush_file_difference_notifications);
    }

    #[test]
    fn connected_session_drain_plan_legacy_compatible_specializes_tick_and_local_input() {
        let autoplay_tick_plan = connected_session_drain_plan_legacy_compatible(
            ConnectedSessionLoopEventKind::AutoplayTick,
            ConnectedSessionDiagnosticsPlan {
                log_player_telemetry: false,
                log_player_drift: false,
                reconnect_correction_diagnostics_format: None,
            },
        );
        assert!(!autoplay_tick_plan.flush_player_playback_diagnostics);
        assert!(autoplay_tick_plan.flush_autoplay_notifications);
        assert!(autoplay_tick_plan.flush_file_difference_notifications);
        assert!(!autoplay_tick_plan.flush_controller_auth_notifications);
        assert!(!autoplay_tick_plan.flush_chat_notifications);
        assert!(!autoplay_tick_plan.flush_user_change_notifications);

        let local_input_plan = connected_session_drain_plan_legacy_compatible(
            ConnectedSessionLoopEventKind::LocalInput,
            ConnectedSessionDiagnosticsPlan {
                log_player_telemetry: true,
                log_player_drift: false,
                reconnect_correction_diagnostics_format: None,
            },
        );
        assert!(local_input_plan.flush_player_playback_diagnostics);
        assert!(local_input_plan.flush_reconnect_notifications);
        assert!(!local_input_plan.flush_autoplay_notifications);
        assert!(!local_input_plan.flush_file_difference_notifications);
        assert!(!local_input_plan.flush_controller_auth_notifications);
        assert!(!local_input_plan.flush_chat_notifications);
        assert!(!local_input_plan.flush_user_change_notifications);
    }

    #[test]
    fn connected_session_drain_actions_legacy_compatible_preserves_shared_ordering() {
        let actions =
            connected_session_drain_actions_legacy_compatible(ConnectedSessionDrainPlan {
                flush_player_playback_diagnostics: true,
                reconnect_correction_diagnostics_format: Some(
                    ReconnectCorrectionDiagnosticsFormat::Text,
                ),
                flush_reconnect_notifications: true,
                flush_controller_auth_notifications: true,
                flush_chat_notifications: true,
                flush_user_change_notifications: true,
                flush_autoplay_notifications: true,
                flush_file_difference_notifications: true,
            });

        assert_eq!(
            actions,
            vec![
                ConnectedSessionDrainAction::FlushPlayerPlaybackDiagnostics,
                ConnectedSessionDrainAction::FlushReconnectNotifications,
                ConnectedSessionDrainAction::FlushReconnectCorrectionDiagnostics(
                    ReconnectCorrectionDiagnosticsFormat::Text
                ),
                ConnectedSessionDrainAction::FlushControllerAuthNotifications,
                ConnectedSessionDrainAction::FlushChatNotifications,
                ConnectedSessionDrainAction::FlushUserChangeNotifications,
                ConnectedSessionDrainAction::FlushAutoplayNotifications,
                ConnectedSessionDrainAction::FlushFileDifferenceNotifications,
            ]
        );
    }

    #[test]
    fn connected_session_drain_actions_legacy_compatible_omits_disabled_actions() {
        let actions =
            connected_session_drain_actions_legacy_compatible(ConnectedSessionDrainPlan {
                flush_player_playback_diagnostics: false,
                reconnect_correction_diagnostics_format: None,
                flush_reconnect_notifications: true,
                flush_controller_auth_notifications: false,
                flush_chat_notifications: false,
                flush_user_change_notifications: false,
                flush_autoplay_notifications: true,
                flush_file_difference_notifications: false,
            });

        assert_eq!(
            actions,
            vec![
                ConnectedSessionDrainAction::FlushReconnectNotifications,
                ConnectedSessionDrainAction::FlushAutoplayNotifications,
            ]
        );
    }

    #[test]
    fn connected_session_inbound_post_apply_actions_legacy_compatible_preserves_shared_ordering() {
        let actions = connected_session_inbound_post_apply_actions_legacy_compatible(
            ConnectedSessionInboundPostApplyPlan {
                consume_pending_ready_at_start: true,
                consume_pending_chat_message_on_connect: true,
                run_reconnect_transition: true,
                run_controller_reidentify: true,
                run_controller_auth_notifications: true,
                run_chat_notifications: true,
                run_user_change_notifications: true,
                run_reconnect_state_restore: true,
                run_reconnect_playlist_restore: true,
            },
        );

        assert_eq!(
            actions,
            vec![
                ConnectedSessionInboundPostApplyAction::ConsumePendingReadyAtStart,
                ConnectedSessionInboundPostApplyAction::ConsumePendingChatMessageOnConnect,
                ConnectedSessionInboundPostApplyAction::RunReconnectTransition,
                ConnectedSessionInboundPostApplyAction::RunControllerReidentify,
                ConnectedSessionInboundPostApplyAction::RunControllerAuthNotifications,
                ConnectedSessionInboundPostApplyAction::RunChatNotifications,
                ConnectedSessionInboundPostApplyAction::RunUserChangeNotifications,
                ConnectedSessionInboundPostApplyAction::RunReconnectStateRestore,
                ConnectedSessionInboundPostApplyAction::RunReconnectPlaylistRestore,
            ]
        );
    }

    #[test]
    fn connected_session_inbound_post_apply_actions_legacy_compatible_omits_disabled_actions() {
        let actions = connected_session_inbound_post_apply_actions_legacy_compatible(
            ConnectedSessionInboundPostApplyPlan {
                consume_pending_ready_at_start: false,
                consume_pending_chat_message_on_connect: false,
                run_reconnect_transition: true,
                run_controller_reidentify: false,
                run_controller_auth_notifications: false,
                run_chat_notifications: true,
                run_user_change_notifications: false,
                run_reconnect_state_restore: true,
                run_reconnect_playlist_restore: false,
            },
        );

        assert_eq!(
            actions,
            vec![
                ConnectedSessionInboundPostApplyAction::RunReconnectTransition,
                ConnectedSessionInboundPostApplyAction::RunChatNotifications,
                ConnectedSessionInboundPostApplyAction::RunReconnectStateRestore,
            ]
        );
    }

    #[test]
    fn connected_session_runtime_step_plan_legacy_compatible_matches_inbound_and_tick_policy() {
        let inbound_plan = connected_session_runtime_step_plan_legacy_compatible(
            ConnectedSessionLoopEventKind::InboundMessage,
        );
        assert!(inbound_plan.run_room_pause_sync);
        assert!(inbound_plan.run_readiness_unpause_attempt);
        assert!(inbound_plan.run_desync_correction);
        assert!(inbound_plan.run_reconnect_state_restore_validation);
        assert!(inbound_plan.publish_pending_local_file_updates);
        assert!(!inbound_plan.run_update_autoplay_check);
        assert!(!inbound_plan.run_tick_autoplay);
        assert!(!inbound_plan.run_state_sync_heartbeat);

        let tick_plan = connected_session_runtime_step_plan_legacy_compatible(
            ConnectedSessionLoopEventKind::AutoplayTick,
        );
        assert!(tick_plan.run_room_pause_sync);
        assert!(tick_plan.run_update_autoplay_check);
        assert!(tick_plan.run_tick_autoplay);
        assert!(tick_plan.run_desync_correction);
        assert!(tick_plan.run_reconnect_state_restore_validation);
        assert!(tick_plan.run_state_sync_heartbeat);
        assert!(tick_plan.publish_pending_local_file_updates);
        assert!(!tick_plan.run_readiness_unpause_attempt);
    }

    #[test]
    fn connected_session_runtime_step_plan_legacy_compatible_keeps_local_input_minimal() {
        let plan = connected_session_runtime_step_plan_legacy_compatible(
            ConnectedSessionLoopEventKind::LocalInput,
        );
        assert!(!plan.run_room_pause_sync);
        assert!(!plan.run_readiness_unpause_attempt);
        assert!(!plan.run_update_autoplay_check);
        assert!(!plan.run_tick_autoplay);
        assert!(!plan.run_desync_correction);
        assert!(plan.run_reconnect_state_restore_validation);
        assert!(!plan.run_state_sync_heartbeat);
        assert!(!plan.publish_pending_local_file_updates);
    }

    #[test]
    fn connected_session_runtime_step_actions_legacy_compatible_preserve_execution_order() {
        let actions = connected_session_runtime_step_actions_legacy_compatible(
            ConnectedSessionRuntimeStepPlan {
                run_room_pause_sync: true,
                run_readiness_unpause_attempt: true,
                run_update_autoplay_check: true,
                run_tick_autoplay: true,
                run_desync_correction: true,
                run_reconnect_state_restore_validation: true,
                run_state_sync_heartbeat: true,
                publish_pending_local_file_updates: true,
            },
            true,
        );

        assert_eq!(
            actions,
            vec![
                ConnectedSessionRuntimeStepAction::RunRoomPauseSync,
                ConnectedSessionRuntimeStepAction::RunReadinessUnpauseAttempt,
                ConnectedSessionRuntimeStepAction::RunUpdateAutoplayCheck,
                ConnectedSessionRuntimeStepAction::RunTickAutoplay,
                ConnectedSessionRuntimeStepAction::RunDesyncCorrection,
                ConnectedSessionRuntimeStepAction::RunReconnectStateRestoreValidation,
                ConnectedSessionRuntimeStepAction::RunStateSyncHeartbeat,
                ConnectedSessionRuntimeStepAction::PublishPendingLocalFileUpdates,
            ]
        );
    }

    #[test]
    fn connected_session_runtime_step_actions_legacy_compatible_omits_disabled_actions() {
        let actions = connected_session_runtime_step_actions_legacy_compatible(
            ConnectedSessionRuntimeStepPlan {
                run_room_pause_sync: false,
                run_readiness_unpause_attempt: false,
                run_update_autoplay_check: true,
                run_tick_autoplay: false,
                run_desync_correction: true,
                run_reconnect_state_restore_validation: true,
                run_state_sync_heartbeat: true,
                publish_pending_local_file_updates: false,
            },
            false,
        );

        assert_eq!(
            actions,
            vec![
                ConnectedSessionRuntimeStepAction::RunUpdateAutoplayCheck,
                ConnectedSessionRuntimeStepAction::RunDesyncCorrection,
                ConnectedSessionRuntimeStepAction::RunReconnectStateRestoreValidation,
            ]
        );
    }

    #[test]
    fn connected_session_inbound_post_apply_plan_legacy_compatible_matches_policy() {
        let full_plan =
            connected_session_inbound_post_apply_plan_legacy_compatible(true, true, true);
        assert!(full_plan.consume_pending_ready_at_start);
        assert!(full_plan.consume_pending_chat_message_on_connect);
        assert!(full_plan.run_reconnect_transition);
        assert!(full_plan.run_controller_reidentify);
        assert!(full_plan.run_controller_auth_notifications);
        assert!(full_plan.run_chat_notifications);
        assert!(full_plan.run_user_change_notifications);
        assert!(full_plan.run_reconnect_state_restore);
        assert!(full_plan.run_reconnect_playlist_restore);

        let reduced_plan =
            connected_session_inbound_post_apply_plan_legacy_compatible(false, false, false);
        assert!(!reduced_plan.consume_pending_ready_at_start);
        assert!(!reduced_plan.consume_pending_chat_message_on_connect);
        assert!(reduced_plan.run_reconnect_transition);
        assert!(reduced_plan.run_controller_reidentify);
        assert!(reduced_plan.run_controller_auth_notifications);
        assert!(reduced_plan.run_chat_notifications);
        assert!(reduced_plan.run_user_change_notifications);
        assert!(reduced_plan.run_reconnect_state_restore);
        assert!(!reduced_plan.run_reconnect_playlist_restore);
    }

    #[test]
    fn connected_session_inbound_apply_plan_legacy_compatible_specializes_state_messages() {
        assert_eq!(
            connected_session_inbound_apply_plan_legacy_compatible(true, false),
            ConnectedSessionInboundApplyPlan {
                reconcile_inbound_state: true,
                apply_message_json_at: false,
                outbound_state_sync_enabled: true,
            }
        );
        assert_eq!(
            connected_session_inbound_apply_plan_legacy_compatible(true, true),
            ConnectedSessionInboundApplyPlan {
                reconcile_inbound_state: true,
                apply_message_json_at: false,
                outbound_state_sync_enabled: true,
            }
        );
    }

    #[test]
    fn connected_session_inbound_apply_plan_legacy_compatible_preserves_non_state_policy() {
        assert_eq!(
            connected_session_inbound_apply_plan_legacy_compatible(false, false),
            ConnectedSessionInboundApplyPlan {
                reconcile_inbound_state: false,
                apply_message_json_at: true,
                outbound_state_sync_enabled: false,
            }
        );
        assert_eq!(
            connected_session_inbound_apply_plan_legacy_compatible(false, true),
            ConnectedSessionInboundApplyPlan {
                reconcile_inbound_state: false,
                apply_message_json_at: true,
                outbound_state_sync_enabled: true,
            }
        );
    }

    #[test]
    fn connected_session_protocol_plan_legacy_compatible_handles_inbound_playlist_policy() {
        assert_eq!(
            connected_session_protocol_plan_legacy_compatible(
                ConnectedSessionLoopEventKind::InboundMessage,
                false,
                true,
            ),
            ConnectedSessionProtocolPlan {
                flush_runtime_protocol_lines: true,
                startup_playlist_disposition:
                    ConnectedSessionStartupPlaylistDisposition::EmitIfAvailable,
            }
        );
        assert_eq!(
            connected_session_protocol_plan_legacy_compatible(
                ConnectedSessionLoopEventKind::InboundMessage,
                false,
                false,
            ),
            ConnectedSessionProtocolPlan {
                flush_runtime_protocol_lines: true,
                startup_playlist_disposition:
                    ConnectedSessionStartupPlaylistDisposition::DiscardIfPending,
            }
        );
    }

    #[test]
    fn connected_session_protocol_plan_legacy_compatible_handles_tick_and_local_input() {
        assert_eq!(
            connected_session_protocol_plan_legacy_compatible(
                ConnectedSessionLoopEventKind::AutoplayTick,
                false,
                true,
            ),
            ConnectedSessionProtocolPlan {
                flush_runtime_protocol_lines: true,
                startup_playlist_disposition:
                    ConnectedSessionStartupPlaylistDisposition::LeavePending,
            }
        );
        assert_eq!(
            connected_session_protocol_plan_legacy_compatible(
                ConnectedSessionLoopEventKind::LocalInput,
                false,
                true,
            ),
            ConnectedSessionProtocolPlan {
                flush_runtime_protocol_lines: false,
                startup_playlist_disposition:
                    ConnectedSessionStartupPlaylistDisposition::LeavePending,
            }
        );
        assert_eq!(
            connected_session_protocol_plan_legacy_compatible(
                ConnectedSessionLoopEventKind::LocalInput,
                true,
                true,
            ),
            ConnectedSessionProtocolPlan {
                flush_runtime_protocol_lines: true,
                startup_playlist_disposition:
                    ConnectedSessionStartupPlaylistDisposition::LeavePending,
            }
        );
    }

    #[test]
    fn connected_session_branch_plan_legacy_compatible_assembles_inbound_policy() {
        assert_eq!(
            connected_session_branch_plan_legacy_compatible(
                ConnectedSessionLoopEventKind::InboundMessage,
                false,
                true,
                ConnectedSessionDiagnosticsPlan {
                    log_player_telemetry: false,
                    log_player_drift: true,
                    reconnect_correction_diagnostics_format: Some(
                        ReconnectCorrectionDiagnosticsFormat::Text,
                    ),
                },
            ),
            ConnectedSessionBranchPlan {
                run_protocol_before_runtime_steps: false,
                runtime_steps: connected_session_runtime_step_plan_legacy_compatible(
                    ConnectedSessionLoopEventKind::InboundMessage,
                ),
                protocol: connected_session_protocol_plan_legacy_compatible(
                    ConnectedSessionLoopEventKind::InboundMessage,
                    false,
                    true,
                ),
                drain: connected_session_drain_plan_legacy_compatible(
                    ConnectedSessionLoopEventKind::InboundMessage,
                    ConnectedSessionDiagnosticsPlan {
                        log_player_telemetry: false,
                        log_player_drift: true,
                        reconnect_correction_diagnostics_format: Some(
                            ReconnectCorrectionDiagnosticsFormat::Text,
                        ),
                    },
                ),
            }
        );
    }

    #[test]
    fn connected_session_branch_plan_legacy_compatible_assembles_local_input_policy() {
        assert_eq!(
            connected_session_branch_plan_legacy_compatible(
                ConnectedSessionLoopEventKind::LocalInput,
                true,
                true,
                ConnectedSessionDiagnosticsPlan {
                    log_player_telemetry: true,
                    log_player_drift: false,
                    reconnect_correction_diagnostics_format: None,
                },
            ),
            ConnectedSessionBranchPlan {
                run_protocol_before_runtime_steps: true,
                runtime_steps: connected_session_runtime_step_plan_legacy_compatible(
                    ConnectedSessionLoopEventKind::LocalInput,
                ),
                protocol: connected_session_protocol_plan_legacy_compatible(
                    ConnectedSessionLoopEventKind::LocalInput,
                    true,
                    true,
                ),
                drain: connected_session_drain_plan_legacy_compatible(
                    ConnectedSessionLoopEventKind::LocalInput,
                    ConnectedSessionDiagnosticsPlan {
                        log_player_telemetry: true,
                        log_player_drift: false,
                        reconnect_correction_diagnostics_format: None,
                    },
                ),
            }
        );
    }

    #[test]
    fn connected_session_event_plan_legacy_compatible_assembles_inbound_post_apply_and_branch() {
        let diagnostics = ConnectedSessionDiagnosticsPlan {
            log_player_telemetry: false,
            log_player_drift: true,
            reconnect_correction_diagnostics_format: Some(
                ReconnectCorrectionDiagnosticsFormat::Text,
            ),
        };
        assert_eq!(
            connected_session_event_plan_legacy_compatible(
                ConnectedSessionLoopEventKind::InboundMessage,
                ConnectedSessionEventPlanInputs {
                    emitted_runtime_action: false,
                    inbound_is_server_hello: true,
                    has_pending_chat_message_on_connect: true,
                    shared_playlists_enabled: true,
                    diagnostics,
                },
            ),
            ConnectedSessionEventPlan {
                inbound_post_apply: Some(
                    connected_session_inbound_post_apply_plan_legacy_compatible(true, true, true),
                ),
                branch: connected_session_branch_plan_legacy_compatible(
                    ConnectedSessionLoopEventKind::InboundMessage,
                    false,
                    true,
                    diagnostics,
                ),
            }
        );
    }

    #[test]
    fn connected_session_event_plan_legacy_compatible_keeps_non_inbound_post_apply_empty() {
        let diagnostics = ConnectedSessionDiagnosticsPlan {
            log_player_telemetry: true,
            log_player_drift: false,
            reconnect_correction_diagnostics_format: None,
        };
        assert_eq!(
            connected_session_event_plan_legacy_compatible(
                ConnectedSessionLoopEventKind::LocalInput,
                ConnectedSessionEventPlanInputs {
                    emitted_runtime_action: true,
                    inbound_is_server_hello: false,
                    has_pending_chat_message_on_connect: true,
                    shared_playlists_enabled: true,
                    diagnostics,
                },
            ),
            ConnectedSessionEventPlan {
                inbound_post_apply: None,
                branch: connected_session_branch_plan_legacy_compatible(
                    ConnectedSessionLoopEventKind::LocalInput,
                    true,
                    true,
                    diagnostics,
                ),
            }
        );
    }

    #[test]
    fn connected_session_event_execution_plan_legacy_compatible_combines_inbound_apply_and_event() {
        let event_inputs = ConnectedSessionEventPlanInputs {
            emitted_runtime_action: false,
            inbound_is_server_hello: true,
            has_pending_chat_message_on_connect: true,
            shared_playlists_enabled: true,
            diagnostics: ConnectedSessionDiagnosticsPlan {
                log_player_telemetry: false,
                log_player_drift: true,
                reconnect_correction_diagnostics_format: Some(
                    ReconnectCorrectionDiagnosticsFormat::Text,
                ),
            },
        };
        assert_eq!(
            connected_session_event_execution_plan_legacy_compatible(
                ConnectedSessionLoopEventKind::InboundMessage,
                ConnectedSessionEventExecutionPlanInputs {
                    event: event_inputs,
                    inbound_message_is_state: true,
                    outbound_state_sync_enabled: false,
                },
            ),
            ConnectedSessionEventExecutionPlan {
                inbound_apply: Some(connected_session_inbound_apply_plan_legacy_compatible(
                    true, false
                ),),
                event: connected_session_event_plan_legacy_compatible(
                    ConnectedSessionLoopEventKind::InboundMessage,
                    event_inputs,
                ),
            }
        );
    }

    #[test]
    fn connected_session_event_execution_plan_legacy_compatible_keeps_non_inbound_apply_empty() {
        let event_inputs = ConnectedSessionEventPlanInputs {
            emitted_runtime_action: true,
            inbound_is_server_hello: false,
            has_pending_chat_message_on_connect: false,
            shared_playlists_enabled: true,
            diagnostics: ConnectedSessionDiagnosticsPlan {
                log_player_telemetry: true,
                log_player_drift: false,
                reconnect_correction_diagnostics_format: None,
            },
        };
        assert_eq!(
            connected_session_event_execution_plan_legacy_compatible(
                ConnectedSessionLoopEventKind::LocalInput,
                ConnectedSessionEventExecutionPlanInputs {
                    event: event_inputs,
                    inbound_message_is_state: true,
                    outbound_state_sync_enabled: false,
                },
            ),
            ConnectedSessionEventExecutionPlan {
                inbound_apply: None,
                event: connected_session_event_plan_legacy_compatible(
                    ConnectedSessionLoopEventKind::LocalInput,
                    event_inputs,
                ),
            }
        );
    }

    #[test]
    fn connected_session_inbound_message_event_execution_plan_legacy_compatible_packs_inputs() {
        let shared = ConnectedSessionSharedExecutionInputs {
            shared_playlists_enabled: true,
            diagnostics: ConnectedSessionDiagnosticsPlan {
                log_player_telemetry: false,
                log_player_drift: true,
                reconnect_correction_diagnostics_format: Some(
                    ReconnectCorrectionDiagnosticsFormat::Text,
                ),
            },
            outbound_state_sync_enabled: false,
        };
        assert_eq!(
            connected_session_inbound_message_event_execution_plan_legacy_compatible(
                true, true, true, shared,
            ),
            connected_session_event_execution_plan_legacy_compatible(
                ConnectedSessionLoopEventKind::InboundMessage,
                ConnectedSessionEventExecutionPlanInputs {
                    event: ConnectedSessionEventPlanInputs {
                        emitted_runtime_action: false,
                        inbound_is_server_hello: true,
                        has_pending_chat_message_on_connect: true,
                        shared_playlists_enabled: true,
                        diagnostics: shared.diagnostics,
                    },
                    inbound_message_is_state: true,
                    outbound_state_sync_enabled: false,
                },
            ),
        );
    }

    #[test]
    fn connected_session_autoplay_tick_event_execution_plan_legacy_compatible_packs_inputs() {
        let shared = ConnectedSessionSharedExecutionInputs {
            shared_playlists_enabled: true,
            diagnostics: ConnectedSessionDiagnosticsPlan {
                log_player_telemetry: true,
                log_player_drift: false,
                reconnect_correction_diagnostics_format: None,
            },
            outbound_state_sync_enabled: true,
        };
        assert_eq!(
            connected_session_autoplay_tick_event_execution_plan_legacy_compatible(shared),
            connected_session_event_execution_plan_legacy_compatible(
                ConnectedSessionLoopEventKind::AutoplayTick,
                ConnectedSessionEventExecutionPlanInputs {
                    event: ConnectedSessionEventPlanInputs {
                        emitted_runtime_action: false,
                        inbound_is_server_hello: false,
                        has_pending_chat_message_on_connect: false,
                        shared_playlists_enabled: true,
                        diagnostics: shared.diagnostics,
                    },
                    inbound_message_is_state: false,
                    outbound_state_sync_enabled: true,
                },
            ),
        );
    }

    #[test]
    fn connected_session_local_input_event_execution_plan_legacy_compatible_packs_inputs() {
        let shared = ConnectedSessionSharedExecutionInputs {
            shared_playlists_enabled: false,
            diagnostics: ConnectedSessionDiagnosticsPlan {
                log_player_telemetry: true,
                log_player_drift: true,
                reconnect_correction_diagnostics_format: None,
            },
            outbound_state_sync_enabled: true,
        };
        assert_eq!(
            connected_session_local_input_event_execution_plan_legacy_compatible(true, shared),
            connected_session_event_execution_plan_legacy_compatible(
                ConnectedSessionLoopEventKind::LocalInput,
                ConnectedSessionEventExecutionPlanInputs {
                    event: ConnectedSessionEventPlanInputs {
                        emitted_runtime_action: true,
                        inbound_is_server_hello: false,
                        has_pending_chat_message_on_connect: false,
                        shared_playlists_enabled: false,
                        diagnostics: shared.diagnostics,
                    },
                    inbound_message_is_state: false,
                    outbound_state_sync_enabled: true,
                },
            ),
        );
    }

    #[test]
    fn client_network_loop_event_plan_legacy_compatible_classifies_outer_loop_policy() {
        assert_eq!(
            client_network_loop_event_plan_legacy_compatible(
                ClientNetworkLoopEventKind::ConnectFailed
            ),
            ClientNetworkLoopEventPlan {
                return_success: false,
                run_disconnect: false,
                run_reconnect_backoff: true,
            }
        );
        assert_eq!(
            client_network_loop_event_plan_legacy_compatible(
                ClientNetworkLoopEventKind::ConnectedSessionTransportClosed
            ),
            ClientNetworkLoopEventPlan {
                return_success: false,
                run_disconnect: true,
                run_reconnect_backoff: true,
            }
        );
        assert_eq!(
            client_network_loop_event_plan_legacy_compatible(
                ClientNetworkLoopEventKind::ConnectedSessionRuntimeWindowElapsed
            ),
            ClientNetworkLoopEventPlan {
                return_success: true,
                run_disconnect: false,
                run_reconnect_backoff: false,
            }
        );
    }

    #[test]
    fn client_network_loop_attempt_plan_legacy_compatible_classifies_attempt_outcomes() {
        assert_eq!(
            client_network_loop_attempt_plan_legacy_compatible(
                ClientNetworkLoopAttemptOutcomeKind::ConnectFailed
            ),
            ClientNetworkLoopAttemptPlan {
                reset_retries_before_event: false,
                event: client_network_loop_event_plan_legacy_compatible(
                    ClientNetworkLoopEventKind::ConnectFailed,
                ),
                reconnect_exhausted_error_kind: Some(
                    ClientNetworkLoopReconnectExhaustedErrorKind::ConnectError,
                ),
            }
        );
        assert_eq!(
            client_network_loop_attempt_plan_legacy_compatible(
                ClientNetworkLoopAttemptOutcomeKind::ConnectedSessionTransportClosed
            ),
            ClientNetworkLoopAttemptPlan {
                reset_retries_before_event: true,
                event: client_network_loop_event_plan_legacy_compatible(
                    ClientNetworkLoopEventKind::ConnectedSessionTransportClosed,
                ),
                reconnect_exhausted_error_kind: Some(
                    ClientNetworkLoopReconnectExhaustedErrorKind::TransportClosedMessage,
                ),
            }
        );
        assert_eq!(
            client_network_loop_attempt_plan_legacy_compatible(
                ClientNetworkLoopAttemptOutcomeKind::ConnectedSessionRuntimeWindowElapsed
            ),
            ClientNetworkLoopAttemptPlan {
                reset_retries_before_event: true,
                event: client_network_loop_event_plan_legacy_compatible(
                    ClientNetworkLoopEventKind::ConnectedSessionRuntimeWindowElapsed,
                ),
                reconnect_exhausted_error_kind: None,
            }
        );
    }

    #[test]
    fn client_network_loop_attempt_plan_helpers_legacy_compatible_map_outer_loop_sources() {
        assert_eq!(
            client_network_loop_attempt_plan_for_connect_failure_legacy_compatible(),
            client_network_loop_attempt_plan_legacy_compatible(
                ClientNetworkLoopAttemptOutcomeKind::ConnectFailed
            )
        );
        assert_eq!(
            client_network_loop_attempt_plan_for_connected_session_exit_legacy_compatible(
                ConnectedSessionOuterLoopExitKind::TransportClosed
            ),
            client_network_loop_attempt_plan_legacy_compatible(
                ClientNetworkLoopAttemptOutcomeKind::ConnectedSessionTransportClosed
            )
        );
        assert_eq!(
            client_network_loop_attempt_plan_for_connected_session_exit_legacy_compatible(
                ConnectedSessionOuterLoopExitKind::RuntimeWindowElapsed
            ),
            client_network_loop_attempt_plan_legacy_compatible(
                ClientNetworkLoopAttemptOutcomeKind::ConnectedSessionRuntimeWindowElapsed
            )
        );
    }

    #[test]
    fn client_network_loop_attempt_plan_for_source_legacy_compatible_maps_both_sources() {
        assert_eq!(
            client_network_loop_attempt_plan_for_source_legacy_compatible(
                ClientNetworkLoopAttemptSource::ConnectFailed,
            ),
            client_network_loop_attempt_plan_for_connect_failure_legacy_compatible(),
        );
        assert_eq!(
            client_network_loop_attempt_plan_for_source_legacy_compatible(
                ClientNetworkLoopAttemptSource::ConnectedSession(
                    ConnectedSessionOuterLoopExitKind::TransportClosed,
                ),
            ),
            client_network_loop_attempt_plan_for_connected_session_exit_legacy_compatible(
                ConnectedSessionOuterLoopExitKind::TransportClosed,
            ),
        );
    }

    #[test]
    fn client_network_loop_attempt_execution_plan_for_source_legacy_compatible_pairs_source_and_plan()
     {
        assert_eq!(
            client_network_loop_attempt_execution_plan_for_source_legacy_compatible(
                ClientNetworkLoopAttemptSource::ConnectFailed,
            ),
            ClientNetworkLoopAttemptExecutionPlan {
                source: ClientNetworkLoopAttemptSource::ConnectFailed,
                attempt_plan:
                    client_network_loop_attempt_plan_for_connect_failure_legacy_compatible(),
            },
        );
    }

    #[test]
    fn client_network_loop_attempt_execution_plan_helpers_legacy_compatible_map_branch_facts() {
        assert_eq!(
            client_network_loop_attempt_execution_plan_for_connect_failure_legacy_compatible(),
            client_network_loop_attempt_execution_plan_for_source_legacy_compatible(
                ClientNetworkLoopAttemptSource::ConnectFailed,
            ),
        );
        assert_eq!(
            client_network_loop_attempt_execution_plan_for_connected_session_exit_legacy_compatible(
                ConnectedSessionOuterLoopExitKind::RuntimeWindowElapsed,
            ),
            client_network_loop_attempt_execution_plan_for_source_legacy_compatible(
                ClientNetworkLoopAttemptSource::ConnectedSession(
                    ConnectedSessionOuterLoopExitKind::RuntimeWindowElapsed,
                ),
            ),
        );
    }

    #[test]
    fn client_network_loop_execution_outcome_legacy_compatible_maps_post_execution_states() {
        assert_eq!(
            client_network_loop_execution_outcome_legacy_compatible(
                ClientNetworkLoopEventPlan {
                    return_success: true,
                    run_disconnect: false,
                    run_reconnect_backoff: false,
                },
                false,
            ),
            ClientNetworkLoopExecutionOutcome::ReturnSuccess
        );
        assert_eq!(
            client_network_loop_execution_outcome_legacy_compatible(
                ClientNetworkLoopEventPlan {
                    return_success: false,
                    run_disconnect: true,
                    run_reconnect_backoff: true,
                },
                false,
            ),
            ClientNetworkLoopExecutionOutcome::Continue
        );
        assert_eq!(
            client_network_loop_execution_outcome_legacy_compatible(
                ClientNetworkLoopEventPlan {
                    return_success: false,
                    run_disconnect: true,
                    run_reconnect_backoff: true,
                },
                true,
            ),
            ClientNetworkLoopExecutionOutcome::ReconnectExhausted
        );
    }

    #[test]
    fn client_network_loop_attempt_disposition_for_execution_plan_legacy_compatible_uses_shared_source_mapping()
     {
        let execution_plan =
            client_network_loop_attempt_execution_plan_for_source_legacy_compatible(
                ClientNetworkLoopAttemptSource::ConnectedSession(
                    ConnectedSessionOuterLoopExitKind::TransportClosed,
                ),
            );
        assert_eq!(
            client_network_loop_attempt_disposition_for_execution_plan_legacy_compatible(
                execution_plan,
                ClientNetworkLoopExecutionOutcome::ReconnectExhausted,
            ),
            ClientNetworkLoopAttemptDisposition::ReconnectExhausted(
                ClientNetworkLoopReconnectExhaustedErrorKind::TransportClosedMessage,
            ),
        );
    }

    #[test]
    fn client_network_loop_attempt_disposition_legacy_compatible_maps_return_success() {
        let plan = client_network_loop_attempt_plan_legacy_compatible(
            ClientNetworkLoopAttemptOutcomeKind::ConnectedSessionRuntimeWindowElapsed,
        );
        assert_eq!(
            client_network_loop_attempt_disposition_legacy_compatible(
                plan,
                ClientNetworkLoopExecutionOutcome::ReturnSuccess,
            ),
            ClientNetworkLoopAttemptDisposition::ReturnSuccess,
        );
    }

    #[test]
    fn client_network_loop_attempt_disposition_legacy_compatible_maps_continue() {
        let plan = client_network_loop_attempt_plan_legacy_compatible(
            ClientNetworkLoopAttemptOutcomeKind::ConnectFailed,
        );
        assert_eq!(
            client_network_loop_attempt_disposition_legacy_compatible(
                plan,
                ClientNetworkLoopExecutionOutcome::Continue,
            ),
            ClientNetworkLoopAttemptDisposition::Continue,
        );
    }

    #[test]
    fn client_network_loop_attempt_disposition_legacy_compatible_maps_exhaustion_policy() {
        let plan = client_network_loop_attempt_plan_legacy_compatible(
            ClientNetworkLoopAttemptOutcomeKind::ConnectedSessionTransportClosed,
        );
        assert_eq!(
            client_network_loop_attempt_disposition_legacy_compatible(
                plan,
                ClientNetworkLoopExecutionOutcome::ReconnectExhausted,
            ),
            ClientNetworkLoopAttemptDisposition::ReconnectExhausted(
                ClientNetworkLoopReconnectExhaustedErrorKind::TransportClosedMessage,
            ),
        );
    }

    #[test]
    fn client_network_loop_reconnect_exhausted_error_action_legacy_compatible_matches_kind() {
        assert_eq!(
            client_network_loop_reconnect_exhausted_error_action_legacy_compatible(
                ClientNetworkLoopReconnectExhaustedErrorKind::ConnectError,
            ),
            ClientNetworkLoopReconnectExhaustedErrorAction::UseConnectError,
        );
        assert_eq!(
            client_network_loop_reconnect_exhausted_error_action_legacy_compatible(
                ClientNetworkLoopReconnectExhaustedErrorKind::TransportClosedMessage,
            ),
            ClientNetworkLoopReconnectExhaustedErrorAction::StaticMessage(
                "server connection closed and reconnect retries were exhausted",
            ),
        );
    }

    #[test]
    fn client_reconnect_backoff_plan_legacy_compatible_stops_or_schedules_retry() {
        assert_eq!(
            client_reconnect_backoff_plan_legacy_compatible(2, true, Some(0.4)),
            ClientReconnectBackoffPlan {
                stop_retrying: true,
                sleep_delay_seconds: None,
                next_retries: 2,
            }
        );
        assert_eq!(
            client_reconnect_backoff_plan_legacy_compatible(2, false, None),
            ClientReconnectBackoffPlan {
                stop_retrying: false,
                sleep_delay_seconds: Some(0.1),
                next_retries: 3,
            }
        );
        assert_eq!(
            client_reconnect_backoff_plan_legacy_compatible(2, false, Some(0.4)),
            ClientReconnectBackoffPlan {
                stop_retrying: false,
                sleep_delay_seconds: Some(0.4),
                next_retries: 3,
            }
        );
    }

    #[test]
    fn client_network_loop_startup_plan_legacy_compatible_collects_startup_bootstrap_state() {
        assert_eq!(
            client_network_loop_startup_plan_legacy_compatible(
                ClientNetworkLoopStartupPlanInputs {
                    endpoint_host: "syncplay.example",
                    endpoint_port: 9001,
                    stdin_enabled: true,
                    has_legacy_overrides: true,
                    chat_message_on_connect: Some("hello"),
                    startup_playlist_file_on_connect: Some("playlist.txt"),
                }
            ),
            ClientNetworkLoopStartupPlan {
                apply_legacy_explicit_mpv_ipc_startup: true,
                spawn_local_input_receiver: true,
                endpoint: "syncplay.example:9001".to_owned(),
                chat_message_on_connect: Some("hello".to_owned()),
                startup_playlist_file_on_connect: Some("playlist.txt".to_owned()),
            }
        );
    }

    #[test]
    fn client_network_loop_startup_plan_legacy_compatible_keeps_optional_bootstrap_inputs_empty() {
        assert_eq!(
            client_network_loop_startup_plan_legacy_compatible(
                ClientNetworkLoopStartupPlanInputs {
                    endpoint_host: "127.0.0.1",
                    endpoint_port: 8999,
                    stdin_enabled: false,
                    has_legacy_overrides: false,
                    chat_message_on_connect: None,
                    startup_playlist_file_on_connect: None,
                }
            ),
            ClientNetworkLoopStartupPlan {
                apply_legacy_explicit_mpv_ipc_startup: false,
                spawn_local_input_receiver: false,
                endpoint: "127.0.0.1:8999".to_owned(),
                chat_message_on_connect: None,
                startup_playlist_file_on_connect: None,
            }
        );
    }
}
