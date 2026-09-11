use super::player_delivery::PlayerDelivery;
use super::*;
use crate::adapter::collect_pending_player_delivery;
use crate::constants::MPV_PROPERTY_PAUSED_FOR_CACHE;
use sorotte_player_api::{
    PlayerCommandFailureKind, PlayerCommandSemanticResult, PlayerEvent, PlayerLoadAttemptResult,
    PlayerPlayIntent, PlayerTransportPhase, PlayerTransportSnapshot, SnapshotField,
};

fn adapter_with_registered_observers(lines: &[&str]) -> MpvAdapter {
    let (transport, _) = fake_transport_with_reads(lines);
    MpvAdapter::with_test_transport_and_registered_observers(transport)
}

fn assert_completed(delivery: &PlayerDelivery, command_id: sorotte_player_api::PlayerCommandId) {
    let outcomes = delivery.command_outcomes().collect::<Vec<_>>();
    assert_eq!(
        outcomes.len(),
        1,
        "one semantic terminal per command: {outcomes:?}"
    );
    assert_eq!(outcomes[0].command_id, command_id);
    assert_eq!(outcomes[0].result, PlayerCommandSemanticResult::Completed);
}

fn assert_transcript_does_not_retain_canary(
    transcript: &crate::transcript::MpvTranscript,
    canary: &str,
) {
    let retained = transcript
        .records()
        .iter()
        .any(|record| record.raw_json.to_string().contains(canary));
    let exported = transcript
        .to_json_lines()
        .expect("captured transcript should serialize");
    assert!(
        !retained && !exported.contains(canary),
        "sanitized lifecycle transcript retained the private canary"
    );
}

#[test]
fn actual_ipc_capture_redacts_json_encoded_client_message_payload() {
    let canary = "SOROTTE_PRIVATE_CLIENT_MESSAGE_CANARY_4f1a";
    let chat_event = format!(
        r#"{{"event":"client-message","args":["third-party-chat","{{\"text\":\"{canary}\"}}"]}}"#
    );
    let mut adapter = adapter_with_registered_observers(&[
        chat_event.as_str(),
        r#"{"request_id":1,"error":"success"}"#,
    ]);
    adapter.enable_lifecycle_transcript_capture();

    adapter
        .set_playback_rate(1.0)
        .expect("scripted IPC command should pump the captured chat event");
    let _ = collect_pending_player_delivery(&mut adapter);

    let transcript = adapter
        .take_lifecycle_transcript()
        .expect("enabled capture should return a transcript");
    assert!(
        transcript
            .records()
            .iter()
            .any(|record| record.event_name() == Some("client-message")),
        "the actual IPC event was not captured: {:?}",
        transcript.records()
    );
    assert_transcript_does_not_retain_canary(&transcript, canary);
}

#[test]
fn actual_ipc_capture_redacts_header_credentials_inside_event_arrays() {
    let canary = "SOROTTE_AUTH_HEADER_CANARY_b821";
    let header_event = format!(
        r#"{{"event":"client-message","args":["third-party-script","Authorization: Bearer {canary}"]}}"#
    );
    let mut adapter = adapter_with_registered_observers(&[
        header_event.as_str(),
        r#"{"request_id":1,"error":"success"}"#,
    ]);
    adapter.enable_lifecycle_transcript_capture();

    adapter
        .set_playback_rate(1.0)
        .expect("scripted IPC command should pump the captured header event");
    let _ = collect_pending_player_delivery(&mut adapter);

    let transcript = adapter
        .take_lifecycle_transcript()
        .expect("enabled capture should return a transcript");
    assert!(
        transcript
            .records()
            .iter()
            .any(|record| record.event_name() == Some("client-message")),
        "the actual IPC event was not captured: {:?}",
        transcript.records()
    );
    assert_transcript_does_not_retain_canary(&transcript, canary);
}

