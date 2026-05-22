use super::{
    GuiPersistedUiState, GuiUpdateCheckState, GuiUpdateIndicatorAction, GuiUpdateIndicatorTone,
    legacy_gui_qsettings_store_path, load_gui_ui_state_from_root, persist_gui_ui_state_at_root,
};

use crate::app::remote_services::{
    LegacyUpdateCheckStatus, StagedUpdate, UpdateCandidate, UpdateCandidateSource, UpdateChannel,
    UpdateDownloadState,
};
use crate::app::testing::support::test_temp_root;
use crate::app::{GuiConfigurationTab, GuiShellView};

fn update_candidate() -> UpdateCandidate {
    UpdateCandidate {
        channel: UpdateChannel::Stable,
        version: "0.2.0".to_owned(),
        git_sha: Some("abcdef123456".to_owned()),
        created_at_utc: "2026-05-20T00:00:00Z".to_owned(),
        target: "windows-x86_64".to_owned(),
        package: "sorotte-gui-0.2.0-windows-x86_64.zip".to_owned(),
        sha256: "a".repeat(64),
        download_url: "https://example.invalid/sorotte-gui.zip".to_owned(),
        details_url: Some("https://example.invalid/release".to_owned()),
        source: UpdateCandidateSource::ReleaseAsset,
    }
}

fn staged_update(candidate: UpdateCandidate) -> StagedUpdate {
    StagedUpdate {
        candidate,
        package_path: "C:/Temp/sorotte.zip".to_owned(),
        source_dir: "C:/Temp/sorotte-update".to_owned(),
        updater_path: "C:/Temp/sorotte-update/sorotte-gui-updater.exe".to_owned(),
        target_exe_path: "C:/Program Files/Sorotte/sorotte-gui.exe".to_owned(),
        backup_dir: "C:/Temp/sorotte-backup".to_owned(),
        log_path: "C:/Temp/sorotte-update.log".to_owned(),
        restart: true,
    }
}

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

#[test]
fn gui_update_indicator_model_covers_status_states() {
    let mut state = GuiUpdateCheckState::default();
    assert_eq!(state.indicator_model(None).title, "Update");
    assert_eq!(
        state.update_indicator_activation_action(),
        Some(GuiUpdateIndicatorAction::Check)
    );

    state.status = Some(LegacyUpdateCheckStatus::Checking);
    let model = state.indicator_model(None);
    assert_eq!(model.title, "Checking for updates");
    assert_eq!(model.tone, GuiUpdateIndicatorTone::Progress);
    assert!(!model.enabled);
    assert_eq!(state.update_indicator_activation_action(), None);

    state.status = Some(LegacyUpdateCheckStatus::UpToDate);
    state.self_update_supported = true;
    state.last_checked_for_updates = Some("2026-05-22 01:02:03.004".to_owned());
    let model = state.indicator_model(Some("en"));
    assert_eq!(model.title, "Up to date");
    assert_eq!(model.tone, GuiUpdateIndicatorTone::Success);
    assert!(model.enabled);

    state.status = Some(LegacyUpdateCheckStatus::Failed);
    let model = state.indicator_model(None);
    assert_eq!(model.title, "Update failed");
    assert_eq!(model.tone, GuiUpdateIndicatorTone::Error);
    assert_eq!(
        state.update_indicator_activation_action(),
        Some(GuiUpdateIndicatorAction::Check)
    );
}

#[test]
fn gui_update_indicator_model_covers_install_states() {
    let candidate = update_candidate();
    let mut state = GuiUpdateCheckState {
        status: Some(LegacyUpdateCheckStatus::UpdateAvailable),
        candidate: Some(candidate.clone()),
        self_update_supported: true,
        ..GuiUpdateCheckState::default()
    };

    let model = state.indicator_model(None);
    assert_eq!(model.title, "Update available");
    assert_eq!(model.tone, GuiUpdateIndicatorTone::Info);
    assert_eq!(
        state.update_indicator_activation_action(),
        Some(GuiUpdateIndicatorAction::InstallAvailable)
    );

    state.download_state = UpdateDownloadState::Downloading;
    let model = state.indicator_model(None);
    assert_eq!(model.title, "Downloading update");
    assert_eq!(model.tone, GuiUpdateIndicatorTone::Progress);
    assert_eq!(state.update_indicator_activation_action(), None);

    state.download_state = UpdateDownloadState::Staged;
    state.staged_update = Some(staged_update(candidate));
    let model = state.indicator_model(None);
    assert_eq!(model.title, "Ready to install");
    assert_eq!(
        state.update_indicator_activation_action(),
        Some(GuiUpdateIndicatorAction::ApplyStaged)
    );

    state.message = Some("Launching update helper...".to_owned());
    let model = state.indicator_model(None);
    assert_eq!(model.title, "Installing update");
    assert!(!model.enabled);
}
