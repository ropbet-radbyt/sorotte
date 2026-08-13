use super::super::*;
use crate::legacy_reconnect_diagnostics::ReconnectCorrectionDiagnosticsFormat;

#[test]
fn connected_session_drain_plan_legacy_compatible_enables_inbound_flushes() {
    let plan = connected_session_drain_plan_legacy_compatible(
        ConnectedSessionLoopEventKind::InboundMessage,
        ConnectedSessionDiagnosticsPlan {
            log_player_telemetry: false,
            log_player_drift: true,
            reconnect_correction_diagnostics_format: Some(
                ReconnectCorrectionDiagnosticsFormat::Text,
            ),
        },
    );
    assert!(plan.flush_player_playback_diagnostics);
    assert_eq!(
        plan.reconnect_correction_diagnostics_format,
        Some(ReconnectCorrectionDiagnosticsFormat::Text)
    );
    assert!(plan.flush_reconnect_notifications);
    assert!(plan.flush_controller_auth_notifications);
    assert!(plan.flush_chat_notifications);
    assert!(plan.flush_user_change_notifications);
    assert!(plan.flush_autoplay_notifications);
    assert!(plan.flush_file_difference_notifications);
}

#[test]
fn connected_session_drain_plan_legacy_compatible_specializes_tick_and_local_input() {
    let autoplay_tick_plan = connected_session_drain_plan_legacy_compatible(
        ConnectedSessionLoopEventKind::AutoplayTick,
        ConnectedSessionDiagnosticsPlan {
            log_player_telemetry: false,
            log_player_drift: false,
            reconnect_correction_diagnostics_format: None,
        },
    );
    assert!(!autoplay_tick_plan.flush_player_playback_diagnostics);
    assert!(autoplay_tick_plan.flush_autoplay_notifications);
    assert!(autoplay_tick_plan.flush_file_difference_notifications);
    assert!(!autoplay_tick_plan.flush_controller_auth_notifications);
    assert!(!autoplay_tick_plan.flush_chat_notifications);
    assert!(!autoplay_tick_plan.flush_user_change_notifications);

    let local_input_plan = connected_session_drain_plan_legacy_compatible(
        ConnectedSessionLoopEventKind::LocalInput,
        ConnectedSessionDiagnosticsPlan {
            log_player_telemetry: true,
            log_player_drift: false,
            reconnect_correction_diagnostics_format: None,
        },
    );
    assert!(local_input_plan.flush_player_playback_diagnostics);
    assert!(local_input_plan.flush_reconnect_notifications);
    assert!(!local_input_plan.flush_autoplay_notifications);
    assert!(!local_input_plan.flush_file_difference_notifications);
    assert!(!local_input_plan.flush_controller_auth_notifications);
    assert!(!local_input_plan.flush_chat_notifications);
    assert!(!local_input_plan.flush_user_change_notifications);
}

#[test]
fn connected_session_drain_actions_legacy_compatible_preserves_shared_ordering() {
    let actions = connected_session_drain_actions_legacy_compatible(ConnectedSessionDrainPlan {
        flush_player_playback_diagnostics: true,
        reconnect_correction_diagnostics_format: Some(ReconnectCorrectionDiagnosticsFormat::Text),
        flush_reconnect_notifications: true,
        flush_controller_auth_notifications: true,
        flush_chat_notifications: true,
        flush_user_change_notifications: true,
        flush_autoplay_notifications: true,
        flush_file_difference_notifications: true,
    });

    assert_eq!(
        actions,
        vec![
            ConnectedSessionDrainAction::FlushPlayerPlaybackDiagnostics,
            ConnectedSessionDrainAction::FlushReconnectNotifications,
            ConnectedSessionDrainAction::FlushReconnectCorrectionDiagnostics(
                ReconnectCorrectionDiagnosticsFormat::Text
            ),
            ConnectedSessionDrainAction::FlushControllerAuthNotifications,
            ConnectedSessionDrainAction::FlushChatNotifications,
            ConnectedSessionDrainAction::FlushUserChangeNotifications,
            ConnectedSessionDrainAction::FlushAutoplayNotifications,
            ConnectedSessionDrainAction::FlushFileDifferenceNotifications,
        ]
    );
}

