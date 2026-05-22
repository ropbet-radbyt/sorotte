use super::*;

#[test]
fn stores_opened_file_path() {
    let mut adapter = MpvAdapter::default();
    adapter
        .open_file("movie.mkv")
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
    let mut adapter = MpvAdapter::default();
    adapter
        .set_paused(true)
        .expect("mpv stub should accept paused updates");
    adapter
        .set_position(24.5)
        .expect("mpv stub should accept position updates");
    adapter
        .set_playback_rate(0.95)
        .expect("mpv stub should accept speed updates");
    adapter
        .set_muted(true)
        .expect("mpv stub should accept mute updates");
    adapter
        .set_volume(50.0)
        .expect("mpv stub should accept volume updates");
    adapter
        .set_deinterlace(true)
        .expect("mpv stub should accept deinterlace updates");
    adapter
        .set_keepaspect(true)
        .expect("mpv stub should accept keepaspect updates");
    adapter
        .set_keepaspect_window(true)
        .expect("mpv stub should accept keepaspect-window updates");
    adapter
        .set_fullscreen(true)
        .expect("mpv stub should accept fullscreen updates");
    adapter
        .set_ontop(true)
        .expect("mpv stub should accept ontop updates");
    adapter
        .set_border(true)
        .expect("mpv stub should accept border updates");
    adapter
        .set_force_window(true)
        .expect("mpv stub should accept force-window updates");
    adapter
        .set_keep_open(true)
        .expect("mpv stub should accept keep-open updates");
    adapter
        .set_keep_open_pause(true)
        .expect("mpv stub should accept keep-open-pause updates");
    adapter
        .set_cursor_autohide_fs_only(true)
        .expect("mpv stub should accept cursor-autohide-fs-only updates");
    adapter
        .set_stop_screensaver(true)
        .expect("mpv stub should accept stop-screensaver updates");
    adapter
        .set_sub_visibility(true)
        .expect("mpv stub should accept sub-visibility updates");
    adapter
        .set_osd_bar(true)
        .expect("mpv stub should accept osd-bar updates");
    adapter
        .set_window_maximized(true)
        .expect("mpv stub should accept window-maximized updates");
    adapter
        .set_window_minimized(true)
        .expect("mpv stub should accept window-minimized updates");

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
