use super::*;

fn complete_detached_missing_media_search(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SorotteGuiShellAppState,
) -> String {
    assert!(state.apply(GuiShellAction::BeginMissingMediaSearch));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);

    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for detached missing-media search completion"
        );
        handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
            GuiPendingCompletionRequest::SearchMissingMedia,
        ));
        GuiQueuedRuntimeOwner::pump(owner, handle, state);
        let actions = handle.drain_actions();
        let completion = actions.iter().find_map(|action| match action {
            GuiShellAction::CompleteMissingMediaSearch(path) => Some(path.clone()),
            _ => None,
        });
        for action in actions {
            assert!(state.apply(action));
        }

        match completion {
            Some(Some(path)) => return path,
            Some(None) => panic!("detached missing-media search unexpectedly found no match"),
            None => std::thread::sleep(std::time::Duration::from_millis(10)),
        }
    }
}

#[test]
fn detached_missing_media_search_uses_saved_directories_until_draft_is_saved() {
    let root = test_temp_root("detached-saved-media-settings");
    let saved_root = root.join("saved-a");
    let draft_root = root.join("draft-b");
    let saved_target = saved_root.join("missing-target.mkv");
    let draft_target = draft_root.join("missing-target.mkv");
    let config_path = root.join("sorotte.ini");
    std::fs::create_dir_all(&saved_root).expect("saved media directory A should be created");
    std::fs::create_dir_all(&draft_root).expect("draft media directory B should be created");
    std::fs::write(&saved_target, b"saved-media-a")
        .expect("saved media target A should be written");
    std::fs::write(&draft_target, b"draft-media-b")
        .expect("draft media target B should be written");

    let saved_settings = StoredClientSettingsMvp {
        media_search_directories: Some(vec![saved_root.to_string_lossy().into_owned()]),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    };
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(&config_path, &saved_settings)
        .expect("saved media directory A should be persisted");

    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path.clone()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&saved_settings);
    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "missing-target.mkv".to_owned(),
        ]))
    );
    assert!(state.apply(GuiShellAction::EditConfigurationText {
        id: SettingId::MediaLibraryDirectories,
        value: draft_root.to_string_lossy().into_owned().into(),
    }));

    assert_eq!(
        owner.automatic_media_search_roots(&state),
        vec![saved_root.clone()],
        "detached runtime roots must ignore the unsaved directory B"
    );
    assert_eq!(
        GuiPersistedConfigRuntimeOwner::detached_runtime_settings_for_state(&state)
            .settings
            .media_search_directories,
        saved_settings.media_search_directories,
        "detached session snapshots must also ignore the unsaved directory B"
    );
    assert_eq!(
        complete_detached_missing_media_search(&mut owner, &handle, &mut state),
        saved_target.to_string_lossy(),
        "the first detached search must resolve through saved directory A"
    );

    assert!(state.apply(GuiShellAction::BeginConfigurationSave));
    let submitted_settings = state.configuration.to_stored_settings();
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SaveConfiguration(submitted_settings.clone()),
    ));
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    for action in handle.drain_actions() {
        assert!(state.apply(action));
    }
    assert_eq!(state.saved_configuration, submitted_settings);
    assert_eq!(
        owner.automatic_media_search_roots(&state),
        vec![draft_root.clone()],
        "directory B must become the detached runtime root after Save"
    );
    assert_eq!(
        GuiPersistedConfigRuntimeOwner::detached_runtime_settings_for_state(&state)
            .settings
            .media_search_directories,
        submitted_settings.media_search_directories,
        "detached session snapshots must adopt directory B after Save"
    );
    assert_eq!(
        complete_detached_missing_media_search(&mut owner, &handle, &mut state),
        draft_target.to_string_lossy(),
        "the subsequent detached search must resolve through newly saved directory B"
    );

    let persisted = load_sorotte_ini_stored_client_settings_mvp_from_path(&config_path)
        .expect("saved directory B configuration should remain readable")
        .expect("saved directory B configuration should remain present");
    assert_eq!(persisted, submitted_settings);
    let _ = std::fs::remove_dir_all(root);
}