#[test]
fn actual_tracked_load_capture_is_explicitly_event_only() {
    let mut adapter = adapter_with_registered_observers(&[
        r#"{"event":"start-file","playlist_entry_id":5}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success","data":"https://media.invalid/capture"}"#,
        r#"{"request_id":3,"error":"success","data":120.0}"#,
        r#"{"request_id":4,"error":"success","data":4096}"#,
    ]);
    adapter.enable_lifecycle_transcript_capture();

    let command_id = adapter
        .execute_tracked(PlayerCommand::OpenFile(
            "https://media.invalid/capture".to_owned(),
        ))
        .expect("scripted tracked load should be accepted");
    let transcript = adapter
        .take_lifecycle_transcript()
        .expect("enabled capture should return a transcript");

    assert!(
        transcript
            .records()
            .iter()
            .any(|record| record.event_name() == Some("start-file"))
    );
    assert!(
        transcript
            .records()
            .iter()
            .any(|record| record.event_name() == Some("file-loaded"))
    );
    assert!(
        transcript.records().iter().all(|record| {
            record.raw_json.get("command").is_none()
                && !(record.raw_json.get("event").is_none()
                    && record.raw_json.get("request_id").is_some())
        }),
        "event capture must not imply outgoing-command or synchronous-response coverage: {:?}",
        transcript.records()
    );
    assert!(
        transcript
            .records()
            .iter()
            .all(|record| record.command_id != Some(command_id)),
        "the tracked command ID belongs to the uncaptured command/response boundary"
    );
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

    assert_eq!(
        collect_pending_player_delivery(&mut adapter)
            .command_outcomes()
            .filter(|outcome| outcome.command_id == command_id)
            .count(),
        0,
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

    assert_eq!(
        collect_pending_player_delivery(&mut adapter)
            .command_outcomes()
            .count(),
        0
    );

    adapter
        .set_playback_rate(1.0)
        .expect("logical pause observation should be drained");
    let completed_delivery = collect_pending_player_delivery(&mut adapter);
    assert_completed(&completed_delivery, command_id);
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

    adapter
        .set_playback_rate(1.0)
        .expect("cache-induced pause should be observed");
    assert_eq!(
        collect_pending_player_delivery(&mut adapter)
            .command_outcomes()
            .count(),
        0,
        "pause=true while paused-for-cache=true is not a logical-pause acknowledgement"
    );

    adapter
        .set_playback_rate(1.0)
        .expect("cache release should be observed");
    let completed_delivery = collect_pending_player_delivery(&mut adapter);
    assert_completed(&completed_delivery, command_id);
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

    adapter
        .set_playback_rate(1.0)
        .expect("out-of-tolerance observations should be drained");
    assert_eq!(
        collect_pending_player_delivery(&mut adapter)
            .command_outcomes()
            .count(),
        0,
        "seeking=false is insufficient while the position is outside tolerance"
    );

    adapter
        .set_playback_rate(1.0)
        .expect("in-tolerance position should be drained");
    let completed = collect_pending_player_delivery(&mut adapter);
    assert_completed(&completed, command_id);
    assert!(
        completed
            .transport_deltas()
            .any(|delta| delta.position_seconds == Some(20.4))
    );
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

    adapter
        .set_playback_rate(1.0)
        .expect("restart while cache-paused should be observed");
    assert_eq!(
        collect_pending_player_delivery(&mut adapter)
            .command_outcomes()
            .count(),
        0,
        "restart must not complete play while cache pause remains active"
    );

    adapter
        .set_playback_rate(1.0)
        .expect("cache release should be observed");
    let completed_delivery = collect_pending_player_delivery(&mut adapter);
    assert_completed(&completed_delivery, command_id);
}

#[test]
fn tracked_start_after_load_reacquires_unchanged_cache_release_after_transient_null() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"event":"start-file","playlist_entry_id":19}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":false}"#,
        r#"{"event":"property-change","name":"pause","data":true}"#,
        r#"{"event":"property-change","name":"core-idle","data":true}"#,
        r#"{"event":"property-change","name":"time-pos","data":12.0}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":null}"#,
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success"}"#,
        r#"{"request_id":3,"error":"success","data":false}"#,
        r#"{"event":"property-change","name":"pause","data":false}"#,
        r#"{"event":"playback-restart"}"#,
        r#"{"event":"property-change","name":"time-pos","data":12.02}"#,
        r#"{"request_id":4,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport_and_registered_observers(transport);
    adapter
        .set_playback_rate(1.0)
        .expect("ready-paused setup observations should be drained");

    let command_id = adapter
        .execute_tracked(PlayerCommand::Play(PlayerPlayIntent::StartAfterLoad {
            baseline_restart_sequence: 0,
        }))
        .expect("start after load should be accepted");

    assert_eq!(
        collect_pending_player_delivery(&mut adapter)
            .command_outcomes()
            .count(),
        0,
        "authoritative cache release still requires restart and advancement"
    );

    adapter
        .set_playback_rate(1.0)
        .expect("post-command playback evidence should be drained");
    let completed_delivery = collect_pending_player_delivery(&mut adapter);
    assert_completed(&completed_delivery, command_id);

    let writes = state.writes();
    let cache_pause_reads = writes
        .iter()
        .filter_map(|write| serde_json::from_str::<Value>(write).ok())
        .filter_map(|value| value.get("command").cloned())
        .filter_map(|command| command.as_array().cloned())
        .filter(|command| {
            command.first().and_then(Value::as_str) == Some(MPV_COMMAND_GET_PROPERTY)
                && command.get(1).and_then(Value::as_str) == Some(MPV_PROPERTY_PAUSED_FOR_CACHE)
        })
        .count();
    assert_eq!(
        cache_pause_reads, 1,
        "missing cache evidence should cause one bounded authoritative read"
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

    assert_eq!(
        collect_pending_player_delivery(&mut adapter)
            .command_outcomes()
            .count(),
        0,
        "logical unpause without a restart and later advancement must not complete play"
    );
    adapter
        .set_playback_rate(1.0)
        .expect("logical unpause should be observed");
    assert_eq!(
        collect_pending_player_delivery(&mut adapter)
            .command_outcomes()
            .count(),
        0,
        "logical unpause alone must not complete play"
    );
    adapter
        .set_playback_rate(1.0)
        .expect("restart followed by forward movement should be observed");
    let completed_delivery = collect_pending_player_delivery(&mut adapter);
    assert_completed(&completed_delivery, command_id);
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
    assert_eq!(latest.playback_restart_sequence, SnapshotField::Unavailable);

    let command_id = adapter
        .execute_tracked(PlayerCommand::Play(PlayerPlayIntent::Resume))
        .expect("resume should be accepted");

    assert_eq!(
        collect_pending_player_delivery(&mut adapter)
            .command_outcomes()
            .count(),
        0
    );

    adapter
        .set_playback_rate(1.0)
        .expect("logical resume should be observed");
    assert_eq!(
        collect_pending_player_delivery(&mut adapter)
            .command_outcomes()
            .count(),
        0,
        "logical resume without fresh advancement must remain pending"
    );

    adapter
        .set_playback_rate(1.0)
        .expect("fresh position advancement should be observed");
    let completed_delivery = collect_pending_player_delivery(&mut adapter);
    assert_completed(&completed_delivery, command_id);

    let mut restart_observed = false;
    for update in completed_delivery.transport_deltas() {
        restart_observed |= update.playback_restart_sequence.is_some();
    }
    assert!(
        !restart_observed,
        "ordinary resume must not manufacture or require playback-restart"
    );
}

