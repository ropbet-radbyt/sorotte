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

fn assert_gui_failure_text_is_sanitized(
    surface: &str,
    text: &str,
    canary: &str,
    source_path: &str,
    resolved_target: &str,
) {
    assert!(
        !text.contains(canary),
        "{surface} leaked the credential canary: {text}"
    );
    assert!(
        !text.contains(source_path),
        "{surface} leaked the raw source target: {text}"
    );
    assert!(
        !text.contains(resolved_target),
        "{surface} leaked the raw resolved target: {text}"
    );
}

fn align_attached_test_player_with_unconfigured_settings(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    settings: &StoredClientSettingsMvp,
) {
    let launch_state =
        GuiPersistedConfigRuntimeOwner::configured_player_launch_state_from_lookup_and_settings(
            &crate::app::startup_support::env_trimmed,
            Some(settings),
        )
        .expect("test environment should contain a valid player configuration");
    owner.player_launch_state = launch_state.clone();
    owner.record_fully_applied_player_launch_state(&launch_state);
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

    let player = match owner.player.as_mut() {
        Some(GuiOwnedPlayer::Mpv(player)) => player,
        _ => unreachable!(),
    };
    player.inject_test_network_media_options_hook_recovery();
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

#[test]
fn hook_degradation_survives_idle_and_local_media_until_positive_hook_recovery() {
    let _env_guard = TestEnvGuard::lock(&CONFIG_ROOT_ENV_LOCK);
    let ipc_path = r"\\.\pipe\sorotte-hook-health-media-policy-separation-test";
    let lookup = |name: &str| (name == "SOROTTE_CLIENT_MPV_IPC_PATH").then(|| ipc_path.to_owned());
    let settings = StoredClientSettingsMvp::default();
    let launch_state =
        GuiPersistedConfigRuntimeOwner::configured_player_launch_state_from_lookup_and_settings(
            &lookup,
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
    align_attached_test_player_with_unconfigured_settings(&mut owner, &settings);
    let original_player_address = match owner.player.as_ref() {
        Some(GuiOwnedPlayer::Mpv(player)) => &**player as *const sorotte_player_mpv::MpvAdapter,
        _ => panic!("initial successful apply should retain mpv"),
    };
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&settings);
    owner.player_apply_state.mark_streaming_apply_superseded();

    let player = match owner.player.as_mut() {
        Some(GuiOwnedPlayer::Mpv(player)) => player,
        _ => unreachable!(),
    };
    player.inject_test_network_media_options_hook_degradation("first independent hook lease loss");
    player.inject_test_network_media_options_hook_degradation(
        "duplicate report while the same hook remains degraded",
    );
    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let initial_issue = owner
        .player_setup_runtime_snapshot_impl()
        .issue
        .expect("hook degradation should project a scoped setup issue");
    assert!(
        initial_issue
            .message
            .contains("first independent hook lease loss")
    );
    assert!(
        !initial_issue
            .message
            .contains("duplicate report while the same hook remains degraded"),
        "a duplicate degradation must be suppressed while hook health stays degraded"
    );
    assert!(
        owner.player_apply_state.streaming_apply_awaiting_transition,
        "hook degradation must preserve an independent authoritative-policy wait"
    );

    let player = match owner.player.as_mut() {
        Some(GuiOwnedPlayer::Mpv(player)) => player,
        _ => unreachable!(),
    };
    player.inject_test_network_media_options_no_active_media();
    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        owner.player_setup_runtime_snapshot_impl().issue.is_some(),
        "terminal idle may resolve media policy state but must not recover hook health"
    );
    assert!(state.player_setup_issue.is_some());
    assert!(
        !owner.player_apply_state.streaming_apply_awaiting_transition,
        "terminal idle should resolve the media-policy wait without recovering the hook"
    );
    assert!(
        state
            .pending_apply_requirements
            .contains(&GuiSettingApplyRequirement::PlayerSettingsRetryAvailable)
    );

    let player = match owner.player.as_mut() {
        Some(GuiOwnedPlayer::Mpv(player)) => player,
        _ => unreachable!(),
    };
    player.inject_test_network_media_options_local_media_unchanged();
    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        owner.player_setup_runtime_snapshot_impl().issue.is_some(),
        "local-media observation may resolve media policy state but must not recover hook health"
    );
    assert!(state.player_setup_issue.is_some());

    let player = match owner.player.as_mut() {
        Some(GuiOwnedPlayer::Mpv(player)) => player,
        _ => unreachable!(),
    };
    player.inject_test_network_media_options_hook_recovery();
    let recovery_actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(owner.player_setup_runtime_snapshot_impl().issue.is_none());
    assert!(state.player_setup_issue.is_none());
    assert!(
        !owner.player_apply_state.core_reapply_required,
        "positive hook recovery should clear the owner's scoped core retry flag"
    );
    assert!(owner.current_player_core_state_is_applied());
    assert!(
        !state
            .pending_apply_requirements
            .contains(&GuiSettingApplyRequirement::PlayerSettingsRetryAvailable),
        "positive hook recovery must clear its scoped footer remediation in the same pump: requirements={:?}, recovery_actions={recovery_actions:?}",
        state.pending_apply_requirements,
    );

    owner.player_apply_state.mark_streaming_apply_superseded();
    let player = match owner.player.as_mut() {
        Some(GuiOwnedPlayer::Mpv(player)) => player,
        _ => unreachable!(),
    };
    player.inject_test_network_media_options_hook_degradation("second independent hook lease loss");
    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let rearmed_issue = owner
        .player_setup_runtime_snapshot_impl()
        .issue
        .expect("a new post-recovery degradation must be surfaced");
    assert!(
        rearmed_issue
            .message
            .contains("second independent hook lease loss")
    );
    assert!(state.player_setup_issue.is_some());
    assert!(
        owner.player_apply_state.streaming_apply_awaiting_transition,
        "a later independent hook degradation must preserve the new policy wait"
    );
    assert_eq!(
        owner.player.as_ref().and_then(|player| match player {
            GuiOwnedPlayer::Mpv(player) => {
                Some(&**player as *const sorotte_player_mpv::MpvAdapter)
            }
            _ => None,
        }),
        Some(original_player_address),
        "scoped hook-health changes must retain the attached player"
    );

    let player = match owner.player.as_mut() {
        Some(GuiOwnedPlayer::Mpv(player)) => player,
        _ => unreachable!(),
    };
    player.inject_test_network_media_options_hook_recovery();
    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(owner.player_setup_runtime_snapshot_impl().issue.is_none());
    assert!(
        owner.player_apply_state.streaming_apply_awaiting_transition,
        "hook-first recovery must not resolve the independent media-policy wait"
    );
    assert!(
        state
            .pending_apply_requirements
            .contains(&GuiSettingApplyRequirement::PlayerSettingsRetryAvailable)
    );

    let player = match owner.player.as_mut() {
        Some(GuiOwnedPlayer::Mpv(player)) => player,
        _ => unreachable!(),
    };
    player.inject_test_network_media_options_local_media_unchanged();
    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(!owner.player_apply_state.streaming_apply_awaiting_transition);
    assert!(owner.player_setup_runtime_snapshot_impl().issue.is_none());
    assert!(
        !state
            .pending_apply_requirements
            .contains(&GuiSettingApplyRequirement::PlayerSettingsRetryAvailable)
    );
}

