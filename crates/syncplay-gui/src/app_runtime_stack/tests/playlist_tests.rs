use super::*;

#[test]
fn gui_client_core_chat_session_runtime_adapter_dispatches_shared_playlist_operations() {
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);
    assert!(
        !GuiSessionRuntimeAdapter::playlist_control_available(&adapter),
        "playlist controls should remain unavailable before server hello"
    );

    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#,
        )
        .expect("inbound server hello should apply");
    assert!(
        GuiSessionRuntimeAdapter::playlist_control_available(&adapter),
        "playlist controls should become available after a successful room hello"
    );

    GuiSessionRuntimeAdapter::queue_playlist_entry(&mut adapter, "episode1.mkv".to_owned(), true)
        .expect("queueing the first playlist entry should dispatch");
    let first_queue_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("first queue lines should encode");
    assert_eq!(first_queue_lines.len(), 2);
    assert!(first_queue_lines[0].contains("\"playlistChange\""));
    assert!(first_queue_lines[0].contains("episode1.mkv"));
    assert!(first_queue_lines[1].contains("\"playlistIndex\""));
    assert!(first_queue_lines[1].contains("\"index\":0"));
    for line in &first_queue_lines {
        adapter
            .apply_message_json(line)
            .expect("first queue echo should apply");
    }

    GuiSessionRuntimeAdapter::queue_playlist_entry(&mut adapter, "episode2.mkv".to_owned(), true)
        .expect("queueing the second playlist entry should dispatch");
    let second_queue_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("second queue lines should encode");
    assert_eq!(second_queue_lines.len(), 2);
    assert!(second_queue_lines[0].contains("episode1.mkv"));
    assert!(second_queue_lines[0].contains("episode2.mkv"));
    assert!(second_queue_lines[1].contains("\"index\":1"));
    for line in &second_queue_lines {
        adapter
            .apply_message_json(line)
            .expect("second queue echo should apply");
    }

    GuiSessionRuntimeAdapter::set_playlist_index(&mut adapter, 0)
        .expect("playlist selection should dispatch");
    let selection_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("selection lines should encode");
    assert_eq!(selection_lines.len(), 2);
    assert!(selection_lines[0].contains("\"playlistIndex\""));
    assert!(selection_lines[0].contains("\"index\":0"));
    assert!(selection_lines[1].contains("\"State\""));
    assert!(selection_lines[1].contains("\"position\":0.0"));
    assert!(selection_lines[1].contains("\"paused\":true"));
    for line in &selection_lines {
        adapter
            .apply_message_json(line)
            .expect("selection echo should apply");
    }

    GuiSessionRuntimeAdapter::advance_playlist_index(&mut adapter)
        .expect("playlist advancement should dispatch");
    let advance_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("advance lines should encode");
    assert!(
        advance_lines
            .iter()
            .any(|line| line.contains("\"playlistIndex\"") && line.contains("\"index\":1")),
        "playlist advancement should emit a playlistIndex update"
    );
    assert!(
        advance_lines.iter().any(|line| {
            line.contains("\"State\"")
                && line.contains("\"position\":0.0")
                && line.contains("\"paused\":true")
        }),
        "playlist advancement should emit the immediate paused-at-zero reset state"
    );
    for line in &advance_lines {
        adapter
            .apply_message_json(line)
            .expect("advance echo should apply");
    }

    GuiSessionRuntimeAdapter::delete_playlist_index(&mut adapter, 0)
        .expect("playlist removal should dispatch");
    let delete_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("delete lines should encode");
    assert!(
        delete_lines
            .iter()
            .any(|line| line.contains("\"playlistChange\"") && line.contains("episode2.mkv")),
        "playlist deletion should emit the updated playlist"
    );
    assert!(
        delete_lines
            .iter()
            .any(|line| line.contains("\"playlistIndex\"") && line.contains("\"index\":0")),
        "playlist deletion should emit the updated playlist index"
    );
    for line in &delete_lines {
        adapter
            .apply_message_json(line)
            .expect("delete echo should apply");
    }

    GuiSessionRuntimeAdapter::replace_playlist(
        &mut adapter,
        vec!["episode3.mkv".to_owned(), "episode2.mkv".to_owned()],
        Some(1),
    )
    .expect("playlist reorder should dispatch");
    let replace_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("replace lines should encode");
    assert!(
        replace_lines.iter().any(|line| {
            line.contains("\"playlistChange\"")
                && line.contains("episode3.mkv")
                && line.contains("episode2.mkv")
        }),
        "playlist replacement should emit the reordered playlist"
    );
    assert!(
        replace_lines
            .iter()
            .any(|line| line.contains("\"playlistIndex\"") && line.contains("\"index\":1")),
        "playlist replacement should emit the selected playlist index"
    );
    for line in &replace_lines {
        adapter
            .apply_message_json(line)
            .expect("replace echo should apply");
    }

    let playlist = adapter
        .runtime
        .session()
        .current_room_playlist()
        .expect("playlist should exist after the echoed operations");
    assert_eq!(playlist.files, vec!["episode3.mkv", "episode2.mkv"]);
    assert_eq!(playlist.index, Some(1));
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_clears_stale_shared_playlist_when_session_has_none()
{
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut stale_snapshot = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
    stale_snapshot.shared_playlist_enabled = true;
    stale_snapshot.playlist = vec!["episode1.mkv".to_owned(), "episode2.mkv".to_owned()];
    stale_snapshot.can_manage_playlist = true;
    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        stale_snapshot
    )));
    let mut stale_interaction = GuiInteractionRuntimeSnapshot::from_shell_state(&state);
    stale_interaction.selection.selected_main_window_playlist = Some(1);
    assert!(
        state.apply(GuiShellAction::ApplyGuiInteractionRuntimeSnapshot(
            stale_interaction
        ))
    );
    assert!(state.apply(GuiShellAction::ApplyMenuDialogRuntimeSnapshot(
        MenuDialogRuntimeSnapshot {
            action_overrides: vec![
                MenuActionRuntimeOverride {
                    section_title: "Window",
                    action_label: "Show Playlist",
                    enabled: true,
                },
                MenuActionRuntimeOverride {
                    section_title: "Playback",
                    action_label: "Playlist Actions",
                    enabled: true,
                },
            ],
            tls_prompt_expected: state.menus.tls_prompt_expected,
            update_notice_expected: state.menus.update_notice_expected,
            about_dialog_available: state.menus.about_dialog_available,
        },
    )));

    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");
    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("inbound server hello should apply");

    let actions = GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    assert_eq!(actions.len(), 3);
    let GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot) = &actions[0] else {
        panic!("stale shared-playlist state should be corrected through a main-window snapshot");
    };
    assert!(!snapshot.shared_playlist_enabled);
    assert!(snapshot.playlist.is_empty());
    assert!(!snapshot.can_manage_playlist);
    let GuiShellAction::ApplyGuiInteractionRuntimeSnapshot(interaction_snapshot) = &actions[1]
    else {
        panic!(
            "stale shared-playlist selection should be corrected through an interaction snapshot"
        );
    };
    assert_eq!(
        interaction_snapshot.selection.selected_main_window_playlist,
        None
    );
    let GuiShellAction::ApplyMenuDialogRuntimeSnapshot(menu_snapshot) = &actions[2] else {
        panic!("stale shared-playlist menu state should be corrected through a menu snapshot");
    };
    assert_eq!(
        menu_snapshot.action_overrides,
        vec![
            MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Playlist",
                enabled: false,
            },
            MenuActionRuntimeOverride {
                section_title: "Playback",
                action_label: "Playlist Actions",
                enabled: false,
            },
            MenuActionRuntimeOverride {
                section_title: "Advanced",
                action_label: "Create Controlled Room",
                enabled: true,
            },
        ]
    );
    for action in actions {
        assert!(state.apply(action));
    }
    assert!(!state.main_window.shared_playlist_enabled);
    assert!(state.main_window.playlist.is_empty());
    assert!(!state.main_window.playback.can_manage_playlist);
    assert_eq!(state.selection.selected_main_window_playlist, None);
    assert!(
        state
            .menus
            .sections
            .iter()
            .find(|section| section.title == "Window")
            .and_then(|section| {
                section
                    .actions
                    .iter()
                    .find(|action| action.label == "Show Playlist")
            })
            .is_some_and(|action| !action.enabled)
    );
    assert!(
        state
            .menus
            .sections
            .iter()
            .find(|section| section.title == "Playback")
            .and_then(|section| {
                section
                    .actions
                    .iter()
                    .find(|action| action.label == "Playlist Actions")
            })
            .is_some_and(|action| !action.enabled)
    );
    assert!(GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state).is_empty());
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_projects_local_playlist_replace_before_server_echo()
{
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");
    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("inbound server hello should apply");
    for action in GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state) {
        assert!(state.apply(action));
    }

    GuiSessionRuntimeAdapter::replace_playlist(
        &mut adapter,
        vec!["episode1.mkv".to_owned()],
        Some(0),
    )
    .expect("playlist replace should dispatch");

    let actions = GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    assert_eq!(actions.len(), 3);
    let GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot) = &actions[0] else {
        panic!("local playlist replace should project a main-window runtime snapshot");
    };
    assert!(snapshot.shared_playlist_enabled);
    assert_eq!(snapshot.playlist, vec!["episode1.mkv".to_owned()]);
    let GuiShellAction::ApplyGuiInteractionRuntimeSnapshot(interaction) = &actions[1] else {
        panic!("local playlist replace should project a playlist selection");
    };
    assert_eq!(interaction.selection.selected_main_window_playlist, Some(0));
    let GuiShellAction::ApplyMenuDialogRuntimeSnapshot(menu_snapshot) = &actions[2] else {
        panic!("local playlist replace should surface playlist menu availability");
    };
    assert_eq!(
        menu_snapshot.action_overrides,
        vec![
            MenuActionRuntimeOverride {
                section_title: "Window",
                action_label: "Show Playlist",
                enabled: true,
            },
            MenuActionRuntimeOverride {
                section_title: "Playback",
                action_label: "Playlist Actions",
                enabled: true,
            },
        ]
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_clears_stale_playback_pause_when_session_has_no_playstate()
 {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut stale_snapshot = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
    stale_snapshot.playback_paused = true;
    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        stale_snapshot
    )));
    assert!(state.main_window.playback_paused);

    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");
    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("inbound server hello should apply");

    let mut expected_snapshot = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
    expected_snapshot.playback_paused = false;
    expected_snapshot.can_set_others_ready = true;
    expected_snapshot.room_control_status =
        "Not required: current room is not controlled.".to_owned();
    let actions = GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    assert_eq!(
        actions,
        vec![
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(expected_snapshot),
            GuiShellAction::ApplyMenuDialogRuntimeSnapshot(MenuDialogRuntimeSnapshot {
                action_overrides: vec![MenuActionRuntimeOverride {
                    section_title: "Advanced",
                    action_label: "Create Controlled Room",
                    enabled: true,
                }],
                tls_prompt_expected: state.menus.tls_prompt_expected,
                update_notice_expected: state.menus.update_notice_expected,
                about_dialog_available: state.menus.about_dialog_available,
            }),
        ]
    );
    for action in actions {
        assert!(state.apply(action));
    }
    assert!(!state.main_window.playback_paused);
    assert!(GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state).is_empty());
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_clears_stale_autoplay_state_when_session_has_no_override()
 {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    let mut stale_snapshot = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
    stale_snapshot.autoplay_active = true;
    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        stale_snapshot
    )));
    assert!(state.main_window.autoplay_active);

    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");
    let startup_lines = adapter
        .flush_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("inbound server hello should apply");

    let mut expected_snapshot = MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
    expected_snapshot.autoplay_active = false;
    expected_snapshot.can_set_others_ready = true;
    expected_snapshot.room_control_status =
        "Not required: current room is not controlled.".to_owned();
    let actions = GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state);
    assert_eq!(
        actions,
        vec![
            GuiShellAction::ApplyMainWindowRuntimeSnapshot(expected_snapshot),
            GuiShellAction::ApplyMenuDialogRuntimeSnapshot(MenuDialogRuntimeSnapshot {
                action_overrides: vec![MenuActionRuntimeOverride {
                    section_title: "Advanced",
                    action_label: "Create Controlled Room",
                    enabled: true,
                }],
                tls_prompt_expected: state.menus.tls_prompt_expected,
                update_notice_expected: state.menus.update_notice_expected,
                about_dialog_available: state.menus.about_dialog_available,
            }),
        ]
    );
    for action in actions {
        assert!(state.apply(action));
    }
    assert!(!state.main_window.autoplay_active);
    assert!(GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &state).is_empty());
}
