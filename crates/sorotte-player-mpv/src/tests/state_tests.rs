use super::*;

#[test]
fn stores_opened_file_path() {
    let mut adapter = SimulatedPlayer::new();
    adapter
        .execute(PlayerCommand::OpenFile("movie.mkv".to_owned()))
        .expect("mpv stub should accept file");
    assert_eq!(adapter.current_path(), Some("movie.mkv"));

    let file_update = adapter
        .take_local_file_update()
        .expect("open file should produce local file update");
    assert_eq!(file_update.name, "movie.mkv");
    assert_eq!(file_update.path.as_deref(), Some("movie.mkv"));
}

#[test]
fn stores_runtime_state_updates() {
    let mut adapter = SimulatedPlayer::new();
    for command in [
        PlayerCommand::SetPaused(true),
        PlayerCommand::SetPosition(24.5),
        PlayerCommand::SetPlaybackRate(0.95),
        PlayerCommand::SetMuted(true),
        PlayerCommand::SetVolume(50.0),
        PlayerCommand::SetDeinterlace(true),
        PlayerCommand::SetKeepaspect(true),
        PlayerCommand::SetKeepaspectWindow(true),
        PlayerCommand::SetFullscreen(true),
        PlayerCommand::SetOntop(true),
        PlayerCommand::SetBorder(true),
        PlayerCommand::SetForceWindow(true),
        PlayerCommand::SetKeepOpen(true),
        PlayerCommand::SetKeepOpenPause(true),
        PlayerCommand::SetCursorAutohideFsOnly(true),
        PlayerCommand::SetStopScreensaver(true),
        PlayerCommand::SetSubVisibility(true),
        PlayerCommand::SetOsdBar(true),
        PlayerCommand::SetWindowMaximized(true),
        PlayerCommand::SetWindowMinimized(true),
    ] {
        adapter
            .execute(command)
            .expect("simulated mpv should accept typed command");
    }

    assert!(adapter.paused());
    assert_eq!(adapter.position_seconds(), 24.5);
    assert_eq!(adapter.playback_rate(), 0.95);
    assert!(adapter.muted());
    assert_eq!(adapter.volume(), 50.0);
    assert!(adapter.deinterlace());
    assert!(adapter.keepaspect());
    assert!(adapter.keepaspect_window());
    assert!(adapter.fullscreen());
    assert!(adapter.ontop());
    assert!(adapter.border());
    assert!(adapter.force_window());
    assert!(adapter.keep_open());
    assert!(adapter.keep_open_pause());
    assert!(adapter.cursor_autohide_fs_only());
    assert!(adapter.stop_screensaver());
    assert!(adapter.sub_visibility());
    assert!(adapter.osd_bar());
    assert!(adapter.window_maximized());
    assert!(adapter.window_minimized());
}

#[test]
fn queue_local_file_update_is_drained_once() {
    let mut adapter = MpvAdapter::default();
    adapter.queue_local_file_update(
        LocalFileUpdate::new("movie.mkv")
            .with_duration_seconds(95.5)
            .with_size_bytes(123),
    );

    let first = adapter
        .take_local_file_update()
        .expect("queued local file update should be returned");
    assert_eq!(first.name, "movie.mkv");
    assert_eq!(first.duration_seconds, Some(95.5));
    assert_eq!(first.size_bytes, Some(123));
    assert_eq!(adapter.take_local_file_update(), None);
}

#[test]
fn disconnected_adapter_does_not_simulate_success() {
    let mut adapter = MpvAdapter::default();

    assert_eq!(
        adapter.open_file("movie.mkv"),
        Err(PlayerError::NotConnected)
    );
    assert_eq!(adapter.current_path(), None);
    assert_eq!(adapter.take_local_file_update(), None);
}
