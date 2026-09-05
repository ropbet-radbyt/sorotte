use super::*;
use crate::lifecycle::LoadAttemptState;
use std::io;

const NETWORK_PATH: &str = "https://media.example.invalid/premature-eof";

#[derive(Debug, Default)]
struct RejectingRecoveryTransport {
    response: Option<String>,
}

impl MpvJsonIpcTransport for RejectingRecoveryTransport {
    fn send_line_until(&mut self, line: &str, _deadline: Instant) -> io::Result<()> {
        let request: Value = serde_json::from_str(line.trim_end()).map_err(io::Error::other)?;
        let request_id = request["request_id"]
            .as_u64()
            .ok_or_else(|| io::Error::other("missing request id"))?;
        self.response = Some(
            json!({
                "request_id": request_id,
                "error": "recovery load rejected",
            })
            .to_string()
                + "\n",
        );
        Ok(())
    }

    fn read_line_until(&mut self, line: &mut String, _deadline: Instant) -> io::Result<usize> {
        let response = self
            .response
            .take()
            .ok_or_else(|| io::Error::other("missing recovery response"))?;
        line.clear();
        line.push_str(&response);
        Ok(line.len())
    }
}

fn loaded_network_vod(position_seconds: f64, duration_seconds: f64) -> MpvAdapter {
    let generation = PlayerMediaGeneration::new(7);
    let mut adapter = MpvAdapter {
        simulation_mode: true,
        current_path: Some(NETWORK_PATH.to_owned()),
        active_media_generation: Some(generation),
        next_media_generation: 8,
        active_playlist_entry_id: Some(10),
        transport_phase: PlayerTransportPhase::Playing,
        active_file_loaded: true,
        active_generation_has_restarted: true,
        timeline_kind: PlayerTimelineKind::Vod,
        path_metadata_generation: Some(generation),
        duration_metadata_generation: Some(generation),
        observed_state: MpvObservedState {
            path: Some(NETWORK_PATH.to_owned()),
            duration_seconds: Some(duration_seconds),
            position_seconds: Some(position_seconds),
            seekable: Some(true),
            eof_reached: Some(false),
            paused_for_cache: Some(false),
            ..MpvObservedState::default()
        },
        ..MpvAdapter::default()
    };
    let attachment_epoch = adapter.lifecycle_epoch();
    adapter.apply_lifecycle_input(PlayerLifecycleInput::ExternalLoadObserved {
        attachment_epoch,
        media_generation: generation,
        playlist_entry_id: 10,
        observed_target: NETWORK_PATH.to_owned(),
        file_loaded: true,
    });
    let attempt_id = adapter
        .player_lifecycle
        .active_load_attempt
        .expect("external fixture load should establish an active attempt");
    adapter.install_physical_projection(
        attempt_id,
        generation,
        Some(10),
        Some(NETWORK_PATH.to_owned()),
        true,
    );
    adapter
}

#[test]
fn premature_network_eof_uses_a_new_attempt_in_the_same_generation() {
    let mut adapter = loaded_network_vod(257.25, 1_919.0);
    adapter
        .network_options
        .network_media_options
        .insert("cache-secs".to_owned(), "30".to_owned());
    let generation = adapter
        .active_media_generation
        .expect("fixture should have an active generation");
    let old_attempt = adapter
        .player_lifecycle
        .active_load_attempt
        .expect("fixture should have an active attempt");
    let command = adapter.interrupted_network_stream_recovery_load_command(NETWORK_PATH, 257.25);
    assert_eq!(command[0], MPV_COMMAND_LOADFILE);
    assert_eq!(command[1], NETWORK_PATH);
    assert_eq!(command[4]["start"], "257.25");
    assert_eq!(command[4]["cache-secs"], "30");

    adapter.handle_end_file_event(&json!({
        "reason": "eof",
        "playlist_entry_id": 10,
    }));

    let recovery = adapter
        .stream_recovery
        .interrupted_network_stream_recovery
        .expect("early EOF should create a bounded recovery attempt");
    assert_eq!(recovery.media_generation, generation);
    assert_ne!(recovery.latest_attempt_id, old_attempt);
    assert_eq!(recovery.resume_position_seconds, 257.25);
    assert_eq!(adapter.player_lifecycle.logical_terminal, None);
    assert!(matches!(
        adapter.player_lifecycle.load_attempts[&old_attempt].state,
        LoadAttemptState::Terminal(PlayerPhysicalLoadOutcome::Ended)
    ));
    assert_eq!(
        adapter.player_lifecycle.load_attempts[&recovery.latest_attempt_id].media_generation,
        generation
    );
    assert_eq!(adapter.transport_phase, PlayerTransportPhase::Empty);
    assert!(
        adapter
            .pending_transport_telemetry_updates
            .iter()
            .all(|update| {
                update.phase != Some(PlayerTransportPhase::Loading)
                    && update.phase != Some(PlayerTransportPhase::Ended)
                    && update.eof_reached != Some(true)
            }),
        "the successor cannot publish transport before its start-file"
    );
}

