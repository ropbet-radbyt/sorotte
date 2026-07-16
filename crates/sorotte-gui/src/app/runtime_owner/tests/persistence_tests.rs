use super::*;
use crate::app::GuiPersistedSettingsPatch;
use crate::app::runtime_owner::{GuiActivePlexPlaylistResolveJob, GuiActivePlexPlaylistSearchJob};
use sorotte_client_app::app_boundary::state::stored_client_settings_runtime_snapshot_legacy_compatible;
use sorotte_plex::{PlexServerConnectionKind, discovery::PlexServerConnection};

fn stage_unrelated_plex_draft(state: &mut SorotteGuiShellAppState) {
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::GeneralLanguage,
        value: "fr".to_owned().into(),
    }));
    assert!(state.apply(GuiShellAction::RemoveServerPassword));
    assert!(state.apply(GuiShellAction::FocusConfigurationControl(
        SettingId::GeneralLanguage,
    )));
    assert!(state.apply(GuiShellAction::BeginConfigurationTextEdit(
        SettingId::GeneralLanguage,
    )));
    assert!(state.apply(GuiShellAction::UpdateConfigurationTextEdit(
        "de".to_owned().into(),
    )));
}

#[test]
fn gui_persisted_config_runtime_owner_persists_save_and_reload_requests() {
    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "sorotte-gui-persisted-config-owner-{}-{unique_suffix}.ini",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(path.clone()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());

    let saved_settings = StoredClientSettingsMvp {
        host: Some("persisted.example".to_owned()),
        room: Some("Cinema".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    state.resync_from_settings(saved_settings.clone());
    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SaveConfiguration(saved_settings.clone()),
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let save_actions = handle.drain_actions();
    assert!(
        save_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::CompleteConfigurationSave(settings) if settings == &saved_settings
        )),
        "save should emit a completion action with the persisted settings"
    );
    for action in save_actions {
        assert!(state.apply(action));
    }
    assert_eq!(
        load_sorotte_ini_stored_client_settings_mvp_from_path(&path)
            .expect("save should leave a readable config file"),
        Some(saved_settings.clone())
    );

    let reloaded_settings = StoredClientSettingsMvp {
        host: Some("reloaded.example".to_owned()),
        room: Some("Rewatch".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(&path, &reloaded_settings)
        .expect("updating the config file should succeed");
    assert!(state.apply(GuiShellAction::BeginConfigurationReload));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::ReloadConfiguration(StoredClientSettingsMvp::default()),
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    let reload_actions = handle.drain_actions();
    assert!(
        reload_actions.iter().any(|action| matches!(
            action,
            GuiShellAction::CompleteConfigurationReload(settings) if settings == &reloaded_settings
        )),
        "reload should emit a completion action with the reloaded settings"
    );
    for action in reload_actions {
        assert!(state.apply(action));
    }

    std::fs::remove_file(&path).expect("temporary config file should be removable");
}

#[test]
fn gui_persisted_config_runtime_owner_plex_cache_uses_sorotte_cache_directory() {
    let root = test_temp_root("plex-cache-path-owner");
    let config_path = root.join("sorotte.ini");
    let owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path));

    assert_eq!(
        owner.plex_cache_path(),
        Some(root.join("cache").join("plex-watch-cache.json"))
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_disables_plex_without_clearing_credentials_or_subsettings() {
    let root = test_temp_root("plugin-disable-plex-preserves-settings");
    let config_path = root.join("sorotte.ini");
    let saved_settings = StoredClientSettingsMvp {
        host: Some("saved.example".to_owned()),
        server_password: Some("saved-secret".into()),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("machine-id".to_owned()),
        plex_selected_server_url: Some("https://plex.example.invalid:32400".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        plex_sync_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    };
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(&config_path, &saved_settings)
        .expect("initial Plex settings should be persisted");
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path.clone()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&saved_settings);
    owner.active_session_settings = Some(
        stored_client_settings_runtime_snapshot_legacy_compatible(&StoredClientSettingsMvp {
            host: Some("active-session.example".to_owned()),
            room: Some("+Room:CB39A19549E8:ab-123-456".to_owned()),
            server_password: Some("active-session-secret".into()),
            ..saved_settings.clone()
        }),
    );
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionHost,
        value: "unsaved-draft.example".to_owned().into(),
    }));
    assert!(state.apply(GuiShellAction::RemoveServerPassword));
    state.main_window.playback.can_manage_playlist = true;
    assert!(state.apply(GuiShellAction::BeginPlexPlaylistSearch));
    assert!(state.apply(GuiShellAction::SubmitPlexPlaylistSearch {
        query: "stale-query".to_owned(),
    }));
    state
        .plex_playlist_search
        .as_mut()
        .expect("Plex picker should be open")
        .adding_rating_key = Some("stale-rating-key".to_owned());
    let (_auth_start_tx, auth_start_rx) = mpsc::channel();
    let (_sync_tx, sync_rx) = mpsc::channel();
    let (_search_tx, search_rx) = mpsc::channel();
    let (_resolve_tx, resolve_rx) = mpsc::channel();
    let (_stream_tx, stream_rx) = mpsc::channel();
    let operation_context = owner.plex_operation_context(&saved_settings);
    owner.plex_auth_start_rx = Some(auth_start_rx);
    owner.plex_auth_poll_due_at = Some(std::time::Instant::now());
    owner.plex_sync_rx = Some(sync_rx);
    owner.plex_sync_next_tick_due_at = Some(std::time::Instant::now());
    owner.plex_playlist_job_generation = 41;
    owner.plex_playlist_search_job = Some(GuiActivePlexPlaylistSearchJob {
        id: 40,
        operation_context: operation_context.clone(),
        query: "stale-query".to_owned(),
        result_rx: search_rx,
    });
    owner.plex_playlist_resolve_job = Some(GuiActivePlexPlaylistResolveJob {
        id: 41,
        operation_context,
        rating_key: "stale-rating-key".to_owned(),
        result_rx: resolve_rx,
    });
    owner.plex_stream_resolve_rx = Some(stream_rx);
    owner.plex_stream_resolve_trigger_key = Some("stale-stream".to_owned());

    handle.push_request(GuiRuntimeRequest::SetPluginEnabled {
        plugin: GuiPluginSelection::Plex,
        enabled: false,
    });
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(owner.plex_auth_start_rx.is_none());
    assert!(owner.plex_auth_poll_due_at.is_none());
    assert!(owner.plex_sync_rx.is_none());
    assert!(owner.plex_sync_next_tick_due_at.is_none());
    assert_eq!(owner.plex_playlist_job_generation, 42);
    assert!(owner.plex_playlist_search_job.is_none());
    assert!(owner.plex_playlist_resolve_job.is_none());
    assert!(owner.plex_stream_resolve_rx.is_none());
    assert!(owner.plex_stream_resolve_trigger_key.is_none());
    assert!(state.plex_playlist_search.is_none());
    assert!(
        !state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::Plex)
    );
    assert!(state.plex.enabled);
    assert!(state.plex.streaming_enabled);
    assert_eq!(
        state.configuration.to_stored_settings().host.as_deref(),
        Some("unsaved-draft.example")
    );
    assert!(matches!(
        state.configuration.server_password,
        SecretDraft::Clear
    ));
    let active_settings = &owner
        .active_session_settings
        .as_ref()
        .expect("immediate patch should preserve the active session snapshot")
        .settings;
    assert_eq!(active_settings.plex_plugin_enabled, Some(false));
    assert_eq!(
        active_settings.host.as_deref(),
        Some("active-session.example")
    );
    assert_eq!(
        active_settings
            .server_password
            .as_ref()
            .map(|password| password.expose_secret()),
        Some("active-session-secret")
    );
    assert_eq!(
        owner
            .active_session_settings
            .as_ref()
            .and_then(|settings| settings.controlled_room_password_override.as_ref())
            .map(|password| password.expose_secret()),
        Some("AB-123-456")
    );
    owner.promote_on_save_runtime_fields(&StoredClientSettingsMvp {
        media_search_directories: Some(vec!["D:/Saved Media".to_owned()]),
        ..saved_settings.clone()
    });
    assert_eq!(
        owner
            .active_session_settings
            .as_ref()
            .and_then(|settings| settings.controlled_room_password_override.as_ref())
            .map(|password| password.expose_secret()),
        Some("AB-123-456"),
        "ordinary OnSave promotion must preserve active controlled-room credentials"
    );

    let settings = load_sorotte_ini_stored_client_settings_mvp_from_path(&config_path)
        .expect("Plex plugin setting config should be readable")
        .expect("Plex plugin setting config should exist");
    assert_eq!(settings.plex_plugin_enabled, Some(false));
    assert_eq!(state.saved_configuration.plex_plugin_enabled, Some(false));
    assert_eq!(
        state.configuration.settings.plex_plugin_enabled,
        Some(false)
    );
    assert_eq!(
        settings
            .plex_user_token
            .as_ref()
            .map(|token| token.expose_secret()),
        Some("user-token")
    );
    assert_eq!(
        settings.plex_selected_server_id.as_deref(),
        Some("machine-id")
    );
    assert_eq!(
        settings.plex_selected_server_url.as_deref(),
        Some("https://plex.example.invalid:32400")
    );
    assert_eq!(
        settings
            .plex_selected_server_token
            .as_ref()
            .map(|token| token.expose_secret()),
        Some("server-token")
    );
    assert_eq!(settings.plex_sync_enabled, Some(true));
    assert_eq!(settings.plex_streaming_enabled, Some(true));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_preserves_plex_jobs_when_plugin_disable_persistence_fails() {
    let root = test_temp_root("plugin-disable-plex-persist-failure");
    std::fs::create_dir_all(&root).expect("test directory should be created");
    let sentinel_path = root.join("unchanged.txt");
    std::fs::write(&sentinel_path, "unchanged").expect("sentinel should be written");
    let settings = StoredClientSettingsMvp {
        host: Some("saved.example".to_owned()),
        server_password: Some("saved-secret".into()),
        plex_plugin_enabled: Some(true),
        shared_playlist_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("machine-id".to_owned()),
        plex_selected_server_url: Some("https://plex.example.invalid:32400".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    };
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(root.clone()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&settings);
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionHost,
        value: "draft.example".to_owned().into(),
    }));
    assert!(state.apply(GuiShellAction::RemoveServerPassword));
    assert!(state.apply(GuiShellAction::FocusConfigurationControl(
        SettingId::ConnectionHost,
    )));
    assert!(state.apply(GuiShellAction::BeginConfigurationTextEdit(
        SettingId::ConnectionHost,
    )));
    assert!(state.apply(GuiShellAction::UpdateConfigurationTextEdit(
        "typing.example".to_owned().into(),
    )));
    let saved_before = state.saved_configuration.clone();
    let draft_before = state.configuration.settings.clone();
    let secret_before = state.configuration.server_password.clone();
    let focused_before = state.focused_configuration_control.clone();
    let edit_before = state.text_edit_session.clone();
    state.main_window.playback.can_manage_playlist = true;
    assert!(state.apply(GuiShellAction::BeginPlexPlaylistSearch));
    assert!(state.apply(GuiShellAction::SubmitPlexPlaylistSearch {
        query: "stale-query".to_owned(),
    }));
    state
        .plex_playlist_search
        .as_mut()
        .expect("Plex picker should be open")
        .adding_rating_key = Some("stale-rating-key".to_owned());
    let operation_context = owner.plex_operation_context(&settings);
    let (_search_tx, search_rx) = mpsc::channel();
    let (_resolve_tx, resolve_rx) = mpsc::channel();
    owner.plex_playlist_job_generation = 9;
    owner.plex_playlist_search_job = Some(GuiActivePlexPlaylistSearchJob {
        id: 8,
        operation_context: operation_context.clone(),
        query: "stale-query".to_owned(),
        result_rx: search_rx,
    });
    owner.plex_playlist_resolve_job = Some(GuiActivePlexPlaylistResolveJob {
        id: 9,
        operation_context,
        rating_key: "stale-rating-key".to_owned(),
        result_rx: resolve_rx,
    });

    handle.push_request(GuiRuntimeRequest::SetPluginEnabled {
        plugin: GuiPluginSelection::Plex,
        enabled: false,
    });
    let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert_eq!(owner.plex_playlist_job_generation, 9);
    assert!(owner.plex_playlist_search_job.is_some());
    assert!(owner.plex_playlist_resolve_job.is_some());
    assert!(state.plex_playlist_search.is_some());
    assert!(
        state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::Plex)
    );
    assert_eq!(state.saved_configuration, saved_before);
    assert_eq!(state.configuration.settings, draft_before);
    assert_eq!(state.configuration.server_password, secret_before);
    assert_eq!(state.focused_configuration_control, focused_before);
    assert_eq!(state.text_edit_session, edit_before);
    assert!(state.has_unsaved_configuration_changes());
    assert!(root.is_dir());
    assert_eq!(
        std::fs::read_to_string(&sentinel_path).expect("sentinel should remain readable"),
        "unchanged"
    );
    assert!(actions.iter().any(|action| matches!(
        action,
        GuiShellAction::PushTransientNotification {
            level: GuiTransientNotificationLevel::Error,
            message,
        } if message.contains("Could not persist Plex plugin setting")
    )));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_configuration_save_invalidates_plex_jobs_against_saved_settings() {
    let root = test_temp_root("configuration-save-plex-context");
    let config_path = root.join("sorotte.ini");
    let saved_settings = StoredClientSettingsMvp {
        plex_plugin_enabled: Some(true),
        shared_playlist_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("old-machine".to_owned()),
        plex_selected_server_url: Some("https://old.example:32400".to_owned()),
        plex_selected_server_token: Some("old-server-token".into()),
        ..StoredClientSettingsMvp::default()
    };
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(&config_path, &saved_settings)
        .expect("initial settings should persist");
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&saved_settings);
    state.main_window.playback.can_manage_playlist = true;
    assert!(state.apply(GuiShellAction::BeginPlexPlaylistSearch));
    assert!(state.apply(GuiShellAction::SubmitPlexPlaylistSearch {
        query: "stale-query".to_owned(),
    }));
    state
        .plex_playlist_search
        .as_mut()
        .expect("Plex picker should be open")
        .adding_rating_key = Some("stale-rating-key".to_owned());
    let operation_context = owner.plex_operation_context(&saved_settings);
    let (_search_tx, search_rx) = mpsc::channel();
    let (_resolve_tx, resolve_rx) = mpsc::channel();
    owner.plex_playlist_search_job = Some(GuiActivePlexPlaylistSearchJob {
        id: 1,
        operation_context: operation_context.clone(),
        query: "stale-query".to_owned(),
        result_rx: search_rx,
    });
    owner.plex_playlist_resolve_job = Some(GuiActivePlexPlaylistResolveJob {
        id: 2,
        operation_context,
        rating_key: "stale-rating-key".to_owned(),
        result_rx: resolve_rx,
    });
    state.configuration.settings.plex_selected_server_id = Some("new-machine".to_owned());
    state.configuration.settings.plex_selected_server_url =
        Some("https://new.example:32400".to_owned());
    state.configuration.settings.plex_selected_server_token = Some("new-server-token".into());
    let submitted_settings = state.configuration.to_stored_settings();
    assert_ne!(state.saved_configuration, submitted_settings);

    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SaveConfiguration(submitted_settings),
    ));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(owner.plex_playlist_search_job.is_none());
    assert!(owner.plex_playlist_resolve_job.is_none());
    assert!(state.plex_playlist_search.is_none());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_disables_media_matching_and_cancels_background_work() {
    let root = test_temp_root("plugin-disable-media-match-cancels");
    let config_path = root.join("sorotte.ini");
    let saved_settings = StoredClientSettingsMvp {
        media_match_fingerprinting_enabled: Some(true),
        media_match_background_warmup_enabled: Some(true),
        media_match_wire_sharing_enabled: Some(true),
        media_match_runtime_tolerance_enabled: Some(true),
        media_match_autoplay_policy: Some("AllowExact".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(&config_path, &saved_settings)
        .expect("initial Media Matching settings should be persisted");
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path.clone()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&saved_settings);
    let (_worker_tx, worker_rx) = mpsc::channel();
    let cancel_flag = Arc::new(AtomicBool::new(false));
    owner.media_match_background_worker_rx = Some(worker_rx);
    owner.media_match_background_worker_cancel = Some(Arc::clone(&cancel_flag));
    owner.media_match_background_trigger_key = Some("background warmup".to_owned());
    owner.media_match_wire_sync_token = Some("wire-token".to_owned());

    handle.push_request(GuiRuntimeRequest::SetPluginEnabled {
        plugin: GuiPluginSelection::MediaMatching,
        enabled: false,
    });
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(
        cancel_flag.load(Ordering::Relaxed),
        "disabling Media Matching should cancel the active background worker"
    );
    assert_eq!(
        owner.media_match_background_cancel_disposition,
        Some(GuiMediaMatchBackgroundCancelDisposition::KeepCheckpoint)
    );
    assert_eq!(
        owner.media_match_runtime_snapshot.remote_status.as_deref(),
        Some("disabled: plugin off")
    );
    assert!(
        !state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::MediaMatching)
    );
    assert!(state.media_match.settings.fingerprinting_enabled);
    assert!(state.media_match.settings.background_warmup_enabled);

    let settings = load_sorotte_ini_stored_client_settings_mvp_from_path(&config_path)
        .expect("Media Matching plugin setting config should be readable")
        .expect("Media Matching plugin setting config should exist");
    assert_eq!(settings.media_matching_plugin_enabled, Some(false));
    assert_eq!(settings.media_match_fingerprinting_enabled, Some(true));
    assert_eq!(settings.media_match_background_warmup_enabled, Some(true));
    assert_eq!(settings.media_match_wire_sharing_enabled, Some(true));
    assert_eq!(settings.media_match_runtime_tolerance_enabled, Some(true));
    assert_eq!(
        settings.media_match_autoplay_policy.as_deref(),
        Some("AllowExact")
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_reenables_media_matching_and_refreshes_snapshot() {
    let root = test_temp_root("plugin-reenable-media-match-refreshes");
    let config_path = root.join("sorotte.ini");
    let saved_settings = StoredClientSettingsMvp {
        media_matching_plugin_enabled: Some(false),
        media_match_fingerprinting_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    };
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(&config_path, &saved_settings)
        .expect("initial Media Matching plugin setting should be persisted");
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path.clone()));
    owner.media_match_runtime_snapshot.remote_status = Some("disabled: plugin off".to_owned());
    owner.media_match_runtime_snapshot.install_location = Some("stale default root".to_owned());
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&saved_settings);

    handle.push_request(GuiRuntimeRequest::SetPluginEnabled {
        plugin: GuiPluginSelection::MediaMatching,
        enabled: true,
    });
    let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(
        actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::ApplyGuiMediaMatchRuntimeSnapshot(_))),
        "reenabling Media Matching should publish a fresh runtime snapshot"
    );
    assert!(
        state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::MediaMatching)
    );
    assert_eq!(
        owner.media_match_runtime_snapshot.remote_status.as_deref(),
        Some("unavailable: no current file")
    );
    let expected_install_location = root
        .join("tools")
        .join("media-match")
        .join("bin")
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        owner
            .media_match_runtime_snapshot
            .install_location
            .as_deref(),
        Some(expected_install_location.as_str())
    );
    assert_eq!(
        state.media_match.install_location.as_deref(),
        Some(expected_install_location.as_str())
    );
    let settings = load_sorotte_ini_stored_client_settings_mvp_from_path(&config_path)
        .expect("Media Matching plugin setting config should be readable")
        .expect("Media Matching plugin setting config should exist");
    assert_eq!(settings.media_matching_plugin_enabled, Some(true));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_disables_stream_support_without_deleting_helper_details() {
    let root = test_temp_root("plugin-disable-stream-support-clears-work");
    let config_path = root.join("sorotte.ini");
    let saved_settings = StoredClientSettingsMvp {
        host: Some("saved.example".to_owned()),
        server_password: Some("saved-secret".into()),
        trusted_domains: Some(vec!["example.invalid".to_owned()]),
        ..StoredClientSettingsMvp::default()
    };
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(&config_path, &saved_settings)
        .expect("initial Stream Support settings should be persisted");
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path.clone()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&saved_settings);
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionHost,
        value: "draft.example".to_owned().into(),
    }));
    assert!(state.apply(GuiShellAction::RemoveServerPassword));
    assert!(state.apply(GuiShellAction::FocusConfigurationControl(
        SettingId::ConnectionHost,
    )));
    assert!(state.apply(GuiShellAction::BeginConfigurationTextEdit(
        SettingId::ConnectionHost,
    )));
    assert!(state.apply(GuiShellAction::UpdateConfigurationTextEdit(
        "typing.example".to_owned().into(),
    )));
    let focused_control = state.focused_configuration_control.clone();
    let edit_session = state.text_edit_session.clone();
    let (_probe_tx, probe_rx) = mpsc::channel();
    owner.startup_stream_helper_probe_completed = false;
    owner.startup_stream_helper_probe_rx = Some(probe_rx);
    owner.pending_stream_retry_target = Some("https://video.example.invalid/watch".to_owned());
    owner.managed_stream_helper_refresh_required = true;
    owner.stream_helper_runtime_snapshot.install_location =
        Some("C:/tools/sorotte-stream-helper".to_owned());
    owner.stream_helper_runtime_snapshot.downloader_status = Some("yt-dlp ready".to_owned());
    owner.stream_helper_runtime_snapshot.js_runtime_status = Some("node ready".to_owned());

    handle.push_request(GuiRuntimeRequest::SetPluginEnabled {
        plugin: GuiPluginSelection::StreamSupport,
        enabled: false,
    });
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(owner.startup_stream_helper_probe_completed);
    assert!(owner.startup_stream_helper_probe_rx.is_none());
    assert!(owner.pending_stream_retry_target.is_none());
    assert!(!owner.managed_stream_helper_refresh_required);
    assert_eq!(
        owner
            .stream_helper_runtime_snapshot
            .install_location
            .as_deref(),
        Some("C:/tools/sorotte-stream-helper")
    );
    assert_eq!(
        owner
            .stream_helper_runtime_snapshot
            .downloader_status
            .as_deref(),
        Some("yt-dlp ready")
    );
    assert_eq!(
        owner
            .stream_helper_runtime_snapshot
            .js_runtime_status
            .as_deref(),
        Some("node ready")
    );
    assert!(
        !state
            .plugin_enablement
            .enabled_for(GuiPluginSelection::StreamSupport)
    );

    let settings = load_sorotte_ini_stored_client_settings_mvp_from_path(&config_path)
        .expect("Stream Support plugin setting config should be readable")
        .expect("Stream Support plugin setting config should exist");
    assert_eq!(settings.stream_support_plugin_enabled, Some(false));
    assert_eq!(settings.host.as_deref(), Some("saved.example"));
    assert_eq!(
        settings
            .server_password
            .as_ref()
            .map(|secret| secret.expose_secret()),
        Some("saved-secret")
    );
    assert_eq!(
        settings.trusted_domains,
        Some(vec!["example.invalid".to_owned()])
    );
    assert_eq!(
        state.saved_configuration.stream_support_plugin_enabled,
        Some(false)
    );
    assert_eq!(
        state.configuration.settings.stream_support_plugin_enabled,
        Some(false)
    );
    assert_eq!(
        state.saved_configuration.host.as_deref(),
        Some("saved.example")
    );
    assert_eq!(
        state.configuration.settings.host.as_deref(),
        Some("draft.example")
    );
    assert_eq!(state.configuration.server_password, SecretDraft::Clear);
    assert_eq!(state.focused_configuration_control, focused_control);
    assert_eq!(state.text_edit_session, edit_session);
    assert!(state.has_unsaved_configuration_changes());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_persists_media_match_settings() {
    let root = test_temp_root("media-match-settings-owner");
    let config_path = root.join("sorotte.ini");
    let saved_settings = StoredClientSettingsMvp {
        language: Some("en".to_owned()),
        server_password: Some("saved-secret".into()),
        media_match_fingerprinting_enabled: Some(false),
        ..StoredClientSettingsMvp::default()
    };
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(&config_path, &saved_settings)
        .expect("initial Media Matching settings should persist");
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path.clone()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&saved_settings);
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::GeneralLanguage,
        value: "fr".to_owned().into(),
    }));
    assert!(state.apply(GuiShellAction::BeginServerPasswordChange));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionServerPassword,
        value: "replacement-secret".to_owned().into(),
    }));
    assert!(state.apply(GuiShellAction::FocusConfigurationControl(
        SettingId::GeneralLanguage,
    )));
    assert!(state.apply(GuiShellAction::BeginConfigurationTextEdit(
        SettingId::GeneralLanguage,
    )));
    assert!(state.apply(GuiShellAction::UpdateConfigurationTextEdit(
        "pt-BR".to_owned().into(),
    )));
    let focused_control = state.focused_configuration_control.clone();
    let edit_session = state.text_edit_session.clone();

    handle.push_request(GuiRuntimeRequest::SetMediaMatchFingerprintingEnabled(true));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    handle.push_request(GuiRuntimeRequest::SetMediaMatchBackgroundWarmupEnabled(
        false,
    ));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    handle.push_request(GuiRuntimeRequest::SetMediaMatchWireSharingEnabled(false));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    handle.push_request(GuiRuntimeRequest::SetMediaMatchRuntimeToleranceEnabled(
        false,
    ));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    handle.push_request(GuiRuntimeRequest::SetMediaMatchAutoplayPolicy(
        sorotte_media_match::MediaMatchAutoplayPolicy::AllowStrongSameMedia,
    ));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    let settings = load_sorotte_ini_stored_client_settings_mvp_from_path(&config_path)
        .expect("media-match settings config should be readable")
        .expect("media-match settings config should exist");
    assert_eq!(settings.media_match_fingerprinting_enabled, Some(true));
    assert_eq!(settings.media_match_background_warmup_enabled, Some(false));
    assert_eq!(settings.media_match_wire_sharing_enabled, Some(false));
    assert_eq!(settings.media_match_runtime_tolerance_enabled, Some(false));
    assert_eq!(
        settings.media_match_autoplay_policy.as_deref(),
        Some("AllowStrongSameMedia")
    );
    assert_eq!(settings.language.as_deref(), Some("en"));
    assert_eq!(
        settings
            .server_password
            .as_ref()
            .map(|secret| secret.expose_secret()),
        Some("saved-secret")
    );
    assert_eq!(
        state.saved_configuration.media_match_fingerprinting_enabled,
        Some(true)
    );
    assert_eq!(
        state
            .saved_configuration
            .media_match_background_warmup_enabled,
        Some(false)
    );
    assert_eq!(
        state.saved_configuration.media_match_wire_sharing_enabled,
        Some(false)
    );
    assert_eq!(
        state
            .saved_configuration
            .media_match_runtime_tolerance_enabled,
        Some(false)
    );
    assert_eq!(
        state
            .saved_configuration
            .media_match_autoplay_policy
            .as_deref(),
        Some("AllowStrongSameMedia")
    );
    assert_eq!(
        state
            .configuration
            .settings
            .media_match_fingerprinting_enabled,
        Some(true)
    );
    assert_eq!(
        state
            .configuration
            .settings
            .media_match_background_warmup_enabled,
        Some(false)
    );
    assert_eq!(
        state
            .configuration
            .settings
            .media_match_wire_sharing_enabled,
        Some(false)
    );
    assert_eq!(
        state
            .configuration
            .settings
            .media_match_runtime_tolerance_enabled,
        Some(false)
    );
    assert_eq!(
        state
            .configuration
            .settings
            .media_match_autoplay_policy
            .as_deref(),
        Some("AllowStrongSameMedia")
    );
    assert_eq!(state.saved_configuration.language.as_deref(), Some("en"));
    assert_eq!(state.configuration.settings.language.as_deref(), Some("fr"));
    assert_eq!(
        state.configuration.server_password,
        SecretDraft::Replace("replacement-secret".into())
    );
    assert_eq!(state.focused_configuration_control, focused_control);
    assert_eq!(state.text_edit_session, edit_session);
    assert!(state.media_match.settings.fingerprinting_enabled);
    assert!(!state.media_match.settings.background_warmup_enabled);
    assert!(!state.media_match.settings.wire_sharing_enabled);
    assert!(!state.media_match.settings.runtime_tolerance_enabled);
    assert_eq!(
        state.media_match.settings.autoplay_policy,
        sorotte_media_match::MediaMatchAutoplayPolicy::AllowStrongSameMedia
    );
    assert!(state.has_unsaved_configuration_changes());

    let restarted_owner =
        GuiPersistedConfigRuntimeOwner::with_config_path_and_startup_player(Some(config_path));
    assert!(
        restarted_owner
            .media_match_runtime_snapshot
            .settings
            .fingerprinting_enabled
    );
    assert!(
        !restarted_owner
            .media_match_runtime_snapshot
            .settings
            .background_warmup_enabled
    );
    assert!(
        !restarted_owner
            .media_match_runtime_snapshot
            .settings
            .wire_sharing_enabled
    );
    assert!(
        !restarted_owner
            .media_match_runtime_snapshot
            .settings
            .runtime_tolerance_enabled
    );
    assert_eq!(
        restarted_owner
            .media_match_runtime_snapshot
            .settings
            .autoplay_policy,
        sorotte_media_match::MediaMatchAutoplayPolicy::AllowStrongSameMedia
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_preserves_media_match_and_draft_when_persistence_fails() {
    let root = test_temp_root("media-match-settings-persist-failure");
    std::fs::create_dir_all(&root).expect("test directory should be created");
    let sentinel_path = root.join("unchanged.txt");
    std::fs::write(&sentinel_path, "unchanged").expect("sentinel should be written");
    let saved_settings = StoredClientSettingsMvp {
        language: Some("en".to_owned()),
        server_password: Some("saved-secret".into()),
        media_match_fingerprinting_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    };
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(root.clone()));
    owner
        .media_match_runtime_snapshot
        .settings
        .fingerprinting_enabled = true;
    let cancel = Arc::new(AtomicBool::new(false));
    owner.media_match_background_worker_cancel = Some(Arc::clone(&cancel));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&saved_settings);
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::GeneralLanguage,
        value: "fr".to_owned().into(),
    }));
    assert!(state.apply(GuiShellAction::BeginServerPasswordChange));
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::ConnectionServerPassword,
        value: "replacement-secret".to_owned().into(),
    }));
    assert!(state.apply(GuiShellAction::FocusConfigurationControl(
        SettingId::GeneralLanguage,
    )));
    assert!(state.apply(GuiShellAction::BeginConfigurationTextEdit(
        SettingId::GeneralLanguage,
    )));
    assert!(state.apply(GuiShellAction::UpdateConfigurationTextEdit(
        "pt-BR".to_owned().into(),
    )));
    let saved_before = state.saved_configuration.clone();
    let draft_before = state.configuration.settings.clone();
    let secret_before = state.configuration.server_password.clone();
    let focused_before = state.focused_configuration_control.clone();
    let edit_before = state.text_edit_session.clone();

    handle.push_request(GuiRuntimeRequest::SetMediaMatchFingerprintingEnabled(false));
    let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(!cancel.load(Ordering::SeqCst));
    assert!(state.media_match.settings.fingerprinting_enabled);
    assert!(
        owner
            .media_match_runtime_snapshot
            .settings
            .fingerprinting_enabled
    );
    assert_eq!(state.saved_configuration, saved_before);
    assert_eq!(state.configuration.settings, draft_before);
    assert_eq!(state.configuration.server_password, secret_before);
    assert_eq!(state.focused_configuration_control, focused_before);
    assert_eq!(state.text_edit_session, edit_before);
    assert!(state.has_unsaved_configuration_changes());
    assert!(actions.iter().any(|action| matches!(
        action,
        GuiShellAction::PushTransientNotification {
            level: GuiTransientNotificationLevel::Error,
            message,
        } if message.contains("Could not persist Media Matching settings")
    )));
    assert!(root.is_dir());
    assert_eq!(
        std::fs::read_to_string(&sentinel_path).expect("sentinel should remain readable"),
        "unchanged"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn cancelling_staged_secret_after_unrelated_resync_or_patch_restores_saved_baseline() {
    for stage_clear in [false, true] {
        for apply_persisted_patch in [false, true] {
            let case = format!(
                "{}-{}",
                if stage_clear { "clear" } else { "replace" },
                if apply_persisted_patch {
                    "persisted-patch"
                } else {
                    "resync"
                },
            );
            let root = test_temp_root(&format!("secret-baseline-{case}"));
            let config_path = root.join("sorotte.ini");
            let saved_settings = StoredClientSettingsMvp {
                language: Some("en".to_owned()),
                server_password: Some("original-secret".into()),
                plex_plugin_enabled: Some(true),
                plex_streaming_enabled: Some(false),
                ..StoredClientSettingsMvp::default()
            };
            upsert_sorotte_ini_stored_client_settings_mvp_at_path(&config_path, &saved_settings)
                .expect("secret baseline fixture should persist");
            let mut owner =
                GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path.clone()));
            owner.startup_saved_connect_attempted = true;
            owner.startup_remote_actions_attempted = true;
            owner.startup_public_server_hydration.completed = true;
            let handle = GuiQueuedRuntimeBridgeHandle::default();
            let mut state = SorotteGuiShellAppState::from_stored_settings(&saved_settings);

            if stage_clear {
                assert!(state.apply(GuiShellAction::RemoveServerPassword));
            } else {
                assert!(state.apply(GuiShellAction::BeginServerPasswordChange));
                assert!(state.apply(GuiShellAction::EditConfigurationText {
                    id: SettingId::ConnectionServerPassword,
                    value: "replacement-secret".to_owned().into(),
                }));
            }
            let staged_intent = state.configuration.server_password.clone();

            if apply_persisted_patch {
                assert!(state.apply(GuiShellAction::ApplyGuiPersistedSettingsPatch(
                    GuiPersistedSettingsPatch::PlexStreamingEnabled(true),
                )));
            } else {
                let mut resynced = state.configuration.to_stored_settings();
                resynced.language = Some("fr".to_owned());
                state.resync_from_settings(resynced);
            }

            assert_eq!(state.configuration.server_password, staged_intent);
            assert_eq!(
                state
                    .configuration
                    .settings
                    .server_password
                    .as_ref()
                    .map(|secret| secret.expose_secret()),
                Some("original-secret"),
                "{case}: unrelated projection must retain the raw saved secret baseline",
            );
            assert!(state.apply(GuiShellAction::CancelServerPasswordChange));
            assert_eq!(state.configuration.server_password, SecretDraft::Unchanged);
            assert_eq!(
                state
                    .configuration
                    .to_stored_settings()
                    .server_password
                    .as_ref()
                    .map(|secret| secret.expose_secret()),
                Some("original-secret"),
                "{case}: cancel must restore the original saved password",
            );

            if state.configuration.settings.language.as_deref() == Some("en") {
                assert!(state.apply(GuiShellAction::EditConfigurationText {
                    id: SettingId::GeneralLanguage,
                    value: "fr".to_owned().into(),
                }));
            }
            assert!(state.apply(GuiShellAction::BeginConfigurationSave));
            let submitted = state.configuration.to_stored_settings();
            handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
                GuiPendingCompletionRequest::SaveConfiguration(submitted),
            ));
            pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

            let disk = load_sorotte_ini_stored_client_settings_mvp_from_path(&config_path)
                .expect("secret baseline config should remain readable")
                .expect("secret baseline config should remain present");
            for (layer, settings) in [
                ("disk", &disk),
                ("saved", &state.saved_configuration),
                ("draft", &state.configuration.settings),
            ] {
                assert_eq!(
                    settings
                        .server_password
                        .as_ref()
                        .map(|secret| secret.expose_secret()),
                    Some("original-secret"),
                    "{case}: later save must preserve the original password at the {layer} layer",
                );
            }
            assert_eq!(state.configuration.server_password, SecretDraft::Unchanged);
            let _ = std::fs::remove_dir_all(root);
        }
    }
}

