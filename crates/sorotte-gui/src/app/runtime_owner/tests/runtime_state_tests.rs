use super::*;
use crate::app::feature_slices::GuiRuntimeInput;

#[test]
fn worker_applies_pending_completion_before_the_next_settings_action() {
    let mut shell = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings::default());
    shell.pending_operation = Some(crate::app::shell_state::GuiPendingOperationState {
        kind: GuiPendingOperationKind::SaveConfiguration,
    });
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    let (_, handle) = GuiQueuedRuntimeBridge::new();
    owner.input_changed(&handle, &GuiRuntimeInput::from_shell(&shell));
    let mut worker = owner.runtime_state.take().unwrap();
    let saved = StoredClientSettings {
        username: Some("saved-user".to_owned()),
        ..StoredClientSettings::default()
    };
    GuiPersistedConfigRuntimeOwner::push_actions_and_project(
        &handle,
        &mut worker,
        vec![GuiShellAction::CompleteConfigurationSave(saved.clone())],
    );
    assert!(worker.session.pending_operation.is_none());
    assert_eq!(worker.settings.saved, saved);
    assert!(!worker.has_unsaved_configuration_changes());

    let next = StoredClientSettings {
        username: Some("next-user".to_owned()),
        ..saved
    };
    GuiPersistedConfigRuntimeOwner::push_actions_and_project(
        &handle,
        &mut worker,
        vec![GuiShellAction::ApplyGuiSavedConfigurationRuntimeSnapshot(
            crate::app::shell_state::GuiSavedConfigurationRuntimeSnapshot {
                settings: next.clone(),
            },
        )],
    );
    assert_eq!(worker.settings.saved, next);
    assert!(worker.has_unsaved_configuration_changes());
    assert!(
        shell.pending_operation.is_some(),
        "UI has not consumed either action yet"
    );
    let actions = handle.drain_actions();
    assert_eq!(actions.len(), 2);
    for action in actions {
        assert!(shell.apply(action));
    }
    assert_eq!(
        GuiRuntimeInput::from_shell(&shell).to_runtime_state(),
        worker
    );
}

#[test]
fn ordinary_polls_keep_worker_changes_and_changed_input_replaces_them() {
    let shell = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings::default());
    let input = GuiRuntimeInput::from_shell(&shell);
    let (_, handle) = GuiQueuedRuntimeBridge::new();
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.startup_saved_connect_attempted = true;
    owner.input_changed(&handle, &input);
    owner.runtime_state.as_mut().unwrap().playlist.shuffle_nonce = 42;
    owner.poll(&handle);
    owner.poll(&handle);
    assert_eq!(
        owner.runtime_state.as_ref().unwrap().playlist.shuffle_nonce,
        42
    );
    let mut changed = shell;
    changed.playlist_shuffle_nonce = 7;
    owner.input_changed(&handle, &GuiRuntimeInput::from_shell(&changed));
    assert_eq!(
        owner.runtime_state.as_ref().unwrap().playlist.shuffle_nonce,
        7
    );
}

#[test]
fn playlist_selection_output_preserves_ui_edit_and_focus_and_respects_local_selection() {
    use crate::app::shell_state::{GuiShellModal, SettingId};
    let mut shell = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettings::default()
    });
    shell.apply_shared_playlist_entries(
        vec!["first.mkv".to_owned(), "second.mkv".to_owned()],
        Some(0),
        false,
    );
    assert!(shell.apply(GuiShellAction::FocusConfigurationControl(
        SettingId::GeneralLanguage
    )));
    assert!(shell.apply(GuiShellAction::BeginConfigurationTextEdit(
        SettingId::GeneralLanguage
    )));
    assert!(shell.apply(GuiShellAction::UpdateConfigurationTextEdit(
        "fr".to_owned().into()
    )));
    shell.open_modal = Some(GuiShellModal::About);
    let focus = shell.focused_configuration_control.clone();
    let edit = shell.text_edit_session.clone();
    let mut worker = GuiRuntimeInput::from_shell(&shell).to_runtime_state();
    assert!(worker.apply(GuiShellAction::ApplySharedPlaylistSelection(Some(1))));
    assert!(shell.apply(GuiShellAction::ApplySharedPlaylistSelection(Some(1))));
    assert_eq!(shell.selection.selected_main_window_playlist, Some(1));
    assert_eq!(shell.focused_configuration_control, focus);
    assert_eq!(shell.text_edit_session, edit);
    assert_eq!(shell.open_modal, Some(GuiShellModal::About));
    assert_eq!(
        GuiRuntimeInput::from_shell(&shell).to_runtime_state(),
        worker
    );

    shell.set_main_window_playlist_selection(Some(0), true);
    assert!(!shell.apply(GuiShellAction::ApplySharedPlaylistSelection(Some(1))));
    assert_eq!(shell.selection.selected_main_window_playlist, Some(0));
    assert!(shell.main_window_playlist_selection_is_local);
    assert_eq!(shell.text_edit_session, edit);
}