#[test]
fn null_terminal_properties_do_not_erase_premature_eof_recovery_evidence() {
    let mut adapter = loaded_network_vod(257.25, 1_919.0);
    adapter.refresh_network_stream_recovery_evidence();
    assert!(
        adapter
            .stream_recovery
            .network_stream_recovery_evidence
            .is_some()
    );

    // mpv is allowed to clear these observed properties before it emits
    // the causal end-file event. Recovery must use the last coherent
    // snapshot for this exact attachment, generation, and physical attempt.
    for property in [
        MPV_PROPERTY_PATH,
        MPV_PROPERTY_DURATION,
        MPV_PROPERTY_TIME_POS,
    ] {
        adapter.handle_ipc_event(&json!({
            "event": MPV_EVENT_PROPERTY_CHANGE,
            "name": property,
            "data": null,
        }));
    }
    assert_eq!(adapter.current_path, None);
    assert_eq!(adapter.observed_state.duration_seconds, None);
    assert_eq!(adapter.observed_state.position_seconds, None);

    adapter.handle_end_file_event(&json!({
        "reason": "eof",
        "playlist_entry_id": 10,
    }));

    assert_eq!(adapter.network_stream_recovery_attempt_count(), 1);
    assert_eq!(adapter.active_media_generation, None);
    assert_eq!(adapter.transport_phase, PlayerTransportPhase::Empty);
}

#[test]
fn seek_without_a_fresh_position_cannot_reload_the_stale_pre_seek_position() {
    let mut adapter = loaded_network_vod(257.25, 1_919.0);
    adapter.refresh_network_stream_recovery_evidence();
    assert_eq!(
        adapter
            .stream_recovery
            .network_stream_recovery_evidence
            .as_ref()
            .map(|evidence| evidence.position_seconds),
        Some(257.25)
    );

    // mpv's seek edge invalidates the old time-pos. If the underlying
    // network request terminates before a post-seek position arrives,
    // Sorotte must not manufacture a recovery load at the pre-seek
    // position and thereby undo the room/user seek.
    adapter.handle_seek_event();
    assert_eq!(adapter.observed_state.seeking, Some(true));

    adapter.handle_end_file_event(&json!({
        "reason": "eof",
        "playlist_entry_id": 10,
    }));

    assert_eq!(
        adapter
            .stream_recovery
            .interrupted_network_stream_recovery
            .as_ref()
            .map(|recovery| recovery.resume_position_seconds),
        None,
        "the pre-seek position must not become the recovery reload target"
    );
    assert_eq!(
        adapter.network_stream_recovery_attempt_count(),
        0,
        "stale pre-seek time-pos must not become a recovery resume target"
    );
    assert_eq!(adapter.transport_phase, PlayerTransportPhase::Ended);
}

#[test]
fn recovery_evidence_rearms_only_after_post_seek_position_and_seek_completion() {
    let mut adapter = loaded_network_vod(257.25, 1_919.0);
    adapter.refresh_network_stream_recovery_evidence();
    adapter.handle_seek_event();

    adapter.handle_ipc_event(&json!({
        "event": MPV_EVENT_PROPERTY_CHANGE,
        "name": MPV_PROPERTY_TIME_POS,
        "data": 512.0,
    }));
    assert!(
        adapter
            .stream_recovery
            .network_stream_recovery_evidence
            .is_none(),
        "time-pos observed while mpv is still seeking is not settled recovery evidence"
    );

    adapter.handle_ipc_event(&json!({
        "event": MPV_EVENT_PROPERTY_CHANGE,
        "name": MPV_PROPERTY_SEEKING,
        "data": false,
    }));
    assert_eq!(
        adapter
            .stream_recovery
            .network_stream_recovery_evidence
            .as_ref()
            .map(|evidence| evidence.position_seconds),
        Some(512.0),
        "the settled post-seek position should re-arm bounded EOF recovery"
    );
}

