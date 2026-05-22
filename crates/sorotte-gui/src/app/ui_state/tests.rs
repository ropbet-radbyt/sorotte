use super::{
    GuiPersistedUiState, legacy_gui_qsettings_store_path, load_gui_ui_state_from_root,
    persist_gui_ui_state_at_root,
};

use crate::app::testing::support::test_temp_root;
use crate::app::{GuiConfigurationTab, GuiShellView};

#[test]
fn gui_persisted_ui_state_roundtrips_at_root() {
    let root = test_temp_root("persisted-ui-roundtrip");
    let expected = GuiPersistedUiState {
        active_view: Some(GuiShellView::Room),
        configuration_tab: Some(GuiConfigurationTab::PrivacyChat),
        selected_public_server_address: Some("custom.example:9001".to_owned()),
        selected_media_search_directory: Some("D:/Media".to_owned()),
        last_media_dialog_directory: Some("E:/Dialogs".to_owned()),
        last_checked_for_updates: None,
        hide_empty_rooms: false,
        public_servers: vec![("Custom".to_owned(), "custom.example:9001".to_owned())],
        ..Default::default()
    };

    persist_gui_ui_state_at_root(&root, &expected).expect("persisted GUI state should be written");

    let loaded = load_gui_ui_state_from_root(&root)
        .expect("persisted GUI state should be readable")
        .expect("persisted GUI state should not be empty");
    assert_eq!(loaded, expected);
    assert!(legacy_gui_qsettings_store_path(&root, "MainWindow").exists());
    assert!(legacy_gui_qsettings_store_path(&root, "Interface").exists());
    assert!(legacy_gui_qsettings_store_path(&root, "MediaBrowseDialog").exists());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_ui_state_ignores_legacy_main_window_tab_at_root() {
    let root = test_temp_root("persisted-ui-invalid-tabs");
    std::fs::create_dir_all(&root).expect("test root should be writable");
    std::fs::write(
        legacy_gui_qsettings_store_path(&root, "MainWindow"),
        "[MainWindow]\nmainWindowTab = playlist\nconfigurationTab = also-nope\n",
    )
    .expect("legacy main-window store should be writable");

    let loaded =
        load_gui_ui_state_from_root(&root).expect("persisted GUI state should be readable");
    assert_eq!(loaded, None);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gui_persisted_ui_state_maps_legacy_active_views_to_room_and_setup() {
    for (legacy_view, expected_view) in [
        ("main-window", GuiShellView::Room),
        ("configuration", GuiShellView::Setup),
        ("public-servers", GuiShellView::Setup),
        ("media-search", GuiShellView::Setup),
        ("menus-and-dialogs", GuiShellView::Setup),
        ("plugins", GuiShellView::Plugins),
        ("stream-support", GuiShellView::Plugins),
    ] {
        let root = test_temp_root(&format!("persisted-ui-active-view-{legacy_view}"));
        std::fs::create_dir_all(&root).expect("test root should be writable");
        std::fs::write(
            legacy_gui_qsettings_store_path(&root, "MainWindow"),
            format!("[MainWindow]\nactiveView = {legacy_view}\nmainWindowTab = chat\n"),
        )
        .expect("legacy main-window store should be writable");

        let loaded = load_gui_ui_state_from_root(&root)
            .expect("persisted GUI state should be readable")
            .expect("legacy active view should migrate into persisted UI state");
        assert_eq!(loaded.active_view, Some(expected_view));
        assert_eq!(loaded.configuration_tab, None);

        let _ = std::fs::remove_dir_all(&root);
    }
}
