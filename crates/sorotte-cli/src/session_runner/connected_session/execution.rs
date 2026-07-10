use super::*;

fn flush_connected_session_branch_outputs_legacy_compatible<F, G>(
    runtime: &mut ClientApplication<MpvAdapter>,
    diagnostics_config: &ClientLoopDiagnosticsConfig,
    reconnect_correction_diagnostics_state: &mut ReconnectCorrectionDiagnosticsState,
    file_difference_state: &mut FileDifferenceNotificationState,
    plan: ConnectedSessionDrainPlan,
    notification_sink: &mut F,
    file_difference_sink: &mut G,
) -> anyhow::Result<()>
where
    F: FnMut(&AutoplayCountdownNotification) -> anyhow::Result<()>,
    G: FnMut(&str) -> anyhow::Result<()>,
{
    for action in connected_session_drain_actions_legacy_compatible(plan) {
        match action {
            ConnectedSessionDrainAction::FlushPlayerPlaybackDiagnostics => {
                flush_player_playback_telemetry_diagnostics(
                    runtime,
                    diagnostics_config.log_player_telemetry,
                    diagnostics_config.log_player_drift,
                )?;
            }
            ConnectedSessionDrainAction::FlushReconnectNotifications => {
                flush_reconnect_notifications_legacy_compatible(runtime)?;
            }
            ConnectedSessionDrainAction::FlushReconnectCorrectionDiagnostics(format) => {
                flush_reconnect_correction_diagnostics_to_sink(
                    runtime,
                    reconnect_correction_diagnostics_state,
                    &diagnostics_config.reconnect_correction_diagnostics_alert_thresholds,
                    format,
                    &mut emit_reconnect_correction_diagnostic,
                )?;
            }
            ConnectedSessionDrainAction::FlushControllerAuthNotifications => {
                flush_controller_auth_notifications_legacy_compatible(runtime)?;
            }
            ConnectedSessionDrainAction::FlushChatNotifications => {
                flush_chat_notifications_legacy_compatible(runtime)?;
            }
            ConnectedSessionDrainAction::FlushUserChangeNotifications => {
                flush_user_change_notifications_legacy_compatible(runtime)?;
            }
            ConnectedSessionDrainAction::FlushAutoplayNotifications => {
                flush_autoplay_notifications_legacy_compatible(runtime, notification_sink)?;
            }
            ConnectedSessionDrainAction::FlushFileDifferenceNotifications => {
                flush_file_difference_notifications_legacy_compatible(
                    runtime,
                    file_difference_state,
                    file_difference_sink,
                )?;
            }
        }
    }

    Ok(())
}

fn run_connected_session_inbound_post_apply_legacy_compatible(
    runtime: &mut ClientApplication<MpvAdapter>,
    pending_ready_at_start_on_server_hello: &mut Option<bool>,
    pending_chat_message_on_connect: &mut Option<String>,
    plan: ConnectedSessionInboundPostApplyPlan,
) -> anyhow::Result<()> {
    for action in connected_session_inbound_post_apply_actions_legacy_compatible(plan) {
        match action {
            ConnectedSessionInboundPostApplyAction::ConsumePendingReadyAtStart => {
                if let Some(ready_at_start) = pending_ready_at_start_on_server_hello.take() {
                    let _ = runtime.run_set_ready_for_user("", ready_at_start, false)?;
                }
            }
            ConnectedSessionInboundPostApplyAction::ConsumePendingChatMessageOnConnect => {
                if let Some(message) = pending_chat_message_on_connect.take() {
                    let _ = runtime.run_send_chat_message(message)?;
                }
            }
            ConnectedSessionInboundPostApplyAction::RunReconnectTransition => {
                runtime.run_reconnect_transition_if_needed()?;
            }
            ConnectedSessionInboundPostApplyAction::RunControllerReidentify => {
                runtime.run_controller_reidentify_if_needed()?;
            }
            ConnectedSessionInboundPostApplyAction::RunControllerAuthNotifications => {
                runtime.run_controller_auth_notifications_if_needed()?;
            }
            ConnectedSessionInboundPostApplyAction::RunChatNotifications => {
                runtime.run_chat_notifications_if_needed()?;
            }
            ConnectedSessionInboundPostApplyAction::RunUserChangeNotifications => {
                runtime.run_user_change_notifications_if_needed()?;
            }
            ConnectedSessionInboundPostApplyAction::RunReconnectStateRestore => {
                runtime.run_reconnect_state_restore_if_needed()?;
            }
            ConnectedSessionInboundPostApplyAction::RunReconnectPlaylistRestore => {
                runtime.run_reconnect_playlist_restore_if_needed()?;
            }
        }
    }

    Ok(())
}

