use super::*;

#[test]
fn gui_shell_app_state_projects_main_window_widget_trees() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        shared_playlist_enabled: Some(true),
        username: Some("Alice".to_owned()),
        player_path: Some("mpv".to_owned()),
        room: Some("Lounge".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(state.apply(GuiShellAction::AddMainWindowUser("Bob".to_owned())));
    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "One".to_owned(),
            "Two".to_owned(),
        ]))
    );
    assert!(state.apply(GuiShellAction::SelectMainWindowUser(1)));
    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(1)));
    assert!(state.apply(GuiShellAction::ApplyGuiDraftRuntimeSnapshot(
        GuiDraftRuntimeSnapshot {
            outgoing_chat_message: Some("hello widget ".to_owned()),
        },
    )));

    let tree = state.main_window_widget_tree();
    assert_eq!(tree.label, "Room");
    assert!(tree.find("main-window:tabs").is_none());
    assert!(tree.find("main-window:tab:overview").is_none());
    let room_panel = tree
        .find("main-window:connection")
        .expect("combined room panel should exist in widget tree");
    assert_eq!(room_panel.kind, GuiWidgetKind::Panel);
    assert_eq!(room_panel.label, "Room");
    assert!(tree.find("main-window:browser").is_none());
    let participants = tree
        .find("main-window:participants")
        .expect("current-room participants should exist in widget tree");
    assert_eq!(
        participants
            .children
            .iter()
            .map(|child| child.id.as_str())
            .collect::<Vec<_>>(),
        vec!["main-window:user:0", "main-window:user:1"]
    );
    let local_user_state = tree
        .find("main-window:user:0:state")
        .expect("local user state should exist in widget tree");
    assert_eq!(local_user_state.kind, GuiWidgetKind::Status);
    assert!(local_user_state.selected);
    let selected_remote_user_state = tree
        .find("main-window:user:1:state")
        .expect("remote user state should exist in widget tree");
    assert_eq!(selected_remote_user_state.kind, GuiWidgetKind::Status);
    assert!(!selected_remote_user_state.selected);
    assert!(tree.find("main-window:user:new").is_none());
    assert!(tree.find("main-window:user:1:open").is_none());
    assert!(tree.find("main-window:user:1:ready").is_none());
    let room_toggle = tree
        .find("main-window:room-actions:toggle")
        .expect("room-change toggle should exist in widget tree");
    assert_eq!(room_toggle.kind, GuiWidgetKind::Button);
    assert_eq!(room_toggle.label, "Change Room");
    assert!(!room_toggle.selected);
    assert!(
        tree.find("main-window:room-input").is_none(),
        "room-change form should be collapsed by default"
    );

    assert!(state.apply(GuiShellAction::ToggleMainWindowRoomChange));
    let tree = state.main_window_widget_tree();
    let room_toggle = tree
        .find("main-window:room-actions:toggle")
        .expect("room-change toggle should still exist in widget tree");
    assert_eq!(room_toggle.label, "Change Room");
    assert!(room_toggle.selected);
    let room_input = tree
        .find("main-window:room-input")
        .expect("room input should exist once room change is expanded");
    assert_eq!(room_input.kind, GuiWidgetKind::TextInput);
    assert_eq!(room_input.label, "Room");
    assert_eq!(room_input.value.as_deref(), Some("Lounge"));
    assert!(room_input.enabled);
    assert!(tree.find("main-window:username").is_none());
    let room_control = tree
        .find("main-window:room-control")
        .expect("room-control status should exist in widget tree");
    assert_eq!(room_control.kind, GuiWidgetKind::Status);
    assert_eq!(
        room_control.value.as_deref(),
        Some("Unavailable: no active server session.")
    );

    let playlist = tree
        .find("main-window:playlist:1")
        .expect("selected playlist row should exist in widget tree");
    assert_eq!(playlist.kind, GuiWidgetKind::ListItem);
    assert!(playlist.selected);
    let playlist_add_files = tree
        .find("main-window:playlist:add-files")
        .expect("playlist add-files button should exist in widget tree");
    assert_eq!(playlist_add_files.kind, GuiWidgetKind::Button);
    let playlist_add_url = tree
        .find("main-window:playlist:add-url")
        .expect("playlist add-url button should exist in widget tree");
    assert_eq!(playlist_add_url.kind, GuiWidgetKind::Button);
    let playlist_add_plex = tree
        .find("main-window:playlist:add-plex")
        .expect("playlist add-plex button should exist in widget tree");
    assert_eq!(playlist_add_plex.kind, GuiWidgetKind::Button);
    assert!(
        !playlist_add_plex.enabled,
        "Plex playlist picker should be disabled until a Plex server is selected"
    );
    let playlist_header = tree
        .find("main-window:playlist-header:actions")
        .expect("playlist header actions should exist in widget tree");
    assert_eq!(
        playlist_header
            .children
            .iter()
            .map(|child| child.id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "main-window:playlist:add-files",
            "main-window:playlist:add-url",
            "main-window:playlist:add-plex",
            "main-window:playlist:more-menu",
        ]
    );
    let playlist_more_menu = tree
        .find("main-window:playlist:more-menu")
        .expect("playlist more menu should exist in widget tree");
    assert!(
        playlist_more_menu.enabled,
        "playlist More menu should remain expandable even when some nested actions are disabled"
    );
    assert!(
        playlist_more_menu
            .children
            .iter()
            .any(|child| child.id == "main-window:playlist:load")
    );
    assert!(
        playlist_more_menu
            .children
            .iter()
            .any(|child| child.id == "main-window:playlist:save")
    );
    assert_eq!(
        playlist_more_menu
            .children
            .iter()
            .map(|child| child.id.as_str())
            .collect::<Vec<_>>(),
        vec!["main-window:playlist:load", "main-window:playlist:save"]
    );
    let playlist_row_remove = tree
        .find("main-window:playlist:1:remove")
        .expect("playlist row remove action should exist on the selected row");
    assert_eq!(playlist_row_remove.kind, GuiWidgetKind::Button);
    assert!(
        tree.find("main-window:playlist-selection:actions")
            .is_none()
    );
    assert!(tree.find("main-window:playlist:count").is_none());
    assert!(tree.find("main-window:playlist-empty").is_none());
    assert!(tree.find("main-window:playlist:new").is_none());
    assert!(tree.find("main-window:playlist:add").is_none());
    assert!(tree.find("main-window:user:add").is_none());

    let chat_input = tree
        .find("main-window:chat-input")
        .expect("chat input should exist in widget tree");
    assert_eq!(chat_input.kind, GuiWidgetKind::TextInput);
    assert_eq!(chat_input.value.as_deref(), Some("hello widget "));
    assert_eq!(chat_input.enabled, state.commands.can_send_chat_message);
}

