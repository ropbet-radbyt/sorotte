use super::*;

#[test]
fn gui_widget_egui_renderer_prefers_playlist_target_for_hovered_shared_playlist_drops() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_manage_playlist = true;
    let request = GuiWidgetEguiRenderer::dropped_files_request_for_input(
        &state,
        true,
        None,
        None,
        None,
        vec![egui::DroppedFile {
            path: Some(PathBuf::from("C:/Media/episode1.mkv")),
            ..Default::default()
        }],
    )
    .expect("dropped-file request should be derived");

    assert_eq!(
        request,
        GuiDroppedFilesRequest {
            target: GuiDroppedFilesTarget::Playlist,
            paths: vec!["C:/Media/episode1.mkv".to_owned()],
            playlist_insert_slot: Some(state.main_window.playlist.len()),
        }
    );
}

#[test]
fn gui_widget_egui_renderer_defaults_shared_playlist_drops_to_playlist_target() {
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    let request = GuiWidgetEguiRenderer::dropped_files_request_for_input(
        &state,
        false,
        None,
        None,
        None,
        vec![egui::DroppedFile {
            path: Some(PathBuf::from("C:/Media/episode2.mkv")),
            ..Default::default()
        }],
    )
    .expect("dropped-file request should be derived");

    assert_eq!(
        request.target,
        GuiDroppedFilesTarget::Playlist,
        "shared-playlist-enabled media drops should default to playlist ingest"
    );
}

#[test]
fn gui_widget_egui_renderer_defaults_drops_to_playlist_target_when_shared_playlist_is_disabled() {
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    let request = GuiWidgetEguiRenderer::dropped_files_request_for_input(
        &state,
        true,
        None,
        None,
        None,
        vec![egui::DroppedFile {
            path: Some(PathBuf::from("C:/Media/movie.mkv")),
            ..Default::default()
        }],
    )
    .expect("dropped-file request should be derived");

    assert_eq!(
        request.target,
        GuiDroppedFilesTarget::Playlist,
        "media drops should default to playlist ingest even when the legacy shared-playlist toggle is off"
    );
}

#[test]
fn gui_widget_egui_renderer_carries_playlist_insert_slot_for_hovered_playlist_drops() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_manage_playlist = true;
    let request = GuiWidgetEguiRenderer::dropped_files_request_for_input(
        &state,
        true,
        None,
        Some(1),
        None,
        vec![egui::DroppedFile {
            path: Some(PathBuf::from("C:/Media/episode3.mkv")),
            ..Default::default()
        }],
    )
    .expect("dropped-file request should be derived");

    assert_eq!(request.playlist_insert_slot, Some(1));
}

#[test]
fn gui_widget_egui_renderer_defaults_playlist_drops_to_append_slot_when_hover_slot_is_missing() {
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

    let request = GuiWidgetEguiRenderer::dropped_files_request_for_input(
        &state,
        false,
        None,
        None,
        None,
        vec![egui::DroppedFile {
            path: Some(PathBuf::from("C:/Media/episode3.mkv")),
            ..Default::default()
        }],
    )
    .expect("dropped-file request should be derived");

    assert_eq!(
        request,
        GuiDroppedFilesRequest {
            target: GuiDroppedFilesTarget::Playlist,
            paths: vec!["C:/Media/episode3.mkv".to_owned()],
            playlist_insert_slot: Some(2),
        }
    );
}

#[test]
fn gui_widget_egui_renderer_shared_playlist_entries_for_media_paths_use_playlist_labels() {
    assert_eq!(
        GuiWidgetEguiRenderer::shared_playlist_entries_for_media_paths(vec![
            "C:/Media/Episode 1.mkv".to_owned(),
            "https://example.com/live".to_owned(),
            "   ".to_owned(),
        ]),
        vec![
            "Episode 1.mkv".to_owned(),
            "https://example.com/live".to_owned(),
        ]
    );
}
