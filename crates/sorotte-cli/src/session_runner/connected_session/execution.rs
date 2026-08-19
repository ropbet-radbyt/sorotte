use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadyAtStartDisposition {
    AwaitCanonicalV2Membership,
    ConsumeWithoutMutation,
    Apply(bool),
}

fn ready_at_start_disposition(
    session: &sorotte_client_core::ClientSession,
    pending: PendingReadyAtStart,
) -> ReadyAtStartDisposition {
    if !session.server_readiness_v2_supported() {
        return ReadyAtStartDisposition::Apply(pending.desired);
    }
    if pending.had_current_v2_membership || session.pending_readiness_intent().is_some() {
        return ReadyAtStartDisposition::ConsumeWithoutMutation;
    }

    let Some(username) = session.username() else {
        return ReadyAtStartDisposition::AwaitCanonicalV2Membership;
    };
    let Some(participant) = session.canonical_participant_readiness(username) else {
        return ReadyAtStartDisposition::AwaitCanonicalV2Membership;
    };

    if pending.desired
        && participant.user_intent == sorotte_protocol::UserReadinessIntent::NotReady
        && participant.user_intent_source
            == sorotte_protocol::ReadinessMutationSource::Initialization
    {
        ReadyAtStartDisposition::Apply(true)
    } else {
        // V2 defaults to NotReady at membership creation. Sending the implicit
        // false startup value would manufacture a user mutation and revision.
        // A canonical non-initialization record is acknowledged user intent and
        // therefore also wins over this startup preference on reconnect.
        ReadyAtStartDisposition::ConsumeWithoutMutation
    }
}

struct ConnectedSessionBranchOutputState<'a> {
    reconnect_correction_diagnostics: &'a mut ReconnectCorrectionDiagnosticsState,
    seek_preparation_notifications: &'a mut SeekPreparationNotificationState,
    readiness_notifications: &'a mut ReadinessNotificationState,
    file_difference_notifications: &'a mut FileDifferenceNotificationState,
}

fn flush_connected_session_branch_outputs_legacy_compatible<F, G>(
    runtime: &mut ClientApplication<MpvAdapter>,
    diagnostics_config: &ClientLoopDiagnosticsConfig,
    output_state: ConnectedSessionBranchOutputState<'_>,
    plan: ConnectedSessionDrainPlan,
    notification_sink: &mut F,
    file_difference_sink: &mut G,
) -> anyhow::Result<()>
where
    F: FnMut(&AutoplayCountdownNotification) -> anyhow::Result<()>,
    G: FnMut(&str) -> anyhow::Result<()>,
{
    let ConnectedSessionBranchOutputState {
        reconnect_correction_diagnostics,
        seek_preparation_notifications,
        readiness_notifications,
        file_difference_notifications,
    } = output_state;
    // Seek-preparation is an ordinary user-visible lifecycle, not verbose
    // telemetry. Project changed states on every branch so the recovery
    // controls remain discoverable with the default diagnostics settings.
    flush_seek_preparation_notifications(runtime, seek_preparation_notifications);
    flush_readiness_status_notifications(runtime, readiness_notifications);
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
                    reconnect_correction_diagnostics,
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
                    file_difference_notifications,
                    file_difference_sink,
                )?;
            }
        }
    }

    Ok(())
}

fn run_connected_session_inbound_post_apply_legacy_compatible<P>(
    runtime: &mut ClientApplication<P>,
    pending_ready_at_start_on_server_hello: &mut Option<PendingReadyAtStart>,
    pending_chat_message_on_connect: &mut Option<String>,
    now_seconds: f64,
    plan: ConnectedSessionInboundPostApplyPlan,
) -> Option<ContainedConnectedSessionPlayerFailure>
where
    P: sorotte_player_api::PlayerAdapter,
{
    for action in connected_session_inbound_post_apply_actions_legacy_compatible(plan) {
        let (operation, outcome) = match action {
            ConnectedSessionInboundPostApplyAction::ConsumePendingReadyAtStart => {
                if let Some(pending) = *pending_ready_at_start_on_server_hello {
                    match ready_at_start_disposition(runtime.session(), pending) {
                        ReadyAtStartDisposition::AwaitCanonicalV2Membership => {}
                        ReadyAtStartDisposition::ConsumeWithoutMutation => {
                            *pending_ready_at_start_on_server_hello = None;
                        }
                        ReadyAtStartDisposition::Apply(ready_at_start) => {
                            *pending_ready_at_start_on_server_hello = None;
                            if let Err(error) = runtime.run_initial_readiness_intent(ready_at_start)
                            {
                                return Some(contain_connected_session_player_failure(
                                    runtime,
                                    now_seconds,
                                    "apply initial readiness intent",
                                    error.into(),
                                ));
                            }
                        }
                    }
                }
                ("apply initial readiness intent", Ok(()))
            }
            ConnectedSessionInboundPostApplyAction::ConsumePendingChatMessageOnConnect => {
                if let Some(message) = pending_chat_message_on_connect.take()
                    && let Err(error) = runtime.run_send_chat_message(message)
                {
                    return Some(contain_connected_session_player_failure(
                        runtime,
                        now_seconds,
                        "send initial chat message",
                        error.into(),
                    ));
                }
                ("send initial chat message", Ok(()))
            }
            ConnectedSessionInboundPostApplyAction::RunReconnectTransition => (
                "apply reconnect transition",
                runtime
                    .run_reconnect_transition_if_needed()
                    .map_err(anyhow::Error::from),
            ),
            ConnectedSessionInboundPostApplyAction::RunControllerReidentify => (
                "reidentify room controller",
                runtime
                    .run_controller_reidentify_if_needed()
                    .map_err(anyhow::Error::from),
            ),
            ConnectedSessionInboundPostApplyAction::RunControllerAuthNotifications => (
                "publish controller authentication notification",
                runtime
                    .run_controller_auth_notifications_if_needed_at(now_seconds)
                    .map_err(anyhow::Error::from),
            ),
            ConnectedSessionInboundPostApplyAction::RunChatNotifications => (
                "publish chat notification",
                runtime
                    .run_chat_notifications_if_needed()
                    .map_err(anyhow::Error::from),
            ),
            ConnectedSessionInboundPostApplyAction::RunUserChangeNotifications => (
                "publish user-change notification",
                runtime
                    .run_user_change_notifications_if_needed()
                    .map_err(anyhow::Error::from),
            ),
            ConnectedSessionInboundPostApplyAction::RunReconnectStateRestore => (
                "restore player state after reconnect",
                runtime
                    .run_reconnect_state_restore_if_needed()
                    .map_err(anyhow::Error::from),
            ),
            ConnectedSessionInboundPostApplyAction::RunReconnectPlaylistRestore => (
                "restore player playlist after reconnect",
                runtime
                    .run_reconnect_playlist_restore_if_needed()
                    .map_err(anyhow::Error::from),
            ),
        };
        if let Err(error) = outcome {
            return Some(contain_connected_session_player_failure(
                runtime,
                now_seconds,
                operation,
                error,
            ));
        }
    }

    None
}

