use super::*;

fn duplicate_playlist() -> (MpvLifecycleVerificationHarness, Runtime) {
    let mut session = selected_session();
    session.apply_message_json(r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode1.mkv","episode2.mkv"],"user":"bob","sorottePlaylistEpoch":3},"playlistIndex":{"index":0,"user":"bob","sorottePlaylistEpoch":3}}}"#).unwrap();
    let (mut harness, mut runtime) = pending_loaded_playback(session, 239.65);
    apply_and_ack(&mut harness, &mut runtime, 1.0);
    (harness, runtime)
}

fn peer_select(runtime: &mut Runtime, index: i64) {
    runtime.session_mut().apply_message_json(&format!(r#"{{"Set":{{"playlistIndex":{{"index":{index},"user":"bob","sorottePlaylistEpoch":4}}}}}}"#)).unwrap();
}

fn complete_retained(
    harness: &mut MpvLifecycleVerificationHarness,
    runtime: &mut Runtime,
    now: f64,
) -> bool {
    retained_eof(harness);
    apply_and_ack(harness, runtime, now);
    runtime
        .run_advance_playlist_after_natural_completion()
        .unwrap()
}

fn snapshot_current(harness: &mut MpvLifecycleVerificationHarness) {
    harness.detect_event_gap();
    harness.apply_authoritative_snapshot(
        [LifecycleVerificationPlaylistEntry::new(
            77,
            Some("episode1.mkv".into()),
            true,
        )],
        Some("episode1.mkv".into()),
    );
}

#[test]
fn preceding_physical_eof_cannot_complete_a_peer_duplicate_selection_or_replay() {
    for selected in [0, 1] {
        for retained in [false, true] {
            let (mut harness, mut runtime) = duplicate_playlist();
            peer_select(&mut runtime, selected);
            if retained {
                retained_eof(&mut harness);
            } else {
                harness.ingest_decoded_mpv_json(
                    json!({"event":"end-file","playlist_entry_id":77,"reason":"eof"}),
                );
            }
            apply_and_ack(&mut harness, &mut runtime, 2.0);
            assert!(
                !runtime
                    .run_advance_playlist_after_natural_completion()
                    .unwrap(),
                "selected={selected}, retained={retained}"
            );
            assert_eq!(
                runtime.session().current_room_playlist().unwrap().index,
                Some(selected)
            );
            assert!(runtime.control().outbound_messages().is_empty());
        }
    }
}

#[test]
fn snapshot_of_the_predecessor_cannot_adopt_a_peer_duplicate_selection_or_replay() {
    for selected in [0, 1] {
        let (mut harness, mut runtime) = duplicate_playlist();
        peer_select(&mut runtime, selected);
        snapshot_current(&mut harness);
        apply_and_ack(&mut harness, &mut runtime, 2.0);
        assert!(!complete_retained(&mut harness, &mut runtime, 3.0));
        assert_eq!(
            runtime.session().current_room_playlist().unwrap().index,
            Some(selected)
        );
    }
}

#[test]
fn unselected_edit_preserves_the_playing_entry_completion() {
    let (mut harness, mut runtime) = duplicate_playlist();
    runtime.session_mut().apply_message_json(r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode1.mkv","episode3.mkv"],"user":"bob","sorottePlaylistEpoch":4}}}"#).unwrap();
    assert!(complete_retained(&mut harness, &mut runtime, 2.0));
    assert_eq!(
        runtime.session().current_room_playlist().unwrap().index,
        Some(1)
    );
}

#[test]
fn compound_reorder_preserves_the_unique_playing_entry_completion() {
    let (mut harness, mut runtime) = loaded_playback(239.65);
    runtime.session_mut().apply_message_json(r#"{"Set":{"playlistChange":{"files":["episode2.mkv","episode1.mkv","episode3.mkv"],"user":"bob","sorottePlaylistEpoch":3},"playlistIndex":{"index":1,"user":"bob","sorottePlaylistEpoch":3}}}"#).unwrap();
    assert!(!runtime.session().has_pending_playlist_index_reset_intent());
    assert!(complete_retained(&mut harness, &mut runtime, 2.0));
    assert_eq!(
        runtime.session().current_room_playlist().unwrap().index,
        Some(2)
    );
}

#[test]
fn real_seek_allows_the_retained_file_to_complete_a_new_replay_selection() {
    for selected in [0, 1] {
        let (mut harness, mut runtime) = duplicate_playlist();
        peer_select(&mut runtime, selected);
        let seek_command_id = accept_replay_seek(&mut harness, &mut runtime, 30.0);
        for event in [
            json!({"event":"seek"}),
            json!({"event":"property-change","name":"eof-reached","data":false}),
            json!({"event":"property-change","name":"time-pos","data":30.0}),
            json!({"event":"playback-restart"}),
            json!({"event":"property-change","name":"core-idle","data":false}),
            json!({"event":"property-change","name":"pause","data":false}),
            json!({"event":"property-change","name":"time-pos","data":239.65}),
        ] {
            harness.ingest_decoded_mpv_json(event);
        }
        let SnapshotField::Known(results) = harness.projection().terminal_command_results else {
            panic!("adapter command outcomes must be available");
        };
        assert_eq!(
            results.get(&seek_command_id),
            Some(&sorotte_player_api::PlayerCommandSemanticResult::Completed)
        );
        apply_and_ack(&mut harness, &mut runtime, 2.0);
        assert!(complete_retained(&mut harness, &mut runtime, 3.0));
        assert_eq!(
            runtime.session().current_room_playlist().unwrap().index,
            Some(selected + 1)
        );
    }
}

#[test]
fn successor_load_can_complete_the_new_duplicate_selection() {
    let (mut harness, mut runtime) = duplicate_playlist();
    peer_select(&mut runtime, 1);
    harness.accept_tracked_load("episode1.mkv", [77]);
    harness.apply_authoritative_snapshot(
        [LifecycleVerificationPlaylistEntry::new(
            88,
            Some("episode1.mkv".into()),
            true,
        )],
        Some("episode1.mkv".into()),
    );
    for event in [
        json!({"event":"start-file","playlist_entry_id":88}),
        json!({"event":"property-change","name":"path","data":"episode1.mkv"}),
        json!({"event":"property-change","name":"duration","data":240.0}),
        json!({"event":"file-loaded"}),
        json!({"event":"playback-restart"}),
        json!({"event":"property-change","name":"pause","data":false}),
        json!({"event":"property-change","name":"time-pos","data":239.65}),
    ] {
        harness.ingest_decoded_mpv_json(event);
    }
    apply_and_ack(&mut harness, &mut runtime, 2.0);
    assert!(complete_retained(&mut harness, &mut runtime, 3.0));
    assert_eq!(
        runtime.session().current_room_playlist().unwrap().index,
        Some(2)
    );
}

#[test]
fn initial_owned_snapshot_can_establish_completion_provenance() {
    let (mut harness, mut runtime) = pending_loaded_playback(selected_session(), 239.65);
    snapshot_current(&mut harness);
    apply_and_ack(&mut harness, &mut runtime, 1.0);
    assert!(complete_retained(&mut harness, &mut runtime, 2.0));
}

#[test]
fn direct_playback_completion_cannot_acquire_a_later_playlist() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .unwrap();
    let (mut harness, mut runtime) = pending_loaded_playback(session, 239.65);
    apply_and_ack(&mut harness, &mut runtime, 1.0);
    assert!(!complete_retained(&mut harness, &mut runtime, 2.0));
    runtime.session_mut().apply_message_json(r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"bob","sorottePlaylistEpoch":1},"playlistIndex":{"index":0,"user":"bob","sorottePlaylistEpoch":1}}}"#).unwrap();
    assert!(!runtime.confirm_initial_playlist_selection_for_current_player());
    assert!(
        !runtime
            .run_advance_playlist_after_natural_completion()
            .unwrap()
    );
}

#[test]
fn confirmed_direct_file_can_join_the_first_canonical_selection_before_eof() {
    let (mut harness, mut runtime) = pending_loaded_playback(ClientSession::default(), 239.65);
    apply_and_ack(&mut harness, &mut runtime, 1.0);
    runtime
        .session_mut()
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .unwrap();
    runtime.session_mut().apply_message_json(r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"bob","sorottePlaylistEpoch":1},"playlistIndex":{"index":0,"user":"bob","sorottePlaylistEpoch":1}}}"#).unwrap();
    assert!(runtime.confirm_initial_playlist_selection_for_current_player());
    assert!(!runtime.confirm_initial_playlist_selection_for_current_player());
    assert!(complete_retained(&mut harness, &mut runtime, 2.0));
}

