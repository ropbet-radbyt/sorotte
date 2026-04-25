use super::*;

#[test]
fn gui_widget_egui_renderer_maps_text_and_checkbox_edits_to_actions() {
    let mut state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    assert!(state.apply(GuiShellAction::SelectConfigurationTab(
        GuiConfigurationTab::Overview,
    )));
    let configuration_tree = state.configuration_widget_tree();
    let host = configuration_tree.find("config:Connection:Host").unwrap();
    let autoplay = configuration_tree
        .find("config:Readiness:Autoplay")
        .unwrap();
    let trusted_domains = configuration_tree
        .find("config:Privacy:Trusted Domains")
        .unwrap();
    let unpause_action = configuration_tree
        .find("config:Readiness:Unpause Action")
        .unwrap();

    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_text_input_node(
            &state,
            host,
            "syncplay.example",
            true,
            false,
        ),
        Some(vec![GuiShellAction::EditConfigurationText {
            section: "Connection",
            label: "Host",
            value: "syncplay.example".to_owned(),
        }])
    );
    assert_eq!(
        GuiWidgetEguiRenderer::action_for_checkbox_node(&state, autoplay, true),
        Some(GuiShellAction::EditConfigurationBool {
            section: "Readiness",
            label: "Autoplay",
            value: true,
        })
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_text_input_node(
            &state,
            trusted_domains,
            "youtube.com; *.example.com/videos",
            true,
            false,
        ),
        Some(vec![GuiShellAction::EditConfigurationText {
            section: "Privacy",
            label: "Trusted Domains",
            value: "youtube.com; *.example.com/videos".to_owned(),
        }])
    );
    assert_eq!(
        GuiWidgetEguiRenderer::configuration_select_options_for_node(&state, unpause_action),
        Some(vec![
            "IfAlreadyReady".to_owned(),
            "IfOthersReady".to_owned(),
            "IfMinUsersReady".to_owned(),
            "Always".to_owned(),
        ])
    );

    let chat_state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    let chat_tree = chat_state.main_window_widget_tree();
    let chat_input = chat_tree.find("main-window:chat-input").unwrap();

    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_text_input_node(
            &chat_state,
            chat_input,
            "Hello world",
            true,
            true,
        ),
        Some(vec![
            GuiShellAction::ApplyGuiDraftRuntimeSnapshot(GuiDraftRuntimeSnapshot {
                outgoing_chat_message: Some("Hello world".to_owned()),
            }),
            GuiShellAction::BeginLocalChatSend("Hello world".to_owned()),
        ])
    );

    let mut room_state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        room: Some("Lounge".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    assert!(room_state.apply(GuiShellAction::ToggleMainWindowRoomChange));
    let room_tree = room_state.main_window_widget_tree();
    let room_input = room_tree.find("main-window:room-input").unwrap();

    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_text_input_node(
            &room_state,
            room_input,
            "  TeamRoom  ",
            true,
            true,
        ),
        Some(vec![
            GuiShellAction::EditConfigurationText {
                section: "Connection",
                label: "Room",
                value: "  TeamRoom  ".to_owned(),
            },
            GuiShellAction::JoinMainWindowRoom("  TeamRoom  ".to_owned()),
        ])
    );
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_text_input_node(
            &room_state,
            room_input,
            "   ",
            true,
            true,
        ),
        Some(vec![
            GuiShellAction::EditConfigurationText {
                section: "Connection",
                label: "Room",
                value: "   ".to_owned(),
            },
            GuiShellAction::JoinMainWindowRoom("   ".to_owned()),
        ])
    );

    assert!(room_tree.find("main-window:user:new").is_none());
    assert!(room_tree.find("main-window:playlist:new").is_none());

    let mut user_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    assert!(user_state.apply(GuiShellAction::AddMainWindowUser("Bob".to_owned())));
    assert!(user_state.apply(GuiShellAction::SelectMainWindowUser(1)));
    assert!(user_state.apply(GuiShellAction::BeginEditSelectedMainWindowUser));
    let user_tree = user_state.main_window_widget_tree();
    assert!(user_tree.find("main-window:user-edit:username").is_none());

    let mut controlled_room_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            room: Some("Lounge".to_owned()),
            ..StoredClientSettingsMvp::default()
        });
    assert!(controlled_room_state.apply(GuiShellAction::BeginCreateControlledRoomEdit));
    let controlled_room_tree = controlled_room_state.main_window_widget_tree();
    let controlled_room_input = controlled_room_tree
        .find("main-window:controlled-room-create:room")
        .unwrap();
    let controlled_room_actions = GuiWidgetEguiRenderer::actions_for_text_input_node(
        &controlled_room_state,
        controlled_room_input,
        "  Studio  ",
        true,
        true,
    )
    .expect("controlled-room input should map edits");
    assert_eq!(controlled_room_actions.len(), 3);
    assert_eq!(
        controlled_room_actions[0],
        GuiShellAction::UpdateCreateControlledRoomEdit("  Studio  ".to_owned())
    );
    assert!(matches!(
        &controlled_room_actions[1],
        GuiShellAction::RequestControllerAuth { room, password }
            if room == "  Studio  "
                && password.len() == 10
                && password.chars().enumerate().all(|(index, c)| match index {
                    2 | 6 => c == '-',
                    0 | 1 => c.is_ascii_uppercase(),
                    _ => c.is_ascii_digit(),
                })
    ));
    assert_eq!(
        controlled_room_actions[2],
        GuiShellAction::CancelCreateControlledRoomEdit
    );

    let mut controller_auth_state =
        SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
            room: Some("+Lounge:ABCDEF123456".to_owned()),
            ..StoredClientSettingsMvp::default()
        });
    assert!(controller_auth_state.apply(GuiShellAction::BeginControllerAuthEdit));
    let controller_auth_tree = controller_auth_state.main_window_widget_tree();
    let controller_auth_input = controller_auth_tree
        .find("main-window:controller-auth:password")
        .unwrap();
    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_text_input_node(
            &controller_auth_state,
            controller_auth_input,
            "ab-123-456",
            true,
            true,
        ),
        Some(vec![
            GuiShellAction::UpdateControllerAuthPasswordEdit("ab-123-456".to_owned()),
            GuiShellAction::RequestControllerAuth {
                room: "+Lounge:ABCDEF123456".to_owned(),
                password: "ab-123-456".to_owned(),
            },
            GuiShellAction::CancelControllerAuthEdit,
        ])
    );
}
