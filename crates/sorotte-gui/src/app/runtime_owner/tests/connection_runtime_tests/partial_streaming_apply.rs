use super::*;
use crate::app::runtime_owner::GuiCorePlayerConfigurationHealth;

fn active_option_properties(commands: &Arc<Mutex<Vec<serde_json::Value>>>) -> Vec<String> {
    commands
        .lock()
        .expect("active-network command log should not be poisoned")
        .iter()
        .map(|command| {
            command
                .get(1)
                .and_then(serde_json::Value::as_str)
                .expect("recorded active-network command should contain a property")
                .to_owned()
        })
        .collect()
}

#[test]
fn partial_active_option_apply_preserves_baseline_until_same_adapter_retry_completes() {
    const REJECTED_WRITE: usize = 3;

    let env_guard = TestEnvGuard::lock(&CONFIG_ROOT_ENV_LOCK);
    let ipc_path = r"\\.\pipe\sorotte-partial-streaming-retry-test";
    env_guard.set_var("SOROTTE_CLIENT_MPV_IPC_PATH", ipc_path);
    let settings = StoredClientSettingsMvp::default();
    let launch_state =
        GuiPersistedConfigRuntimeOwner::configured_player_launch_state_from_lookup_and_settings(
            &crate::app::startup_support::env_trimmed,
            Some(&settings),
        )
        .expect("explicit mpv launch state should resolve");
    let (ui_settings, desired_streaming_options) = match &launch_state {
        GuiPlayerLaunchRuntimeState::ExplicitMpvIpc {
            ui_settings,
            effective_streaming_options,
            ..
        } => ((**ui_settings).clone(), effective_streaming_options.clone()),
        _ => panic!("expected explicit mpv launch state"),
    };
    assert!(
        desired_streaming_options.len() > REJECTED_WRITE,
        "the regression needs successful writes both before and after the rejected option"
    );
    let mut previous_streaming_options = desired_streaming_options.clone();
    previous_streaming_options[0].effective_value = "previous-baseline".to_owned();
    let (adapter, commands) =
        sorotte_player_mpv::MpvAdapter::with_nth_active_network_option_rejection_test_ipc(
            ui_settings.clone(),
            REJECTED_WRITE,
        );
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_launch_state = launch_state;
    owner.player_apply_state.applied_streaming_options = Some(previous_streaming_options.clone());

    owner.complete_explicit_mpv_attachment_after_ipc_connect(
        ipc_path,
        &ui_settings,
        &desired_streaming_options,
        adapter,
    );

    let mut desired_properties = desired_streaming_options
        .iter()
        .map(|option| format!("file-local-options/{}", option.name))
        .collect::<Vec<_>>();
    desired_properties.sort();
    assert_eq!(
        active_option_properties(&commands),
        desired_properties[..REJECTED_WRITE],
        "the transport must accept a real prefix before rejecting exactly the Nth write"
    );
    assert_eq!(
        owner.player_apply_state.applied_streaming_options,
        Some(previous_streaming_options),
        "partial success must not promote the desired streaming baseline"
    );
    assert!(owner.player_apply_state.core_reapply_required);
    assert!(matches!(
        owner.core_player_configuration_health,
        GuiCorePlayerConfigurationHealth::StreamingDegraded {
            retryable_in_place: true,
            ..
        }
    ));
    let issue = owner
        .player_setup_runtime_snapshot_impl()
        .issue
        .expect("partial active-media apply should project a scoped issue");
    assert_eq!(
        issue.kind,
        crate::app::shell_state::GuiPlayerSetupIssueKind::PlayerSettingsDegraded
    );
    let original_player_address = match owner.player.as_ref() {
        Some(GuiOwnedPlayer::Mpv(player)) => &**player as *const sorotte_player_mpv::MpvAdapter,
        _ => panic!("healthy partial apply should retain mpv"),
    };
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&settings);

    handle.push_request(GuiRuntimeRequest::RetryPlayerSettings);
    let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert_eq!(
        owner.player.as_ref().and_then(|player| match player {
            GuiOwnedPlayer::Mpv(player) => {
                Some(&**player as *const sorotte_player_mpv::MpvAdapter)
            }
            _ => None,
        }),
        Some(original_player_address),
        "the in-place retry must reuse the healthy attached adapter"
    );
    let mut expected_retry_log = desired_properties[..REJECTED_WRITE].to_vec();
    expected_retry_log.extend(desired_properties.clone());
    assert_eq!(
        active_option_properties(&commands),
        expected_retry_log,
        "the retry must complete every desired write before the baseline is promoted"
    );
    assert_eq!(
        owner.player_apply_state.applied_streaming_options,
        Some(desired_streaming_options),
        "only the complete retry may promote the desired streaming baseline"
    );
    assert!(!owner.player_apply_state.core_reapply_required);
    assert!(matches!(
        owner.core_player_configuration_health,
        GuiCorePlayerConfigurationHealth::Ready
    ));
    assert!(owner.player_setup_runtime_snapshot_impl().issue.is_none());
    assert!(actions.iter().any(|action| matches!(
        action,
        GuiShellAction::PushTransientNotification {
            level: GuiTransientNotificationLevel::Success,
            message,
        } if message.contains("without restarting playback")
    )));

    env_guard.remove_var("SOROTTE_CLIENT_MPV_IPC_PATH");
}

