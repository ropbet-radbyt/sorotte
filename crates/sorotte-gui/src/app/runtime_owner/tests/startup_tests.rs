use super::*;
use crate::app::runtime_owner::{
    GuiCorePlayerConfigurationHealth, GuiPlayerProcessTarget, GuiStreamingDegradationOrigin,
};
use sorotte_client_app::app_boundary::state::EffectiveMpvStreamingOption;

fn managed_mpv_test_child() -> std::process::Child {
    use std::process::{Command, Stdio};

    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("powershell");
        command.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"]);
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 30"]);
        command
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("managed-mpv test child should spawn")
}

fn managed_mpv_test_guard() -> crate::app::mpv_launch::ManagedMpvProcessGuard {
    crate::app::mpv_launch::ManagedMpvProcessGuard::from_test_child(managed_mpv_test_child())
}

fn managed_mpv_test_guard_with_drop_observer(
    drop_observer: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> crate::app::mpv_launch::ManagedMpvProcessGuard {
    crate::app::mpv_launch::ManagedMpvProcessGuard::from_test_child_with_drop_observer(
        managed_mpv_test_child(),
        drop_observer,
    )
}

fn acknowledgement_timeout_health() -> sorotte_player_mpv::SorotteBridgeHealth {
    sorotte_player_mpv::SorotteBridgeHealth::Degraded(sorotte_player_mpv::SorotteBridgeFailure {
        kind: sorotte_player_mpv::SorotteBridgeFailureKind::AcknowledgementTimeout,
        reason: "test bridge acknowledgement timed out".to_owned(),
    })
}

fn terminal_cleanup_commands(
    commands: &std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
) -> Vec<serde_json::Value> {
    commands
        .lock()
        .expect("terminal cleanup command log should not be poisoned")
        .clone()
}

fn assert_original_osd_restored_before_bridge_release(commands: &[serde_json::Value]) {
    assert_eq!(commands.len(), 3, "GUI cleanup should queue three commands");
    assert_eq!(
        commands[0],
        serde_json::json!(["set_property", "osd-align-y", "top"])
    );
    assert_eq!(
        commands[1],
        serde_json::json!(["set_property", "osd-margin-y", 16])
    );
    assert_eq!(commands[2][2], "sorotte_syncplayintf_release");
}

#[test]
fn explicit_mpv_discovery_failure_uses_shared_completion_and_retains_core_playback() {
    let ui_settings = sorotte_player_mpv::LegacySyncplayUiSettings {
        chat_move_osd: false,
        ..sorotte_player_mpv::LegacySyncplayUiSettings::default()
    };
    let launch_state = GuiPlayerLaunchRuntimeState::ExplicitMpvIpc {
        ipc_path: r"\\.\pipe\sorotte-degraded-test".to_owned(),
        ui_settings: Box::new(ui_settings.clone()),
        effective_streaming_options: Vec::new(),
    };
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_launch_state = launch_state.clone();
    owner.record_fully_applied_player_launch_state(&launch_state);
    let adapter = sorotte_player_mpv::MpvAdapter::with_rejected_sorotte_bridge_discovery_test_ipc(
        ui_settings.clone(),
    );
    owner
        .complete_mpv_attachment_after_core_configuration(adapter, None, &ui_settings)
        .expect("healthy core IPC should retain an explicit player after discovery fails");

    assert!(owner.player.is_some(), "bridge failure must retain mpv");
    assert!(owner.player_unavailability_reason.is_none());
    assert!(!owner.player_apply_state.core_reapply_required);
    assert!(matches!(
        owner.player_integration_health,
        GuiPlayerIntegrationHealth::BridgeDegraded { ref reason, .. }
            if reason.contains("discover")
    ));
    assert!(!owner.current_player_launch_state_is_applied());
    assert!(owner.current_player_core_state_is_applied());
    let player = owner.player.as_mut().expect("retained mpv should exist");
    player
        .open_file("degraded-playback.mkv")
        .expect("open must remain available while bridge is degraded");
    player
        .set_paused(false)
        .expect("pause must remain available while bridge is degraded");
    player
        .set_position(42.0)
        .expect("seek must remain available while bridge is degraded");

    let snapshot = owner.player_setup_runtime_snapshot_impl();
    assert!(snapshot.issue.is_some_and(|issue| {
        issue.kind == crate::app::shell_state::GuiPlayerSetupIssueKind::BridgeDegraded
            && issue.retry_available
    }));
}

#[test]
fn initial_explicit_mpv_streaming_rejection_retains_core_player_and_continues_optional_setup() {
    let ipc_path = r"\\.\pipe\sorotte-streaming-degraded-test";
    let ui_settings = sorotte_player_mpv::LegacySyncplayUiSettings::default();
    let desired_streaming_options = vec![EffectiveMpvStreamingOption {
        name: "cache-secs".to_owned(),
        configured_value: "30".to_owned(),
        effective_value: "75".to_owned(),
        overridden_by_advanced_arguments: true,
    }];
    let previous_streaming_options = vec![EffectiveMpvStreamingOption {
        name: "cache-secs".to_owned(),
        configured_value: "15".to_owned(),
        effective_value: "15".to_owned(),
        overridden_by_advanced_arguments: false,
    }];
    let launch_state = GuiPlayerLaunchRuntimeState::ExplicitMpvIpc {
        ipc_path: ipc_path.to_owned(),
        ui_settings: Box::new(ui_settings.clone()),
        effective_streaming_options: desired_streaming_options.clone(),
    };
    let adapter =
        sorotte_player_mpv::MpvAdapter::with_first_active_network_option_rejection_test_ipc(
            ui_settings.clone(),
        );
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_launch_state = launch_state.clone();
    owner.player_apply_state.applied_streaming_options = Some(previous_streaming_options.clone());

    owner.complete_explicit_mpv_attachment_after_ipc_connect(
        ipc_path,
        &ui_settings,
        &desired_streaming_options,
        adapter,
    );

    assert_eq!(
        owner.player_apply_state.applied_process_target,
        Some(GuiPlayerProcessTarget::ExplicitMpvIpc {
            ipc_path: ipc_path.to_owned(),
        }),
        "a successful IPC connection must promote the process target independently"
    );
    assert_eq!(
        owner.player_apply_state.applied_streaming_options,
        Some(previous_streaming_options),
        "a partial active-file apply must preserve the previous streaming baseline"
    );
    assert!(owner.player_apply_state.core_reapply_required);
    assert!(!owner.current_player_core_state_is_applied());
    assert!(matches!(
        owner.core_player_configuration_health,
        GuiCorePlayerConfigurationHealth::StreamingDegraded {
            retryable_in_place: true,
            ..
        }
    ));
    assert!(matches!(
        owner.player_integration_health,
        GuiPlayerIntegrationHealth::Ready
    ));
    assert_eq!(
        owner.player_apply_state.applied_mpv_ui_settings,
        Some(ui_settings.clone()),
        "optional mpv UI setup must continue after the scoped streaming failure"
    );
    assert_eq!(
        owner.player_apply_state.acknowledged_bridge_settings,
        Some(ui_settings),
        "optional Lua setup must continue after the scoped streaming failure"
    );
    assert!(
        owner
            .player_apply_state
            .acknowledged_bridge_generation
            .is_some(),
        "optional Lua setup should retain its exact acknowledgement generation"
    );
    assert!(
        owner
            .player_unavailability_reason
            .as_deref()
            .is_some_and(|reason| {
                reason.contains("player streaming settings could not be applied")
                    && reason.contains("invalid parameter")
            }),
        "optional setup must not clear the scoped core error"
    );

    let issue = owner
        .player_setup_runtime_snapshot_impl()
        .issue
        .expect("a retained partial streaming failure should remain visible");
    assert_eq!(
        issue.kind,
        crate::app::shell_state::GuiPlayerSetupIssueKind::PlayerSettingsDegraded
    );
    assert!(issue.retry_available);
    assert_ne!(
        issue.kind,
        crate::app::shell_state::GuiPlayerSetupIssueKind::IpcAttachFailed
    );
    let mut projected_state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    projected_state.player_setup_issue = Some(issue);
    assert_eq!(
        projected_state.player_setup_retry_action(),
        GuiShellAction::RetryPlayerSettings,
        "the degraded-settings issue must route to the in-place retry action"
    );

    let player = owner
        .player
        .as_mut()
        .expect("healthy IPC must retain the attached player");
    player
        .open_file("https://media.example.test/next.m3u8")
        .expect("network media open must remain available");
    player
        .set_paused(true)
        .expect("pause must remain available");
    player
        .set_position(42.0)
        .expect("seek must remain available");
    assert_eq!(
        player.take_transport_telemetry_update(),
        None,
        "an accepted load without start-file must not fabricate a physical transport projection"
    );
    let _ = player.take_playback_telemetry_update();
    let retained_player_address = match player {
        GuiOwnedPlayer::Mpv(player) => {
            assert!(
                player.is_connected(),
                "playback telemetry polling must preserve the healthy IPC attachment"
            );
            &**player as *const sorotte_player_mpv::MpvAdapter
        }
        _ => panic!("fixture should retain mpv"),
    };
    owner.ensure_configured_player_attached();
    assert_eq!(
        owner.player.as_ref().and_then(|player| match player {
            GuiOwnedPlayer::Mpv(player) => {
                Some(&**player as *const sorotte_player_mpv::MpvAdapter)
            }
            _ => None,
        }),
        Some(retained_player_address),
        "on-demand actions must reuse the healthy retained adapter"
    );
}

#[test]
fn initial_explicit_mpv_streaming_failure_remains_fatal_when_ipc_is_unhealthy() {
    let ipc_path = r"\\.\pipe\sorotte-streaming-disconnect-test";
    let ui_settings = sorotte_player_mpv::LegacySyncplayUiSettings::default();
    let desired_streaming_options = vec![EffectiveMpvStreamingOption {
        name: "cache-secs".to_owned(),
        configured_value: "30".to_owned(),
        effective_value: "75".to_owned(),
        overridden_by_advanced_arguments: true,
    }];
    let launch_state = GuiPlayerLaunchRuntimeState::ExplicitMpvIpc {
        ipc_path: ipc_path.to_owned(),
        ui_settings: Box::new(ui_settings.clone()),
        effective_streaming_options: desired_streaming_options.clone(),
    };
    let adapter =
        sorotte_player_mpv::MpvAdapter::with_first_active_network_option_rejection_test_ipc(
            ui_settings.clone(),
        );
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_launch_state = launch_state;

    owner.complete_explicit_mpv_attachment_with_active_apply_for_test(
        ipc_path,
        &ui_settings,
        &desired_streaming_options,
        adapter,
        |adapter| {
            adapter.mark_test_ipc_unhealthy("test transport disconnected during active apply");
            Err("test transport disconnected during active apply".to_owned())
        },
    );

    assert!(
        owner.player.is_none(),
        "an unhealthy adapter must not be retained"
    );
    assert!(owner.player_apply_state.core_reapply_required);
    assert!(owner.player_apply_state.applied_streaming_options.is_none());
    assert!(owner.player_apply_state.applied_mpv_ui_settings.is_none());
    assert!(
        owner
            .player_apply_state
            .acknowledged_bridge_settings
            .is_none(),
        "optional setup must not run after fatal core transport loss"
    );
    assert!(
        owner
            .player_unavailability_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("became unavailable"))
    );
    let issue = owner
        .player_setup_runtime_snapshot_impl()
        .issue
        .expect("fatal IPC loss should remain visible");
    assert_eq!(
        issue.kind,
        crate::app::shell_state::GuiPlayerSetupIssueKind::IpcAttachFailed
    );
}

