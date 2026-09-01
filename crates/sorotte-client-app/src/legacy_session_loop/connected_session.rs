use super::types::{
    ConnectedSessionBranchPlan, ConnectedSessionDiagnosticsPlan, ConnectedSessionDrainAction,
    ConnectedSessionDrainPlan, ConnectedSessionEventExecutionPlan,
    ConnectedSessionEventExecutionPlanInputs, ConnectedSessionEventPlan,
    ConnectedSessionEventPlanInputs, ConnectedSessionInboundApplyPlan,
    ConnectedSessionInboundPostApplyAction, ConnectedSessionInboundPostApplyPlan,
    ConnectedSessionLoopEventKind, ConnectedSessionProtocolPlan, ConnectedSessionRuntimeStepAction,
    ConnectedSessionRuntimeStepPlan, ConnectedSessionSharedExecutionInputs,
    ConnectedSessionStartupPlaylistDisposition,
};

pub fn connected_session_inbound_post_apply_plan_legacy_compatible(
    inbound_is_server_hello: bool,
    has_pending_chat_message_on_connect: bool,
    shared_playlists_enabled: bool,
) -> ConnectedSessionInboundPostApplyPlan {
    ConnectedSessionInboundPostApplyPlan {
        consume_pending_ready_at_start: inbound_is_server_hello,
        consume_pending_chat_message_on_connect: has_pending_chat_message_on_connect,
        run_reconnect_transition: true,
        run_controller_reidentify: true,
        run_controller_auth_notifications: true,
        run_chat_notifications: true,
        run_user_change_notifications: true,
        run_reconnect_state_restore: true,
        run_reconnect_playlist_restore: shared_playlists_enabled,
    }
}

pub fn connected_session_inbound_post_apply_actions_legacy_compatible(
    plan: ConnectedSessionInboundPostApplyPlan,
) -> Vec<ConnectedSessionInboundPostApplyAction> {
    let mut actions = Vec::new();

    if plan.consume_pending_ready_at_start {
        actions.push(ConnectedSessionInboundPostApplyAction::ConsumePendingReadyAtStart);
    }
    if plan.consume_pending_chat_message_on_connect {
        actions.push(ConnectedSessionInboundPostApplyAction::ConsumePendingChatMessageOnConnect);
    }
    if plan.run_reconnect_transition {
        actions.push(ConnectedSessionInboundPostApplyAction::RunReconnectTransition);
    }
    if plan.run_controller_reidentify {
        actions.push(ConnectedSessionInboundPostApplyAction::RunControllerReidentify);
    }
    if plan.run_controller_auth_notifications {
        actions.push(ConnectedSessionInboundPostApplyAction::RunControllerAuthNotifications);
    }
    if plan.run_chat_notifications {
        actions.push(ConnectedSessionInboundPostApplyAction::RunChatNotifications);
    }
    if plan.run_user_change_notifications {
        actions.push(ConnectedSessionInboundPostApplyAction::RunUserChangeNotifications);
    }
    if plan.run_reconnect_state_restore {
        actions.push(ConnectedSessionInboundPostApplyAction::RunReconnectStateRestore);
    }
    if plan.run_reconnect_playlist_restore {
        actions.push(ConnectedSessionInboundPostApplyAction::RunReconnectPlaylistRestore);
    }

    actions
}

pub fn connected_session_inbound_apply_plan_legacy_compatible(
    inbound_message_is_state: bool,
    outbound_state_sync_enabled: bool,
) -> ConnectedSessionInboundApplyPlan {
    if inbound_message_is_state {
        ConnectedSessionInboundApplyPlan {
            reconcile_inbound_state: true,
            apply_message_json_at: false,
            outbound_state_sync_enabled: true,
        }
    } else {
        ConnectedSessionInboundApplyPlan {
            reconcile_inbound_state: false,
            apply_message_json_at: true,
            outbound_state_sync_enabled,
        }
    }
}

