mod connected_session;
mod network_loop;
mod types;

pub use connected_session::{
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
pub use network_loop::{
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
};
pub use types::{
    ClientNetworkLoopAttemptDisposition, ClientNetworkLoopAttemptExecutionPlan,
    ClientNetworkLoopAttemptOutcomeKind, ClientNetworkLoopAttemptPlan,
    ClientNetworkLoopAttemptSource, ClientNetworkLoopEventKind, ClientNetworkLoopEventPlan,
    ClientNetworkLoopExecutionOutcome, ClientNetworkLoopReconnectExhaustedErrorAction,
    ClientNetworkLoopReconnectExhaustedErrorKind, ClientNetworkLoopStartupPlan,
    ClientNetworkLoopStartupPlanInputs, ClientReconnectBackoffPlan, ConnectedSessionBranchPlan,
    ConnectedSessionDiagnosticsPlan, ConnectedSessionDrainAction, ConnectedSessionDrainPlan,
    ConnectedSessionEventExecutionPlan, ConnectedSessionEventExecutionPlanInputs,
    ConnectedSessionEventPlan, ConnectedSessionEventPlanInputs, ConnectedSessionInboundApplyPlan,
    ConnectedSessionInboundPostApplyAction, ConnectedSessionInboundPostApplyPlan,
    ConnectedSessionLoopEventKind, ConnectedSessionOuterLoopExitKind, ConnectedSessionProtocolPlan,
    ConnectedSessionRuntimeStepAction, ConnectedSessionRuntimeStepPlan,
    ConnectedSessionSharedExecutionInputs, ConnectedSessionStartupPlaylistDisposition,
};

#[cfg(test)]
mod tests;