#[test]
fn plex_streaming_toggle_persists_disk_saved_draft_and_feature_runtime() {
    let root = test_temp_root("plex-streaming-four-layer-success");
    let config_path = root.join("sorotte.ini");
    let saved_settings = StoredClientSettingsMvp {
        server_password: Some("saved-secret".into()),
        plex_plugin_enabled: Some(true),
        plex_streaming_enabled: Some(false),
        ..StoredClientSettingsMvp::default()
    };
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(&config_path, &saved_settings)
        .expect("initial Plex streaming settings should persist");
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path.clone()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&saved_settings);

    assert!(owner.handle_toggle_plex_streaming_request(&handle, &mut state, true));

    let disk = load_sorotte_ini_stored_client_settings_mvp_from_path(&config_path)
        .expect("Plex streaming config should be readable")
        .expect("Plex streaming config should exist");
    assert_eq!(disk.plex_streaming_enabled, Some(true));
    assert_eq!(state.saved_configuration.plex_streaming_enabled, Some(true));
    assert_eq!(
        state.configuration.settings.plex_streaming_enabled,
        Some(true)
    );
    assert!(state.plex.streaming_enabled);
    assert!(owner.plex_runtime_snapshot.streaming_enabled);
    assert_eq!(
        disk.server_password
            .as_ref()
            .map(|secret| secret.expose_secret()),
        Some("saved-secret")
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn plex_sync_persistence_patch_preserves_unrelated_draft_and_secret_intent() {
    let root = test_temp_root("plex-sync-field-patch");
    let config_path = root.join("sorotte.ini");
    let saved_settings = StoredClientSettingsMvp {
        language: Some("en".to_owned()),
        server_password: Some("saved-secret".into()),
        plex_plugin_enabled: Some(true),
        plex_sync_enabled: Some(false),
        ..StoredClientSettingsMvp::default()
    };
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(&config_path, &saved_settings)
        .expect("initial Plex settings should persist");
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path.clone()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&saved_settings);
    stage_unrelated_plex_draft(&mut state);
    let focused_before = state.focused_configuration_control.clone();
    let edit_before = state.text_edit_session.clone();

    assert!(owner.handle_toggle_plex_sync_request(&handle, &mut state, true));

    let disk = load_sorotte_ini_stored_client_settings_mvp_from_path(&config_path)
        .expect("Plex config should be readable")
        .expect("Plex config should exist");
    assert_eq!(disk.plex_sync_enabled, Some(true));
    assert_eq!(disk.language.as_deref(), Some("en"));
    assert_eq!(
        disk.server_password
            .as_ref()
            .map(|secret| secret.expose_secret()),
        Some("saved-secret")
    );
    assert_eq!(state.saved_configuration.plex_sync_enabled, Some(true));
    assert_eq!(state.saved_configuration.language.as_deref(), Some("en"));
    assert_eq!(state.configuration.settings.plex_sync_enabled, Some(true));
    assert_eq!(state.configuration.settings.language.as_deref(), Some("fr"));
    assert_eq!(state.configuration.server_password, SecretDraft::Clear);
    assert_eq!(state.focused_configuration_control, focused_before);
    assert_eq!(state.text_edit_session, edit_before);
    assert!(state.plex.enabled);
    assert!(state.has_unsaved_configuration_changes());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn plex_server_selection_patch_preserves_unrelated_draft_and_secret_intent() {
    let root = test_temp_root("plex-server-field-patch");
    let config_path = root.join("sorotte.ini");
    let saved_settings = StoredClientSettingsMvp {
        language: Some("en".to_owned()),
        server_password: Some("saved-secret".into()),
        plex_plugin_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        ..StoredClientSettingsMvp::default()
    };
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(&config_path, &saved_settings)
        .expect("initial Plex settings should persist");
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path.clone()));
    owner.plex_servers.push(PlexServerConnection {
        name: "Raptor".to_owned(),
        machine_identifier: "raptor-machine".to_owned(),
        uri: "https://raptor.example:32400".to_owned(),
        access_token: "server-token".into(),
        owned: true,
        has_local_connection: false,
        connection_kind: PlexServerConnectionKind::Remote,
    });
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&saved_settings);
    stage_unrelated_plex_draft(&mut state);
    let focused_before = state.focused_configuration_control.clone();
    let edit_before = state.text_edit_session.clone();

    assert!(owner.handle_select_plex_server_request(
        &handle,
        &mut state,
        "raptor-machine".to_owned(),
        "https://raptor.example:32400".to_owned(),
    ));

    let disk = load_sorotte_ini_stored_client_settings_mvp_from_path(&config_path)
        .expect("Plex config should be readable")
        .expect("Plex config should exist");
    assert_eq!(
        disk.plex_selected_server_id.as_deref(),
        Some("raptor-machine")
    );
    assert_eq!(
        disk.plex_selected_server_url.as_deref(),
        Some("https://raptor.example:32400")
    );
    assert_eq!(
        disk.plex_selected_server_token
            .as_ref()
            .map(|secret| secret.expose_secret()),
        Some("server-token")
    );
    assert_eq!(disk.language.as_deref(), Some("en"));
    assert_eq!(
        disk.server_password
            .as_ref()
            .map(|secret| secret.expose_secret()),
        Some("saved-secret")
    );
    assert_eq!(
        state.saved_configuration.plex_selected_server_id.as_deref(),
        Some("raptor-machine")
    );
    assert_eq!(
        state
            .saved_configuration
            .plex_selected_server_url
            .as_deref(),
        Some("https://raptor.example:32400")
    );
    assert_eq!(
        state
            .saved_configuration
            .plex_selected_server_token
            .as_ref()
            .map(|secret| secret.expose_secret()),
        Some("server-token")
    );
    assert_eq!(
        state
            .configuration
            .settings
            .plex_selected_server_id
            .as_deref(),
        Some("raptor-machine")
    );
    assert_eq!(
        state
            .configuration
            .settings
            .plex_selected_server_url
            .as_deref(),
        Some("https://raptor.example:32400")
    );
    assert_eq!(
        state
            .configuration
            .settings
            .plex_selected_server_token
            .as_ref()
            .map(|secret| secret.expose_secret()),
        Some("server-token")
    );
    assert_eq!(state.saved_configuration.language.as_deref(), Some("en"));
    assert_eq!(state.configuration.settings.language.as_deref(), Some("fr"));
    assert_eq!(state.configuration.server_password, SecretDraft::Clear);
    assert_eq!(state.focused_configuration_control, focused_before);
    assert_eq!(state.text_edit_session, edit_before);
    assert_eq!(
        state.plex.selected_server_id.as_deref(),
        Some("raptor-machine")
    );
    assert_eq!(
        state.plex.selected_server_url.as_deref(),
        Some("https://raptor.example:32400")
    );
    assert!(state.has_unsaved_configuration_changes());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn plex_disconnect_patch_preserves_unrelated_draft_and_secret_intent() {
    let root = test_temp_root("plex-disconnect-field-patch");
    let config_path = root.join("sorotte.ini");
    let saved_settings = StoredClientSettingsMvp {
        language: Some("en".to_owned()),
        server_password: Some("saved-secret".into()),
        plex_plugin_enabled: Some(true),
        plex_sync_enabled: Some(true),
        plex_streaming_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_id: Some("raptor-machine".to_owned()),
        plex_selected_server_url: Some("https://raptor.example:32400".to_owned()),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    };
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(&config_path, &saved_settings)
        .expect("initial Plex settings should persist");
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path.clone()));
    owner.plex_servers.push(PlexServerConnection {
        name: "Raptor".to_owned(),
        machine_identifier: "raptor-machine".to_owned(),
        uri: "https://raptor.example:32400".to_owned(),
        access_token: "server-token".into(),
        owned: true,
        has_local_connection: false,
        connection_kind: PlexServerConnectionKind::Remote,
    });
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&saved_settings);
    stage_unrelated_plex_draft(&mut state);
    let focused_before = state.focused_configuration_control.clone();
    let edit_before = state.text_edit_session.clone();

    assert!(owner.handle_disconnect_plex_request(&handle, &mut state));

    let disk = load_sorotte_ini_stored_client_settings_mvp_from_path(&config_path)
        .expect("Plex config should be readable")
        .expect("Plex config should exist");
    assert_eq!(disk.plex_sync_enabled, Some(false));
    assert_eq!(disk.plex_streaming_enabled, Some(false));
    assert!(disk.plex_user_token.is_none());
    assert!(disk.plex_selected_server_id.is_none());
    assert_eq!(disk.language.as_deref(), Some("en"));
    assert_eq!(
        disk.server_password
            .as_ref()
            .map(|secret| secret.expose_secret()),
        Some("saved-secret")
    );
    assert!(state.saved_configuration.plex_user_token.is_none());
    assert!(state.saved_configuration.plex_selected_server_id.is_none());
    assert_eq!(state.saved_configuration.language.as_deref(), Some("en"));
    assert!(state.configuration.settings.plex_user_token.is_none());
    assert_eq!(state.configuration.settings.language.as_deref(), Some("fr"));
    assert_eq!(state.configuration.server_password, SecretDraft::Clear);
    assert_eq!(state.focused_configuration_control, focused_before);
    assert_eq!(state.text_edit_session, edit_before);
    assert!(!state.plex.authenticated);
    assert!(!state.plex.enabled);
    assert!(!state.plex.streaming_enabled);
    assert!(owner.plex_servers.is_empty());
    assert!(state.has_unsaved_configuration_changes());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn plex_persistence_failure_preserves_disk_saved_draft_and_runtime_state() {
    let root = test_temp_root("plex-sync-persist-failure");
    std::fs::create_dir_all(&root).expect("test directory should be created");
    let sentinel_path = root.join("unchanged.txt");
    std::fs::write(&sentinel_path, "unchanged").expect("sentinel should be written");
    let saved_settings = StoredClientSettingsMvp {
        language: Some("en".to_owned()),
        server_password: Some("saved-secret".into()),
        plex_plugin_enabled: Some(true),
        plex_sync_enabled: Some(false),
        ..StoredClientSettingsMvp::default()
    };
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(root.clone()));
    owner.plex_sync_next_tick_due_at = Some(std::time::Instant::now());
    let runtime_due_before = owner.plex_sync_next_tick_due_at;
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&saved_settings);
    stage_unrelated_plex_draft(&mut state);
    let saved_before = state.saved_configuration.clone();
    let draft_before = state.configuration.settings.clone();
    let secret_before = state.configuration.server_password.clone();
    let focused_before = state.focused_configuration_control.clone();
    let edit_before = state.text_edit_session.clone();
    let runtime_before = state.plex.clone();

    assert!(owner.handle_toggle_plex_sync_request(&handle, &mut state, true));

    assert_eq!(state.saved_configuration, saved_before);
    assert_eq!(state.configuration.settings, draft_before);
    assert_eq!(state.configuration.server_password, secret_before);
    assert_eq!(state.focused_configuration_control, focused_before);
    assert_eq!(state.text_edit_session, edit_before);
    assert_eq!(state.plex, runtime_before);
    assert_eq!(owner.plex_sync_next_tick_due_at, runtime_due_before);
    assert!(state.has_unsaved_configuration_changes());
    assert!(handle.drain_actions().iter().any(|action| matches!(
        action,
        GuiShellAction::PushTransientNotification {
            level: GuiTransientNotificationLevel::Warning,
            message,
        } if message.contains("Plex sync setting was not saved")
    )));
    assert!(root.is_dir());
    assert_eq!(
        std::fs::read_to_string(&sentinel_path).expect("sentinel should remain readable"),
        "unchanged"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_recovers_media_match_config_path_from_storage_snapshot() {
    let root = test_temp_root("media-match-settings-storage-snapshot");
    let config_path = root.join("sorotte.ini");
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state =
        SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    assert!(
        state.apply(GuiShellAction::ApplyGuiConfigStorageRuntimeSnapshot(
            GuiConfigStorageRuntimeSnapshot {
                config_path: Some(config_path.to_string_lossy().into_owned()),
                storage_root: Some(root.to_string_lossy().into_owned()),
                default_storage_root: Some(root.to_string_lossy().into_owned()),
                source_label: "test".to_owned(),
                external_override_active: false,
            },
        ))
    );

    handle.push_request(GuiRuntimeRequest::SetMediaMatchFingerprintingEnabled(true));
    pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert_eq!(owner.config_path, Some(config_path.clone()));
    let settings = load_sorotte_ini_stored_client_settings_mvp_from_path(&config_path)
        .expect("media-match settings config should be readable")
        .expect("media-match settings config should exist");
    assert_eq!(settings.media_match_fingerprinting_enabled, Some(true));
    let expected_install_location = root
        .join("tools")
        .join("media-match")
        .join("bin")
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        state.media_match.install_location.as_deref(),
        Some(expected_install_location.as_str())
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_preserves_media_match_tools_when_player_cache_clears() {
    let root = test_temp_root("media-match-player-cache-clear");
    let config_path = root.join("sorotte.ini");
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path));
    owner.refresh_startup_media_match_snapshot(None);
    owner.media_match_runtime_snapshot.current_decision = Some("strong: seeded".to_owned());
    owner.media_match_runtime_snapshot.nearest_match =
        Some("episode-b.mkv (strong: seeded)".to_owned());
    owner.media_match_runtime_snapshot.last_evidence = Some("seeded evidence".to_owned());
    let before = owner.media_match_runtime_snapshot.clone();

    owner.sync_player_from_lookup_and_settings(
        &|name| (name == "SOROTTE_GUI_ENABLE_TEST_PLAYER").then(|| "true".to_owned()),
        None,
        true,
    );

    assert_eq!(
        owner.media_match_runtime_snapshot.install_location,
        before.install_location
    );
    assert_eq!(
        owner.media_match_runtime_snapshot.ffmpeg_status,
        before.ffmpeg_status
    );
    assert_eq!(
        owner.media_match_runtime_snapshot.ffprobe_status,
        before.ffprobe_status
    );
    assert_eq!(
        owner.media_match_runtime_snapshot.cache_status,
        before.cache_status
    );
    assert_eq!(owner.media_match_runtime_snapshot.current_decision, None);
    assert_eq!(owner.media_match_runtime_snapshot.nearest_match, None);
    assert_eq!(owner.media_match_runtime_snapshot.last_evidence, None);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_config_runtime_owner_changes_config_storage_root_and_copies_known_files() {
    let env = TestEnvGuard::lock(&CONFIG_ROOT_ENV_LOCK);
    let prior_appdata = std::env::var_os("APPDATA");
    let prior_home = std::env::var_os("HOME");
    let prior_xdg_config_home = std::env::var_os("XDG_CONFIG_HOME");
    let prior_install_root = std::env::var_os(SOROTTE_CLIENT_INSTALL_ROOT_ENV);

    let default_env_parent = test_temp_root("config-storage-default-parent");
    let install_root = test_temp_root("config-storage-install-root");
    env.set_var(SOROTTE_CLIENT_INSTALL_ROOT_ENV, &install_root);
    if cfg!(windows) {
        env.set_var("APPDATA", &default_env_parent);
    } else if cfg!(target_os = "macos") {
        env.set_var("HOME", &default_env_parent);
    } else {
        env.set_var("XDG_CONFIG_HOME", &default_env_parent);
    }

    let old_root = test_temp_root("config-storage-old-root");
    let new_root = test_temp_root("config-storage-new-root");
    let _ = std::fs::remove_dir_all(&new_root);
    let old_config_path = old_root.join("sorotte.ini");
    let saved_settings = StoredClientSettingsMvp {
        host: Some("portable.example".to_owned()),
        room: Some("Portable".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(&old_config_path, &saved_settings)
        .expect("old config should be written");
    std::fs::write(
        legacy_gui_qsettings_store_path(&old_root, "MainWindow"),
        "[MainWindow]\nactiveView = setup\n",
    )
    .expect("old GUI state should be written");
    std::fs::create_dir_all(old_root.join("cache")).expect("cache directory should be created");
    std::fs::write(old_root.join("cache").join("plex-watch-cache.json"), "{}")
        .expect("Plex cache should be written");
    std::fs::create_dir_all(old_root.join("tools").join("stream-helper"))
        .expect("tools directory should be created");
    std::fs::write(
        old_root
            .join("tools")
            .join("stream-helper")
            .join("helper.txt"),
        "tool",
    )
    .expect("tool file should be written");
    std::fs::create_dir_all(old_root.join("tools").join("media-match").join("bin"))
        .expect("media-match tools directory should be created");
    std::fs::write(
        old_root
            .join("tools")
            .join("media-match")
            .join("bin")
            .join("ffmpeg.exe"),
        "tool",
    )
    .expect("media-match tool file should be written");
    std::fs::create_dir_all(old_root.join("updates")).expect("updates directory should be created");
    std::fs::write(old_root.join("updates").join("stage.txt"), "update")
        .expect("update staging file should be written");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(old_config_path.clone()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&saved_settings);
    assert!(state.apply(GuiShellAction::BeginConfigStorageRootChange(
        new_root.display().to_string(),
    )));
    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::ChangeConfigStorageRoot {
            target: GuiConfigStorageChangeTarget::CustomRoot(new_root.display().to_string()),
            settings: saved_settings.clone(),
        },
    ));

    let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        actions.iter().any(|action| {
            matches!(
                action,
                GuiShellAction::CompleteConfigStorageRootChange { snapshot, settings }
                    if snapshot.storage_root.as_deref()
                        == Some(new_root.to_string_lossy().as_ref())
                        && settings == &saved_settings
            )
        }),
        "root change should complete with the new storage snapshot"
    );
    assert_eq!(owner.config_path, Some(new_root.join("sorotte.ini")));
    assert!(
        old_config_path.exists(),
        "changing storage roots should preserve the old config file"
    );
    assert_eq!(
        load_sorotte_ini_stored_client_settings_mvp_from_path(&new_root.join("sorotte.ini"))
            .expect("new config should be readable"),
        Some(saved_settings)
    );
    assert!(
        legacy_gui_qsettings_store_path(&new_root, "MainWindow").exists(),
        "known GUI state should be copied to the new root"
    );
    assert!(
        new_root
            .join("cache")
            .join("plex-watch-cache.json")
            .exists(),
        "cache files should be copied to the new root"
    );
    assert!(
        new_root
            .join("tools")
            .join("stream-helper")
            .join("helper.txt")
            .exists(),
        "stream-helper tools should be copied to the new root"
    );
    assert!(
        new_root
            .join("tools")
            .join("media-match")
            .join("bin")
            .join("ffmpeg.exe")
            .exists(),
        "media-match tools should be copied to the new root"
    );
    assert!(
        new_root.join("updates").join("stage.txt").exists(),
        "update staging should be copied to the new root"
    );
    let install_locator_path = sorotte_client_install_locator_path(&install_root);
    let locator_contents =
        std::fs::read_to_string(&install_locator_path).expect("install locator should be readable");
    assert_eq!(
        parse_sorotte_client_install_locator_config_root(&locator_contents, &install_root),
        Some(new_root.clone())
    );

    match prior_appdata {
        Some(value) => env.set_var("APPDATA", value),
        None => env.remove_var("APPDATA"),
    }
    match prior_home {
        Some(value) => env.set_var("HOME", value),
        None => env.remove_var("HOME"),
    }
    match prior_xdg_config_home {
        Some(value) => env.set_var("XDG_CONFIG_HOME", value),
        None => env.remove_var("XDG_CONFIG_HOME"),
    }
    match prior_install_root {
        Some(value) => env.set_var(SOROTTE_CLIENT_INSTALL_ROOT_ENV, value),
        None => env.remove_var(SOROTTE_CLIENT_INSTALL_ROOT_ENV),
    }
    let _ = std::fs::remove_dir_all(&old_root);
    let _ = std::fs::remove_dir_all(&new_root);
    let _ = std::fs::remove_dir_all(&default_env_parent);
    let _ = std::fs::remove_dir_all(&install_root);
}

