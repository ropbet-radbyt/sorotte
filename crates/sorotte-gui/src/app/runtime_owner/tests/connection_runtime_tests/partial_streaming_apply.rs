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
