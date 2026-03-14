use super::{
    GuiAppHost, GuiNativeApp, GuiNativeRuntimeBridge, GuiPreviewRuntimeBridge, GuiShellAction,
    GuiTextPreviewHost, GuiTransientNotificationLevel, SyncplayGuiShellAppState,
};

use crate::app::render_io::{GuiDroppedFilesRequest, GuiDroppedFilesTarget};
use syncplay_client_app::app_boundary::state::StoredClientSettingsMvp;

#[test]
fn gui_text_preview_host_uses_summary_and_widget_tree_output() {
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    let mut host = GuiTextPreviewHost;
    let rendered = host.render(state);

    assert!(rendered.contains("[Shell App State]"));
    assert!(rendered.contains("[Widget Tree]"));
    assert!(rendered.contains("- Syncplay GUI [panel] id=shell-root"));
}

#[test]
fn gui_native_app_and_preview_runtime_map_seek_prompt_input_to_runtime_actions() {
    assert_eq!(
        GuiNativeApp::parse_seek_offset_seconds(" 12.5 "),
        Some(12.5)
    );
    assert_eq!(GuiNativeApp::parse_seek_offset_seconds("NaN"), None);
    assert_eq!(GuiNativeApp::parse_seek_offset_seconds(""), None);

    let mut runtime = GuiPreviewRuntimeBridge;
    assert_eq!(
        runtime.actions_for_seek_offset(12.5),
        vec![
            GuiShellAction::PushTransientNotification {
                level: GuiTransientNotificationLevel::Info,
                message: "Seek requested: 12.5 seconds.".to_owned(),
            },
            GuiShellAction::AnnounceSystemChatEvent("Seek requested: 12.5 seconds.".to_owned(),),
        ]
    );
}

#[test]
fn gui_native_app_reads_drag_and_drop_test_override_from_lookup() {
    assert_eq!(
        GuiNativeApp::test_drop_request_from_lookup(&|name| match name {
            "SYNCPLAY_GUI_TEST_DROP_FILE_PATHS" => {
                Some("  C:/Drops/episode1.mkv | D:/Alt/episode2.mp4 ".to_owned())
            }
            "SYNCPLAY_GUI_TEST_DROP_TARGET" => Some(" playlist ".to_owned()),
            _ => None,
        })
        .expect("drop override should parse"),
        Some(GuiDroppedFilesRequest {
            target: GuiDroppedFilesTarget::Playlist,
            paths: vec![
                "C:/Drops/episode1.mkv".to_owned(),
                "D:/Alt/episode2.mp4".to_owned(),
            ],
        })
    );
    assert_eq!(
        GuiNativeApp::test_drop_request_from_lookup(&|_name| None)
            .expect("missing drop override should not fail"),
        None
    );
}
