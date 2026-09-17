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

#[derive(Clone, Copy)]
pub(super) enum ConnectedSessionEvent<'a> {
    InboundMessage(&'a str),
    AutoplayTick,
    PlayerCoordinationTick,
    LocalInput { emitted: bool },
}

fn flush_connected_session_outputs<F, G>(
    runtime: &mut ClientApplication<MpvAdapter>,
    event: ConnectedSessionEvent<'_>,
    context: &mut ConnectedSessionExecutionContext<'_, F, G>,
) -> anyhow::Result<()>
where
    F: FnMut(&AutoplayCountdownNotification) -> anyhow::Result<()>,
    G: FnMut(&str) -> anyhow::Result<()>,
{
    // Readiness and seek preparation are ordinary user-visible state on every event.
    flush_seek_preparation_notifications(runtime, context.seek_preparation_notification_state);
    flush_readiness_status_notifications(runtime, context.readiness_notification_state);
    let diagnostics = context.diagnostics_config;
    if diagnostics.log_player_telemetry || diagnostics.log_player_drift {
        flush_player_playback_telemetry_diagnostics(
            runtime,
            diagnostics.log_player_telemetry,
            diagnostics.log_player_drift,
        )?;
    }
    flush_reconnect_notifications(runtime, &mut emit_reconnect_transition_notification)?;
    if let Some(format) = diagnostics.reconnect_correction_diagnostics_format {
        flush_reconnect_correction_diagnostics_to_sink(
            runtime,
            context.reconnect_correction_diagnostics_state,
            &diagnostics.reconnect_correction_diagnostics_alert_thresholds,
            format,
            &mut emit_reconnect_correction_diagnostic,
        )?;
    }
    if matches!(event, ConnectedSessionEvent::InboundMessage(_)) {
        flush_controller_auth_notifications(
            runtime,
            &mut emit_controller_auth_transition_notification,
        )?;
        flush_chat_notifications(runtime, &mut emit_chat_notification)?;
        flush_user_change_notifications(runtime, &mut emit_user_change_notification)?;
    }
    if matches!(
        event,
        ConnectedSessionEvent::InboundMessage(_) | ConnectedSessionEvent::AutoplayTick
    ) {
        flush_autoplay_notifications(runtime, context.notification_sink)?;
    }
    if !matches!(event, ConnectedSessionEvent::LocalInput { .. }) {
        flush_file_difference_notifications(
            runtime,
            context.file_difference_state,
            context.file_difference_sink,
        )?;
    }
    Ok(())
}

fn run_connected_session_inbound_post_apply<P>(
    runtime: &mut ClientApplication<P>,
    pending_ready_at_start_on_server_hello: &mut Option<PendingReadyAtStart>,
    pending_chat_message_on_connect: &mut Option<String>,
    now_seconds: f64,
    consume_pending_readiness: bool,
    shared_playlists_enabled: bool,
) -> Option<ContainedConnectedSessionPlayerFailure>
where
    P: sorotte_player_api::PlayerAdapter,
{
    let result = (|| -> Result<(), (&'static str, anyhow::Error)> {
        if consume_pending_readiness && let Some(pending) = *pending_ready_at_start_on_server_hello
        {
            match ready_at_start_disposition(runtime.session(), pending) {
                ReadyAtStartDisposition::AwaitCanonicalV2Membership => {}
                ReadyAtStartDisposition::ConsumeWithoutMutation => {
                    *pending_ready_at_start_on_server_hello = None;
                }
                ReadyAtStartDisposition::Apply(ready_at_start) => {
                    *pending_ready_at_start_on_server_hello = None;
                    runtime
                        .run_initial_readiness_intent(ready_at_start)
                        .map_err(|error| ("apply initial readiness intent", error.into()))?;
                }
            }
        }
        if let Some(message) = pending_chat_message_on_connect.take() {
            runtime
                .run_send_chat_message(message)
                .map_err(|error| ("send initial chat message", error.into()))?;
        }
        runtime
            .run_reconnect_transition_if_needed()
            .map_err(|error| ("apply reconnect transition", error.into()))?;
        runtime
            .run_controller_reidentify_if_needed()
            .map_err(|error| ("reidentify room controller", error.into()))?;
        runtime
            .run_controller_auth_notifications_if_needed_at(now_seconds)
            .map_err(|error| {
                (
                    "publish controller authentication notification",
                    error.into(),
                )
            })?;
        runtime
            .run_chat_notifications_if_needed()
            .map_err(|error| ("publish chat notification", error.into()))?;
        runtime
            .run_user_change_notifications_if_needed()
            .map_err(|error| ("publish user-change notification", error.into()))?;
        runtime
            .run_reconnect_state_restore_if_needed()
            .map_err(|error| ("restore player state after reconnect", error.into()))?;
        if shared_playlists_enabled {
            runtime
                .run_reconnect_playlist_restore_if_needed()
                .map_err(|error| ("restore player playlist after reconnect", error.into()))?;
        }
        Ok(())
    })();
    result.err().map(|(operation, error)| {
        contain_connected_session_player_failure(runtime, now_seconds, operation, error)
    })
}

