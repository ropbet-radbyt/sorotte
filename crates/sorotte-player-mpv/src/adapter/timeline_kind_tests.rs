use super::*;

fn loaded_adapter(path: &str, duration_seconds: Option<f64>) -> MpvAdapter {
    let generation = PlayerMediaGeneration::new(41);
    let mut adapter = MpvAdapter {
        active_file_loaded: true,
        active_media_generation: Some(generation),
        next_media_generation: 42,
        current_path: Some(path.to_owned()),
        path_metadata_generation: Some(generation),
        duration_metadata_generation: Some(generation),
        observed_state: MpvObservedState {
            path: Some(path.to_owned()),
            duration_seconds,
            seekable: Some(true),
            ..MpvObservedState::default()
        },
        ..MpvAdapter::default()
    };
    let attachment_epoch = adapter.lifecycle_epoch();
    adapter.apply_lifecycle_input(PlayerLifecycleInput::ExternalLoadObserved {
        attachment_epoch,
        media_generation: generation,
        playlist_entry_id: 41,
        observed_target: path.to_owned(),
        file_loaded: true,
    });
    let attempt_id = adapter
        .player_lifecycle
        .active_load_attempt
        .expect("external fixture load should establish an active attempt");
    adapter.install_physical_projection(
        attempt_id,
        generation,
        Some(41),
        Some(path.to_owned()),
        true,
    );
    adapter.refresh_timeline_kind_from_metadata();
    adapter
}

fn observe_ytdl_is_live(adapter: &mut MpvAdapter, data: Value) {
    adapter.handle_ipc_event(&json!({
        "event": MPV_EVENT_PROPERTY_CHANGE,
        "name": MPV_PROPERTY_YTDL_IS_LIVE,
        "data": data,
    }));
}

fn observe_full_metadata(adapter: &mut MpvAdapter, data: Value) {
    adapter.handle_ipc_event(&json!({
        "event": MPV_EVENT_PROPERTY_CHANGE,
        "name": MPV_PROPERTY_METADATA,
        "data": data,
    }));
}

