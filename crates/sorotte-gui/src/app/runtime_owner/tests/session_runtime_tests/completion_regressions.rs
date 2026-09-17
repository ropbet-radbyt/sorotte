use super::*;
use crate::app::testing::support::runtime_state_for_shell;
use serde_json::{Value, json};
use sorotte_player_api::{PlayerError, PlayerEventAcknowledgementToken, PlayerEventBatch};
use sorotte_player_mpv::{LifecycleVerificationPlaylistEntry, MpvLifecycleVerificationHarness};

#[derive(Default)]
struct CompletionPlayer {
    harness: MpvLifecycleVerificationHarness,
    applied_pauses: Vec<bool>,
    applied_positions: Vec<f64>,
}

struct CompletionAdapter(Arc<Mutex<CompletionPlayer>>);

impl PlayerAdapter for CompletionAdapter {
    fn name(&self) -> &'static str {
        "completion-mpv-ingress"
    }
    fn set_paused(&mut self, paused: bool) -> Result<(), PlayerError> {
        self.0.lock().unwrap().applied_pauses.push(paused);
        Ok(())
    }
    fn set_position(&mut self, position: f64) -> Result<(), PlayerError> {
        self.0.lock().unwrap().applied_positions.push(position);
        Ok(())
    }
    fn execute_tracked(
        &mut self,
        command: sorotte_player_api::PlayerCommand,
    ) -> Result<sorotte_player_api::PlayerCommandId, PlayerError> {
        match command {
            sorotte_player_api::PlayerCommand::OpenFile(path) => Ok(self
                .0
                .lock()
                .unwrap()
                .harness
                .accept_tracked_load(path, [])
                .command_id),
            _ => Err(PlayerError::Unsupported("execute_tracked")),
        }
    }
    fn take_player_event_batch(&mut self) -> Option<PlayerEventBatch> {
        self.0.lock().unwrap().harness.take_event_batch()
    }
    fn acknowledge_player_event_batch(
        &mut self,
        token: PlayerEventAcknowledgementToken,
    ) -> Result<(), PlayerError> {
        self.0.lock().unwrap().harness.acknowledge(token)
    }
}

#[derive(Debug)]
struct CompletionResult {
    advances: usize,
    selected: Option<i64>,
    requests: Vec<Value>,
    applied_pauses: Vec<bool>,
    applied_positions: Vec<f64>,
    projected_position: Option<f64>,
    terminal_positions: Vec<f64>,
}

#[derive(Debug, Clone, Copy)]
enum EndEvidence {
    None,
    MatchedEndFile,
    KeepOpen,
    Stop,
    KeepOpenThenSeekBack,
    KeepOpenThenPeerSelection,
    KeepOpenThenSameRowReplay,
    KeepOpenLastRow,
}

#[derive(Debug, Clone, Copy)]
enum CompletionSource {
    SelectedName,
    MediaMatch,
    MediaMatchAlreadyPlaying,
    UnrelatedFile,
    MediaMatchThenPeerSelection,
}

fn gui_completion(
    position: f64,
    paused: bool,
    end_evidence: EndEvidence,
    offset: f64,
    looping: bool,
) -> CompletionResult {
    gui_completion_with_source(
        position,
        paused,
        end_evidence,
        offset,
        looping,
        CompletionSource::SelectedName,
    )
}