fn apply_connected_session_inbound_message<P>(
    application: &mut ClientApplication<P>,
    line: &str,
    now_seconds: f64,
    dont_slow_down_with_me: bool,
    contains_state: bool,
) -> anyhow::Result<sorotte_client_app::app_boundary::application::ProtocolLineApplyOutcome>
where
    P: sorotte_player_api::PlayerAdapter,
{
    let ping_received_at_seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0);
    Ok(application.apply_protocol_line_prefix_at_clocks(
        line,
        now_seconds,
        ping_received_at_seconds,
        contains_state,
        dont_slow_down_with_me,
        !contains_state,
    )?)
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
    let result =
        run_planned_local_runtime_action(runtime, user_offset_seconds, now_seconds, action);
    contain_planned_local_runtime_action_result(runtime, now_seconds, player_bound, result)
}

pub(super) fn planned_local_runtime_action_is_player_bound(
    action: &PlannedLocalRuntimeAction,
) -> bool {
    matches!(
        action,
        PlannedLocalRuntimeAction::UndoSeek
            | PlannedLocalRuntimeAction::SetUserOffset(_)
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

fn run_connected_session_branch_runtime_steps(
    runtime: &mut ClientApplication<MpvAdapter>,
    config: &ClientLoopConfig,
    network_options_health_reporter: &mut CliNetworkOptionsHealthReporter,
    now_seconds: f64,
    dont_slow_down_with_me: bool,
    outbound_state_sync_enabled: bool,
    event: ConnectedSessionEvent<'_>,
) -> Option<ContainedConnectedSessionPlayerFailure> {
    let shared_playlists_enabled = shared_playlists_enabled_cli(config);
    runtime.set_shared_playlist_sync_enabled(shared_playlists_enabled);
    // Maintenance may attach a replacement player. Record that transition before telemetry.
    runtime.with_player_io(|player| player.maintain_runtime_integrations());
    if let Err(error) = synchronize_connected_session_player_availability(runtime, now_seconds) {
        return Some(contain_connected_session_player_failure(
            runtime,
            now_seconds,
            "synchronize player availability",
            error.into(),
        ));
    }
    let inputs = derive_runtime_loop_inputs(runtime, config, now_seconds);
    let result = (|| -> Result<(), (&'static str, anyhow::Error)> {
        if !matches!(event, ConnectedSessionEvent::LocalInput { .. }) {
            let outcome = if shared_playlists_enabled {
                runtime.run_room_pause_sync_if_needed_at_for_canonical_playlist_owner(now_seconds)
            } else {
                runtime.run_room_pause_sync_if_needed_at(now_seconds)
            };
            outcome.map_err(|error| ("synchronize room pause state", error.into()))?;
        }
        match event {
            ConnectedSessionEvent::InboundMessage(_) => {
                runtime
                    .run_readiness_unpause_attempt(
                        now_seconds,
                        inputs.readiness_supported,
                        inputs.local_can_control,
                        inputs.is_playing_music,
                    )
                    .map_err(|error| ("apply readiness unpause", error.into()))?;
            }
            ConnectedSessionEvent::AutoplayTick => {
                runtime.update_autoplay_check(
                    inputs.readiness_supported,
                    inputs.local_can_control,
                    inputs.is_playing_music,
                    inputs.recently_advanced,
                );
                runtime
                    .tick_autoplay(
                        inputs.readiness_supported,
                        inputs.local_can_control,
                        inputs.is_playing_music,
                        inputs.recently_advanced,
                    )
                    .map_err(|error| ("advance autoplay", error.into()))?;
            }
            ConnectedSessionEvent::PlayerCoordinationTick
            | ConnectedSessionEvent::LocalInput { .. } => {}
        }
        if matches!(
            event,
            ConnectedSessionEvent::InboundMessage(_) | ConnectedSessionEvent::AutoplayTick
        ) {
            runtime
                .run_desync_correction_if_needed(
                    now_seconds,
                    inputs.local_can_control,
                    dont_slow_down_with_me,
                    true,
                )
                .map_err(|error| ("apply desync correction", error.into()))?;
        }
        runtime
            .run_reconnect_state_restore_validation_if_needed_at(now_seconds)
            .map_err(|error| ("validate player state after reconnect", error.into()))?;
        if matches!(event, ConnectedSessionEvent::AutoplayTick) {
            if outbound_state_sync_enabled {
                let _ = runtime.run_state_sync_reconcile_with_inbound_state_with_ping_at(
                    StatePayload::new(),
                    dont_slow_down_with_me,
                    now_seconds,
                );
            } else {
                let _ = runtime.run_participant_status_heartbeat(now_seconds);
            }
        }
        if !matches!(event, ConnectedSessionEvent::LocalInput { .. }) {
            publish_pending_local_file_updates(
                runtime,
                config,
                network_options_health_reporter,
                now_seconds,
            )
            .map_err(|error| ("publish local file update", error))?;
        }
        Ok(())
    })();
    if let Err((operation, error)) = result {
        return Some(contain_connected_session_player_failure(
            runtime,
            now_seconds,
            operation,
            error,
        ));
    }

    if shared_playlists_enabled {
        for (operation, outcome) in [
            (
                "advance playlist after natural completion",
                runtime
                    .run_advance_playlist_after_natural_completion()
                    .map(|_| ())
                    .map_err(anyhow::Error::from),
            ),
            (
                "synchronize canonical playlist selection",
                runtime
                    .synchronize_canonical_playlist_selection_to_player()
                    .map(|_| ())
                    .map_err(anyhow::Error::from),
            ),
        ] {
            if let Err(error) = outcome {
                return Some(contain_connected_session_player_failure(
                    runtime,
                    now_seconds,
                    operation,
                    error,
                ));
            }
        }
    }

    None
}

pub(super) struct ConnectedSessionExecutionContext<'a, F, G>
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
    pub(super) pending_ready_at_start_on_server_hello: &'a mut Option<PendingReadyAtStart>,
    pub(super) pending_chat_message_on_connect: &'a mut Option<String>,
    pub(super) outbound_state_sync_enabled: &'a mut bool,
}

pub(super) async fn run_connected_session_event<F, G>(
    runtime: &mut ClientApplication<MpvAdapter>,
    event: ConnectedSessionEvent<'_>,
    now_seconds: f64,
    mut context: ConnectedSessionExecutionContext<'_, F, G>,
) -> anyhow::Result<()>
where
    F: FnMut(&AutoplayCountdownNotification) -> anyhow::Result<()>,
    G: FnMut(&str) -> anyhow::Result<()>,
{
    let dont_slow_down_with_me = context
        .config
        .dont_slow_down_with_me_override
        .unwrap_or(false);
    let shared_playlists_enabled = shared_playlists_enabled_cli(context.config);
    runtime.set_shared_playlist_sync_enabled(shared_playlists_enabled);
    let mut trailing_decode_error = None;
    let mut player_failure = None;
    if let ConnectedSessionEvent::InboundMessage(line) = event {
        let (messages, _) = decode_inbound_message_prefix(line);
        let contains_state = messages
            .iter()
            .any(|message| matches!(message, ProtocolMessage::State(_)));
        let consume_pending_readiness = context.pending_ready_at_start_on_server_hello.is_some()
            && (messages
                .iter()
                .any(|message| matches!(message, ProtocolMessage::Hello(_)))
                || runtime.session().server_readiness_v2_supported());
        let outcome = apply_connected_session_inbound_message(
            runtime,
            line,
            now_seconds,
            dont_slow_down_with_me,
            contains_state,
        )?;
        *context.outbound_state_sync_enabled |= contains_state || outcome.state_sync_emitted;
        match (outcome.applied_message_count, outcome.trailing_decode_error) {
            (0, Some(error)) => return Err(error.into()),
            (_, error) => trailing_decode_error = error,
        }
        player_failure = run_connected_session_inbound_post_apply(
            runtime,
            context.pending_ready_at_start_on_server_hello,
            context.pending_chat_message_on_connect,
            now_seconds,
            consume_pending_readiness,
            shared_playlists_enabled,
        );
    }
    // Local input delivers its command before maintenance; timers and inbound messages
    // deliver after runtime work. Keep this order and exact transport receipts.
    if matches!(event, ConnectedSessionEvent::LocalInput { emitted: true }) {
        flush_runtime_protocol_lines(runtime, context.writer).await?;
    }
    let player_failure = player_failure.or_else(|| {
        run_connected_session_branch_runtime_steps(
            runtime,
            context.config,
            context.network_options_health_reporter,
            now_seconds,
            dont_slow_down_with_me,
            *context.outbound_state_sync_enabled,
            event,
        )
    });
    if !matches!(event, ConnectedSessionEvent::LocalInput { .. }) {
        flush_runtime_protocol_lines(runtime, context.writer).await?;
    }
    if matches!(event, ConnectedSessionEvent::InboundMessage(_))
        && let Some(playlist_path) = context.startup_playlist_file_on_connect.take()
        && shared_playlists_enabled
    {
        let _ =
            emit_startup_playlist_load_from_file(runtime, context.writer, &playlist_path).await?;
    }
    if player_failure.is_some() {
        // Contain player faults within the room and give advisory status a write opportunity,
        // including local input whose normal write happened before maintenance.
        let _ = runtime.run_participant_status_heartbeat(now_seconds);
        flush_runtime_protocol_lines(runtime, context.writer).await?;
    }
    flush_connected_session_outputs(runtime, event, &mut context)?;
    if let Some(failure) = player_failure.as_ref() {
        report_contained_connected_session_player_failure(failure);
    }
    // Effects from accepted commands, including their write receipts, precede a suffix error.
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

    #[test]
    fn player_coordination_publishes_file_updates_deferred_by_local_input() {
        let mut application =
            ClientApplication::new(ClientSession::default(), MpvAdapter::simulated());
        application
            .player_mut()
            .open_file("movie.mkv")
            .expect("the adapter should queue the opened file");
        let config = crate::tests::test_client_loop_config();
        let mut network_options = CliNetworkOptionsHealthReporter::default();

        for (event, expected_file) in [
            (ConnectedSessionEvent::LocalInput { emitted: true }, None),
            (
                ConnectedSessionEvent::PlayerCoordinationTick,
                Some("movie.mkv"),
            ),
        ] {
            assert!(
                run_connected_session_branch_runtime_steps(
                    &mut application,
                    &config,
                    &mut network_options,
                    1.0,
                    false,
                    false,
                    event,
                )
                .is_none(),
                "processing a queued file should not produce a player fault",
            );
            assert_eq!(
                application
                    .last_local_file_update()
                    .and_then(|update| update.path.as_deref()),
                expected_file,
                "local input should leave the queued file for player coordination",
            );
        }

        let pending = application
            .pending_protocol_line()
            .expect("the published file should encode")
            .expect("player coordination should queue the file announcement");
        let message = application
            .acknowledge_protocol_line(pending.lease())
            .expect("the file announcement should remain queued until acknowledged");
        let ProtocolMessage::Set(message) = message else {
            panic!("expected a Set.file announcement");
        };
        assert_eq!(
            message.set.file.and_then(|file| file.name),
            Some("movie.mkv".to_owned()),
        );
    }

    #[test]
    fn canonical_playlist_desync_branch_preserves_predecessor_until_resolution() {
        for shared_playlists_enabled in [true, false] {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs_f64();
            let mut session = ClientSession::default();
            for line in [
                r#"{"Hello":{"username":"alice","room":{"name":"room"},"version":"1.7.5","features":{"sharedPlaylists":true}}}"#,
                r#"{"Set":{"user":{"bob":{"room":{"name":"room"}}}}}"#,
                r#"{"Set":{"playlistChange":{"files":["episode-a.mkv","plex://machine/123"],"user":"bob"}}}"#,
                r#"{"Set":{"playlistIndex":{"index":0,"user":"bob"}}}"#,
                r#"{"State":{"playstate":{"position":9.8,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            ] {
                session.apply_message_json_at(line, now).unwrap();
            }
            let mut application = ClientApplication::new(session, MpvAdapter::simulated());
            application.player_mut().open_file("episode-a.mkv").unwrap();
            application.player_mut().set_position(9.8).unwrap();
            application.player_mut().set_paused(false).unwrap();
            application.run_room_pause_sync_if_needed_at(now).unwrap();
            application
                .publish_pending_local_file_update(
                    sorotte_client_core::PrivacyMode::SendRaw,
                    sorotte_client_core::PrivacyMode::SendRaw,
                )
                .unwrap();
            application
                .synchronize_canonical_playlist_selection_to_player()
                .unwrap();
            for _ in 0..2 {
                application.player_mut().set_paused(false).unwrap();
                application.run_room_pause_sync_if_needed_at(now).unwrap();
            }
            assert!(
                !application
                    .session()
                    .has_pending_playlist_index_reset_intent()
            );
            let predecessor_position = application.player().position_seconds();
            assert!(predecessor_position >= 9.8);
            assert!(
                !application
                    .playback_coordination_snapshot()
                    .ordinary_correction_blocked,
                "the predecessor must have completed startup coordination: {:?}",
                application.playback_coordination_snapshot(),
            );

            application
                .session_mut()
                .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"bob"}}}"#)
                .unwrap();
            application
                .session_mut()
                .apply_message_json_at(
                    &serde_json::json!({"State":{"playstate":{
                        "position":0.0,
                        "paused":shared_playlists_enabled,
                        "doSeek":false,
                        "setBy":"bob"
                    }}})
                    .to_string(),
                    now + 0.1,
                )
                .unwrap();
            let mut config = crate::tests::test_client_loop_config();
            config.shared_playlists_enabled_override = Some(shared_playlists_enabled);
            let mut network_options = CliNetworkOptionsHealthReporter::default();
            let failure = run_connected_session_branch_runtime_steps(
                &mut application,
                &config,
                &mut network_options,
                now + 0.2,
                false,
                false,
                ConnectedSessionEvent::InboundMessage(""),
            );
            assert!(
                failure.is_none(),
                "a pending selected source must not manufacture a player failure"
            );
            assert_eq!(application.player().current_path(), Some("episode-a.mkv"));
            if shared_playlists_enabled {
                assert_eq!(
                    application.player().position_seconds(),
                    predecessor_position,
                    "the canonical owner must not apply successor state to its predecessor"
                );
            } else {
                assert!(
                    application.player().position_seconds() < 1.0,
                    "without a pause change or explicit seek, an opted-out client still corrects ordinary drift: position={}, coordination={:?}",
                    application.player().position_seconds(),
                    application.playback_coordination_snapshot(),
                );
            }
            assert!(
                application
                    .session()
                    .has_pending_playlist_index_reset_intent()
            );
        }
    }

    #[test]
    fn canonical_playlist_sync_does_not_adopt_preloaded_file_for_unresolved_selection() {
        use sorotte_player_api::{PlayerAdapter, PlayerCommand};

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        let mut player = MpvAdapter::simulated();
        player
            .execute(PlayerCommand::OpenFile("episode-a.mkv".to_owned()))
            .unwrap();
        player.execute(PlayerCommand::SetPosition(9.8)).unwrap();
        player.execute(PlayerCommand::SetPaused(false)).unwrap();
        let predecessor_position = player.position_seconds();
        let mut application = ClientApplication::new(ClientSession::default(), player);
        for line in [
            r#"{"Hello":{"username":"alice","room":{"name":"room"},"version":"1.7.5","features":{"sharedPlaylists":true}}}"#,
            r#"{"Set":{"user":{"bob":{"room":{"name":"room"}}}}}"#,
            r#"{"Set":{"playlistChange":{"files":["episode-a.mkv","plex://machine/123"],"user":"bob"}}}"#,
            r#"{"Set":{"playlistIndex":{"index":0,"user":"bob"}}}"#,
            r#"{"Set":{"playlistIndex":{"index":1,"user":"bob"}}}"#,
            r#"{"State":{"playstate":{"position":0.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
        ] {
            application
                .session_mut()
                .apply_message_json_at(line, now)
                .unwrap();
        }
        assert_eq!(
            application.session().current_room_playlist().unwrap().index,
            Some(1)
        );
        assert!(
            application
                .session()
                .has_pending_playlist_index_reset_intent()
        );
        let mut config = crate::tests::test_client_loop_config();
        config.shared_playlists_enabled_override = Some(true);
        let failure = run_connected_session_branch_runtime_steps(
            &mut application,
            &config,
            &mut CliNetworkOptionsHealthReporter::default(),
            now,
            false,
            false,
            ConnectedSessionEvent::PlayerCoordinationTick,
        );
        assert!(failure.is_none());
        assert_eq!(application.player().current_path(), Some("episode-a.mkv"));
        assert_eq!(
            application.player().position_seconds(),
            predecessor_position,
            "publishing the initial old file cannot assign it the unresolved successor's reset"
        );
        assert_eq!(
            application
                .last_local_file_update()
                .unwrap()
                .path
                .as_deref(),
            Some("episode-a.mkv"),
            "the fence must continue consuming and publishing actual player observations"
        );
        assert!(
            application
                .session()
                .has_pending_playlist_index_reset_intent()
        );
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

        run_connected_session_event(
            &mut application,
            ConnectedSessionEvent::PlayerCoordinationTick,
            2.0,
            ConnectedSessionExecutionContext {
                pending_ready_at_start_on_server_hello: &mut None,
                pending_chat_message_on_connect: &mut None,
                outbound_state_sync_enabled: &mut false,
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
        assert!(
            run_connected_session_inbound_post_apply(
                &mut application,
                &mut pending_ready,
                &mut pending_chat,
                1.0,
                true,
                false,
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
            run_connected_session_inbound_post_apply(
                &mut application,
                &mut pending_ready,
                &mut pending_chat,
                2.0,
                true,
                false,
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
        apply_connected_session_inbound_message(&mut application, &retry_line, 10.0, false, false)
            .expect("CLI inbound apply should keep retryLater nonfatal");
        let mut pending_ready = None;
        let mut pending_chat = None;
        assert!(
            run_connected_session_inbound_post_apply(
                &mut application,
                &mut pending_ready,
                &mut pending_chat,
                10.0,
                false,
                false,
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