#[test]
fn connected_session_drain_actions_legacy_compatible_omits_disabled_actions() {
    let actions = connected_session_drain_actions_legacy_compatible(ConnectedSessionDrainPlan {
        flush_player_playback_diagnostics: false,
        reconnect_correction_diagnostics_format: None,
        flush_reconnect_notifications: true,
        flush_controller_auth_notifications: false,
        flush_chat_notifications: false,
        flush_user_change_notifications: false,
        flush_autoplay_notifications: true,
        flush_file_difference_notifications: false,
    });

    assert_eq!(
        actions,
        vec![
            ConnectedSessionDrainAction::FlushReconnectNotifications,
            ConnectedSessionDrainAction::FlushAutoplayNotifications,
        ]
    );
}

#[test]
fn connected_session_inbound_post_apply_actions_legacy_compatible_preserves_shared_ordering() {
    let actions = connected_session_inbound_post_apply_actions_legacy_compatible(
        ConnectedSessionInboundPostApplyPlan {
            consume_pending_ready_at_start: true,
            consume_pending_chat_message_on_connect: true,
            run_reconnect_transition: true,
            run_controller_reidentify: true,
            run_controller_auth_notifications: true,
            run_chat_notifications: true,
            run_user_change_notifications: true,
            run_reconnect_state_restore: true,
            run_reconnect_playlist_restore: true,
        },
    );

    assert_eq!(
        actions,
        vec![
            ConnectedSessionInboundPostApplyAction::ConsumePendingReadyAtStart,
            ConnectedSessionInboundPostApplyAction::ConsumePendingChatMessageOnConnect,
            ConnectedSessionInboundPostApplyAction::RunReconnectTransition,
            ConnectedSessionInboundPostApplyAction::RunControllerReidentify,
            ConnectedSessionInboundPostApplyAction::RunControllerAuthNotifications,
            ConnectedSessionInboundPostApplyAction::RunChatNotifications,
            ConnectedSessionInboundPostApplyAction::RunUserChangeNotifications,
            ConnectedSessionInboundPostApplyAction::RunReconnectStateRestore,
            ConnectedSessionInboundPostApplyAction::RunReconnectPlaylistRestore,
        ]
    );
}

#[test]
fn connected_session_inbound_post_apply_actions_legacy_compatible_omits_disabled_actions() {
    let actions = connected_session_inbound_post_apply_actions_legacy_compatible(
        ConnectedSessionInboundPostApplyPlan {
            consume_pending_ready_at_start: false,
            consume_pending_chat_message_on_connect: false,
            run_reconnect_transition: true,
            run_controller_reidentify: false,
            run_controller_auth_notifications: false,
            run_chat_notifications: true,
            run_user_change_notifications: false,
            run_reconnect_state_restore: true,
            run_reconnect_playlist_restore: false,
        },
    );

    assert_eq!(
        actions,
        vec![
            ConnectedSessionInboundPostApplyAction::RunReconnectTransition,
            ConnectedSessionInboundPostApplyAction::RunChatNotifications,
            ConnectedSessionInboundPostApplyAction::RunReconnectStateRestore,
        ]
    );
}

#[test]
fn connected_session_runtime_step_plan_legacy_compatible_matches_inbound_and_tick_policy() {
    let inbound_plan = connected_session_runtime_step_plan_legacy_compatible(
        ConnectedSessionLoopEventKind::InboundMessage,
    );
    assert!(inbound_plan.run_room_pause_sync);
    assert!(inbound_plan.run_readiness_unpause_attempt);
    assert!(inbound_plan.run_desync_correction);
    assert!(inbound_plan.run_reconnect_state_restore_validation);
    assert!(inbound_plan.publish_pending_local_file_updates);
    assert!(!inbound_plan.run_update_autoplay_check);
    assert!(!inbound_plan.run_tick_autoplay);
    assert!(!inbound_plan.run_state_sync_heartbeat);

    let tick_plan = connected_session_runtime_step_plan_legacy_compatible(
        ConnectedSessionLoopEventKind::AutoplayTick,
    );
    assert!(tick_plan.run_room_pause_sync);
    assert!(tick_plan.run_update_autoplay_check);
    assert!(tick_plan.run_tick_autoplay);
    assert!(tick_plan.run_desync_correction);
    assert!(tick_plan.run_reconnect_state_restore_validation);
    assert!(tick_plan.run_state_sync_heartbeat);
    assert!(tick_plan.publish_pending_local_file_updates);
    assert!(!tick_plan.run_readiness_unpause_attempt);
}

