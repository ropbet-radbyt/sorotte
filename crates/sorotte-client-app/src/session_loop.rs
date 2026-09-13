mod connected_session;
mod types;

pub use connected_session::{
    connected_session_autoplay_tick_event_execution_plan, connected_session_drain_actions,
    connected_session_inbound_message_event_execution_plan,
    connected_session_inbound_post_apply_actions,
    connected_session_local_input_event_execution_plan,
    connected_session_player_coordination_tick_event_execution_plan,
    connected_session_runtime_step_actions,
};
pub use types::{
    ConnectedSessionBranchPlan, ConnectedSessionDiagnosticsPlan, ConnectedSessionDrainAction,
    ConnectedSessionDrainPlan, ConnectedSessionEventExecutionPlan,
    ConnectedSessionInboundApplyPlan, ConnectedSessionInboundPostApplyAction,
    ConnectedSessionInboundPostApplyPlan, ConnectedSessionProtocolPlan,
    ConnectedSessionRuntimeStepAction, ConnectedSessionRuntimeStepPlan,
    ConnectedSessionSharedExecutionInputs, ConnectedSessionStartupPlaylistDisposition,
};

#[cfg(test)]
pub use connected_session::{
    connected_session_branch_plan, connected_session_drain_plan,
    connected_session_event_execution_plan, connected_session_event_plan,
    connected_session_inbound_apply_plan, connected_session_inbound_post_apply_plan,
    connected_session_protocol_plan, connected_session_runtime_step_plan,
};
#[cfg(test)]
pub use types::{
    ConnectedSessionEventExecutionPlanInputs, ConnectedSessionEventPlan,
    ConnectedSessionEventPlanInputs, ConnectedSessionLoopEventKind,
};

#[cfg(test)]
mod tests;
