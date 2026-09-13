use crate::reconnect_diagnostics::ReconnectCorrectionDiagnosticsFormat;

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
pub struct ConnectedSessionSharedExecutionInputs {
    pub shared_playlists_enabled: bool,
    pub diagnostics: ConnectedSessionDiagnosticsPlan,
    pub outbound_state_sync_enabled: bool,
}
