use super::*;

#[test]
fn gui_portable_smoke_regression_covers_nontransport_script_parity() {
    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "sorotte-gui-portable-nontransport-{}-{unique_suffix}.ini",
        std::process::id()
    ));
    let preview_media_root = std::env::temp_dir().join(format!(
        "sorotte-gui-portable-open-target-{}-{unique_suffix}",
        std::process::id()
    ));
    let preview_media_path = preview_media_root.join("open-target.mkv");
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir_all(&preview_media_root);
    std::fs::create_dir_all(&preview_media_root)
        .expect("portable preview media fixture directory should be created");
    std::fs::write(&preview_media_path, b"test")
        .expect("portable preview media fixture should be written");

    let mut persisted_owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(path.clone()));
    let persisted_handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut persisted_state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    let saved_settings = StoredClientSettingsMvp {
        host: Some("syncplay.example".to_owned()),
        port: Some(8999),
        username: Some("smoke-user".to_owned()),
        room: Some("smoke-room".to_owned()),
        player_path: Some("C:/Windows/System32/notepad.exe".to_owned()),
        ready_at_start: Some(true),
        autoplay_initial_state: Some(true),
        autoplay_require_same_filenames: Some(true),
        shared_playlist_enabled: Some(true),
        pause_on_leave: Some(true),
        unpause_action: Some(UnpauseActionMode::Always),
        autoplay_min_users: Some(AutoplayThresholdOverride::Set(3)),
        filename_privacy_mode: Some(PrivacyMode::SendHashed),
        filesize_privacy_mode: Some(PrivacyMode::DoNotSend),
        only_switch_to_trusted_domains: Some(true),
        trusted_domains: Some(vec![
            "youtube.com".to_owned(),
            "*.example.com/videos".to_owned(),
        ]),
        rewind_on_desync: Some(true),
        fastforward_on_desync: Some(true),
        slow_on_desync: Some(true),
        dont_slow_down_with_me: Some(true),
        rewind_threshold_seconds: Some(1.25),
        fastforward_threshold_seconds: Some(3.5),
        slowdown_threshold_seconds: Some(2.25),
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        folder_search_first_file_timeout_seconds: Some(3.0),
        folder_search_timeout_seconds: Some(30.0),
        folder_search_double_check_interval_seconds: Some(2.5),
        folder_search_warning_threshold_seconds: Some(7.5),
        chat_input_enabled: Some(true),
        chat_output_enabled: Some(true),
        chat_direct_input: Some(true),
        chat_move_osd: Some(true),
        chat_max_lines: Some(7),
        chat_input_font_family: Some("Consolas".to_owned()),
        chat_output_font_family: Some("Cascadia Mono".to_owned()),
        show_osd: Some(true),
        show_duration_notification: Some(true),
        show_same_room_osd: Some(true),
        show_osd_warnings: Some(true),
        show_noncontroller_osd: Some(true),
        show_different_room_osd: Some(true),
        show_contact_info: Some(true),
        language: Some("pt_BR".to_owned()),
        check_for_updates_automatically: Some(true),
        ..StoredClientSettingsMvp::default()
    };
    assert!(persisted_state.apply(GuiShellAction::BeginConfigurationSave));
    persisted_handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SaveConfiguration(saved_settings.clone()),
    ));
    GuiQueuedRuntimeOwner::pump(&mut persisted_owner, &persisted_handle, &persisted_state);
    let save_actions = persisted_handle.drain_actions();
    assert!(
        save_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::CompleteConfigurationSave(settings) if settings == &saved_settings
        )),
        "portable nontransport smoke save should emit completion with persisted settings"
    );
    for action in save_actions {
        assert!(persisted_state.apply(action));
    }
    assert_eq!(
        load_sorotte_ini_stored_client_settings_mvp_from_path(&path)
            .expect("portable nontransport smoke save should leave a readable config"),
        Some(saved_settings.clone())
    );

    let saved_contents = std::fs::read_to_string(&path)
        .expect("portable nontransport smoke save should leave ini text");
    for expected_line in [
        "host = syncplay.example",
        "port = 8999",
        "name = smoke-user",
        "room = smoke-room",
        "playerPath = C:/Windows/System32/notepad.exe",
        "sharedPlaylistEnabled = True",
    ] {
        assert!(
            saved_contents.contains(expected_line),
            "portable nontransport smoke save should persist line: {expected_line}"
        );
    }

    let reloaded_settings = StoredClientSettingsMvp {
        host: Some("syncplay.reload.example".to_owned()),
        port: Some(8998),
        username: Some("smoke-reloaded".to_owned()),
        room: Some("smoke-room-b".to_owned()),
        player_path: Some("C:/Program Files/mpv/mpv.exe".to_owned()),
        ready_at_start: Some(false),
        autoplay_initial_state: Some(true),
        autoplay_require_same_filenames: Some(false),
        shared_playlist_enabled: Some(true),
        pause_on_leave: Some(false),
        unpause_action: Some(UnpauseActionMode::IfMinUsersReady),
        autoplay_min_users: Some(AutoplayThresholdOverride::Set(4)),
        filename_privacy_mode: Some(PrivacyMode::DoNotSend),
        filesize_privacy_mode: Some(PrivacyMode::SendHashed),
        only_switch_to_trusted_domains: Some(true),
        trusted_domains: Some(vec!["reload.example".to_owned()]),
        rewind_on_desync: Some(true),
        fastforward_on_desync: Some(false),
        slow_on_desync: Some(true),
        dont_slow_down_with_me: Some(false),
        rewind_threshold_seconds: Some(2.5),
        fastforward_threshold_seconds: Some(4.5),
        slowdown_threshold_seconds: Some(1.5),
        media_search_directories: Some(vec![
            "C:/ReloadMedia".to_owned(),
            "D:/ReloadArchive".to_owned(),
        ]),
        folder_search_first_file_timeout_seconds: Some(4.0),
        folder_search_timeout_seconds: Some(40.0),
        folder_search_double_check_interval_seconds: Some(3.0),
        folder_search_warning_threshold_seconds: Some(8.0),
        chat_input_enabled: Some(true),
        chat_output_enabled: Some(true),
        chat_direct_input: Some(false),
        chat_move_osd: Some(true),
        chat_max_lines: Some(9),
        chat_input_font_family: Some("Consolas".to_owned()),
        chat_output_font_family: Some("Segoe UI".to_owned()),
        show_osd: Some(true),
        show_duration_notification: Some(false),
        show_same_room_osd: Some(true),
        show_osd_warnings: Some(true),
        show_noncontroller_osd: Some(false),
        show_different_room_osd: Some(true),
        show_contact_info: Some(true),
        language: Some("es".to_owned()),
        check_for_updates_automatically: Some(true),
        ..StoredClientSettingsMvp::default()
    };
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(&path, &reloaded_settings)
        .expect("portable nontransport smoke reload seed should write config");
    assert!(persisted_state.apply(GuiShellAction::BeginConfigurationReload));
    persisted_handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::ReloadConfiguration(StoredClientSettingsMvp::default()),
    ));
    GuiQueuedRuntimeOwner::pump(&mut persisted_owner, &persisted_handle, &persisted_state);
    let reload_actions = persisted_handle.drain_actions();
    assert!(
        reload_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::CompleteConfigurationReload(settings)
                if settings == &reloaded_settings
        )),
        "portable nontransport smoke reload should emit completion with reloaded settings"
    );
    for action in reload_actions {
        assert!(persisted_state.apply(action));
    }
    assert_eq!(
        persisted_state.saved_configuration, reloaded_settings,
        "portable nontransport smoke reload should project saved settings into shell state"
    );
    assert_eq!(
        persisted_state
            .configuration
            .control_value(SettingId::PlaybackUnpauseAction),
        Some("IfMinUsersReady")
    );
    assert_eq!(
        persisted_state
            .configuration
            .control_value(SettingId::PlaybackAutoplayMinUsers),
        Some("4")
    );
    assert_eq!(
        persisted_state
            .configuration
            .control_value(SettingId::PrivacyTrustedDomainCount),
        Some("1")
    );
    assert_eq!(
        persisted_state
            .configuration
            .control_value(SettingId::GeneralLanguage),
        Some("es")
    );
    assert_eq!(persisted_state.media_search.directories.len(), 2);
    assert_eq!(
        persisted_state.media_search.directories[0].path,
        "C:/ReloadMedia"
    );
    assert!(persisted_state.main_window.shared_playlist_enabled);
    assert!(persisted_state.menus.tls_prompt_expected);
    assert!(!persisted_state.menus.update_notice_expected);
    assert!(
        persisted_state
            .menus
            .sections
            .iter()
            .flat_map(|section| &section.actions)
            .all(|action| !matches!(action.label, "Show Chat" | "Show Playlist" | "Show Users"))
    );

    let mut no_runtime_owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let no_runtime_handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut no_runtime_state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            shared_playlist_enabled: Some(true),
            public_servers: Some(vec![
                ("Alpha".to_owned(), "alpha.example:8999".to_owned()),
                ("Beta".to_owned(), "beta.example:9000".to_owned()),
            ]),
            ..StoredClientSettingsMvp::default()
        });

    assert!(no_runtime_state.apply(GuiShellAction::SelectPublicServer(0)));
    assert!(no_runtime_state.apply(GuiShellAction::BeginSelectedPublicServerConnect));
    no_runtime_handle.push_request(GuiRuntimeRequest::CancelPendingOperation(
        GuiPendingOperationKind::ConnectPublicServer,
    ));
    GuiQueuedRuntimeOwner::pump(&mut no_runtime_owner, &no_runtime_handle, &no_runtime_state);
    for action in no_runtime_handle.drain_actions() {
        assert!(no_runtime_state.apply(action));
    }
    assert!(no_runtime_state.pending_operation.is_none());
    assert_eq!(
        no_runtime_state
            .public_servers
            .servers
            .iter()
            .map(|row| (row.label.as_str(), row.address.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("Alpha", "alpha.example:8999"),
            ("Beta", "beta.example:9000")
        ]
    );
    assert_eq!(
        no_runtime_state
            .notifications
            .last()
            .map(|notification| notification.message.as_str()),
        Some("Public server connect canceled.")
    );

    assert!(no_runtime_state.apply(GuiShellAction::BeginPublicServerRefresh));
    no_runtime_handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::RefreshPublicServers(vec![
            ("Alpha".to_owned(), "alpha.example:8999".to_owned()),
            ("Beta".to_owned(), "beta.example:9000".to_owned()),
        ]),
    ));
    GuiQueuedRuntimeOwner::pump(&mut no_runtime_owner, &no_runtime_handle, &no_runtime_state);
    for action in no_runtime_handle.drain_actions() {
        assert!(no_runtime_state.apply(action));
    }
    assert!(no_runtime_state.pending_operation.is_none());
    assert_eq!(no_runtime_state.public_servers.servers.len(), 2);

    assert!(
        no_runtime_state.apply(GuiShellAction::AnnounceMediaSearchDirectoryBrowsed(
            "C:/SmokeMedia".to_owned(),
        ))
    );
    assert_eq!(no_runtime_state.media_search.directories.len(), 1);
    assert!(
        !no_runtime_state.apply(GuiShellAction::AnnounceMediaSearchDirectoryBrowsed(
            "C:/SmokeMedia".to_owned(),
        ))
    );
    assert_eq!(no_runtime_state.media_search.directories.len(), 1);

    assert!(no_runtime_state.apply(GuiShellAction::BeginMissingMediaSearch));
    no_runtime_handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SearchMissingMedia,
    ));
    GuiQueuedRuntimeOwner::pump(&mut no_runtime_owner, &no_runtime_handle, &no_runtime_state);
    for action in no_runtime_handle.drain_actions() {
        assert!(no_runtime_state.apply(action));
    }
    assert!(no_runtime_state.pending_operation.is_none());
    assert_eq!(
        no_runtime_state
            .notifications
            .last()
            .map(|notification| notification.message.as_str()),
        Some("Missing media search completed: no match found.")
    );

    let preview_open_actions = GuiPreviewRuntimeBridge::preview_open_media_file_actions(
        None,
        vec![preview_media_path.to_string_lossy().into_owned()],
        true,
        None,
    );
    assert_eq!(
        preview_open_actions,
        vec![
            GuiShellAction::SwitchView(GuiShellView::Room),
            GuiShellAction::AnnounceSharedPlaylistLoaded(vec!["open-target.mkv".to_owned()]),
        ],
    );
    for action in preview_open_actions {
        assert!(no_runtime_state.apply(action));
    }
    assert_eq!(no_runtime_state.active_view, GuiShellView::Room);
    assert_eq!(
        no_runtime_state
            .main_window
            .playlist
            .iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        vec!["open-target.mkv"]
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir_all(&preview_media_root);
}