#[test]
fn connected_session_runtime_step_plan_legacy_compatible_keeps_local_input_minimal() {
    let plan = connected_session_runtime_step_plan_legacy_compatible(
        ConnectedSessionLoopEventKind::LocalInput,
    );
    assert!(!plan.run_room_pause_sync);
    assert!(!plan.run_readiness_unpause_attempt);
    assert!(!plan.run_update_autoplay_check);
    assert!(!plan.run_tick_autoplay);
    assert!(!plan.run_desync_correction);
    assert!(plan.run_reconnect_state_restore_validation);
    assert!(!plan.run_state_sync_heartbeat);
    assert!(!plan.publish_pending_local_file_updates);
}

#[test]
fn connected_session_runtime_step_actions_legacy_compatible_preserve_execution_order() {
    let actions = connected_session_runtime_step_actions_legacy_compatible(
        ConnectedSessionRuntimeStepPlan {
            run_room_pause_sync: true,
            run_readiness_unpause_attempt: true,
            run_update_autoplay_check: true,
            run_tick_autoplay: true,
            run_desync_correction: true,
            run_reconnect_state_restore_validation: true,
            run_state_sync_heartbeat: true,
            publish_pending_local_file_updates: true,
        },
        true,
    );

    assert_eq!(
        actions,
        vec![
            ConnectedSessionRuntimeStepAction::RunRoomPauseSync,
            ConnectedSessionRuntimeStepAction::RunReadinessUnpauseAttempt,
            ConnectedSessionRuntimeStepAction::RunUpdateAutoplayCheck,
            ConnectedSessionRuntimeStepAction::RunTickAutoplay,
            ConnectedSessionRuntimeStepAction::RunDesyncCorrection,
            ConnectedSessionRuntimeStepAction::RunReconnectStateRestoreValidation,
            ConnectedSessionRuntimeStepAction::RunStateSyncHeartbeat,
            ConnectedSessionRuntimeStepAction::PublishPendingLocalFileUpdates,
        ]
    );
}

#[test]
fn connected_session_runtime_step_actions_preserve_the_public_heartbeat_action() {
    let actions = connected_session_runtime_step_actions_legacy_compatible(
        ConnectedSessionRuntimeStepPlan {
            run_room_pause_sync: false,
            run_readiness_unpause_attempt: false,
            run_update_autoplay_check: true,
            run_tick_autoplay: false,
            run_desync_correction: true,
            run_reconnect_state_restore_validation: true,
            run_state_sync_heartbeat: true,
            publish_pending_local_file_updates: false,
        },
        false,
    );

    assert_eq!(
        actions,
        vec![
            ConnectedSessionRuntimeStepAction::RunUpdateAutoplayCheck,
            ConnectedSessionRuntimeStepAction::RunDesyncCorrection,
            ConnectedSessionRuntimeStepAction::RunReconnectStateRestoreValidation,
            ConnectedSessionRuntimeStepAction::RunStateSyncHeartbeat,
        ]
    );
}

#[test]
fn connected_session_inbound_post_apply_plan_legacy_compatible_matches_policy() {
    let full_plan = connected_session_inbound_post_apply_plan_legacy_compatible(true, true, true);
    assert!(full_plan.consume_pending_ready_at_start);
    assert!(full_plan.consume_pending_chat_message_on_connect);
    assert!(full_plan.run_reconnect_transition);
    assert!(full_plan.run_controller_reidentify);
    assert!(full_plan.run_controller_auth_notifications);
    assert!(full_plan.run_chat_notifications);
    assert!(full_plan.run_user_change_notifications);
    assert!(full_plan.run_reconnect_state_restore);
    assert!(full_plan.run_reconnect_playlist_restore);

    let reduced_plan =
        connected_session_inbound_post_apply_plan_legacy_compatible(false, false, false);
    assert!(!reduced_plan.consume_pending_ready_at_start);
    assert!(!reduced_plan.consume_pending_chat_message_on_connect);
    assert!(reduced_plan.run_reconnect_transition);
    assert!(reduced_plan.run_controller_reidentify);
    assert!(reduced_plan.run_controller_auth_notifications);
    assert!(reduced_plan.run_chat_notifications);
    assert!(reduced_plan.run_user_change_notifications);
    assert!(reduced_plan.run_reconnect_state_restore);
    assert!(!reduced_plan.run_reconnect_playlist_restore);
}

#[test]
fn connected_session_inbound_apply_plan_legacy_compatible_specializes_state_messages() {
    assert_eq!(
        connected_session_inbound_apply_plan_legacy_compatible(true, false),
        ConnectedSessionInboundApplyPlan {
            reconcile_inbound_state: true,
            apply_message_json_at: false,
            outbound_state_sync_enabled: true,
        }
    );
    assert_eq!(
        connected_session_inbound_apply_plan_legacy_compatible(true, true),
        ConnectedSessionInboundApplyPlan {
            reconcile_inbound_state: true,
            apply_message_json_at: false,
            outbound_state_sync_enabled: true,
        }
    );
}