fn gui_completion_with_source(
    position: f64,
    paused: bool,
    end_evidence: EndEvidence,
    offset: f64,
    looping: bool,
    source: CompletionSource,
) -> CompletionResult {
    let mut session = crate::app::GuiClientSession::new("alice", "room1");
    session.deliver_outbound_protocol_lines().unwrap();
    session.apply_message_json(r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"sharedPlaylists":true,"chat":true,"readiness":true,"sorottePlaybackBarrierV1":true,"sorotteReadinessV2":true}}}"#).unwrap();
    let files = if looping || matches!(end_evidence, EndEvidence::KeepOpenLastRow) {
        json!(["episode1.mkv"])
    } else {
        json!(["episode1.mkv", "episode2.mkv"])
    };
    session
        .apply_message_json(
            &json!({"Set":{"playlistChange":{"files":files,"user":"alice","sorottePlaylistEpoch":1}}}).to_string(),
        )
        .unwrap();
    session
        .apply_message_json(
            r#"{"Set":{"playlistIndex":{"index":0,"user":"alice","sorottePlaylistEpoch":2}}}"#,
        )
        .unwrap();
    session
        .apply_message_json(
            r#"{"Set":{"user":{"alice":{"file":{"name":"episode1.mkv","duration":240.0}}}}}"#,
        )
        .unwrap();
    let advances = Arc::new(Mutex::new(0));
    let observed_advances = advances.clone();
    let session = session.with_observer(move |event| {
        if matches!(
            event,
            crate::app::runtime_stack::test_support::SessionObservation::PlaylistAdvance
        ) {
            *observed_advances.lock().unwrap() += 1;
        }
    });
    let player = Arc::new(Mutex::new(CompletionPlayer::default()));
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_session_runtime(Box::new(session));
    owner.player = Some(GuiOwnedPlayer::Custom(Box::new(CompletionAdapter(
        player.clone(),
    ))));
    owner.user_offset_seconds = offset;
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        shared_playlist_enabled: Some(true),
        loop_single_files: Some(looping),
        ..Default::default()
    });
    state.apply_shared_playlist_entries(
        files
            .as_array()
            .unwrap()
            .iter()
            .map(|file| file.as_str().unwrap().to_owned())
            .collect(),
        Some(0),
        false,
    );
    // Establish the connected shell's row identities before selecting a source.
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    for action in handle.drain_actions() {
        state.apply(action);
    }
    let physical_target = if matches!(source, CompletionSource::SelectedName) {
        "episode1.mkv"
    } else {
        "episode1-alternate-encode.mkv"
    };
    if matches!(
        source,
        CompletionSource::MediaMatch
            | CompletionSource::MediaMatchAlreadyPlaying
            | CompletionSource::MediaMatchThenPeerSelection
    ) {
        assert_eq!(
            owner.open_media_match_resolution_candidate_for_test(
                &runtime_state_for_shell(&state),
                physical_target.to_owned(),
            ),
            crate::app::runtime_owner::player::SelectedPlaylistMediaSyncOutcome::StartedLoading
        );
    } else {
        player
            .lock()
            .unwrap()
            .harness
            .accept_tracked_load(physical_target, []);
    }
    {
        let mut player = player.lock().unwrap();
        let harness = &mut player.harness;
        harness.apply_authoritative_snapshot(
            [LifecycleVerificationPlaylistEntry::new(
                77,
                Some(physical_target.into()),
                true,
            )],
            Some(physical_target.into()),
        );
        for event in [
            json!({"event":"start-file","playlist_entry_id":77}),
            json!({"event":"property-change","name":"path","data":physical_target}),
            json!({"event":"property-change","name":"duration","data":240.0}),
            json!({"event":"file-loaded"}),
            json!({"event":"playback-restart"}),
            json!({"event":"property-change","name":"pause","data":false}),
            json!({"event":"property-change","name":"paused-for-cache","data":false}),
            json!({"event":"property-change","name":"eof-reached","data":false}),
            json!({"event":"property-change","name":"core-idle","data":false}),
            json!({"event":"property-change","name":"time-pos","data":30.0}),
        ] {
            harness.ingest_decoded_mpv_json(event);
        }
    }
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    handle.drain_actions();
    assert_eq!(
        *advances.lock().unwrap(),
        0,
        "fixture must not already advance"
    );
    owner
        .session
        .as_mut()
        .unwrap()
        .deliver_outbound_protocol_lines()
        .unwrap();
    player.lock().unwrap().applied_positions.clear();
    player.lock().unwrap().applied_pauses.clear();
    if matches!(source, CompletionSource::MediaMatchAlreadyPlaying) {
        assert_eq!(owner.open_media_match_resolution_candidate_for_test(
            &runtime_state_for_shell(&state), physical_target.to_owned(),
        ), crate::app::runtime_owner::player::SelectedPlaylistMediaSyncOutcome::MatchedCurrentTarget);
    }
    if matches!(source, CompletionSource::MediaMatchThenPeerSelection) {
        owner
            .session
            .as_mut()
            .unwrap()
            .apply_message_json(
                r#"{"Set":{"playlistIndex":{"index":1,"user":"bob","sorottePlaylistEpoch":3}}}"#,
            )
            .unwrap();
        state.apply_shared_playlist_entries(
            vec!["episode1.mkv".into(), "episode2.mkv".into()],
            Some(1),
            false,
        );
    }
    if matches!(
        source,
        CompletionSource::MediaMatch | CompletionSource::MediaMatchAlreadyPlaying
    ) {
        let resolution = owner.playlist_resolution_attempt.as_ref().unwrap();
        assert_eq!(
            resolution.state,
            crate::app::runtime_owner::player::PlaylistResolutionAttemptState::Active
        );
        assert_eq!(
            resolution.candidate_provider,
            Some(crate::app::shell_state::GuiMediaSourceProviderId::media_matching())
        );
        assert!(resolution.player_media_generation.is_some());
        assert!(resolution.load_attempt_id.is_some());
    }
    {
        let mut player = player.lock().unwrap();
        if matches!(end_evidence, EndEvidence::None) {
            player
                .harness
                .ingest_decoded_mpv_json(json!({"event":"seek"}));
        }
        if matches!(
            end_evidence,
            EndEvidence::KeepOpen
                | EndEvidence::KeepOpenThenSeekBack
                | EndEvidence::KeepOpenThenPeerSelection
                | EndEvidence::KeepOpenThenSameRowReplay
                | EndEvidence::KeepOpenLastRow
        ) {
            player.harness.ingest_decoded_mpv_json(
                json!({"event":"property-change","name":"time-pos","data":position - 0.35}),
            );
            player.harness.ingest_decoded_mpv_json(
                json!({"event":"property-change","name":"eof-reached","data":true}),
            );
            player.harness.ingest_decoded_mpv_json(
                json!({"event":"property-change","name":"core-idle","data":true}),
            );
        }
        player.harness.ingest_decoded_mpv_json(
            json!({"event":"property-change","name":"time-pos","data":position}),
        );
        if matches!(end_evidence, EndEvidence::None) {
            player.harness.ingest_decoded_mpv_json(
                json!({"event":"property-change","name":"core-idle","data":paused}),
            );
        }
        player.harness.ingest_decoded_mpv_json(
            json!({"event":"property-change","name":"pause","data":paused}),
        );
        if matches!(end_evidence, EndEvidence::None) {
            player
                .harness
                .ingest_decoded_mpv_json(json!({"event":"playback-restart"}));
            player.harness.ingest_decoded_mpv_json(
                json!({"event":"property-change","name":"time-pos","data":position}),
            );
        }
        if matches!(
            end_evidence,
            EndEvidence::MatchedEndFile | EndEvidence::Stop
        ) {
            let reason = if matches!(end_evidence, EndEvidence::Stop) {
                "stop"
            } else {
                "eof"
            };
            player.harness.ingest_decoded_mpv_json(
                json!({"event":"end-file","playlist_entry_id":77,"reason":reason}),
            );
        }
        if matches!(end_evidence, EndEvidence::KeepOpenThenSeekBack) {
            for event in [
                json!({"event":"seek"}),
                json!({"event":"property-change","name":"eof-reached","data":false}),
                json!({"event":"property-change","name":"time-pos","data":30.0}),
                json!({"event":"playback-restart"}),
                json!({"event":"property-change","name":"pause","data":false}),
            ] {
                player.harness.ingest_decoded_mpv_json(event);
            }
        }
    }
    if matches!(
        end_evidence,
        EndEvidence::KeepOpenThenPeerSelection | EndEvidence::KeepOpenThenSameRowReplay
    ) {
        // The owner has accepted completion, but room authority changes before
        // the next projection cycle can request playlist progression.
        owner.refresh_player_state_impl();
        let index = i64::from(matches!(
            end_evidence,
            EndEvidence::KeepOpenThenPeerSelection
        ));
        owner.session.as_mut().unwrap().apply_message_json(
            &json!({"Set":{"playlistIndex":{"index":index,"user":"bob","sorottePlaylistEpoch":3}}}).to_string()
        ).unwrap();
    }
    GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
    handle.drain_actions();
    let session = owner.session.as_mut().unwrap();
    let selected = session.projected_current_room_playlist().unwrap().index;
    let messages = session
        .deliver_outbound_protocol_lines()
        .unwrap()
        .into_iter()
        .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
        .collect::<Vec<_>>();
    let terminal_positions = messages
        .iter()
        .filter_map(|value| {
            let playstate = value.get("State")?.get("playstate")?;
            (playstate.get("paused")?.as_bool()?
                && !playstate
                    .get("doSeek")
                    .and_then(Value::as_bool)
                    .unwrap_or(false))
            .then(|| playstate.get("position").and_then(Value::as_f64))
            .flatten()
        })
        .collect();
    let requests = messages
        .into_iter()
        .filter_map(|value| {
            value
                .get("Set")
                .and_then(|set| set.get("playlistIndex"))
                .cloned()
        })
        .collect();
    let player = player.lock().unwrap();
    assert_eq!(
        owner.player_local_file.as_ref().unwrap().name,
        physical_target,
        "completion projection must preserve the published physical filename"
    );
    CompletionResult {
        advances: *advances.lock().unwrap(),
        selected,
        requests,
        applied_pauses: player.applied_pauses.clone(),
        applied_positions: player.applied_positions.clone(),
        projected_position: owner.player_position_seconds,
        terminal_positions,
    }
}

