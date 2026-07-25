use super::*;
use sorotte_player_api::{
    PlayerCommandFailureKind, PlayerCommandProgress, PlayerCommandProgressState,
    PlayerCommandResult, PlayerMediaLoadOutcome, PlayerOrderedEventKind, PlayerPlayIntent,
    PlayerTransportPhase, PlayerTransportTelemetryUpdate,
};

fn adapter_with_registered_observers(lines: &[&str]) -> MpvAdapter {
    let (transport, _) = fake_transport_with_reads(lines);
    MpvAdapter::with_test_transport_and_registered_observers(transport)
}

fn assert_accepted(
    progress: PlayerCommandProgress,
    command_id: sorotte_player_api::PlayerCommandId,
) {
    assert_eq!(progress.command_id, command_id);
    assert_eq!(progress.state, PlayerCommandProgressState::Accepted);
    assert!(progress.observed_at.is_some());
}

fn assert_completed(
    progress: PlayerCommandProgress,
    command_id: sorotte_player_api::PlayerCommandId,
) {
    assert_eq!(progress.command_id, command_id);
    assert_eq!(
        progress.state,
        PlayerCommandProgressState::Finished(PlayerCommandResult::Completed)
    );
    assert!(progress.observed_at.is_some());
}

#[test]
fn tracked_ipc_success_is_accepted_but_not_completed() {
    let mut adapter = adapter_with_registered_observers(&[r#"{"request_id":1,"error":"success"}"#]);

    let command_id = adapter
        .execute_tracked(PlayerCommand::SetPaused(true))
        .expect("mpv should accept the tracked pause command");

    assert!(
        !adapter.paused(),
        "tracked IPC acceptance must not overwrite the last observed pause state"
    );

    assert_accepted(
        adapter
            .take_command_progress()
            .expect("acceptance should be reported"),
        command_id,
    );
    assert_eq!(
        adapter.take_command_progress(),
        None,
        "the JSON IPC success response must not complete the command"
    );
}

#[test]
fn tracked_pause_completes_only_after_logical_pause_is_observed() {
    let mut adapter = adapter_with_registered_observers(&[
        r#"{"event":"start-file","playlist_entry_id":1}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":false}"#,
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"event":"property-change","name":"pause","data":true}"#,
        r#"{"request_id":3,"error":"success"}"#,
    ]);
    adapter
        .set_playback_rate(1.0)
        .expect("setup observations should be drained");

    let command_id = adapter
        .execute_tracked(PlayerCommand::SetPaused(true))
        .expect("pause should be accepted");
    assert_accepted(
        adapter.take_command_progress().expect("accepted progress"),
        command_id,
    );
    assert_eq!(adapter.take_command_progress(), None);

    adapter
        .set_playback_rate(1.0)
        .expect("logical pause observation should be drained");
    assert_completed(
        adapter.take_command_progress().expect("completed progress"),
        command_id,
    );
}

#[test]
fn cache_induced_pause_does_not_acknowledge_logical_pause_until_cache_releases() {
    let mut adapter = adapter_with_registered_observers(&[
        r#"{"event":"start-file","playlist_entry_id":7}"#,
        r#"{"event":"property-change","name":"pause","data":true}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":true}"#,
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"event":"property-change","name":"pause","data":true}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":false}"#,
        r#"{"request_id":4,"error":"success"}"#,
    ]);
    adapter
        .set_playback_rate(1.0)
        .expect("cache-paused setup state should be observed");

    let command_id = adapter
        .execute_tracked(PlayerCommand::SetPaused(true))
        .expect("pause should be accepted");
    assert_accepted(
        adapter.take_command_progress().expect("accepted progress"),
        command_id,
    );
    adapter
        .set_playback_rate(1.0)
        .expect("cache-induced pause should be observed");
    assert_eq!(
        adapter.take_command_progress(),
        None,
        "pause=true while paused-for-cache=true is not a logical-pause acknowledgement"
    );

    adapter
        .set_playback_rate(1.0)
        .expect("cache release should be observed");
    assert_completed(
        adapter.take_command_progress().expect("completed progress"),
        command_id,
    );
}