#[test]
fn gui_shell_app_state_displays_plex_playlist_rows_by_media_name() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    let media_name = "[EG]Gurren_Lagann_03_BD(720p_10bit)[BB5590A5].mkv";
    let playlist_entry = format_plex_playlist_uri(&PlexPlaylistUri {
        machine_identifier: "3f6ba9fad8b4b33a803f1151b5d49ee1fd83e860".to_owned(),
        rating_key: "2918".to_owned(),
        title: Some("Gurren Lagann Episode 3".to_owned()),
        file_name: Some(media_name.to_owned()),
        duration_millis: Some(1_452_000),
        size_bytes: Some(657_000_000),
        media_type: Some(PlexMediaType::Episode),
    });

    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            playlist_entry.clone(),
        ]))
    );

    let tree = state.main_window_widget_tree();
    let playlist_row = tree
        .find("main-window:playlist:0")
        .expect("Plex playlist row should exist");
    assert_eq!(playlist_row.label, media_name);
    assert_eq!(
        state.current_shared_playlist_entries(),
        vec![playlist_entry]
    );
}

#[test]
fn gui_shell_app_state_projects_compact_playback_controls_and_ready_button_text() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        player_path: Some("mpv".to_owned()),
        room: Some("Lounge".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut snapshot = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
    snapshot.can_toggle_pause = true;
    snapshot.can_seek = true;
    snapshot.can_undo_seek = true;
    snapshot.can_set_offset = true;
    snapshot.can_set_ready = true;
    snapshot.users = vec![browser_runtime_user("alice", "Lounge", true, false, false)];

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        snapshot.clone(),
    )));

    let tree = state.main_window_widget_tree();
    assert!(
        tree.find("main-window:controls").is_none(),
        "standalone Controls panel should be folded into the Playlist panel"
    );
    let playlist_playback = tree
        .find("main-window:playlist-playback")
        .expect("playlist playback footer should exist");
    assert_eq!(playlist_playback.label, "Playback");
    let playback_actions = tree
        .find("main-window:controls:playback-actions")
        .expect("compact playback controls should exist in the playlist footer");
    assert_eq!(
        playback_actions.layout_mode,
        Some(GuiLayoutMode::CompactButtonWrap {
            button_width: 40.0,
            button_height: 36.0,
            gap: 8.0,
        })
    );
    assert_eq!(playback_actions.children.len(), 5);
    assert!(
        tree.find("main-window:control:set-offset").is_none(),
        "Set Offset should not be exposed in the consolidated playlist controls"
    );
    assert_eq!(
        tree.find("main-window:control:set-ready")
            .expect("ready button should exist")
            .label,
        "Not Ready"
    );

    snapshot.users = vec![browser_runtime_user("alice", "Lounge", true, true, false)];
    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        snapshot.clone(),
    )));

    let tree = state.main_window_widget_tree();
    assert_eq!(
        tree.find("main-window:control:set-ready")
            .expect("ready button should still exist")
            .label,
        "Ready"
    );

    snapshot.users = vec![browser_runtime_user("alice", "Lounge", true, false, false)];
    state.pending_local_ready_target = Some(true);
    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot)));

    let tree = state.main_window_widget_tree();
    let ready_button = tree
        .find("main-window:control:set-ready")
        .expect("ready button should exist while readiness is pending");
    assert_eq!(ready_button.label, "Ready");
    assert!(ready_button.enabled);
}

