use super::*;
use crate::app::runtime_stack::test_support::GuiSessionDeliveryTestExt;
use crate::app::testing::support::runtime_state_for_shell;

#[test]
fn gui_client_core_chat_session_runtime_adapter_bridges_chat_protocol_and_notifications() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        chat_output_enabled: Some(true),
        ..StoredClientSettings::default()
    });
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let startup_lines = adapter
        .deliver_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);
    assert!(startup_lines[0].contains("\"Hello\""));
    assert!(startup_lines[0].contains("\"alice\""));
    assert!(startup_lines[0].contains("\"room1\""));
    assert!(startup_lines[0].contains("\"chat\":true"));
    assert_eq!(
        adapter.runtime.session().connection_phase(),
        &ConnectionPhase::AwaitingHello
    );
    assert!(
        GuiSessionRuntimeAdapter::send_chat_message(&mut adapter, "hello room".to_owned(),)
            .is_err(),
        "chat should stay blocked until the adapter receives a server Hello"
    );

    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("inbound server hello should apply");
    assert!(
        GuiSessionRuntimeAdapter::send_chat_message(&mut adapter, "hello room".to_owned(),).is_ok(),
        "chat-capable client-core adapter should queue outbound chat"
    );
    let outbound_lines = adapter
        .deliver_outbound_protocol_lines()
        .expect("queued outbound protocol lines should encode");
    assert_eq!(outbound_lines.len(), 1);
    assert!(outbound_lines[0].contains("\"Chat\""));
    assert!(outbound_lines[0].contains("hello room"));
    for action in
        GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &runtime_state_for_shell(&state))
    {
        assert!(state.apply(action));
    }

    adapter
        .apply_message_json(r#"{"Chat":{"username":"alice","message":"hello room"}}"#)
        .expect("inbound server echo should apply");
    assert_eq!(
        GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &runtime_state_for_shell(&state)),
        vec![GuiShellAction::PushChatMessage {
            sender: "alice".to_owned(),
            message: "hello room".to_owned(),
        }]
    );
    assert!(
        GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &runtime_state_for_shell(&state))
            .is_empty()
    );
}