pub fn connected_session_protocol_plan_legacy_compatible(
    event_kind: ConnectedSessionLoopEventKind,
    emitted_runtime_action: bool,
    shared_playlists_enabled: bool,
) -> ConnectedSessionProtocolPlan {
    match event_kind {
        ConnectedSessionLoopEventKind::InboundMessage => ConnectedSessionProtocolPlan {
            flush_runtime_protocol_lines: true,
            startup_playlist_disposition: if shared_playlists_enabled {
                ConnectedSessionStartupPlaylistDisposition::EmitIfAvailable
            } else {
                ConnectedSessionStartupPlaylistDisposition::DiscardIfPending
            },
        },
        ConnectedSessionLoopEventKind::AutoplayTick => ConnectedSessionProtocolPlan {
            flush_runtime_protocol_lines: true,
            startup_playlist_disposition: ConnectedSessionStartupPlaylistDisposition::LeavePending,
        },
        ConnectedSessionLoopEventKind::LocalInput => ConnectedSessionProtocolPlan {
            flush_runtime_protocol_lines: emitted_runtime_action,
            startup_playlist_disposition: ConnectedSessionStartupPlaylistDisposition::LeavePending,
        },
    }
}

pub fn connected_session_branch_plan_legacy_compatible(
    event_kind: ConnectedSessionLoopEventKind,
    emitted_runtime_action: bool,
    shared_playlists_enabled: bool,
    diagnostics: ConnectedSessionDiagnosticsPlan,
) -> ConnectedSessionBranchPlan {
    ConnectedSessionBranchPlan {
        run_protocol_before_runtime_steps: matches!(
            event_kind,
            ConnectedSessionLoopEventKind::LocalInput
        ),
        runtime_steps: connected_session_runtime_step_plan_legacy_compatible(event_kind),
        protocol: connected_session_protocol_plan_legacy_compatible(
            event_kind,
            emitted_runtime_action,
            shared_playlists_enabled,
        ),
        drain: connected_session_drain_plan_legacy_compatible(event_kind, diagnostics),
    }
}

pub fn connected_session_event_plan_legacy_compatible(
    event_kind: ConnectedSessionLoopEventKind,
    inputs: ConnectedSessionEventPlanInputs,
) -> ConnectedSessionEventPlan {
    ConnectedSessionEventPlan {
        inbound_post_apply: match event_kind {
            ConnectedSessionLoopEventKind::InboundMessage => {
                Some(connected_session_inbound_post_apply_plan_legacy_compatible(
                    inputs.inbound_is_server_hello,
                    inputs.has_pending_chat_message_on_connect,
                    inputs.shared_playlists_enabled,
                ))
            }
            ConnectedSessionLoopEventKind::AutoplayTick
            | ConnectedSessionLoopEventKind::LocalInput => None,
        },
        branch: connected_session_branch_plan_legacy_compatible(
            event_kind,
            inputs.emitted_runtime_action,
            inputs.shared_playlists_enabled,
            inputs.diagnostics,
        ),
    }
}

pub fn connected_session_event_execution_plan_legacy_compatible(
    event_kind: ConnectedSessionLoopEventKind,
    inputs: ConnectedSessionEventExecutionPlanInputs,
) -> ConnectedSessionEventExecutionPlan {
    ConnectedSessionEventExecutionPlan {
        inbound_apply: match event_kind {
            ConnectedSessionLoopEventKind::InboundMessage => {
                Some(connected_session_inbound_apply_plan_legacy_compatible(
                    inputs.inbound_message_is_state,
                    inputs.outbound_state_sync_enabled,
                ))
            }
            ConnectedSessionLoopEventKind::AutoplayTick
            | ConnectedSessionLoopEventKind::LocalInput => None,
        },
        event: connected_session_event_plan_legacy_compatible(event_kind, inputs.event),
    }
}

pub fn connected_session_inbound_message_event_execution_plan_legacy_compatible(
    inbound_is_server_hello: bool,
    has_pending_chat_message_on_connect: bool,
    inbound_message_is_state: bool,
    shared: ConnectedSessionSharedExecutionInputs,
) -> ConnectedSessionEventExecutionPlan {
    connected_session_event_execution_plan_legacy_compatible(
        ConnectedSessionLoopEventKind::InboundMessage,
        ConnectedSessionEventExecutionPlanInputs {
            event: ConnectedSessionEventPlanInputs {
                emitted_runtime_action: false,
                inbound_is_server_hello,
                has_pending_chat_message_on_connect,
                shared_playlists_enabled: shared.shared_playlists_enabled,
                diagnostics: shared.diagnostics,
            },
            inbound_message_is_state,
            outbound_state_sync_enabled: shared.outbound_state_sync_enabled,
        },
    )
}