#[test]
fn player_settings_retry_request_reuses_attached_adapter_and_clears_streaming_degradation() {
    let env_guard = TestEnvGuard::lock(&CONFIG_ROOT_ENV_LOCK);
    let ipc_path = r"\\.\pipe\sorotte-streaming-in-place-retry-test";
    env_guard.set_var("SOROTTE_CLIENT_MPV_IPC_PATH", ipc_path);
    let settings = StoredClientSettingsMvp::default();
    let launch_state =
        GuiPersistedConfigRuntimeOwner::configured_player_launch_state_from_lookup_and_settings(
            &|name| match name {
                "SOROTTE_CLIENT_MPV_IPC_PATH" => Some(ipc_path.to_owned()),
                _ => None,
            },
            Some(&settings),
        )
        .expect("explicit mpv launch state should resolve");
    let (ui_settings, effective_streaming_options) = match &launch_state {
        GuiPlayerLaunchRuntimeState::ExplicitMpvIpc {
            ui_settings,
            effective_streaming_options,
            ..
        } => ((**ui_settings).clone(), effective_streaming_options.clone()),
        _ => panic!("expected explicit mpv launch state"),
    };
    assert!(
        !effective_streaming_options.is_empty(),
        "the regression requires a real active-media option write"
    );
    let adapter =
        sorotte_player_mpv::MpvAdapter::with_first_active_network_option_rejection_test_ipc(
            ui_settings.clone(),
        );
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_launch_state = launch_state;
    owner.complete_explicit_mpv_attachment_after_ipc_connect(
        ipc_path,
        &ui_settings,
        &effective_streaming_options,
        adapter,
    );
    assert!(owner.player_apply_state.core_reapply_required);
    let original_player_address = match owner.player.as_ref() {
        Some(GuiOwnedPlayer::Mpv(player)) => &**player as *const sorotte_player_mpv::MpvAdapter,
        _ => panic!("healthy rejected adapter should remain attached"),
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
        "the streaming retry must not detach or replace healthy mpv"
    );
    assert!(!owner.player_apply_state.core_reapply_required);
    assert_eq!(
        owner
            .player_apply_state
            .applied_streaming_options
            .as_deref(),
        owner.player_launch_state.effective_mpv_streaming_options(),
        "a successful retry should promote only the now-accepted streaming baseline"
    );
    assert!(matches!(
        owner.core_player_configuration_health,
        GuiCorePlayerConfigurationHealth::Ready
    ));
    assert!(owner.player_unavailability_reason.is_none());
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
fn managed_mpv_option_value_url_does_not_create_spurious_streaming_degradation() {
    let env_guard = TestEnvGuard::lock(&CONFIG_ROOT_ENV_LOCK);
    env_guard.remove_var("SOROTTE_CLIENT_MPV_IPC_PATH");
    env_guard.remove_var("SOROTTE_MPV_IPC_PATH");
    env_guard.remove_var("SOROTTE_GUI_ENABLE_TEST_PLAYER");

    let player_path = "C:/Players/mpv.exe";
    let mut per_player_arguments = std::collections::BTreeMap::new();
    per_player_arguments.insert(
        player_path.to_owned(),
        vec![
            "--script-opts".to_owned(),
            "integration-source=https://example.test/value".to_owned(),
        ],
    );
    let settings = StoredClientSettingsMvp {
        player_path: Some(player_path.to_owned()),
        per_player_arguments: Some(per_player_arguments),
        streaming_read_ahead_seconds: Some(9.0),
        ..StoredClientSettingsMvp::default()
    };
    let launch_state =
        GuiPersistedConfigRuntimeOwner::configured_player_launch_state_from_lookup_and_settings(
            &|_| None,
            Some(&settings),
        )
        .expect("managed option-value launch state should resolve");
    let config = match &launch_state {
        GuiPlayerLaunchRuntimeState::ManagedMpv(config) => (**config).clone(),
        _ => panic!("expected managed mpv launch state"),
    };
    let desired_streaming_options = config.effective_streaming_options.clone();
    let (adapter, commands) =
        sorotte_player_mpv::MpvAdapter::with_delayed_active_network_media_test_ipc(
            config.ui_settings.clone(),
        );
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_launch_state = launch_state;

    owner.complete_managed_mpv_attachment_after_ipc_connect(
        &config,
        adapter,
        managed_mpv_test_guard(),
    );

    assert_eq!(
        owner.player_apply_state.applied_streaming_options,
        Some(desired_streaming_options),
        "configuring future network-media options is complete even while mpv is idle"
    );
    assert!(!owner.player_apply_state.core_reapply_required);
    assert!(matches!(
        owner.core_player_configuration_health,
        GuiCorePlayerConfigurationHealth::Ready
    ));
    assert!(owner.player.is_some());
    assert!(owner.managed_mpv_process.is_some());
    assert!(
        commands
            .lock()
            .expect("delayed active-media command log should not be poisoned")
            .is_empty(),
        "no file-local options should be written while mpv has no active media"
    );
    assert!(owner.player_unavailability_reason.is_none());
}

#[test]
fn managed_mpv_applies_streaming_options_when_local_launch_item_advances_to_queued_network_item() {
    let ui_settings = sorotte_player_mpv::LegacySyncplayUiSettings::default();
    let desired_streaming_options = vec![
        EffectiveMpvStreamingOption {
            name: "cache-secs".to_owned(),
            configured_value: "30".to_owned(),
            effective_value: "75".to_owned(),
            overridden_by_advanced_arguments: true,
        },
        EffectiveMpvStreamingOption {
            name: "cache-pause-wait".to_owned(),
            configured_value: "3".to_owned(),
            effective_value: "5".to_owned(),
            overridden_by_advanced_arguments: true,
        },
    ];
    let config = crate::app::mpv_launch::ManagedMpvLaunchConfig {
        requested_player_path: "mpv".to_owned(),
        program: PathBuf::from("mpv"),
        effective_streaming_options: desired_streaming_options.clone(),
        extra_args: vec![
            "C:/media/local-intro.mkv".to_owned(),
            "https://media.example.test/main-stream.m3u8".to_owned(),
        ],
        ui_settings: ui_settings.clone(),
    };
    let launch_state = GuiPlayerLaunchRuntimeState::ManagedMpv(Box::new(config.clone()));
    let (adapter, commands, transition_trigger) =
        sorotte_player_mpv::MpvAdapter::with_external_network_media_transition_test_ipc(
            ui_settings,
        );
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_launch_state = launch_state;

    owner.complete_managed_mpv_attachment_after_ipc_connect(
        &config,
        adapter,
        managed_mpv_test_guard(),
    );

    assert_eq!(
        owner.player_apply_state.applied_streaming_options,
        Some(desired_streaming_options.clone()),
        "a local active item should still install the policy for future network media"
    );
    assert!(
        commands
            .lock()
            .expect("transition command log should not be poisoned")
            .is_empty(),
        "local media must retain the user's ordinary mpv option values"
    );

    transition_trigger.store(true, std::sync::atomic::Ordering::SeqCst);
    owner.refresh_player_state_impl();

    let applied_options = commands
        .lock()
        .expect("transition command log should not be poisoned")
        .clone();
    assert_eq!(
        applied_options,
        vec![
            serde_json::json!(["set_property", "file-local-options/cache-pause-wait", "5"]),
            serde_json::json!(["set_property", "file-local-options/cache-secs", "75"]),
        ],
        "an externally advanced queued network item must receive every configured file-local option"
    );
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some("https://media.example.test/main-stream.m3u8"),
        "the runtime pump should observe the same authoritative network transition"
    );
    assert!(owner.player.is_some());
    assert!(owner.managed_mpv_process.is_some());
    assert!(!owner.player_apply_state.core_reapply_required);
    assert!(matches!(
        owner.core_player_configuration_health,
        GuiCorePlayerConfigurationHealth::Ready
    ));
    assert!(owner.player_unavailability_reason.is_none());
}