#[test]
fn tracked_seek_requires_both_seek_end_and_position_tolerance() {
    let mut adapter = adapter_with_registered_observers(&[
        r#"{"event":"start-file","playlist_entry_id":2}"#,
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"event":"property-change","name":"seeking","data":false}"#,
        r#"{"event":"property-change","name":"time-pos","data":22.0}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"event":"property-change","name":"time-pos","data":20.4}"#,
        r#"{"request_id":4,"error":"success"}"#,
    ]);
    adapter
        .set_playback_rate(1.0)
        .expect("setup generation should be observed");

    let command_id = adapter
        .execute_tracked(PlayerCommand::SetPosition(20.0))
        .expect("seek should be accepted");
    assert_accepted(
        adapter.take_command_progress().expect("accepted progress"),
        command_id,
    );

    adapter
        .set_playback_rate(1.0)
        .expect("out-of-tolerance observations should be drained");
    assert_eq!(
        adapter.take_command_progress(),
        None,
        "seeking=false is insufficient while the position is outside tolerance"
    );

    adapter
        .set_playback_rate(1.0)
        .expect("in-tolerance position should be drained");
    let completed = adapter
        .take_command_progress()
        .expect("matching position should complete seek");
    assert_completed(completed, command_id);
    assert_eq!(completed.observed_position_seconds, Some(20.4));
}

#[test]
fn tracked_start_after_load_waits_for_logical_play_cache_release_restart_and_advancement() {
    let mut adapter = adapter_with_registered_observers(&[
        r#"{"event":"start-file","playlist_entry_id":3}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":false}"#,
        r#"{"event":"property-change","name":"pause","data":true}"#,
        r#"{"event":"property-change","name":"time-pos","data":10.0}"#,
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"event":"property-change","name":"pause","data":false}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":true}"#,
        r#"{"event":"playback-restart"}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":false}"#,
        r#"{"event":"property-change","name":"time-pos","data":10.02}"#,
        r#"{"request_id":4,"error":"success"}"#,
    ]);
    adapter
        .set_playback_rate(1.0)
        .expect("setup state should be observed");

    let command_id = adapter
        .execute_tracked(PlayerCommand::Play(PlayerPlayIntent::StartAfterLoad {
            baseline_restart_sequence: 0,
        }))
        .expect("play should be accepted");
    assert_accepted(
        adapter.take_command_progress().expect("accepted progress"),
        command_id,
    );

    adapter
        .set_playback_rate(1.0)
        .expect("restart while cache-paused should be observed");
    assert_eq!(
        adapter.take_command_progress(),
        None,
        "restart must not complete play while cache pause remains active"
    );

    adapter
        .set_playback_rate(1.0)
        .expect("cache release should be observed");
    assert_completed(
        adapter.take_command_progress().expect("completed progress"),
        command_id,
    );
}

#[test]
fn tracked_start_after_seek_requires_restart_followed_by_forward_position_advancement() {
    let mut adapter = adapter_with_registered_observers(&[
        r#"{"event":"start-file","playlist_entry_id":4}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":false}"#,
        r#"{"event":"property-change","name":"pause","data":true}"#,
        r#"{"event":"property-change","name":"time-pos","data":30.0}"#,
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"event":"property-change","name":"pause","data":false}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"event":"playback-restart"}"#,
        r#"{"event":"property-change","name":"time-pos","data":30.02}"#,
        r#"{"request_id":4,"error":"success"}"#,
    ]);
    adapter
        .set_playback_rate(1.0)
        .expect("setup state should be observed");

    let command_id = adapter
        .execute_tracked(PlayerCommand::Play(PlayerPlayIntent::StartAfterSeek {
            baseline_restart_sequence: 0,
        }))
        .expect("play should be accepted");
    assert_accepted(
        adapter.take_command_progress().expect("accepted progress"),
        command_id,
    );
    assert_eq!(
        adapter.take_command_progress(),
        None,
        "logical unpause without a restart and later advancement must not complete play"
    );
    adapter
        .set_playback_rate(1.0)
        .expect("logical unpause should be observed");
    assert_eq!(
        adapter.take_command_progress(),
        None,
        "logical unpause alone must not complete play"
    );
    adapter
        .set_playback_rate(1.0)
        .expect("restart followed by forward movement should be observed");
    assert_completed(
        adapter.take_command_progress().expect("completed progress"),
        command_id,
    );
}

