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
) -> anyhow::Result<()>
where
    P: sorotte_player_api::PlayerAdapter,
{
    for action in connected_session_inbound_post_apply_actions_legacy_compatible(plan) {
        match action {
            ConnectedSessionInboundPostApplyAction::ConsumePendingReadyAtStart => {
                if let Some(pending) = *pending_ready_at_start_on_server_hello {
                    match ready_at_start_disposition(runtime.session(), pending) {
                        ReadyAtStartDisposition::AwaitCanonicalV2Membership => {}
                        ReadyAtStartDisposition::ConsumeWithoutMutation => {
                            *pending_ready_at_start_on_server_hello = None;
                        }
                        ReadyAtStartDisposition::Apply(ready_at_start) => {
                            *pending_ready_at_start_on_server_hello = None;
                            let _ = runtime.run_initial_readiness_intent(ready_at_start)?;
                        }
                    }
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
                runtime.run_controller_auth_notifications_if_needed_at(now_seconds)?;
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
    let outcome = application.apply_protocol_line_prefix(
        line,
        now_seconds,
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
                runtime.run_room_pause_sync_if_needed_at(now_seconds)?;
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
        seek_preparation_notification_state,
        readiness_notification_state,
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
    if let Some(inbound_post_apply) = event_execution_plan.event.inbound_post_apply {
        run_connected_session_inbound_post_apply_legacy_compatible(
            runtime,
            pending_ready_at_start_on_server_hello,
            pending_chat_message_on_connect,
            now_seconds,
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

    if let Some(error) = trailing_decode_error {
        return Err(error.into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use sorotte_client_core::{
        ClientSession, LogicalMediaId, MediaTransportKind, PlaybackBarrierStartConfig,
    };
    use sorotte_player_api::DisconnectedPlayer;
    use sorotte_protocol::{
        DirectReadinessSurface, ParticipantReadinessUpdate, PlaybackBarrierPolicy,
        PlaybackBarrierRequestResultPayload, PlaybackBarrierSetExtension, ReadinessMutationSource,
        ReadinessSetExtension, RoomPauseOwner, RoomReadinessSnapshot, RoomStartGatePhase,
        SetPayload, StartParticipationRole, TechnicalPlayabilityPhase, TechnicalPlayabilitySummary,
        UserReadinessIntent,
    };

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

        run_connected_session_inbound_post_apply_legacy_compatible(
            &mut application,
            &mut pending_ready,
            &mut pending_chat,
            1.0,
            plan,
        )
        .expect("post-Hello startup readiness should defer");
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
        run_connected_session_inbound_post_apply_legacy_compatible(
            &mut application,
            &mut pending_ready,
            &mut pending_chat,
            2.0,
            plan,
        )
        .expect("post-snapshot startup readiness should emit");
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
        .expect("CLI post-apply should use the same monotonic timestamp");

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