#[test]
fn explicit_retry_defers_a_to_b_supersession_and_clears_degradation_after_b_applies() {
    let env_guard = TestEnvGuard::lock(&CONFIG_ROOT_ENV_LOCK);
    let ipc_path = r"\\.\pipe\sorotte-explicit-supersession-retry-test";
    env_guard.set_var("SOROTTE_CLIENT_MPV_IPC_PATH", ipc_path);
    let settings = StoredClientSettingsMvp::default();
    let launch_state =
        GuiPersistedConfigRuntimeOwner::configured_player_launch_state_from_lookup_and_settings(
            &crate::app::startup_support::env_trimmed,
            Some(&settings),
        )
        .expect("explicit mpv launch state should resolve");
    let (ui_settings, desired_streaming_options) = match &launch_state {
        GuiPlayerLaunchRuntimeState::ExplicitMpvIpc {
            ui_settings,
            effective_streaming_options,
            ..
        } => ((**ui_settings).clone(), effective_streaming_options.clone()),
        _ => panic!("expected explicit mpv launch state"),
    };
    let mut previous_streaming_options = desired_streaming_options.clone();
    previous_streaming_options[0].effective_value = "previous-baseline".to_owned();
    let (adapter, commands) =
        sorotte_player_mpv::MpvAdapter::with_active_network_media_supersession_test_ipc(
            ui_settings.clone(),
        );
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_launch_state = launch_state.clone();
    owner
        .player_apply_state
        .record_process_target_applied(&launch_state);
    owner.player_apply_state.applied_streaming_options = Some(previous_streaming_options);
    owner.player_apply_state.applied_mpv_ui_settings = Some(ui_settings.clone());
    owner.player_apply_state.acknowledged_bridge_settings = Some(ui_settings);
    owner.player_apply_state.core_reapply_required = true;
    owner.core_player_configuration_health = GuiCorePlayerConfigurationHealth::StreamingDegraded {
        reason: "retry the previous explicit apply".to_owned(),
        retryable_in_place: true,
        origin: crate::app::runtime_owner::GuiStreamingDegradationOrigin::ExplicitApply,
    };
    owner.player_unavailability_reason = Some("retry the previous explicit apply".to_owned());
    owner.player = Some(GuiOwnedPlayer::Mpv(Box::new(adapter)));
    let original_player_address = match owner.player.as_ref() {
        Some(GuiOwnedPlayer::Mpv(player)) => &**player as *const sorotte_player_mpv::MpvAdapter,
        _ => unreachable!(),
    };

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&settings);
    state.pending_apply_requirements =
        vec![GuiSettingApplyRequirement::PlayerSettingsRetryAvailable];
    handle.push_request(GuiRuntimeRequest::RetryPlayerSettings);
    let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert_eq!(
        owner.player.as_ref().and_then(|player| match player {
            GuiOwnedPlayer::Mpv(player) => {
                Some(&**player as *const sorotte_player_mpv::MpvAdapter)
            }
            _ => None,
        }),
        Some(original_player_address),
        "supersession must retain the attached adapter"
    );
    let mut desired_properties = desired_streaming_options
        .iter()
        .map(|option| format!("file-local-options/{}", option.name))
        .collect::<Vec<_>>();
    desired_properties.sort();
    let recorded_properties = active_option_properties(&commands);
    assert_eq!(
        &recorded_properties[recorded_properties.len() - desired_properties.len()..],
        desired_properties,
        "newer network B must accept the complete configured map"
    );
    assert_eq!(
        owner.player_apply_state.applied_streaming_options,
        Some(desired_streaming_options),
        "only B's ordered success may promote the desired baseline"
    );
    assert!(!owner.player_apply_state.core_reapply_required);
    assert!(!owner.player_apply_state.streaming_apply_awaiting_transition);
    assert!(matches!(
        owner.core_player_configuration_health,
        GuiCorePlayerConfigurationHealth::Ready
    ));
    assert!(owner.player_unavailability_reason.is_none());
    assert!(owner.player_setup_runtime_snapshot_impl().issue.is_none());
    assert!(state.pending_apply_requirements.is_empty());
    assert!(actions.iter().any(|action| matches!(
        action,
        GuiShellAction::ApplyPendingApplyRequirementsSnapshot(requirements)
            if requirements.is_empty()
    )));

    env_guard.remove_var("SOROTTE_CLIENT_MPV_IPC_PATH");
}