#[test]
fn new_start_file_clears_recovery_evidence_before_identity_resolution() {
    let mut adapter = loaded_network_vod(257.25, 1_919.0);
    adapter.refresh_network_stream_recovery_evidence();
    assert!(
        adapter
            .stream_recovery
            .network_stream_recovery_evidence
            .is_some()
    );

    adapter.handle_start_file_observation(u64::MAX);

    assert_eq!(
        adapter.stream_recovery.network_stream_recovery_evidence,
        None
    );
    assert!(adapter.lifecycle_reconciliation_due);
}

#[test]
fn near_tail_and_local_eof_are_not_reloaded() {
    let keep_open_eof = json!({
        "event": MPV_EVENT_PROPERTY_CHANGE,
        "name": MPV_PROPERTY_EOF_REACHED,
        "data": true,
    });
    let mut near_tail = loaded_network_vod(1_910.0, 1_919.0);
    near_tail.handle_ipc_event(&keep_open_eof);
    assert_eq!(near_tail.network_stream_recovery_attempt_count(), 0);
    near_tail.handle_end_file_event(&json!({
        "reason": "eof",
        "playlist_entry_id": 10,
    }));
    assert_eq!(near_tail.transport_phase, PlayerTransportPhase::Ended);
    assert_eq!(near_tail.network_stream_recovery_attempt_count(), 0);

    let mut local = loaded_network_vod(257.25, 1_919.0);
    local.current_path = Some("C:/media/movie.mkv".to_owned());
    local.observed_state.path = local.current_path.clone();
    local.handle_ipc_event(&keep_open_eof);
    assert_eq!(local.network_stream_recovery_attempt_count(), 0);
    local.handle_end_file_event(&json!({
        "reason": "eof",
        "playlist_entry_id": 10,
    }));
    assert_eq!(local.transport_phase, PlayerTransportPhase::Ended);
    assert_eq!(local.network_stream_recovery_attempt_count(), 0);
}

#[test]
fn deferred_start_replay_retains_newer_restart_and_arms_cache_watchdog() {
    let mut adapter = MpvAdapter {
        simulation_mode: true,
        ..MpvAdapter::default()
    };
    let generation = adapter.allocate_media_generation();
    let attempt_id = adapter.submit_lifecycle_load(None, generation, NETWORK_PATH, BTreeSet::new());
    let attachment_epoch = adapter.lifecycle_epoch();
    adapter.apply_lifecycle_input(PlayerLifecycleInput::LoadAttemptAccepted {
        attachment_epoch,
        attempt_id,
    });

    // `open_file`'s first authoritative playlist response reduces this
    // entire event prefix before applying the identity snapshot.
    adapter.handle_start_file_observation(42);
    assert!(adapter.deferred_start_file_observation.is_some());
    adapter.handle_playback_restart_event();
    assert!(!adapter.active_generation_has_restarted);
    assert!(
        adapter
            .deferred_start_file_observation
            .is_some_and(|observation| observation.playback_restart_observed_after_start)
    );

    adapter.apply_lifecycle_input(PlayerLifecycleInput::PlaylistSnapshot {
        attachment_epoch,
        entries: vec![AuthoritativePlaylistEntry::new(
            42,
            Some(NETWORK_PATH.to_owned()),
            true,
        )],
        current_path: Some(NETWORK_PATH.to_owned()),
    });
    adapter.replay_deferred_start_file_if_bound();
    assert!(
        adapter.active_generation_has_restarted,
        "replaying the older start-file must retain the newer restart for this attempt"
    );

    adapter.handle_file_loaded_observation(Some(NETWORK_PATH.to_owned()));
    adapter.current_path = Some(NETWORK_PATH.to_owned());
    adapter.observed_state.path = Some(NETWORK_PATH.to_owned());
    adapter.observed_state.duration_seconds = Some(45.0);
    adapter.observed_state.position_seconds = Some(7.424);
    adapter.path_metadata_generation = Some(generation);
    adapter.duration_metadata_generation = Some(generation);
    adapter.refresh_timeline_kind_from_metadata();
    assert_eq!(adapter.timeline_kind, PlayerTimelineKind::Vod);

    adapter.handle_ipc_event(&json!({
        "event": MPV_EVENT_PROPERTY_CHANGE,
        "name": MPV_PROPERTY_PAUSED_FOR_CACHE,
        "data": true,
    }));
    assert!(
        adapter.stream_recovery.network_cache_stall.is_some(),
        "a post-progress cache pause must arm bounded recovery"
    );
}