#[test]
fn paused_core_idle_internal_seek_does_not_latch_transport_in_seeking() {
    let mut adapter = loaded_adapter("C:/media/paused.wav", Some(8.0));
    adapter.logical_pause_explicit = true;
    adapter.paused = true;
    adapter.observed_state.paused = Some(true);
    adapter.observed_state.logical_pause = Some(true);
    adapter.observed_state.paused_for_cache = Some(false);
    adapter.observed_state.core_idle = Some(true);
    adapter.active_generation_has_restarted = true;
    adapter.playback_restart_sequence = 1;
    adapter.transport_phase = PlayerTransportPhase::ReadyPaused;
    adapter.pending_ordered_player_events.clear();
    adapter.pending_transport_telemetry_updates.clear();

    adapter.handle_ipc_event(&json!({
        "event": MPV_EVENT_SEEK,
    }));
    assert_eq!(
        adapter.transport_phase,
        PlayerTransportPhase::ReadyPaused,
        "an internal resync edge must not displace settled intentional pause"
    );
    assert_eq!(adapter.observed_state.seeking, Some(false));
    let seek_edges = adapter
        .pending_ordered_player_events
        .iter()
        .filter_map(|event| match &event.kind {
            PlayerOrderedEventKind::Transport(update) if update.seeking.is_some() => {
                Some((update.phase, update.seeking))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        seek_edges,
        vec![
            (Some(PlayerTransportPhase::Seeking), Some(true)),
            (Some(PlayerTransportPhase::ReadyPaused), Some(false)),
        ],
        "a raw paused native-seek edge must precede the stable normalized transport state"
    );

    adapter.handle_ipc_event(&json!({
        "event": MPV_EVENT_PROPERTY_CHANGE,
        "name": MPV_PROPERTY_SEEKING,
        "data": true,
    }));
    assert_eq!(
        adapter.transport_phase,
        PlayerTransportPhase::ReadyPaused,
        "mpv documents that seeking can remain true while it internally restarts playback"
    );
    assert_eq!(adapter.observed_state.seeking, Some(false));
}

#[test]
fn paused_core_idle_tracked_seek_remains_in_seeking_until_completion() {
    let mut adapter = loaded_adapter("C:/media/paused.wav", Some(8.0));
    adapter.logical_pause_explicit = true;
    adapter.paused = true;
    adapter.observed_state.paused = Some(true);
    adapter.observed_state.logical_pause = Some(true);
    adapter.observed_state.paused_for_cache = Some(false);
    adapter.observed_state.core_idle = Some(true);
    adapter.active_generation_has_restarted = true;
    adapter.playback_restart_sequence = 1;
    adapter.transport_phase = PlayerTransportPhase::ReadyPaused;
    let command_id = adapter.register_tracked_command(
        adapter.active_media_generation,
        TrackedCommandKind::Seek {
            target_seconds: 4.0,
            seeking_finished: false,
            position_in_tolerance: false,
        },
    );
    adapter.accept_tracked_command(command_id);

    adapter.handle_ipc_event(&json!({
        "event": MPV_EVENT_SEEK,
    }));

    assert_eq!(adapter.transport_phase, PlayerTransportPhase::Seeking);
    assert_eq!(adapter.observed_state.seeking, Some(true));
    assert!(
        adapter
            .pending_tracked_commands
            .iter()
            .any(|command| command.id == command_id),
        "the paused resync exception must not complete a Sorotte-owned seek"
    );
}

#[test]
fn youtube_live_metadata_is_positive_sliding_timeline_evidence() {
    let mut adapter = loaded_adapter("https://www.youtube.com/watch?v=live", None);

    observe_ytdl_is_live(&mut adapter, json!("true"));

    assert_eq!(adapter.timeline_kind, PlayerTimelineKind::SlidingLive);
    assert_eq!(
        adapter.ytdl_is_live_metadata_generation,
        adapter.active_media_generation
    );
}

#[test]
fn full_metadata_event_detects_youtube_live_media() {
    let mut adapter = loaded_adapter("https://www.youtube.com/watch?v=live", None);

    observe_full_metadata(
        &mut adapter,
        json!({ "title": "Live channel", "ytdl_is_live": "true" }),
    );

    assert_eq!(adapter.timeline_kind, PlayerTimelineKind::SlidingLive);
    assert!(adapter.ytdl_is_live);
}

#[test]
fn absent_or_false_live_metadata_keeps_durationless_network_media_unknown() {
    for data in [Value::Null, json!(false), json!("false")] {
        let mut adapter = loaded_adapter("https://media.invalid/unknown.m3u8", None);

        observe_ytdl_is_live(&mut adapter, data);

        assert_eq!(adapter.timeline_kind, PlayerTimelineKind::Unknown);
        assert!(!adapter.ytdl_is_live);
    }
}

#[test]
fn positive_live_metadata_is_sticky_for_the_active_generation() {
    let mut adapter = loaded_adapter("https://www.youtube.com/watch?v=live", None);
    observe_full_metadata(&mut adapter, json!({ "ytdl_is_live": "true" }));

    observe_ytdl_is_live(&mut adapter, Value::Null);
    observe_ytdl_is_live(&mut adapter, json!("false"));
    observe_full_metadata(&mut adapter, json!({ "title": "metadata refresh" }));

    assert_eq!(adapter.timeline_kind, PlayerTimelineKind::SlidingLive);
    assert!(adapter.ytdl_is_live);

    let mut reverse_order = loaded_adapter("https://www.youtube.com/watch?v=live", None);
    observe_ytdl_is_live(&mut reverse_order, json!("true"));
    observe_full_metadata(&mut reverse_order, json!({ "ytdl_is_live": "false" }));
    assert_eq!(reverse_order.timeline_kind, PlayerTimelineKind::SlidingLive);
    assert!(reverse_order.ytdl_is_live);
}

#[test]
fn youtube_cache_stall_recovery_preserves_the_active_generation_and_live_timeline() {
    let mut adapter = loaded_adapter("https://www.youtube.com/watch?v=characterization", None);
    let generation = adapter
        .active_media_generation
        .expect("the characterization fixture should have active media");
    adapter.active_generation_has_restarted = true;
    adapter.transport_phase = PlayerTransportPhase::Playing;
    observe_ytdl_is_live(&mut adapter, json!("true"));
    adapter.pending_transport_telemetry_updates.clear();

    adapter.handle_ipc_event(&json!({
        "event": "property-change",
        "name": "paused-for-cache",
        "data": true,
    }));
    adapter.handle_ipc_event(&json!({
        "event": "property-change",
        "name": "core-idle",
        "data": true,
    }));
    adapter.handle_ipc_event(&json!({
        "event": "property-change",
        "name": "demuxer-cache-state",
        "data": {
            "cache-duration": 0.0,
            "raw-input-rate": 0,
            "eof": false,
            "underrun": true,
        },
    }));

    assert_eq!(adapter.transport_phase(), PlayerTransportPhase::Rebuffering);
    assert_eq!(adapter.active_media_generation, Some(generation));
    assert_eq!(adapter.timeline_kind, PlayerTimelineKind::SlidingLive);
    assert_eq!(adapter.take_media_load_outcome(), None);

    adapter.handle_ipc_event(&json!({
        "event": "property-change",
        "name": "paused-for-cache",
        "data": false,
    }));
    adapter.handle_ipc_event(&json!({
        "event": "property-change",
        "name": "core-idle",
        "data": false,
    }));
    adapter.handle_ipc_event(&json!({ "event": "playback-restart" }));

    assert_eq!(adapter.transport_phase(), PlayerTransportPhase::Playing);
    assert_eq!(adapter.active_media_generation, Some(generation));
    assert_eq!(adapter.timeline_kind, PlayerTimelineKind::SlidingLive);
    assert_eq!(adapter.take_media_load_outcome(), None);
    assert!(
        adapter
            .pending_transport_telemetry_updates
            .iter()
            .all(|update| update.media_generation == Some(generation))
    );
}

#[test]
fn finite_duration_network_media_is_vod_without_positive_live_metadata() {
    let mut adapter = loaded_adapter("https://media.invalid/movie.m3u8", Some(120.0));
    observe_ytdl_is_live(&mut adapter, json!("false"));

    assert_eq!(adapter.timeline_kind, PlayerTimelineKind::Vod);
}

#[test]
fn local_paths_and_file_urls_are_always_vod() {
    for path in ["C:/media/movie.mkv", "file:///C:/media/movie.mkv"] {
        let mut adapter = loaded_adapter(path, None);
        observe_ytdl_is_live(&mut adapter, json!("true"));

        assert_eq!(adapter.timeline_kind, PlayerTimelineKind::Vod);
    }
}

#[test]
fn new_generation_clears_live_evidence_and_rejects_stale_metadata() {
    let mut adapter = loaded_adapter("https://www.youtube.com/watch?v=live", None);
    observe_ytdl_is_live(&mut adapter, json!("true"));
    let previous_generation = adapter.active_media_generation;
    assert_eq!(adapter.timeline_kind, PlayerTimelineKind::SlidingLive);

    adapter.handle_start_file_event(&json!({ "playlist_entry_id": 42 }));
    let current_generation = adapter.active_media_generation;
    assert_ne!(current_generation, previous_generation);
    assert_eq!(adapter.timeline_kind, PlayerTimelineKind::Unknown);
    assert!(!adapter.ytdl_is_live);
    assert_eq!(adapter.ytdl_is_live_metadata_generation, None);

    adapter.active_file_loaded = true;
    adapter.current_path = Some("https://media.invalid/next.m3u8".to_owned());
    adapter.observed_state.path = adapter.current_path.clone();
    adapter.observed_state.duration_seconds = None;
    adapter.path_metadata_generation = current_generation;
    adapter.duration_metadata_generation = current_generation;
    adapter.ytdl_is_live = true;
    adapter.ytdl_is_live_metadata_generation = previous_generation;
    adapter.refresh_timeline_kind_from_metadata();

    assert_eq!(adapter.timeline_kind, PlayerTimelineKind::Unknown);
}

#[test]
fn ending_the_active_generation_clears_live_evidence() {
    let mut adapter = loaded_adapter("https://www.youtube.com/watch?v=live", None);
    observe_ytdl_is_live(&mut adapter, json!("true"));
    assert_eq!(adapter.timeline_kind, PlayerTimelineKind::SlidingLive);

    adapter.handle_end_file_event(&json!({ "reason": "eof", "playlist_entry_id": 41 }));

    assert_eq!(adapter.timeline_kind, PlayerTimelineKind::Unknown);
    assert!(!adapter.ytdl_is_live);
    assert_eq!(adapter.ytdl_is_live_metadata_generation, None);
}

#[test]
fn empty_cache_range_snapshot_clears_the_conservative_live_window() {
    let mut adapter = loaded_adapter("https://www.youtube.com/watch?v=live", None);
    observe_ytdl_is_live(&mut adapter, json!("true"));

    let populated = adapter.cache_state_telemetry_update(&json!({
        "seekable-ranges": [{ "start": 80.0, "end": 100.0 }],
    }));
    assert_eq!(
        populated.known_live_seekable_window,
        Some(PlayerSeekableRange::new(80.0, 100.0))
    );

    let cleared = adapter.cache_state_telemetry_update(&json!({
        "seekable-ranges": [],
    }));
    assert_eq!(cleared.seekable_ranges, Some(Vec::new()));
    assert_eq!(cleared.known_live_seekable_window, None);
    assert_eq!(adapter.latest_cached_seekable_window, None);
}

#[test]
fn newer_cache_state_snapshot_clears_metrics_that_mpv_omits() {
    let mut adapter = loaded_adapter("https://media.invalid/first.m3u8", Some(120.0));
    adapter.cache_state_telemetry_update(&json!({
        "cache-duration": 30.0,
        "fw-bytes": 157_286_400,
        "raw-input-rate": 4_000_000,
        "reader-pts": 42.0,
        "cache-end": 72.0,
        "eof": false,
        "underrun": true,
    }));
    adapter.cache_state_telemetry_update(&json!({}));
    let cleared = adapter
        .pending_cache_telemetry_updates
        .pop_back()
        .expect("the newer cache-state observation should be queued");

    assert!(cleared.media_generation.is_some());
    assert!(cleared.observed_at.is_some());
    assert_eq!(cleared.buffered_ahead_seconds, None);
    assert_eq!(cleared.buffered_ahead_bytes, None);
    assert_eq!(cleared.input_rate_bytes_per_second, None);
    assert_eq!(cleared.reader_position_seconds, None);
    assert_eq!(cleared.cache_end_seconds, None);
    assert_eq!(cleared.eof, None);
    assert_eq!(cleared.underrun, None);
}

#[test]
fn authoritative_seek_event_emits_an_explicit_same_generation_cache_clear() {
    let mut adapter = loaded_adapter("https://media.invalid/first.m3u8", Some(120.0));
    let generation = adapter.active_media_generation;
    adapter.cache_state_telemetry_update(&json!({
        "cache-duration": 30.0,
        "fw-bytes": 157_286_400,
        "raw-input-rate": 4_000_000,
        "reader-pts": 42.0,
        "cache-end": 72.0,
        "eof": false,
        "underrun": true,
    }));
    adapter.pending_cache_telemetry_updates.clear();
    adapter.pending_transport_telemetry_updates.clear();

    adapter.handle_seek_event();

    let cleared = adapter
        .pending_cache_telemetry_updates
        .pop_front()
        .expect("the production seek handler should queue a cache clear");
    assert_eq!(cleared.media_generation, generation);
    assert!(cleared.observed_at.is_some());
    assert_eq!(cleared.buffered_ahead_seconds, None);
    assert_eq!(cleared.buffered_ahead_bytes, None);
    assert_eq!(cleared.input_rate_bytes_per_second, None);
    assert_eq!(cleared.reader_position_seconds, None);
    assert_eq!(cleared.cache_end_seconds, None);
    assert_eq!(cleared.eof, None);
    assert_eq!(cleared.underrun, None);
    assert!(
        adapter
            .pending_transport_telemetry_updates
            .iter()
            .any(|update| {
                update.media_generation == generation
                    && update.phase == Some(PlayerTransportPhase::Seeking)
                    && update.seeking == Some(true)
            })
    );
}

#[test]
fn new_media_generation_clears_all_cache_cap_diagnostics() {
    let mut adapter = loaded_adapter("https://media.invalid/first.m3u8", Some(120.0));
    let first_generation = adapter.active_media_generation;
    adapter.cache_state_telemetry_update(&json!({
        "cache-duration": 30.0,
        "fw-bytes": 157_286_400,
        "raw-input-rate": 4_000_000,
        "reader-pts": 42.0,
        "cache-end": 72.0,
        "eof": false,
        "underrun": true,
    }));
    adapter.observed_state.demuxer_cache_idle = Some(true);
    adapter.observed_state.paused_for_cache = Some(false);
    let populated = adapter.network_media_diagnostic_snapshot();
    assert_eq!(populated.forward_bytes, Some(157_286_400));
    assert_eq!(populated.cache_underrun, Some(true));

    adapter.handle_start_file_event(&json!({ "playlist_entry_id": 99 }));

    let reset = adapter.network_media_diagnostic_snapshot();
    assert_ne!(reset.media_generation, first_generation);
    assert_eq!(reset.cache_duration_seconds, None);
    assert_eq!(reset.forward_bytes, None);
    assert_eq!(reset.raw_input_rate_bytes_per_second, None);
    assert_eq!(reset.reader_position_seconds, None);
    assert_eq!(reset.cache_end_seconds, None);
    assert_eq!(reset.cache_eof, None);
    assert_eq!(reset.cache_underrun, None);
    assert_eq!(reset.demuxer_cache_idle, None);
    assert_eq!(reset.paused_for_cache, None);
    assert_eq!(reset.observed_at, None);
}