#[test]
fn independent_hook_degradation_reappears_after_successful_same_adapter_retry() {
    let env_guard = TestEnvGuard::lock(&CONFIG_ROOT_ENV_LOCK);
    let ipc_path = r"\\.\pipe\sorotte-hook-degradation-rearm-test";
    env_guard.set_var("SOROTTE_CLIENT_MPV_IPC_PATH", ipc_path);
    let settings = StoredClientSettingsMvp::default();
    let launch_state =
        GuiPersistedConfigRuntimeOwner::configured_player_launch_state_from_lookup_and_settings(
            &crate::app::startup_support::env_trimmed,
            Some(&settings),
        )
        .expect("explicit mpv launch state should resolve");
    let (ui_settings, desired_streaming_options) = match &launch_state {
        GuiPlayerLaunchRuntimeState::ExplicitMpvIpc {
            ui_settings,
            effective_streaming_options,
            ..
        } => ((**ui_settings).clone(), effective_streaming_options.clone()),
        _ => panic!("expected explicit mpv launch state"),
    };
    let (adapter, _) =
        sorotte_player_mpv::MpvAdapter::with_nth_active_network_option_rejection_test_ipc(
            ui_settings.clone(),
            usize::MAX,
        );
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_launch_state = launch_state;
    owner.complete_explicit_mpv_attachment_after_ipc_connect(
        ipc_path,
        &ui_settings,
        &desired_streaming_options,
        adapter,
    );
    let original_player_address = match owner.player.as_ref() {
        Some(GuiOwnedPlayer::Mpv(player)) => &**player as *const sorotte_player_mpv::MpvAdapter,
        _ => panic!("initial successful apply should retain mpv"),
    };
    assert_eq!(
        owner.player_apply_state.applied_streaming_options,
        Some(desired_streaming_options.clone())
    );

    let player = match owner.player.as_mut() {
        Some(GuiOwnedPlayer::Mpv(player)) => player,
        _ => unreachable!(),
    };
    player.inject_test_network_media_options_hook_degradation("first independent lease loss");
    player.inject_test_network_media_options_hook_degradation(
        "duplicate report while continuously degraded",
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&settings);
    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert_eq!(
        owner
            .player_setup_runtime_snapshot_impl()
            .issue
            .expect("first degradation should project a retry issue")
            .kind,
        crate::app::shell_state::GuiPlayerSetupIssueKind::PlayerSettingsDegraded
    );
    assert!(
        state
            .pending_apply_requirements
            .contains(&GuiSettingApplyRequirement::PlayerSettingsRetryAvailable)
    );
    assert_eq!(
        owner.player.as_ref().and_then(|player| match player {
            GuiOwnedPlayer::Mpv(player) => {
                Some(&**player as *const sorotte_player_mpv::MpvAdapter)
            }
            _ => None,
        }),
        Some(original_player_address)
    );

    handle.push_request(GuiRuntimeRequest::RetryPlayerSettings);
    let retry_actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(owner.player_setup_runtime_snapshot_impl().issue.is_none());
    assert!(state.pending_apply_requirements.is_empty());
    assert!(retry_actions.iter().any(|action| matches!(
        action,
        GuiShellAction::PushTransientNotification {
            level: GuiTransientNotificationLevel::Success,
            ..
        }
    )));

    let player = match owner.player.as_mut() {
        Some(GuiOwnedPlayer::Mpv(player)) => player,
        _ => unreachable!(),
    };
    player.inject_test_network_media_options_hook_degradation("second independent lease loss");
    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert_eq!(
        owner
            .player_setup_runtime_snapshot_impl()
            .issue
            .expect("second independent degradation should restore the retry issue")
            .kind,
        crate::app::shell_state::GuiPlayerSetupIssueKind::PlayerSettingsDegraded
    );
    assert!(
        state
            .pending_apply_requirements
            .contains(&GuiSettingApplyRequirement::PlayerSettingsRetryAvailable)
    );
    assert_eq!(
        owner.player.as_ref().and_then(|player| match player {
            GuiOwnedPlayer::Mpv(player) => {
                Some(&**player as *const sorotte_player_mpv::MpvAdapter)
            }
            _ => None,
        }),
        Some(original_player_address),
        "both scoped degradations must retain the same playback attachment"
    );

    env_guard.remove_var("SOROTTE_CLIENT_MPV_IPC_PATH");
}