#[test]
fn managed_mpv_surfaces_retryable_degradation_when_network_transition_option_is_rejected() {
    let ui_settings = sorotte_player_mpv::LegacySyncplayUiSettings::default();
    let desired_streaming_options = vec![EffectiveMpvStreamingOption {
        name: "cache-secs".to_owned(),
        configured_value: "30".to_owned(),
        effective_value: "75".to_owned(),
        overridden_by_advanced_arguments: true,
    }];
    let config = crate::app::mpv_launch::ManagedMpvLaunchConfig {
        requested_player_path: "mpv".to_owned(),
        program: PathBuf::from("mpv"),
        effective_streaming_options: desired_streaming_options.clone(),
        extra_args: vec![
            "C:/media/local-intro.mkv".to_owned(),
            "https://media.example.test/main-stream.m3u8".to_owned(),
        ],
        ui_settings: ui_settings.clone(),
    };
    let launch_state = GuiPlayerLaunchRuntimeState::ManagedMpv(Box::new(config.clone()));
    let (adapter, commands, transition_trigger) =
        sorotte_player_mpv::MpvAdapter::with_rejected_external_network_media_transition_test_ipc(
            ui_settings,
        );
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_launch_state = launch_state;

    owner.complete_managed_mpv_attachment_after_ipc_connect(
        &config,
        adapter,
        managed_mpv_test_guard(),
    );
    transition_trigger.store(true, std::sync::atomic::Ordering::SeqCst);
    owner.refresh_player_state_impl();

    assert_eq!(
        commands
            .lock()
            .expect("rejected transition command log should not be poisoned")
            .as_slice(),
        [serde_json::json!([
            "set_property",
            "file-local-options/cache-secs",
            "75"
        ])]
    );
    assert!(
        owner.player.is_some(),
        "a healthy option rejection must retain playback"
    );
    assert!(owner.managed_mpv_process.is_some());
    assert_eq!(
        owner.player_apply_state.applied_streaming_options,
        Some(desired_streaming_options),
        "the configured policy baseline remains known while its active-file application needs retry"
    );
    assert!(owner.player_apply_state.core_reapply_required);
    assert!(matches!(
        owner.core_player_configuration_health,
        GuiCorePlayerConfigurationHealth::StreamingDegraded {
            retryable_in_place: true,
            ref reason,
            origin: GuiStreamingDegradationOrigin::AuthoritativeMediaTransition,
        } if reason.contains("switched to network media")
            && reason.contains("invalid parameter")
    ));
    assert!(
        owner
            .player_unavailability_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("configured streaming settings"))
    );
}

#[test]
fn later_successful_network_transition_clears_only_transition_origin_degradation() {
    let ui_settings = sorotte_player_mpv::LegacySyncplayUiSettings::default();
    let desired_streaming_options = vec![EffectiveMpvStreamingOption {
        name: "cache-secs".to_owned(),
        configured_value: "30".to_owned(),
        effective_value: "75".to_owned(),
        overridden_by_advanced_arguments: true,
    }];
    let config = crate::app::mpv_launch::ManagedMpvLaunchConfig {
        requested_player_path: "mpv".to_owned(),
        program: PathBuf::from("mpv"),
        effective_streaming_options: desired_streaming_options,
        extra_args: vec![
            "C:/media/local-intro.mkv".to_owned(),
            "https://media.example.test/main-stream.m3u8".to_owned(),
        ],
        ui_settings: ui_settings.clone(),
    };
    let launch_state = GuiPlayerLaunchRuntimeState::ManagedMpv(Box::new(config.clone()));
    let (adapter, _commands, transition_trigger) =
        sorotte_player_mpv::MpvAdapter::with_external_network_media_transition_test_ipc(
            ui_settings,
        );
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_launch_state = launch_state;
    owner.complete_managed_mpv_attachment_after_ipc_connect(
        &config,
        adapter,
        managed_mpv_test_guard(),
    );

    let transition_failure = "the previous network generation rejected streaming settings";
    owner.mark_network_media_transition_apply_failed(transition_failure.to_owned());
    transition_trigger.store(true, std::sync::atomic::Ordering::Release);
    owner.refresh_player_state_impl();

    assert!(!owner.player_apply_state.core_reapply_required);
    assert!(matches!(
        owner.core_player_configuration_health,
        GuiCorePlayerConfigurationHealth::Ready
    ));
    assert!(owner.player_unavailability_reason.is_none());

    let explicit_failure = "an explicit settings apply still needs a user retry";
    owner.player_apply_state.core_reapply_required = true;
    owner.core_player_configuration_health = GuiCorePlayerConfigurationHealth::StreamingDegraded {
        reason: explicit_failure.to_owned(),
        retryable_in_place: true,
        origin: GuiStreamingDegradationOrigin::ExplicitApply,
    };
    owner.player_unavailability_reason = Some(explicit_failure.to_owned());

    owner.record_network_media_transition_recovered();

    assert!(owner.player_apply_state.core_reapply_required);
    assert!(matches!(
        owner.core_player_configuration_health,
        GuiCorePlayerConfigurationHealth::StreamingDegraded {
            origin: GuiStreamingDegradationOrigin::ExplicitApply,
            ..
        }
    ));
    assert_eq!(
        owner.player_unavailability_reason.as_deref(),
        Some(explicit_failure)
    );
}

#[test]
fn managed_mpv_positional_network_media_rejection_retains_player_guard_and_optional_setup() {
    let ui_settings = sorotte_player_mpv::LegacySyncplayUiSettings::default();
    let desired_streaming_options = vec![EffectiveMpvStreamingOption {
        name: "cache-secs".to_owned(),
        configured_value: "30".to_owned(),
        effective_value: "75".to_owned(),
        overridden_by_advanced_arguments: true,
    }];
    let previous_streaming_options = vec![EffectiveMpvStreamingOption {
        name: "cache-secs".to_owned(),
        configured_value: "15".to_owned(),
        effective_value: "15".to_owned(),
        overridden_by_advanced_arguments: false,
    }];
    let positional_media = "https://media.example.test/active.m3u8";
    let config = crate::app::mpv_launch::ManagedMpvLaunchConfig {
        requested_player_path: "mpv".to_owned(),
        program: PathBuf::from("mpv"),
        effective_streaming_options: desired_streaming_options.clone(),
        extra_args: vec!["--profile=syncplay".to_owned(), positional_media.to_owned()],
        ui_settings: ui_settings.clone(),
    };
    assert_eq!(
        config.extra_args.last().map(String::as_str),
        Some(positional_media),
        "the regression requires mpv to receive network media as a positional launch argument"
    );
    let launch_state = GuiPlayerLaunchRuntimeState::ManagedMpv(Box::new(config.clone()));
    let adapter =
        sorotte_player_mpv::MpvAdapter::with_first_active_network_option_rejection_test_ipc(
            ui_settings.clone(),
        );
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_launch_state = launch_state;
    owner.player_apply_state.applied_streaming_options = Some(previous_streaming_options.clone());

    owner.complete_managed_mpv_attachment_after_ipc_connect(
        &config,
        adapter,
        managed_mpv_test_guard(),
    );

    assert_eq!(
        owner.player_apply_state.applied_process_target,
        Some(GuiPlayerProcessTarget::ManagedMpv {
            requested_player_path: "mpv".to_owned(),
            program: PathBuf::from("mpv"),
            extra_args: config.extra_args.clone(),
        }),
        "spawn plus IPC attachment must promote the managed process target independently"
    );
    assert_eq!(
        owner.player_apply_state.applied_streaming_options,
        Some(previous_streaming_options),
        "a rejected active-media write must not promote desired streaming settings"
    );
    assert!(owner.player_apply_state.core_reapply_required);
    assert!(matches!(
        owner.core_player_configuration_health,
        GuiCorePlayerConfigurationHealth::StreamingDegraded {
            retryable_in_place: true,
            ..
        }
    ));
    assert!(
        owner.player.is_some(),
        "healthy managed IPC must be retained"
    );
    assert!(
        owner.managed_mpv_process.is_some(),
        "the live managed process guard must move into the retained owner"
    );
    assert!(
        owner
            .managed_mpv_process
            .as_mut()
            .expect("managed guard should remain owned")
            .try_wait()
            .expect("managed child status should be readable")
            .is_none(),
        "a healthy server-side settings rejection must not terminate managed mpv"
    );
    assert!(
        owner
            .player_unavailability_reason
            .as_deref()
            .is_some_and(|reason| {
                reason.contains("player streaming settings could not be applied")
                    && reason.contains("invalid parameter")
            }),
        "the active-media rejection must remain visible after optional setup"
    );
    assert_eq!(
        owner.player_apply_state.applied_mpv_ui_settings,
        Some(ui_settings.clone()),
        "optional mpv UI setup must continue after managed streaming degradation"
    );
    assert_eq!(
        owner.player_apply_state.acknowledged_bridge_settings,
        Some(ui_settings),
        "optional Lua setup must continue after managed streaming degradation"
    );
    assert!(
        owner
            .player_apply_state
            .acknowledged_bridge_generation
            .is_some()
    );
    let issue = owner
        .player_setup_runtime_snapshot_impl()
        .issue
        .expect("managed streaming degradation should be projected");
    assert_eq!(
        issue.kind,
        crate::app::shell_state::GuiPlayerSetupIssueKind::PlayerSettingsDegraded
    );
}

