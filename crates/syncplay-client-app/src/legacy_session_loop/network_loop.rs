use super::types::{
    ClientNetworkLoopAttemptDisposition, ClientNetworkLoopAttemptExecutionPlan,
    ClientNetworkLoopAttemptOutcomeKind, ClientNetworkLoopAttemptPlan,
    ClientNetworkLoopAttemptSource, ClientNetworkLoopEventKind, ClientNetworkLoopEventPlan,
    ClientNetworkLoopExecutionOutcome, ClientNetworkLoopReconnectExhaustedErrorAction,
    ClientNetworkLoopReconnectExhaustedErrorKind, ClientNetworkLoopStartupPlan,
    ClientNetworkLoopStartupPlanInputs, ClientReconnectBackoffPlan,
    ConnectedSessionOuterLoopExitKind,
};

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