#[test]
fn first_selection_confirmation_cannot_rebind_a_peer_replay() {
    let (mut harness, mut runtime) = duplicate_playlist();
    peer_select(&mut runtime, 0);
    assert!(!runtime.confirm_initial_playlist_selection_for_current_player());
    assert!(!complete_retained(&mut harness, &mut runtime, 2.0));
}

#[test]
fn same_generation_recovery_preserves_the_original_selection_provenance() {
    for peer_changed_selection in [false, true] {
        let (mut harness, mut runtime) = duplicate_playlist();
        let SnapshotField::Known(generation) = harness.projection().physical_media_generation
        else {
            panic!("fixture must have owned physical media");
        };
        if peer_changed_selection {
            peer_select(&mut runtime, 1);
        }
        harness.accept_same_generation_recovery(generation, "episode1.mkv", [77]);
        harness.apply_authoritative_snapshot(
            [LifecycleVerificationPlaylistEntry::new(
                88,
                Some("episode1.mkv".into()),
                true,
            )],
            Some("episode1.mkv".into()),
        );
        for event in [
            json!({"event":"start-file","playlist_entry_id":88}),
            json!({"event":"property-change","name":"path","data":"episode1.mkv"}),
            json!({"event":"property-change","name":"duration","data":240.0}),
            json!({"event":"file-loaded"}),
            json!({"event":"playback-restart"}),
            json!({"event":"property-change","name":"pause","data":false}),
            json!({"event":"property-change","name":"time-pos","data":239.65}),
        ] {
            harness.ingest_decoded_mpv_json(event);
        }
        apply_and_ack(&mut harness, &mut runtime, 2.0);
        assert_eq!(
            complete_retained(&mut harness, &mut runtime, 3.0),
            !peer_changed_selection
        );
    }
}

