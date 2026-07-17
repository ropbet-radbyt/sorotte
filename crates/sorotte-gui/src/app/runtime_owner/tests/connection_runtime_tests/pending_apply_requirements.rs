use super::*;
use crate::app::GuiClientCoreChatSessionRuntimeAdapter;

fn save_current_draft(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SorotteGuiShellAppState,
) {
    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    let submitted = state.configuration.to_stored_settings();
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SaveConfiguration(submitted),
    ));
    GuiQueuedRuntimeOwner::pump(owner, handle, state);
    let actions = handle.drain_actions();
    assert!(actions.iter().any(|action| matches!(
        action,
        GuiShellAction::ApplyPendingApplyRequirementsSnapshot(_)
    )));
    for action in actions {
        assert!(state.apply(action));
    }
    assert!(state.pending_operation.is_none());
}

fn persisted_owner_and_state(
    label: &str,
    settings: &StoredClientSettingsMvp,
) -> (
    PathBuf,
    GuiPersistedConfigRuntimeOwner,
    GuiQueuedRuntimeBridgeHandle,
    SorotteGuiShellAppState,
) {
    let root = test_temp_root(label);
    let config_path = root.join("sorotte.ini");
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(&config_path, settings)
        .expect("pending-apply fixture should persist its initial settings");
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path));
    owner.startup_saved_connect_attempted = true;
    (
        root,
        owner,
        GuiQueuedRuntimeBridgeHandle::default(),
        SorotteGuiShellAppState::from_stored_settings(settings),
    )
}

fn install_active_settings_baseline(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    settings: &StoredClientSettingsMvp,
) {
    let snapshot = sorotte_client_app::app_boundary::state::
        stored_client_settings_runtime_snapshot_legacy_compatible(settings);
    owner.session_projects_to_shell = true;
    owner.active_session_settings = Some(snapshot.clone());
    owner.active_session_configured_settings = Some(snapshot);
}

fn install_attached_mpv_baseline(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    settings: &StoredClientSettingsMvp,
) {
    let applied =
        GuiPersistedConfigRuntimeOwner::configured_player_launch_state_from_lookup_and_settings(
            &crate::app::startup_support::env_trimmed,
            Some(settings),
        )
        .expect("mpv lifecycle fixture should resolve to a launch state");
    assert!(matches!(
        applied,
        GuiPlayerLaunchRuntimeState::ManagedMpv(_)
    ));
    owner.player_launch_state = applied.clone();
    owner.applied_player_launch_state = Some(applied);
    owner.player = Some(GuiOwnedPlayer::Mpv(Box::new(
        sorotte_player_mpv::SimulatedPlayer::new().into_inner(),
    )));
}

fn install_attached_unacknowledging_mpv_baseline(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    settings: &StoredClientSettingsMvp,
) {
    let applied =
        GuiPersistedConfigRuntimeOwner::configured_player_launch_state_from_lookup_and_settings(
            &crate::app::startup_support::env_trimmed,
            Some(settings),
        )
        .expect("unacknowledged mpv fixture should resolve to a launch state");
    assert!(matches!(
        applied,
        GuiPlayerLaunchRuntimeState::ManagedMpv(_)
    ));
    let ui_settings =
        crate::app::mpv_launch::legacy_syncplay_ui_settings_from_stored_settings(Some(settings));
    owner.player_launch_state = applied.clone();
    owner.applied_player_launch_state = Some(applied);
    owner.player = Some(GuiOwnedPlayer::Mpv(Box::new(
        sorotte_player_mpv::MpvAdapter::with_unacknowledging_syncplayintf_test_ipc(ui_settings),
    )));
}