pub fn connected_session_autoplay_tick_event_execution_plan_legacy_compatible(
    shared: ConnectedSessionSharedExecutionInputs,
) -> ConnectedSessionEventExecutionPlan {
    connected_session_event_execution_plan_legacy_compatible(
        ConnectedSessionLoopEventKind::AutoplayTick,
        ConnectedSessionEventExecutionPlanInputs {
            event: ConnectedSessionEventPlanInputs {
                emitted_runtime_action: false,
                inbound_is_server_hello: false,
                has_pending_chat_message_on_connect: false,
                shared_playlists_enabled: shared.shared_playlists_enabled,
                diagnostics: shared.diagnostics,
            },
            inbound_message_is_state: false,
            outbound_state_sync_enabled: shared.outbound_state_sync_enabled,
        },
    )
}

pub fn connected_session_player_coordination_tick_event_execution_plan_legacy_compatible(
    shared: ConnectedSessionSharedExecutionInputs,
) -> ConnectedSessionEventExecutionPlan {
    ConnectedSessionEventExecutionPlan {
        inbound_apply: None,
        event: ConnectedSessionEventPlan {
            inbound_post_apply: None,
            branch: ConnectedSessionBranchPlan {
                run_protocol_before_runtime_steps: false,
                runtime_steps: ConnectedSessionRuntimeStepPlan {
                    run_room_pause_sync: true,
                    run_readiness_unpause_attempt: false,
                    run_update_autoplay_check: false,
                    run_tick_autoplay: false,
                    run_desync_correction: false,
                    run_reconnect_state_restore_validation: true,
                    run_state_sync_heartbeat: false,
                    publish_pending_local_file_updates: true,
                },
                protocol: ConnectedSessionProtocolPlan {
                    flush_runtime_protocol_lines: true,
                    startup_playlist_disposition:
                        ConnectedSessionStartupPlaylistDisposition::LeavePending,
                },
                drain: ConnectedSessionDrainPlan {
                    flush_player_playback_diagnostics: shared.diagnostics.log_player_telemetry
                        || shared.diagnostics.log_player_drift,
                    reconnect_correction_diagnostics_format: shared
                        .diagnostics
                        .reconnect_correction_diagnostics_format,
                    flush_reconnect_notifications: true,
                    flush_controller_auth_notifications: false,
                    flush_chat_notifications: false,
                    flush_user_change_notifications: false,
                    flush_autoplay_notifications: false,
                    flush_file_difference_notifications: true,
                },
            },
        },
    }
}

pub fn connected_session_local_input_event_execution_plan_legacy_compatible(
    emitted_runtime_action: bool,
    shared: ConnectedSessionSharedExecutionInputs,
) -> ConnectedSessionEventExecutionPlan {
    connected_session_event_execution_plan_legacy_compatible(
        ConnectedSessionLoopEventKind::LocalInput,
        ConnectedSessionEventExecutionPlanInputs {
            event: ConnectedSessionEventPlanInputs {
                emitted_runtime_action,
                inbound_is_server_hello: false,
                has_pending_chat_message_on_connect: false,
                shared_playlists_enabled: shared.shared_playlists_enabled,
                diagnostics: shared.diagnostics,
            },
            inbound_message_is_state: false,
            outbound_state_sync_enabled: shared.outbound_state_sync_enabled,
        },
    )
}

pub fn connected_session_runtime_step_plan_legacy_compatible(
    event_kind: ConnectedSessionLoopEventKind,
) -> ConnectedSessionRuntimeStepPlan {
    match event_kind {
        ConnectedSessionLoopEventKind::InboundMessage => ConnectedSessionRuntimeStepPlan {
            run_room_pause_sync: true,
            run_readiness_unpause_attempt: true,
            run_update_autoplay_check: false,
            run_tick_autoplay: false,
            run_desync_correction: true,
            run_reconnect_state_restore_validation: true,
            run_state_sync_heartbeat: false,
            publish_pending_local_file_updates: true,
        },
        ConnectedSessionLoopEventKind::AutoplayTick => ConnectedSessionRuntimeStepPlan {
            run_room_pause_sync: true,
            run_readiness_unpause_attempt: false,
            run_update_autoplay_check: true,
            run_tick_autoplay: true,
            run_desync_correction: true,
            run_reconnect_state_restore_validation: true,
            run_state_sync_heartbeat: true,
            publish_pending_local_file_updates: true,
        },
        ConnectedSessionLoopEventKind::LocalInput => ConnectedSessionRuntimeStepPlan {
            run_room_pause_sync: false,
            run_readiness_unpause_attempt: false,
            run_update_autoplay_check: false,
            run_tick_autoplay: false,
            run_desync_correction: false,
            run_reconnect_state_restore_validation: true,
            run_state_sync_heartbeat: false,
            publish_pending_local_file_updates: false,
        },
    }
}

