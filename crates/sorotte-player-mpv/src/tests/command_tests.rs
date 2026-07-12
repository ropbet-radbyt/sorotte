use super::*;
use sorotte_player_api::{
    PlayerCommandFailureKind, PlayerCommandProgress, PlayerCommandProgressState,
    PlayerCommandResult,
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
fn tracked_play_waits_for_logical_play_cache_release_and_restart() {
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
        .execute_tracked(PlayerCommand::SetPaused(false))
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
fn tracked_play_requires_restart_and_forward_position_advancement() {
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
        .execute_tracked(PlayerCommand::SetPaused(false))
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
        PlayerCommand::SetPosition(12.0),
        PlayerCommand::SetPaused(true),
        PlayerCommand::SetPaused(false),
    ] {
        let command_id = player
            .execute_tracked(command)
            .expect("simulated tracked command should execute");
        assert_accepted(
            player
                .take_command_progress()
                .expect("simulated acceptance"),
            command_id,
        );
        assert_completed(
            player
                .take_command_progress()
                .expect("simulated observed completion"),
            command_id,
        );
    }
}