fn apply_connected_session_inbound_message_legacy_compatible(
    application: &mut ClientApplication<MpvAdapter>,
    line: &str,
    now_seconds: f64,
    dont_slow_down_with_me: bool,
    plan: ConnectedSessionInboundApplyPlan,
) -> anyhow::Result<bool> {
    let state_sync_emitted = application.apply_protocol_line(
        line,
        now_seconds,
        plan.reconcile_inbound_state,
        dont_slow_down_with_me,
        plan.apply_message_json_at,
    )?;
    Ok(state_sync_emitted || plan.outbound_state_sync_enabled)
}

async fn apply_connected_session_protocol_plan_legacy_compatible(
    runtime: &mut ClientApplication<MpvAdapter>,
    writer: &mut ConnectedSessionWriteHalf,
    startup_playlist_file_on_connect: &mut Option<String>,
    plan: ConnectedSessionProtocolPlan,
) -> anyhow::Result<()> {
    if plan.flush_runtime_protocol_lines {
        flush_runtime_protocol_lines(runtime, writer).await?;
    }

    match plan.startup_playlist_disposition {
        ConnectedSessionStartupPlaylistDisposition::LeavePending => {}
        ConnectedSessionStartupPlaylistDisposition::EmitIfAvailable => {
            if let Some(playlist_path) = startup_playlist_file_on_connect.take() {
                let _ =
                    emit_startup_playlist_load_from_file_legacy_compatible(writer, &playlist_path)
                        .await?;
            }
        }
        ConnectedSessionStartupPlaylistDisposition::DiscardIfPending => {
            let _ = startup_playlist_file_on_connect.take();
        }
    }

    Ok(())
}

fn run_connected_session_branch_runtime_steps_legacy_compatible(
    runtime: &mut ClientApplication<MpvAdapter>,
    config: &ClientLoopConfig,
    now_seconds: f64,
    dont_slow_down_with_me: bool,
    outbound_state_sync_enabled: bool,
    plan: ConnectedSessionRuntimeStepPlan,
) -> anyhow::Result<()> {
    let actions =
        connected_session_runtime_step_actions_legacy_compatible(plan, outbound_state_sync_enabled);
    let inputs = derive_runtime_loop_inputs(runtime, config, now_seconds);

    for action in actions {
        match action {
            ConnectedSessionRuntimeStepAction::RunRoomPauseSync => {
                runtime.run_room_pause_sync_if_needed()?;
            }
            ConnectedSessionRuntimeStepAction::RunReadinessUnpauseAttempt => {
                runtime.run_readiness_unpause_attempt(
                    now_seconds,
                    inputs.readiness_supported,
                    inputs.local_can_control,
                    inputs.is_playing_music,
                )?;
            }
            ConnectedSessionRuntimeStepAction::RunUpdateAutoplayCheck => {
                runtime.update_autoplay_check(
                    inputs.readiness_supported,
                    inputs.local_can_control,
                    inputs.is_playing_music,
                    inputs.recently_advanced,
                );
            }
            ConnectedSessionRuntimeStepAction::RunTickAutoplay => {
                runtime.tick_autoplay(
                    inputs.readiness_supported,
                    inputs.local_can_control,
                    inputs.is_playing_music,
                    inputs.recently_advanced,
                )?;
            }
            ConnectedSessionRuntimeStepAction::RunDesyncCorrection => {
                runtime.run_desync_correction_if_needed(
                    now_seconds,
                    inputs.local_can_control,
                    dont_slow_down_with_me,
                    true,
                )?;
            }
            ConnectedSessionRuntimeStepAction::RunReconnectStateRestoreValidation => {
                runtime.run_reconnect_state_restore_validation_if_needed()?;
            }
            ConnectedSessionRuntimeStepAction::RunStateSyncHeartbeat => {
                let _ = runtime.run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible(
                    StatePayload::new(),
                    dont_slow_down_with_me,
                );
            }
            ConnectedSessionRuntimeStepAction::PublishPendingLocalFileUpdates => {
                publish_pending_local_file_updates(runtime, config)?;
            }
        }
    }

    Ok(())
}

