use crate::legacy_reconnect_diagnostics::ReconnectCorrectionDiagnosticsFormat;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectedSessionLoopEventKind {
    InboundMessage,
    AutoplayTick,
    PlayerCoordinationTick,
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
    pub advance_playlist_after_natural_completion: bool,
    pub synchronize_canonical_playlist_selection: bool,
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
    AdvancePlaylistAfterNaturalCompletion,
    SynchronizeCanonicalPlaylistSelection,
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
