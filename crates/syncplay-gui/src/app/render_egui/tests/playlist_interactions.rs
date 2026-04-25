use super::*;

#[test]
fn gui_widget_egui_renderer_maps_playlist_drag_targets_to_row_moves() {
    assert_eq!(
        GuiWidgetEguiRenderer::playlist_row_move_action(2, 0, 3),
        Some(GuiShellAction::MoveMainWindowPlaylistRow {
            from_index: 2,
            to_index: 0,
        })
    );
    assert_eq!(
        GuiWidgetEguiRenderer::playlist_row_move_action(0, 3, 3),
        Some(GuiShellAction::MoveMainWindowPlaylistRow {
            from_index: 0,
            to_index: 2,
        })
    );
    assert_eq!(
        GuiWidgetEguiRenderer::playlist_row_move_action(1, 1, 3),
        None
    );
    assert_eq!(
        GuiWidgetEguiRenderer::playlist_row_move_action(1, 2, 3),
        None
    );
    assert_eq!(
        GuiWidgetEguiRenderer::playlist_row_move_action(4, 0, 3),
        None
    );
    assert_eq!(
        GuiWidgetEguiRenderer::playlist_row_move_action(1, 4, 3),
        None
    );
}

#[test]
fn gui_widget_egui_renderer_uses_click_and_drag_for_reorderable_playlist_rows() {
    let reorderable = GuiWidgetEguiRenderer::playlist_row_sense(true);
    assert!(reorderable.senses_click());
    assert!(reorderable.senses_drag());

    let static_row = GuiWidgetEguiRenderer::playlist_row_sense(false);
    assert!(static_row.senses_click());
    assert!(!static_row.senses_drag());
}

#[test]
fn gui_widget_egui_renderer_uses_focusable_noninteractive_playlist_keyboard_target() {
    let sense = GuiWidgetEguiRenderer::playlist_focus_sense();
    assert!(!sense.senses_click());
    assert!(!sense.senses_drag());
    assert!(sense.is_focusable());
}

#[test]
fn gui_widget_egui_renderer_maps_playlist_pointer_actions_to_local_select_and_double_click_activate()
 {
    assert_eq!(
        GuiWidgetEguiRenderer::playlist_row_pointer_actions(2, true, false),
        vec![GuiShellAction::SelectMainWindowPlaylist(2)]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::playlist_row_pointer_actions(2, false, true),
        vec![
            GuiShellAction::SelectMainWindowPlaylist(2),
            GuiShellAction::ActivateMainWindowPlaylist(2),
        ]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::playlist_row_pointer_actions(2, false, false),
        Vec::<GuiShellAction>::new()
    );
}

#[test]
fn gui_widget_egui_renderer_maps_playlist_row_shortcuts_to_selection_and_delete_actions() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_manage_playlist = true;
    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "Episode 1.mkv".to_owned(),
            "Episode 2.mkv".to_owned(),
        ]))
    );
    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(1)));

    assert_eq!(
        GuiWidgetEguiRenderer::playlist_row_shortcut_actions(&state, 1, true, true, true, false),
        vec![GuiShellAction::ActivateMainWindowPlaylist(1)]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::playlist_row_shortcut_actions(&state, 1, true, true, false, true),
        vec![GuiShellAction::RemoveSelectedMainWindowPlaylist]
    );
}

#[test]
fn gui_widget_egui_renderer_ignores_playlist_row_shortcuts_without_focus_or_delete_permission() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "Episode 1.mkv".to_owned(),
            "Episode 2.mkv".to_owned(),
        ]))
    );
    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(1)));

    assert!(
        GuiWidgetEguiRenderer::playlist_row_shortcut_actions(&state, 1, true, false, true, true)
            .is_empty()
    );
    assert_eq!(
        GuiWidgetEguiRenderer::playlist_row_shortcut_actions(&state, 1, true, true, false, true),
        Vec::<GuiShellAction>::new()
    );
}

#[test]
fn gui_widget_egui_renderer_ignores_playlist_row_shortcuts_for_unselected_rows() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_manage_playlist = true;
    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "Episode 1.mkv".to_owned(),
            "Episode 2.mkv".to_owned(),
        ]))
    );
    assert!(state.apply(GuiShellAction::SelectMainWindowPlaylist(0)));

    assert!(
        GuiWidgetEguiRenderer::playlist_row_shortcut_actions(&state, 1, true, true, true, true)
            .is_empty()
    );
}
