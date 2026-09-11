use super::*;
use crate::adapter::collect_pending_player_delivery;
use sorotte_client_core::{
    CoordinatorPlayerCommand, DesiredRoomPlayback, DesiredRoomPlaybackUpdateKind, LogicalMediaId,
    MediaTransportKind, PlaybackCoordinator, PlaybackCoordinatorAction, PlayerTransportObservation,
    SeekPreparationTerminalOutcome,
};
use sorotte_player_api::{
    PlayerLoadAttemptResult, PlayerTransportDelta, PlayerTransportSnapshot, SnapshotField,
};

fn coordinator_observation(
    update: PlayerTransportDelta,
    media_generation: u64,
) -> PlayerTransportObservation {
    PlayerTransportObservation {
        media_generation,
        observed_at_seconds: update
            .observed_at
            .expect("mpv transport updates should be timestamped")
            .elapsed_since_adapter_start()
            .as_secs_f64(),
        phase: update.phase,
        position_seconds: update.position_seconds,
        playback_rate: update.playback_rate,
        logical_pause: update.logical_pause,
        paused_for_cache: update.paused_for_cache,
        seeking: update.seeking,
        seekable: update.seekable,
        timeline_kind: update.timeline_kind,
        seekable_ranges: update.seekable_ranges,
        known_live_seekable_window: update.known_live_seekable_window,
        core_idle: update.core_idle,
        playback_restart_sequence: update.playback_restart_sequence,
        cache_buffering_percent: update.cache_percentage,
        buffered_ahead_seconds: update.buffered_duration_seconds,
        input_rate_bytes_per_second: update.input_rate_bytes_per_second,
    }
}

#[test]
fn owned_file_metadata_is_delivered_once_after_acknowledgement() {
    let (transport, _) = fake_transport_with_reads(&[
        r#"{"event":"start-file","playlist_entry_id":41}"#,
        r#"{"event":"property-change","name":"path","data":"C:/media/movie.mkv"}"#,
        r#"{"event":"property-change","name":"duration","data":1439.5}"#,
        r#"{"event":"property-change","name":"file-size","data":123456}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"request_id":1,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport_and_registered_observers(transport);
    adapter
        .set_playback_rate(1.0)
        .expect("ingress should be processed");
    let delivery = collect_pending_player_delivery(&mut adapter);
    let files = delivery.local_files().collect::<Vec<_>>();
    let first = files
        .last()
        .expect("owned file metadata should be published");
    assert_eq!(first.name, "movie.mkv");
    assert_eq!(first.path.as_deref(), Some("C:/media/movie.mkv"));
    assert_eq!(first.duration_seconds, Some(1439.5));
    assert_eq!(first.size_bytes, Some(123456));
    assert_eq!(
        collect_pending_player_delivery(&mut adapter)
            .local_files()
            .count(),
        0,
        "acknowledged metadata must not repeat without fresh ingress"
    );
}

#[test]
fn file_metadata_waits_for_an_owned_available_path() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"event":"start-file","playlist_entry_id":42}"#,
        r#"{"request_id":1,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport_and_registered_observers(transport);
    adapter
        .set_playback_rate(1.0)
        .expect("start without metadata");
    assert_eq!(
        collect_pending_player_delivery(&mut adapter)
            .local_files()
            .count(),
        0
    );
    state.queue_reads(&[
        r#"{"event":"property-change","name":"path","data":"C:/media/movie2.mkv"}"#,
        r#"{"event":"property-change","name":"duration","data":42.0}"#,
        r#"{"event":"property-change","name":"file-size","data":1000}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"request_id":2,"error":"success"}"#,
    ]);
    adapter
        .set_playback_rate(1.0)
        .expect("metadata arrives later");
    let delivery = collect_pending_player_delivery(&mut adapter);
    let update = delivery
        .local_files()
        .last()
        .expect("available file metadata");
    assert_eq!(update.name, "movie2.mkv");
    assert_eq!(update.duration_seconds, Some(42.0));
    assert_eq!(update.size_bytes, Some(1000));
}