#[test]
fn deferred_start_replay_delays_restart_until_identity_snapshot() {
    let mut adapter = MpvAdapter {
        simulation_mode: true,
        ..MpvAdapter::default()
    };
    let generation = adapter.allocate_media_generation();
    let attempt_id = adapter.submit_lifecycle_load(None, generation, NETWORK_PATH, BTreeSet::new());
    let attachment_epoch = adapter.lifecycle_epoch();
    adapter.apply_lifecycle_input(PlayerLifecycleInput::LoadAttemptAccepted {
        attachment_epoch,
        attempt_id,
    });
    adapter.handle_start_file_observation(42);

    // This represents the sibling response-boundary ordering where the
    // accepted pending attempt is already reducer-active even though its
    // playlist identity has not yet been applied.
    adapter.player_lifecycle.active_load_attempt = Some(attempt_id);
    adapter.handle_playback_restart_event();
    assert!(!adapter.active_generation_has_restarted);
    assert!(
        adapter
            .deferred_start_file_observation
            .is_some_and(|observation| observation.playback_restart_observed_after_start),
        "the restart must remain attached to the causally newer deferred start"
    );

    adapter.apply_lifecycle_input(PlayerLifecycleInput::PlaylistSnapshot {
        attachment_epoch,
        entries: vec![AuthoritativePlaylistEntry::new(
            42,
            Some(NETWORK_PATH.to_owned()),
            true,
        )],
        current_path: Some(NETWORK_PATH.to_owned()),
    });
    adapter.replay_deferred_start_file_if_bound();

    assert!(
        adapter.active_generation_has_restarted,
        "the older deferred start must not erase a restart already reduced for this attempt"
    );
    assert_eq!(adapter.active_load_attempt_id, Some(attempt_id));
    assert_eq!(adapter.active_playlist_entry_id, Some(42));
}

#[test]
fn deferred_recovery_start_does_not_attribute_restart_to_retained_predecessor() {
    let mut adapter = loaded_network_vod(7.424, 45.0);
    let generation = adapter
        .active_media_generation
        .expect("fixture should have an active generation");
    let predecessor_attempt = adapter
        .player_lifecycle
        .active_load_attempt
        .expect("fixture should have an active predecessor");
    let restart_sequence_before_successor = adapter.playback_restart_sequence;
    let successor_attempt =
        adapter.submit_lifecycle_load(None, generation, NETWORK_PATH, BTreeSet::new());
    let attachment_epoch = adapter.lifecycle_epoch();
    adapter.apply_lifecycle_input(PlayerLifecycleInput::LoadAttemptAccepted {
        attachment_epoch,
        attempt_id: successor_attempt,
    });
    assert_eq!(
        adapter.player_lifecycle.active_load_attempt,
        Some(predecessor_attempt),
        "the predecessor remains reducer-active until the successor playlist ID binds"
    );

    adapter.handle_start_file_observation(42);
    assert!(adapter.deferred_start_file_observation.is_some());
    adapter.handle_playback_restart_event();
    assert_eq!(
        adapter.playback_restart_sequence, restart_sequence_before_successor,
        "the successor restart must not be projected onto the retained predecessor"
    );
    assert!(
        adapter
            .deferred_start_file_observation
            .is_some_and(|observation| observation.playback_restart_observed_after_start)
    );

    adapter.apply_lifecycle_input(PlayerLifecycleInput::PlaylistSnapshot {
        attachment_epoch,
        entries: vec![AuthoritativePlaylistEntry::new(
            42,
            Some(NETWORK_PATH.to_owned()),
            true,
        )],
        current_path: Some(NETWORK_PATH.to_owned()),
    });
    adapter.replay_deferred_start_file_if_bound();
    assert_eq!(adapter.active_load_attempt_id, Some(successor_attempt));
    assert_eq!(adapter.active_playlist_entry_id, Some(42));
    assert!(
        adapter.active_generation_has_restarted,
        "the causal restart must replay only after the successor binds"
    );
    assert_eq!(
        adapter.playback_restart_sequence,
        restart_sequence_before_successor.wrapping_add(1).max(1)
    );

    adapter.handle_file_loaded_observation(Some(NETWORK_PATH.to_owned()));
    adapter.current_path = Some(NETWORK_PATH.to_owned());
    adapter.observed_state.path = Some(NETWORK_PATH.to_owned());
    adapter.observed_state.duration_seconds = Some(45.0);
    adapter.observed_state.position_seconds = Some(7.424);
    adapter.path_metadata_generation = Some(generation);
    adapter.duration_metadata_generation = Some(generation);
    adapter.refresh_timeline_kind_from_metadata();
    adapter.handle_ipc_event(&json!({
        "event": MPV_EVENT_PROPERTY_CHANGE,
        "name": MPV_PROPERTY_PAUSED_FOR_CACHE,
        "data": true,
    }));
    assert!(
        adapter.stream_recovery.network_cache_stall.is_some(),
        "the correctly attributed successor restart must leave the cache watchdog armed"
    );
}