#[test]
fn connected_session_inbound_apply_plan_legacy_compatible_preserves_non_state_policy() {
    assert_eq!(
        connected_session_inbound_apply_plan_legacy_compatible(false, false),
        ConnectedSessionInboundApplyPlan {
            reconcile_inbound_state: false,
            apply_message_json_at: true,
            outbound_state_sync_enabled: false,
        }
    );
    assert_eq!(
        connected_session_inbound_apply_plan_legacy_compatible(false, true),
        ConnectedSessionInboundApplyPlan {
            reconcile_inbound_state: false,
            apply_message_json_at: true,
            outbound_state_sync_enabled: true,
        }
    );
}

#[test]
fn connected_session_protocol_plan_legacy_compatible_handles_inbound_playlist_policy() {
    assert_eq!(
        connected_session_protocol_plan_legacy_compatible(
            ConnectedSessionLoopEventKind::InboundMessage,
            false,
            true,
        ),
        ConnectedSessionProtocolPlan {
            flush_runtime_protocol_lines: true,
            startup_playlist_disposition:
                ConnectedSessionStartupPlaylistDisposition::EmitIfAvailable,
        }
    );
    assert_eq!(
        connected_session_protocol_plan_legacy_compatible(
            ConnectedSessionLoopEventKind::InboundMessage,
            false,
            false,
        ),
        ConnectedSessionProtocolPlan {
            flush_runtime_protocol_lines: true,
            startup_playlist_disposition:
                ConnectedSessionStartupPlaylistDisposition::DiscardIfPending,
        }
    );
}

#[test]
fn connected_session_protocol_plan_legacy_compatible_handles_tick_and_local_input() {
    assert_eq!(
        connected_session_protocol_plan_legacy_compatible(
            ConnectedSessionLoopEventKind::AutoplayTick,
            false,
            true,
        ),
        ConnectedSessionProtocolPlan {
            flush_runtime_protocol_lines: true,
            startup_playlist_disposition: ConnectedSessionStartupPlaylistDisposition::LeavePending,
        }
    );
    assert_eq!(
        connected_session_protocol_plan_legacy_compatible(
            ConnectedSessionLoopEventKind::LocalInput,
            false,
            true,
        ),
        ConnectedSessionProtocolPlan {
            flush_runtime_protocol_lines: false,
            startup_playlist_disposition: ConnectedSessionStartupPlaylistDisposition::LeavePending,
        }
    );
    assert_eq!(
        connected_session_protocol_plan_legacy_compatible(
            ConnectedSessionLoopEventKind::LocalInput,
            true,
            true,
        ),
        ConnectedSessionProtocolPlan {
            flush_runtime_protocol_lines: true,
            startup_playlist_disposition: ConnectedSessionStartupPlaylistDisposition::LeavePending,
        }
    );
}

#[test]
fn connected_session_branch_plan_legacy_compatible_assembles_inbound_policy() {
    assert_eq!(
        connected_session_branch_plan_legacy_compatible(
            ConnectedSessionLoopEventKind::InboundMessage,
            false,
            true,
            ConnectedSessionDiagnosticsPlan {
                log_player_telemetry: false,
                log_player_drift: true,
                reconnect_correction_diagnostics_format: Some(
                    ReconnectCorrectionDiagnosticsFormat::Text,
                ),
            },
        ),
        ConnectedSessionBranchPlan {
            run_protocol_before_runtime_steps: false,
            runtime_steps: connected_session_runtime_step_plan_legacy_compatible(
                ConnectedSessionLoopEventKind::InboundMessage,
            ),
            protocol: connected_session_protocol_plan_legacy_compatible(
                ConnectedSessionLoopEventKind::InboundMessage,
                false,
                true,
            ),
            drain: connected_session_drain_plan_legacy_compatible(
                ConnectedSessionLoopEventKind::InboundMessage,
                ConnectedSessionDiagnosticsPlan {
                    log_player_telemetry: false,
                    log_player_drift: true,
                    reconnect_correction_diagnostics_format: Some(
                        ReconnectCorrectionDiagnosticsFormat::Text,
                    ),
                },
            ),
        }
    );
}

