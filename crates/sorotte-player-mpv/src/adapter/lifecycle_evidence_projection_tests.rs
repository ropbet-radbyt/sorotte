use super::*;

fn changed_states(
    attachment_epoch: PlayerAttachmentEpoch,
) -> (PlayerLifecycleState, PlayerLifecycleState) {
    let before = PlayerLifecycleState::new(attachment_epoch);
    let mut after = before.clone();
    after.now_tick = 1;
    (before, after)
}

fn project(
    input: &PlayerLifecycleInput,
    before: &PlayerLifecycleState,
    after: &PlayerLifecycleState,
) -> Vec<CapturedPlayerLifecycleTransition> {
    capture_player_lifecycle_transitions(|| {
        emit_player_lifecycle_input_evidence(input, before, after);
    })
}

fn transition_ids(transitions: &[CapturedPlayerLifecycleTransition]) -> Vec<&'static str> {
    transitions
        .iter()
        .map(|transition| transition.transition)
        .collect()
}

fn transition(
    transitions: &[CapturedPlayerLifecycleTransition],
    transition_id: &str,
) -> CapturedPlayerLifecycleTransition {
    transitions
        .iter()
        .find(|transition| transition.transition == transition_id)
        .cloned()
        .unwrap_or_else(|| panic!("missing lifecycle evidence transition {transition_id}"))
}

fn assert_identity(transition: &CapturedPlayerLifecycleTransition, name: &'static str, value: u64) {
    assert!(
        transition.identities.contains(&(name, value)),
        "{} should carry {name}={value}: {:?}",
        transition.transition,
        transition.identities
    );
}

fn reduce(state: PlayerLifecycleState, input: PlayerLifecycleInput) -> PlayerLifecycleState {
    reduce_player_lifecycle(state, input).0
}

fn active_media_with_provisional_eof() -> PlayerLifecycleState {
    let epoch = PlayerAttachmentEpoch::new(1);
    let generation = PlayerMediaGeneration::new(7);
    let state = reduce(
        PlayerLifecycleState::new(epoch),
        PlayerLifecycleInput::ExternalLoadObserved {
            attachment_epoch: epoch,
            media_generation: generation,
            playlist_entry_id: 41,
            observed_target: "generated.mp4".to_owned(),
            file_loaded: true,
        },
    );
    reduce(
        state,
        PlayerLifecycleInput::EofObserved {
            attachment_epoch: epoch,
            playlist_entry_id: Some(41),
            reached: true,
            position_seconds: Some(12.0),
        },
    )
}

#[test]
fn load_submission_evidence_attributes_the_new_attempt_and_recovery_predecessor() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let generation = PlayerMediaGeneration::new(7);
    let command_id = PlayerCommandId::new(11);
    let fresh_input = PlayerLifecycleInput::LoadAttemptSubmitted {
        command_id: Some(command_id),
        media_generation: generation,
        requested_target: "fresh.mp4".to_owned(),
        baseline_playlist_entry_ids: BTreeSet::new(),
    };
    let fresh_before = PlayerLifecycleState::new(epoch);
    let fresh_after = reduce(fresh_before.clone(), fresh_input.clone());
    let fresh = project(&fresh_input, &fresh_before, &fresh_after);

    assert_eq!(
        transition_ids(&fresh),
        ["LOAD-SUBMIT-001", "TRANSPORT-LOAD-001"]
    );
    let submit = transition(&fresh, "LOAD-SUBMIT-001");
    assert_eq!(submit.machine, "load-attempt");
    assert_eq!(submit.trigger, Trigger::LocalInput);
    assert_eq!(submit.disposition, Disposition::Submitted);
    assert_identity(&submit, "attachment-epoch", 1);
    assert_identity(&submit, "media-generation", 7);
    assert_identity(&submit, "command-id", 11);
    assert_identity(&submit, "load-attempt-id", 1);

    let recovery_before = active_media_with_provisional_eof();
    let predecessor = recovery_before
        .active_attempt()
        .expect("fixture should have an active predecessor")
        .id;
    let recovery_input = PlayerLifecycleInput::LoadAttemptSubmitted {
        command_id: Some(PlayerCommandId::new(12)),
        media_generation: generation,
        requested_target: "recovered.mp4".to_owned(),
        baseline_playlist_entry_ids: BTreeSet::from([41]),
    };
    let recovery_after = reduce(recovery_before.clone(), recovery_input.clone());
    let successor = recovery_after
        .load_attempts
        .values()
        .find(|attempt| !recovery_before.load_attempts.contains_key(&attempt.id))
        .expect("recovery should allocate a successor")
        .id;
    let recovery = project(&recovery_input, &recovery_before, &recovery_after);

    assert_eq!(
        transition_ids(&recovery),
        [
            "LOAD-SUPERSEDE-001",
            "TRANSPORT-EOF-CANCEL-001",
            "TRANSPORT-FAIL-001",
            "LOAD-RECOVER-001",
            "LOAD-RECOVERY-SUBMIT-001",
            "TRANSPORT-LOAD-001",
        ]
    );
    for projected in &recovery {
        assert_identity(projected, "predecessor-load-attempt-id", predecessor.get());
        assert_identity(projected, "load-attempt-id", successor.get());
    }
    assert_eq!(
        transition(&recovery, "LOAD-SUPERSEDE-001").disposition,
        Disposition::Superseded
    );
    assert_eq!(
        transition(&recovery, "LOAD-RECOVER-001").trigger,
        Trigger::Recovery
    );
}