#[test]
fn superseded_retry_keeps_baseline_issue_and_footer_until_authoritative_result() {
    let env_guard = TestEnvGuard::lock(&CONFIG_ROOT_ENV_LOCK);
    let ipc_path = r"\\.\pipe\sorotte-superseded-awaiting-hook-result-test";
    env_guard.set_var("SOROTTE_CLIENT_MPV_IPC_PATH", ipc_path);
    let settings = StoredClientSettingsMvp::default();
    let launch_state =
        GuiPersistedConfigRuntimeOwner::configured_player_launch_state_from_lookup_and_settings(
            &crate::app::startup_support::env_trimmed,
            Some(&settings),
        )
        .expect("explicit mpv launch state should resolve");
    let (ui_settings, desired_streaming_options) = match &launch_state {
        GuiPlayerLaunchRuntimeState::ExplicitMpvIpc {
            ui_settings,
            effective_streaming_options,
            ..
        } => ((**ui_settings).clone(), effective_streaming_options.clone()),
        _ => panic!("expected explicit mpv launch state"),
    };
    let mut previous_streaming_options = desired_streaming_options.clone();
    previous_streaming_options[0].effective_value = "previous-baseline".to_owned();
    let (adapter, _) =
        sorotte_player_mpv::MpvAdapter::with_nth_active_network_option_rejection_test_ipc(
            ui_settings.clone(),
            usize::MAX,
        );
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_launch_state = launch_state.clone();
    owner
        .player_apply_state
        .record_process_target_applied(&launch_state);
    owner.player_apply_state.applied_streaming_options = Some(previous_streaming_options.clone());
    owner.player_apply_state.core_reapply_required = true;
    owner.core_player_configuration_health = GuiCorePlayerConfigurationHealth::StreamingDegraded {
        reason: "waiting for the successor hook result".to_owned(),
        retryable_in_place: true,
        origin: crate::app::runtime_owner::GuiStreamingDegradationOrigin::ExplicitApply,
    };
    owner.player_unavailability_reason = Some("waiting for the successor hook result".to_owned());
    owner.player = Some(GuiOwnedPlayer::Mpv(Box::new(adapter)));

    owner.player_apply_state.mark_streaming_apply_superseded();

    assert_eq!(
        owner.player_apply_state.applied_streaming_options,
        Some(previous_streaming_options),
        "start/path observations without a hook result must not promote the desired baseline"
    );
    assert!(owner.player_apply_state.streaming_apply_awaiting_transition);
    assert_eq!(
        owner
            .player_setup_runtime_snapshot_impl()
            .issue
            .expect("the visible issue must remain until the hook result")
            .kind,
        crate::app::shell_state::GuiPlayerSetupIssueKind::PlayerSettingsDegraded
    );
    let projected_state = SorotteGuiShellAppState::from_stored_settings(&settings);
    assert!(
        owner
            .pending_apply_requirements_for_settings(&projected_state, &settings)
            .contains(&GuiSettingApplyRequirement::PlayerSettingsRetryAvailable),
        "the footer retry requirement must remain while the authoritative result is pending"
    );

    owner.record_network_media_transition_recovered();
    assert_eq!(
        owner.player_apply_state.applied_streaming_options,
        Some(desired_streaming_options)
    );
    assert!(!owner.player_apply_state.streaming_apply_awaiting_transition);
    assert!(owner.player_setup_runtime_snapshot_impl().issue.is_none());

    env_guard.remove_var("SOROTTE_CLIENT_MPV_IPC_PATH");
}