struct TrackedLoadPlayer(sorotte_player_api::PlayerCommandId);

impl sorotte_player_api::PlayerAdapter for TrackedLoadPlayer {
    fn name(&self) -> &'static str {
        "completion-load-receipt"
    }

    fn execute(
        &mut self,
        _: sorotte_player_api::PlayerCommand,
    ) -> Result<(), sorotte_player_api::PlayerError> {
        Ok(())
    }

    fn execute_tracked(
        &mut self,
        command: sorotte_player_api::PlayerCommand,
    ) -> Result<sorotte_player_api::PlayerCommandId, sorotte_player_api::PlayerError> {
        match command {
            sorotte_player_api::PlayerCommand::OpenFile(_) => Ok(self.0),
            _ => Err(sorotte_player_api::PlayerError::Unsupported(
                "execute_tracked",
            )),
        }
    }
}

#[test]
fn delayed_physical_load_delivery_retains_the_submitted_selection() {
    for snapshot_only in [false, true] {
        let (mut harness, _) = pending_loaded_playback(selected_session(), 239.65);
        let command_id = harness
            .projection()
            .attempts
            .values()
            .find_map(|attempt| attempt.command_id)
            .unwrap();
        let mut runtime = ClientRuntime::new(
            selected_session(),
            TrackedLoadPlayer(command_id),
            QueuedRuntimeControl::default(),
        );
        runtime.player_mut().open_file("episode1.mkv").unwrap();
        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"playlistIndex":{"index":0,"user":"bob","sorottePlaylistEpoch":3}}}"#,
            )
            .unwrap();
        if snapshot_only {
            snapshot_current(&mut harness);
        }
        apply_and_ack(&mut harness, &mut runtime, 1.0);
        retained_eof(&mut harness);
        apply_and_ack(&mut harness, &mut runtime, 2.0);
        assert!(
            !runtime
                .run_advance_playlist_after_natural_completion()
                .unwrap(),
            "snapshot_only={snapshot_only}"
        );
        assert_eq!(
            runtime.session().current_room_playlist().unwrap().index,
            Some(0)
        );
    }
}