#[test]
fn pending_play_harvests_post_response_events_without_an_unrelated_command() {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"event":"start-file","playlist_entry_id":18}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"event":"property-change","name":"paused-for-cache","data":false}"#,
        r#"{"event":"property-change","name":"pause","data":true}"#,
        r#"{"event":"property-change","name":"core-idle","data":true}"#,
        r#"{"event":"property-change","name":"time-pos","data":40.0}"#,
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success"}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport_and_registered_observers(transport);
    adapter
        .set_playback_rate(1.0)
        .expect("ready-paused setup observations should be drained");

    let command_id = adapter
        .execute_tracked(PlayerCommand::Play(PlayerPlayIntent::Resume))
        .expect("resume should be accepted");

    assert_eq!(
        collect_pending_player_delivery(&mut adapter)
            .command_outcomes()
            .count(),
        0,
        "the command response is not semantic completion"
    );

    // mpv can emit these observations just after the set_property response. They therefore
    // enter the socket only after the synchronous command has stopped reading it.
    state.queue_reads(&[
        r#"{"event":"property-change","name":"pause","data":false}"#,
        r#"{"event":"property-change","name":"core-idle","data":false}"#,
        r#"{"event":"property-change","name":"time-pos","data":40.02}"#,
        r#"{"request_id":3,"error":"success","data":false}"#,
    ]);
    adapter.force_ipc_event_fence_due_for_test();

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    let completed = loop {
        let delivery = collect_player_delivery(&mut adapter);
        if delivery.command_outcomes().next().is_some() {
            break delivery;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "event fence did not harvest the queued resume observations; writes: {:?}",
            state.writes()
        );
        std::thread::yield_now();
    };
    assert_completed(&completed, command_id);
    assert_eq!(
        collect_player_delivery(&mut adapter)
            .command_outcomes()
            .count(),
        0,
        "one accepted command must have exactly one semantic terminal"
    );

    let writes = state.writes();
    let property_queries = writes
        .iter()
        .filter_map(|write| serde_json::from_str::<Value>(write).ok())
        .filter_map(|value| value.get("command").cloned())
        .filter_map(|command| command.as_array().cloned())
        .filter(|command| command.first().and_then(Value::as_str) == Some("get_property"))
        .collect::<Vec<_>>();
    assert_eq!(
        property_queries.len(),
        2,
        "one event fence and one transport readback must share bounded maintenance"
    );
    assert_eq!(
        property_queries
            .iter()
            .filter_map(|command| command.get(1).and_then(Value::as_str))
            .collect::<Vec<_>>(),
        vec!["pause", "time-pos"]
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

    adapter
        .set_playback_rate(1.0)
        .expect("logical play should be observed");
    assert_eq!(
        collect_pending_player_delivery(&mut adapter)
            .command_outcomes()
            .count(),
        0,
        "the pre-command restart still requires fresh post-command advancement"
    );

    adapter
        .set_playback_rate(1.0)
        .expect("post-command advancement should be observed");
    let completed_delivery = collect_pending_player_delivery(&mut adapter);
    assert_completed(&completed_delivery, command_id);
}