#[test]
fn same_map_retry_does_not_report_success_while_authoritative_result_is_pending() {
    let env_guard = TestEnvGuard::lock(&CONFIG_ROOT_ENV_LOCK);
    let ipc_path = r"\\.\pipe\sorotte-superseded-retry-notification-test";
    env_guard.set_var("SOROTTE_CLIENT_MPV_IPC_PATH", ipc_path);
    let settings = StoredClientSettingsMvp::default();
    let launch_state =
        GuiPersistedConfigRuntimeOwner::configured_player_launch_state_from_lookup_and_settings(
            &crate::app::startup_support::env_trimmed,
            Some(&settings),
        )
        .expect("explicit mpv launch state should resolve");
    let (ui_settings, desired_streaming_options) = match &launch_state {
        GuiPlayerLaunchRuntimeState::ExplicitMpvIpc {
            ui_settings,
            effective_streaming_options,
            ..
        } => ((**ui_settings).clone(), effective_streaming_options.clone()),
        _ => panic!("expected explicit mpv launch state"),
    };
    let (adapter, _) =
        sorotte_player_mpv::MpvAdapter::with_nth_active_network_option_rejection_test_ipc(
            ui_settings.clone(),
            usize::MAX,
        );
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_launch_state = launch_state.clone();
    owner
        .player_apply_state
        .record_process_target_applied(&launch_state);
    owner.player_apply_state.applied_streaming_options = Some(desired_streaming_options.clone());
    owner.player_apply_state.applied_mpv_ui_settings = Some(ui_settings.clone());
    owner.player_apply_state.acknowledged_bridge_settings = Some(ui_settings);
    owner.player_apply_state.acknowledged_bridge_generation = Some(1);
    owner.core_player_configuration_health = GuiCorePlayerConfigurationHealth::StreamingDegraded {
        reason: "waiting for the successor hook result".to_owned(),
        retryable_in_place: true,
        origin: crate::app::runtime_owner::GuiStreamingDegradationOrigin::ExplicitApply,
    };
    owner.player_unavailability_reason = Some("waiting for the successor hook result".to_owned());
    owner.player = Some(GuiOwnedPlayer::Mpv(Box::new(adapter)));
    owner.player_apply_state.mark_streaming_apply_superseded();
    assert!(
        !owner.apply_saved_player_settings_in_place(&settings),
        "save completion must not promote restart baselines while the hook result is pending"
    );

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&settings);
    state.pending_apply_requirements =
        vec![GuiSettingApplyRequirement::PlayerSettingsRetryAvailable];
    handle.push_request(GuiRuntimeRequest::RetryPlayerSettings);
    let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(owner.player_apply_state.streaming_apply_awaiting_transition);
    assert_eq!(
        owner.player_apply_state.applied_streaming_options,
        Some(desired_streaming_options),
        "a same-map retry must retain its baseline while waiting for the load result"
    );
    assert!(!owner.current_player_core_state_is_applied());
    assert!(
        state
            .pending_apply_requirements
            .contains(&GuiSettingApplyRequirement::PlayerSettingsRetryAvailable),
        "the footer requirement must remain until the authoritative result"
    );
    assert!(actions.iter().any(|action| matches!(
        action,
        GuiShellAction::PushTransientNotification {
            level: GuiTransientNotificationLevel::Error,
            message,
        } if message.contains("waiting for the successor hook result")
    )));
    assert!(!actions.iter().any(|action| matches!(
        action,
        GuiShellAction::PushTransientNotification {
            level: GuiTransientNotificationLevel::Success,
            message,
        } if message.contains("streaming settings were applied")
    )));

    env_guard.remove_var("SOROTTE_CLIENT_MPV_IPC_PATH");
}