#[test]
fn connected_session_branch_plan_legacy_compatible_assembles_local_input_policy() {
    assert_eq!(
        connected_session_branch_plan_legacy_compatible(
            ConnectedSessionLoopEventKind::LocalInput,
            true,
            true,
            ConnectedSessionDiagnosticsPlan {
                log_player_telemetry: true,
                log_player_drift: false,
                reconnect_correction_diagnostics_format: None,
            },
        ),
        ConnectedSessionBranchPlan {
            run_protocol_before_runtime_steps: true,
            runtime_steps: connected_session_runtime_step_plan_legacy_compatible(
                ConnectedSessionLoopEventKind::LocalInput,
            ),
            protocol: connected_session_protocol_plan_legacy_compatible(
                ConnectedSessionLoopEventKind::LocalInput,
                true,
                true,
            ),
            drain: connected_session_drain_plan_legacy_compatible(
                ConnectedSessionLoopEventKind::LocalInput,
                ConnectedSessionDiagnosticsPlan {
                    log_player_telemetry: true,
                    log_player_drift: false,
                    reconnect_correction_diagnostics_format: None,
                },
            ),
        }
    );
}

#[test]
fn connected_session_event_plan_legacy_compatible_assembles_inbound_post_apply_and_branch() {
    let diagnostics = ConnectedSessionDiagnosticsPlan {
        log_player_telemetry: false,
        log_player_drift: true,
        reconnect_correction_diagnostics_format: Some(ReconnectCorrectionDiagnosticsFormat::Text),
    };
    assert_eq!(
        connected_session_event_plan_legacy_compatible(
            ConnectedSessionLoopEventKind::InboundMessage,
            ConnectedSessionEventPlanInputs {
                emitted_runtime_action: false,
                inbound_is_server_hello: true,
                has_pending_chat_message_on_connect: true,
                shared_playlists_enabled: true,
                diagnostics,
            },
        ),
        ConnectedSessionEventPlan {
            inbound_post_apply: Some(connected_session_inbound_post_apply_plan_legacy_compatible(
                true, true, true
            ),),
            branch: connected_session_branch_plan_legacy_compatible(
                ConnectedSessionLoopEventKind::InboundMessage,
                false,
                true,
                diagnostics,
            ),
        }
    );
}

#[test]
fn connected_session_event_plan_legacy_compatible_keeps_non_inbound_post_apply_empty() {
    let diagnostics = ConnectedSessionDiagnosticsPlan {
        log_player_telemetry: true,
        log_player_drift: false,
        reconnect_correction_diagnostics_format: None,
    };
    assert_eq!(
        connected_session_event_plan_legacy_compatible(
            ConnectedSessionLoopEventKind::LocalInput,
            ConnectedSessionEventPlanInputs {
                emitted_runtime_action: true,
                inbound_is_server_hello: false,
                has_pending_chat_message_on_connect: true,
                shared_playlists_enabled: true,
                diagnostics,
            },
        ),
        ConnectedSessionEventPlan {
            inbound_post_apply: None,
            branch: connected_session_branch_plan_legacy_compatible(
                ConnectedSessionLoopEventKind::LocalInput,
                true,
                true,
                diagnostics,
            ),
        }
    );
}

#[test]
fn connected_session_event_execution_plan_legacy_compatible_combines_inbound_apply_and_event() {
    let event_inputs = ConnectedSessionEventPlanInputs {
        emitted_runtime_action: false,
        inbound_is_server_hello: true,
        has_pending_chat_message_on_connect: true,
        shared_playlists_enabled: true,
        diagnostics: ConnectedSessionDiagnosticsPlan {
            log_player_telemetry: false,
            log_player_drift: true,
            reconnect_correction_diagnostics_format: Some(
                ReconnectCorrectionDiagnosticsFormat::Text,
            ),
        },
    };
    assert_eq!(
        connected_session_event_execution_plan_legacy_compatible(
            ConnectedSessionLoopEventKind::InboundMessage,
            ConnectedSessionEventExecutionPlanInputs {
                event: event_inputs,
                inbound_message_is_state: true,
                outbound_state_sync_enabled: false,
            },
        ),
        ConnectedSessionEventExecutionPlan {
            inbound_apply: Some(connected_session_inbound_apply_plan_legacy_compatible(
                true, false
            ),),
            event: connected_session_event_plan_legacy_compatible(
                ConnectedSessionLoopEventKind::InboundMessage,
                event_inputs,
            ),
        }
    );
}