fn apply_connected_session_inbound_message_legacy_compatible<P>(
    application: &mut ClientApplication<P>,
    line: &str,
    now_seconds: f64,
    dont_slow_down_with_me: bool,
    plan: ConnectedSessionInboundApplyPlan,
) -> anyhow::Result<ConnectedSessionInboundApplyOutcome>
where
    P: sorotte_player_api::PlayerAdapter,
{
    let ping_received_at_seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0);
    let outcome = application.apply_protocol_line_prefix_at_clocks(
        line,
        now_seconds,
        ping_received_at_seconds,
        plan.reconcile_inbound_state,
        dont_slow_down_with_me,
        plan.apply_message_json_at,
    )?;
    Ok(ConnectedSessionInboundApplyOutcome {
        outbound_state_sync_enabled: outcome.state_sync_emitted || plan.outbound_state_sync_enabled,
        applied_message_count: outcome.applied_message_count,
        trailing_decode_error: outcome.trailing_decode_error,
    })
}

struct ConnectedSessionInboundApplyOutcome {
    outbound_state_sync_enabled: bool,
    applied_message_count: usize,
    trailing_decode_error: Option<ProtocolError>,
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
                let _ = emit_startup_playlist_load_from_file_legacy_compatible(
                    runtime,
                    writer,
                    &playlist_path,
                )
                .await?;
            }
        }
        ConnectedSessionStartupPlaylistDisposition::DiscardIfPending => {
            let _ = startup_playlist_file_on_connect.take();
        }
    }

    Ok(())
}

fn synchronize_connected_session_player_availability<P>(
    runtime: &mut ClientApplication<P>,
    now_seconds: f64,
) -> Result<bool, sorotte_player_api::PlayerError>
where
    P: sorotte_player_api::PlayerAdapter,
{
    runtime.synchronize_player_availability(now_seconds)
}

pub(super) struct ContainedConnectedSessionPlayerFailure {
    operation: &'static str,
    error: anyhow::Error,
    status_publish_error: Option<sorotte_player_api::PlayerError>,
}

pub(super) fn contain_connected_session_player_failure<P>(
    runtime: &mut ClientApplication<P>,
    now_seconds: f64,
    operation: &'static str,
    error: anyhow::Error,
) -> ContainedConnectedSessionPlayerFailure
where
    P: sorotte_player_api::PlayerAdapter,
{
    let transport_disconnected = runtime.player().transport_is_connected() == Some(false);
    let reported_not_connected = error
        .downcast_ref::<sorotte_player_api::PlayerError>()
        .is_some_and(|error| matches!(error, sorotte_player_api::PlayerError::NotConnected));
    let availability = if transport_disconnected || reported_not_connected {
        sorotte_client_core::ExternalPlayerAvailability::Disconnected
    } else {
        sorotte_client_core::ExternalPlayerAvailability::Failed
    };
    let status_publish_error = runtime
        .record_contained_external_player_failure(availability, now_seconds)
        .err();
    ContainedConnectedSessionPlayerFailure {
        operation,
        error,
        status_publish_error,
    }
}

pub(super) fn run_contained_planned_local_runtime_action(
    runtime: &mut ClientApplication<MpvAdapter>,
    user_offset_seconds: &mut f64,
    now_seconds: f64,
    action: PlannedLocalRuntimeAction,
) -> anyhow::Result<(bool, Option<ContainedConnectedSessionPlayerFailure>)> {
    let player_bound = planned_local_runtime_action_is_player_bound(&action);
    let result = run_planned_local_runtime_action_legacy_compatible(
        runtime,
        user_offset_seconds,
        now_seconds,
        action,
    );
    contain_planned_local_runtime_action_result(runtime, now_seconds, player_bound, result)
}

fn planned_local_runtime_action_is_player_bound(action: &PlannedLocalRuntimeAction) -> bool {
    matches!(
        action,
        PlannedLocalRuntimeAction::UndoSeek
            | PlannedLocalRuntimeAction::KeepWaitingForSeekPreparation
            | PlannedLocalRuntimeAction::JoinNearestBufferedSeekPreparation
            | PlannedLocalRuntimeAction::CancelSeekPreparation
            | PlannedLocalRuntimeAction::SeekToPosition(_)
            | PlannedLocalRuntimeAction::SeekByOffset(_)
            | PlannedLocalRuntimeAction::Play
            | PlannedLocalRuntimeAction::Pause
            | PlannedLocalRuntimeAction::TogglePause
    )
}

fn contain_planned_local_runtime_action_result<P>(
    runtime: &mut ClientApplication<P>,
    now_seconds: f64,
    player_bound: bool,
    result: anyhow::Result<bool>,
) -> anyhow::Result<(bool, Option<ContainedConnectedSessionPlayerFailure>)>
where
    P: sorotte_player_api::PlayerAdapter,
{
    match result {
        Ok(emitted) => Ok((emitted, None)),
        Err(error) if player_bound => Ok((
            false,
            Some(contain_connected_session_player_failure(
                runtime,
                now_seconds,
                "apply local player command",
                error,
            )),
        )),
        Err(error) => Err(error),
    }
}

