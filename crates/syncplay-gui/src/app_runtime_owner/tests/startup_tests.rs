use super::*;

#[test]
fn gui_persisted_config_runtime_owner_startup_player_lookup_honors_test_player_env() {
    let owner = GuiPersistedConfigRuntimeOwner::with_config_path_and_startup_player_lookup(
        Some(PathBuf::from("C:/Config/syncplay.ini")),
        &|name| match name {
            "SYNCPLAY_GUI_ENABLE_TEST_PLAYER" => Some("true".to_owned()),
            _ => None,
        },
    );
    assert_eq!(
        owner.player.as_ref().map(|player| player.name()),
        Some("test")
    );
    assert_eq!(owner.player_unavailability_reason, None);

    let detached_owner = GuiPersistedConfigRuntimeOwner::with_config_path_and_startup_player_lookup(
        Some(PathBuf::from("C:/Config/syncplay.ini")),
        &|_name| None,
    );
    assert!(detached_owner.player.is_none());
    assert_eq!(detached_owner.player_unavailability_reason, None);
}

#[test]
fn gui_persisted_config_runtime_owner_uses_saved_player_path_for_managed_mpv_launch_state() {
    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    let config_path =
        std::env::temp_dir().join(format!("syncplay-gui-startup-player-{unique_suffix}.ini"));
    let mut per_player_arguments = std::collections::BTreeMap::new();
    per_player_arguments.insert(
        "C:/missing/mpv.exe".to_owned(),
        vec![
            "--profile=syncplay".to_owned(),
            "--keep-open=yes".to_owned(),
        ],
    );
    upsert_syncplay_ini_stored_client_settings_mvp_at_path(
        &config_path,
        &StoredClientSettingsMvp {
            player_path: Some("C:/missing/mpv.exe".to_owned()),
            per_player_arguments: Some(per_player_arguments),
            chat_input_enabled: Some(true),
            show_osd: Some(false),
            ..StoredClientSettingsMvp::default()
        },
    )
    .expect("startup-player seed should write syncplay.ini");

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
fn gui_persisted_config_runtime_owner_auto_attaches_configured_player_for_active_session() {
    let (mut owner, _session_transport) =
        GuiPersistedConfigRuntimeOwner::with_config_path_and_startup_player_lookup(
            Some(PathBuf::from("C:/Config/syncplay.ini")),
            &|name| match name {
                "SYNCPLAY_GUI_ENABLE_TEST_PLAYER" => Some("true".to_owned()),
                _ => None,
            },
        )
        .with_client_core_chat_session_runtime("alice", "room1")
        .expect("client-core chat runtime owner should bootstrap");
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