#[test]
fn managed_mpv_active_streaming_success_promotes_baseline_after_process_attachment() {
    let ui_settings = sorotte_player_mpv::LegacySyncplayUiSettings::default();
    let desired_streaming_options = vec![EffectiveMpvStreamingOption {
        name: "cache-secs".to_owned(),
        configured_value: "30".to_owned(),
        effective_value: "75".to_owned(),
        overridden_by_advanced_arguments: true,
    }];
    let config = crate::app::mpv_launch::ManagedMpvLaunchConfig {
        requested_player_path: "mpv".to_owned(),
        program: PathBuf::from("mpv"),
        effective_streaming_options: desired_streaming_options.clone(),
        extra_args: vec!["https://media.example.test/active.m3u8".to_owned()],
        ui_settings: ui_settings.clone(),
    };
    let launch_state = GuiPlayerLaunchRuntimeState::ManagedMpv(Box::new(config.clone()));
    let mut adapter =
        sorotte_player_mpv::MpvAdapter::with_first_active_network_option_rejection_test_ipc(
            ui_settings,
        );
    crate::app::mpv_launch::configure_effective_streaming_options_for_network_media(
        &mut adapter,
        &desired_streaming_options,
    );
    assert!(
        crate::app::mpv_launch::apply_effective_streaming_options_to_active_network_media_classified(
            &mut adapter,
        )
        .is_err(),
        "the fixture's first active-media write should be rejected"
    );
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_launch_state = launch_state;

    owner.complete_managed_mpv_attachment_after_ipc_connect(
        &config,
        adapter,
        managed_mpv_test_guard(),
    );

    assert!(owner.player.is_some());
    assert!(owner.managed_mpv_process.is_some());
    assert!(!owner.player_apply_state.core_reapply_required);
    assert_eq!(
        owner.player_apply_state.applied_streaming_options,
        Some(desired_streaming_options),
        "only an accepted active-media apply should promote the desired baseline"
    );
    assert!(matches!(
        owner.core_player_configuration_health,
        GuiCorePlayerConfigurationHealth::Ready
    ));
    assert!(owner.player_unavailability_reason.is_none());
}

#[test]
fn managed_mpv_streaming_failure_drops_guard_when_ipc_is_unhealthy() {
    let ui_settings = sorotte_player_mpv::LegacySyncplayUiSettings::default();
    let desired_streaming_options = vec![EffectiveMpvStreamingOption {
        name: "cache-secs".to_owned(),
        configured_value: "30".to_owned(),
        effective_value: "75".to_owned(),
        overridden_by_advanced_arguments: true,
    }];
    let config = crate::app::mpv_launch::ManagedMpvLaunchConfig {
        requested_player_path: "mpv".to_owned(),
        program: PathBuf::from("mpv"),
        effective_streaming_options: desired_streaming_options,
        extra_args: vec!["https://media.example.test/active.m3u8".to_owned()],
        ui_settings: ui_settings.clone(),
    };
    let launch_state = GuiPlayerLaunchRuntimeState::ManagedMpv(Box::new(config.clone()));
    let adapter =
        sorotte_player_mpv::MpvAdapter::with_first_active_network_option_rejection_test_ipc(
            ui_settings,
        );
    let guard_dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let guard = managed_mpv_test_guard_with_drop_observer(guard_dropped.clone());
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_launch_state = launch_state;

    owner.complete_managed_mpv_attachment_with_active_apply_for_test(
        &config,
        adapter,
        guard,
        |adapter| {
            adapter.mark_test_ipc_unhealthy("managed test IPC disconnected during active apply");
            Err("managed test IPC disconnected during active apply".to_owned())
        },
    );

    assert!(owner.player.is_none(), "unhealthy managed IPC is fatal");
    assert!(
        owner.managed_mpv_process.is_none(),
        "a fatal adapter must not transfer its guard into the owner"
    );
    assert!(
        guard_dropped.load(std::sync::atomic::Ordering::Acquire),
        "the unretained guard must synchronously terminate and reap managed mpv"
    );
    assert!(owner.player_apply_state.core_reapply_required);
    assert!(owner.player_apply_state.applied_streaming_options.is_none());
    assert!(owner.player_apply_state.applied_mpv_ui_settings.is_none());
    assert!(
        owner
            .player_unavailability_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("became unavailable"))
    );
}

#[test]
fn optional_bridge_degradation_is_fatal_only_when_core_ipc_becomes_unhealthy() {
    let ui_settings = sorotte_player_mpv::LegacySyncplayUiSettings {
        chat_move_osd: false,
        ..sorotte_player_mpv::LegacySyncplayUiSettings::default()
    };
    let prepare = || {
        let mut adapter =
            sorotte_player_mpv::MpvAdapter::with_rejected_sorotte_bridge_discovery_test_ipc(
                ui_settings.clone(),
            );
        let core_ipc_was_connected = adapter.is_connected();
        let bridge_health = crate::app::mpv_launch::configure_sorotte_chat_osd_integration(
            &mut adapter,
            &ui_settings,
        )
        .bridge_health;
        assert!(matches!(
            bridge_health,
            sorotte_player_mpv::SorotteBridgeHealth::Degraded(_)
        ));
        (adapter, bridge_health, core_ipc_was_connected)
    };

    let (healthy_adapter, healthy_bridge_health, was_connected) = prepare();
    assert!(was_connected && healthy_adapter.is_connected());
    let mut retained_owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    retained_owner
        .retain_mpv_after_optional_bridge_attempt(
            healthy_adapter,
            None,
            healthy_bridge_health,
            was_connected,
        )
        .expect("discovery degradation with healthy core IPC must be retained");
    assert!(retained_owner.player.is_some());

    let (mut unhealthy_adapter, unhealthy_bridge_health, was_connected) = prepare();
    unhealthy_adapter.mark_test_ipc_unhealthy("test core IPC loss after optional bridge attempt");
    let mut unavailable_owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let error = unavailable_owner
        .retain_mpv_after_optional_bridge_attempt(
            unhealthy_adapter,
            None,
            unhealthy_bridge_health,
            was_connected,
        )
        .expect_err("an initially healthy IPC becoming unhealthy must not be retained");
    assert!(error.contains("JSON IPC became unavailable"));
    assert!(unavailable_owner.player.is_none());
    assert!(unavailable_owner.player_integration_health == GuiPlayerIntegrationHealth::Ready);
}

#[test]
fn osd_configuration_failure_transitions_ready_adapter_and_disables_player_chat() {
    let initial_ui_settings = sorotte_player_mpv::LegacySyncplayUiSettings {
        chat_move_osd: false,
        ..sorotte_player_mpv::LegacySyncplayUiSettings::default()
    };
    let mut adapter = sorotte_player_mpv::MpvAdapter::with_unacknowledging_syncplayintf_test_ipc(
        initial_ui_settings.clone(),
    );
    assert!(matches!(
        adapter.configure_bundled_sorotte_bridge(),
        sorotte_player_mpv::SorotteBridgeHealth::Ready
    ));
    let target_ui_settings = sorotte_player_mpv::LegacySyncplayUiSettings {
        chat_move_osd: true,
        ..initial_ui_settings
    };

    let health = crate::app::mpv_launch::configure_sorotte_chat_osd_integration(
        &mut adapter,
        &target_ui_settings,
    )
    .bridge_health;

    assert!(adapter.is_connected());
    assert!(matches!(
        health,
        sorotte_player_mpv::SorotteBridgeHealth::Degraded(
            sorotte_player_mpv::SorotteBridgeFailure {
                kind: sorotte_player_mpv::SorotteBridgeFailureKind::IpcCommand,
                ..
            }
        )
    ));
    assert_eq!(adapter.sorotte_bridge_health(), health);
    assert!(!adapter.legacy_syncplayintf_options_ready());
    assert_eq!(adapter.take_pending_chat_request(), None);
}

