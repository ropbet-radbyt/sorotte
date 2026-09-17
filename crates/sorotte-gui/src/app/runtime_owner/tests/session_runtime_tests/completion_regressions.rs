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
    snapshot_only_next_batch: bool,
    accepted_seek_commands: Vec<sorotte_player_api::PlayerCommandId>,
    completed_commands: Vec<sorotte_player_api::PlayerCommandId>,
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
            sorotte_player_api::PlayerCommand::SetPosition(position) => {
                let mut player = self.0.lock().unwrap();
                let command_id = player.harness.accept_tracked_seek(position);
                player.applied_positions.push(position);
                player.accepted_seek_commands.push(command_id);
                Ok(command_id)
            }
            _ => Err(PlayerError::Unsupported("execute_tracked")),
        }
    }
    fn take_player_event_batch(&mut self) -> Option<PlayerEventBatch> {
        let mut player = self.0.lock().unwrap();
        let mut batch = player.harness.take_event_batch()?;
        for outcome in &batch.semantic_outcomes {
            if let sorotte_player_api::PlayerSemanticOutcome::Command(command) = outcome.outcome
                && command.result == sorotte_player_api::PlayerCommandSemanticResult::Completed
            {
                player.completed_commands.push(command.command_id);
            }
        }
        if std::mem::take(&mut player.snapshot_only_next_batch) {
            assert!(batch.authoritative_snapshot.is_some());
            // The snapshot covers these older events. Exercise the supported
            // snapshot-only recovery contract without replayed load/file edges.
            batch.events.clear();
            batch.semantic_outcomes.clear();
        }
        Some(batch)
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
    natural_end_observed: bool,
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
    KeepOpenAfterPeerDuplicateSelection,
    KeepOpenAfterPeerReplay,
    KeepOpenAfterPeerReplaySeek,
    KeepOpenAfterPeerReplayQueuedSeek,
    KeepOpenAfterPeerReplayAcceptedSeek,
    KeepOpenAfterPeerReplaySeekThenFailedSeek,
    KeepOpenAfterPeerReplayFailedSeek,
    KeepOpenAfterUnselectedEdit,
    KeepOpenAfterReorder,
    KeepOpenFromInitialSnapshot,
    KeepOpenAfterPeerReplaySnapshot,
    KeepOpenWithoutPlaylist,
}

impl EndEvidence {
    fn is_keep_open(self) -> bool {
        !matches!(self, Self::None | Self::MatchedEndFile | Self::Stop)
    }
}