#[test]
fn unknown_timeline_cache_stall_uses_coherent_vod_recovery_evidence() {
    let mut adapter = loaded_network_vod(7.424, 45.0);
    adapter.timeline_kind = PlayerTimelineKind::Unknown;
    adapter.handle_ipc_event(&json!({
        "event": MPV_EVENT_PROPERTY_CHANGE,
        "name": MPV_PROPERTY_PAUSED_FOR_CACHE,
        "data": true,
    }));
    adapter
        .stream_recovery
        .network_cache_stall
        .as_mut()
        .expect("an unknown timeline without positive live evidence should arm")
        .last_progress_at =
        Instant::now() - adapter.network_cache_stall_recovery_delay() - Duration::from_millis(1);

    adapter.maintain_network_cache_stall_recovery();

    assert_eq!(adapter.network_stream_recovery_attempt_count(), 1);
    assert_eq!(adapter.stream_recovery.network_cache_stall, None);

    let mut positive_live = loaded_network_vod(7.424, 45.0);
    let generation = positive_live
        .active_media_generation
        .expect("fixture generation");
    positive_live.timeline_kind = PlayerTimelineKind::Unknown;
    positive_live.ytdl_is_live = true;
    positive_live.ytdl_is_live_metadata_generation = Some(generation);
    positive_live.handle_ipc_event(&json!({
        "event": MPV_EVENT_PROPERTY_CHANGE,
        "name": MPV_PROPERTY_PAUSED_FOR_CACHE,
        "data": true,
    }));
    assert!(
        positive_live.stream_recovery.network_cache_stall.is_none(),
        "generation-bound positive live metadata must remain excluded"
    );
}

#[test]
fn sustained_cache_stall_uses_the_same_bounded_recovery_transaction() {
    let mut adapter = loaded_network_vod(400.0, 1_919.0);
    let generation = adapter.active_media_generation;
    adapter.handle_ipc_event(&json!({
        "event": "property-change",
        "name": MPV_PROPERTY_PAUSED_FOR_CACHE,
        "data": true,
    }));
    let delay = adapter.network_cache_stall_recovery_delay();
    adapter
        .stream_recovery
        .network_cache_stall
        .as_mut()
        .expect("cache pause should arm the watchdog")
        .last_progress_at = Instant::now() - delay - Duration::from_millis(1);

    adapter.maintain_network_cache_stall_recovery();

    assert_eq!(adapter.active_media_generation, generation);
    assert_eq!(adapter.network_stream_recovery_attempt_count(), 1);
    assert_eq!(
        adapter.transport_phase,
        PlayerTransportPhase::Rebuffering,
        "the accepted successor does not own transport until start-file"
    );
    assert_eq!(adapter.stream_recovery.network_cache_stall, None);
}