#[test]
fn overlapping_hook_and_media_policy_failures_recover_independently() {
    let _env_guard = TestEnvGuard::lock(&CONFIG_ROOT_ENV_LOCK);
    let ipc_path = r"\\.\pipe\sorotte-overlapping-streaming-failures-test";
    let lookup = |name: &str| (name == "SOROTTE_CLIENT_MPV_IPC_PATH").then(|| ipc_path.to_owned());
    let settings = StoredClientSettingsMvp::default();
    let launch_state =
        GuiPersistedConfigRuntimeOwner::configured_player_launch_state_from_lookup_and_settings(
            &lookup,
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
    align_attached_test_player_with_unconfigured_settings(&mut owner, &settings);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&settings);

    let player = match owner.player.as_mut() {
        Some(GuiOwnedPlayer::Mpv(player)) => player,
        _ => unreachable!(),
    };
    player.inject_test_network_media_options_hook_degradation("overlapping hook lease loss");
    player.inject_test_network_media_options_policy_failure(
        880,
        "https://media.example/source.m3u8",
        "https://cdn.example/resolved.m3u8",
    );
    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(owner.network_options_hook_failure_reason.is_some());
    assert!(matches!(
        owner.core_player_configuration_health,
        GuiCorePlayerConfigurationHealth::StreamingDegraded {
            origin: crate::app::runtime_owner::GuiStreamingDegradationOrigin::AuthoritativeMediaTransition,
            ..
        }
    ));

    let player = match owner.player.as_mut() {
        Some(GuiOwnedPlayer::Mpv(player)) => player,
        _ => unreachable!(),
    };
    player.inject_test_network_media_options_hook_recovery();
    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let policy_issue = owner
        .player_setup_runtime_snapshot_impl()
        .issue
        .expect("recovering the hook must leave the overlapping media-policy issue");
    assert!(policy_issue.message.contains("hook load 880"));
    assert!(owner.network_options_hook_failure_reason.is_none());

    let player = match owner.player.as_mut() {
        Some(GuiOwnedPlayer::Mpv(player)) => player,
        _ => unreachable!(),
    };
    player.inject_test_network_media_options_hook_degradation("second overlapping hook lease loss");
    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(owner.network_options_hook_failure_reason.is_some());
    assert!(
        owner
            .player_setup_runtime_snapshot_impl()
            .issue
            .is_some_and(|issue| issue.message.contains("hook load 880")),
        "the already-projected media-policy issue should retain precedence"
    );

    let player = match owner.player.as_mut() {
        Some(GuiOwnedPlayer::Mpv(player)) => player,
        _ => unreachable!(),
    };
    player.inject_test_network_media_options_no_active_media();
    let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    let restored_hook_issue = owner
        .player_setup_runtime_snapshot_impl()
        .issue
        .expect("clearing media policy must restore the overlapping hook issue");
    assert!(
        restored_hook_issue
            .message
            .contains("second overlapping hook lease loss")
    );
    assert!(state.player_setup_issue.is_some());
    assert!(
        state
            .pending_apply_requirements
            .contains(&GuiSettingApplyRequirement::PlayerSettingsRetryAvailable)
    );
}

