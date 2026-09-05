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
fn narrow_playlist_rows_paint_a_readable_title_beside_compact_actions() {
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    let mut playlist = GuiWidgetNode::leaf(
        "main-window:playlist",
        "Playlist",
        GuiWidgetKind::List,
        None,
        true,
        false,
    );
    let mut row = GuiWidgetNode::leaf(
        "main-window:playlist:0",
        "Episode 001 with a long title.mkv",
        GuiWidgetKind::ListItem,
        None,
        true,
        false,
    );
    row.children = ["source", "remove"]
        .into_iter()
        .map(|action| {
            GuiWidgetNode::leaf(
                format!("main-window:playlist:0:{action}"),
                action,
                GuiWidgetKind::Button,
                None,
                true,
                false,
            )
        })
        .collect();
    playlist.children.push(row);
    for width in [207.0, 220.0] {
        let context = egui::Context::default();
        let mut renderer = GuiWidgetEguiRenderer::default();
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(width);
            ui.set_max_width(width);
            renderer.render_playlist_list(ui, &playlist, &state);
        });
        assert!(
            output.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Text(text)
                if text.galley.job.text.starts_with("Ep"))
            }),
            "the filename must retain visible letters at {width} points"
        );
    }
}

#[test]
fn playlist_keyboard_owner_tracks_selected_row_in_the_accessibility_tree() {
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    let context = egui::Context::default();
    context.enable_accesskit();
    let mut renderer = GuiWidgetEguiRenderer::default();
    let mut playlist = GuiWidgetNode::leaf(
        "main-window:playlist",
        "Playlist",
        GuiWidgetKind::List,
        None,
        true,
        false,
    );
    playlist.children = ["first.mkv", "last-\u{754c}.mkv"]
        .into_iter()
        .enumerate()
        .map(|(index, label)| {
            GuiWidgetNode::leaf(
                format!("main-window:playlist:{index}"),
                label,
                GuiWidgetKind::ListItem,
                None,
                true,
                false,
            )
        })
        .collect();
    for (selection, expected) in [
        (Some(0), "Playlist: first.mkv"),
        (Some(1), "Playlist: last-\u{754c}.mkv"),
        (None, "Playlist"),
    ] {
        for (index, row) in playlist.children.iter_mut().enumerate() {
            row.selected = selection == Some(index);
        }
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            renderer.render_playlist_list(ui, &playlist, &state);
        });
        let update = output.platform_output.accesskit_update.unwrap();
        let owner = update
            .nodes
            .iter()
            .find_map(|(_, node)| {
                (node.author_id() == Some("main-window:playlist:keyboard-focus")).then_some(node)
            })
            .expect("the real playlist keyboard owner must be accessible");
        assert_eq!(owner.label(), Some(expected));
    }
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