#[test]
fn tracked_load_completes_on_owning_file_loaded_before_ready_phase() {
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

    let completed_delivery = collect_pending_player_delivery(&mut adapter);
    assert_completed(&completed_delivery, command_id);

    adapter
        .set_playback_rate(1.0)
        .expect("playback restart should be observed");
    assert_eq!(
        collect_pending_player_delivery(&mut adapter)
            .command_outcomes()
            .count(),
        0,
        "later readiness must not emit a duplicate load terminal"
    );
}

#[test]
fn active_media_harvests_end_file_without_a_pending_command() {
    let target = "https://media.invalid/active";
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"event":"start-file","playlist_entry_id":25}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success","data":"https://media.invalid/active"}"#,
        r#"{"request_id":3,"error":"success","data":120.0}"#,
        r#"{"request_id":4,"error":"success","data":4096}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport_and_registered_observers(transport);
    let command_id = adapter
        .execute_tracked(PlayerCommand::OpenFile(target.to_owned()))
        .expect("load should be accepted");

    let completed_delivery = collect_pending_player_delivery(&mut adapter);
    assert_completed(&completed_delivery, command_id);
    let generation = adapter
        .media_generation()
        .expect("the loaded file should remain the active physical attempt");
    let _ = collect_pending_player_delivery(&mut adapter);
    let writes_before_fence = state.writes().len();

    // With no tracked command left, an external end-file still has to leave the worker's socket
    // and terminate its exact active attempt.
    state.queue_reads(&[
        r#"{"event":"end-file","playlist_entry_id":25,"reason":"eof"}"#,
        r#"{"request_id":5,"error":"success","data":false}"#,
    ]);
    adapter.force_ipc_event_fence_due_for_test();

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    let terminal = loop {
        let delivery = collect_player_delivery(&mut adapter);
        if let Some(update) = delivery.transport_deltas().find(|delta| {
            delta.media_generation == Some(generation)
                && delta.phase == Some(PlayerTransportPhase::Ended)
        }) {
            break update.clone();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "active-media event fence did not harvest end-file; adapter: {adapter:?}; writes: {:?}",
            state.writes()
        );
        std::thread::yield_now();
    };
    assert_eq!(terminal.eof_reached, Some(true));
    assert_eq!(
        state.writes().len(),
        writes_before_fence + 1,
        "one active-media maintenance fence should harvest the terminal event"
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

    let completed_delivery = collect_pending_player_delivery(&mut adapter);
    assert_completed(&completed_delivery, command_id);
    assert_eq!(
        collect_pending_player_delivery(&mut adapter)
            .command_outcomes()
            .count(),
        0
    );
    assert_eq!(
        completed_delivery
            .load_outcomes()
            .map(|outcome| (
                &*outcome.requested_target,
                outcome.loaded_target.as_deref(),
                outcome.result
            ))
            .collect::<Vec<_>>(),
        vec![(target, None, PlayerLoadAttemptResult::Loaded)]
    );
}