#[test]
fn config_storage_root_change_restores_target_config_when_locator_commit_fails() {
    let env = TestEnvGuard::lock(&CONFIG_ROOT_ENV_LOCK);
    let prior_appdata = std::env::var_os("APPDATA");
    let prior_home = std::env::var_os("HOME");
    let prior_xdg_config_home = std::env::var_os("XDG_CONFIG_HOME");
    let prior_install_root = std::env::var_os(SOROTTE_CLIENT_INSTALL_ROOT_ENV);

    let default_env_parent = test_temp_root("config-storage-rollback-default-parent");
    let install_root = test_temp_root("config-storage-rollback-install-root");
    env.set_var(SOROTTE_CLIENT_INSTALL_ROOT_ENV, &install_root);
    if cfg!(windows) {
        env.set_var("APPDATA", &default_env_parent);
    } else if cfg!(target_os = "macos") {
        env.set_var("HOME", &default_env_parent);
    } else {
        env.set_var("XDG_CONFIG_HOME", &default_env_parent);
    }

    let old_root = test_temp_root("config-storage-rollback-old-root");
    let target_root = test_temp_root("config-storage-rollback-target-root");
    let old_config_path = old_root.join("sorotte.ini");
    let target_config_path = target_root.join("sorotte.ini");
    let original_target_contents = b"[client_settings]\nname = target-before\n";
    std::fs::create_dir_all(&target_root).expect("target root should be created");
    std::fs::write(&target_config_path, original_target_contents)
        .expect("original target config should be written");
    std::fs::create_dir_all(sorotte_client_install_locator_path(&install_root))
        .expect("a directory at the locator path should force locator persistence to fail");

    let saved_settings = StoredClientSettingsMvp {
        host: Some("saved.example".to_owned()),
        room: Some("Saved".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    let attempted_settings = StoredClientSettingsMvp {
        host: Some("attempted.example".to_owned()),
        room: Some("Attempted".to_owned()),
        ..StoredClientSettingsMvp::default()
    };
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(&old_config_path, &saved_settings)
        .expect("old config should be written");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(old_config_path.clone()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&saved_settings);
    assert!(state.apply(GuiShellAction::BeginConfigStorageRootChange(
        target_root.display().to_string(),
    )));
    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::ChangeConfigStorageRoot {
            target: GuiConfigStorageChangeTarget::CustomRoot(target_root.display().to_string()),
            settings: attempted_settings,
        },
    ));

    let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);
    assert!(
        actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CancelConfigStorageRootChange)),
        "locator failure should cancel the shell transaction"
    );
    assert!(
        !actions.iter().any(|action| matches!(
            action,
            GuiShellAction::CompleteConfigStorageRootChange { .. }
        )),
        "locator failure must not publish a successful root change"
    );
    assert_eq!(owner.config_path, Some(old_config_path));
    assert_eq!(state.saved_configuration, saved_settings);
    assert_eq!(
        std::fs::read(&target_config_path).expect("target config should remain readable"),
        original_target_contents,
        "the pre-existing target config must be restored byte-for-byte"
    );

    match prior_appdata {
        Some(value) => env.set_var("APPDATA", value),
        None => env.remove_var("APPDATA"),
    }
    match prior_home {
        Some(value) => env.set_var("HOME", value),
        None => env.remove_var("HOME"),
    }
    match prior_xdg_config_home {
        Some(value) => env.set_var("XDG_CONFIG_HOME", value),
        None => env.remove_var("XDG_CONFIG_HOME"),
    }
    match prior_install_root {
        Some(value) => env.set_var(SOROTTE_CLIENT_INSTALL_ROOT_ENV, value),
        None => env.remove_var(SOROTTE_CLIENT_INSTALL_ROOT_ENV),
    }
    let _ = std::fs::remove_dir_all(&old_root);
    let _ = std::fs::remove_dir_all(&target_root);
    let _ = std::fs::remove_dir_all(&default_env_parent);
    let _ = std::fs::remove_dir_all(&install_root);
}