#[test]
fn async_property_change_events_from_mpv_queue_local_file_update() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"event":"start-file","playlist_entry_id":41}"#,
        r#"{"event":"property-change","name":"path","data":"C:/media/from-event.mkv"}"#,
        r#"{"event":"property-change","name":"duration","data":120.0}"#,
        r#"{"event":"property-change","name":"file-size","data":987654}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"request_id":1,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport_and_registered_observers(transport);

    adapter.set_paused(true).expect("command should succeed");

    let delivery = collect_pending_player_delivery(&mut adapter);
    let update = delivery.local_files().last().expect("async file metadata");
    assert_eq!(update.name, "from-event.mkv");
    assert_eq!(update.path.as_deref(), Some("C:/media/from-event.mkv"));
    assert_eq!(update.duration_seconds, Some(120.0));
    assert_eq!(update.size_bytes, Some(987654));

    let writes = state.writes();
    assert_eq!(writes.len(), 1);
    let last_payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
    assert_eq!(
        last_payload,
        json!({
            "command": ["set_property", "pause", true],
            "request_id": 1
        })
    );
}

#[test]
fn async_property_change_events_queue_playback_telemetry_update() {
    let (transport, _state) = fake_transport_with_reads(&[
        r#"{"event":"start-file","playlist_entry_id":45}"#,
        r#"{"event":"property-change","name":"pause","data":true}"#,
        r#"{"event":"property-change","name":"time-pos","data":123.25}"#,
        r#"{"event":"property-change","name":"speed","data":1.10}"#,
        r#"{"request_id":1,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport_and_registered_observers(transport);

    adapter
        .set_position(10.0)
        .expect("command should drain and process queued events");

    let delivery = collect_pending_player_delivery(&mut adapter);
    let telemetry = delivery.transport_snapshot();
    assert_eq!(telemetry.logical_pause, SnapshotField::Known(true));
    assert_eq!(telemetry.position_seconds, SnapshotField::Known(123.25));
    assert_eq!(telemetry.playback_rate, SnapshotField::Known(1.10));
    assert_eq!(
        collect_pending_player_delivery(&mut adapter)
            .transport_deltas()
            .count(),
        0
    );
    assert!(adapter.paused());
    assert_eq!(
        adapter.position_seconds(),
        10.0,
        "commanded local state currently wins over earlier async time-pos event in this slice"
    );
    assert_eq!(adapter.playback_rate(), 1.10);
}

#[test]
fn cache_property_change_events_queue_cache_playback_telemetry_without_manual_pause() {
    let (transport, _state) = fake_transport_with_reads(&[
        r#"{"event":"start-file","playlist_entry_id":45}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"event":"playback-restart"}"#,
        r#"{"event":"property-change","name":"pause","data":true}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":true}"#,
        r#"{"event":"property-change","name":"cache-buffering-state","data":42.5}"#,
        r#"{"request_id":1,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport_and_registered_observers(transport);

    adapter
        .set_position(10.0)
        .expect("command should drain and process queued events");

    let delivery = collect_pending_player_delivery(&mut adapter);
    let telemetry = delivery.transport_snapshot();
    assert_eq!(telemetry.paused_for_cache, SnapshotField::Known(true));
    assert_eq!(telemetry.cache_percentage, SnapshotField::Known(42.5));
    assert_eq!(telemetry.logical_pause, SnapshotField::Known(false));
    assert!(adapter.paused());
    assert!(adapter.paused_for_cache());
    assert_eq!(adapter.cache_buffering_percent(), Some(42.5));
    let transport_updates = delivery.transport_deltas().collect::<Vec<_>>();
    assert!(
        transport_updates
            .iter()
            .filter(|update| update.paused_for_cache == Some(true))
            .all(|update| update.logical_pause != Some(true)),
        "confirmed cache pause must not remain logical pause intent"
    );
    assert!(
        transport_updates
            .iter()
            .any(|update| update.paused_for_cache == Some(true))
    );
    assert!(
        transport_updates
            .iter()
            .any(|update| update.cache_percentage == Some(42.5))
    );
}

#[test]
fn transport_lifecycle_and_cache_hints_are_generation_correlated() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"event":"start-file","playlist_entry_id":41}"#,
        r#"{"event":"property-change","name":"path","data":"https://media.invalid/watch?v=generation-test"}"#,
        r#"{"event":"property-change","name":"pause","data":false}"#,
        r#"{"event":"property-change","name":"seeking","data":false}"#,
        r#"{"event":"property-change","name":"seekable","data":true}"#,
        r#"{"event":"playback-restart"}"#,
        r#"{"event":"property-change","name":"pause","data":true}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":true}"#,
        r#"{"event":"property-change","name":"core-idle","data":true}"#,
        r#"{"event":"property-change","name":"demuxer-cache-state","data":{"seekable-ranges":[{"start":10.0,"end":80.0},{"start":90.0,"end":85.0}],"cache-duration":5.25,"fw-bytes":345678,"raw-input-rate":2000000,"reader-pts":42.5,"cache-end":47.75,"eof":false,"underrun":true}}"#,
        r#"{"event":"property-change","name":"demuxer-cache-idle","data":false}"#,
        r#"{"event":"property-change","name":"cache-buffering-state","data":42.5}"#,
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":false}"#,
        r#"{"event":"property-change","name":"core-idle","data":true}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"event":"property-change","name":"core-idle","data":false}"#,
        r#"{"request_id":3,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport_and_registered_observers(transport);

    adapter
        .open_file("https://media.invalid/watch?v=generation-test")
        .expect("load command should be accepted");

    let delivery = collect_pending_player_delivery(&mut adapter);
    let updates = delivery.transport_deltas().collect::<Vec<_>>();
    let mut cache_updates = Vec::new();
    while let Some(update) = adapter.take_cache_telemetry_update() {
        cache_updates.push(update);
    }

    let generation = adapter
        .media_generation()
        .expect("start-file should establish a media generation");
    assert_eq!(generation.get(), 1);
    let restart_index = updates
        .iter()
        .position(|update| update.playback_restart_sequence == Some(1))
        .expect("playback-restart should establish strict physical ownership");
    assert!(
        updates[restart_index..]
            .iter()
            .all(|update| update.media_generation == Some(generation)),
        "transport updates after strict ownership must remain correlated: {updates:#?}"
    );
    assert!(
        updates[..restart_index]
            .iter()
            .all(|update| update.media_generation.is_none()
                || update.media_generation == Some(generation)),
        "pre-binding observations must remain unowned rather than being assigned by a pending-generation guess: {updates:#?}"
    );
    assert!(updates.iter().all(|update| update.observed_at.is_some()));
    assert!(updates.windows(2).all(|window| {
        window[0].observed_at.expect("timestamp should be present")
            <= window[1].observed_at.expect("timestamp should be present")
    }));
    assert!(
        updates
            .iter()
            .any(|update| update.phase == Some(PlayerTransportPhase::Loading))
    );
    let restarted = &updates[restart_index];
    assert_eq!(restarted.phase, Some(PlayerTransportPhase::Playing));
    assert!(
        updates
            .iter()
            .any(|delta| delta.phase == Some(PlayerTransportPhase::Rebuffering))
    );
    let rebuffering = delivery.transport_snapshot();
    assert_eq!(rebuffering.paused_for_cache, SnapshotField::Known(true));
    assert_eq!(rebuffering.core_idle, SnapshotField::Known(true));
    assert_eq!(rebuffering.cache_percentage, SnapshotField::Known(42.5));
    assert_eq!(rebuffering.logical_pause, SnapshotField::Known(false));
    assert!(
        updates[updates
            .iter()
            .position(|delta| delta.paused_for_cache == Some(true))
            .expect("cache pause ingress")..]
            .iter()
            .all(|update| update.phase != Some(PlayerTransportPhase::ReadyPaused))
    );
    assert_eq!(
        rebuffering.seekable_ranges,
        SnapshotField::Known(vec![PlayerSeekableRange::new(10.0, 80.0)])
    );
    assert_eq!(
        rebuffering.buffered_duration_seconds,
        SnapshotField::Known(5.25)
    );
    assert_eq!(rebuffering.buffered_bytes, SnapshotField::Known(345_678));
    assert_eq!(
        rebuffering.input_rate_bytes_per_second,
        SnapshotField::Known(2_000_000)
    );
    let cache_snapshot = cache_updates
        .iter()
        .rev()
        .find(|update| update.media_generation == Some(generation))
        .expect("demuxer-cache-state should emit a complete replacement snapshot");
    assert_eq!(cache_snapshot.reader_position_seconds, Some(42.5));
    assert_eq!(cache_snapshot.cache_end_seconds, Some(47.75));
    assert_eq!(cache_snapshot.eof, Some(false));
    assert_eq!(cache_snapshot.underrun, Some(true));
    let diagnostics = adapter.network_media_diagnostic_snapshot();
    assert_eq!(diagnostics.media_generation, Some(generation));
    assert_eq!(diagnostics.cache_duration_seconds, Some(5.25));
    assert_eq!(diagnostics.forward_bytes, Some(345_678));
    assert_eq!(diagnostics.raw_input_rate_bytes_per_second, Some(2_000_000));
    assert_eq!(diagnostics.reader_position_seconds, Some(42.5));
    assert_eq!(diagnostics.cache_end_seconds, Some(47.75));
    assert_eq!(diagnostics.cache_eof, Some(false));
    assert_eq!(diagnostics.cache_underrun, Some(true));
    assert_eq!(diagnostics.demuxer_cache_idle, Some(false));
    assert_eq!(diagnostics.paused_for_cache, Some(true));
    assert_eq!(adapter.transport_phase(), PlayerTransportPhase::Rebuffering);

    adapter
        .set_playback_rate(1.0)
        .expect("command should drain cache-release observations");
    let settling = collect_pending_player_delivery(&mut adapter).transport_snapshot();
    assert_eq!(settling.paused_for_cache, SnapshotField::Known(false));
    assert_eq!(settling.core_idle, SnapshotField::Known(true));
    assert_eq!(
        settling.phase,
        SnapshotField::Known(PlayerTransportPhase::Rebuffering)
    );
    adapter
        .set_playback_rate(1.0)
        .expect("command should drain resumed core observation");
    let resumed = collect_pending_player_delivery(&mut adapter).transport_snapshot();
    assert_eq!(resumed.core_idle, SnapshotField::Known(false));
    assert_eq!(
        resumed.phase,
        SnapshotField::Known(PlayerTransportPhase::Playing)
    );

    assert_eq!(
        state.writes().len(),
        3,
        "only the three explicit command boundaries advance this fixture"
    );
}

#[test]
fn playback_restart_preserves_observed_core_idle_for_intentionally_paused_media() {
    let (transport, _) = fake_transport_with_reads(&[
        r#"{"event":"start-file","playlist_entry_id":42}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":false}"#,
        r#"{"event":"property-change","name":"pause","data":true}"#,
        r#"{"event":"property-change","name":"core-idle","data":true}"#,
        r#"{"event":"playback-restart"}"#,
        r#"{"request_id":1,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport_and_registered_observers(transport);

    adapter
        .set_playback_rate(1.0)
        .expect("paused playback lifecycle should be observed");

    let mut latest = PlayerTransportSnapshot::default();
    for update in collect_pending_player_delivery(&mut adapter)
        .transport_deltas()
        .cloned()
    {
        latest.apply_delta(update);
    }
    assert_eq!(
        latest.phase,
        SnapshotField::Known(PlayerTransportPhase::ReadyPaused)
    );
    assert_eq!(latest.logical_pause, SnapshotField::Known(true));
    assert_eq!(latest.paused_for_cache, SnapshotField::Known(false));
    assert_eq!(latest.core_idle, SnapshotField::Known(true));
    assert_eq!(latest.playback_restart_sequence, SnapshotField::Known(1));
}

#[test]
fn paused_load_binds_global_pause_and_core_idle_and_restores_pause_after_cache_release() {
    let (transport, _) = fake_transport_with_reads(&[
        r#"{"event":"property-change","name":"pause","data":true}"#,
        r#"{"event":"property-change","name":"core-idle","data":true}"#,
        r#"{"event":"start-file","playlist_entry_id":43}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":false}"#,
        r#"{"event":"playback-restart"}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":true}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":false}"#,
        r#"{"request_id":1,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport_and_registered_observers(transport);

    adapter
        .set_playback_rate(1.0)
        .expect("paused load and cache lifecycle should be observed");

    let generation = adapter
        .media_generation()
        .expect("start-file should bind a media generation");
    let mut latest = PlayerTransportSnapshot::default();
    for update in collect_pending_player_delivery(&mut adapter)
        .transport_deltas()
        .cloned()
    {
        if update.media_generation == Some(generation) {
            latest.apply_delta(update);
        }
    }
    assert_eq!(
        latest.phase,
        SnapshotField::Known(PlayerTransportPhase::ReadyPaused)
    );
    assert_eq!(latest.logical_pause, SnapshotField::Known(true));
    assert_eq!(latest.paused_for_cache, SnapshotField::Known(false));
    assert_eq!(latest.core_idle, SnapshotField::Known(true));
    assert_eq!(latest.playback_restart_sequence, SnapshotField::Known(1));
}

#[test]
fn drained_ready_paused_cannot_finish_preparation_before_delayed_cache_pause() {
    let (transport, _) = fake_transport_with_reads(&[
        r#"{"event":"start-file","playlist_entry_id":44}"#,
        r#"{"event":"property-change","name":"path","data":"https://media.invalid/race.wav"}"#,
        r#"{"event":"property-change","name":"duration","data":60.0}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":false}"#,
        r#"{"event":"property-change","name":"pause","data":true}"#,
        r#"{"event":"property-change","name":"seeking","data":false}"#,
        r#"{"event":"property-change","name":"seekable","data":true}"#,
        r#"{"event":"property-change","name":"time-pos","data":5.0}"#,
        r#"{"event":"property-change","name":"demuxer-cache-state","data":{"seekable-ranges":[{"start":0.0,"end":10.0}],"cache-duration":10.0,"raw-input-rate":5000000}}"#,
        r#"{"event":"property-change","name":"cache-buffering-state","data":100.0}"#,
        r#"{"event":"playback-restart"}"#,
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"event":"property-change","name":"time-pos","data":5.0}"#,
        r#"{"request_id":2,"error":"success"}"#,
        // These delayed pre-seek cache values arrive after command dispatch.
        // A later target-position event must not inherit them.
        r#"{"event":"property-change","name":"cache-buffering-state","data":100.0}"#,
        r#"{"event":"property-change","name":"demuxer-cache-state","data":{"seekable-ranges":[{"start":0.0,"end":10.0}],"cache-duration":10.0,"raw-input-rate":9000000}}"#,
        r#"{"event":"property-change","name":"seeking","data":true}"#,
        // Input-rate/byte-only cache telemetry must also form a position boundary.
        r#"{"event":"property-change","name":"demuxer-cache-state","data":{"fw-bytes":123456,"raw-input-rate":8000000}}"#,
        r#"{"event":"property-change","name":"time-pos","data":40.0}"#,
        // The reverse ordering must not merge old cache evidence back into
        // the already queued target-position update.
        r#"{"event":"property-change","name":"demuxer-cache-state","data":{"seekable-ranges":[{"start":0.0,"end":10.0}],"cache-duration":10.0,"fw-bytes":654321,"raw-input-rate":9000000}}"#,
        r#"{"event":"property-change","name":"seeking","data":false}"#,
        r#"{"event":"playback-restart"}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":true}"#,
        r#"{"request_id":4,"error":"success"}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":false}"#,
        r#"{"request_id":5,"error":"success"}"#,
        // A fresh but delayed old-position sample after Ready must not change
        // the recovery decision inherited from the target epoch.
        r#"{"event":"property-change","name":"demuxer-cache-state","data":{"seekable-ranges":[{"start":0.0,"end":10.0}],"cache-duration":10.0,"raw-input-rate":9000000}}"#,
        r#"{"request_id":6,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport_and_registered_observers(transport);
    let mut coordinator = PlaybackCoordinator::default();
    let generation = coordinator
        .prepare_media(
            LogicalMediaId::new("actual-mpv-ready-cache-race").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;

    adapter
        .set_playback_rate(1.0)
        .expect("initial paused media observations should drain");
    for update in collect_pending_player_delivery(&mut adapter)
        .transport_deltas()
        .cloned()
    {
        coordinator.observe(coordinator_observation(update, generation));
    }
    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            media_generation: generation,
            state_revision: 1,
            paused: false,
            anchor_position_seconds: 40.0,
            anchor_observed_at_seconds: 0.0,
            force_seek: true,
        },
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );

    adapter
        .set_playback_rate(1.0)
        .expect("pre-seek position should trigger coordinator dispatch");
    let mut dispatch_actions = Vec::new();
    for update in collect_pending_player_delivery(&mut adapter)
        .transport_deltas()
        .cloned()
    {
        dispatch_actions.extend(coordinator.observe(coordinator_observation(update, generation)));
    }
    assert!(dispatch_actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if (*position - 40.0).abs() <= f64::EPSILON
    )));

    adapter
        .execute_tracked(PlayerCommand::SetPosition(40.0))
        .expect("mpv should accept the primary seek");
    let mut transient_updates = Vec::new();
    for update in collect_pending_player_delivery(&mut adapter)
        .transport_deltas()
        .cloned()
    {
        transient_updates.push(update);
    }
    assert!(
        transient_updates.iter().any(|update| {
            update.phase == Some(PlayerTransportPhase::ReadyPaused)
                && update.playback_restart_sequence.is_some()
        }),
        "transient updates: {transient_updates:#?}"
    );
    assert!(
        transient_updates
            .iter()
            .filter(|update| {
                update
                    .position_seconds
                    .is_some_and(|position| (position - 40.0).abs() <= f64::EPSILON)
            })
            .all(|update| {
                update.cache_percentage.is_none()
                    && update.buffered_duration_seconds.is_none()
                    && update.buffered_bytes.is_none()
                    && update.input_rate_bytes_per_second.is_none()
            })
    );
    for update in transient_updates {
        coordinator.observe(coordinator_observation(update, generation));
    }
    assert!(coordinator.seek_preparation_snapshot().is_some());
    assert_eq!(coordinator.last_seek_preparation_terminal_outcome(), None);
    assert_eq!(coordinator.metrics().last_buffered_ahead_seconds, None);
    assert_eq!(coordinator.metrics().last_input_rate_bytes_per_second, None);

    adapter
        .set_playback_rate(1.0)
        .expect("delayed cache pause should drain separately");
    for update in collect_pending_player_delivery(&mut adapter)
        .transport_deltas()
        .cloned()
    {
        coordinator.observe(coordinator_observation(update, generation));
    }
    assert!(coordinator.seek_preparation_snapshot().is_some());
    assert_eq!(coordinator.last_seek_preparation_terminal_outcome(), None);
    assert_eq!(coordinator.metrics().last_buffered_ahead_seconds, None);
    assert_eq!(coordinator.metrics().last_input_rate_bytes_per_second, None);

    adapter
        .set_playback_rate(1.0)
        .expect("cache release should drain separately");
    for update in collect_pending_player_delivery(&mut adapter)
        .transport_deltas()
        .cloned()
    {
        coordinator.observe(coordinator_observation(update, generation));
    }
    assert_eq!(
        coordinator.last_seek_preparation_terminal_outcome(),
        Some(SeekPreparationTerminalOutcome::Ready)
    );
    assert_eq!(coordinator.metrics().last_buffered_ahead_seconds, None);
    assert_eq!(coordinator.metrics().last_input_rate_bytes_per_second, None);

    adapter
        .set_playback_rate(1.0)
        .expect("post-ready delayed old cache state should drain separately");
    for update in collect_pending_player_delivery(&mut adapter)
        .transport_deltas()
        .cloned()
    {
        coordinator.observe(coordinator_observation(update, generation));
    }
    assert_eq!(coordinator.metrics().last_buffered_ahead_seconds, None);
    assert_eq!(coordinator.metrics().last_input_rate_bytes_per_second, None);
}

#[test]
fn end_file_error_is_classified_for_the_matching_generation() {
    let (transport, _state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"event":"start-file","playlist_entry_id":900}"#,
        r#"{"event":"end-file","playlist_entry_id":900,"reason":"error","file_error":"Failed to recognize file format."}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"request_id":4,"error":"success"}"#,
        r#"{"request_id":5,"error":"success"}"#,
        r#"{"request_id":6,"error":"success"}"#,
        r#"{"request_id":7,"error":"success"}"#,
        r#"{"request_id":8,"error":"success"}"#,
        r#"{"request_id":9,"error":"success"}"#,
        r#"{"request_id":10,"error":"success"}"#,
        r#"{"request_id":11,"error":"success"}"#,
        r#"{"request_id":12,"error":"success"}"#,
        r#"{"request_id":13,"error":"success"}"#,
        r#"{"request_id":14,"error":"success"}"#,
        r#"{"request_id":15,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .open_file("https://media.invalid/unsupported")
        .expect("load command should be accepted");

    let delivery = collect_player_delivery(&mut adapter);
    let updates = delivery.transport_deltas().collect::<Vec<_>>();
    let failed = updates
        .iter()
        .find(|update| update.phase == Some(PlayerTransportPhase::Failed))
        .expect("end-file error should emit a failed transport observation");
    assert_eq!(
        failed.media_generation.map(|generation| generation.get()),
        Some(1)
    );
    assert_eq!(failed.eof_reached, Some(true));
    assert_eq!(
        failed.error_kind,
        Some(PlayerMediaLoadFailureKind::FormatUnsupported)
    );
    assert_eq!(adapter.transport_phase(), PlayerTransportPhase::Failed);

    let failures = delivery
        .load_outcomes()
        .filter(|outcome| {
            outcome.result
                == PlayerLoadAttemptResult::Failed(PlayerMediaLoadFailureKind::FormatUnsupported)
        })
        .collect::<Vec<_>>();
    assert_eq!(failures.len(), 1);
    assert_eq!(Some(failures[0].media_generation), failed.media_generation);
}

#[test]
fn ending_old_physical_file_before_replacement_start_leaves_transport_empty() {
    let (transport, _state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"event":"start-file","playlist_entry_id":100}"#,
        r#"{"event":"playback-restart"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"request_id":4,"error":"success"}"#,
        r#"{"request_id":5,"error":"success"}"#,
        r#"{"request_id":6,"error":"success"}"#,
        r#"{"request_id":7,"error":"success"}"#,
        r#"{"request_id":8,"error":"success"}"#,
        r#"{"request_id":9,"error":"success"}"#,
        r#"{"request_id":10,"error":"success"}"#,
        r#"{"request_id":11,"error":"success"}"#,
        r#"{"request_id":12,"error":"success"}"#,
        r#"{"request_id":13,"error":"success"}"#,
        r#"{"request_id":14,"error":"success"}"#,
        r#"{"request_id":15,"error":"success"}"#,
        r#"{"request_id":16,"error":"success"}"#,
        r#"{"request_id":17,"error":"success"}"#,
        r#"{"event":"end-file","playlist_entry_id":100,"reason":"stop"}"#,
        r#"{"request_id":18,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);

    adapter
        .open_file("https://media.invalid/first")
        .expect("first load should be accepted");
    let _ = collect_pending_player_delivery(&mut adapter);

    adapter
        .open_file("https://media.invalid/second")
        .expect("replacement load should be accepted");

    assert_eq!(
        adapter.transport_phase(),
        PlayerTransportPhase::Empty,
        "the replacement cannot own transport until its start-file"
    );
    assert_eq!(
        collect_pending_player_delivery(&mut adapter)
            .transport_deltas()
            .count(),
        0,
        "neither the unstarted successor nor the superseded physical episode may publish successor telemetry"
    );
}

#[test]
fn client_message_events_from_syncplayintf_queue_pending_chat_requests() {
    let (transport, _state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"request_id":4,"error":"success"}"#,
        r#"{"request_id":5,"error":"success"}"#,
        r#"{"request_id":6,"error":"success"}"#,
        r#"{"request_id":7,"error":"success"}"#,
        r#"{"request_id":8,"error":"success"}"#,
        r#"{"event":"client-message","args":["syncplayintf-chat","{\"protocol\":\"sorotte-syncplayintf-v1\",\"bridgeInstanceId\":\"test-bridge\",\"ownerId\":\"test-owner\",\"attachmentId\":\"test-attachment\",\"text\":\"hello \\\\ world\"}"]}"#,
        r#"{"request_id":9,"error":"success","data":false}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.enable_test_syncplay_chat_input();

    assert_eq!(
        adapter.take_pending_chat_request(),
        Some("hello \\ world".to_owned())
    );
    assert_eq!(adapter.take_pending_chat_request(), None);
}

#[test]
fn endpoint_attachment_reset_discards_queued_syncplayintf_chat() {
    let (transport, _state) = fake_transport_with_reads(&[
        r#"{"event":"client-message","args":["syncplayintf-chat","{\"protocol\":\"sorotte-syncplayintf-v1\",\"bridgeInstanceId\":\"test-bridge\",\"ownerId\":\"test-owner\",\"attachmentId\":\"test-attachment\",\"text\":\"old endpoint\"}"]}"#,
        r#"{"request_id":1,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport(transport);
    adapter.enable_test_syncplay_chat_input();

    adapter
        .set_paused(false)
        .expect("a command should pump the queued client-message event");
    adapter.reset_test_syncplayintf_attachment();

    assert_eq!(adapter.take_pending_chat_request(), None);
}