#[test]
fn rejected_recovery_preserves_old_physical_owner_and_total_budget() {
    let mut adapter = loaded_network_vod(257.25, 1_919.0);
    adapter.simulation_mode = false;
    adapter.ipc_client = Some(MpvJsonIpcClient::new(Box::new(
        RejectingRecoveryTransport::default(),
    )));
    let generation = adapter
        .active_media_generation
        .expect("fixture should have an active generation");
    let active_attempt = adapter
        .player_lifecycle
        .active_load_attempt
        .expect("fixture should have an active attempt");
    adapter.observed_state.eof_reached = Some(true);
    adapter.queue_transport_telemetry_update(
        adapter
            .transport_update_for(generation)
            .with_phase(PlayerTransportPhase::Ended),
    );
    adapter.queue_cache_telemetry_update(PlayerCacheTelemetryUpdate {
        media_generation: Some(generation),
        eof: Some(true),
        ..PlayerCacheTelemetryUpdate::default()
    });

    for attempt in 1..=MAX_TOTAL_INTERRUPTED_NETWORK_STREAM_RECOVERY_ATTEMPTS {
        assert!(!adapter.try_recover_interrupted_network_stream(generation));
        assert_eq!(adapter.network_stream_recovery_attempt_count(), attempt);
        assert_eq!(adapter.transport_phase, PlayerTransportPhase::Playing);
        assert_eq!(adapter.observed_state.eof_reached, Some(true));
        assert_eq!(adapter.pending_transport_telemetry_updates.len(), 1);
        assert_eq!(adapter.pending_cache_telemetry_updates.len(), 1);
        assert_eq!(
            adapter.player_lifecycle.active_load_attempt,
            Some(active_attempt)
        );
        assert_eq!(
            adapter.player_lifecycle.load_attempts[&active_attempt].superseded_by,
            None
        );
        if attempt < MAX_TOTAL_INTERRUPTED_NETWORK_STREAM_RECOVERY_ATTEMPTS {
            let advanced = 257.25 + attempt as f64 * 3.0;
            adapter.observe_interrupted_network_stream_recovery_progress(advanced);
            adapter.observed_state.position_seconds = Some(advanced);
        }
    }
    assert!(!adapter.try_recover_interrupted_network_stream(generation));
    assert_eq!(
        adapter.network_stream_recovery_attempt_count(),
        MAX_TOTAL_INTERRUPTED_NETWORK_STREAM_RECOVERY_ATTEMPTS
    );
}

#[test]
fn keep_open_premature_eof_property_starts_bounded_recovery_without_end_file() {
    let mut adapter = loaded_network_vod(257.25, 1_919.0);
    let generation = adapter
        .active_media_generation
        .expect("fixture should have an active generation");
    let old_attempt = adapter
        .player_lifecycle
        .active_load_attempt
        .expect("fixture should have an active attempt");
    adapter.handle_ipc_event(&json!({
        "event": MPV_EVENT_PROPERTY_CHANGE,
        "name": MPV_PROPERTY_EOF_REACHED,
        "data": true,
    }));

    let recovery = adapter
        .stream_recovery
        .interrupted_network_stream_recovery
        .expect("keep-open premature EOF should create a bounded recovery attempt");
    assert_eq!(recovery.media_generation, generation);
    assert_ne!(recovery.latest_attempt_id, old_attempt);
    assert_eq!(recovery.resume_position_seconds, 257.25);
    assert_eq!(adapter.player_lifecycle.provisional_eof_attempt(), None);
    assert_eq!(adapter.player_lifecycle.logical_terminal, None);
    assert_eq!(
        adapter.player_lifecycle.load_attempts[&old_attempt].superseded_by,
        Some(recovery.latest_attempt_id)
    );
    let provisional = adapter
        .take_ordered_event_batch()
        .expect("the recovery transition remains pump-visible");
    assert!(provisional.ordered_events.iter().all(|event| {
        !matches!(
            &event.kind,
            PlayerOrderedEventKind::Transport(update)
                if update.media_generation == Some(generation)
                    && (matches!(
                        update.phase,
                        Some(PlayerTransportPhase::Ended | PlayerTransportPhase::Failed)
                    ) || update.eof_reached == Some(true))
        )
    }));
    assert_eq!(adapter.transport_phase, PlayerTransportPhase::Playing);
    assert_eq!(
        adapter.observed_state.eof_reached,
        Some(true),
        "physical EOF remains internal evidence while the successor is starting"
    );
}