#[test]
fn graceful_explicit_gui_detach_restores_osd_before_terminal_bridge_release() {
    let ui_settings = sorotte_player_mpv::LegacySyncplayUiSettings::default();
    let (adapter, commands) =
        sorotte_player_mpv::MpvAdapter::with_cleanup_recording_sorotte_bridge_test_ipc(
            ui_settings.clone(),
            Some(("top".to_owned(), 16)),
        );
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_launch_state = GuiPlayerLaunchRuntimeState::ExplicitMpvIpc {
        ipc_path: r"\\.\pipe\sorotte-cleanup-test".to_owned(),
        ui_settings: Box::new(ui_settings),
        effective_streaming_options: Vec::new(),
    };
    owner.player = Some(GuiOwnedPlayer::Mpv(Box::new(adapter)));

    owner.detach_player();

    assert!(owner.player.is_none());
    let commands = terminal_cleanup_commands(&commands);
    assert_original_osd_restored_before_bridge_release(&commands);
}

#[test]
fn gui_runtime_owner_drop_restores_osd_before_terminal_bridge_release() {
    let (adapter, commands) =
        sorotte_player_mpv::MpvAdapter::with_cleanup_recording_sorotte_bridge_test_ipc(
            sorotte_player_mpv::LegacySyncplayUiSettings::default(),
            Some(("top".to_owned(), 16)),
        );
    {
        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        owner.player = Some(GuiOwnedPlayer::Mpv(Box::new(adapter)));
    }

    let commands = terminal_cleanup_commands(&commands);
    assert_original_osd_restored_before_bridge_release(&commands);
}

#[test]
fn gui_player_pump_projects_runtime_bridge_degradation_and_recovery() {
    let mut adapter = sorotte_player_mpv::SimulatedPlayer::new().into_inner();
    assert!(matches!(
        adapter.mark_sorotte_bridge_degraded(
            sorotte_player_mpv::SorotteBridgeFailureKind::LeaseBusy,
            "another mpv bridge owner retained the input lease",
        ),
        sorotte_player_mpv::SorotteBridgeHealth::Degraded(_)
    ));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Mpv(Box::new(adapter)));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    let degraded_actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(degraded_actions.iter().any(|action| matches!(
        action,
        GuiShellAction::ApplyGuiPlayerSetupRuntimeSnapshot(snapshot)
            if snapshot.issue.as_ref().is_some_and(|issue| {
                issue.kind == crate::app::shell_state::GuiPlayerSetupIssueKind::BridgeDegraded
                    && issue.retry_available
            })
    )));
    assert!(state.player_setup_issue.as_ref().is_some_and(|issue| {
        issue.kind == crate::app::shell_state::GuiPlayerSetupIssueKind::BridgeDegraded
            && issue.retry_available
    }));
    let player = owner
        .player
        .as_mut()
        .expect("runtime-degraded player should stay attached");
    player
        .open_file("runtime-degraded-playback.mkv")
        .expect("runtime bridge degradation must preserve open");
    player
        .set_paused(false)
        .expect("runtime bridge degradation must preserve pause control");
    player
        .set_position(23.0)
        .expect("runtime bridge degradation must preserve seek control");

    owner.player_apply_state.core_reapply_required = true;
    owner.player_unavailability_reason =
        Some("failed to update active mpv network-media options".to_owned());
    let recovered_health = match owner.player.as_mut() {
        Some(GuiOwnedPlayer::Mpv(player)) => player.configure_bundled_sorotte_bridge(),
        _ => panic!("runtime-degraded mpv should stay attached"),
    };
    assert_eq!(
        recovered_health,
        sorotte_player_mpv::SorotteBridgeHealth::Ready
    );

    let recovered_actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(matches!(
        owner.player_integration_health,
        GuiPlayerIntegrationHealth::Ready
    ));
    assert!(state.player_setup_issue.is_none());
    assert!(recovered_actions.iter().any(|action| matches!(
        action,
        GuiShellAction::ApplyGuiPlayerSetupRuntimeSnapshot(snapshot)
            if snapshot.issue.is_none()
    )));
    assert!(
        owner.player_apply_state.core_reapply_required,
        "bridge recovery must not clear an independent core restart requirement"
    );
    assert_eq!(
        owner.player_unavailability_reason.as_deref(),
        Some("failed to update active mpv network-media options"),
        "bridge recovery must not clear the independent streaming failure"
    );
    assert!(owner.player.is_some(), "bridge recovery must retain mpv");
}

#[test]
fn gui_player_pump_does_not_promote_historical_ready_after_current_degradation() {
    let mut adapter = sorotte_player_mpv::SimulatedPlayer::new().into_inner();
    assert_eq!(
        adapter.configure_bundled_sorotte_bridge(),
        sorotte_player_mpv::SorotteBridgeHealth::Ready
    );
    assert!(matches!(
        adapter.mark_sorotte_bridge_degraded(
            sorotte_player_mpv::SorotteBridgeFailureKind::LeaseBusy,
            "the bridge lease was lost before the GUI observed readiness",
        ),
        sorotte_player_mpv::SorotteBridgeHealth::Degraded(_)
    ));

    let acknowledged_settings = sorotte_player_mpv::LegacySyncplayUiSettings {
        notification_timeout_ms: 1_234,
        ..sorotte_player_mpv::LegacySyncplayUiSettings::default()
    };
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_apply_state.acknowledged_bridge_settings = Some(acknowledged_settings.clone());
    owner.player_apply_state.acknowledged_bridge_generation = Some(77);
    owner.player = Some(GuiOwnedPlayer::Mpv(Box::new(adapter)));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(matches!(
        owner.player_integration_health,
        GuiPlayerIntegrationHealth::BridgeDegraded { .. }
    ));
    assert!(actions.iter().any(|action| matches!(
        action,
        GuiShellAction::ApplyGuiPlayerSetupRuntimeSnapshot(snapshot)
            if snapshot.issue.as_ref().is_some_and(|issue| {
                issue.kind == crate::app::shell_state::GuiPlayerSetupIssueKind::BridgeDegraded
            })
    )));
    assert_eq!(
        owner.player_apply_state.acknowledged_bridge_settings,
        Some(acknowledged_settings),
        "a stale Ready event must not replace the last authoritative bridge baseline"
    );
    assert_eq!(
        owner.player_apply_state.acknowledged_bridge_generation,
        Some(77),
        "a stale Ready event must not clear the last authoritative acknowledged generation"
    );
}

#[test]
fn degraded_gui_notifications_still_reach_the_attached_mpv_osd() {
    let mut adapter = sorotte_player_mpv::SimulatedPlayer::new().into_inner();
    let health = adapter.mark_sorotte_bridge_degraded(
        sorotte_player_mpv::SorotteBridgeFailureKind::AcknowledgementTimeout,
        "test bridge acknowledgement timed out",
    );
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Mpv(Box::new(adapter)));
    owner.record_sorotte_bridge_health(health);

    owner.emit_gui_actions_to_attached_player_impl(&[GuiShellAction::PushTransientNotification {
        level: GuiTransientNotificationLevel::Warning,
        message: "bridge warning".to_owned(),
    }]);
    let Some(GuiOwnedPlayer::Mpv(player)) = owner.player.as_ref() else {
        panic!("degraded mpv fixture should remain attached");
    };
    assert_eq!(
        player.last_simulated_legacy_syncplay_osd_message(),
        Some(&(
            "bridge warning".to_owned(),
            sorotte_player_mpv::LegacySyncplayOsdKind::Alert,
        ))
    );
}

#[test]
fn managed_mpv_bridge_ack_failure_retains_process_guard() {
    use std::process::{Command, Stdio};

    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("powershell");
        command.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"]);
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 30"]);
        command
    };
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("test child should spawn");
    let guard = crate::app::mpv_launch::ManagedMpvProcessGuard::from_test_child(child);
    let initial_ui_settings = sorotte_player_mpv::LegacySyncplayUiSettings {
        chat_move_osd: false,
        ..sorotte_player_mpv::LegacySyncplayUiSettings::default()
    };
    let target_ui_settings = sorotte_player_mpv::LegacySyncplayUiSettings {
        notification_timeout_ms: 9_000,
        ..initial_ui_settings.clone()
    };
    let adapter = sorotte_player_mpv::MpvAdapter::with_unacknowledging_syncplayintf_test_ipc(
        initial_ui_settings,
    );
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner
        .complete_mpv_attachment_after_core_configuration(adapter, Some(guard), &target_ui_settings)
        .expect("healthy core IPC should retain its managed guard after missing bridge ack");

    assert!(owner.player.is_some());
    assert!(owner.managed_mpv_process.is_some());
    assert!(
        owner
            .managed_mpv_process
            .as_mut()
            .expect("managed guard should be retained")
            .try_wait()
            .expect("test child status should be readable")
            .is_none(),
        "bridge failure must not terminate the managed mpv process"
    );
    assert!(owner.player_unavailability_reason.is_none());
    assert!(matches!(
        owner.player_integration_health,
        GuiPlayerIntegrationHealth::BridgeDegraded { ref reason, .. }
            if reason.contains("acknowledge")
    ));
}