#[test]
fn gui_shell_app_state_disables_playback_controls_when_playlist_is_empty() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        player_path: Some("mpv".to_owned()),
        room: Some("Lounge".to_owned()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.commands.can_toggle_pause = true;
    let mut snapshot = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
    snapshot.can_toggle_pause = true;
    snapshot.can_seek = true;
    snapshot.can_undo_seek = true;
    snapshot.can_set_offset = true;
    snapshot.can_toggle_autoplay = true;
    snapshot.can_adjust_autoplay_threshold = true;
    snapshot.can_set_ready = true;
    snapshot.users = vec![browser_runtime_user("alice", "Lounge", true, false, false)];
    snapshot.playlist = Vec::new();
    snapshot.playlist_entry_ids.clear();
    snapshot.playlist_source_states.clear();

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        snapshot.clone(),
    )));

    let tree = state.main_window_widget_tree();
    for id in [
        "main-window:control:play",
        "main-window:control:pause",
        "main-window:control:toggle-pause",
        "main-window:control:seek",
        "main-window:control:undo-seek",
    ] {
        assert!(
            !tree
                .find(id)
                .unwrap_or_else(|| panic!("{id} should exist"))
                .enabled,
            "{id} should be disabled while the shared playlist is empty"
        );
    }
    assert!(
        tree.find("main-window:control:set-ready")
            .expect("ready button should exist")
            .enabled,
        "Ready should stay available even while the shared playlist is empty"
    );

    snapshot.playlist = vec!["episode1.mkv".to_owned()];
    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot)));
    state.commands.can_toggle_pause = true;

    let tree = state.main_window_widget_tree();
    assert!(tree.find("main-window:control:play").unwrap().enabled);
    assert!(
        tree.find("main-window:control:toggle-pause")
            .unwrap()
            .enabled
    );
    assert!(tree.find("main-window:control:set-ready").unwrap().enabled);
    assert!(
        tree.find("main-window:control:autoplay-toggle").is_none(),
        "autoplay controls should not be shown in the consolidated Room dashboard"
    );
}

#[test]
fn gui_shell_app_state_projects_player_setup_into_main_window_widgets() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        host: Some("syncplay.example".to_owned()),
        player_path: Some("C:/missing/mpv.exe".to_owned()),
        room: Some("Lounge".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(
        state.apply(GuiShellAction::ApplyGuiPlayerSetupRuntimeSnapshot(
            GuiPlayerSetupRuntimeSnapshot {
                issue: Some(GuiPlayerSetupIssue {
                    kind: GuiPlayerSetupIssueKind::ExitedAfterLaunch,
                    message: "GUI-owned mpv exited with exit code 1.".to_owned(),
                }),
            },
        ))
    );

    let main_window = state.main_window_widget_tree();
    assert!(main_window.find("main-window:player-setup").is_some());
    assert!(
        main_window
            .find("main-window:player-setup:retry")
            .expect("retry button should exist")
            .enabled
    );
    assert!(
        main_window
            .find("main-window:player-setup:open-settings")
            .expect("open-settings button should exist")
            .enabled
    );

    let shell = state.shell_widget_tree();
    assert_eq!(
        shell
            .find("shell:open-modal")
            .and_then(|node| node.value.as_deref()),
        Some("player-setup")
    );
    assert!(
        shell
            .find("shell:modal:close")
            .expect("player setup modal close button should exist")
            .enabled
    );
}