fn queue_observed_seek(harness: &mut MpvLifecycleVerificationHarness, target: f64) {
    for event in [
        json!({"event":"seek"}),
        json!({"event":"property-change","name":"eof-reached","data":false}),
        json!({"event":"property-change","name":"time-pos","data":target}),
        json!({"event":"playback-restart"}),
        json!({"event":"property-change","name":"core-idle","data":false}),
        json!({"event":"property-change","name":"pause","data":false}),
        json!({"event":"property-change","name":"time-pos","data":239.65}),
    ] {
        harness.ingest_decoded_mpv_json(event);
    }
}

#[test]
fn queued_predecessor_seek_cannot_adopt_a_later_replay_even_after_reset_dispatch() {
    for dispatch_replay_reset in [false, true] {
        let (mut harness, mut runtime) = duplicate_playlist();
        queue_observed_seek(&mut harness, 30.0);
        peer_select(&mut runtime, 0);
        if dispatch_replay_reset {
            accept_replay_seek(&mut harness, &mut runtime, 0.0);
            assert!(
                runtime
                    .session_mut()
                    .mark_pending_playlist_index_reset_physical_effect_applied(1)
            );
            assert!(
                runtime
                    .session_mut()
                    .take_pending_playlist_index_reset_intent()
                    .is_some()
            );
        }
        apply_and_ack(&mut harness, &mut runtime, 2.0);
        harness.ingest_decoded_mpv_json(
            json!({"event":"end-file","playlist_entry_id":77,"reason":"eof"}),
        );
        apply_and_ack(&mut harness, &mut runtime, 3.0);
        assert!(
            !runtime
                .run_advance_playlist_after_natural_completion()
                .unwrap(),
            "dispatched={dispatch_replay_reset}"
        );
        assert_eq!(
            runtime.session().current_room_playlist().unwrap().index,
            Some(0)
        );
    }
}

#[test]
fn unobserved_or_failed_seek_receipt_cannot_establish_replay_completion() {
    use sorotte_player_api::{PlayerCommandFailureKind, PlayerCommandSemanticResult};
    for (failure, expected) in [
        (
            PlayerCommandFailureKind::TimedOut,
            PlayerCommandSemanticResult::CompletionNotObserved,
        ),
        (
            PlayerCommandFailureKind::TransportDisconnected,
            PlayerCommandSemanticResult::TransportDisconnected,
        ),
    ] {
        let (mut harness, mut runtime) = duplicate_playlist();
        peer_select(&mut runtime, 0);
        let command_id = accept_replay_seek(&mut harness, &mut runtime, 30.0);
        harness.fail_tracked_command(command_id, failure);
        let SnapshotField::Known(results) = harness.projection().terminal_command_results else {
            panic!("terminal outcomes");
        };
        assert_eq!(results.get(&command_id), Some(&expected));
        queue_observed_seek(&mut harness, 30.0);
        apply_and_ack(&mut harness, &mut runtime, 2.0);
        assert!(!complete_retained(&mut harness, &mut runtime, 3.0));
    }
}

#[test]
fn completed_replay_seek_is_retained_when_a_later_seek_fails_before_drain() {
    let (mut harness, mut runtime) = duplicate_playlist();
    peer_select(&mut runtime, 0);
    let completed = accept_replay_seek(&mut harness, &mut runtime, 30.0);
    queue_observed_seek(&mut harness, 30.0);
    let failed = accept_replay_seek(&mut harness, &mut runtime, 0.0);
    harness.fail_tracked_command(
        failed,
        sorotte_player_api::PlayerCommandFailureKind::TimedOut,
    );
    let SnapshotField::Known(results) = harness.projection().terminal_command_results else {
        panic!("terminal outcomes");
    };
    assert_eq!(
        results.get(&completed),
        Some(&sorotte_player_api::PlayerCommandSemanticResult::Completed)
    );
    assert_eq!(
        results.get(&failed),
        Some(&sorotte_player_api::PlayerCommandSemanticResult::CompletionNotObserved)
    );
    apply_and_ack(&mut harness, &mut runtime, 2.0);
    assert!(complete_retained(&mut harness, &mut runtime, 3.0));
}