pub(super) fn report_contained_connected_session_player_failure(
    failure: &ContainedConnectedSessionPlayerFailure,
) {
    let operation = failure.operation;
    let error = &failure.error;
    eprintln!(
        "warning: external player step '{operation}' failed while the Sorotte session remains connected: {error}"
    );
    if let Some(error) = failure.status_publish_error.as_ref() {
        eprintln!(
            "warning: could not immediately publish the external-player failure status: {error}"
        );
    }
}

fn run_connected_session_branch_runtime_steps_legacy_compatible(
    runtime: &mut ClientApplication<MpvAdapter>,
    config: &ClientLoopConfig,
    network_options_health_reporter: &mut CliNetworkOptionsHealthReporter,
    now_seconds: f64,
    dont_slow_down_with_me: bool,
    outbound_state_sync_enabled: bool,
    plan: ConnectedSessionRuntimeStepPlan,
) -> Option<ContainedConnectedSessionPlayerFailure> {
    // Reconnection/lease maintenance can change the attachment state and
    // produce the first sample for a replacement player. Observe that
    // transition before opening the lifecycle fence for telemetry.
    runtime.with_player_io(|player| player.maintain_runtime_integrations());
    if let Err(error) = synchronize_connected_session_player_availability(runtime, now_seconds) {
        return Some(contain_connected_session_player_failure(
            runtime,
            now_seconds,
            "synchronize player availability",
            error.into(),
        ));
    }
    let actions =
        connected_session_runtime_step_actions_legacy_compatible(plan, outbound_state_sync_enabled);
    let inputs = derive_runtime_loop_inputs(runtime, config, now_seconds);

    for action in actions {
        let (operation, outcome) = match action {
            ConnectedSessionRuntimeStepAction::RunRoomPauseSync => (
                "synchronize room pause state",
                runtime
                    .run_room_pause_sync_if_needed_at(now_seconds)
                    .map_err(anyhow::Error::from),
            ),
            ConnectedSessionRuntimeStepAction::RunReadinessUnpauseAttempt => (
                "apply readiness unpause",
                runtime
                    .run_readiness_unpause_attempt(
                        now_seconds,
                        inputs.readiness_supported,
                        inputs.local_can_control,
                        inputs.is_playing_music,
                    )
                    .map_err(anyhow::Error::from),
            ),
            ConnectedSessionRuntimeStepAction::RunUpdateAutoplayCheck => {
                runtime.update_autoplay_check(
                    inputs.readiness_supported,
                    inputs.local_can_control,
                    inputs.is_playing_music,
                    inputs.recently_advanced,
                );
                ("update autoplay", Ok(()))
            }
            ConnectedSessionRuntimeStepAction::RunTickAutoplay => (
                "advance autoplay",
                runtime
                    .tick_autoplay(
                        inputs.readiness_supported,
                        inputs.local_can_control,
                        inputs.is_playing_music,
                        inputs.recently_advanced,
                    )
                    .map_err(anyhow::Error::from),
            ),
            ConnectedSessionRuntimeStepAction::RunDesyncCorrection => (
                "apply desync correction",
                runtime
                    .run_desync_correction_if_needed(
                        now_seconds,
                        inputs.local_can_control,
                        dont_slow_down_with_me,
                        true,
                    )
                    .map_err(anyhow::Error::from),
            ),
            ConnectedSessionRuntimeStepAction::RunReconnectStateRestoreValidation => (
                "validate player state after reconnect",
                runtime
                    .run_reconnect_state_restore_validation_if_needed_at(now_seconds)
                    .map_err(anyhow::Error::from),
            ),
            ConnectedSessionRuntimeStepAction::RunStateSyncHeartbeat => {
                if outbound_state_sync_enabled {
                    let _ = runtime
                        .run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible_at(
                            StatePayload::new(),
                            dont_slow_down_with_me,
                            now_seconds,
                        );
                    ("publish state heartbeat", Ok(()))
                } else {
                    let _ = runtime.run_participant_status_heartbeat(now_seconds);
                    ("publish participant status heartbeat", Ok(()))
                }
            }
            ConnectedSessionRuntimeStepAction::PublishPendingLocalFileUpdates => (
                "publish local file update",
                publish_pending_local_file_updates(
                    runtime,
                    config,
                    network_options_health_reporter,
                ),
            ),
        };
        if let Err(error) = outcome {
            return Some(contain_connected_session_player_failure(
                runtime,
                now_seconds,
                operation,
                error,
            ));
        }
    }

    None
}

async fn run_connected_session_branch_plan_legacy_compatible<F, G>(
    runtime: &mut ClientApplication<MpvAdapter>,
    now_seconds: f64,
    dont_slow_down_with_me: bool,
    outbound_state_sync_enabled: bool,
    plan: ConnectedSessionBranchPlan,
    prior_player_failure: Option<ContainedConnectedSessionPlayerFailure>,
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
        seek_preparation_notification_state,
        readiness_notification_state,
        file_difference_state,
        network_options_health_reporter,
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
    let player_failure = prior_player_failure.or_else(|| {
        run_connected_session_branch_runtime_steps_legacy_compatible(
            runtime,
            config,
            network_options_health_reporter,
            now_seconds,
            dont_slow_down_with_me,
            outbound_state_sync_enabled,
            plan.runtime_steps,
        )
    });
    if !plan.run_protocol_before_runtime_steps {
        apply_connected_session_protocol_plan_legacy_compatible(
            runtime,
            writer,
            startup_playlist_file_on_connect,
            plan.protocol,
        )
        .await?;
    }
    if player_failure.is_some() {
        // A player fault is not a Sorotte transport fault. Always give the
        // freshly queued advisory status and its steady heartbeat a write
        // opportunity even on plans
        // whose ordinary protocol flush happened before runtime work.
        let _ = runtime.run_participant_status_heartbeat(now_seconds);
        flush_runtime_protocol_lines(runtime, writer).await?;
    }
    flush_connected_session_branch_outputs_legacy_compatible(
        runtime,
        diagnostics_config,
        ConnectedSessionBranchOutputState {
            reconnect_correction_diagnostics: reconnect_correction_diagnostics_state,
            seek_preparation_notifications: seek_preparation_notification_state,
            readiness_notifications: readiness_notification_state,
            file_difference_notifications: file_difference_state,
        },
        plan.drain,
        notification_sink,
        file_difference_sink,
    )?;

    if let Some(failure) = player_failure.as_ref() {
        report_contained_connected_session_player_failure(failure);
    }

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
    pub(super) seek_preparation_notification_state: &'a mut SeekPreparationNotificationState,
    pub(super) readiness_notification_state: &'a mut ReadinessNotificationState,
    pub(super) file_difference_state: &'a mut FileDifferenceNotificationState,
    pub(super) network_options_health_reporter: &'a mut CliNetworkOptionsHealthReporter,
    pub(super) notification_sink: &'a mut F,
    pub(super) file_difference_sink: &'a mut G,
}