#[test]
fn progress_seek_and_restart_cancel_provisional_eof_without_a_terminal() {
    // A near-tail EOF is deliberately ineligible for automatic reload, leaving the
    // provisional lifecycle candidate available for contradictory evidence to cancel.
    let mut adapter = loaded_network_vod(1_910.0, 1_919.0);
    let eof_event = json!({
        "event": MPV_EVENT_PROPERTY_CHANGE,
        "name": MPV_PROPERTY_EOF_REACHED,
        "data": true,
    });

    adapter.handle_ipc_event(&eof_event);
    assert!(adapter.player_lifecycle.provisional_eof_attempt().is_some());
    adapter.handle_ipc_event(&json!({
        "event": MPV_EVENT_PROPERTY_CHANGE,
        "name": MPV_PROPERTY_TIME_POS,
        "data": 1_911.0,
    }));
    assert_eq!(adapter.player_lifecycle.provisional_eof_attempt(), None);

    adapter.handle_ipc_event(&eof_event);
    adapter.handle_seek_event();
    assert_eq!(adapter.player_lifecycle.provisional_eof_attempt(), None);

    adapter.handle_ipc_event(&eof_event);
    adapter.handle_playback_restart_event();
    assert_eq!(adapter.player_lifecycle.provisional_eof_attempt(), None);
    assert_ne!(adapter.transport_phase, PlayerTransportPhase::Ended);
    assert_eq!(adapter.player_lifecycle.logical_terminal, None);
    assert!(
        adapter
            .pending_transport_telemetry_updates
            .iter()
            .all(|update| {
                update.phase != Some(PlayerTransportPhase::Ended)
                    && update.phase != Some(PlayerTransportPhase::Failed)
                    && update.eof_reached != Some(true)
            })
    );
}

#[test]
fn cache_progress_and_configured_wait_restart_the_watchdog() {
    let mut adapter = loaded_network_vod(257.25, 1_919.0);
    adapter
        .network_options
        .network_media_options
        .insert("cache-pause-wait".to_owned(), "45".to_owned());
    assert_eq!(
        adapter.network_cache_stall_recovery_delay(),
        Duration::from_secs(50)
    );
    adapter.observed_state.paused_for_cache = Some(true);
    adapter.observed_state.buffered_ahead_bytes = Some(1_000);
    adapter.observed_state.cache_end_seconds = Some(258.0);
    adapter.observe_network_cache_pause_for_recovery(true);
    let old_progress = Instant::now() - Duration::from_secs(60);
    adapter
        .stream_recovery
        .network_cache_stall
        .as_mut()
        .expect("rebuffering network VOD should arm recovery")
        .last_progress_at = old_progress;

    adapter.observed_state.buffered_ahead_bytes = Some(2_000);
    adapter.observed_state.cache_end_seconds = Some(259.0);
    adapter.maintain_network_cache_stall_recovery();

    let stall = adapter
        .stream_recovery
        .network_cache_stall
        .expect("forward cache growth should keep the watchdog armed");
    assert!(stall.last_progress_at > old_progress);
    assert_eq!(adapter.transport_phase, PlayerTransportPhase::Playing);
    assert_eq!(adapter.network_stream_recovery_attempt_count(), 0);
}

#[test]
fn first_usable_cache_sample_restarts_watchdog_instead_of_reloading() {
    let mut adapter = loaded_network_vod(257.25, 1_919.0);
    adapter.observed_state.paused_for_cache = Some(true);
    adapter.observe_network_cache_pause_for_recovery(true);
    let old_progress = Instant::now() - adapter.network_cache_stall_recovery_delay();
    adapter
        .stream_recovery
        .network_cache_stall
        .as_mut()
        .expect("rebuffering network VOD should arm recovery")
        .last_progress_at = old_progress;

    adapter.observed_state.buffered_ahead_bytes = Some(1_000);
    adapter.observed_state.cache_end_seconds = Some(258.0);
    adapter.maintain_network_cache_stall_recovery();

    let stall = adapter
        .stream_recovery
        .network_cache_stall
        .expect("the first positive cache sample should refresh the watchdog");
    assert!(stall.last_progress_at > old_progress);
    assert_eq!(adapter.transport_phase, PlayerTransportPhase::Playing);
    assert_eq!(adapter.network_stream_recovery_attempt_count(), 0);
}