#[test]
fn gui_shell_app_state_projects_runtime_room_control_status_into_main_window_widget_tree() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("+room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut snapshot = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
    snapshot.room_name = "+room1".to_owned();
    snapshot.controlled_room_active = true;
    snapshot.room_control_status = "Not granted by server: room controls are locked.".to_owned();

    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot)));

    let tree = state.main_window_widget_tree();
    assert_eq!(
        tree.find("main-window:room-control")
            .and_then(|node| node.value.as_deref()),
        Some("Not granted by server: room controls are locked.")
    );
}

#[test]
fn gui_shell_app_state_projects_stream_seek_refill_without_changing_local_file_surface() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        player_path: Some("mpv".to_owned()),
        room: Some("Lounge".to_owned()),
        ..StoredClientSettingsMvp::default()
    });

    assert!(
        state
            .main_window_widget_tree()
            .find("main-window:seek-preparation")
            .is_none(),
        "ordinary local-file playback must not gain a stream refill panel"
    );

    assert!(
        state.apply(GuiShellAction::ApplyGuiSeekPreparationRuntimeSnapshot(
            GuiSeekPreparationRuntimeSnapshot {
                preparation: Some(GuiSeekPreparationState {
                    phase: GuiSeekPreparationPhase::Refilling,
                    frozen_target_seconds: 135.0,
                    cache_refill_percent: Some(64.6),
                    buffered_ahead_seconds: Some(12.3),
                    nearest_safe_buffered_position_seconds: Some(128.0),
                    can_keep_waiting: true,
                    can_cancel_and_remain: false,
                    can_join_nearest_buffered: true,
                }),
                degraded_reason: None,
            },
        ))
    );

    let tree = state.main_window_widget_tree();
    assert_eq!(
        tree.find("main-window:seek-preparation:status")
            .and_then(|node| node.value.as_deref()),
        Some("Buffer refill 65%")
    );
    assert_eq!(
        tree.find("main-window:seek-preparation:target")
            .and_then(|node| node.value.as_deref()),
        Some("02:15")
    );
    assert_eq!(
        tree.find("main-window:seek-preparation:buffered-ahead")
            .and_then(|node| node.value.as_deref()),
        Some("12.3 s")
    );
    assert!(
        tree.find("main-window:seek-preparation:refill")
            .and_then(|node| node.tooltip.as_deref())
            .is_some_and(|tooltip| tooltip.contains("not file download progress"))
    );
    assert!(
        tree.find("main-window:seek-preparation:cancel").is_none(),
        "cancel must remain hidden after the core says the primary seek cannot be revoked"
    );
    assert!(
        tree.find("main-window:seek-preparation:join-nearest")
            .is_some_and(|node| node.enabled)
    );

    assert!(
        state.apply(GuiShellAction::ApplyGuiSeekPreparationRuntimeSnapshot(
            GuiSeekPreparationRuntimeSnapshot {
                preparation: None,
                degraded_reason: Some(GuiSeekPreparationDegradedReason::ConvergenceDegraded),
            },
        ))
    );
    let convergence_degraded = state.main_window_widget_tree();
    assert_eq!(
        convergence_degraded
            .find("main-window:seek-preparation:status")
            .and_then(|node| node.value.as_deref()),
        Some("Seek completed, but room convergence degraded.")
    );
    assert!(
        convergence_degraded
            .find("main-window:seek-preparation:keep-waiting")
            .is_none()
    );

    assert!(
        state.apply(GuiShellAction::ApplyGuiSeekPreparationRuntimeSnapshot(
            GuiSeekPreparationRuntimeSnapshot {
                preparation: None,
                degraded_reason: Some(GuiSeekPreparationDegradedReason::TimedOut),
            },
        ))
    );
    let degraded = state.main_window_widget_tree();
    assert_eq!(
        degraded
            .find("main-window:seek-preparation:status")
            .and_then(|node| node.value.as_deref()),
        Some("Buffer refill timed out.")
    );
    assert!(
        degraded
            .find("main-window:seek-preparation:keep-waiting")
            .is_none()
    );
    assert!(
        degraded
            .find("main-window:seek-preparation:join-nearest")
            .is_none()
    );

    assert!(
        state.apply(GuiShellAction::ApplyGuiSeekPreparationRuntimeSnapshot(
            GuiSeekPreparationRuntimeSnapshot::default(),
        ))
    );
    assert!(
        state
            .main_window_widget_tree()
            .find("main-window:seek-preparation")
            .is_none()
    );
}
