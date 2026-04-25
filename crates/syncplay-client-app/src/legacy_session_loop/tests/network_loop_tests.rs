use super::super::*;

#[test]
fn client_network_loop_event_plan_legacy_compatible_classifies_outer_loop_policy() {
    assert_eq!(
        client_network_loop_event_plan_legacy_compatible(ClientNetworkLoopEventKind::ConnectFailed),
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
fn client_network_loop_attempt_execution_plan_for_source_legacy_compatible_pairs_source_and_plan() {
    assert_eq!(
        client_network_loop_attempt_execution_plan_for_source_legacy_compatible(
            ClientNetworkLoopAttemptSource::ConnectFailed,
        ),
        ClientNetworkLoopAttemptExecutionPlan {
            source: ClientNetworkLoopAttemptSource::ConnectFailed,
            attempt_plan: client_network_loop_attempt_plan_for_connect_failure_legacy_compatible(),
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
    let execution_plan = client_network_loop_attempt_execution_plan_for_source_legacy_compatible(
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
fn client_network_loop_attempt_disposition_legacy_compatible_handles_missing_exhaustion_policy() {
    let plan = client_network_loop_attempt_plan_legacy_compatible(
        ClientNetworkLoopAttemptOutcomeKind::ConnectedSessionRuntimeWindowElapsed,
    );
    assert_eq!(
        client_network_loop_attempt_disposition_legacy_compatible(
            plan,
            ClientNetworkLoopExecutionOutcome::ReconnectExhausted,
        ),
        ClientNetworkLoopAttemptDisposition::ReturnSuccess,
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
        client_network_loop_startup_plan_legacy_compatible(ClientNetworkLoopStartupPlanInputs {
            endpoint_host: "syncplay.example",
            endpoint_port: 9001,
            stdin_enabled: true,
            has_legacy_overrides: true,
            chat_message_on_connect: Some("hello"),
            startup_playlist_file_on_connect: Some("playlist.txt"),
        }),
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
        client_network_loop_startup_plan_legacy_compatible(ClientNetworkLoopStartupPlanInputs {
            endpoint_host: "127.0.0.1",
            endpoint_port: 8999,
            stdin_enabled: false,
            has_legacy_overrides: false,
            chat_message_on_connect: None,
            startup_playlist_file_on_connect: None,
        }),
        ClientNetworkLoopStartupPlan {
            apply_legacy_explicit_mpv_ipc_startup: false,
            spawn_local_input_receiver: false,
            endpoint: "127.0.0.1:8999".to_owned(),
            chat_message_on_connect: None,
            startup_playlist_file_on_connect: None,
        }
    );
}
