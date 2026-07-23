use super::*;

#[test]
fn stores_opened_file_path() {
    let mut adapter = SimulatedPlayer::new();
    adapter
        .execute(PlayerCommand::OpenFile("movie.mkv".to_owned()))
        .expect("mpv stub should accept file");
    assert_eq!(adapter.test_adapter().current_path(), Some("movie.mkv"));

    let observation = adapter
        .take_local_file_observation()
        .expect("open file should produce a sequenced local file observation");
    let file_update = observation.update;
    assert_eq!(file_update.name, "movie.mkv");
    assert_eq!(file_update.path.as_deref(), Some("movie.mkv"));
    assert_eq!(
        observation.media_generation,
        Some(sorotte_player_api::PlayerMediaGeneration::new(1))
    );
    assert!(observation.observed_at.is_some());
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

    let state = adapter.test_adapter();
    assert!(state.paused());
    assert_eq!(state.position_seconds(), 24.5);
    assert_eq!(state.playback_rate(), 0.95);
    assert!(state.muted());
    assert_eq!(state.volume(), 50.0);
    assert!(state.deinterlace());
    assert!(state.keepaspect());
    assert!(state.keepaspect_window());
    assert!(state.fullscreen());
    assert!(state.ontop());
    assert!(state.border());
    assert!(state.force_window());
    assert!(state.keep_open());
    assert!(state.keep_open_pause());
    assert!(state.cursor_autohide_fs_only());
    assert!(state.stop_screensaver());
    assert!(state.sub_visibility());
    assert!(state.osd_bar());
    assert!(state.window_maximized());
    assert!(state.window_minimized());
}

#[test]
fn queue_local_file_update_is_drained_once() {
    let mut adapter = MpvAdapter::default();
    adapter.queue_local_file_update(
        LocalFileUpdate::new("movie.mkv")
            .with_duration_seconds(95.5)
            .with_size_bytes(123),
    );

    let observation = adapter
        .take_local_file_observation()
        .expect("queued local file observation should be returned");
    let first = observation.update;
    assert_eq!(first.name, "movie.mkv");
    assert_eq!(first.duration_seconds, Some(95.5));
    assert_eq!(first.size_bytes, Some(123));
    assert_eq!(observation.media_generation, None);
    assert!(observation.observed_at.is_some());
    assert_eq!(adapter.take_local_file_observation(), None);
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