async fn run_connected_session_branch_plan_legacy_compatible<F, G>(
    runtime: &mut ClientApplication<MpvAdapter>,
    now_seconds: f64,
    dont_slow_down_with_me: bool,
    outbound_state_sync_enabled: bool,
    plan: ConnectedSessionBranchPlan,
    context: ConnectedSessionBranchExecutionContext<'_, F, G>,
) -> anyhow::Result<()>
where
    F: FnMut(&AutoplayCountdownNotification) -> anyhow::Result<()>,
    G: FnMut(&str) -> anyhow::Result<()>,
{
    let ConnectedSessionBranchExecutionContext {
        config,
        writer,
        startup_playlist_file_on_connect,
        diagnostics_config,
        reconnect_correction_diagnostics_state,
        file_difference_state,
        notification_sink,
        file_difference_sink,
    } = context;
    if plan.run_protocol_before_runtime_steps {
        apply_connected_session_protocol_plan_legacy_compatible(
            runtime,
            writer,
            startup_playlist_file_on_connect,
            plan.protocol,
        )
        .await?;
    }
    run_connected_session_branch_runtime_steps_legacy_compatible(
        runtime,
        config,
        now_seconds,
        dont_slow_down_with_me,
        outbound_state_sync_enabled,
        plan.runtime_steps,
    )?;
    if !plan.run_protocol_before_runtime_steps {
        apply_connected_session_protocol_plan_legacy_compatible(
            runtime,
            writer,
            startup_playlist_file_on_connect,
            plan.protocol,
        )
        .await?;
    }
    flush_connected_session_branch_outputs_legacy_compatible(
        runtime,
        diagnostics_config,
        reconnect_correction_diagnostics_state,
        file_difference_state,
        plan.drain,
        notification_sink,
        file_difference_sink,
    )?;

    Ok(())
}

pub(super) struct ConnectedSessionBranchExecutionContext<'a, F, G>
where
    F: FnMut(&AutoplayCountdownNotification) -> anyhow::Result<()>,
    G: FnMut(&str) -> anyhow::Result<()>,
{
    pub(super) config: &'a ClientLoopConfig,
    pub(super) writer: &'a mut ConnectedSessionWriteHalf,
    pub(super) startup_playlist_file_on_connect: &'a mut Option<String>,
    pub(super) diagnostics_config: &'a ClientLoopDiagnosticsConfig,
    pub(super) reconnect_correction_diagnostics_state: &'a mut ReconnectCorrectionDiagnosticsState,
    pub(super) file_difference_state: &'a mut FileDifferenceNotificationState,
    pub(super) notification_sink: &'a mut F,
    pub(super) file_difference_sink: &'a mut G,
}

pub(super) struct ConnectedSessionEventExecutionContext<'a, F, G>
where
    F: FnMut(&AutoplayCountdownNotification) -> anyhow::Result<()>,
    G: FnMut(&str) -> anyhow::Result<()>,
{
    pub(super) pending_ready_at_start_on_server_hello: &'a mut Option<bool>,
    pub(super) pending_chat_message_on_connect: &'a mut Option<String>,
    pub(super) outbound_state_sync_enabled: &'a mut bool,
    pub(super) branch: ConnectedSessionBranchExecutionContext<'a, F, G>,
}

pub(super) async fn run_connected_session_event_plan_legacy_compatible<F, G>(
    runtime: &mut ClientApplication<MpvAdapter>,
    inbound_message_line: Option<&str>,
    now_seconds: f64,
    dont_slow_down_with_me: bool,
    event_execution_plan: ConnectedSessionEventExecutionPlan,
    context: ConnectedSessionEventExecutionContext<'_, F, G>,
) -> anyhow::Result<()>
where
    F: FnMut(&AutoplayCountdownNotification) -> anyhow::Result<()>,
    G: FnMut(&str) -> anyhow::Result<()>,
{
    let ConnectedSessionEventExecutionContext {
        pending_ready_at_start_on_server_hello,
        pending_chat_message_on_connect,
        outbound_state_sync_enabled,
        branch,
    } = context;
    if let Some(inbound_apply) = event_execution_plan.inbound_apply {
        let inbound_message_line = inbound_message_line.ok_or_else(|| {
            anyhow::anyhow!("inbound apply plan requires an inbound message line")
        })?;
        *outbound_state_sync_enabled = apply_connected_session_inbound_message_legacy_compatible(
            runtime,
            inbound_message_line,
            now_seconds,
            dont_slow_down_with_me,
            inbound_apply,
        )?;
    }
    if let Some(inbound_post_apply) = event_execution_plan.event.inbound_post_apply {
        run_connected_session_inbound_post_apply_legacy_compatible(
            runtime,
            pending_ready_at_start_on_server_hello,
            pending_chat_message_on_connect,
            inbound_post_apply,
        )?;
    }
    run_connected_session_branch_plan_legacy_compatible(
        runtime,
        now_seconds,
        dont_slow_down_with_me,
        *outbound_state_sync_enabled,
        event_execution_plan.event.branch,
        branch,
    )
    .await?;

    Ok(())
}