#[test]
fn acknowledged_batch_includes_events_generated_by_file_metadata_readback() {
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
        .take_player_event_batch()
        .expect("acknowledged batch");
    assert!(
        batch
            .events
            .windows(2)
            .all(|events| events[0].order < events[1].order)
    );
    let transport = batch
        .events
        .iter()
        .find(|item| {
            matches!(&item.event,
                PlayerEvent::TransportDelta(delta) if delta.position_seconds == Some(41.5)
            )
        })
        .expect("interleaved time-pos event");
    let local_file = batch.events.iter().find(|item| matches!(&item.event,
        PlayerEvent::LocalFileChanged { update, .. } if update.path.as_deref() == Some(target)
    )).expect("derived local file");
    assert!(
        transport.order < local_file.order,
        "interleaved IPC ingress precedes the derived file publication"
    );
    let completed = batch.semantic_outcomes.iter().filter(|item| matches!(&item.outcome,
        sorotte_player_api::PlayerSemanticOutcome::Command(outcome) if outcome.command_id == command_id
            && outcome.result == PlayerCommandSemanticResult::Completed
    )).collect::<Vec<_>>();
    assert_eq!(completed.len(), 1);
    assert!(batch.semantic_outcomes.iter().any(|item| matches!(&item.outcome,
        sorotte_player_api::PlayerSemanticOutcome::LoadAttempt(outcome) if outcome.requested_target == target
            && outcome.result == PlayerLoadAttemptResult::Loaded
    )));
    adapter
        .acknowledge_player_event_batch(batch.acknowledgement_token)
        .expect("matching receipt");
    assert!(
        collect_pending_player_delivery(&mut adapter)
            .batches
            .is_empty()
    );
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

    let second = adapter
        .execute_tracked(PlayerCommand::OpenFile("second.mkv".to_owned()))
        .expect("replacement load should be accepted");
    assert_ne!(first, second);

    let delivery = collect_pending_player_delivery(&mut adapter);
    let outcomes = delivery.command_outcomes().collect::<Vec<_>>();
    assert_eq!(outcomes.len(), 1, "obsolete load should terminate");
    let superseded = outcomes[0];
    assert_eq!(superseded.command_id, first);
    assert_eq!(superseded.result, PlayerCommandSemanticResult::Superseded);
}