#[test]
fn chat_and_osd_requirements_follow_their_runtime_consumers() {
    for id in [
        SettingId::ChatInputFont,
        SettingId::ChatTopMargin,
        SettingId::ChatOutputEnabled,
        SettingId::OsdShow,
        SettingId::OsdNotificationTimeout,
    ] {
        assert_eq!(
            id.apply_requirement(),
            GuiSettingApplyRequirement::OnSave,
            "{id:?} should be applied to the player during Save"
        );
    }
    assert_eq!(
        SettingId::OsdShowContactInfo.apply_requirement(),
        GuiSettingApplyRequirement::OnSave,
        "contact info is a saved GUI preference, not an active-session setting"
    );
    for id in [
        SettingId::ChatInputEnabled,
        SettingId::OsdShowWarnings,
        SettingId::OsdShowSlowdown,
    ] {
        assert_eq!(
            id.apply_requirement(),
            GuiSettingApplyRequirement::Reconnect,
            "{id:?} is also consumed by the active session"
        );
    }
}

#[test]
fn streaming_requirements_follow_player_and_session_consumers() {
    for id in [
        SettingId::StreamingCustomFormat,
        SettingId::StreamingReadAheadSeconds,
        SettingId::StreamingMemoryCacheMib,
        SettingId::StreamingDiskCache,
        SettingId::StreamingEffectiveMpvOptions,
    ] {
        assert_eq!(
            id.apply_requirement(),
            GuiSettingApplyRequirement::OnSave,
            "{id:?} is player-only and should be applied during Save"
        );
    }
    for id in [
        SettingId::StreamingQuality,
        SettingId::StreamingBufferTargetSeconds,
        SettingId::StreamingRecoveryPolicy,
        SettingId::StreamingMaximumCatchupRate,
        SettingId::StreamingHardSeekThresholdSeconds,
        SettingId::StreamingMaximumHardSeeks,
        SettingId::StreamingStabilityIntervalSeconds,
        SettingId::StreamingRecoveryRetryBudget,
        SettingId::StreamingRecoveryCooldownSeconds,
        SettingId::StreamingRoomBufferingPolicy,
        SettingId::StreamingRoomQuorumPercent,
        SettingId::StreamingRoomMaximumPauseSeconds,
        SettingId::StreamingStartSynchronization,
        SettingId::StreamingStartQuorumPercent,
        SettingId::StreamingStartTimeoutSeconds,
        SettingId::StreamingStartTimeoutAction,
        SettingId::StreamingQualityDowngradeSuggestions,
    ] {
        assert_eq!(
            id.apply_requirement(),
            GuiSettingApplyRequirement::Reconnect,
            "{id:?} changes active session coordination"
        );
    }
    for id in [SettingId::PlayerExecutable, SettingId::PlayerArguments] {
        assert_eq!(
            id.apply_requirement(),
            GuiSettingApplyRequirement::RestartPlayer,
            "{id:?} changes the player process target"
        );
    }
}