#[test]
fn load_observation_evidence_projects_each_physical_boundary() {
    let epoch = PlayerAttachmentEpoch::new(3);
    let generation = PlayerMediaGeneration::new(9);
    let (before, after) = changed_states(epoch);
    let cases = [
        (
            PlayerLifecycleInput::ExternalLoadObserved {
                attachment_epoch: epoch,
                media_generation: generation,
                playlist_entry_id: 17,
                observed_target: "loading.mp4".to_owned(),
                file_loaded: false,
            },
            vec!["LOAD-START-001", "TRANSPORT-LOAD-001"],
        ),
        (
            PlayerLifecycleInput::ExternalLoadObserved {
                attachment_epoch: epoch,
                media_generation: generation,
                playlist_entry_id: 18,
                observed_target: "active.mp4".to_owned(),
                file_loaded: true,
            },
            vec!["LOAD-ACTIVE-001", "TRANSPORT-LOAD-001"],
        ),
        (
            PlayerLifecycleInput::LoadAttemptAccepted {
                attachment_epoch: epoch,
                attempt_id: LoadAttemptId::new(21),
            },
            vec!["LOAD-ACCEPT-001"],
        ),
        (
            PlayerLifecycleInput::LoadAttemptRejected {
                attachment_epoch: epoch,
                attempt_id: LoadAttemptId::new(22),
                failure: PlayerCommandFailureKind::Unknown,
            },
            vec!["LOAD-TERMINAL-001"],
        ),
        (
            PlayerLifecycleInput::StartFile {
                attachment_epoch: epoch,
                playlist_entry_id: 23,
            },
            vec!["LOAD-BIND-001", "LOAD-START-001"],
        ),
        (
            PlayerLifecycleInput::FileLoaded {
                attachment_epoch: epoch,
                playlist_entry_id: Some(24),
                loaded_target: Some("loaded.mp4".to_owned()),
            },
            vec!["LOAD-ACTIVE-001"],
        ),
        (
            PlayerLifecycleInput::EndFile {
                attachment_epoch: epoch,
                playlist_entry_id: 25,
                outcome: PlayerPhysicalLoadOutcome::Ended,
            },
            vec!["LOAD-TERMINAL-001", "TRANSPORT-END-001"],
        ),
        (
            PlayerLifecycleInput::EndFile {
                attachment_epoch: epoch,
                playlist_entry_id: 26,
                outcome: PlayerPhysicalLoadOutcome::Failed(PlayerMediaLoadFailureKind::Network),
            },
            vec!["LOAD-TERMINAL-001", "TRANSPORT-FAIL-001"],
        ),
        (
            PlayerLifecycleInput::EndFile {
                attachment_epoch: epoch,
                playlist_entry_id: 27,
                outcome: PlayerPhysicalLoadOutcome::NeverStarted,
            },
            vec!["LOAD-TERMINAL-001", "TRANSPORT-FAIL-001"],
        ),
        (
            PlayerLifecycleInput::EndFile {
                attachment_epoch: epoch,
                playlist_entry_id: 28,
                outcome: PlayerPhysicalLoadOutcome::TransportDisconnected,
            },
            vec!["LOAD-TERMINAL-001", "TRANSPORT-FAIL-001"],
        ),
    ];

    for (input, expected) in cases {
        let projected = project(&input, &before, &after);
        assert_eq!(transition_ids(&projected), expected, "input: {input:?}");
        assert!(
            projected
                .iter()
                .all(|item| item.identities.contains(&("attachment-epoch", 3)))
        );
    }

    let rejected = project(
        &PlayerLifecycleInput::LoadAttemptRejected {
            attachment_epoch: epoch,
            attempt_id: LoadAttemptId::new(22),
            failure: PlayerCommandFailureKind::TimedOut,
        },
        &before,
        &after,
    );
    assert_eq!(rejected[0].disposition, Disposition::Rejected);
    assert_identity(&rejected[0], "load-attempt-id", 22);
    let failed = project(
        &PlayerLifecycleInput::EndFile {
            attachment_epoch: epoch,
            playlist_entry_id: 26,
            outcome: PlayerPhysicalLoadOutcome::Failed(PlayerMediaLoadFailureKind::Network),
        },
        &before,
        &after,
    );
    assert_eq!(failed[1].trigger, Trigger::PlayerEvent);
    assert_eq!(failed[1].disposition, Disposition::Failed);
    assert_identity(&failed[1], "playlist-entry-id", 26);
}

