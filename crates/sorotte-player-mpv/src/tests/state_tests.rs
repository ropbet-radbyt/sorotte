use super::*;
use sorotte_player_api::{PlayerEvent, PlayerMediaGeneration};

#[test]
fn simulated_unload_delivers_terminal_lifecycle_after_load_acknowledgement() {
    let mut adapter = MpvAdapter::simulated();
    adapter.open_file("episode.mkv").unwrap();
    while let Some(batch) = adapter.take_player_event_batch() {
        adapter
            .acknowledge_player_event_batch(batch.acknowledgement_token)
            .unwrap();
    }

    adapter.unload().unwrap();

    assert_eq!(adapter.current_path(), None);
    let batch = adapter
        .take_player_event_batch()
        .expect("unload must retire the observed physical load for ordered consumers");
    assert!(batch.events.iter().any(|event| matches!(
        event.event,
        sorotte_player_api::PlayerEvent::LoadAttemptTerminal {
            outcome: sorotte_player_api::PlayerPhysicalLoadOutcome::Ended,
            ..
        }
    )));
    adapter
        .acknowledge_player_event_batch(batch.acknowledgement_token)
        .unwrap();
    assert!(adapter.take_player_event_batch().is_none());
}

#[test]
fn stores_opened_file_path() {
    let mut adapter = MpvAdapter::simulated();
    adapter
        .execute(PlayerCommand::OpenFile("movie.mkv".to_owned()))
        .expect("mpv stub should accept file");
    assert_eq!(adapter.current_path(), Some("movie.mkv"));

    let batch = adapter
        .take_player_event_batch()
        .expect("opened file batch");
    let (attempt_id, generation, file) = batch
        .events
        .iter()
        .find_map(|item| match &item.event {
            PlayerEvent::LocalFileChanged {
                attempt_id,
                media_generation,
                update,
            } => Some((*attempt_id, *media_generation, update)),
            _ => None,
        })
        .expect("owned local file");
    assert_eq!(file.name, "movie.mkv");
    assert_eq!(file.path.as_deref(), Some("movie.mkv"));
    assert_eq!(generation, PlayerMediaGeneration::new(1));
    assert!(batch.events.iter().any(|item| matches!(item.event,
        PlayerEvent::LoadAttemptActive { attempt_id: active, .. } if active == attempt_id
    )));
}

#[test]
fn stores_runtime_state_updates() {
    let mut adapter = MpvAdapter::simulated();
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

    let state = &adapter;
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
    let mut adapter = MpvAdapter::simulated();
    adapter.open_file("movie.mkv").expect("physical owner");
    let _ = collect_player_delivery(&mut adapter);
    adapter.queue_local_file_update(
        LocalFileUpdate::new("movie.mkv")
            .with_duration_seconds(95.5)
            .with_size_bytes(123),
    );
    let batch = adapter
        .take_player_event_batch()
        .expect("owned metadata batch");
    let files = batch
        .events
        .iter()
        .filter_map(|item| match &item.event {
            PlayerEvent::LocalFileChanged {
                media_generation,
                update,
                ..
            } => Some((media_generation, update)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(files.len(), 1);
    assert_eq!(*files[0].0, PlayerMediaGeneration::new(1));
    assert_eq!(files[0].1.duration_seconds, Some(95.5));
    assert_eq!(files[0].1.size_bytes, Some(123));
    adapter
        .acknowledge_player_event_batch(batch.acknowledgement_token)
        .expect("metadata receipt");
    assert!(adapter.take_player_event_batch().is_none());
}

#[test]
fn disconnected_adapter_does_not_simulate_success() {
    let mut adapter = MpvAdapter::default();

    assert_eq!(
        adapter.open_file("movie.mkv"),
        Err(PlayerError::NotConnected)
    );
    assert_eq!(adapter.current_path(), None);
    assert_eq!(
        collect_player_delivery(&mut adapter).local_files().count(),
        0
    );
}