#[test]
fn tracked_resume_completes_without_playback_restart_after_fresh_advancement() {
    let mut adapter = adapter_with_registered_observers(&[
        r#"{"event":"start-file","playlist_entry_id":8}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":false}"#,
        r#"{"event":"property-change","name":"pause","data":true}"#,
        r#"{"event":"property-change","name":"core-idle","data":true}"#,
        r#"{"event":"property-change","name":"time-pos","data":40.0}"#,
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"event":"property-change","name":"pause","data":false}"#,
        r#"{"event":"property-change","name":"core-idle","data":false}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"event":"property-change","name":"time-pos","data":40.02}"#,
        r#"{"request_id":4,"error":"success"}"#,
    ]);
    adapter
        .set_playback_rate(1.0)
        .expect("ready-paused setup observations should be drained");

    let mut latest = PlayerTransportTelemetryUpdate::default();
    while let Some(update) = adapter.take_transport_telemetry_update() {
        latest.merge_from(update);
    }
    assert_eq!(latest.phase, Some(PlayerTransportPhase::ReadyPaused));
    assert_eq!(latest.logical_pause, Some(true));
    assert_eq!(latest.paused_for_cache, Some(false));
    assert_eq!(latest.core_idle, Some(true));
    assert_eq!(latest.playback_restart_sequence, None);

    let command_id = adapter
        .execute_tracked(PlayerCommand::Play(PlayerPlayIntent::Resume))
        .expect("resume should be accepted");
    assert_accepted(
        adapter.take_command_progress().expect("accepted progress"),
        command_id,
    );
    assert_eq!(adapter.take_command_progress(), None);

    adapter
        .set_playback_rate(1.0)
        .expect("logical resume should be observed");
    assert_eq!(
        adapter.take_command_progress(),
        None,
        "logical resume without fresh advancement must remain pending"
    );

    adapter
        .set_playback_rate(1.0)
        .expect("fresh position advancement should be observed");
    assert_completed(
        adapter.take_command_progress().expect("completed progress"),
        command_id,
    );

    let mut restart_observed = false;
    while let Some(update) = adapter.take_transport_telemetry_update() {
        restart_observed |= update.playback_restart_sequence.is_some();
    }
    assert!(
        !restart_observed,
        "ordinary resume must not manufacture or require playback-restart"
    );
}

#[test]
fn tracked_start_after_load_honors_restart_observed_before_later_play_command() {
    let mut adapter = adapter_with_registered_observers(&[
        r#"{"event":"start-file","playlist_entry_id":9}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":false}"#,
        r#"{"event":"property-change","name":"pause","data":true}"#,
        r#"{"event":"property-change","name":"time-pos","data":50.0}"#,
        r#"{"event":"playback-restart"}"#,
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"event":"property-change","name":"pause","data":false}"#,
        r#"{"request_id":3,"error":"success"}"#,
        r#"{"event":"property-change","name":"time-pos","data":50.02}"#,
        r#"{"request_id":4,"error":"success"}"#,
    ]);
    adapter
        .set_playback_rate(1.0)
        .expect("paused load and its restart should be observed");

    let command_id = adapter
        .execute_tracked(PlayerCommand::Play(PlayerPlayIntent::StartAfterLoad {
            baseline_restart_sequence: 0,
        }))
        .expect("start after load should be accepted");
    assert_accepted(
        adapter.take_command_progress().expect("accepted progress"),
        command_id,
    );

    adapter
        .set_playback_rate(1.0)
        .expect("logical play should be observed");
    assert_eq!(
        adapter.take_command_progress(),
        None,
        "the pre-command restart still requires fresh post-command advancement"
    );

    adapter
        .set_playback_rate(1.0)
        .expect("post-command advancement should be observed");
    assert_completed(
        adapter.take_command_progress().expect("completed progress"),
        command_id,
    );
}

#[test]
fn tracked_load_waits_for_file_loaded_and_ready_phase() {
    let mut adapter = adapter_with_registered_observers(&[
        r#"{"event":"start-file","playlist_entry_id":5}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success","data":"https://media.invalid/video"}"#,
        r#"{"request_id":3,"error":"success","data":null}"#,
        r#"{"request_id":4,"error":"success","data":null}"#,
        r#"{"event":"playback-restart"}"#,
        r#"{"request_id":5,"error":"success"}"#,
    ]);

    let command_id = adapter
        .execute_tracked(PlayerCommand::OpenFile(
            "https://media.invalid/video".to_owned(),
        ))
        .expect("load should be accepted");
    assert_accepted(
        adapter.take_command_progress().expect("accepted progress"),
        command_id,
    );
    assert_eq!(
        adapter.take_command_progress(),
        None,
        "file-loaded alone must not complete an unready network load"
    );

    adapter
        .set_playback_rate(1.0)
        .expect("playback restart should be observed");
    assert_completed(
        adapter.take_command_progress().expect("completed progress"),
        command_id,
    );
}