#[test]
fn detached_ordinary_save_does_not_report_reconnect() {
    let initial = StoredClientSettingsMvp {
        host: Some("active-a.example".to_owned()),
        port: Some(8999),
        ..StoredClientSettingsMvp::default()
    };
    let (root, mut owner, handle, mut state) =
        persisted_owner_and_state("pending-apply-detached-save", &initial);

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionHost,
        value: "saved-b.example".to_owned().into(),
    }));
    save_current_draft(&mut owner, &handle, &mut state);

    assert_eq!(
        state.saved_configuration.host.as_deref(),
        Some("saved-b.example")
    );
    assert!(
        !state
            .pending_apply_requirements
            .contains(&GuiSettingApplyRequirement::Reconnect)
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn connected_host_save_and_revert_toggle_reconnect_symmetrically() {
    let active = StoredClientSettingsMvp {
        host: Some("active-a.example".to_owned()),
        port: Some(8999),
        username: Some("alice".to_owned()),
        room: Some("room-a".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    let (root, mut owner, handle, mut state) =
        persisted_owner_and_state("pending-apply-connected-host", &active);
    owner.session_projects_to_shell = true;
    owner.active_session_settings = Some(
        sorotte_client_app::app_boundary::state::stored_client_settings_runtime_snapshot_legacy_compatible(
            &active,
        ),
    );

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionHost,
        value: "saved-b.example".to_owned().into(),
    }));
    save_current_draft(&mut owner, &handle, &mut state);
    assert!(
        state
            .pending_apply_requirements
            .contains(&GuiSettingApplyRequirement::Reconnect)
    );

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionHost,
        value: "active-a.example".to_owned().into(),
    }));
    save_current_draft(&mut owner, &handle, &mut state);
    assert!(
        !state
            .pending_apply_requirements
            .contains(&GuiSettingApplyRequirement::Reconnect)
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn application_language_and_force_prompt_save_reverts_are_symmetric() {
    let initial = StoredClientSettingsMvp {
        language: Some("en".to_owned()),
        force_gui_prompt: Some(false),
        ..StoredClientSettingsMvp::default()
    };
    let (root, mut owner, handle, mut state) =
        persisted_owner_and_state("pending-apply-application-reverts", &initial);

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::GeneralLanguage,
        value: "fr".to_owned().into(),
    }));
    save_current_draft(&mut owner, &handle, &mut state);
    assert_eq!(
        state.pending_apply_requirements,
        vec![GuiSettingApplyRequirement::RestartApplication]
    );

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::GeneralLanguage,
        value: "en".to_owned().into(),
    }));
    save_current_draft(&mut owner, &handle, &mut state);
    assert!(state.pending_apply_requirements.is_empty());

    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        id: SettingId::GeneralForceGuiPrompt,
        value: true,
    }));
    save_current_draft(&mut owner, &handle, &mut state);
    assert_eq!(
        state.pending_apply_requirements,
        vec![GuiSettingApplyRequirement::RestartApplication]
    );

    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        id: SettingId::GeneralForceGuiPrompt,
        value: false,
    }));
    save_current_draft(&mut owner, &handle, &mut state);
    assert!(state.pending_apply_requirements.is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn active_player_save_and_revert_toggle_restart_player_symmetrically() {
    let initial = StoredClientSettingsMvp {
        player_path: Some("C:/Players/vlc-a.exe".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    let (root, mut owner, handle, mut state) =
        persisted_owner_and_state("pending-apply-player-revert", &initial);
    let applied =
        GuiPersistedConfigRuntimeOwner::configured_player_launch_state_from_lookup_and_settings(
            &crate::app::startup_support::env_trimmed,
            Some(&initial),
        )
        .expect("initial player settings should resolve to a launch state");
    owner.player_launch_state = applied.clone();
    owner.applied_player_launch_state = Some(applied);

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::PlayerExecutable,
        value: "C:/Players/vlc-b.exe".to_owned().into(),
    }));
    save_current_draft(&mut owner, &handle, &mut state);
    assert!(
        state
            .pending_apply_requirements
            .contains(&GuiSettingApplyRequirement::RestartPlayer)
    );

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::PlayerExecutable,
        value: "C:/Players/vlc-a.exe".to_owned().into(),
    }));
    save_current_draft(&mut owner, &handle, &mut state);
    assert!(
        !state
            .pending_apply_requirements
            .contains(&GuiSettingApplyRequirement::RestartPlayer)
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn tainted_partial_player_apply_is_rolled_back_before_restart_guidance_clears() {
    let applied_settings = StoredClientSettingsMvp {
        player_path: Some("C:/Players/mpv.exe".to_owned()),
        chat_top_margin: Some(25),
        ..StoredClientSettingsMvp::default()
    };
    let failed_saved_settings = StoredClientSettingsMvp {
        player_path: Some("C:/Players/mpv.exe".to_owned()),
        chat_top_margin: Some(45),
        ..StoredClientSettingsMvp::default()
    };
    let (root, mut owner, handle, mut state) = persisted_owner_and_state(
        "pending-apply-tainted-player-revert",
        &failed_saved_settings,
    );
    install_attached_mpv_baseline(&mut owner, &applied_settings);
    let failed_ui = crate::app::mpv_launch::legacy_syncplay_ui_settings_from_stored_settings(Some(
        &failed_saved_settings,
    ));
    let Some(GuiOwnedPlayer::Mpv(player)) = owner.player.as_mut() else {
        panic!("tainted player fixture should retain its simulated mpv adapter");
    };
    player
        .configure_legacy_syncplay_ui_settings(failed_ui)
        .expect("the fixture should model the locally mutated side of a partial failure");
    owner.player_settings_reapply_required = true;
    owner.player_unavailability_reason =
        Some("mpv player settings were only partially applied".to_owned());

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ChatTopMargin,
        value: "25".to_owned().into(),
    }));
    save_current_draft(&mut owner, &handle, &mut state);

    assert!(
        !owner.player_settings_reapply_required,
        "the uncertainty marker may clear only after the applied baseline is re-established"
    );
    assert!(state.pending_apply_requirements.is_empty());
    let Some(GuiOwnedPlayer::Mpv(player)) = owner.player.as_ref() else {
        panic!("successful rollback should retain the attached mpv adapter");
    };
    assert_eq!(player.legacy_syncplay_ui_settings().chat_top_margin, 25);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn tainted_player_without_a_live_adapter_keeps_restart_guidance_and_failure_reason() {
    let settings = StoredClientSettingsMvp {
        player_path: Some("C:/Players/mpv.exe".to_owned()),
        check_for_updates_automatically: Some(true),
        ..StoredClientSettingsMvp::default()
    };
    let (root, mut owner, handle, mut state) =
        persisted_owner_and_state("pending-apply-tainted-player-detached", &settings);
    let applied =
        GuiPersistedConfigRuntimeOwner::configured_player_launch_state_from_lookup_and_settings(
            &crate::app::startup_support::env_trimmed,
            Some(&settings),
        )
        .expect("tainted player settings should resolve to a launch state");
    owner.player_launch_state = applied.clone();
    owner.applied_player_launch_state = Some(applied);
    owner.player = None;
    owner.player_settings_reapply_required = true;
    owner.player_unavailability_reason =
        Some("mpv exited after a partial settings apply".to_owned());

    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        id: SettingId::GeneralCheckForUpdatesAutomatically,
        value: false,
    }));
    save_current_draft(&mut owner, &handle, &mut state);

    assert_eq!(
        state.pending_apply_requirements,
        vec![GuiSettingApplyRequirement::RestartPlayer]
    );
    assert_eq!(
        owner.player_unavailability_reason.as_deref(),
        Some("mpv exited after a partial settings apply")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn reverting_a_failed_player_target_reconciles_the_stale_target_and_error() {
    let settings_a = StoredClientSettingsMvp {
        player_path: Some("C:/Players/vlc-a.exe".to_owned()),
        check_for_updates_automatically: Some(true),
        ..StoredClientSettingsMvp::default()
    };
    let settings_b = StoredClientSettingsMvp {
        player_path: Some("C:/Players/vlc-b.exe".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    let (root, mut owner, handle, mut state) =
        persisted_owner_and_state("pending-apply-stale-player-target", &settings_a);
    let launch_a =
        GuiPersistedConfigRuntimeOwner::configured_player_launch_state_from_lookup_and_settings(
            &crate::app::startup_support::env_trimmed,
            Some(&settings_a),
        )
        .expect("player target A should resolve");
    let launch_b =
        GuiPersistedConfigRuntimeOwner::configured_player_launch_state_from_lookup_and_settings(
            &crate::app::startup_support::env_trimmed,
            Some(&settings_b),
        )
        .expect("player target B should resolve");
    owner.applied_player_launch_state = Some(launch_a.clone());
    owner.player_launch_state = launch_b;
    owner.player_unavailability_reason = Some("failed to launch vlc-b.exe".to_owned());
    state.pending_apply_requirements = vec![GuiSettingApplyRequirement::RestartPlayer];

    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        id: SettingId::GeneralCheckForUpdatesAutomatically,
        value: false,
    }));
    save_current_draft(&mut owner, &handle, &mut state);

    assert_eq!(owner.player_launch_state, launch_a);
    assert!(
        !state
            .pending_apply_requirements
            .contains(&GuiSettingApplyRequirement::RestartPlayer)
    );
    assert!(
        owner
            .player_unavailability_reason
            .as_deref()
            .is_some_and(|reason| !reason.contains("vlc-b")),
        "the reconciled error must describe target A, not the failed target B"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn reverting_a_failed_attachable_target_keeps_restart_until_the_restored_target_is_attached() {
    let settings_a = StoredClientSettingsMvp {
        player_path: Some("C:/Players/A/mpv.exe".to_owned()),
        check_for_updates_automatically: Some(true),
        ..StoredClientSettingsMvp::default()
    };
    let settings_b = StoredClientSettingsMvp {
        player_path: Some("C:/Players/B/mpv.exe".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    let (root, mut owner, handle, mut state) =
        persisted_owner_and_state("pending-apply-stale-attachable-target", &settings_a);
    let launch_a =
        GuiPersistedConfigRuntimeOwner::configured_player_launch_state_from_lookup_and_settings(
            &crate::app::startup_support::env_trimmed,
            Some(&settings_a),
        )
        .expect("attachable player target A should resolve");
    let launch_b =
        GuiPersistedConfigRuntimeOwner::configured_player_launch_state_from_lookup_and_settings(
            &crate::app::startup_support::env_trimmed,
            Some(&settings_b),
        )
        .expect("attachable player target B should resolve");
    owner.applied_player_launch_state = Some(launch_a.clone());
    owner.player_launch_state = launch_b;
    owner.player = None;
    owner.player_unavailability_reason = Some("failed to launch C:/Players/B/mpv.exe".to_owned());

    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        id: SettingId::GeneralCheckForUpdatesAutomatically,
        value: false,
    }));
    save_current_draft(&mut owner, &handle, &mut state);

    assert_eq!(owner.player_launch_state, launch_a);
    assert!(owner.player_settings_reapply_required);
    assert_eq!(
        state.pending_apply_requirements,
        vec![GuiSettingApplyRequirement::RestartPlayer]
    );
    assert!(
        owner
            .player_unavailability_reason
            .as_deref()
            .is_some_and(|reason| !reason.contains("Players/B")),
        "guidance should describe the restored target rather than the failed target B"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn osd_timeout_save_applies_to_attached_mpv_without_reconnect_or_restart() {
    let initial = StoredClientSettingsMvp {
        player_path: Some("C:/Players/mpv.exe".to_owned()),
        notification_timeout_seconds: Some(3),
        ..StoredClientSettingsMvp::default()
    };
    let (root, mut owner, handle, mut state) =
        persisted_owner_and_state("pending-apply-osd-timeout", &initial);
    install_active_settings_baseline(&mut owner, &initial);
    install_attached_mpv_baseline(&mut owner, &initial);

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::OsdNotificationTimeout,
        value: "9".to_owned().into(),
    }));
    save_current_draft(&mut owner, &handle, &mut state);

    assert_eq!(
        state.pending_apply_requirements,
        Vec::new(),
        "in-place mpv failure: {:?}",
        owner.player_unavailability_reason
    );
    assert_eq!(
        owner
            .active_session_configured_settings
            .as_ref()
            .and_then(|snapshot| snapshot.settings.notification_timeout_seconds),
        Some(3),
        "pure player UI must not rewrite the active session baseline"
    );
    let Some(GuiOwnedPlayer::Mpv(player)) = owner.player.as_ref() else {
        panic!("OSD timeout fixture should retain its attached mpv adapter");
    };
    assert_eq!(
        player.legacy_syncplay_ui_settings().notification_timeout_ms,
        9_000
    );
    assert_eq!(
        owner
            .applied_player_launch_state
            .as_ref()
            .and_then(GuiPlayerLaunchRuntimeState::mpv_ui_settings)
            .map(|settings| settings.notification_timeout_ms),
        Some(9_000)
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn missing_syncplayintf_ack_keeps_restart_player_guidance_after_save() {
    let initial = StoredClientSettingsMvp {
        player_path: Some("C:/Players/mpv.exe".to_owned()),
        chat_move_osd: Some(false),
        notification_timeout_seconds: Some(3),
        ..StoredClientSettingsMvp::default()
    };
    let (root, mut owner, handle, mut state) =
        persisted_owner_and_state("pending-apply-missing-syncplayintf-ack", &initial);
    install_attached_unacknowledging_mpv_baseline(&mut owner, &initial);

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::OsdNotificationTimeout,
        value: "9".to_owned().into(),
    }));
    save_current_draft(&mut owner, &handle, &mut state);

    assert_eq!(
        state.saved_configuration.notification_timeout_seconds,
        Some(9)
    );
    assert!(owner.player_settings_reapply_required);
    assert_eq!(
        state.pending_apply_requirements,
        vec![GuiSettingApplyRequirement::RestartPlayer]
    );
    assert!(
        owner
            .player_unavailability_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("did not acknowledge")),
        "the player failure should identify the missing Lua acknowledgement: {:?}",
        owner.player_unavailability_reason
    );
    assert_eq!(
        owner
            .applied_player_launch_state
            .as_ref()
            .and_then(GuiPlayerLaunchRuntimeState::mpv_ui_settings)
            .map(|settings| settings.notification_timeout_ms),
        Some(3_000),
        "an unacknowledged update must not advance the applied player baseline"
    );
    assert!(!owner.current_player_launch_state_is_applied());
    let Some(GuiOwnedPlayer::Mpv(player)) = owner.player.as_ref() else {
        panic!("missing-ack fixture should retain its connected mpv adapter");
    };
    assert!(!player.legacy_syncplayintf_options_ready());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn chat_margin_save_applies_to_attached_mpv_without_reconnect_or_restart() {
    let initial = StoredClientSettingsMvp {
        player_path: Some("C:/Players/mpv.exe".to_owned()),
        chat_top_margin: Some(25),
        ..StoredClientSettingsMvp::default()
    };
    let (root, mut owner, handle, mut state) =
        persisted_owner_and_state("pending-apply-chat-margin", &initial);
    install_active_settings_baseline(&mut owner, &initial);
    install_attached_mpv_baseline(&mut owner, &initial);

    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ChatTopMargin,
        value: "45".to_owned().into(),
    }));
    save_current_draft(&mut owner, &handle, &mut state);

    assert_eq!(
        state.pending_apply_requirements,
        Vec::new(),
        "in-place mpv failure: {:?}",
        owner.player_unavailability_reason
    );
    let Some(GuiOwnedPlayer::Mpv(player)) = owner.player.as_ref() else {
        panic!("chat margin fixture should retain its attached mpv adapter");
    };
    assert_eq!(player.legacy_syncplay_ui_settings().chat_top_margin, 45);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn chat_input_save_applies_to_mpv_and_reports_only_reconnect_until_reconnected() {
    let initial = StoredClientSettingsMvp {
        player_path: Some("C:/Players/mpv.exe".to_owned()),
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    };
    let (root, mut owner, handle, mut state) =
        persisted_owner_and_state("pending-apply-chat-input", &initial);
    install_active_settings_baseline(&mut owner, &initial);
    install_attached_mpv_baseline(&mut owner, &initial);

    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        id: SettingId::ChatInputEnabled,
        value: false,
    }));
    save_current_draft(&mut owner, &handle, &mut state);

    assert_eq!(
        state.pending_apply_requirements,
        vec![GuiSettingApplyRequirement::Reconnect],
        "in-place mpv failure: {:?}",
        owner.player_unavailability_reason
    );
    let Some(GuiOwnedPlayer::Mpv(player)) = owner.player.as_ref() else {
        panic!("chat input fixture should retain its attached mpv adapter");
    };
    assert!(!player.legacy_syncplay_ui_settings().chat_input_enabled);

    install_active_settings_baseline(&mut owner, &state.saved_configuration);
    assert!(
        state.apply(owner.pending_apply_requirements_action(&state, &state.saved_configuration,))
    );
    assert!(state.pending_apply_requirements.is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn successful_player_retry_reconciles_restart_player_requirement() {
    let initial = StoredClientSettingsMvp::default();
    let (root, mut owner, handle, mut state) =
        persisted_owner_and_state("pending-apply-player-retry", &initial);
    owner.player_launch_state = GuiPlayerLaunchRuntimeState::TestPlayer;
    owner.applied_player_launch_state = Some(GuiPlayerLaunchRuntimeState::TestPlayer);
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    state.pending_apply_requirements = vec![GuiSettingApplyRequirement::RestartPlayer];

    handle.push_request(GuiRuntimeRequest::RetryPlayerLaunch);
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    for action in handle.drain_actions() {
        assert!(state.apply(action));
    }

    assert!(owner.current_player_launch_state_is_applied());
    assert!(
        !state
            .pending_apply_requirements
            .contains(&GuiSettingApplyRequirement::RestartPlayer)
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn live_autoplay_overrides_do_not_create_reconnect_guidance_on_unrelated_save() {
    let initial = StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room-a".to_owned()),
        autoplay_initial_state: Some(false),
        autoplay_min_users: Some(
            sorotte_client_app::app_boundary::state::AutoplayThresholdOverride::Set(2),
        ),
        check_for_updates_automatically: Some(true),
        ..StoredClientSettingsMvp::default()
    };
    let (root, mut owner, handle, mut state) =
        persisted_owner_and_state("pending-apply-live-autoplay", &initial);
    let session = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room-a")
        .expect("autoplay requirement fixture should create a session");
    owner.install_active_session_runtime(
        Box::new(session),
        sorotte_client_app::app_boundary::state::stored_client_settings_runtime_snapshot_legacy_compatible(
            &initial,
        ),
    );

    handle.push_request(GuiRuntimeRequest::SetAutoplayEnabled(true));
    handle.push_request(GuiRuntimeRequest::SetAutoplayThreshold(5));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    for action in handle.drain_actions() {
        let _ = state.apply(action);
    }
    let live = owner
        .active_session_settings
        .as_ref()
        .expect("live session settings should remain available");
    let configured = owner
        .active_session_configured_settings
        .as_ref()
        .expect("configured session baseline should remain available");
    assert_eq!(live.settings.autoplay_initial_state, Some(true));
    assert_eq!(
        live.settings.autoplay_min_users,
        Some(sorotte_client_app::app_boundary::state::AutoplayThresholdOverride::Set(5))
    );
    assert_eq!(configured.settings.autoplay_initial_state, Some(false));
    assert_eq!(
        configured.settings.autoplay_min_users,
        Some(sorotte_client_app::app_boundary::state::AutoplayThresholdOverride::Set(2))
    );

    assert!(state.apply(GuiShellAction::EditConfigurationBool {
        id: SettingId::GeneralCheckForUpdatesAutomatically,
        value: false,
    }));
    save_current_draft(&mut owner, &handle, &mut state);
    assert!(
        !state
            .pending_apply_requirements
            .contains(&GuiSettingApplyRequirement::Reconnect),
        "temporary live autoplay controls must not be compared as configured reconnect drift"
    );
    assert_eq!(
        owner
            .active_session_settings
            .as_ref()
            .and_then(|settings| settings.settings.autoplay_initial_state),
        Some(true),
        "ordinary saves must retain the live autoplay override"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn reload_to_intentionally_unconfigured_player_clears_restart_requirement() {
    let initial = StoredClientSettingsMvp {
        player_path: Some("C:/Players/vlc.exe".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    let (root, mut owner, handle, mut state) =
        persisted_owner_and_state("pending-apply-reload-no-player", &initial);
    owner.player = None;
    owner.player_launch_state = GuiPlayerLaunchRuntimeState::None;
    owner.applied_player_launch_state = Some(GuiPlayerLaunchRuntimeState::None);
    state.pending_apply_requirements = vec![GuiSettingApplyRequirement::RestartPlayer];

    let reloaded = StoredClientSettingsMvp {
        language: Some("en".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    std::fs::remove_file(root.join("sorotte.ini"))
        .expect("no-player reload fixture should replace the original config");
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(&root.join("sorotte.ini"), &reloaded)
        .expect("no-player reload fixture should update the config");
    assert!(state.apply(GuiShellAction::BeginConfigurationReload));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::ReloadConfiguration(initial),
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    for action in handle.drain_actions() {
        let _ = state.apply(action);
    }

    assert!(
        owner.current_player_launch_state_is_applied(),
        "reload launch state should be applied: launch={:?}, applied={:?}",
        owner.player_launch_state,
        owner.applied_player_launch_state,
    );
    assert_eq!(state.saved_configuration.player_path, None);
    assert!(
        !state
            .pending_apply_requirements
            .contains(&GuiSettingApplyRequirement::RestartPlayer)
    );
    let _ = std::fs::remove_dir_all(root);
}
