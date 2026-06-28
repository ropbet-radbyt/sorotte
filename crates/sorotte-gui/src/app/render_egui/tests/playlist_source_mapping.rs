use super::*;

#[test]
fn gui_widget_egui_renderer_maps_playlist_source_menu_options_to_shell_actions() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "Episode 1".to_owned(),
        ]))
    );

    let tree = state.main_window_widget_tree();
    let local_source = tree
        .find("main-window:playlist:0:source:local")
        .expect("local source menu option should exist");

    assert_eq!(
        GuiWidgetEguiRenderer::actions_for_button_node(&state, local_source),
        vec![GuiShellAction::SelectMainWindowPlaylistSource {
            index: 0,
            provider_id: GuiMediaSourceProviderId::local(),
        }]
    );
}