#[test]
fn credential_bearing_media_targets_never_reach_gui_issue_notification_or_chat_surfaces() {
    let _env_guard = TestEnvGuard::lock(&CONFIG_ROOT_ENV_LOCK);
    let ipc_path = r"\\.\pipe\sorotte-streaming-failure-redaction-test";
    let lookup = |name: &str| (name == "SOROTTE_CLIENT_MPV_IPC_PATH").then(|| ipc_path.to_owned());
    let settings = StoredClientSettingsMvp::default();
    let launch_state =
        GuiPersistedConfigRuntimeOwner::configured_player_launch_state_from_lookup_and_settings(
            &lookup,
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
    align_attached_test_player_with_unconfigured_settings(&mut owner, &settings);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&settings);

    let cases = [
        (
            "userinfo",
            "GUI_USERINFO_CANARY",
            "https://viewer:GUI_USERINFO_CANARY@media.example/source.m3u8",
            "https://viewer:GUI_USERINFO_CANARY@cdn.example/resolved.m3u8",
            "source: HTTPS",
            "resolved target: HTTPS",
        ),
        (
            "signature query",
            "GUI_SIG_CANARY",
            "https://media.example/source.m3u8?sig=GUI_SIG_CANARY",
            "https://cdn.example/resolved.m3u8?sig=GUI_SIG_CANARY",
            "source: HTTPS",
            "resolved target: HTTPS",
        ),
        (
            "authorization query",
            "GUI_AUTH_CANARY",
            "http://media.example/source.m3u8?auth=GUI_AUTH_CANARY",
            "https://cdn.example/resolved.m3u8?authorization=GUI_AUTH_CANARY",
            "source: HTTP",
            "resolved target: HTTPS",
        ),
        (
            "AWS signing query",
            "GUI_AWS_CANARY",
            "https://media.example/source.m3u8?X-Amz-Credential=GUI_AWS_CANARY&X-Amz-Signature=GUI_AWS_CANARY",
            "https://cdn.example/resolved.m3u8?X-Amz-Security-Token=GUI_AWS_CANARY",
            "source: HTTPS",
            "resolved target: HTTPS",
        ),
        (
            "nested EDL target",
            "GUI_EDL_CANARY",
            "edl://https://media.example/one.m3u8?sig=GUI_EDL_CANARY;https://media.example/two.m3u8",
            "edl://https://cdn.example/one.m3u8?auth=GUI_EDL_CANARY;https://cdn.example/two.m3u8",
            "source: EDL",
            "resolved target: EDL",
        ),
        (
            "local path rewrite",
            "GUI_LOCAL_PATH_CANARY",
            r"C:\Users\GUI_LOCAL_PATH_CANARY\private\source.mkv",
            "file:///C:/Users/GUI_LOCAL_PATH_CANARY/private/resolved.mkv",
            "source: local path",
            "resolved target: file URL",
        ),
    ];

    for (index, (label, canary, source_path, resolved_target, source_kind, resolved_kind)) in
        cases.into_iter().enumerate()
    {
        let load_sequence = 700 + index as u64;
        let player = match owner.player.as_mut() {
            Some(GuiOwnedPlayer::Mpv(player)) => player,
            _ => unreachable!(),
        };
        player.inject_test_network_media_options_policy_failure(
            load_sequence,
            source_path,
            resolved_target,
        );
        let _ = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

        let issue = owner
            .player_setup_runtime_snapshot_impl()
            .issue
            .expect("policy failure should project a scoped setup issue");
        assert_eq!(
            issue.kind,
            crate::app::shell_state::GuiPlayerSetupIssueKind::PlayerSettingsDegraded
        );
        assert!(
            issue
                .message
                .contains(&format!("hook load {load_sequence}"))
        );
        assert!(issue.message.contains(source_kind));
        assert!(issue.message.contains(resolved_kind));
        assert_gui_failure_text_is_sanitized(
            &format!("{label} owner issue"),
            &issue.message,
            canary,
            source_path,
            resolved_target,
        );
        let projected_issue = state
            .player_setup_issue
            .as_ref()
            .expect("policy failure should reach the shell issue projection");
        assert_gui_failure_text_is_sanitized(
            &format!("{label} shell issue"),
            &projected_issue.message,
            canary,
            source_path,
            resolved_target,
        );

        owner.finish_retry_player_launch_request(&handle, &mut state, &settings);
        let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
        let notification_message = actions
            .iter()
            .find_map(|action| match action {
                GuiShellAction::PushTransientNotification {
                    level: GuiTransientNotificationLevel::Error,
                    message,
                } => Some(message.as_str()),
                _ => None,
            })
            .expect("retry should surface the sanitized failure as an error notification");
        assert_gui_failure_text_is_sanitized(
            &format!("{label} notification action"),
            notification_message,
            canary,
            source_path,
            resolved_target,
        );
        let chat_message = actions
            .iter()
            .find_map(|action| match action {
                GuiShellAction::AnnounceSystemChatEvent(message) => Some(message.as_str()),
                _ => None,
            })
            .expect("retry should surface the sanitized failure in system chat");
        assert_gui_failure_text_is_sanitized(
            &format!("{label} chat action"),
            chat_message,
            canary,
            source_path,
            resolved_target,
        );
        let stored_notification = state
            .notifications
            .last()
            .expect("notification action should be applied to shell state");
        assert_gui_failure_text_is_sanitized(
            &format!("{label} stored notification"),
            &stored_notification.message,
            canary,
            source_path,
            resolved_target,
        );
        let stored_chat = state
            .main_window
            .chat
            .last()
            .expect("system-chat action should be applied to shell state");
        assert_gui_failure_text_is_sanitized(
            &format!("{label} stored system chat"),
            &stored_chat.message,
            canary,
            source_path,
            resolved_target,
        );
    }
}