fn assert_guarded_advance(result: &CompletionResult, index: i64) {
    assert_eq!(result.advances, 1);
    assert_eq!(result.requests.len(), 1);
    assert_eq!(result.requests[0]["index"], index);
    assert_eq!(result.requests[0]["sorotteExpectedPlaylistIndex"], 0);
    assert_eq!(result.requests[0]["sorotteExpectedPlaylistEpoch"], 2);
}

#[test]
fn media_match_alternate_basename_natural_completion_advances() {
    for source in [
        CompletionSource::MediaMatch,
        CompletionSource::MediaMatchAlreadyPlaying,
    ] {
        for evidence in [EndEvidence::KeepOpen, EndEvidence::MatchedEndFile] {
            for offset in [0.0, 10.0] {
                let result =
                    gui_completion_with_source(240.0, true, evidence, offset, false, source);
                assert_eq!(
                    result.selected,
                    Some(1),
                    "{source:?}, {evidence:?}, offset={offset}"
                );
                assert_guarded_advance(&result, 1);
            }
        }
    }
}

#[test]
fn unrelated_alternate_basename_natural_completion_does_not_advance() {
    let result = gui_completion_with_source(
        240.0,
        true,
        EndEvidence::KeepOpen,
        0.0,
        false,
        CompletionSource::UnrelatedFile,
    );
    assert_eq!(result.advances, 0);
    assert_eq!(result.selected, Some(0));
    assert!(result.requests.is_empty());
}