#[test]
fn cache_watchdog_ignores_initial_buffering_and_active_seek_and_disarms_on_release() {
    let mut adapter = loaded_network_vod(257.25, 1_919.0);
    adapter.active_generation_has_restarted = false;
    adapter.observed_state.paused_for_cache = Some(true);
    adapter.observe_network_cache_pause_for_recovery(true);
    assert!(adapter.stream_recovery.network_cache_stall.is_none());

    adapter.active_generation_has_restarted = true;
    adapter.observed_state.seeking = Some(true);
    adapter.observe_network_cache_pause_for_recovery(true);
    assert!(adapter.stream_recovery.network_cache_stall.is_none());

    adapter.observed_state.seeking = Some(false);
    adapter.observe_network_cache_pause_for_recovery(true);
    assert!(adapter.stream_recovery.network_cache_stall.is_some());
    adapter.observed_state.paused_for_cache = Some(false);
    adapter.observe_network_cache_pause_for_recovery(false);
    assert!(adapter.stream_recovery.network_cache_stall.is_none());
}

#[test]
fn repro_cache_watchdog_rearms_when_seek_finishes_still_cache_paused() {
    let mut adapter = loaded_network_vod(257.25, 1_919.0);
    adapter.handle_ipc_event(&json!({
        "event": MPV_EVENT_PROPERTY_CHANGE,
        "name": MPV_PROPERTY_PAUSED_FOR_CACHE,
        "data": true,
    }));
    assert!(
        adapter.stream_recovery.network_cache_stall.is_some(),
        "the initial rebuffer should arm the watchdog"
    );

    adapter.handle_seek_event();
    assert_eq!(adapter.observed_state.paused_for_cache, Some(true));
    assert!(
        adapter.stream_recovery.network_cache_stall.is_none(),
        "an active seek should temporarily disarm recovery"
    );

    // `paused-for-cache` did not change across the seek, so mpv is not
    // required to emit that property again. The seeking=false edge is the
    // only new observation proving that the watchdog may safely rearm.
    adapter.handle_ipc_event(&json!({
        "event": MPV_EVENT_PROPERTY_CHANGE,
        "name": MPV_PROPERTY_SEEKING,
        "data": false,
    }));

    assert_eq!(adapter.observed_state.seeking, Some(false));
    assert_eq!(adapter.observed_state.paused_for_cache, Some(true));
    assert!(
        adapter.stream_recovery.network_cache_stall.is_some(),
        "a dead stream that remains cache-paused after seeking must not lose recovery forever"
    );
}

#[test]
fn cache_stall_near_tail_can_recover_but_only_acceptance_disarms_it() {
    let mut near_tail = loaded_network_vod(1_918.0, 1_919.0);
    near_tail.observed_state.paused_for_cache = Some(true);
    near_tail.observe_network_cache_pause_for_recovery(true);
    near_tail
        .stream_recovery
        .network_cache_stall
        .as_mut()
        .expect("near-tail cache stall should arm recovery")
        .last_progress_at = Instant::now() - near_tail.network_cache_stall_recovery_delay();

    near_tail.maintain_network_cache_stall_recovery();

    assert_eq!(
        near_tail.transport_phase,
        PlayerTransportPhase::Playing,
        "the accepted recovery successor cannot replace the playing projection before start-file"
    );
    assert_eq!(near_tail.network_stream_recovery_attempt_count(), 1);
    assert!(near_tail.stream_recovery.network_cache_stall.is_none());

    let mut exhausted = loaded_network_vod(257.25, 1_919.0);
    let generation = exhausted
        .active_media_generation
        .expect("fixture should have an active generation");
    let active_attempt = exhausted
        .player_lifecycle
        .active_load_attempt
        .expect("fixture should have an active attempt");
    exhausted.observed_state.paused_for_cache = Some(true);
    exhausted
        .stream_recovery
        .interrupted_network_stream_recovery = Some(InterruptedNetworkStreamRecovery {
        media_generation: generation,
        latest_attempt_id: active_attempt,
        resume_position_seconds: 257.25,
        consecutive_attempts: MAX_CONSECUTIVE_INTERRUPTED_NETWORK_STREAM_RECOVERY_ATTEMPTS,
        total_attempts: MAX_TOTAL_INTERRUPTED_NETWORK_STREAM_RECOVERY_ATTEMPTS,
    });
    exhausted.observe_network_cache_pause_for_recovery(true);
    exhausted
        .stream_recovery
        .network_cache_stall
        .as_mut()
        .expect("exhausted cache stall should remain represented")
        .last_progress_at = Instant::now() - exhausted.network_cache_stall_recovery_delay();

    exhausted.maintain_network_cache_stall_recovery();

    assert!(exhausted.stream_recovery.network_cache_stall.is_some());
    assert_eq!(exhausted.transport_phase, PlayerTransportPhase::Playing);
}