#[test]
fn eof_cancellation_evidence_requires_an_observed_candidate_to_be_cleared() {
    let epoch = PlayerAttachmentEpoch::new(1);
    let generation = PlayerMediaGeneration::new(7);
    let cancellation_inputs = [
        PlayerLifecycleInput::PlaybackRestart {
            attachment_epoch: epoch,
            playlist_entry_id: Some(41),
        },
        PlayerLifecycleInput::PositionObserved {
            attachment_epoch: epoch,
            media_generation: generation,
            observed_sequence: 2,
            position_seconds: 13.0,
        },
        PlayerLifecycleInput::SeekingObserved {
            attachment_epoch: epoch,
            media_generation: generation,
            observed_sequence: 3,
            seeking: true,
        },
        PlayerLifecycleInput::PhaseObserved {
            attachment_epoch: epoch,
            phase: PlayerTransportPhase::Playing,
        },
    ];

    for input in cancellation_inputs {
        let before = active_media_with_provisional_eof();
        let after = reduce(before.clone(), input.clone());
        let projected = project(&input, &before, &after);
        assert_eq!(
            projected[0].transition, "TRANSPORT-EOF-CANCEL-001",
            "input: {input:?}"
        );
        assert_eq!(projected[0].disposition, Disposition::Applied);
        assert!(before.provisional_eof_attempt().is_some());
        assert!(after.provisional_eof_attempt().is_none());
    }

    let before = active_media_with_provisional_eof();
    let after = reduce(
        before.clone(),
        PlayerLifecycleInput::EofObserved {
            attachment_epoch: epoch,
            playlist_entry_id: Some(41),
            reached: false,
            position_seconds: Some(12.0),
        },
    );
    let cancelled = project(
        &PlayerLifecycleInput::EofObserved {
            attachment_epoch: epoch,
            playlist_entry_id: Some(41),
            reached: false,
            position_seconds: Some(12.0),
        },
        &before,
        &after,
    );
    assert_eq!(transition_ids(&cancelled), ["TRANSPORT-EOF-CANCEL-001"]);

    let (before, after) = changed_states(epoch);
    let candidate = project(
        &PlayerLifecycleInput::EofObserved {
            attachment_epoch: epoch,
            playlist_entry_id: Some(41),
            reached: true,
            position_seconds: Some(12.0),
        },
        &before,
        &after,
    );
    assert_eq!(transition_ids(&candidate), ["TRANSPORT-EOF-CANDIDATE-001"]);
    assert_identity(&candidate[0], "playlist-entry-id", 41);
}