#[test]
fn superseded_launch_completion_reports_pending_and_refreshes_footer() {
    let env_guard = TestEnvGuard::lock(&CONFIG_ROOT_ENV_LOCK);
    let ipc_path = r"\\.\pipe\sorotte-superseded-launch-notification-test";
    env_guard.set_var("SOROTTE_CLIENT_MPV_IPC_PATH", ipc_path);
    let settings = StoredClientSettingsMvp::default();
    let launch_state =
        GuiPersistedConfigRuntimeOwner::configured_player_launch_state_from_lookup_and_settings(
            &crate::app::startup_support::env_trimmed,
            Some(&settings),
        )
        .expect("explicit mpv launch state should resolve");
    let (ui_settings, desired_streaming_options) = match &launch_state {
        GuiPlayerLaunchRuntimeState::ExplicitMpvIpc {
            ui_settings,
            effective_streaming_options,
            ..
        } => ((**ui_settings).clone(), effective_streaming_options.clone()),
        _ => panic!("expected explicit mpv launch state"),
    };
    let (adapter, _) =
        sorotte_player_mpv::MpvAdapter::with_nth_active_network_option_rejection_test_ipc(
            ui_settings.clone(),
            usize::MAX,
        );
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_launch_state = launch_state.clone();
    owner
        .player_apply_state
        .record_process_target_applied(&launch_state);
    owner.player_apply_state.applied_streaming_options = Some(desired_streaming_options);
    owner.player_apply_state.applied_mpv_ui_settings = Some(ui_settings.clone());
    owner.player_apply_state.acknowledged_bridge_settings = Some(ui_settings);
    owner.player_apply_state.acknowledged_bridge_generation = Some(1);
    owner.core_player_configuration_health = GuiCorePlayerConfigurationHealth::StreamingDegraded {
        reason: "waiting for the successor hook result".to_owned(),
        retryable_in_place: true,
        origin: crate::app::runtime_owner::GuiStreamingDegradationOrigin::ExplicitApply,
    };
    owner.player_unavailability_reason = Some("waiting for the successor hook result".to_owned());
    owner.player = Some(GuiOwnedPlayer::Mpv(Box::new(adapter)));
    owner.player_apply_state.mark_streaming_apply_superseded();

    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&settings);
    state.pending_apply_requirements = vec![GuiSettingApplyRequirement::RestartPlayer];
    owner.finish_retry_player_launch_request(&handle, &mut state, &settings);
    let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(
        owner.player.is_some(),
        "superseded launch must retain playback"
    );
    assert!(owner.player_apply_state.streaming_apply_awaiting_transition);
    assert!(!owner.current_player_core_state_is_applied());
    assert!(
        state
            .pending_apply_requirements
            .contains(&GuiSettingApplyRequirement::PlayerSettingsRetryAvailable),
        "launch completion must refresh the footer to the pending hook requirement"
    );
    assert!(
        !state
            .pending_apply_requirements
            .contains(&GuiSettingApplyRequirement::RestartPlayer),
        "the stale restart requirement must not survive an attached superseded launch"
    );
    assert!(actions.iter().any(|action| matches!(
        action,
        GuiShellAction::PushTransientNotification {
            level: GuiTransientNotificationLevel::Error,
            message,
        } if message.contains("waiting for the successor hook result")
    )));
    assert!(!actions.iter().any(|action| matches!(
        action,
        GuiShellAction::PushTransientNotification {
            level: GuiTransientNotificationLevel::Success,
            message,
        } if message.contains("ready with the current player settings")
    )));

    env_guard.remove_var("SOROTTE_CLIENT_MPV_IPC_PATH");
}