#[test]
fn gui_persisted_config_runtime_owner_clears_gui_data_files_and_returns_first_run_state() {
    let root = test_temp_root("clear-gui-data-owner");
    let path = root.join("sorotte.ini");
    let saved_settings = StoredClientSettingsMvp {
        host: Some("persisted.example".to_owned()),
        room: Some("Cinema".to_owned()),
        public_servers: Some(vec![("Saved".to_owned(), "saved.example:8999".to_owned())]),
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        ..StoredClientSettingsMvp::default()
    };
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(&path, &saved_settings)
        .expect("saved configuration should be written");
    persist_gui_ui_state_at_root(
        &root,
        &GuiPersistedUiState {
            active_view: Some(GuiShellView::Setup),
            selected_public_server_address: Some("custom.example:9001".to_owned()),
            selected_media_search_directory: None,
            last_media_dialog_directory: Some("D:/Dialogs".to_owned()),
            last_checked_for_updates: None,
            hide_empty_rooms: false,
            public_servers: vec![("Custom".to_owned(), "custom.example:9001".to_owned())],
            ..Default::default()
        },
    )
    .expect("GUI state should be written");
    crate::app::media_search_cache::persist_media_search_root_index_at_root(
        &root,
        &crate::app::media_search_cache::PersistedMediaSearchRootIndexV2 {
            version: 2,
            root_key: crate::app::media_search_cache::normalized_media_search_root_key(
                std::path::Path::new("C:/Media"),
            ),
            root_path: "C:/Media".to_owned(),
            built_at_unix_ms: 1,
            candidates_by_name: std::collections::HashMap::from([(
                "episode1.mkv".to_owned(),
                vec!["Season 1\\episode1.mkv".to_owned()],
            )]),
        },
    )
    .expect("media-search cache should be written");
    let plex_cache_path = root.join("cache").join("plex-watch-cache.json");
    std::fs::create_dir_all(
        plex_cache_path
            .parent()
            .expect("Plex cache path should have parent"),
    )
    .expect("Plex cache directory should be created");
    std::fs::write(&plex_cache_path, "{}").expect("Plex cache should be written");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(path.clone()));
    let media_root_key = crate::app::media_search_cache::normalized_media_search_root_key(
        std::path::Path::new("C:/Media"),
    );
    owner.attached_media_search_index = Some(GuiAttachedMediaSearchIndex {
        roots: vec![media_root_key.clone()],
        root_indexes_by_key: std::collections::HashMap::from([(
            media_root_key.clone(),
            GuiAttachedMediaSearchRootIndex {
                root_key: media_root_key.clone(),
                root_path: std::path::PathBuf::from("C:/Media"),
                built_at_unix_ms: 1,
                candidates_by_name: std::collections::HashMap::from([(
                    "episode1.mkv".to_owned(),
                    vec!["Season 1\\episode1.mkv".to_owned()],
                )]),
            },
        )]),
        roots_requiring_refresh: [media_root_key.clone()].into_iter().collect(),
    });
    owner.attached_media_search_next_retry_at = Some(std::time::Instant::now());
    owner.attached_media_search_progress = Some(GuiAttachedMediaSearchBuildProgress {
        total_roots: 1,
        completed_roots: 0,
        current_root_key: media_root_key.clone(),
        current_root_path: std::path::PathBuf::from("C:/Media"),
        scanned_directories: 1,
        indexed_files: 1,
    });
    owner.attached_media_search_build_state = GuiAttachedMediaSearchBuildState::Building;
    owner.attached_media_search_build_roots = vec![media_root_key.clone()];
    owner.unresolved_attached_media_target = Some("episode1.mkv".to_owned());
    let (_result_tx, result_rx) = std::sync::mpsc::channel();
    let cancel_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    owner.pending_attached_media_resolution = Some(GuiPendingAttachedMediaResolution {
        roots: vec![media_root_key.clone()],
        cancel_flag: cancel_flag.clone(),
        latest_progress: std::sync::Arc::new(std::sync::Mutex::new(None)),
        result_rx,
    });
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&saved_settings);

    assert!(state.apply(GuiShellAction::BeginClearGuiData));
    assert!(state.apply(GuiShellAction::ConfirmClearGuiData));
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::ClearGuiData,
    ));
    let actions = pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut state);

    assert!(
        actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteClearGuiData)),
        "clear-GUI-data runtime completion should round-trip through the queued owner"
    );
    assert!(!path.exists(), "clear-GUI-data should remove sorotte.ini");
    for store_name in ["MainWindow", "Interface", "MediaBrowseDialog"] {
        assert!(
            !legacy_gui_qsettings_store_path(&root, store_name).exists(),
            "clear-GUI-data should remove legacy GUI state store {store_name}"
        );
    }
    assert!(
        !crate::app::media_search_cache::persisted_media_search_cache_root_at_root(&root).exists(),
        "clear-GUI-data should remove the persisted media-search cache"
    );
    assert!(
        !plex_cache_path.exists(),
        "clear-GUI-data should remove the persisted Plex watch cache"
    );
    assert_eq!(state.configuration.launch_mode, GuiLaunchMode::FirstRun);
    assert_eq!(state.active_view, GuiShellView::Setup);
    assert_eq!(
        state.saved_configuration,
        StoredClientSettingsMvp::default()
    );
    assert_eq!(state.last_media_dialog_directory, None);
    assert!(state.public_servers.servers.is_empty());
    assert!(state.media_search.directories.is_empty());
    assert!(cancel_flag.load(std::sync::atomic::Ordering::Relaxed));
    assert!(owner.attached_media_search_index.is_none());
    assert!(owner.pending_attached_media_resolution.is_none());
    assert!(owner.attached_media_search_next_retry_at.is_none());
    assert!(owner.attached_media_search_progress.is_none());
    assert_eq!(
        owner.attached_media_search_build_state,
        GuiAttachedMediaSearchBuildState::Idle
    );
    assert!(owner.attached_media_search_build_roots.is_empty());
    assert!(owner.unresolved_attached_media_target.is_none());

    let _ = std::fs::remove_dir_all(&root);
}