#[test]
fn tracked_load_retains_buffered_ready_evidence_until_command_acceptance() {
    let target = "https://media.invalid/video";
    let mut adapter = adapter_with_registered_observers(&[
        r#"{"event":"start-file","playlist_entry_id":5}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":false}"#,
        r#"{"event":"property-change","name":"pause","data":true}"#,
        r#"{"event":"playback-restart"}"#,
        r#"{"event":"property-change","name":"seeking","data":true}"#,
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success","data":"https://media.invalid/video"}"#,
        r#"{"request_id":3,"error":"success","data":null}"#,
        r#"{"request_id":4,"error":"success","data":null}"#,
    ]);

    let command_id = adapter
        .execute_tracked(PlayerCommand::OpenFile(target.to_owned()))
        .expect("load should be accepted after buffered events are reduced");

    assert_eq!(adapter.transport_phase(), PlayerTransportPhase::Seeking);
    assert_accepted(
        adapter.take_command_progress().expect("accepted progress"),
        command_id,
    );
    assert_completed(
        adapter.take_command_progress().expect("completed progress"),
        command_id,
    );
    assert_eq!(adapter.take_command_progress(), None);
    assert_eq!(
        adapter
            .take_media_load_outcome()
            .expect("file-loaded should retain its successful load outcome"),
        PlayerMediaLoadOutcome::success(target, Some(target.to_owned()))
    );
}

#[test]
fn ordered_batch_includes_events_generated_by_final_local_file_poll() {
    let target = "https://media.invalid/video";
    let mut adapter = adapter_with_registered_observers(&[
        r#"{"event":"start-file","playlist_entry_id":5}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":false}"#,
        r#"{"event":"property-change","name":"pause","data":true}"#,
        r#"{"event":"playback-restart"}"#,
        r#"{"event":"property-change","name":"seeking","data":true}"#,
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success","data":"https://media.invalid/video"}"#,
        r#"{"event":"property-change","name":"time-pos","data":41.5}"#,
        r#"{"request_id":3,"error":"success","data":null}"#,
        r#"{"request_id":4,"error":"success","data":null}"#,
    ]);
    let command_id = adapter
        .execute_tracked(PlayerCommand::OpenFile(target.to_owned()))
        .expect("tracked load should be accepted");

    let batch = adapter
        .take_ordered_event_batch()
        .expect("mpv supports atomic ordered batches");

    assert!(
        batch
            .ordered_events
            .windows(2)
            .all(|events| events[0].sequence < events[1].sequence)
    );
    assert!(batch.ordered_events.iter().any(|event| matches!(
        event.kind,
        sorotte_player_api::PlayerOrderedEventKind::LocalFile(_)
    )));
    assert!(batch.ordered_events.iter().any(|event| matches!(
        event.kind,
        sorotte_player_api::PlayerOrderedEventKind::MediaLoad(_)
    )));
    assert!(batch.ordered_events.iter().any(|event| matches!(
        event.kind,
        sorotte_player_api::PlayerOrderedEventKind::Transport(_)
    )));
    let interleaved_transport = batch
        .ordered_events
        .iter()
        .position(|event| {
            matches!(
                event.kind,
                sorotte_player_api::PlayerOrderedEventKind::Transport(
                    PlayerTransportTelemetryUpdate {
                        position_seconds: Some(41.5),
                        ..
                    }
                )
            )
        })
        .expect("interleaved time-pos transport event");
    let local_file = batch
        .ordered_events
        .iter()
        .position(|event| {
            matches!(
                event.kind,
                sorotte_player_api::PlayerOrderedEventKind::LocalFile(_)
            )
        })
        .expect("derived local file event");
    let media_load = batch
        .ordered_events
        .iter()
        .position(|event| {
            matches!(
                event.kind,
                sorotte_player_api::PlayerOrderedEventKind::MediaLoad(_)
            )
        })
        .expect("derived media-load event");
    assert!(interleaved_transport < local_file);
    assert!(interleaved_transport < media_load);
    let observed_at = |kind: &sorotte_player_api::PlayerOrderedEventKind| match kind {
        sorotte_player_api::PlayerOrderedEventKind::CommandProgress(progress) => {
            progress.observed_at
        }
        sorotte_player_api::PlayerOrderedEventKind::LocalFile(observation) => {
            observation.observed_at
        }
        sorotte_player_api::PlayerOrderedEventKind::MediaLoad(observation) => {
            observation.observed_at
        }
        sorotte_player_api::PlayerOrderedEventKind::Transport(update) => update.observed_at,
    };
    let transport_observed_at = observed_at(&batch.ordered_events[interleaved_transport].kind)
        .expect("interleaved transport timestamp");
    for derived_index in [local_file, media_load] {
        assert!(
            transport_observed_at
                > observed_at(&batch.ordered_events[derived_index].kind)
                    .expect("derived media timestamp"),
            "later-sequenced derived media may retain the outer file-loaded timestamp"
        );
    }
    let command_progress: Vec<_> = batch
        .ordered_events
        .iter()
        .filter_map(|event| match event.kind {
            sorotte_player_api::PlayerOrderedEventKind::CommandProgress(progress)
                if progress.command_id == command_id =>
            {
                Some(progress.state)
            }
            _ => None,
        })
        .collect();
    assert!(command_progress.contains(&PlayerCommandProgressState::Accepted));
    assert!(
        command_progress
            .iter()
            .any(|state| matches!(state, PlayerCommandProgressState::Finished(_)))
    );
    assert_eq!(adapter.take_command_progress(), None);
    assert_eq!(adapter.take_transport_telemetry_update(), None);
    assert_eq!(adapter.take_media_load_observation(), None);
    assert_eq!(adapter.take_local_file_observation(), None);
}