#[test]
fn bridge_retry_runs_in_place_and_clears_degraded_health() {
    let initial_settings = StoredClientSettingsMvp {
        player_path: Some("mpv".to_owned()),
        notification_timeout_seconds: Some(3),
        ..StoredClientSettingsMvp::default()
    };
    let desired_settings = StoredClientSettingsMvp {
        notification_timeout_seconds: Some(9),
        ..initial_settings.clone()
    };
    let launch_state_for = |settings: &StoredClientSettingsMvp| {
        GuiPlayerLaunchRuntimeState::ManagedMpv(Box::new(
            match crate::app::mpv_launch::managed_mpv_settings_decision_from_settings(Some(
                settings,
            )) {
                crate::app::mpv_launch::ManagedMpvSettingsDecision::Launch(config) => *config,
                other => panic!("expected managed mpv launch state, got {other:?}"),
            },
        ))
    };
    let initial_launch_state = launch_state_for(&initial_settings);
    let desired_launch_state = launch_state_for(&desired_settings);
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player_launch_state = desired_launch_state.clone();
    owner.record_fully_applied_player_launch_state(&initial_launch_state);
    owner
        .retain_mpv_after_optional_bridge_attempt(
            sorotte_player_mpv::SimulatedPlayer::new().into_inner(),
            None,
            acknowledgement_timeout_health(),
            false,
        )
        .expect("simulation fixture should retain its player");
    let original_player_address = match owner.player.as_ref() {
        Some(GuiOwnedPlayer::Mpv(player)) => &**player as *const sorotte_player_mpv::MpvAdapter,
        _ => panic!("fixture mpv should exist"),
    };
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&desired_settings);

    let Some(GuiOwnedPlayer::Mpv(player)) = owner.player.as_ref() else {
        panic!("fixture mpv should exist");
    };
    assert_eq!(
        player.legacy_syncplay_ui_settings().notification_timeout_ms,
        3_000
    );

    handle.push_request(GuiRuntimeRequest::RetryChatOsdIntegration);
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(matches!(
        owner.player_integration_health,
        GuiPlayerIntegrationHealth::Ready
    ));
    assert!(!owner.player_apply_state.core_reapply_required);
    assert_eq!(
        owner.player.as_ref().and_then(|player| match player {
            GuiOwnedPlayer::Mpv(player) => {
                Some(&**player as *const sorotte_player_mpv::MpvAdapter)
            }
            _ => None,
        }),
        Some(original_player_address),
        "bridge retry must not replace the player adapter"
    );
    let Some(GuiOwnedPlayer::Mpv(player)) = owner.player.as_ref() else {
        panic!("retried mpv should remain attached");
    };
    assert_eq!(
        player.legacy_syncplay_ui_settings().notification_timeout_ms,
        9_000,
        "retry must apply the desired bridge settings rather than the old baseline"
    );
    assert_eq!(
        owner
            .player_apply_state
            .acknowledged_bridge_settings
            .as_ref()
            .map(|settings| settings.notification_timeout_ms),
        Some(9_000),
        "successful retry must promote the desired settings to the applied baseline"
    );
    assert_eq!(
        owner
            .player_apply_state
            .applied_mpv_ui_settings
            .as_ref()
            .map(|settings| settings.notification_timeout_ms),
        Some(9_000),
        "successful retry must separately retain the mpv UI-property baseline"
    );
    assert!(
        owner
            .player_apply_state
            .acknowledged_bridge_generation
            .is_some(),
        "successful retry must retain the exact acknowledged Lua generation"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_startup_player_lookup_honors_test_player_env() {
    let owner = GuiPersistedConfigRuntimeOwner::with_config_path_and_startup_player_lookup(
        Some(PathBuf::from("C:/Config/sorotte.ini")),
        &|name| match name {
            "SOROTTE_GUI_ENABLE_TEST_PLAYER" => Some("true".to_owned()),
            _ => None,
        },
    );
    assert_eq!(
        owner.player.as_ref().map(|player| player.name()),
        Some("test")
    );
    assert_eq!(owner.player_unavailability_reason, None);

    let detached_owner = GuiPersistedConfigRuntimeOwner::with_config_path_and_startup_player_lookup(
        Some(PathBuf::from("C:/Config/sorotte.ini")),
        &|_name| None,
    );
    assert!(detached_owner.player.is_none());
    assert!(
        detached_owner
            .player_unavailability_reason
            .as_deref()
            .is_some_and(|message| message.contains("Set playerPath to mpv")),
        "startup owner should surface explicit mpv setup guidance when no player is configured"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_uses_saved_player_path_for_managed_mpv_launch_state() {
    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    let config_path =
        std::env::temp_dir().join(format!("sorotte-gui-startup-player-{unique_suffix}.ini"));
    let mut per_player_arguments = std::collections::BTreeMap::new();
    per_player_arguments.insert(
        "C:/missing/mpv.exe".to_owned(),
        vec![
            "--profile=syncplay".to_owned(),
            "--keep-open=yes".to_owned(),
        ],
    );
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(
        &config_path,
        &StoredClientSettingsMvp {
            player_path: Some("C:/missing/mpv.exe".to_owned()),
            per_player_arguments: Some(per_player_arguments),
            chat_input_enabled: Some(true),
            show_osd: Some(false),
            ..StoredClientSettingsMvp::default()
        },
    )
    .expect("startup-player seed should write sorotte.ini");

    let owner = GuiPersistedConfigRuntimeOwner::with_config_path_and_startup_player_lookup(
        Some(config_path.clone()),
        &|_name| None,
    );
    match &owner.player_launch_state {
        GuiPlayerLaunchRuntimeState::ManagedMpv(config) => {
            assert_eq!(config.requested_player_path, "C:/missing/mpv.exe");
            assert_eq!(
                config.extra_args,
                vec![
                    "--profile=syncplay".to_owned(),
                    "--keep-open=yes".to_owned()
                ]
            );
            assert!(!config.ui_settings.show_osd);
            assert!(config.ui_settings.chat_input_enabled);
        }
        other => panic!("expected managed-mpv launch state, got {other:?}"),
    }
    assert!(owner.player.is_none());
    assert!(
        owner
            .player_unavailability_reason
            .as_deref()
            .is_some_and(|message| {
                message.contains("GUI-owned mpv launch failed from saved player path")
            }),
        "startup attach should fail deterministically for a missing mpv binary"
    );

    let _ = std::fs::remove_file(config_path);
}

#[test]
fn explicit_mpv_ipc_launch_state_honors_selected_players_saved_streaming_overrides() {
    let player_path = "C:/Program Files/mpv/mpv.exe";
    let mut per_player_arguments = std::collections::BTreeMap::new();
    per_player_arguments.insert(player_path.to_owned(), vec!["--cache-secs=75".to_owned()]);
    let settings = StoredClientSettingsMvp {
        player_path: Some(player_path.to_owned()),
        per_player_arguments: Some(per_player_arguments),
        ..StoredClientSettingsMvp::default()
    };

    let launch_state =
        GuiPersistedConfigRuntimeOwner::configured_player_launch_state_from_lookup_and_settings(
            &|name| match name {
                "SOROTTE_CLIENT_MPV_IPC_PATH" => Some("test-explicit-ipc".to_owned()),
                _ => None,
            },
            Some(&settings),
        )
        .expect("explicit mpv IPC launch state should resolve");

    let GuiPlayerLaunchRuntimeState::ExplicitMpvIpc {
        ipc_path,
        effective_streaming_options,
        ..
    } = launch_state
    else {
        panic!("expected explicit mpv IPC launch state");
    };
    assert_eq!(ipc_path, "test-explicit-ipc");
    let cache_secs = effective_streaming_options
        .iter()
        .find(|option| option.name == "cache-secs")
        .expect("network cache duration should be configured");
    assert_eq!(cache_secs.configured_value, "30");
    assert_eq!(cache_secs.effective_value, "75");
    assert!(cache_secs.overridden_by_advanced_arguments);
}

#[test]
fn gui_persisted_config_runtime_owner_auto_attaches_configured_player_for_active_session() {
    let (mut owner, _session_transport) =
        GuiPersistedConfigRuntimeOwner::with_config_path_and_startup_player_lookup(
            Some(PathBuf::from("C:/Config/sorotte.ini")),
            &|name| match name {
                "SOROTTE_GUI_ENABLE_TEST_PLAYER" => Some("true".to_owned()),
                _ => None,
            },
        )
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    owner.player = None;
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);

    assert_eq!(
        owner.player.as_ref().map(|player| player.name()),
        Some("test"),
        "active session pumps should auto-attach the configured player runtime"
    );
}

#[test]
fn gui_persisted_config_runtime_owner_applies_deferred_startup_remote_actions_once() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    let action = GuiShellAction::ApplyStartupPublicServerCache(vec![(
        "Deferred Primary".to_owned(),
        "deferred.example:8999".to_owned(),
    )]);

    owner.apply_deferred_startup_remote_actions_for_test(&handle, &mut state, vec![action.clone()]);
    owner.apply_deferred_startup_remote_actions_for_test(&handle, &mut state, vec![action]);

    let actions = handle.drain_actions();
    assert_eq!(actions.len(), 1);
    assert_eq!(
        state.public_servers.servers[0].address,
        "deferred.example:8999"
    );
}

fn startup_public_server_test_state() -> SorotteGuiShellAppState {
    SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        check_for_updates_automatically: Some(true),
        last_checked_for_updates: Some("2099-01-01 00:00:00.000".to_owned()),
        public_servers: None,
        ..StoredClientSettingsMvp::default()
    })
}

