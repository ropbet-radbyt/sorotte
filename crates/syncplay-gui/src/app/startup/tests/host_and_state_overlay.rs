use super::*;

#[test]
fn run_gui_host_passes_shell_state_through_host_boundary() {
    #[derive(Default)]
    struct RecordingHost {
        saw_configuration_view: bool,
    }

    impl GuiAppHost for RecordingHost {
        type Output = String;

        fn render(&mut self, state: SyncplayGuiShellAppState) -> Self::Output {
            self.saw_configuration_view = state.active_view == GuiShellView::Setup;
            format!("host:{}", state.active_view.label())
        }
    }

    let mut host = RecordingHost::default();
    let rendered = run_gui_host(&StoredClientSettingsMvp::default(), &mut host);

    assert_eq!(rendered, "host:setup");
    assert!(host.saw_configuration_view);
}

#[test]
fn run_gui_host_with_startup_actions_and_gui_state_restores_non_ini_state() {
    #[derive(Default)]
    struct RecordingHost;

    impl GuiAppHost for RecordingHost {
        type Output = SyncplayGuiShellAppState;

        fn render(&mut self, state: SyncplayGuiShellAppState) -> Self::Output {
            state
        }
    }

    let settings = StoredClientSettingsMvp {
        media_search_directories: Some(vec!["C:/Media".to_owned()]),
        ..StoredClientSettingsMvp::default()
    };
    let persisted_ui_state = GuiPersistedUiState {
        active_view: Some(GuiShellView::Setup),
        selected_public_server_address: Some("custom.example:9001".to_owned()),
        selected_media_search_directory: Some("C:/Media".to_owned()),
        last_media_dialog_directory: Some("D:/Dialogs".to_owned()),
        last_checked_for_updates: None,
        hide_empty_rooms: false,
        public_servers: vec![("Custom".to_owned(), "custom.example:9001".to_owned())],
        ..Default::default()
    };

    let mut host = RecordingHost;
    let state = super::super::run_gui_host_with_startup_actions_and_gui_state(
        &settings,
        Some(&persisted_ui_state),
        Vec::new(),
        &mut host,
    );

    assert_eq!(state.active_view, GuiShellView::Setup);
    assert_eq!(
        state.last_media_dialog_directory.as_deref(),
        Some("D:/Dialogs")
    );
    assert_eq!(
        state
            .public_servers
            .servers
            .iter()
            .map(|row| (row.label.clone(), row.address.clone()))
            .collect::<Vec<_>>(),
        persisted_ui_state.public_servers
    );
    assert_eq!(state.selected_public_server_index(), Some(0));
    assert_eq!(state.selection.selected_media_search_directory, Some(0));
    assert_eq!(
        state.saved_configuration.public_servers,
        Some(vec![(
            "Custom".to_owned(),
            "custom.example:9001".to_owned()
        )])
    );
    assert_eq!(
        state.configuration.to_stored_settings().host.as_deref(),
        Some("custom.example")
    );
    assert_eq!(state.configuration.to_stored_settings().port, Some(9001));
}

#[test]
fn run_gui_host_with_startup_actions_and_gui_state_prefers_gui_public_servers_over_ini_rows() {
    #[derive(Default)]
    struct RecordingHost;

    impl GuiAppHost for RecordingHost {
        type Output = SyncplayGuiShellAppState;

        fn render(&mut self, state: SyncplayGuiShellAppState) -> Self::Output {
            state
        }
    }

    let settings = StoredClientSettingsMvp {
        host: Some("saved.example".to_owned()),
        port: Some(8999),
        public_servers: Some(vec![("Saved".to_owned(), "saved.example:8999".to_owned())]),
        ..StoredClientSettingsMvp::default()
    };
    let persisted_ui_state = GuiPersistedUiState {
        active_view: Some(GuiShellView::Setup),
        selected_public_server_address: Some("custom.example:9001".to_owned()),
        selected_media_search_directory: None,
        last_media_dialog_directory: None,
        last_checked_for_updates: None,
        hide_empty_rooms: false,
        public_servers: vec![("Custom".to_owned(), "custom.example:9001".to_owned())],
        ..Default::default()
    };

    let mut host = RecordingHost;
    let state = super::super::run_gui_host_with_startup_actions_and_gui_state(
        &settings,
        Some(&persisted_ui_state),
        Vec::new(),
        &mut host,
    );

    assert_eq!(state.active_view, GuiShellView::Setup);
    assert_eq!(
        state
            .public_servers
            .servers
            .iter()
            .map(|row| (row.label.clone(), row.address.clone()))
            .collect::<Vec<_>>(),
        vec![("Custom".to_owned(), "custom.example:9001".to_owned())]
    );
    assert_eq!(
        state.saved_configuration.public_servers,
        Some(vec![(
            "Custom".to_owned(),
            "custom.example:9001".to_owned()
        )])
    );
    assert_eq!(
        state.configuration.to_stored_settings().host.as_deref(),
        Some("custom.example")
    );
    assert_eq!(state.configuration.to_stored_settings().port, Some(9001));
}
