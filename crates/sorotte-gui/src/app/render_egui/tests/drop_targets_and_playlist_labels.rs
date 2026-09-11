use super::*;

#[derive(Debug)]
struct TestDroppedFile(PathBuf);

impl egui::DroppedFile for TestDroppedFile {
    fn path(&self) -> &std::path::Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        panic!("routing a dropped media file must not read its contents")
    }
}

fn dropped_file(path: &str) -> egui::DroppedFileHandle {
    std::sync::Arc::new(TestDroppedFile(PathBuf::from(path)))
}

#[test]
fn gui_widget_egui_renderer_routes_drop_handles_without_reading_media() {
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings::default());
    let request = GuiWidgetEguiRenderer::dropped_files_request_for_input(
        &state,
        false,
        None,
        None,
        None,
        vec![
            dropped_file(""),
            dropped_file("episode 1.mkv"),
            dropped_file("C:/Media/episode 2.mkv"),
        ],
    )
    .expect("nonempty file paths should be routed");
    assert_eq!(request.paths, ["episode 1.mkv", "C:/Media/episode 2.mkv"]);
    assert!(
        GuiWidgetEguiRenderer::dropped_files_request_for_input(
            &state,
            false,
            None,
            None,
            None,
            vec![dropped_file("")],
        )
        .is_none()
    );
}

#[test]
fn gui_widget_egui_renderer_prefers_playlist_target_for_hovered_shared_playlist_drops() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettings::default()
    });
    state.main_window.playback.can_manage_playlist = true;
    let request = GuiWidgetEguiRenderer::dropped_files_request_for_input(
        &state,
        true,
        None,
        None,
        None,
        vec![dropped_file("C:/Media/episode1.mkv")],
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
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettings::default()
    });
    let request = GuiWidgetEguiRenderer::dropped_files_request_for_input(
        &state,
        false,
        None,
        None,
        None,
        vec![dropped_file("C:/Media/episode2.mkv")],
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
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings::default());
    let request = GuiWidgetEguiRenderer::dropped_files_request_for_input(
        &state,
        true,
        None,
        None,
        None,
        vec![dropped_file("C:/Media/movie.mkv")],
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettings::default()
    });
    state.main_window.playback.can_manage_playlist = true;
    let request = GuiWidgetEguiRenderer::dropped_files_request_for_input(
        &state,
        true,
        None,
        Some(1),
        None,
        vec![dropped_file("C:/Media/episode3.mkv")],
    )
    .expect("dropped-file request should be derived");

    assert_eq!(request.playlist_insert_slot, Some(1));
}

#[test]
fn gui_widget_egui_renderer_defaults_playlist_drops_to_append_slot_when_hover_slot_is_missing() {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        shared_playlist_enabled: Some(true),
        ..StoredClientSettings::default()
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
        vec![dropped_file("C:/Media/episode3.mkv")],
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
