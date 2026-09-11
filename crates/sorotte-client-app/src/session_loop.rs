mod connected_session;
mod network_loop;
mod types;

pub use connected_session::{
    connected_session_autoplay_tick_event_execution_plan, connected_session_drain_actions,
    connected_session_inbound_message_event_execution_plan,
    connected_session_inbound_post_apply_actions,
    connected_session_local_input_event_execution_plan,
    connected_session_player_coordination_tick_event_execution_plan,
    connected_session_runtime_step_actions,
};
pub use network_loop::{
    client_network_loop_attempt_disposition_for_execution_plan,
    client_network_loop_attempt_execution_plan_for_connect_failure,
    client_network_loop_attempt_execution_plan_for_connected_session_exit,
    client_network_loop_execution_outcome, client_network_loop_reconnect_exhausted_error_action,
    client_network_loop_startup_plan, client_reconnect_backoff_plan,
};
pub use types::{
    ClientNetworkLoopAttemptDisposition, ClientNetworkLoopAttemptExecutionPlan,
    ClientNetworkLoopAttemptPlan, ClientNetworkLoopEventPlan, ClientNetworkLoopExecutionOutcome,
    ClientNetworkLoopReconnectExhaustedErrorAction, ClientNetworkLoopReconnectExhaustedErrorKind,
    ClientNetworkLoopStartupPlan, ClientNetworkLoopStartupPlanInputs, ClientReconnectBackoffPlan,
    ConnectedSessionBranchPlan, ConnectedSessionDiagnosticsPlan, ConnectedSessionDrainAction,
    ConnectedSessionDrainPlan, ConnectedSessionEventExecutionPlan,
    ConnectedSessionInboundApplyPlan, ConnectedSessionInboundPostApplyAction,
    ConnectedSessionInboundPostApplyPlan, ConnectedSessionOuterLoopExitKind,
    ConnectedSessionProtocolPlan, ConnectedSessionRuntimeStepAction,
    ConnectedSessionRuntimeStepPlan, ConnectedSessionSharedExecutionInputs,
    ConnectedSessionStartupPlaylistDisposition,
};

#[cfg(test)]
pub use connected_session::{
    connected_session_branch_plan, connected_session_drain_plan,
    connected_session_event_execution_plan, connected_session_event_plan,
    connected_session_inbound_apply_plan, connected_session_inbound_post_apply_plan,
    connected_session_protocol_plan, connected_session_runtime_step_plan,
};
#[cfg(test)]
pub use network_loop::{
    client_network_loop_attempt_disposition, client_network_loop_attempt_execution_plan_for_source,
    client_network_loop_attempt_plan, client_network_loop_attempt_plan_for_connect_failure,
    client_network_loop_attempt_plan_for_connected_session_exit,
    client_network_loop_attempt_plan_for_source, client_network_loop_event_plan,
};
#[cfg(test)]
pub use types::{
    ClientNetworkLoopAttemptOutcomeKind, ClientNetworkLoopAttemptSource,
    ClientNetworkLoopEventKind, ConnectedSessionEventExecutionPlanInputs,
    ConnectedSessionEventPlan, ConnectedSessionEventPlanInputs, ConnectedSessionLoopEventKind,
};

#[cfg(test)]
mod tests;