#[test]
fn cancellation_settles_the_same_feature_state_in_worker_and_ui() {
    use crate::app::shell_state::GuiPendingOperationState;
    let kinds = [
        GuiPendingOperationKind::SaveConfiguration,
        GuiPendingOperationKind::DiscardConfigurationChanges,
        GuiPendingOperationKind::ReloadConfiguration,
        GuiPendingOperationKind::ClearGuiData,
        GuiPendingOperationKind::ChangeConfigStorageRoot,
        GuiPendingOperationKind::ConnectSavedServer,
        GuiPendingOperationKind::DisconnectSession,
        GuiPendingOperationKind::ConnectPublicServer,
        GuiPendingOperationKind::RefreshPublicServers,
        GuiPendingOperationKind::SearchMissingMedia,
        GuiPendingOperationKind::SetPlaybackPause(true),
        GuiPendingOperationKind::TogglePlaybackPause,
        GuiPendingOperationKind::SendChatMessage,
    ];
    for kind in kinds {
        let mut shell =
            SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings::default());
        shell.pending_operation = Some(GuiPendingOperationState { kind });
        if kind == GuiPendingOperationKind::SendChatMessage {
            shell.outgoing_chat_message = Some("queued message".to_owned());
        }
        let mut worker = GuiRuntimeInput::from_shell(&shell).to_runtime_state();
        assert!(
            worker.apply(GuiShellAction::CancelPendingOperation),
            "{kind:?}"
        );
        assert!(
            shell.apply(GuiShellAction::CancelPendingOperation),
            "{kind:?}"
        );
        assert_eq!(
            GuiRuntimeInput::from_shell(&shell).to_runtime_state(),
            worker,
            "{kind:?}"
        );
    }
}

#[test]
fn public_server_refresh_preserves_selection_and_pending_completion_before_next_action() {
    let mut shell = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        public_servers: Some(vec![
            ("First".to_owned(), "first.example:8999".to_owned()),
            ("Selected".to_owned(), "selected.example:9000".to_owned()),
        ]),
        ..StoredClientSettings::default()
    });
    shell.set_selected_public_server_index(Some(1));
    assert!(shell.apply(GuiShellAction::BeginPublicServerRefresh));
    let mut worker = GuiRuntimeInput::from_shell(&shell).to_runtime_state();
    for action in [
        GuiShellAction::CompletePublicServerRefresh(vec![
            (
                "Selected renamed".to_owned(),
                "selected.example:9000".to_owned(),
            ),
            ("New".to_owned(), "new.example:9001".to_owned()),
        ]),
        GuiShellAction::ApplyStartupPublicServerCache(vec![
            ("New".to_owned(), "new.example:9001".to_owned()),
            (
                "Selected again".to_owned(),
                "selected.example:9000".to_owned(),
            ),
        ]),
    ] {
        assert!(worker.apply(action.clone()));
        assert!(shell.apply(action));
        assert_eq!(
            shell.selected_public_server_address(),
            Some("selected.example:9000")
        );
        assert_eq!(
            GuiRuntimeInput::from_shell(&shell).to_runtime_state(),
            worker
        );
    }
}

#[test]
fn plex_search_completion_preserves_selection_and_discards_results_on_error_in_both_owners() {
    use crate::app::shell_state::{GuiPlexPlaylistSearchResult, GuiPlexPlaylistSearchState};
    let result = |key: &str| GuiPlexPlaylistSearchResult {
        rating_key: key.to_owned(),
        title: key.to_owned(),
        parent_title: None,
        grandparent_title: None,
        media_type: sorotte_plex::PlexMediaType::Movie,
        duration_millis: None,
        file_name: None,
    };
    for error in [None, Some(" Search failed ".to_owned())] {
        let mut shell =
            SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings::default());
        shell.plex_playlist_search = Some(GuiPlexPlaylistSearchState {
            query: "movie".to_owned(),
            searching: true,
            selected_index: Some(1),
            results: vec![result("old-1"), result("old-2")],
            ..Default::default()
        });
        let mut worker = GuiRuntimeInput::from_shell(&shell).to_runtime_state();
        let action = GuiShellAction::CompletePlexPlaylistSearch {
            query: "movie".to_owned(),
            results: vec![result("new-1"), result("new-2")],
            error: error.clone(),
        };
        assert!(worker.apply(action.clone()));
        assert!(shell.apply(action));
        assert_eq!(
            GuiRuntimeInput::from_shell(&shell).to_runtime_state(),
            worker
        );
        let search = worker.plex.playlist_search.as_ref().unwrap();
        assert_eq!(
            search.selected_index,
            if error.is_some() { None } else { Some(1) }
        );
        assert_eq!(search.results.len(), if error.is_some() { 0 } else { 2 });
    }
}

#[test]
fn update_results_and_room_edits_preserve_the_same_worker_and_ui_features() {
    use crate::app::{
        remote_services::{UpdateCheckResult, UpdateCheckStatus},
        shell_state::SettingId,
    };
    let mut shell = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        room: Some("old-room".to_owned()),
        ..StoredClientSettings::default()
    });
    let mut worker = GuiRuntimeInput::from_shell(&shell).to_runtime_state();
    for action in [
        GuiShellAction::EditConfigurationText {
            id: SettingId::ConnectionRoom,
            value: "new-room".to_owned().into(),
        },
        GuiShellAction::BeginUpdateCheck {
            user_initiated: true,
        },
        GuiShellAction::ApplyUpdateCheckResult(UpdateCheckResult {
            status: UpdateCheckStatus::UpToDate,
            message: "Current".to_owned(),
            url: None,
            candidate: None,
            self_update_supported: false,
            public_servers: Some(vec![(
                "Primary".to_owned(),
                "primary.example:8999".to_owned(),
            )]),
            checked_at_utc: "2026-09-11T00:00:00Z".to_owned(),
            user_initiated: true,
        }),
    ] {
        assert!(worker.apply(action.clone()));
        assert!(shell.apply(action));
        let expected = GuiRuntimeInput::from_shell(&shell).to_runtime_state();
        assert_eq!(expected, worker);
    }
}