pub(super) struct ConnectedSessionEventExecutionContext<'a, F, G>
where
    F: FnMut(&AutoplayCountdownNotification) -> anyhow::Result<()>,
    G: FnMut(&str) -> anyhow::Result<()>,
{
    pub(super) pending_ready_at_start_on_server_hello: &'a mut Option<PendingReadyAtStart>,
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
    let mut trailing_decode_error = None;
    if let Some(inbound_apply) = event_execution_plan.inbound_apply {
        let inbound_message_line = inbound_message_line.ok_or_else(|| {
            anyhow::anyhow!("inbound apply plan requires an inbound message line")
        })?;
        let outcome = apply_connected_session_inbound_message_legacy_compatible(
            runtime,
            inbound_message_line,
            now_seconds,
            dont_slow_down_with_me,
            inbound_apply,
        )?;
        let ConnectedSessionInboundApplyOutcome {
            outbound_state_sync_enabled: next_outbound_state_sync_enabled,
            applied_message_count,
            trailing_decode_error: outcome_trailing_decode_error,
        } = outcome;
        *outbound_state_sync_enabled = next_outbound_state_sync_enabled;
        match (applied_message_count, outcome_trailing_decode_error) {
            (0, Some(error)) => return Err(error.into()),
            (_, error) => trailing_decode_error = error,
        }
    }
    let player_failure =
        event_execution_plan
            .event
            .inbound_post_apply
            .and_then(|inbound_post_apply| {
                run_connected_session_inbound_post_apply_legacy_compatible(
                    runtime,
                    pending_ready_at_start_on_server_hello,
                    pending_chat_message_on_connect,
                    now_seconds,
                    inbound_post_apply,
                )
            });
    run_connected_session_branch_plan_legacy_compatible(
        runtime,
        now_seconds,
        dont_slow_down_with_me,
        *outbound_state_sync_enabled,
        event_execution_plan.event.branch,
        player_failure,
        branch,
    )
    .await?;

    if let Some(error) = trailing_decode_error {
        return Err(error.into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use std::time::Duration;

    use tokio::io::AsyncBufReadExt;

    use sorotte_client_core::{
        ClientSession, ExternalPlayerAvailability, LogicalMediaId, MediaTransportKind,
        PlaybackBarrierStartConfig,
    };
    use sorotte_player_api::{
        DisconnectedPlayer, PlayerAdapter, PlayerMediaGeneration, PlayerObservationTimestamp,
        PlayerTransportPhase, PlayerTransportTelemetryUpdate,
    };
    use sorotte_protocol::{
        DirectReadinessSurface, ParticipantPlayerConnection, ParticipantReadinessUpdate,
        ParticipantStatusReport, PlaybackBarrierPolicy, PlaybackBarrierRequestResultPayload,
        PlaybackBarrierSetExtension, ReadinessMutationSource, ReadinessSetExtension,
        RoomPauseOwner, RoomReadinessSnapshot, RoomStartGatePhase, SetPayload,
        StartParticipationRole, TechnicalPlayabilityPhase, TechnicalPlayabilitySummary,
        UserReadinessIntent,
    };

    struct LifecyclePlayer {
        connected: Arc<AtomicBool>,
    }

    impl PlayerAdapter for LifecyclePlayer {
        fn name(&self) -> &'static str {
            "cli-lifecycle-test"
        }

        fn transport_is_connected(&self) -> Option<bool> {
            Some(self.connected.load(Ordering::SeqCst))
        }
    }

    fn take_participant_status_reports<P>(
        application: &mut ClientApplication<P>,
    ) -> Vec<ParticipantStatusReport>
    where
        P: PlayerAdapter,
    {
        let mut reports = Vec::new();
        while let Some(pending) = application
            .pending_protocol_line()
            .expect("participant status should encode")
        {
            let message = application
                .acknowledge_protocol_line(pending.lease())
                .expect("acknowledging a pending line should return its message");
            if let ProtocolMessage::State(state) = message
                && let Some(report) = state
                    .state
                    .participant_status_v1()
                    .expect("participant-status extension should decode")
                    .and_then(|extension| extension.report)
            {
                reports.push(report);
            }
        }
        reports
    }

    #[test]
    fn local_player_command_failure_is_contained_without_leaving_the_room() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room"},"version":"1.7.5","features":{"sorotteParticipantStatusV1":true}}}"#,
            )
            .expect("participant-status Hello should apply");
        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":true,"setBy":"bob"}}}"#,
            )
            .expect("room playstate should make the pause action player-bound");
        let mut application = ClientApplication::new(session, MpvAdapter::default());
        let _ = take_participant_status_reports(&mut application);
        assert!(planned_local_runtime_action_is_player_bound(
            &PlannedLocalRuntimeAction::Pause
        ));
        assert!(!planned_local_runtime_action_is_player_bound(
            &PlannedLocalRuntimeAction::SendChat("still connected".to_owned())
        ));
        let mut user_offset_seconds = 0.0;
        let (emitted, failure) = run_contained_planned_local_runtime_action(
            &mut application,
            &mut user_offset_seconds,
            2.0,
            PlannedLocalRuntimeAction::Pause,
        )
        .expect("a player-bound local failure should be contained");

        assert!(!emitted);
        let failure = failure.expect("the disconnected adapter should produce a contained fault");
        assert_eq!(failure.operation, "apply local player command");
        assert!(failure.status_publish_error.is_none());
        assert!(application.session().is_active());
        assert_eq!(application.session().username(), Some("alice"));
        assert_eq!(application.session().room(), Some("room"));
        let reports = take_participant_status_reports(&mut application);
        assert_eq!(reports.len(), 1);
        assert_eq!(
            reports[0].player_connection,
            ParticipantPlayerConnection::Disconnected
        );
    }

    #[test]
    fn non_player_local_runtime_failures_remain_fatal() {
        let mut application =
            ClientApplication::new(ClientSession::default(), MpvAdapter::default());
        let result = contain_planned_local_runtime_action_result(
            &mut application,
            2.0,
            false,
            Err(anyhow::anyhow!("non-player test failure")),
        );
        let Err(error) = result else {
            panic!("non-player failures must not be contained as player lifecycle faults");
        };

        assert_eq!(error.to_string(), "non-player test failure");
        assert!(take_participant_status_reports(&mut application).is_empty());
    }

    #[test]
    fn cli_connected_session_publishes_player_disconnect_and_reattach_lifecycle() {
        let connected = Arc::new(AtomicBool::new(false));
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room"},"version":"1.7.5","features":{"sorotteParticipantStatusV1":true}}}"#,
            )
            .expect("participant-status Hello should apply");
        let mut application = ClientApplication::new(
            session,
            LifecyclePlayer {
                connected: Arc::clone(&connected),
            },
        );

        assert!(
            take_participant_status_reports(&mut application).is_empty(),
            "construction must wait for the owner clock before publishing lifecycle state"
        );
        assert!(
            synchronize_connected_session_player_availability(&mut application, 0.0)
                .expect("the first owner observation should publish")
        );
        let reports = take_participant_status_reports(&mut application);
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].report_sequence, 1);
        assert_eq!(
            reports[0].player_connection,
            ParticipantPlayerConnection::Unavailable
        );

        connected.store(true, Ordering::SeqCst);
        assert!(
            synchronize_connected_session_player_availability(&mut application, 1.0)
                .expect("attach transition should publish")
        );
        let reports = take_participant_status_reports(&mut application);
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].report_sequence, 2);
        assert_eq!(
            reports[0].player_connection,
            ParticipantPlayerConnection::Starting
        );
        assert!(
            !synchronize_connected_session_player_availability(&mut application, 1.1)
                .expect("unchanged attachment should be a no-op")
        );

        application.prepare_playback_media(
            LogicalMediaId::new("cli-player-lifecycle").expect("logical ID should be valid"),
            MediaTransportKind::NetworkVod,
            1.1,
        );
        application.observe_external_player_transport(
            PlayerTransportTelemetryUpdate::new(
                PlayerMediaGeneration::new(1),
                PlayerObservationTimestamp::from_adapter_start(Duration::from_secs_f64(1.2)),
            )
            .with_phase(PlayerTransportPhase::Playing)
            .with_position_seconds(12.5)
            .with_logical_pause(false),
            1.2,
        );
        let reports = take_participant_status_reports(&mut application);
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].report_sequence, 3);
        assert_eq!(
            reports[0].player_connection,
            ParticipantPlayerConnection::Connected
        );

        connected.store(false, Ordering::SeqCst);
        assert!(
            synchronize_connected_session_player_availability(&mut application, 2.0)
                .expect("disconnect transition should publish")
        );
        let reports = take_participant_status_reports(&mut application);
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].report_sequence, 4);
        assert_eq!(
            reports[0].player_connection,
            ParticipantPlayerConnection::Disconnected
        );
        assert!(application.session().is_active());
        assert_eq!(application.session().username(), Some("alice"));
        assert_eq!(application.session().room(), Some("room"));

        connected.store(true, Ordering::SeqCst);
        assert!(
            synchronize_connected_session_player_availability(&mut application, 3.0)
                .expect("reattach transition should publish")
        );
        let reports = take_participant_status_reports(&mut application);
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].report_sequence, 5);
        assert_eq!(
            reports[0].player_connection,
            ParticipantPlayerConnection::Starting
        );
    }

    #[test]
    fn contained_player_failure_keeps_membership_and_queues_disconnect_status() {
        let connected = Arc::new(AtomicBool::new(true));
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room"},"version":"1.7.5","features":{"sorotteParticipantStatusV1":true}}}"#,
            )
            .expect("participant-status Hello should apply");
        let mut application = ClientApplication::new(
            session,
            LifecyclePlayer {
                connected: Arc::clone(&connected),
            },
        );
        let _ = take_participant_status_reports(&mut application);
        assert!(
            synchronize_connected_session_player_availability(&mut application, 1.0)
                .expect("initial attachment should publish")
        );
        let _ = take_participant_status_reports(&mut application);

        let failure = contain_connected_session_player_failure(
            &mut application,
            2.0,
            "test player operation",
            anyhow::Error::new(sorotte_player_api::PlayerError::NotConnected),
        );

        assert_eq!(failure.operation, "test player operation");
        assert!(failure.status_publish_error.is_none());
        assert!(application.session().is_active());
        assert_eq!(application.session().username(), Some("alice"));
        assert_eq!(application.session().room(), Some("room"));
        let reports = take_participant_status_reports(&mut application);
        assert_eq!(reports.len(), 1);
        assert_eq!(
            reports[0].player_connection,
            ParticipantPlayerConnection::Disconnected
        );
    }

    #[test]
    fn contained_player_failure_reopens_telemetry_for_an_attached_player() {
        let connected = Arc::new(AtomicBool::new(true));
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room"},"version":"1.7.5","features":{"sorotteParticipantStatusV1":true}}}"#,
            )
            .expect("participant-status Hello should apply");
        let mut application = ClientApplication::new(
            session,
            LifecyclePlayer {
                connected: Arc::clone(&connected),
            },
        );
        assert!(
            synchronize_connected_session_player_availability(&mut application, 1.0)
                .expect("initial attachment should publish")
        );
        let _ = take_participant_status_reports(&mut application);

        let failure = contain_connected_session_player_failure(
            &mut application,
            2.0,
            "test transient player operation",
            anyhow::Error::new(sorotte_player_api::PlayerError::OperationFailed(
                "transient test failure".to_owned(),
            )),
        );
        assert!(failure.status_publish_error.is_none());
        let reports = take_participant_status_reports(&mut application);
        assert_eq!(reports.len(), 1);
        assert_eq!(
            reports[0].player_connection,
            ParticipantPlayerConnection::Failed
        );
        assert!(application.session().is_active());

        assert!(
            synchronize_connected_session_player_availability(&mut application, 3.0)
                .expect("the still-attached player should begin a fresh lifecycle")
        );
        let reports = take_participant_status_reports(&mut application);
        assert_eq!(reports.len(), 1);
        assert_eq!(
            reports[0].player_connection,
            ParticipantPlayerConnection::Starting
        );

        application.prepare_playback_media(
            LogicalMediaId::new("cli-contained-failure-recovery")
                .expect("logical ID should be valid"),
            MediaTransportKind::NetworkVod,
            3.0,
        );
        application.observe_external_player_transport(
            PlayerTransportTelemetryUpdate::new(
                PlayerMediaGeneration::new(1),
                PlayerObservationTimestamp::from_adapter_start(Duration::from_secs_f64(3.1)),
            )
            .with_phase(PlayerTransportPhase::Playing)
            .with_position_seconds(12.5)
            .with_logical_pause(false),
            3.1,
        );
        let reports = take_participant_status_reports(&mut application);
        assert_eq!(reports.len(), 1);
        assert_eq!(
            reports[0].player_connection,
            ParticipantPlayerConnection::Connected
        );
        assert!(application.session().is_active());
    }

    #[tokio::test]
    async fn failed_player_step_is_flushed_as_status_without_failing_the_branch() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room"},"version":"1.7.5","features":{"sorotteParticipantStatusV1":true}}}"#,
            )
            .expect("participant-status Hello should apply");
        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":true,"setBy":"bob"}}}"#,
            )
            .expect("room playstate should apply");
        let mut application = ClientApplication::new(session, MpvAdapter::default());
        let _ = take_participant_status_reports(&mut application);
        assert!(
            application
                .record_contained_external_player_failure(
                    ExternalPlayerAvailability::Disconnected,
                    0.0,
                )
                .expect("the baseline disconnected status should publish")
        );
        let baseline = take_participant_status_reports(&mut application);
        assert_eq!(baseline.len(), 1);
        assert_eq!(baseline[0].report_sequence, 1);

        let (transport, peer) = tokio::io::duplex(16 * 1024);
        let transport: Box<dyn ConnectedSessionAsyncStream> = Box::new(transport);
        let (_transport_reader, mut writer) = tokio::io::split(transport);
        let mut peer = BufReader::new(peer);
        let config = crate::tests::test_client_loop_config();
        let diagnostics_config = client_loop_diagnostics_config(None);
        let mut startup_playlist = None;
        let mut reconnect_diagnostics = ReconnectCorrectionDiagnosticsState::default();
        let mut seek_notifications = SeekPreparationNotificationState::default();
        let mut readiness_notifications = ReadinessNotificationState::default();
        let mut file_difference_notifications = FileDifferenceNotificationState::default();
        let mut network_options = CliNetworkOptionsHealthReporter::default();
        let mut notification_sink = |_notification: &AutoplayCountdownNotification| Ok(());
        let mut file_difference_sink = |_line: &str| Ok(());

        run_connected_session_branch_plan_legacy_compatible(
            &mut application,
            2.0,
            false,
            false,
            ConnectedSessionBranchPlan {
                run_protocol_before_runtime_steps: true,
                runtime_steps: ConnectedSessionRuntimeStepPlan {
                    run_room_pause_sync: true,
                    run_readiness_unpause_attempt: false,
                    run_update_autoplay_check: false,
                    run_tick_autoplay: false,
                    run_desync_correction: false,
                    run_reconnect_state_restore_validation: false,
                    run_state_sync_heartbeat: false,
                    publish_pending_local_file_updates: false,
                },
                protocol: ConnectedSessionProtocolPlan {
                    flush_runtime_protocol_lines: false,
                    startup_playlist_disposition:
                        ConnectedSessionStartupPlaylistDisposition::LeavePending,
                },
                drain: ConnectedSessionDrainPlan {
                    flush_player_playback_diagnostics: false,
                    reconnect_correction_diagnostics_format: None,
                    flush_reconnect_notifications: false,
                    flush_controller_auth_notifications: false,
                    flush_chat_notifications: false,
                    flush_user_change_notifications: false,
                    flush_autoplay_notifications: false,
                    flush_file_difference_notifications: false,
                },
            },
            None,
            ConnectedSessionBranchExecutionContext {
                config: &config,
                writer: &mut writer,
                startup_playlist_file_on_connect: &mut startup_playlist,
                diagnostics_config: &diagnostics_config,
                reconnect_correction_diagnostics_state: &mut reconnect_diagnostics,
                seek_preparation_notification_state: &mut seek_notifications,
                readiness_notification_state: &mut readiness_notifications,
                file_difference_state: &mut file_difference_notifications,
                network_options_health_reporter: &mut network_options,
                notification_sink: &mut notification_sink,
                file_difference_sink: &mut file_difference_sink,
            },
        )
        .await
        .expect("a player failure must not fail the connected-session branch");

        let mut line = String::new();
        tokio::time::timeout(Duration::from_secs(1), peer.read_line(&mut line))
            .await
            .expect("terminal player status should be written promptly")
            .expect("duplex status read should succeed");
        let ProtocolMessage::State(state) =
            decode_message_line(line.trim_end()).expect("status line should decode")
        else {
            panic!("expected participant-status State");
        };
        let report = state
            .state
            .participant_status_v1()
            .expect("participant-status extension should decode")
            .and_then(|extension| extension.report)
            .expect("failed player step should publish a report");
        assert_eq!(
            report.player_connection,
            ParticipantPlayerConnection::Disconnected
        );
        assert_eq!(
            report.report_sequence, 2,
            "an unchanged contained failure must still flush the due advisory heartbeat",
        );
        assert!(application.session().is_active());
        assert_eq!(application.session().room(), Some("room"));
    }

    fn v2_session_with_canonical_intent(
        intent: UserReadinessIntent,
        source: ReadinessMutationSource,
    ) -> ClientSession {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room"},"version":"1.7.5","features":{"readiness":true,"sorotteReadinessV2":true}}}"#,
            )
            .expect("V2 Hello should apply");
        let participant = ParticipantReadinessUpdate {
            room_readiness_revision: 1,
            membership_epoch: 41,
            username: "alice".to_owned(),
            user_intent: intent,
            user_intent_revision: 1,
            last_technical_report_sequence: 0,
            user_intent_source: source,
            last_user_mutation: None,
            terminal_technical_block: None,
            technical_state: TechnicalPlayabilitySummary {
                phase: TechnicalPlayabilityPhase::Playable,
                media_generation: Some(7),
                reason: None,
                recovery: None,
            },
            participation_role: StartParticipationRole::Required,
            room_ready: intent == UserReadinessIntent::Ready,
            start_eligible: intent == UserReadinessIntent::Ready,
            accepted_operation_id: None,
        };
        let snapshot = RoomReadinessSnapshot {
            room_readiness_revision: 1,
            media_generation: Some(7),
            start_gate_phase: RoomStartGatePhase::WaitingForIntent {
                media_generation: 7,
            },
            pause_owner: RoomPauseOwner::ReadinessStartGate {
                media_generation: 7,
            },
            mixed_readiness_policy: Default::default(),
            participants: BTreeMap::from([("alice".to_owned(), participant)]),
        };
        session
            .apply_protocol_message(ProtocolMessage::set(
                SetPayload::new()
                    .with_readiness_v2(ReadinessSetExtension::new().with_snapshot(snapshot)),
            ))
            .expect("canonical V2 snapshot should apply");
        session
    }

    #[test]
    fn v2_ready_at_start_waits_for_fresh_canonical_membership() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room"},"version":"1.7.5","features":{"readiness":true,"sorotteReadinessV2":true}}}"#,
            )
            .expect("V2 Hello should apply");
        let ready = PendingReadyAtStart {
            desired: true,
            had_current_v2_membership: false,
        };
        assert_eq!(
            ready_at_start_disposition(&session, ready),
            ReadyAtStartDisposition::AwaitCanonicalV2Membership
        );

        session = v2_session_with_canonical_intent(
            UserReadinessIntent::NotReady,
            ReadinessMutationSource::Initialization,
        );
        assert_eq!(
            ready_at_start_disposition(&session, ready),
            ReadyAtStartDisposition::Apply(true)
        );
        assert_eq!(
            ready_at_start_disposition(
                &session,
                PendingReadyAtStart {
                    desired: false,
                    had_current_v2_membership: false,
                },
            ),
            ReadyAtStartDisposition::ConsumeWithoutMutation,
            "implicit V2 NotReady must not manufacture a user mutation"
        );
    }

    #[test]
    fn v2_ready_at_start_preserves_acknowledged_intent_on_reconnect() {
        for intent in [UserReadinessIntent::Ready, UserReadinessIntent::NotReady] {
            let session = v2_session_with_canonical_intent(
                intent,
                ReadinessMutationSource::DirectUser {
                    surface: DirectReadinessSurface::CliCommand,
                },
            );
            assert_eq!(
                ready_at_start_disposition(
                    &session,
                    PendingReadyAtStart {
                        desired: intent != UserReadinessIntent::Ready,
                        had_current_v2_membership: true,
                    },
                ),
                ReadyAtStartDisposition::ConsumeWithoutMutation,
                "startup preference must not replace acknowledged {intent:?}"
            );
        }
    }

    #[test]
    fn v2_ready_at_start_never_supersedes_a_semantic_pending_operation() {
        let mut session = v2_session_with_canonical_intent(
            UserReadinessIntent::NotReady,
            ReadinessMutationSource::Initialization,
        );
        let actions = session.runtime_actions_for_direct_readiness_intent(
            UserReadinessIntent::Ready,
            DirectReadinessSurface::CliCommand,
            None,
        );
        assert_eq!(actions.len(), 1);
        assert!(session.pending_readiness_intent().is_some());
        assert_eq!(
            ready_at_start_disposition(
                &session,
                PendingReadyAtStart {
                    desired: false,
                    had_current_v2_membership: false,
                },
            ),
            ReadyAtStartDisposition::ConsumeWithoutMutation
        );
    }

    #[test]
    fn legacy_ready_at_start_keeps_existing_post_hello_behavior() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room"},"version":"1.7.5","features":{"readiness":true}}}"#,
            )
            .expect("legacy readiness Hello should apply");
        for desired in [false, true] {
            assert_eq!(
                ready_at_start_disposition(
                    &session,
                    PendingReadyAtStart {
                        desired,
                        had_current_v2_membership: true,
                    },
                ),
                ReadyAtStartDisposition::Apply(desired)
            );
        }
    }

    #[test]
    fn v2_ready_at_start_is_emitted_only_after_the_canonical_snapshot_arrives() {
        let canonical = v2_session_with_canonical_intent(
            UserReadinessIntent::NotReady,
            ReadinessMutationSource::Initialization,
        )
        .readiness_snapshot()
        .expect("fixture should contain a readiness snapshot")
        .clone();
        let mut application = ClientApplication::new(ClientSession::default(), DisconnectedPlayer);
        application
            .apply_protocol_line(
                r#"{"Hello":{"username":"alice","room":{"name":"room"},"version":"1.7.5","features":{"readiness":true,"sorotteReadinessV2":true}}}"#,
                1.0,
                false,
                false,
                false,
            )
            .expect("V2 Hello should apply");
        let mut pending_ready = Some(PendingReadyAtStart {
            desired: true,
            had_current_v2_membership: false,
        });
        let mut pending_chat = None;
        let plan = ConnectedSessionInboundPostApplyPlan {
            consume_pending_ready_at_start: true,
            consume_pending_chat_message_on_connect: false,
            run_reconnect_transition: false,
            run_controller_reidentify: false,
            run_controller_auth_notifications: false,
            run_chat_notifications: false,
            run_user_change_notifications: false,
            run_reconnect_state_restore: false,
            run_reconnect_playlist_restore: false,
        };

        assert!(
            run_connected_session_inbound_post_apply_legacy_compatible(
                &mut application,
                &mut pending_ready,
                &mut pending_chat,
                1.0,
                plan,
            )
            .is_none(),
            "deferring startup readiness must not manufacture a contained player failure"
        );
        assert!(pending_ready.is_some());
        assert_eq!(application.pending_protocol_message_count(), 0);

        let snapshot_line = encode_message_line(&ProtocolMessage::set(
            SetPayload::new()
                .with_readiness_v2(ReadinessSetExtension::new().with_snapshot(canonical)),
        ))
        .expect("snapshot should encode");
        application
            .apply_protocol_line(&snapshot_line, 2.0, false, false, false)
            .expect("snapshot should apply");
        assert!(
            run_connected_session_inbound_post_apply_legacy_compatible(
                &mut application,
                &mut pending_ready,
                &mut pending_chat,
                2.0,
                plan,
            )
            .is_none(),
            "emitting startup readiness must not manufacture a contained player failure"
        );
        assert!(pending_ready.is_none());

        let pending_line = application
            .pending_protocol_line()
            .expect("queued readiness should encode")
            .expect("fresh V2 membership should queue Ready");
        let ProtocolMessage::Set(set) =
            decode_message_line(pending_line.line()).expect("queued readiness should decode")
        else {
            panic!("V2 readiness should use Set");
        };
        let intent = set
            .set
            .readiness_v2()
            .expect("readiness extension should decode")
            .and_then(|extension| extension.intent)
            .expect("queued readiness intent should be present");
        assert_eq!(intent.desired, UserReadinessIntent::Ready);
        assert_eq!(intent.membership_epoch, 41);
        assert_eq!(
            intent.source,
            sorotte_protocol::UserReadinessMutationSource::Initialization,
            "fresh ready-at-start must retain room-entry initialization provenance"
        );
    }

    #[test]
    fn cli_post_apply_uses_the_inbound_clock_and_does_not_retry_early() {
        let mut application = ClientApplication::new(ClientSession::default(), DisconnectedPlayer);
        application
            .apply_protocol_line(
                r#"{"Hello":{"username":"alice","room":{"name":"room"},"version":"1.7.5","features":{"sorottePlaybackBarrierV1":true}}}"#,
                1.0,
                false,
                false,
                false,
            )
            .expect("barrier-aware Hello should apply");
        application.set_playback_barrier_start_config(PlaybackBarrierStartConfig {
            policy: Some(PlaybackBarrierPolicy::Controller),
            ..PlaybackBarrierStartConfig::default()
        });
        application.prepare_playback_media(
            LogicalMediaId::new("cli-post-apply-retry").expect("logical ID should be valid"),
            MediaTransportKind::NetworkVod,
            2.0,
        );
        let pending = application
            .pending_protocol_line()
            .expect("request should encode")
            .expect("request should be queued");
        let ProtocolMessage::Set(set) =
            decode_message_line(pending.line()).expect("request should decode")
        else {
            panic!("playback request should use Set");
        };
        let prepare = set
            .set
            .playback_barrier_v1()
            .expect("extension should decode")
            .and_then(|extension| extension.prepare)
            .expect("request should include prepare");
        application
            .acknowledge_protocol_line(pending.lease())
            .expect("local write should release the request frame");

        let retry_later = ProtocolMessage::set(SetPayload::new().with_playback_barrier_v1(
            PlaybackBarrierSetExtension::new().with_request_result(
                PlaybackBarrierRequestResultPayload::retry_later(
                    prepare.request_id.expect("request ID should be present"),
                    prepare.request_nonce,
                    1_000,
                ),
            ),
        ));
        let retry_line = encode_message_line(&retry_later).expect("retry result should encode");
        apply_connected_session_inbound_message_legacy_compatible(
            &mut application,
            &retry_line,
            10.0,
            false,
            ConnectedSessionInboundApplyPlan {
                reconcile_inbound_state: false,
                apply_message_json_at: true,
                outbound_state_sync_enabled: false,
            },
        )
        .expect("CLI inbound apply should keep retryLater nonfatal");
        let mut pending_ready = None;
        let mut pending_chat = None;
        assert!(
            run_connected_session_inbound_post_apply_legacy_compatible(
                &mut application,
                &mut pending_ready,
                &mut pending_chat,
                10.0,
                ConnectedSessionInboundPostApplyPlan {
                    consume_pending_ready_at_start: false,
                    consume_pending_chat_message_on_connect: false,
                    run_reconnect_transition: false,
                    run_controller_reidentify: false,
                    run_controller_auth_notifications: true,
                    run_chat_notifications: false,
                    run_user_change_notifications: false,
                    run_reconnect_state_restore: false,
                    run_reconnect_playlist_restore: false,
                },
            )
            .is_none(),
            "CLI post-apply should use the same monotonic timestamp without a contained failure"
        );

        assert_eq!(application.pending_protocol_message_count(), 0);
        assert_eq!(
            application.pending_playback_barrier_retry_delay_at(10.0),
            Some(1.0)
        );
        application
            .run_pending_playback_barrier_retry_at(11.0)
            .expect("due retry should emit");
        application
            .run_pending_playback_barrier_retry_at(12.0)
            .expect("repeated pump should be idempotent");
        assert_eq!(application.pending_protocol_message_count(), 1);
    }
}