pub fn connected_session_runtime_step_actions_legacy_compatible(
    plan: ConnectedSessionRuntimeStepPlan,
    _outbound_state_sync_enabled: bool,
) -> Vec<ConnectedSessionRuntimeStepAction> {
    let mut actions = Vec::new();

    if plan.run_room_pause_sync {
        actions.push(ConnectedSessionRuntimeStepAction::RunRoomPauseSync);
    }
    if plan.run_readiness_unpause_attempt {
        actions.push(ConnectedSessionRuntimeStepAction::RunReadinessUnpauseAttempt);
    }
    if plan.run_update_autoplay_check {
        actions.push(ConnectedSessionRuntimeStepAction::RunUpdateAutoplayCheck);
    }
    if plan.run_tick_autoplay {
        actions.push(ConnectedSessionRuntimeStepAction::RunTickAutoplay);
    }
    if plan.run_desync_correction {
        actions.push(ConnectedSessionRuntimeStepAction::RunDesyncCorrection);
    }
    if plan.run_reconnect_state_restore_validation {
        actions.push(ConnectedSessionRuntimeStepAction::RunReconnectStateRestoreValidation);
    }
    if plan.run_state_sync_heartbeat {
        actions.push(ConnectedSessionRuntimeStepAction::RunStateSyncHeartbeat);
    }
    if plan.publish_pending_local_file_updates {
        actions.push(ConnectedSessionRuntimeStepAction::PublishPendingLocalFileUpdates);
    }
    actions
}

pub fn connected_session_drain_plan_legacy_compatible(
    event_kind: ConnectedSessionLoopEventKind,
    diagnostics: ConnectedSessionDiagnosticsPlan,
) -> ConnectedSessionDrainPlan {
    let flush_player_playback_diagnostics =
        diagnostics.log_player_telemetry || diagnostics.log_player_drift;

    let (
        flush_controller_auth_notifications,
        flush_chat_notifications,
        flush_user_change_notifications,
        flush_autoplay_notifications,
        flush_file_difference_notifications,
    ) = match event_kind {
        ConnectedSessionLoopEventKind::InboundMessage => (true, true, true, true, true),
        ConnectedSessionLoopEventKind::AutoplayTick => (false, false, false, true, true),
        ConnectedSessionLoopEventKind::LocalInput => (false, false, false, false, false),
    };

    ConnectedSessionDrainPlan {
        flush_player_playback_diagnostics,
        reconnect_correction_diagnostics_format: diagnostics
            .reconnect_correction_diagnostics_format,
        flush_reconnect_notifications: true,
        flush_controller_auth_notifications,
        flush_chat_notifications,
        flush_user_change_notifications,
        flush_autoplay_notifications,
        flush_file_difference_notifications,
    }
}

pub fn connected_session_drain_actions_legacy_compatible(
    plan: ConnectedSessionDrainPlan,
) -> Vec<ConnectedSessionDrainAction> {
    let mut actions = Vec::new();

    if plan.flush_player_playback_diagnostics {
        actions.push(ConnectedSessionDrainAction::FlushPlayerPlaybackDiagnostics);
    }
    if plan.flush_reconnect_notifications {
        actions.push(ConnectedSessionDrainAction::FlushReconnectNotifications);
    }
    if let Some(format) = plan.reconnect_correction_diagnostics_format {
        actions.push(ConnectedSessionDrainAction::FlushReconnectCorrectionDiagnostics(format));
    }
    if plan.flush_controller_auth_notifications {
        actions.push(ConnectedSessionDrainAction::FlushControllerAuthNotifications);
    }
    if plan.flush_chat_notifications {
        actions.push(ConnectedSessionDrainAction::FlushChatNotifications);
    }
    if plan.flush_user_change_notifications {
        actions.push(ConnectedSessionDrainAction::FlushUserChangeNotifications);
    }
    if plan.flush_autoplay_notifications {
        actions.push(ConnectedSessionDrainAction::FlushAutoplayNotifications);
    }
    if plan.flush_file_difference_notifications {
        actions.push(ConnectedSessionDrainAction::FlushFileDifferenceNotifications);
    }

    actions
}