#[test]
fn startup_public_server_hydration_preserves_explicit_empty_cache_without_fetching() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        check_for_updates_automatically: Some(false),
        public_servers: Some(Vec::new()),
        ..StoredClientSettingsMvp::default()
    });

    owner.run_deferred_startup_remote_actions_with_fetcher(&handle, &mut state, |_language| {
        panic!("an explicitly empty public-server list must suppress deferred startup hydration")
    });

    assert!(owner.startup_public_server_hydration.completed);
    assert_eq!(owner.startup_public_server_hydration.attempts_started, 0);
    assert!(owner.startup_remote_actions_rx.is_none());
    assert!(state.public_servers.servers.is_empty());
    assert!(handle.drain_actions().is_empty());
}

type StartupPublicServerResults =
    Arc<Mutex<std::collections::VecDeque<Result<Vec<(String, String)>, String>>>>;

fn pump_startup_public_server_results_until(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SorotteGuiShellAppState,
    results: &StartupPublicServerResults,
    completed: impl Fn(&GuiPersistedConfigRuntimeOwner) -> bool,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let worker_results = results.clone();
        owner.run_deferred_startup_remote_actions_with_fetcher(handle, state, move |_language| {
            worker_results
                .lock()
                .expect("startup public-server results should remain available")
                .pop_front()
                .expect("each started hydration attempt should have a result")
        });
        if completed(owner) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "startup public-server worker should complete before timeout"
        );
        std::thread::yield_now();
    }
}

#[test]
fn startup_public_server_hydration_runs_without_starting_disabled_automatic_updates() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        check_for_updates_automatically: Some(false),
        last_checked_for_updates: None,
        public_servers: None,
        ..StoredClientSettingsMvp::default()
    });
    let results = Arc::new(Mutex::new(std::collections::VecDeque::from([Ok(vec![(
        "Hydrated".to_owned(),
        "hydrated.example:8999".to_owned(),
    )])])));

    pump_startup_public_server_results_until(&mut owner, &handle, &mut state, &results, |owner| {
        owner.startup_public_server_hydration.completed
    });

    let actions = handle.drain_actions();
    assert!(actions.iter().any(|action| matches!(
        action,
        GuiShellAction::ApplyStartupPublicServerCache(servers)
            if servers == &vec![(
                "Hydrated".to_owned(),
                "hydrated.example:8999".to_owned()
            )]
    )));
    assert!(
        actions
            .iter()
            .all(|action| !matches!(action, GuiShellAction::BeginUpdateCheck { .. })),
        "disabled automatic updates must not start while public servers hydrate"
    );
    assert_eq!(state.public_servers.servers.len(), 1);
    assert_eq!(
        state.public_servers.servers[0].address,
        "hydrated.example:8999"
    );
    assert!(
        results
            .lock()
            .expect("startup public-server results should remain available")
            .is_empty()
    );
}

#[test]
fn startup_public_server_hydration_retries_transient_failure_and_suppresses_duplicates() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = startup_public_server_test_state();
    let results = Arc::new(Mutex::new(std::collections::VecDeque::from([
        Err("temporary outage".to_owned()),
        Err("temporary outage".to_owned()),
        Ok(vec![(
            "Recovered".to_owned(),
            "recovered.example:8999".to_owned(),
        )]),
    ])));

    pump_startup_public_server_results_until(&mut owner, &handle, &mut state, &results, |owner| {
        owner.startup_public_server_hydration.attempts_started == 1
            && owner.startup_remote_actions_rx.is_none()
            && owner
                .startup_public_server_hydration
                .next_retry_at
                .is_some()
    });
    let first_actions = handle.drain_actions();
    assert_eq!(
        first_actions
            .iter()
            .filter(|action| matches!(action, GuiShellAction::AnnounceSystemChatEvent(_)))
            .count(),
        1
    );

    owner.startup_public_server_hydration.next_retry_at = Some(std::time::Instant::now());
    pump_startup_public_server_results_until(&mut owner, &handle, &mut state, &results, |owner| {
        owner.startup_public_server_hydration.attempts_started == 2
            && owner.startup_remote_actions_rx.is_none()
            && owner
                .startup_public_server_hydration
                .next_retry_at
                .is_some()
    });
    assert!(
        handle.drain_actions().is_empty(),
        "an identical retry failure must not repeat the warning"
    );

    owner.startup_public_server_hydration.next_retry_at = Some(std::time::Instant::now());
    pump_startup_public_server_results_until(&mut owner, &handle, &mut state, &results, |owner| {
        owner.startup_public_server_hydration.completed
    });
    let recovered_actions = handle.drain_actions();
    assert!(recovered_actions.iter().any(|action| matches!(
        action,
        GuiShellAction::ApplyStartupPublicServerCache(servers)
            if servers == &vec![(
                "Recovered".to_owned(),
                "recovered.example:8999".to_owned()
            )]
    )));
    assert_eq!(state.public_servers.servers.len(), 1);
    assert_eq!(
        state.public_servers.servers[0].address,
        "recovered.example:8999"
    );
    assert!(
        results
            .lock()
            .expect("startup public-server results should remain available")
            .is_empty()
    );
}

#[test]
fn startup_public_server_unsaved_language_change_during_backoff_preserves_retry_context() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = startup_public_server_test_state();
    let results = Arc::new(Mutex::new(std::collections::VecDeque::from([
        Err("temporary outage".to_owned()),
        Ok(vec![(
            "Saved Language".to_owned(),
            "saved-language.example:8999".to_owned(),
        )]),
    ])));

    pump_startup_public_server_results_until(&mut owner, &handle, &mut state, &results, |owner| {
        owner.startup_public_server_hydration.attempts_started == 1
            && owner.startup_remote_actions_rx.is_none()
            && owner
                .startup_public_server_hydration
                .next_retry_at
                .is_some()
    });
    let _ = handle.drain_actions();
    let initial_context = owner.startup_public_server_hydration.context.clone();
    let initial_retry_at = owner.startup_public_server_hydration.next_retry_at;
    let initial_warning = owner.startup_public_server_hydration.last_warning.clone();

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::GeneralLanguage,
        value: "fr".to_owned().into(),
    }));
    owner.run_deferred_startup_remote_actions_with_fetcher(&handle, &mut state, |_language| {
        panic!("an unsaved language edit must not bypass startup hydration backoff")
    });
    assert_eq!(
        owner.startup_public_server_hydration.context,
        initial_context
    );
    assert_eq!(
        owner.startup_public_server_hydration.attempts_started, 1,
        "an unsaved language edit must not reset the bounded retry budget"
    );
    assert_eq!(
        owner.startup_public_server_hydration.next_retry_at,
        initial_retry_at
    );
    assert_eq!(
        owner.startup_public_server_hydration.last_warning,
        initial_warning
    );

    owner.startup_public_server_hydration.next_retry_at = Some(std::time::Instant::now());
    pump_startup_public_server_results_until(&mut owner, &handle, &mut state, &results, |owner| {
        owner.startup_public_server_hydration.completed
    });

    assert_eq!(
        owner.startup_public_server_hydration.attempts_started, 2,
        "the retry must continue in the original saved-language context"
    );
    assert_eq!(state.public_servers.servers.len(), 1);
    assert_eq!(state.public_servers.servers[0].label, "Saved Language");
    assert_eq!(
        state.saved_configuration.language, None,
        "the unsaved language edit must not replace the frozen startup settings"
    );
    assert!(
        results
            .lock()
            .expect("startup public-server results should remain available")
            .is_empty()
    );
}

#[test]
fn startup_public_server_saved_language_change_during_backoff_resets_retry_context() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = startup_public_server_test_state();
    let results = Arc::new(Mutex::new(std::collections::VecDeque::from([
        Err("temporary outage".to_owned()),
        Ok(vec![(
            "French".to_owned(),
            "french.example:8999".to_owned(),
        )]),
    ])));

    pump_startup_public_server_results_until(&mut owner, &handle, &mut state, &results, |owner| {
        owner.startup_public_server_hydration.attempts_started == 1
            && owner.startup_remote_actions_rx.is_none()
            && owner
                .startup_public_server_hydration
                .next_retry_at
                .is_some()
    });
    let _ = handle.drain_actions();

    let mut changed_settings = state.saved_configuration.clone();
    changed_settings.language = Some("fr".to_owned());
    assert!(
        state.apply(GuiShellAction::ApplyGuiSavedConfigurationRuntimeSnapshot(
            crate::app::GuiSavedConfigurationRuntimeSnapshot {
                settings: changed_settings,
            },
        ))
    );
    pump_startup_public_server_results_until(&mut owner, &handle, &mut state, &results, |owner| {
        owner.startup_public_server_hydration.completed
    });

    assert_eq!(
        owner.startup_public_server_hydration.attempts_started, 1,
        "an authoritative saved-language change should receive a fresh retry budget"
    );
    assert_eq!(owner.startup_public_server_hydration.last_warning, None);
    assert_eq!(state.public_servers.servers.len(), 1);
    assert_eq!(state.public_servers.servers[0].label, "French");
    assert_eq!(state.saved_configuration.language.as_deref(), Some("fr"));
    assert!(
        results
            .lock()
            .expect("startup public-server results should remain available")
            .is_empty()
    );
}