#[test]
fn gui_session_treats_chat_disabled_as_active_after_hello() {
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");
    let _ = adapter
        .deliver_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":false}}}"#,
        )
        .expect("inbound server hello should apply");

    assert!(matches!(
        adapter.runtime.session().connection_phase(),
        ConnectionPhase::Active(capabilities) if !capabilities.chat
    ));
    assert_eq!(
        GuiSessionRuntimeAdapter::send_chat_message(&mut adapter, "blocked".to_owned())
            .expect_err("disabled chat should reject the message"),
        "Client-core session runtime cannot send chat because the server disabled chat."
    );
    GuiSessionRuntimeAdapter::set_room(&mut adapter, "room2".to_owned())
        .expect("an active session may change rooms even when chat is disabled");
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_projects_session_state_into_main_window_snapshot() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(false),
        ..StoredClientSettings::default()
    });
    let mut playback_ready_snapshot =
        MainWindowRuntimeSnapshot::from_shell_state(&state.main_window);
    playback_ready_snapshot.can_toggle_pause = true;
    playback_ready_snapshot.can_seek = true;
    assert!(state.apply(GuiShellAction::ApplyMainWindowRuntimeSnapshot(
        playback_ready_snapshot
    )));
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");
    sync_adapter_to_saved_session_settings(&mut adapter, &state);

    let startup_lines = adapter
        .deliver_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("inbound server hello should apply");
    for action in
        GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &runtime_state_for_shell(&state))
    {
        assert!(state.apply(action));
    }

    adapter
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
        )
        .expect("playlist-change set message should apply");
    adapter
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
        .expect("playlist-index set message should apply");
    adapter
        .apply_message_json(
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"alice"}}}"#,
        )
        .expect("playstate message should apply");
    let actions =
        GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &runtime_state_for_shell(&state));
    assert_eq!(actions.len(), 3);
    let GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot) = &actions[0] else {
        panic!("session state changes should become a main-window runtime snapshot");
    };
    assert_eq!(snapshot.room_name, "room1");
    assert!(snapshot.shared_playlist_enabled);
    assert_eq!(
        snapshot.users,
        vec![browser_runtime_user("alice", "room1", true, false, false)]
    );
    assert_eq!(
        snapshot.playlist,
        vec!["episode1.mkv".to_owned(), "episode2.mkv".to_owned()]
    );
    assert!(snapshot.playback_paused);
    let GuiShellAction::ApplySharedPlaylistSelection(selection) = &actions[1] else {
        panic!("session playlist index should become a playlist selection action");
    };
    assert_eq!(*selection, Some(1));
    let GuiShellAction::ApplyMenuDialogRuntimeSnapshot(menu_snapshot) = &actions[2] else {
        panic!("session playlist availability should become a menu runtime snapshot");
    };
    assert_eq!(
        menu_snapshot.action_overrides,
        vec![MenuActionRuntimeOverride {
            id: MenuActionId::SharedPlaylist,
            enabled: true,
        }]
    );
    for action in actions {
        assert!(state.apply(action));
    }
    assert!(state.main_window.shared_playlist_enabled);
    assert_eq!(state.selection.selected_main_window_playlist, Some(1));
    assert!(
        state
            .menus
            .action(MenuActionId::SharedPlaylist)
            .is_some_and(|action| action.enabled)
    );
    assert!(
        GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &runtime_state_for_shell(&state))
            .is_empty()
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_preserves_local_playlist_selection_when_session_playlist_index_changes()
 {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettings::default()
    });
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("inbound server hello should apply");
    for action in
        GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &runtime_state_for_shell(&state))
    {
        assert!(state.apply(action));
    }

    adapter
        .apply_message_json(
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv","episode3.mkv"],"user":"alice"}}}"#,
        )
        .expect("playlist-change set message should apply");
    adapter
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
        .expect("initial playlist-index set message should apply");
    for action in
        GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &runtime_state_for_shell(&state))
    {
        assert!(state.apply(action));
    }
    assert_eq!(state.selection.selected_main_window_playlist, Some(0));

    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(2)));
    assert_eq!(state.selection.selected_main_window_playlist, Some(2));

    adapter
        .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
        .expect("follow-up playlist-index set message should apply");

    let actions =
        GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &runtime_state_for_shell(&state));
    assert!(
        actions.iter().all(|action| !matches!(
            action,
            GuiShellAction::ApplyGuiInteractionRuntimeSnapshot(_)
        )),
        "session playlist-index changes should not overwrite an existing local playlist selection"
    );
    for action in actions {
        assert!(state.apply(action));
    }
    assert_eq!(state.selection.selected_main_window_playlist, Some(2));
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_surfaces_user_changes_as_system_chat_events() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        shared_playlist_enabled: Some(false),
        ..StoredClientSettings::default()
    });
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");
    sync_adapter_to_saved_session_settings(&mut adapter, &state);

    let startup_lines = adapter
        .deliver_outbound_protocol_lines()
        .expect("startup protocol lines should encode");
    assert_eq!(startup_lines.len(), 1);

    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("inbound server hello should apply");
    for action in
        GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &runtime_state_for_shell(&state))
    {
        assert!(state.apply(action));
    }

    adapter
        .apply_message_json(
            r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"controller":true}}}}"#,
        )
        .expect("user join message should apply");
    let actions =
        GuiSessionRuntimeAdapter::drain_gui_actions(&mut adapter, &runtime_state_for_shell(&state));
    assert_eq!(actions.len(), 2);
    let GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot) = &actions[0] else {
        panic!("user changes should still refresh the main-window runtime snapshot");
    };
    assert_eq!(
        snapshot.users,
        vec![
            browser_runtime_user("alice", "room1", true, false, false),
            browser_runtime_user("bob", "room1", false, false, true),
        ]
    );
    assert_eq!(
        actions[1],
        GuiShellAction::AnnounceSystemChatEvent("bob has joined the room: 'room1'".to_owned())
    );
}
