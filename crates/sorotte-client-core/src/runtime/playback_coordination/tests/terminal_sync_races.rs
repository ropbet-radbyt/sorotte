use super::*;

fn terminal_after_rejected_seek(outcome: PlayerPhysicalLoadOutcome) -> PlayerEventBatch {
    let epoch = PlayerAttachmentEpoch::new(1);
    let attempt_id = LoadAttemptId::new(1);
    let media_generation = PlayerMediaGeneration::new(1);
    ordered_batch(
        epoch,
        3,
        2,
        None,
        vec![
            SequencedPlayerEvent {
                order: PlayerEventOrder::new(epoch, 2),
                event: PlayerEvent::LoadAttemptTerminal {
                    attempt_id,
                    media_generation,
                    outcome,
                },
            },
            SequencedPlayerEvent {
                order: PlayerEventOrder::new(epoch, 3),
                event: PlayerEvent::LogicalPlaybackTerminal {
                    attempt_id,
                    media_generation,
                    outcome,
                },
            },
        ],
        Vec::new(),
    )
}

fn failing_room_correction(
    terminal: Option<PlayerPhysicalLoadOutcome>,
) -> ClientRuntime<CoordinatedTestPlayer, QueuedRuntimeControl> {
    let mut runtime = natural_completion_eof_race_runtime();
    runtime
        .session_mut()
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":0.0,"paused":true,"doSeek":true,"setBy":"bob"}}}"#,
            1.0,
        )
        .expect("a remote seek with pause should require physical correction");
    runtime.player.reject_seek_commands = true;
    runtime.player.ordered_batch_after_rejected_seek = terminal.map(terminal_after_rejected_seek);
    runtime
}

#[test]
fn natural_eof_overtaking_room_pause_seek_retains_completion() {
    let mut runtime = failing_room_correction(Some(PlayerPhysicalLoadOutcome::Ended));
    runtime
        .run_room_pause_sync_if_needed_at(1.1)
        .expect("EOF should retire the losing room-pause correction");
    assert!(
        runtime
            .player
            .commands
            .iter()
            .any(|command| { matches!(command, PlayerCommand::SetPosition(_)) })
    );
    assert_eq!(
        runtime.playback_coordination_snapshot().diagnostic,
        PlaybackDiagnostic::Ended
    );
    assert!(runtime.has_pending_natural_playback_completion());
    assert!(
        runtime
            .run_advance_playlist_after_natural_completion()
            .unwrap()
    );
    assert_eq!(
        runtime.session().current_room_playlist().unwrap().index,
        Some(1)
    );
    assert!(
        !runtime
            .run_advance_playlist_after_natural_completion()
            .unwrap()
    );
}

#[test]
fn room_pause_seek_failure_without_terminal_evidence_remains_an_error() {
    let mut runtime = failing_room_correction(None);
    runtime.run_room_pause_sync_if_needed_at(1.1).unwrap_err();
    assert!(!runtime.has_pending_natural_playback_completion());
    assert_eq!(
        runtime.session().current_room_playlist().unwrap().index,
        Some(0)
    );
}

#[test]
fn desync_seek_failure_without_terminal_evidence_remains_an_error() {
    let mut runtime = failing_room_correction(None);
    runtime
        .run_desync_correction_if_needed(1.1, false, false, true)
        .unwrap_err();
    assert!(!runtime.has_pending_natural_playback_completion());
    assert_eq!(
        runtime.session().current_room_playlist().unwrap().index,
        Some(0)
    );
}

#[test]
fn room_pause_seek_failure_with_media_failure_remains_an_error() {
    let mut runtime = failing_room_correction(Some(PlayerPhysicalLoadOutcome::Failed(
        sorotte_player_api::PlayerMediaLoadFailureKind::Unknown,
    )));
    runtime.run_room_pause_sync_if_needed_at(1.1).unwrap_err();
    assert!(!runtime.has_pending_natural_playback_completion());
    assert_eq!(
        runtime.playback_coordination_snapshot().diagnostic,
        PlaybackDiagnostic::Failed
    );
}

#[test]
fn desync_seek_failure_with_media_failure_remains_an_error() {
    let mut runtime = failing_room_correction(Some(PlayerPhysicalLoadOutcome::Failed(
        sorotte_player_api::PlayerMediaLoadFailureKind::Unknown,
    )));
    runtime
        .run_desync_correction_if_needed(1.1, false, false, true)
        .unwrap_err();
    assert!(!runtime.has_pending_natural_playback_completion());
    assert_eq!(
        runtime.playback_coordination_snapshot().diagnostic,
        PlaybackDiagnostic::Failed
    );
}

#[test]
fn queued_predecessor_seek_completion_cannot_resume_after_successor_selection() {
    for successor_selected in [false, true] {
        let mut runtime = natural_completion_eof_race_runtime();
        runtime.run_room_pause_sync_if_needed_at(1.1).unwrap();
        let accepted_command_id = PlayerCommandId::new(runtime.player.next_command_id);
        assert!(
            runtime
                .playback_coordination
                .player_command_bindings
                .contains_key(&accepted_command_id)
        );
        runtime.player.commands.clear();

        let epoch = PlayerAttachmentEpoch::new(1);
        let generation = PlayerMediaGeneration::new(1);
        runtime.player.ordered_batches.push_back(ordered_batch(
            epoch,
            3,
            2,
            None,
            vec![SequencedPlayerEvent {
                order: PlayerEventOrder::new(epoch, 3),
                event: PlayerEvent::TransportDelta(PlayerTransportDelta {
                    load_attempt_id: Some(LoadAttemptId::new(1)),
                    media_generation: Some(generation),
                    observed_at: Some(PlayerObservationTimestamp::from_adapter_start(
                        Duration::from_millis(1150),
                    )),
                    phase: Some(PlayerTransportPhase::ReadyPaused),
                    position_seconds: Some(0.0),
                    logical_pause: Some(true),
                    paused_for_cache: Some(false),
                    seeking: Some(false),
                    ..PlayerTransportDelta::default()
                }),
            }],
            vec![SequencedPlayerSemanticOutcome {
                order: PlayerEventOrder::new(epoch, 2),
                outcome: PlayerSemanticOutcome::Command(sorotte_player_api::PlayerCommandOutcome {
                    attachment_epoch: epoch,
                    command_id: accepted_command_id,
                    media_generation: Some(generation),
                    result: PlayerCommandSemanticResult::Completed,
                }),
            }],
        ));
        if successor_selected {
            runtime.session_mut().apply_message_json_at(
                r#"{"Set":{"playlistIndex":{"index":1,"user":"bob","sorottePlaylistEpoch":5}}}"#,
                1.18,
            ).unwrap();
            assert!(runtime.session().has_pending_playlist_index_reset_intent());
        }

        runtime.drain_player_transport_coordination(1.2).unwrap();
        assert!(
            !runtime
                .playback_coordination
                .player_command_bindings
                .contains_key(&accepted_command_id)
        );
        assert!(runtime.player.ordered_batches.is_empty());
        if successor_selected {
            assert!(
                runtime.player.commands.is_empty(),
                "old seek evidence must not dispatch predecessor follow-on commands: {:?}",
                runtime.player.commands
            );
            assert_eq!(runtime.player.next_command_id, accepted_command_id.get());
        } else {
            assert!(
                runtime.player.commands.iter().any(|command| matches!(
                    command,
                    PlayerCommand::Play(_) | PlayerCommand::SetPaused(false)
                )),
                "the same physical completion resumes the still-current selection: {:?}",
                runtime.player.commands
            );
        }
    }
}