#[test]
fn startup_public_server_failure_preserves_cache_added_while_worker_runs() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = startup_public_server_test_state();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();

    owner.run_deferred_startup_remote_actions_with_fetcher(&handle, &mut state, move |_language| {
        entered_tx
            .send(())
            .expect("startup hydration should report entry");
        release_rx
            .recv()
            .expect("startup hydration should be released");
        Err("late failure".to_owned())
    });
    entered_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("startup hydration should enter before timeout");

    let _ = state.apply(GuiShellAction::ApplyStartupPublicServerCache(vec![(
        "Manual Cache".to_owned(),
        "manual.example:8999".to_owned(),
    )]));
    release_tx
        .send(())
        .expect("startup hydration release should send");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !owner.startup_public_server_hydration.completed {
        owner.run_deferred_startup_remote_actions_with_fetcher(&handle, &mut state, |_language| {
            panic!("cached data must prevent a retry")
        });
        assert!(
            std::time::Instant::now() < deadline,
            "late startup hydration failure should complete before timeout"
        );
        std::thread::yield_now();
    }

    assert_eq!(state.public_servers.servers.len(), 1);
    assert_eq!(state.public_servers.servers[0].label, "Manual Cache");
    assert_eq!(
        state.public_servers.servers[0].address,
        "manual.example:8999"
    );
    assert!(state.commands.can_refresh_public_servers);
}

#[test]
fn startup_public_server_manual_empty_refresh_during_worker_prevents_repopulation() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = startup_public_server_test_state();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (finished_tx, finished_rx) = mpsc::channel();

    owner.run_deferred_startup_remote_actions_with_fetcher(&handle, &mut state, move |_language| {
        entered_tx
            .send(())
            .expect("startup hydration should report entry");
        release_rx
            .recv()
            .expect("startup hydration should be released");
        finished_tx
            .send(())
            .expect("startup hydration should report completion");
        Ok(vec![(
            "Remote Cache".to_owned(),
            "remote.example:8999".to_owned(),
        )])
    });
    entered_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("startup hydration should enter before timeout");

    assert!(state.apply(GuiShellAction::BeginPublicServerRefresh));
    assert!(state.apply(GuiShellAction::CompletePublicServerRefresh(Vec::new())));
    assert_eq!(
        state.configuration.settings.public_servers,
        Some(Vec::new())
    );
    release_tx
        .send(())
        .expect("startup hydration release should send");
    finished_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("startup hydration should finish before timeout");

    owner.run_deferred_startup_remote_actions_with_fetcher(&handle, &mut state, |_language| {
        panic!("an explicit empty refresh must prevent replacement hydration")
    });

    assert!(owner.startup_public_server_hydration.completed);
    assert!(owner.startup_remote_actions_rx.is_none());
    assert!(state.public_servers.servers.is_empty());
    assert!(handle.drain_actions().iter().all(|action| !matches!(
        action,
        GuiShellAction::ApplyStartupPublicServerCache(servers) if !servers.is_empty()
    )));
}

#[test]
fn startup_public_server_manual_empty_refresh_during_backoff_prevents_retry() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = startup_public_server_test_state();
    let results = Arc::new(Mutex::new(std::collections::VecDeque::from([Err(
        "temporary outage".to_owned(),
    )])));

    pump_startup_public_server_results_until(&mut owner, &handle, &mut state, &results, |owner| {
        owner.startup_public_server_hydration.attempts_started == 1
            && owner.startup_remote_actions_rx.is_none()
            && owner
                .startup_public_server_hydration
                .next_retry_at
                .is_some()
    });
    let _ = handle.drain_actions();

    assert!(state.apply(GuiShellAction::BeginPublicServerRefresh));
    assert!(state.apply(GuiShellAction::CompletePublicServerRefresh(Vec::new())));
    owner.startup_public_server_hydration.next_retry_at = Some(std::time::Instant::now());
    owner.run_deferred_startup_remote_actions_with_fetcher(&handle, &mut state, |_language| {
        panic!("an explicit empty refresh must prevent a scheduled retry")
    });

    assert!(owner.startup_public_server_hydration.completed);
    assert_eq!(owner.startup_public_server_hydration.attempts_started, 1);
    assert!(
        owner
            .startup_public_server_hydration
            .next_retry_at
            .is_none()
    );
    assert!(state.public_servers.servers.is_empty());
    assert_eq!(
        state.configuration.settings.public_servers,
        Some(Vec::new())
    );
}

#[test]
fn startup_public_server_hydration_keeps_saved_language_worker_when_draft_changes() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = startup_public_server_test_state();
    let (old_entered_tx, old_entered_rx) = mpsc::channel();
    let (old_release_tx, old_release_rx) = mpsc::channel();
    let (old_finished_tx, old_finished_rx) = mpsc::channel();

    owner.run_deferred_startup_remote_actions_with_fetcher(&handle, &mut state, move |language| {
        assert_eq!(language, "en");
        old_entered_tx
            .send(())
            .expect("old-language hydration should report entry");
        old_release_rx
            .recv()
            .expect("old-language hydration should be released");
        old_finished_tx
            .send(())
            .expect("old-language hydration should report completion");
        Ok(vec![(
            "Old Language".to_owned(),
            "old-language.example:8999".to_owned(),
        )])
    });
    old_entered_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("old-language hydration should enter before timeout");

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::GeneralLanguage,
        value: "fr".to_owned().into(),
    }));
    owner.run_deferred_startup_remote_actions_with_fetcher(&handle, &mut state, |_language| {
        panic!("an unsaved language edit must not replace the running worker")
    });
    assert!(owner.startup_remote_actions_rx.is_some());

    old_release_tx
        .send(())
        .expect("old-language hydration release should send");
    old_finished_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("saved-language hydration should finish before timeout");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !owner.startup_public_server_hydration.completed {
        owner.run_deferred_startup_remote_actions_with_fetcher(&handle, &mut state, |_language| {
            panic!("the saved-language hydration should start only once")
        });
        assert!(
            std::time::Instant::now() < deadline,
            "saved-language hydration should complete before timeout"
        );
        std::thread::yield_now();
    }

    assert_eq!(state.public_servers.servers.len(), 1);
    assert_eq!(state.public_servers.servers[0].label, "Old Language");
    assert_eq!(
        state.public_servers.servers[0].address,
        "old-language.example:8999"
    );
    assert!(handle.drain_actions().iter().any(|action| matches!(
        action,
        GuiShellAction::ApplyStartupPublicServerCache(servers)
            if servers.iter().any(|(label, _)| label == "Old Language")
    )));
    assert_eq!(state.saved_configuration.language, None);
}

#[test]
fn gui_persisted_config_runtime_owner_applies_deferred_stream_helper_snapshot_once() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    let snapshot = crate::app::GuiStreamHelperRuntimeSnapshot {
        downloader_status: Some("yt-dlp checked after startup".to_owned()),
        js_runtime_status: Some("Deno checked after startup".to_owned()),
        integration_supported: true,
        ..Default::default()
    };

    owner.apply_deferred_startup_stream_helper_snapshot_for_test(
        &handle,
        &mut state,
        snapshot.clone(),
    );
    owner.apply_deferred_startup_stream_helper_snapshot_for_test(&handle, &mut state, snapshot);

    let actions = handle.drain_actions();
    assert_eq!(actions.len(), 1);
    assert_eq!(
        state.stream_helper.downloader_status.as_deref(),
        Some("yt-dlp checked after startup")
    );
}

#[test]
fn gui_persisted_config_runtime_owner_retry_without_player_path_keeps_setup_guidance() {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path_and_startup_player_lookup(
        Some(PathBuf::from("C:/Config/sorotte.ini")),
        &|_name| None,
    );
    let initial_reason = owner.player_unavailability_reason.clone();

    assert!(owner.player.is_none());
    assert_eq!(owner.player_launch_state, GuiPlayerLaunchRuntimeState::None);
    assert!(
        initial_reason
            .as_deref()
            .is_some_and(|message| message.contains("Set playerPath to mpv")),
        "startup owner should surface explicit mpv setup guidance when no player is configured"
    );

    owner.sync_player_from_lookup_and_settings(
        &|_name| None,
        Some(&StoredClientSettingsMvp::default()),
        true,
    );

    assert!(owner.player.is_none());
    assert_eq!(owner.player_launch_state, GuiPlayerLaunchRuntimeState::None);
    assert_eq!(owner.player_unavailability_reason, initial_reason);
}
