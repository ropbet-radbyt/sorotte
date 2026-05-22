use super::*;

#[test]
fn gui_widget_egui_renderer_exposes_modal_specific_titles_and_actions() {
    assert_eq!(
        GuiWidgetEguiRenderer::modal_window_title(GuiShellModal::TlsCertificatePrompt),
        "TLS Certificate Prompt"
    );
    assert_eq!(
        GuiWidgetEguiRenderer::modal_actions(GuiShellModal::TlsCertificatePrompt),
        vec![
            ("shell:modal:tls:trust", "Trust Certificate"),
            ("shell:modal:tls:reject", "Reject Certificate"),
            ("shell:modal:tls:help", "Open Help"),
        ]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::modal_actions(GuiShellModal::UpdateNotice),
        Vec::<(&'static str, &'static str)>::new()
    );
    assert_eq!(
        GuiWidgetEguiRenderer::modal_actions(GuiShellModal::About),
        vec![
            ("shell:modal:about:help", "Open Help"),
            ("shell:modal:about:update", "Check for Updates"),
        ]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::modal_window_title(GuiShellModal::PlayerSetup),
        "mpv Setup Required"
    );
    assert_eq!(
        GuiWidgetEguiRenderer::modal_actions(GuiShellModal::PlayerSetup),
        vec![
            ("shell:modal:player-setup:autodetect", "Auto-detect mpv"),
            ("shell:modal:player-setup:choose-path", "Choose mpv.exe"),
            ("shell:modal:player-setup:retry", "Retry mpv"),
            ("shell:modal:player-setup:open-settings", "Open Settings"),
        ]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::modal_window_title(GuiShellModal::StreamSupport),
        "Stream Support"
    );
    assert_eq!(
        GuiWidgetEguiRenderer::modal_actions(GuiShellModal::StreamSupport),
        vec![
            ("shell:modal:stream-support:install", "Install Helper"),
            (
                "shell:modal:stream-support:import-downloader",
                "Import yt-dlp"
            ),
            (
                "shell:modal:stream-support:import-js-runtime",
                "Import Deno"
            ),
            (
                "shell:modal:stream-support:open-location",
                "Open Install Location"
            ),
            ("shell:modal:stream-support:recheck", "Recheck Support"),
            ("shell:modal:stream-support:retry", "Retry URL"),
            ("shell:modal:stream-support:open-settings", "Open Plugins"),
        ]
    );
}

#[test]
fn gui_widget_egui_renderer_prefers_selected_media_search_directory_for_native_browse_dialog() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        media_search_directories: Some(vec!["C:/Media".to_owned(), "D:/AltMedia".to_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    assert!(state.apply(GuiShellAction::SelectMediaSearchDirectory(1)));

    assert_eq!(
        GuiWidgetEguiRenderer::media_search_dialog_start_directory(&state),
        Some("D:/AltMedia")
    );
}

#[test]
fn gui_widget_egui_renderer_prefers_last_media_dialog_directory_for_native_browse_dialog() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        media_search_directories: Some(vec!["C:/Media".to_owned(), "D:/AltMedia".to_owned()]),
        ..StoredClientSettingsMvp::default()
    });
    state.last_media_dialog_directory = Some("E:/Dialogs".to_owned());
    assert!(state.apply(GuiShellAction::SelectMediaSearchDirectory(1)));

    assert_eq!(
        GuiWidgetEguiRenderer::media_search_dialog_start_directory(&state),
        Some("E:/Dialogs")
    );
}

#[test]
fn gui_widget_egui_renderer_reads_media_search_browse_override_paths_from_lookup() {
    assert_eq!(
        GuiWidgetEguiRenderer::media_search_browse_override_paths_from_lookup(&|name| match name {
            "SOROTTE_GUI_TEST_MEDIA_SEARCH_BROWSE_PATH" => {
                Some("  C:/Smoke/Media Search  | | D:/Alt/Search  ".to_owned())
            }
            _ => None,
        }),
        Some(vec![
            "C:/Smoke/Media Search".to_owned(),
            "D:/Alt/Search".to_owned(),
        ])
    );
    assert_eq!(
        GuiWidgetEguiRenderer::media_search_browse_override_paths_from_lookup(&|_name| None),
        None
    );
}

#[test]
fn gui_widget_egui_renderer_reads_media_file_pick_override_paths_from_lookup() {
    assert_eq!(
        GuiWidgetEguiRenderer::media_file_pick_override_paths_from_lookup(&|name| match name {
            "SOROTTE_GUI_TEST_OPEN_MEDIA_FILE_PATHS" => {
                Some("  C:/Smoke/episode1.mkv | | D:/Alt/episode2.mp4  ".to_owned())
            }
            _ => None,
        }),
        Some(vec![
            "C:/Smoke/episode1.mkv".to_owned(),
            "D:/Alt/episode2.mp4".to_owned(),
        ])
    );
    assert_eq!(
        GuiWidgetEguiRenderer::media_file_pick_override_paths_from_lookup(&|_name| None),
        None
    );
    assert_eq!(
        GuiWidgetEguiRenderer::media_file_pick_override_paths_from_lookup(&|name| match name {
            "SOROTTE_GUI_TEST_OPEN_MEDIA_FILE_PATHS" => Some("   |  ".to_owned()),
            _ => None,
        }),
        None
    );
}

#[test]
fn gui_widget_egui_renderer_reads_stream_helper_override_paths_from_lookup() {
    assert_eq!(
        GuiWidgetEguiRenderer::stream_helper_downloader_override_path_from_lookup(
            &|name| match name {
                "SOROTTE_GUI_TEST_STREAM_HELPER_DOWNLOADER_PATH" => {
                    Some("  C:/Tools/yt-dlp.exe  ".to_owned())
                }
                _ => None,
            }
        ),
        Some("C:/Tools/yt-dlp.exe".to_owned())
    );
    assert_eq!(
        GuiWidgetEguiRenderer::stream_helper_js_runtime_override_path_from_lookup(
            &|name| match name {
                "SOROTTE_GUI_TEST_STREAM_HELPER_JS_RUNTIME_PATH" => {
                    Some("  C:/Tools/deno.exe  ".to_owned())
                }
                _ => None,
            }
        ),
        Some("C:/Tools/deno.exe".to_owned())
    );
    assert_eq!(
        GuiWidgetEguiRenderer::stream_helper_downloader_override_path_from_lookup(&|_name| None),
        None
    );
    assert_eq!(
        GuiWidgetEguiRenderer::stream_helper_js_runtime_override_path_from_lookup(
            &|name| match name {
                "SOROTTE_GUI_TEST_STREAM_HELPER_JS_RUNTIME_PATH" => Some("   ".to_owned()),
                _ => None,
            }
        ),
        None
    );
}