#[test]
fn media_match_natural_completion_does_not_advance_after_peer_selection() {
    let result = gui_completion_with_source(
        240.0,
        true,
        EndEvidence::KeepOpen,
        0.0,
        false,
        CompletionSource::MediaMatchThenPeerSelection,
    );
    assert_eq!(result.advances, 0);
    assert_eq!(result.selected, Some(1));
    assert!(result.requests.is_empty());
}

#[test]
fn native_pause_near_end_must_not_advance_gui_playlist() {
    let result = gui_completion(239.0, true, EndEvidence::None, 0.0, false);
    assert_eq!(
        result.advances, 0,
        "an intentional pause before EOF must retain the selected file"
    );
    assert_eq!(result.selected, Some(0));
    assert!(result.requests.is_empty());
}

#[test]
fn pause_outside_eof_window_control_preserves_selection() {
    assert_eq!(
        gui_completion(234.0, true, EndEvidence::None, 0.0, false).advances,
        0
    );
}

#[test]
fn still_playing_near_end_control_preserves_selection() {
    assert_eq!(
        gui_completion(239.0, false, EndEvidence::None, 0.0, false).advances,
        0
    );
}

#[test]
fn actual_eof_zero_offset_control_advances() {
    let result = gui_completion(240.0, true, EndEvidence::MatchedEndFile, 0.0, false);
    assert_guarded_advance(&result, 1);
    assert_eq!(result.selected, Some(1));
}