fn exercise_buffered_b_terminal_after_c_submission(reason: &str) {
    let (transport, state) = fake_transport_with_reads(&[
        r#"{"event":"start-file","playlist_entry_id":10}"#,
        r#"{"event":"file-loaded"}"#,
        r#"{"request_id":1,"error":"success"}"#,
        r#"{"request_id":2,"error":"success","data":"a.mkv"}"#,
        r#"{"request_id":3,"error":"success","data":1200.0}"#,
        r#"{"request_id":4,"error":"success","data":4096}"#,
    ]);
    let mut adapter = MpvAdapter::with_test_transport_and_registered_observers(transport);
    adapter
        .open_file("a.mkv")
        .expect("A should become active before replacements");
    let _ = collect_pending_player_delivery(&mut adapter);

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

    let replacement_delivery = collect_pending_player_delivery(&mut adapter);
    let replacement_progress = replacement_delivery.command_outcomes().collect::<Vec<_>>();
    assert!(replacement_progress.iter().any(|progress| {
        progress.command_id == command_b
            && progress.result == PlayerCommandSemanticResult::Superseded
    }));
    assert!(replacement_progress.iter().all(|progress| {
        progress.command_id != command_b
            || !matches!(progress.result, PlayerCommandSemanticResult::Failed(_))
    }));
    assert!(
        replacement_progress
            .iter()
            .all(|outcome| outcome.command_id != command_c),
        "C is accepted but has no completion evidence yet"
    );
    let terminal_updates = replacement_delivery.transport_deltas().collect::<Vec<_>>();
    assert!(terminal_updates.iter().all(|update| {
        update.media_generation != Some(generation_c)
            || !matches!(
                update.phase,
                Some(PlayerTransportPhase::Ended | PlayerTransportPhase::Failed)
            )
    }));
    if reason == "error" {
        assert_eq!(
            replacement_delivery
                .load_outcomes()
                .filter(|outcome| matches!(outcome.result, PlayerLoadAttemptResult::Failed(_)))
                .count(),
            0,
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
    let completion_delivery = collect_pending_player_delivery(&mut adapter);
    let completion_progress = completion_delivery.command_outcomes().collect::<Vec<_>>();
    assert!(
        completion_progress.iter().any(|progress| {
            progress.command_id == command_c
                && progress.result == PlayerCommandSemanticResult::Completed
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
    let _ = collect_player_delivery(&mut adapter);
    let queries_after_failed_snapshot = playlist_query_count(&state);
    assert_eq!(
        queries_after_failed_snapshot,
        queries_before_failed_snapshot + 1,
        "one maintenance cycle should issue at most one playlist query"
    );
    let _ = collect_player_delivery(&mut adapter);
    let _ = adapter.take_cache_telemetry_update();
    let _ = collect_player_delivery(&mut adapter);
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

    let completion_delivery = collect_player_delivery(&mut adapter);
    let progress = completion_delivery.command_outcomes().collect::<Vec<_>>();
    assert!(
        progress.iter().any(|progress| {
            progress.command_id == command_c
                && progress.result == PlayerCommandSemanticResult::Completed
        }),
        "C should complete from its retained file-loaded lifecycle after exact binding: {progress:#?}; adapter: {adapter:?}; writes: {:#?}",
        state.writes()
    );
    assert_eq!(
        progress
            .iter()
            .filter(|progress| {
                progress.command_id == command_c
                    && progress.result == PlayerCommandSemanticResult::Completed
            })
            .count(),
        1,
        "retained file-loaded evidence must complete C exactly once: {progress:#?}"
    );
    assert!(progress.iter().all(|progress| {
        progress.command_id != command_b
            || !matches!(progress.result, PlayerCommandSemanticResult::Failed(_))
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

    let first_generation = adapter
        .media_generation()
        .expect("the first accepted load should remain pending");

    let error = adapter
        .execute_tracked(PlayerCommand::OpenFile("rejected.mkv".to_owned()))
        .expect_err("the replacement loadfile command should be rejected");

    assert!(matches!(error, PlayerError::OperationFailed { .. }));
    assert_eq!(adapter.media_generation(), Some(first_generation));
    assert_eq!(
        collect_pending_player_delivery(&mut adapter)
            .command_outcomes()
            .filter(|outcome| outcome.command_id == first)
            .count(),
        0,
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

    let delivery = collect_pending_player_delivery(&mut adapter);
    let outcomes = delivery.command_outcomes().collect::<Vec<_>>();
    assert_eq!(
        outcomes.len(),
        1,
        "observed media failure should terminate the load"
    );
    let failed = outcomes[0];
    assert_eq!(failed.command_id, command_id);
    assert_eq!(
        failed.result,
        PlayerCommandSemanticResult::Failed(PlayerCommandFailureKind::MediaEnded)
    );
}

#[test]
fn simulated_player_reports_observed_completion_for_tracked_commands() {
    let mut player = MpvAdapter::simulated();

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

        let completed_delivery = collect_pending_player_delivery(&mut player);
        assert_completed(&completed_delivery, command_id);
    }
}