#[test]
fn phase_and_sparse_delta_evidence_preserve_transport_precedence() {
    let epoch = PlayerAttachmentEpoch::new(5);
    let (before, after) = changed_states(epoch);
    let phases = [
        (PlayerTransportPhase::Empty, "TRANSPORT-DETACH-001"),
        (PlayerTransportPhase::Loading, "TRANSPORT-LOAD-001"),
        (PlayerTransportPhase::Prebuffering, "TRANSPORT-LOAD-001"),
        (PlayerTransportPhase::ReadyPaused, "TRANSPORT-PAUSE-001"),
        (PlayerTransportPhase::Playing, "TRANSPORT-PLAY-001"),
        (
            PlayerTransportPhase::Rebuffering,
            "TRANSPORT-CACHE-PAUSE-001",
        ),
        (PlayerTransportPhase::Seeking, "TRANSPORT-SEEK-001"),
        (PlayerTransportPhase::Ended, "TRANSPORT-END-001"),
        (PlayerTransportPhase::Failed, "TRANSPORT-FAIL-001"),
    ];
    for (phase, expected) in phases {
        let projected = project(
            &PlayerLifecycleInput::PhaseObserved {
                attachment_epoch: epoch,
                phase,
            },
            &before,
            &after,
        );
        assert_eq!(transition_ids(&projected), [expected]);
        assert_eq!(
            projected[0].disposition,
            if phase == PlayerTransportPhase::Failed {
                Disposition::Failed
            } else {
                Disposition::Observed
            }
        );
    }

    let deltas = [
        (
            PlayerTransportDelta {
                paused_for_cache: Some(true),
                seeking: Some(true),
                eof_reached: Some(true),
                logical_pause: Some(true),
                ..PlayerTransportDelta::default()
            },
            Some("TRANSPORT-CACHE-PAUSE-001"),
        ),
        (
            PlayerTransportDelta {
                seeking: Some(true),
                eof_reached: Some(true),
                logical_pause: Some(true),
                ..PlayerTransportDelta::default()
            },
            Some("TRANSPORT-SEEK-001"),
        ),
        (
            PlayerTransportDelta {
                eof_reached: Some(true),
                logical_pause: Some(true),
                ..PlayerTransportDelta::default()
            },
            Some("TRANSPORT-EOF-CANDIDATE-001"),
        ),
        (
            PlayerTransportDelta {
                logical_pause: Some(true),
                ..PlayerTransportDelta::default()
            },
            Some("TRANSPORT-PAUSE-001"),
        ),
        (
            PlayerTransportDelta {
                logical_pause: Some(false),
                ..PlayerTransportDelta::default()
            },
            Some("TRANSPORT-PLAY-001"),
        ),
        (PlayerTransportDelta::default(), None),
    ];
    for (mut delta, expected) in deltas {
        delta.load_attempt_id = Some(LoadAttemptId::new(31));
        delta.media_generation = Some(PlayerMediaGeneration::new(32));
        let projected = project(
            &PlayerLifecycleInput::TransportDelta {
                attachment_epoch: epoch,
                delta,
            },
            &before,
            &after,
        );
        assert_eq!(
            transition_ids(&projected),
            expected.into_iter().collect::<Vec<_>>()
        );
        if let Some(projected) = projected.first() {
            assert_identity(projected, "load-attempt-id", 31);
            assert_identity(projected, "media-generation", 32);
        }
    }
}

#[test]
fn transport_loss_and_reattachment_evidence_are_distinct() {
    let epoch = PlayerAttachmentEpoch::new(2);
    let (before, after) = changed_states(epoch);
    let disconnected = project(
        &PlayerLifecycleInput::TransportDisconnected {
            attachment_epoch: epoch,
        },
        &before,
        &after,
    );
    assert_eq!(
        transition_ids(&disconnected),
        ["TRANSPORT-FAIL-001", "PLAYER-LOSS-001"]
    );
    assert!(
        disconnected
            .iter()
            .all(|item| item.trigger == Trigger::Fault && item.disposition == Disposition::Failed)
    );

    let reattached = project(&PlayerLifecycleInput::AttachmentReplaced, &before, &after);
    assert_eq!(transition_ids(&reattached), ["TRANSPORT-DETACH-001"]);
    assert_eq!(reattached[0].trigger, Trigger::Recovery);
    assert_eq!(reattached[0].disposition, Disposition::Superseded);
    assert_identity(&reattached[0], "attachment-epoch", 2);

    let unchanged = project(
        &PlayerLifecycleInput::TransportDisconnected {
            attachment_epoch: epoch,
        },
        &before,
        &before,
    );
    assert!(unchanged.is_empty());
}