fn observe_completed_seek(player: &mut CompletionPlayer, physical_position: f64) {
    for event in [
        json!({"event":"seek"}),
        json!({"event":"property-change","name":"seeking","data":true}),
        json!({"event":"property-change","name":"eof-reached","data":false}),
        json!({"event":"property-change","name":"time-pos","data":physical_position}),
        json!({"event":"property-change","name":"seeking","data":false}),
        json!({"event":"playback-restart"}),
        json!({"event":"property-change","name":"pause","data":false}),
        json!({"event":"property-change","name":"core-idle","data":false}),
        json!({"event":"property-change","name":"time-pos","data":physical_position + 1.0}),
    ] {
        player.harness.ingest_decoded_mpv_json(event);
    }
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
    let files = if matches!(end_evidence, EndEvidence::KeepOpenWithoutPlaylist) {
        json!([])
    } else if looping || matches!(end_evidence, EndEvidence::KeepOpenLastRow) {
        json!(["episode1.mkv"])
    } else if matches!(
        end_evidence,
        EndEvidence::KeepOpenAfterPeerDuplicateSelection
    ) {
        json!(["episode1.mkv", "episode1.mkv", "episode2.mkv"])
    } else {
        json!(["episode1.mkv", "episode2.mkv"])
    };
    session
        .apply_message_json(
            &json!({"Set":{"playlistChange":{"files":files,"user":"alice","sorottePlaylistEpoch":1}}}).to_string(),
        )
        .unwrap();
    if !files.as_array().unwrap().is_empty() {
        session
            .apply_message_json(
                r#"{"Set":{"playlistIndex":{"index":0,"user":"alice","sorottePlaylistEpoch":2}}}"#,
            )
            .unwrap();
    }
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
    let origin_root = tempfile::tempdir().unwrap();
    let first_origin = if matches!(
        end_evidence,
        EndEvidence::KeepOpenAfterPeerDuplicateSelection
    ) {
        let paths = ["first/episode1.mkv", "second/episode1.mkv", "episode2.mkv"].map(|relative| {
            let path = origin_root.path().join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, relative.as_bytes()).unwrap();
            path.to_string_lossy().into_owned()
        });
        let runtime_state = runtime_state_for_shell(&state);
        let outcome = owner.bind_selected_local_origins_for_test(&runtime_state, paths.to_vec());
        assert_eq!(outcome.bound_row_ids.len(), 3);
        assert_ne!(
            paths[0], paths[1],
            "same labels retain distinct exact origins"
        );
        Some(paths[0].clone())
    } else {
        None
    };
    let physical_target = if let Some(origin) = first_origin.as_deref() {
        origin
    } else if matches!(source, CompletionSource::SelectedName) {
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
        if matches!(end_evidence, EndEvidence::KeepOpenFromInitialSnapshot) {
            harness.detect_event_gap();
            harness.apply_authoritative_snapshot(
                [LifecycleVerificationPlaylistEntry::new(
                    77,
                    Some(physical_target.into()),
                    true,
                )],
                Some(physical_target.into()),
            );
            player.snapshot_only_next_batch = true;
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
    if matches!(
        end_evidence,
        EndEvidence::KeepOpenAfterPeerReplayQueuedSeek
            | EndEvidence::KeepOpenAfterPeerReplayAcceptedSeek
    ) {
        let command_id = owner
            .player
            .as_mut()
            .unwrap()
            .set_position_tracked(30.0 + offset)
            .unwrap();
        owner.note_attached_runtime_position_dispatched(command_id, 30.0, 30.0 + offset);
        observe_completed_seek(&mut player.lock().unwrap(), 30.0 + offset);
        // Its real command receipt and physical seek are queued under the
        // predecessor selection, before the peer requests a replay.
        player.lock().unwrap().applied_positions.clear();
    }
    if matches!(source, CompletionSource::MediaMatchAlreadyPlaying) {
        assert_eq!(owner.open_media_match_resolution_candidate_for_test(
            &runtime_state_for_shell(&state), physical_target.to_owned(),
        ), crate::app::runtime_owner::player::SelectedPlaylistMediaSyncOutcome::MatchedCurrentTarget);
    }
    if matches!(
        end_evidence,
        EndEvidence::KeepOpenAfterPeerDuplicateSelection
            | EndEvidence::KeepOpenAfterPeerReplay
            | EndEvidence::KeepOpenAfterPeerReplaySeek
            | EndEvidence::KeepOpenAfterPeerReplayQueuedSeek
            | EndEvidence::KeepOpenAfterPeerReplayAcceptedSeek
            | EndEvidence::KeepOpenAfterPeerReplaySeekThenFailedSeek
            | EndEvidence::KeepOpenAfterPeerReplayFailedSeek
            | EndEvidence::KeepOpenAfterUnselectedEdit
            | EndEvidence::KeepOpenAfterReorder
            | EndEvidence::KeepOpenAfterPeerReplaySnapshot
    ) {
        let (next_files, next_index) = match end_evidence {
            EndEvidence::KeepOpenAfterPeerDuplicateSelection => (files.clone(), 1),
            EndEvidence::KeepOpenAfterReorder => (json!(["episode2.mkv", "episode1.mkv"]), 1),
            EndEvidence::KeepOpenAfterUnselectedEdit => {
                (json!(["episode1.mkv", "episode3.mkv", "episode2.mkv"]), 0)
            }
            _ => (files.clone(), 0),
        };
        let session = owner.session.as_mut().unwrap();
        let mut epoch = 3;
        if next_files != files {
            session.apply_message_json(
                &json!({"Set":{"playlistChange":{"files":next_files,"user":"bob","sorottePlaylistEpoch":epoch}}}).to_string(),
            ).unwrap();
            epoch += 1;
        }
        session.apply_message_json(
            &json!({"Set":{"playlistIndex":{"index":next_index,"user":"bob","sorottePlaylistEpoch":epoch}}}).to_string(),
        ).unwrap();
        state.apply_shared_playlist_entries(
            next_files
                .as_array()
                .unwrap()
                .iter()
                .map(|file| file.as_str().unwrap().to_owned())
                .collect(),
            Some(next_index as usize),
            false,
        );
        if matches!(
            end_evidence,
            EndEvidence::KeepOpenAfterPeerReplaySeek
                | EndEvidence::KeepOpenAfterPeerReplayAcceptedSeek
                | EndEvidence::KeepOpenAfterPeerReplaySeekThenFailedSeek
                | EndEvidence::KeepOpenAfterPeerReplayFailedSeek
        ) {
            owner.apply_pending_playlist_index_reset_to_attached_player_impl(
                &runtime_state_for_shell(&state),
                true,
            );
            let command_id = {
                let player = player.lock().unwrap();
                assert_eq!(
                    player.applied_positions,
                    [offset],
                    "the production replay reset must be accepted"
                );
                *player.accepted_seek_commands.last().unwrap()
            };
            if matches!(end_evidence, EndEvidence::KeepOpenAfterPeerReplayFailedSeek) {
                player.lock().unwrap().harness.fail_tracked_command(
                    command_id,
                    sorotte_player_api::PlayerCommandFailureKind::Unknown,
                );
                owner.refresh_player_state_impl();
            } else if matches!(
                end_evidence,
                EndEvidence::KeepOpenAfterPeerReplaySeek
                    | EndEvidence::KeepOpenAfterPeerReplaySeekThenFailedSeek
            ) {
                observe_completed_seek(&mut player.lock().unwrap(), offset);
                if matches!(
                    end_evidence,
                    EndEvidence::KeepOpenAfterPeerReplaySeekThenFailedSeek
                ) {
                    let later_command = owner
                        .player
                        .as_mut()
                        .unwrap()
                        .set_position_tracked(50.0 + offset)
                        .unwrap();
                    owner.note_attached_runtime_position_dispatched(
                        later_command,
                        50.0,
                        50.0 + offset,
                    );
                    player.lock().unwrap().harness.fail_tracked_command(
                        later_command.unwrap(),
                        sorotte_player_api::PlayerCommandFailureKind::Unknown,
                    );
                }
                owner.refresh_player_state_impl();
                assert!(
                    player
                        .lock()
                        .unwrap()
                        .completed_commands
                        .contains(&command_id),
                    "fresh replay must have a matching physical completion receipt"
                );
            } else {
                assert!(
                    !player
                        .lock()
                        .unwrap()
                        .completed_commands
                        .contains(&command_id),
                    "reset acceptance is not its completion receipt"
                );
            }
        } else if matches!(end_evidence, EndEvidence::KeepOpenAfterPeerReplaySnapshot) {
            let mut player = player.lock().unwrap();
            player.harness.detect_event_gap();
            player.harness.apply_authoritative_snapshot(
                [LifecycleVerificationPlaylistEntry::new(
                    77,
                    Some(physical_target.into()),
                    true,
                )],
                Some(physical_target.into()),
            );
            player.snapshot_only_next_batch = true;
            drop(player);
            owner.refresh_player_state_impl();
        } else if matches!(
            end_evidence,
            EndEvidence::KeepOpenAfterPeerDuplicateSelection | EndEvidence::KeepOpenAfterPeerReplay
        ) {
            assert!(
                player.lock().unwrap().applied_positions.is_empty(),
                "predecessor EOF is queued before any successor physical reset"
            );
        }
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
        if end_evidence.is_keep_open() {
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
        std::path::Path::new(physical_target)
            .file_name()
            .unwrap()
            .to_string_lossy(),
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
        natural_end_observed: owner.attached_player_observation_is_end_of_file(),
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
fn predecessor_eof_after_peer_duplicate_selection_does_not_skip_the_selected_row() {
    let result = gui_completion(
        240.0,
        true,
        EndEvidence::KeepOpenAfterPeerDuplicateSelection,
        0.0,
        false,
    );
    assert_eq!(
        result.advances, 0,
        "predecessor completion must not acquire the successor's selection identity"
    );
    assert_eq!(result.selected, Some(1));
    assert!(result.requests.is_empty());
}

#[test]
fn predecessor_eof_after_peer_same_row_replay_does_not_skip_the_replay() {
    let result = gui_completion(
        240.0,
        true,
        EndEvidence::KeepOpenAfterPeerReplay,
        0.0,
        false,
    );
    assert_eq!(result.advances, 0);
    assert_eq!(result.selected, Some(0));
    assert!(result.requests.is_empty());
}

#[test]
fn fresh_eof_after_observed_same_row_replay_seek_advances() {
    for offset in [0.0, 10.0] {
        let result = gui_completion(
            240.0,
            true,
            EndEvidence::KeepOpenAfterPeerReplaySeek,
            offset,
            false,
        );
        assert_eq!(result.advances, 1);
        assert_eq!(result.selected, Some(1));
        assert_eq!(result.requests[0]["sorotteExpectedPlaylistEpoch"], 3);
    }
}

#[test]
fn queued_predecessor_seek_cannot_adopt_a_peer_replay() {
    let result = gui_completion(
        240.0,
        true,
        EndEvidence::KeepOpenAfterPeerReplayQueuedSeek,
        0.0,
        false,
    );
    assert_eq!(result.advances, 0);
    assert_eq!(result.selected, Some(0));
    assert!(result.requests.is_empty());
}

#[test]
fn accepted_replay_command_does_not_authorize_the_queued_predecessor_seek() {
    let result = gui_completion(
        240.0,
        true,
        EndEvidence::KeepOpenAfterPeerReplayAcceptedSeek,
        0.0,
        false,
    );
    assert_eq!(result.advances, 0);
    assert_eq!(result.selected, Some(0));
    assert!(result.requests.is_empty());
}

#[test]
fn completed_replay_receipt_survives_a_later_failed_seek_before_drain() {
    let result = gui_completion(
        240.0,
        true,
        EndEvidence::KeepOpenAfterPeerReplaySeekThenFailedSeek,
        0.0,
        false,
    );
    assert_eq!(result.advances, 1);
    assert_eq!(result.selected, Some(1));
}

#[test]
fn failed_replay_seek_receipt_cannot_adopt_the_predecessor() {
    let result = gui_completion(
        240.0,
        true,
        EndEvidence::KeepOpenAfterPeerReplayFailedSeek,
        0.0,
        false,
    );
    assert_eq!(result.advances, 0);
    assert_eq!(result.selected, Some(0));
    assert!(result.requests.is_empty());
}

#[test]
fn first_owned_snapshot_without_load_event_replay_can_complete() {
    let result = gui_completion(
        240.0,
        true,
        EndEvidence::KeepOpenFromInitialSnapshot,
        0.0,
        false,
    );
    assert_guarded_advance(&result, 1);
}

#[test]
fn direct_file_completion_without_playlist_retains_natural_terminal_state() {
    let result = gui_completion(
        240.0,
        true,
        EndEvidence::KeepOpenWithoutPlaylist,
        0.0,
        false,
    );
    assert!(result.natural_end_observed);
    assert_eq!(result.advances, 0);
    assert_eq!(result.selected, None);
    assert!(result.requests.is_empty());
}

#[test]
fn snapshot_of_predecessor_after_same_row_replay_does_not_rebind_completion() {
    let result = gui_completion(
        240.0,
        true,
        EndEvidence::KeepOpenAfterPeerReplaySnapshot,
        0.0,
        false,
    );
    assert_eq!(result.advances, 0);
    assert_eq!(result.selected, Some(0));
    assert!(result.requests.is_empty());
}

#[test]
fn natural_completion_survives_unselected_playlist_edit() {
    let result = gui_completion(
        240.0,
        true,
        EndEvidence::KeepOpenAfterUnselectedEdit,
        0.0,
        false,
    );
    assert_eq!(result.advances, 1);
    assert_eq!(result.selected, Some(1));
    assert_eq!(result.requests[0]["sorotteExpectedPlaylistEpoch"], 4);
}

#[test]
fn natural_completion_survives_reorder_preserving_the_active_row() {
    let result = gui_completion(240.0, true, EndEvidence::KeepOpenAfterReorder, 0.0, false);
    assert_eq!(result.advances, 1);
    assert_eq!(result.selected, Some(1));
    assert!(result.requests.is_empty());
    assert_eq!(result.terminal_positions, [240.0]);
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