#[test]
fn actual_eof_positive_offset_must_advance() {
    let result = gui_completion(240.0, true, EndEvidence::MatchedEndFile, 10.0, false);
    assert_eq!(result.projected_position, Some(230.0));
    assert_eq!(
        result.advances, 1,
        "actual matched EOF must work in physical coordinates despite a user sync offset"
    );
    assert_eq!(result.selected, Some(1));
}

#[test]
fn actual_eof_small_positive_offset_control_advances() {
    assert_eq!(
        gui_completion(240.0, true, EndEvidence::MatchedEndFile, 4.0, false).advances,
        1
    );
}

#[test]
fn native_pause_near_end_must_not_restart_single_item_loop() {
    let result = gui_completion(239.0, true, EndEvidence::None, 0.0, true);
    assert_eq!(
        result.advances, 0,
        "native pause must not request a same-row replay"
    );
    assert!(result.requests.is_empty());
    assert!(
        result.applied_positions.is_empty(),
        "native pause must not rewind playback"
    );
    assert!(
        result.applied_pauses.is_empty(),
        "native pause must not automatically resume playback"
    );
}

#[test]
fn keep_open_eof_zero_offset_control_advances() {
    assert_eq!(
        gui_completion(240.0, true, EndEvidence::KeepOpen, 0.0, false).advances,
        1
    );
}

#[test]
fn keep_open_eof_positive_offset_must_advance() {
    let result = gui_completion(240.0, true, EndEvidence::KeepOpen, 10.0, false);
    assert_eq!(result.projected_position, Some(230.0));
    assert_eq!(
        result.advances, 1,
        "GUI default keep-open EOF must also advance with a user offset"
    );
    assert_guarded_advance(&result, 1);
}

#[test]
fn stop_at_duration_does_not_advance_or_loop() {
    for looping in [false, true] {
        let result = gui_completion(240.0, true, EndEvidence::Stop, 0.0, looping);
        assert_eq!(result.advances, 0);
        assert!(result.requests.is_empty());
        assert_eq!(result.selected, Some(0));
    }
}

#[test]
fn seek_back_before_completion_dispatch_preserves_current_selection() {
    let result = gui_completion(240.0, true, EndEvidence::KeepOpenThenSeekBack, 0.0, false);
    assert_eq!(result.advances, 0);
    assert!(result.requests.is_empty());
    assert_eq!(result.selected, Some(0));
}

#[test]
fn completed_predecessor_cannot_advance_a_peer_selection_or_same_row_replay() {
    for (evidence, index) in [
        (EndEvidence::KeepOpenThenPeerSelection, 1),
        (EndEvidence::KeepOpenThenSameRowReplay, 0),
    ] {
        let result = gui_completion(240.0, true, evidence, 0.0, false);
        assert_eq!(result.advances, 0);
        assert!(result.requests.is_empty());
        assert_eq!(result.selected, Some(index));
    }
}

#[test]
fn retained_completion_loops_with_exact_selection_guard_and_any_offset() {
    for offset in [-10.0, 0.0, 10.0] {
        let result = gui_completion(240.0, true, EndEvidence::KeepOpen, offset, true);
        assert_guarded_advance(&result, 0);
        assert!(
            result.applied_positions.is_empty(),
            "modern loop waits for canonical delivery"
        );
    }
}

#[test]
fn last_row_completion_publishes_a_bounded_room_position_with_offset() {
    for offset in [-10.0, 0.0, 10.0] {
        let result = gui_completion(240.0, true, EndEvidence::KeepOpenLastRow, offset, false);
        assert_eq!(result.advances, 1);
        assert!(result.requests.is_empty());
        assert_eq!(result.selected, Some(0));
        assert_eq!(result.terminal_positions, vec![240.0 - offset]);
    }
}