#[test]
fn connected_session_event_execution_plan_legacy_compatible_keeps_non_inbound_apply_empty() {
    let event_inputs = ConnectedSessionEventPlanInputs {
        emitted_runtime_action: true,
        inbound_is_server_hello: false,
        has_pending_chat_message_on_connect: false,
        shared_playlists_enabled: true,
        diagnostics: ConnectedSessionDiagnosticsPlan {
            log_player_telemetry: true,
            log_player_drift: false,
            reconnect_correction_diagnostics_format: None,
        },
    };
    assert_eq!(
        connected_session_event_execution_plan_legacy_compatible(
            ConnectedSessionLoopEventKind::LocalInput,
            ConnectedSessionEventExecutionPlanInputs {
                event: event_inputs,
                inbound_message_is_state: true,
                outbound_state_sync_enabled: false,
            },
        ),
        ConnectedSessionEventExecutionPlan {
            inbound_apply: None,
            event: connected_session_event_plan_legacy_compatible(
                ConnectedSessionLoopEventKind::LocalInput,
                event_inputs,
            ),
        }
    );
}

#[test]
fn connected_session_inbound_message_event_execution_plan_legacy_compatible_packs_inputs() {
    let shared = ConnectedSessionSharedExecutionInputs {
        shared_playlists_enabled: true,
        diagnostics: ConnectedSessionDiagnosticsPlan {
            log_player_telemetry: false,
            log_player_drift: true,
            reconnect_correction_diagnostics_format: Some(
                ReconnectCorrectionDiagnosticsFormat::Text,
            ),
        },
        outbound_state_sync_enabled: false,
    };
    assert_eq!(
        connected_session_inbound_message_event_execution_plan_legacy_compatible(
            true, true, true, shared,
        ),
        connected_session_event_execution_plan_legacy_compatible(
            ConnectedSessionLoopEventKind::InboundMessage,
            ConnectedSessionEventExecutionPlanInputs {
                event: ConnectedSessionEventPlanInputs {
                    emitted_runtime_action: false,
                    inbound_is_server_hello: true,
                    has_pending_chat_message_on_connect: true,
                    shared_playlists_enabled: true,
                    diagnostics: shared.diagnostics,
                },
                inbound_message_is_state: true,
                outbound_state_sync_enabled: false,
            },
        ),
    );
}

#[test]
fn connected_session_autoplay_tick_event_execution_plan_legacy_compatible_packs_inputs() {
    let shared = ConnectedSessionSharedExecutionInputs {
        shared_playlists_enabled: true,
        diagnostics: ConnectedSessionDiagnosticsPlan {
            log_player_telemetry: true,
            log_player_drift: false,
            reconnect_correction_diagnostics_format: None,
        },
        outbound_state_sync_enabled: true,
    };
    assert_eq!(
        connected_session_autoplay_tick_event_execution_plan_legacy_compatible(shared),
        connected_session_event_execution_plan_legacy_compatible(
            ConnectedSessionLoopEventKind::AutoplayTick,
            ConnectedSessionEventExecutionPlanInputs {
                event: ConnectedSessionEventPlanInputs {
                    emitted_runtime_action: false,
                    inbound_is_server_hello: false,
                    has_pending_chat_message_on_connect: false,
                    shared_playlists_enabled: true,
                    diagnostics: shared.diagnostics,
                },
                inbound_message_is_state: false,
                outbound_state_sync_enabled: true,
            },
        ),
    );
}

#[test]
fn connected_session_local_input_event_execution_plan_legacy_compatible_packs_inputs() {
    let shared = ConnectedSessionSharedExecutionInputs {
        shared_playlists_enabled: false,
        diagnostics: ConnectedSessionDiagnosticsPlan {
            log_player_telemetry: true,
            log_player_drift: true,
            reconnect_correction_diagnostics_format: None,
        },
        outbound_state_sync_enabled: true,
    };
    assert_eq!(
        connected_session_local_input_event_execution_plan_legacy_compatible(true, shared),
        connected_session_event_execution_plan_legacy_compatible(
            ConnectedSessionLoopEventKind::LocalInput,
            ConnectedSessionEventExecutionPlanInputs {
                event: ConnectedSessionEventPlanInputs {
                    emitted_runtime_action: true,
                    inbound_is_server_hello: false,
                    has_pending_chat_message_on_connect: false,
                    shared_playlists_enabled: false,
                    diagnostics: shared.diagnostics,
                },
                inbound_message_is_state: false,
                outbound_state_sync_enabled: true,
            },
        ),
    );
}
