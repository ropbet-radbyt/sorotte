use super::*;

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
        public_servers: Some(Vec::new()),
        ..StoredClientSettingsMvp::default()
    })
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
fn startup_public_server_language_change_during_backoff_resets_retry_context() {
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

    let mut changed_settings = state.configuration.to_stored_settings();
    changed_settings.language = Some("fr".to_owned());
    state.resync_from_settings(changed_settings);
    pump_startup_public_server_results_until(&mut owner, &handle, &mut state, &results, |owner| {
        owner.startup_public_server_hydration.completed
    });

    assert_eq!(
        owner.startup_public_server_hydration.attempts_started, 1,
        "the new language should receive a fresh bounded retry budget"
    );
    assert_eq!(owner.startup_public_server_hydration.last_warning, None);
    assert_eq!(state.public_servers.servers.len(), 1);
    assert_eq!(state.public_servers.servers[0].label, "French");
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
fn startup_public_server_hydration_discards_old_language_worker_result() {
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

    let mut changed_settings = state.configuration.to_stored_settings();
    changed_settings.language = Some("fr".to_owned());
    state.resync_from_settings(changed_settings);
    owner.run_deferred_startup_remote_actions_with_fetcher(&handle, &mut state, |language| {
        assert_eq!(language, "fr");
        Ok(vec![(
            "French".to_owned(),
            "french.example:8999".to_owned(),
        )])
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !owner.startup_public_server_hydration.completed {
        owner.run_deferred_startup_remote_actions_with_fetcher(&handle, &mut state, |_language| {
            panic!("the current-language hydration should start only once")
        });
        assert!(
            std::time::Instant::now() < deadline,
            "current-language hydration should complete before timeout"
        );
        std::thread::yield_now();
    }

    assert_eq!(state.public_servers.servers.len(), 1);
    assert_eq!(state.public_servers.servers[0].label, "French");
    assert_eq!(
        state.public_servers.servers[0].address,
        "french.example:8999"
    );
    assert!(handle.drain_actions().iter().any(|action| matches!(
        action,
        GuiShellAction::ApplyStartupPublicServerCache(servers)
            if servers.iter().any(|(label, _)| label == "French")
    )));

    old_release_tx
        .send(())
        .expect("old-language hydration release should send");
    old_finished_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("old-language hydration should finish before timeout");
    owner.run_deferred_startup_remote_actions_with_fetcher(&handle, &mut state, |_language| {
        panic!("completed hydration must not restart")
    });
    assert!(handle.drain_actions().is_empty());
    assert_eq!(state.public_servers.servers[0].label, "French");
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