#[test]
fn replacement_load_supersedes_obsolete_tracked_load() {
    let mut adapter = adapter_with_registered_observers(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success"}"#,
    ]);
    let first = adapter
        .execute_tracked(PlayerCommand::OpenFile("first.mkv".to_owned()))
        .expect("first load should be accepted");
    assert_accepted(
        adapter.take_command_progress().expect("first acceptance"),
        first,
    );

    let second = adapter
        .execute_tracked(PlayerCommand::OpenFile("second.mkv".to_owned()))
        .expect("replacement load should be accepted");
    assert_ne!(first, second);
    assert_accepted(
        adapter.take_command_progress().expect("second acceptance"),
        second,
    );
    let superseded = adapter
        .take_command_progress()
        .expect("obsolete load should terminate");
    assert_eq!(superseded.command_id, first);
    assert_eq!(
        superseded.state,
        PlayerCommandProgressState::Finished(PlayerCommandResult::Superseded)
    );
}

fn exercise_buffered_b_terminal_after_c_submission(reason: &str) {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"event":"start-file","playlist_entry_id":10}"#,
        r#"{"request_id":2,"error":"success","data":"a.mkv"}"#,
        r#"{"request_id":3,"error":"success","data":1200.0}"#,
        r#"{"request_id":4,"error":"success","data":4096}"#,
        r#"{"request_id":5,"error":"success","data":"a.mkv"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport_and_registered_observers(transport);
    adapter
        .open_file("a.mkv")
        .expect("A should become active before replacements");
    while adapter.take_transport_telemetry_update().is_some() {}
    while adapter.take_media_load_observation().is_some() {}
    while adapter.take_local_file_observation().is_some() {}
    while adapter.take_media_load_observation().is_some() {}

    let next_request_id = |state: &FakeTransportStateHandle| {
        state
            .writes()
            .iter()
            .filter_map(|write| {
                serde_json::from_str::<Value>(write)
                    .ok()?
                    .get("request_id")?
                    .as_u64()
            })
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    };
    let b_request_id = next_request_id(&state);
    let b_response = format!(r#"{{"request_id":{b_request_id},"error":"success"}}"#);
    state.queue_reads(&[&b_response]);
    let command_b = adapter
        .execute_tracked(PlayerCommand::OpenFile("b.mkv".to_owned()))
        .unwrap_or_else(|error| {
            panic!(
                "B should be accepted: {error:?}; writes: {:?}",
                state.writes()
            )
        });
    assert_accepted(
        adapter
            .take_command_progress()
            .expect("B acceptance should be reported"),
        command_b,
    );
    let generation_b = adapter
        .media_generation()
        .expect("B should retain its pending generation");
    adapter.inject_authoritative_playlist_snapshot_for_test(
        [
            (10, Some("a.mkv".to_owned()), false),
            (11, Some("b.mkv".to_owned()), true),
        ],
        Some("b.mkv".to_owned()),
    );

    let terminal = if reason == "error" {
        r#"{"event":"end-file","playlist_entry_id":11,"reason":"error","file_error":"B failed after C was accepted"}"#
    } else {
        r#"{"event":"end-file","playlist_entry_id":11,"reason":"stop"}"#
    };
    let c_request_id = next_request_id(&state);
    let c_response = format!(r#"{{"request_id":{c_request_id},"error":"success"}}"#);
    state.queue_reads(&[
        r#"{"event":"start-file","playlist_entry_id":11}"#,
        terminal,
        &c_response,
    ]);
    let command_c = adapter
        .execute_tracked(PlayerCommand::OpenFile("c.mkv".to_owned()))
        .expect("C should be accepted after binding its authoritative playlist entry");
    let generation_c = adapter
        .media_generation()
        .expect("C should remain the pending generation");
    assert_ne!(generation_b, generation_c);

    let mut replacement_progress = Vec::new();
    while let Some(progress) = adapter.take_command_progress() {
        replacement_progress.push(progress);
    }
    assert!(replacement_progress.iter().any(|progress| {
        progress.command_id == command_b
            && progress.state
                == PlayerCommandProgressState::Finished(PlayerCommandResult::Superseded)
    }));
    assert!(replacement_progress.iter().all(|progress| {
        progress.command_id != command_b
            || !matches!(
                progress.state,
                PlayerCommandProgressState::Finished(PlayerCommandResult::Failed(_))
            )
    }));
    assert!(replacement_progress.iter().any(|progress| {
        progress.command_id == command_c && progress.state == PlayerCommandProgressState::Accepted
    }));
    let terminal_updates =
        std::iter::from_fn(|| adapter.take_transport_telemetry_update()).collect::<Vec<_>>();
    assert!(terminal_updates.iter().all(|update| {
        update.media_generation != Some(generation_c)
            || !matches!(
                update.phase,
                Some(PlayerTransportPhase::Ended | PlayerTransportPhase::Failed)
            )
    }));
    if reason == "error" {
        assert_eq!(
            adapter.take_media_load_observation(),
            None,
            "a superseded physical episode must not publish a logical-generation load failure"
        );
    }

    let lifecycle_request_id = next_request_id(&state);
    let mut lifecycle_reads = vec![
        r#"{"event":"start-file","playlist_entry_id":12}"#.to_owned(),
        r#"{"event":"file-loaded"}"#.to_owned(),
        r#"{"event":"playback-restart"}"#.to_owned(),
        format!(r#"{{"request_id":{lifecycle_request_id},"error":"success"}}"#),
    ];
    lifecycle_reads.extend(
        (lifecycle_request_id.saturating_add(1)..=lifecycle_request_id.saturating_add(16)).map(
            |request_id| format!(r#"{{"request_id":{request_id},"error":"success","data":null}}"#),
        ),
    );
    state.queue_reads(
        &lifecycle_reads
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );
    adapter
        .set_playback_rate(1.0)
        .expect("C lifecycle should be reduced");
    let completion_progress =
        std::iter::from_fn(|| adapter.take_command_progress()).collect::<Vec<_>>();
    assert!(
        completion_progress.iter().any(|progress| {
            progress.command_id == command_c
                && progress.state
                    == PlayerCommandProgressState::Finished(PlayerCommandResult::Completed)
        }),
        "C did not complete; adapter: {adapter:?}; progress: {completion_progress:?}; writes: {:?}",
        state.writes()
    );
    assert!(
        completion_progress
            .iter()
            .all(|progress| progress.command_id != command_b)
    );
}

#[test]
fn buffered_b_error_after_c_submission_never_rewrites_c_or_b_command_ownership() {
    exercise_buffered_b_terminal_after_c_submission("error");
}

#[test]
fn buffered_b_stop_after_c_submission_never_publishes_a_c_terminal() {
    exercise_buffered_b_terminal_after_c_submission("stop");
}

#[test]
fn ambiguous_load_lifecycle_reacquires_playlist_ownership_on_later_maintenance() {
    let (transport, state) = fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
    state.synthesize_path_queries();
    let mut adapter = MpvAdapter::with_test_transport_and_registered_observers(transport);
    let command_b = adapter
        .execute_tracked(PlayerCommand::OpenFile("b.mkv".to_owned()))
        .expect("B should be accepted");
    assert_accepted(
        adapter
            .take_command_progress()
            .expect("B acceptance should be reported"),
        command_b,
    );
    let generation_b = adapter
        .media_generation()
        .expect("B should own a pending generation");

    let next_request_id = |state: &FakeTransportStateHandle| {
        state
            .writes()
            .iter()
            .filter_map(|write| {
                serde_json::from_str::<Value>(write)
                    .ok()?
                    .get("request_id")?
                    .as_u64()
            })
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    };
    let c_request_id = next_request_id(&state);
    let c_response = format!(r#"{{"request_id":{c_request_id},"error":"success"}}"#);
    state.queue_playlist_query_unavailable();
    state.queue_reads(&[&c_response]);
    let command_c = adapter
        .execute_tracked(PlayerCommand::OpenFile("c.mkv".to_owned()))
        .expect("C should be accepted despite the missing initial playlist snapshot");
    adapter.observe_load_ready_before_binding_for_test(999, "c.mkv");
    let generation_c = adapter
        .media_generation()
        .expect("C should remain the pending generation");
    assert_ne!(generation_b, generation_c);
    assert!(
        adapter.load_lifecycle_reacquisition_required_for_test(),
        "the missing playlist result and unknown start-file ID require adapter reconciliation"
    );

    // One failed authoritative maintenance attempt keeps adapter ownership unresolved. The
    // consumer may invoke several getters, but the adapter issues at most one query group per
    // maintenance cycle and backs off before retrying.
    let playlist_query_count = |state: &FakeTransportStateHandle| {
        state
            .writes()
            .iter()
            .filter(|write| {
                serde_json::from_str::<Value>(write)
                    .ok()
                    .and_then(|value| value.get("command").cloned())
                    .and_then(|command| command.as_array().cloned())
                    .is_some_and(|command| {
                        command.first().and_then(Value::as_str) == Some("get_property")
                            && command.get(1).and_then(Value::as_str) == Some("playlist")
                    })
            })
            .count()
    };
    state.queue_playlist_query_error();
    let queries_before_failed_snapshot = playlist_query_count(&state);
    adapter.force_load_lifecycle_reacquisition_due_for_test();
    let _ = adapter
        .take_ordered_event_batch()
        .expect("consumer batch should remain available during ownership ambiguity");
    let queries_after_failed_snapshot = playlist_query_count(&state);
    assert_eq!(
        queries_after_failed_snapshot,
        queries_before_failed_snapshot + 1,
        "one maintenance cycle should issue at most one playlist query"
    );
    let _ = adapter.take_transport_telemetry_update();
    let _ = adapter.take_cache_telemetry_update();
    let _ = adapter.take_local_file_update();
    let _ = adapter.take_media_load_outcome();
    assert_eq!(
        playlist_query_count(&state),
        queries_after_failed_snapshot,
        "subsequent getters must honor reacquisition backoff instead of flooding mpv"
    );
    assert!(
        adapter.load_lifecycle_reacquisition_required_for_test(),
        "consumer event replay must not clear unresolved physical load ownership"
    );

    // The successful retry supplies exact causal identity rather than inferring C from the
    // single pending request. The production maintenance path obtains the same evidence from
    // mpv's playlist/path snapshot.
    let lifecycle_request_id = next_request_id(&state);
    let lifecycle_reads = [
        format!(r#"{{"request_id":{lifecycle_request_id},"error":"success","data":"c.mkv"}}"#),
        format!(
            r#"{{"request_id":{},"error":"success","data":120.0}}"#,
            lifecycle_request_id.saturating_add(1)
        ),
        format!(
            r#"{{"request_id":{},"error":"success","data":123456}}"#,
            lifecycle_request_id.saturating_add(2)
        ),
    ];
    state.queue_reads(
        &lifecycle_reads
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );
    adapter.inject_authoritative_playlist_snapshot_for_test(
        [
            (10, Some("b.mkv".to_owned()), false),
            (999, Some("c.mkv".to_owned()), true),
        ],
        Some("c.mkv".to_owned()),
    );
    assert!(
        !adapter.load_lifecycle_reacquisition_required_for_test(),
        "a later authoritative playlist/path snapshot should reconcile ownership; adapter: {adapter:?}; writes: {:?}",
        state.writes()
    );
    assert!(
        adapter.has_load_transition_for_test(generation_b),
        "one snapshot must not retire B before its terminal event or accepted-attempt timeout; pending: {:?}",
        adapter.pending_load_transition_generations_for_test()
    );
    assert!(
        adapter.has_load_transition_for_test(generation_c),
        "the authoritative current playlist entry should bind to C"
    );

    let mut ordered_events = adapter
        .take_ordered_event_batch()
        .expect("C lifecycle should be observable after deferred file-loaded reconciliation")
        .ordered_events;
    if let Some(follow_up) = adapter.take_ordered_event_batch() {
        ordered_events.extend(follow_up.ordered_events);
    }
    let progress = ordered_events
        .into_iter()
        .filter_map(|event| match event.kind {
            PlayerOrderedEventKind::CommandProgress(progress) => Some(progress),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        progress.iter().any(|progress| {
            progress.command_id == command_c
                && progress.state
                    == PlayerCommandProgressState::Finished(PlayerCommandResult::Completed)
        }),
        "C should complete from its retained file-loaded lifecycle after exact binding: {progress:#?}; adapter: {adapter:?}; writes: {:#?}",
        state.writes()
    );
    assert_eq!(
        progress
            .iter()
            .filter(|progress| {
                progress.command_id == command_c
                    && progress.state
                        == PlayerCommandProgressState::Finished(PlayerCommandResult::Completed)
            })
            .count(),
        1,
        "retained file-loaded evidence must complete C exactly once: {progress:#?}"
    );
    assert!(progress.iter().all(|progress| {
        progress.command_id != command_b
            || !matches!(
                progress.state,
                PlayerCommandProgressState::Finished(PlayerCommandResult::Failed(_))
            )
    }));
}

#[test]
fn rejected_replacement_restores_the_previous_accepted_load_transition() {
    let mut adapter = adapter_with_registered_observers(&[
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"invalid parameter"}"#,
    ]);
    let first = adapter
        .execute_tracked(PlayerCommand::OpenFile("first.mkv".to_owned()))
        .expect("first load should be accepted");
    assert_accepted(
        adapter.take_command_progress().expect("first acceptance"),
        first,
    );
    let first_generation = adapter
        .media_generation()
        .expect("the first accepted load should remain pending");

    let error = adapter
        .execute_tracked(PlayerCommand::OpenFile("rejected.mkv".to_owned()))
        .expect_err("the replacement loadfile command should be rejected");

    assert!(matches!(error, PlayerError::OperationFailed { .. }));
    assert_eq!(adapter.media_generation(), Some(first_generation));
    assert_eq!(
        adapter.take_command_progress(),
        None,
        "rejecting C must neither fail nor supersede accepted B"
    );
}

#[test]
fn observed_media_failure_finishes_an_accepted_tracked_load() {
    let mut adapter = adapter_with_registered_observers(&[
        r#"{"event":"start-file","playlist_entry_id":6}"#,
        r#"{"event":"end-file","playlist_entry_id":6,"reason":"error","file_error":"network failed"}"#,
        r#"{"request_id":1,"error":"success"}"#,
    ]);

    let command_id = adapter
        .execute_tracked(PlayerCommand::OpenFile(
            "https://media.invalid/failure".to_owned(),
        ))
        .expect("the loadfile IPC command itself should be accepted");

    assert_accepted(
        adapter.take_command_progress().expect("accepted progress"),
        command_id,
    );
    let failed = adapter
        .take_command_progress()
        .expect("observed media failure should terminate the load");
    assert_eq!(failed.command_id, command_id);
    assert_eq!(
        failed.state,
        PlayerCommandProgressState::Finished(PlayerCommandResult::Failed(
            PlayerCommandFailureKind::MediaEnded
        ))
    );
}

#[test]
fn simulated_player_reports_observed_completion_for_tracked_commands() {
    let mut player = SimulatedPlayer::new();

    for command in [
        PlayerCommand::OpenFile("movie.mkv".to_owned()),
        PlayerCommand::Play(PlayerPlayIntent::StartAfterLoad {
            baseline_restart_sequence: 0,
        }),
        PlayerCommand::SetPaused(true),
        PlayerCommand::Play(PlayerPlayIntent::Resume),
        PlayerCommand::SetPaused(true),
        PlayerCommand::SetPosition(12.0),
        PlayerCommand::Play(PlayerPlayIntent::StartAfterSeek {
            baseline_restart_sequence: 1,
        }),
    ] {
        let command_debug = format!("{command:?}");
        let command_id = player
            .execute_tracked(command)
            .unwrap_or_else(|error| panic!("{command_debug} should execute: {error}"));
        assert_accepted(
            player
                .take_command_progress()
                .unwrap_or_else(|| panic!("{command_debug} should be accepted")),
            command_id,
        );
        assert_completed(
            player
                .take_command_progress()
                .unwrap_or_else(|| panic!("{command_debug} should complete from observation")),
            command_id,
        );
    }
}
