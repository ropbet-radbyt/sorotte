use super::*;

fn coordinator(kind: MediaTransportKind) -> (PlaybackCoordinator, u64) {
    let mut coordinator = PlaybackCoordinator::default();
    let generation = coordinator
        .prepare_media(LogicalMediaId::new("episode-1").unwrap(), kind, 0.0)
        .media_generation;
    (coordinator, generation)
}

#[test]
fn transport_refresh_preserves_room_participation_intent_for_changed_local_identity() {
    let mut coordinator = PlaybackCoordinator::default();
    let initial = coordinator.prepare_media_for_room_participation(
        LogicalMediaId::new("joined-room-episode").unwrap(),
        MediaTransportKind::LocalFile,
        1.0,
    );

    assert!(initial.logical_media_changed);
    assert!(initial.playback_episode_changed);
    assert_eq!(initial.load_intent, MediaLoadIntent::TransportRefresh);

    let replacement = coordinator.prepare_media_for_room_participation(
        LogicalMediaId::new("joined-room-episode-local-match").unwrap(),
        MediaTransportKind::LocalFile,
        2.0,
    );

    assert!(replacement.logical_media_changed);
    assert!(replacement.playback_episode_changed);
    assert_eq!(replacement.load_intent, MediaLoadIntent::TransportRefresh);
    assert_ne!(replacement.media_generation, initial.media_generation);
}

fn desired(generation: u64, revision: u64, paused: bool, position: f64) -> DesiredRoomPlayback {
    DesiredRoomPlayback {
        media_generation: generation,
        state_revision: revision,
        paused,
        anchor_position_seconds: position,
        anchor_observed_at_seconds: 0.0,
        force_seek: false,
    }
}

fn playing(generation: u64, at: f64, position: f64) -> PlayerTransportObservation {
    PlayerTransportObservation::new(generation, at)
        .with_phase(PlayerTransportPhase::Playing)
        .with_position(position)
        .with_logical_pause(false)
        .with_cache_pause(false)
        .with_seeking(false)
        .with_seekable(true)
}

fn begin_catchup_override(
    config: PlaybackCoordinatorConfig,
) -> (PlaybackCoordinator, u64, f64, CoordinatorCommandId) {
    begin_catchup_override_with_decision_rate(config, None)
}

fn begin_catchup_override_with_decision_rate(
    config: PlaybackCoordinatorConfig,
    decision_playback_rate: Option<f64>,
) -> (PlaybackCoordinator, u64, f64, CoordinatorCommandId) {
    let mut coordinator = PlaybackCoordinator::new(config);
    let generation = coordinator
        .prepare_media(
            LogicalMediaId::new("catchup-override").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;
    coordinator.update_desired_room_state(DesiredRoomPlayback {
        anchor_observed_at_seconds: 10.0,
        ..desired(generation, 1, false, 25.0)
    });
    coordinator.observe(
        PlayerTransportObservation::new(generation, 10.0)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(20.0)
            .with_logical_pause(false)
            .with_cache_pause(true),
    );
    coordinator.observe(playing(generation, 11.0, 20.5));
    let mut decision = playing(generation, 12.0, 21.0);
    if let Some(playback_rate) = decision_playback_rate {
        decision = decision.with_playback_rate(playback_rate);
    }
    let actions = coordinator.observe(decision);
    let (command_id, target_rate) = actions
        .iter()
        .find_map(|action| match action {
            PlaybackCoordinatorAction::Execute {
                command_id,
                command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
            } if *rate > NORMAL_PLAYBACK_RATE => Some((*command_id, *rate)),
            _ => None,
        })
        .expect("moderate post-buffer lag should start rate catch-up");
    (coordinator, generation, target_rate, command_id)
}

fn observe_catchup_override(
    coordinator: &mut PlaybackCoordinator,
    generation: u64,
    target_rate: f64,
    command_id: CoordinatorCommandId,
) {
    assert!(coordinator.command_accepted(command_id));
    coordinator.observe(playing(generation, 13.0, 21.5).with_playback_rate(target_rate));
}

#[test]
fn room_advancing_while_buffered_never_emits_a_correction() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    coordinator.update_desired_room_state(desired(generation, 1, false, 10.0));

    for second in 10..20 {
        let actions = coordinator.observe(
            PlayerTransportObservation::new(generation, second as f64)
                .with_phase(PlayerTransportPhase::Rebuffering)
                .with_position(10.0)
                .with_logical_pause(false)
                .with_cache_pause(true),
        );
        assert!(actions.is_empty());
    }
    assert_eq!(coordinator.metrics().hard_seek_count, 0);
    assert_eq!(coordinator.metrics().buffer_episode_count, 1);
}

#[test]
fn cache_release_with_small_lag_continues_without_seek() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    coordinator.update_desired_room_state(DesiredRoomPlayback {
        anchor_observed_at_seconds: 10.0,
        ..desired(generation, 1, false, 20.0)
    });
    coordinator.observe(
        PlayerTransportObservation::new(generation, 10.0)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(20.0)
            .with_logical_pause(false)
            .with_cache_pause(true),
    );
    coordinator.observe(playing(generation, 11.0, 20.4));
    let actions = coordinator.observe(playing(generation, 12.0, 21.4));

    assert!(!actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_),
            ..
        }
    )));
    assert_eq!(coordinator.metrics().hard_seek_count, 0);
}

#[test]
fn moderate_lag_without_headroom_uses_conservative_catchup() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    coordinator.update_desired_room_state(DesiredRoomPlayback {
        anchor_observed_at_seconds: 10.0,
        ..desired(generation, 1, false, 25.0)
    });
    coordinator.observe(
        PlayerTransportObservation::new(generation, 10.0)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(20.0)
            .with_logical_pause(false)
            .with_cache_pause(true),
    );
    coordinator.observe(playing(generation, 11.0, 20.5));
    let actions = coordinator.observe(playing(generation, 12.0, 21.0));

    assert!(actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
            ..
        } if (*rate - CONSERVATIVE_CATCHUP_RATE_WITHOUT_HEADROOM).abs() < f64::EPSILON
    )));
}

#[test]
fn buffered_headroom_allows_the_configured_catchup_rate() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    coordinator.update_desired_room_state(DesiredRoomPlayback {
        anchor_observed_at_seconds: 10.0,
        ..desired(generation, 1, false, 25.0)
    });
    let mut buffering = PlayerTransportObservation::new(generation, 10.0)
        .with_phase(PlayerTransportPhase::Rebuffering)
        .with_position(20.0)
        .with_logical_pause(false)
        .with_cache_pause(true);
    buffering.buffered_ahead_seconds = Some(5.0);
    coordinator.observe(buffering);
    coordinator.observe(playing(generation, 11.0, 20.5));
    let actions = coordinator.observe(playing(generation, 12.0, 21.0));

    assert!(actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
            ..
        } if (*rate - 1.05).abs() < f64::EPSILON
    )));
}

#[test]
fn balanced_catchup_does_not_close_before_position_converges() {
    let config = PlaybackCoordinatorConfig {
        stability_interval_seconds: 1.0,
        ..PlaybackCoordinatorConfig::default()
    };
    let mut coordinator = PlaybackCoordinator::new(config);
    let generation = coordinator
        .prepare_media(
            LogicalMediaId::new("catchup-media").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;
    coordinator.update_desired_room_state(DesiredRoomPlayback {
        anchor_observed_at_seconds: 10.0,
        ..desired(generation, 1, false, 25.0)
    });
    coordinator.observe(
        PlayerTransportObservation::new(generation, 10.0)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(20.0)
            .with_logical_pause(false)
            .with_cache_pause(true),
    );
    coordinator.observe(playing(generation, 11.0, 20.5));
    coordinator.observe(playing(generation, 12.0, 21.0));

    // More than a stability interval of healthy advancement is not enough
    // while the client remains several seconds behind.
    coordinator.observe(playing(generation, 13.5, 22.0));
    assert!(coordinator.recovery_episode().is_some());

    // Once catch-up reaches the moving room anchor, the already-stable
    // episode may close and ordinary steady-state correction can resume.
    coordinator.observe(playing(generation, 16.0, 31.0));
    assert!(coordinator.recovery_episode().is_none());
}

#[test]
fn catchup_that_never_reduces_lag_degrades_at_a_bounded_deadline() {
    let config = PlaybackCoordinatorConfig {
        maximum_catchup_rate: 1.25,
        stability_interval_seconds: 1.0,
        ..PlaybackCoordinatorConfig::default()
    };
    let mut coordinator = PlaybackCoordinator::new(config);
    let generation = coordinator
        .prepare_media(
            LogicalMediaId::new("bounded-catchup").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;
    coordinator.update_desired_room_state(DesiredRoomPlayback {
        anchor_observed_at_seconds: 10.0,
        ..desired(generation, 1, false, 25.0)
    });
    let mut stalled = PlayerTransportObservation::new(generation, 10.0)
        .with_phase(PlayerTransportPhase::Rebuffering)
        .with_position(20.0)
        .with_logical_pause(false)
        .with_cache_pause(true);
    stalled.buffered_ahead_seconds = Some(5.0);
    coordinator.observe(stalled);
    coordinator.observe(playing(generation, 11.0, 20.5));
    coordinator.observe(playing(generation, 12.0, 21.0));
    let deadline = coordinator
        .recovery_episode()
        .and_then(|episode| episode.catchup_deadline_seconds)
        .expect("catchup decision should establish a deadline");

    let mut rate_applied = playing(generation, 13.0, 22.0);
    rate_applied.playback_rate = Some(1.25);
    coordinator.observe(rate_applied);
    let actions = coordinator.observe(playing(
        generation,
        deadline + 0.1,
        22.0 + (deadline + 0.1 - 13.0),
    ));

    assert!(actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Degraded {
            reason: DegradedPlaybackReason::CatchupDidNotConverge,
            ..
        }
    )));
    assert!(actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
            ..
        } if (*rate - 1.0).abs() < f64::EPSILON
    )));
    assert!(
        coordinator
            .recovery_episode()
            .is_some_and(|episode| episode.degraded)
    );
    coordinator.observe(
        playing(generation, deadline + 0.2, 22.1 + (deadline + 0.1 - 13.0))
            .with_playback_rate(NORMAL_PLAYBACK_RATE),
    );
    assert!(
        coordinator.rate_override.is_none(),
        "degraded recovery must release rate ownership only after baseline telemetry"
    );
}

#[test]
fn repeated_cache_transitions_do_not_reset_hard_seek_budget() {
    let config = PlaybackCoordinatorConfig {
        maximum_hard_seeks_per_episode: 1,
        ..PlaybackCoordinatorConfig::default()
    };
    let mut coordinator = PlaybackCoordinator::new(config);
    let generation = coordinator
        .prepare_media(
            LogicalMediaId::new("episode-1").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;
    coordinator.update_desired_room_state(DesiredRoomPlayback {
        anchor_observed_at_seconds: 10.0,
        ..desired(generation, 1, false, 40.0)
    });
    coordinator.observe(
        PlayerTransportObservation::new(generation, 10.0)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(10.0)
            .with_logical_pause(false)
            .with_cache_pause(true),
    );
    coordinator.observe(playing(generation, 11.0, 10.2));
    let first = coordinator.observe(playing(generation, 12.0, 10.5));
    assert_eq!(
        first
            .iter()
            .filter(|action| matches!(
                action,
                PlaybackCoordinatorAction::Execute {
                    command: CoordinatorPlayerCommand::SetPosition(_),
                    ..
                }
            ))
            .count(),
        1
    );

    let renewed_stall = coordinator.observe(
        PlayerTransportObservation::new(generation, 13.0)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(10.5)
            .with_logical_pause(false)
            .with_cache_pause(true),
    );
    assert!(renewed_stall.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Degraded {
            reason: DegradedPlaybackReason::HardSeekBudgetExhausted,
            ..
        }
    )));
    coordinator.observe(playing(generation, 14.0, 10.7));
    let second = coordinator.observe(playing(generation, 15.0, 11.0));
    assert!(!second.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_),
            ..
        }
    )));
    assert_eq!(coordinator.metrics().hard_seek_count, 1);
    assert!(
        coordinator
            .recovery_episode()
            .is_some_and(|episode| episode.degraded)
    );
}

#[test]
fn large_residual_lag_after_recovery_seek_degrades_without_second_seek() {
    let config = PlaybackCoordinatorConfig {
        maximum_hard_seeks_per_episode: 1,
        ..PlaybackCoordinatorConfig::default()
    };
    let mut coordinator = PlaybackCoordinator::new(config);
    let generation = coordinator
        .prepare_media(
            LogicalMediaId::new("episode-1").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;
    coordinator.update_desired_room_state(DesiredRoomPlayback {
        anchor_observed_at_seconds: 10.0,
        ..desired(generation, 1, false, 40.0)
    });
    coordinator.observe(
        PlayerTransportObservation::new(generation, 10.0)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(10.0)
            .with_logical_pause(false)
            .with_cache_pause(true),
    );
    coordinator.observe(playing(generation, 11.0, 10.2));
    let first = coordinator.observe(playing(generation, 12.0, 10.5));
    let target = first
        .iter()
        .find_map(|action| match action {
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPosition(target),
                ..
            } => Some(*target),
            _ => None,
        })
        .expect("recovery should issue its one hard seek");

    coordinator.observe(
        PlayerTransportObservation::new(generation, 29.0)
            .with_phase(PlayerTransportPhase::Seeking)
            .with_position(target)
            .with_logical_pause(false)
            .with_seeking(true)
            .with_seekable(true),
    );
    coordinator.observe(playing(generation, 30.0, target));
    let residual = coordinator.observe(playing(generation, 31.0, target + 0.5));

    assert!(!residual.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_),
            ..
        }
    )));
    assert!(residual.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Degraded {
            reason: DegradedPlaybackReason::HardSeekBudgetExhausted,
            ..
        }
    )));
    assert_eq!(coordinator.metrics().hard_seek_count, 1);
}

#[test]
fn play_received_during_loading_remains_pending_until_advancement() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    coordinator.update_desired_room_state(desired(generation, 7, false, 5.0));
    assert!(
        coordinator
            .observe(
                PlayerTransportObservation::new(generation, 1.0)
                    .with_phase(PlayerTransportPhase::Loading)
                    .with_position(0.0)
                    .with_logical_pause(true)
            )
            .is_empty()
    );
    assert_eq!(coordinator.desired_revision_pending(), Some(7));

    let actions = coordinator.observe(
        PlayerTransportObservation::new(generation, 2.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_seekable(true),
    );
    let play_id = actions
        .iter()
        .find_map(|action| match action {
            PlaybackCoordinatorAction::Execute {
                command_id,
                command: CoordinatorPlayerCommand::Play(_),
            } => Some(*command_id),
            _ => None,
        })
        .expect("ready player should receive retained play");
    assert!(coordinator.command_accepted(play_id));
    assert_eq!(coordinator.desired_revision_pending(), Some(7));

    coordinator.observe(playing(generation, 3.0, 5.0).with_restart_sequence(1));
    let applied = coordinator.observe(playing(generation, 4.0, 5.5).with_restart_sequence(1));
    assert!(applied.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::RevisionApplied {
            state_revision: 7,
            ..
        }
    )));
    assert_eq!(coordinator.desired_revision_pending(), None);
}

#[test]
fn ready_paused_core_idle_allows_prepare_seek_and_play() {
    let (mut paused_coordinator, paused_generation) = coordinator(MediaTransportKind::NetworkVod);
    paused_coordinator.update_desired_room_state(DesiredRoomPlayback {
        force_seek: true,
        ..desired(paused_generation, 1, true, 12.0)
    });
    let seek_actions = paused_coordinator.observe(
        PlayerTransportObservation::new(paused_generation, 1.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(0.0)
            .with_logical_pause(true)
            .with_seekable(true)
            .with_core_idle(true),
    );
    assert!(seek_actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if (*position - 12.0).abs() < f64::EPSILON
    )));

    let (mut playing_coordinator, playing_generation) = coordinator(MediaTransportKind::NetworkVod);
    playing_coordinator.update_desired_room_state(desired(playing_generation, 1, false, 0.0));
    let play_actions = playing_coordinator.observe(
        PlayerTransportObservation::new(playing_generation, 1.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(0.0)
            .with_logical_pause(true)
            .with_seekable(true)
            .with_core_idle(true),
    );
    assert!(play_actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::Play(PlayerPlayIntent::StartAfterLoad { .. }),
            ..
        }
    )));
}

#[test]
fn authoritative_pause_aligns_position_before_sending_pause() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    coordinator.update_desired_room_state(desired(generation, 1, false, 10.0));
    coordinator.observe(playing(generation, 1.0, 10.0).with_restart_sequence(1));
    coordinator.observe(playing(generation, 1.1, 10.1).with_restart_sequence(1));

    coordinator.update_desired_room_state(DesiredRoomPlayback {
        media_generation: generation,
        state_revision: 2,
        paused: true,
        anchor_position_seconds: 12.0,
        anchor_observed_at_seconds: 2.0,
        force_seek: true,
    });
    let seek_first = coordinator.observe(playing(generation, 2.0, 10.2));
    assert!(seek_first.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if (*position - 12.0).abs() < f64::EPSILON
    )));
    assert!(!seek_first.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPaused(true),
            ..
        }
    )));

    let pause_second = coordinator.observe(
        playing(generation, 2.1, 12.0)
            .with_seeking(false)
            .with_restart_sequence(2),
    );
    assert!(pause_second.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPaused(true),
            ..
        }
    )));
}

#[test]
fn ordinary_resume_completes_without_a_new_playback_restart() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    coordinator.update_desired_room_state(desired(generation, 1, false, 0.0));
    let initial = coordinator.observe(
        PlayerTransportObservation::new(generation, 0.1)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(0.0)
            .with_logical_pause(true)
            .with_seekable(true)
            .with_core_idle(true),
    );
    assert!(initial.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::Play(PlayerPlayIntent::StartAfterLoad { .. }),
            ..
        }
    )));
    coordinator.observe(playing(generation, 0.2, 0.0).with_restart_sequence(1));
    coordinator.observe(playing(generation, 0.3, 0.1).with_restart_sequence(1));

    coordinator.update_desired_room_state(desired(generation, 2, true, 0.1));
    coordinator.observe(playing(generation, 0.4, 0.2).with_restart_sequence(1));
    coordinator.observe(
        PlayerTransportObservation::new(generation, 0.5)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(0.2)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seekable(true)
            .with_core_idle(true)
            .with_restart_sequence(1),
    );

    coordinator.update_desired_room_state(desired(generation, 3, false, 0.2));
    let resume = coordinator.observe(
        PlayerTransportObservation::new(generation, 0.6)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(0.2)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seekable(true)
            .with_core_idle(true)
            .with_restart_sequence(1),
    );
    assert!(resume.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::Play(PlayerPlayIntent::Resume),
            ..
        }
    )));
    let resumed = coordinator.observe(
        playing(generation, 0.7, 0.3)
            .with_restart_sequence(1)
            .with_core_idle(false),
    );
    assert!(resumed.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::RevisionApplied {
            state_revision: 3,
            ..
        } | PlaybackCoordinatorAction::Started {
            state_revision: 3,
            ..
        }
    )));
    assert_eq!(coordinator.desired_revision_pending(), None);
}

#[test]
fn play_after_paused_seek_uses_the_seek_start_restart_baseline() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    coordinator.update_desired_room_state(DesiredRoomPlayback {
        force_seek: true,
        ..desired(generation, 1, true, 10.0)
    });
    let seek = coordinator.observe(
        PlayerTransportObservation::new(generation, 1.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(0.0)
            .with_logical_pause(true)
            .with_seekable(true)
            .with_core_idle(true)
            .with_restart_sequence(5),
    );
    assert!(seek.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if (*position - 10.0).abs() < f64::EPSILON
    )));
    coordinator.observe(
        PlayerTransportObservation::new(generation, 1.1)
            .with_phase(PlayerTransportPhase::Seeking)
            .with_position(10.0)
            .with_logical_pause(true)
            .with_seeking(true)
            .with_seekable(true)
            .with_restart_sequence(5),
    );
    coordinator.observe(
        PlayerTransportObservation::new(generation, 1.2)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(10.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true)
            .with_core_idle(true)
            .with_restart_sequence(6),
    );

    coordinator.update_desired_room_state(desired(generation, 2, false, 10.0));
    let play = coordinator.observe(
        PlayerTransportObservation::new(generation, 1.3)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(10.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seekable(true)
            .with_core_idle(true)
            .with_restart_sequence(6),
    );
    assert!(play.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::Play(PlayerPlayIntent::StartAfterSeek {
                baseline_restart_sequence: 5,
            }),
            ..
        }
    )));
    let started = coordinator.observe(
        playing(generation, 1.4, 10.1)
            .with_core_idle(false)
            .with_restart_sequence(6),
    );
    assert!(started.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Started {
            state_revision: 2,
            ..
        }
    )));
}

#[test]
fn pause_received_during_recovery_supersedes_catchup_without_a_recovery_seek() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    coordinator.update_desired_room_state(DesiredRoomPlayback {
        anchor_observed_at_seconds: 10.0,
        ..desired(generation, 1, false, 25.0)
    });
    coordinator.observe(
        PlayerTransportObservation::new(generation, 10.0)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(20.0)
            .with_logical_pause(false)
            .with_cache_pause(true),
    );
    coordinator.observe(playing(generation, 11.0, 20.5));

    coordinator.update_desired_room_state(DesiredRoomPlayback {
        anchor_observed_at_seconds: 12.0,
        ..desired(generation, 2, true, 26.0)
    });
    let actions = coordinator.observe(playing(generation, 12.0, 21.0));

    assert!(actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPaused(true),
            ..
        }
    )));
    assert!(!actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
            ..
        } if *rate > NORMAL_PLAYBACK_RATE
    )));
    assert!(coordinator.recovery_episode().is_none());
}

#[test]
fn authoritative_pause_latches_in_every_transport_blocked_phase_without_seeking() {
    let blocked_states = [
        (PlayerTransportPhase::Empty, false, false),
        (PlayerTransportPhase::Loading, false, false),
        (PlayerTransportPhase::Prebuffering, false, false),
        (PlayerTransportPhase::Rebuffering, true, false),
        (PlayerTransportPhase::Seeking, false, true),
        (PlayerTransportPhase::Ended, false, false),
        (PlayerTransportPhase::Failed, false, false),
    ];

    for (phase, paused_for_cache, seeking) in blocked_states {
        let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
        coordinator.update_desired_room_state(DesiredRoomPlayback {
            force_seek: true,
            ..desired(generation, 1, true, 12.0)
        });

        let actions = coordinator.observe(
            PlayerTransportObservation::new(generation, 1.0)
                .with_phase(phase)
                .with_position(5.0)
                .with_logical_pause(false)
                .with_cache_pause(paused_for_cache)
                .with_seeking(seeking)
                .with_seekable(true),
        );

        assert!(
            actions.iter().any(|action| matches!(
                action,
                PlaybackCoordinatorAction::Execute {
                    command: CoordinatorPlayerCommand::SetPaused(true),
                    ..
                }
            )),
            "authoritative pause was not latched during {phase:?}: {actions:?}"
        );
        assert!(
            !actions.iter().any(|action| matches!(
                action,
                PlaybackCoordinatorAction::Execute {
                    command: CoordinatorPlayerCommand::SetPosition(_),
                    ..
                }
            )),
            "position correction must remain deferred during {phase:?}: {actions:?}"
        );
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, PlaybackCoordinatorAction::RevisionApplied { .. })),
            "pause dispatch alone must not complete the revision during {phase:?}"
        );
    }
}

#[test]
fn cache_pause_suspends_pause_timeout_and_defers_seek_and_revision_completion() {
    let config = PlaybackCoordinatorConfig {
        command_timeout_seconds: 1.0,
        ..PlaybackCoordinatorConfig::default()
    };
    let mut coordinator = PlaybackCoordinator::new(config);
    let generation = coordinator
        .prepare_media(
            LogicalMediaId::new("cache-paused-authoritative-pause").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;
    coordinator.update_desired_room_state(DesiredRoomPlayback {
        force_seek: true,
        ..desired(generation, 1, true, 12.0)
    });

    let latch = coordinator.observe(
        PlayerTransportObservation::new(generation, 1.0)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(5.0)
            .with_logical_pause(false)
            .with_cache_pause(true)
            .with_seeking(false)
            .with_seekable(true),
    );
    let pause_id = latch
        .iter()
        .find_map(|action| match action {
            PlaybackCoordinatorAction::Execute {
                command_id,
                command: CoordinatorPlayerCommand::SetPaused(true),
            } => Some(*command_id),
            _ => None,
        })
        .expect("cache-paused transport should latch authoritative pause");
    assert!(coordinator.command_accepted(pause_id));

    let stalled_tick = coordinator.tick(20.0);
    assert!(
        !stalled_tick.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::CommandTimedOut { command_id }
                if *command_id == pause_id
        )),
        "logical-pause acknowledgement is masked during cache pause"
    );
    assert_eq!(coordinator.metrics().command_timeouts, 0);

    let cache_released = coordinator.observe(
        PlayerTransportObservation::new(generation, 20.1)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true),
    );
    assert!(cache_released.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if (*position - 12.0).abs() < f64::EPSILON
    )));
    assert!(
        !cache_released
            .iter()
            .any(|action| matches!(action, PlaybackCoordinatorAction::RevisionApplied { .. }))
    );

    let seeking = coordinator.observe(
        PlayerTransportObservation::new(generation, 20.2)
            .with_phase(PlayerTransportPhase::Seeking)
            .with_position(12.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(true)
            .with_seekable(true),
    );
    assert!(
        !seeking
            .iter()
            .any(|action| matches!(action, PlaybackCoordinatorAction::RevisionApplied { .. }))
    );

    let applied = coordinator.observe(
        PlayerTransportObservation::new(generation, 20.3)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(12.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true),
    );
    assert!(applied.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::RevisionApplied {
            state_revision: 1,
            ..
        }
    )));
}

#[test]
fn explicit_room_seek_supersedes_the_old_recovery_target() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    coordinator.update_desired_room_state(DesiredRoomPlayback {
        anchor_observed_at_seconds: 10.0,
        ..desired(generation, 1, false, 40.0)
    });
    coordinator.observe(
        PlayerTransportObservation::new(generation, 10.0)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(10.0)
            .with_logical_pause(false)
            .with_cache_pause(true),
    );

    coordinator.update_desired_room_state(DesiredRoomPlayback {
        media_generation: generation,
        state_revision: 2,
        paused: false,
        anchor_position_seconds: 8.0,
        anchor_observed_at_seconds: 11.0,
        force_seek: true,
    });
    let actions = coordinator.observe(playing(generation, 11.0, 10.0));

    assert!(actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if (*position - 8.0).abs() < f64::EPSILON
    )));
    assert!(coordinator.recovery_episode().is_none());
    assert_eq!(coordinator.metrics().hard_seek_count, 0);
}

#[test]
fn pending_desired_revision_and_command_both_block_legacy_correction() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    coordinator.update_desired_room_state(desired(generation, 1, true, 0.0));
    assert!(coordinator.ordinary_correction_blocked());

    coordinator.observe(
        PlayerTransportObservation::new(generation, 0.1)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(0.0)
            .with_logical_pause(true)
            .with_seekable(true)
            .with_core_idle(true),
    );
    assert!(!coordinator.ordinary_correction_blocked());

    coordinator.update_desired_room_state(desired(generation, 2, false, 0.0));
    let play = coordinator.observe(
        PlayerTransportObservation::new(generation, 0.2)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(0.0)
            .with_logical_pause(true)
            .with_seekable(true)
            .with_core_idle(true),
    );
    assert!(play.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::Play(_),
            ..
        }
    )));
    assert!(coordinator.ordinary_correction_blocked());

    coordinator.observe(
        playing(generation, 0.3, 0.1)
            .with_restart_sequence(1)
            .with_core_idle(false),
    );
    assert!(!coordinator.ordinary_correction_blocked());
}

#[test]
fn newer_forced_revision_supersedes_pending_barrier_seek() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    coordinator.update_desired_room_state(DesiredRoomPlayback {
        force_seek: true,
        ..desired(generation, 1, true, 10.0)
    });
    let first = coordinator.observe(
        PlayerTransportObservation::new(generation, 0.1)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(0.0)
            .with_logical_pause(true)
            .with_seekable(true)
            .with_core_idle(true),
    );
    assert!(first.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if (*position - 10.0).abs() < f64::EPSILON
    )));

    coordinator.update_desired_room_state(DesiredRoomPlayback {
        force_seek: true,
        ..desired(generation, 2, true, 20.0)
    });
    let superseding = coordinator.observe(
        PlayerTransportObservation::new(generation, 0.2)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(0.0)
            .with_logical_pause(true)
            .with_seekable(true)
            .with_core_idle(true),
    );
    assert!(superseding.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if (*position - 20.0).abs() < f64::EPSILON
    )));
    assert!(!superseding.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if (*position - 10.0).abs() < f64::EPSILON
    )));

    let stale_target = coordinator.observe(
        PlayerTransportObservation::new(generation, 0.3)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(10.0)
            .with_logical_pause(true)
            .with_seekable(true)
            .with_core_idle(true),
    );
    assert!(!stale_target.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::RevisionApplied {
            state_revision: 2,
            ..
        }
    )));
    assert_eq!(coordinator.desired_revision_pending(), Some(2));

    let applied = coordinator.observe(
        PlayerTransportObservation::new(generation, 0.4)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(20.0)
            .with_logical_pause(true)
            .with_seekable(true)
            .with_core_idle(true),
    );
    assert!(applied.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::RevisionApplied {
            state_revision: 2,
            ..
        }
    )));
    assert_eq!(coordinator.desired_revision_pending(), None);
}

#[test]
fn superseded_seek_landing_after_replacement_completion_rearms_alignment_before_started() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    let old = begin_fetch_required_seek(&mut coordinator, generation, 1, 40.0);
    assert!(old.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(40.0),
            ..
        }
    )));
    assert!(coordinator.cancel_seek_preparation_for_lifecycle());
    coordinator.clear_seek_preparation_terminal();
    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            media_generation: generation,
            state_revision: 2,
            paused: false,
            anchor_position_seconds: 5.0,
            anchor_observed_at_seconds: 2.0,
            force_seek: true,
        },
        DesiredRoomPlaybackUpdateKind::AuthoritativeSeekAfterSupersededDispatch,
    );

    let replacement = coordinator.observe(
        PlayerTransportObservation::new(generation, 2.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_seeking(false)
            .with_seekable(true),
    );
    assert!(replacement.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(5.0),
            ..
        }
    )));
    coordinator.observe(
        PlayerTransportObservation::new(generation, 2.1)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.1)
            .with_logical_pause(true)
            .with_seeking(false)
            .with_seekable(true),
    );

    let late_old_seek = coordinator.observe(
        PlayerTransportObservation::new(generation, 2.2)
            .with_phase(PlayerTransportPhase::Playing)
            .with_position(40.0)
            .with_logical_pause(false)
            .with_seeking(false)
            .with_seekable(true),
    );
    assert!(late_old_seek.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if (*position - 5.2).abs() <= f64::EPSILON
    )));
    assert!(!late_old_seek.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::RevisionApplied {
            state_revision: 2,
            ..
        } | PlaybackCoordinatorAction::Started {
            state_revision: 2,
            ..
        }
    )));
}

#[test]
fn newer_explicit_seek_fences_a_late_superseded_preparation_seek() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    let old = begin_fetch_required_seek(&mut coordinator, generation, 1, 40.0);
    assert!(old.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(40.0),
            ..
        }
    )));

    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            media_generation: generation,
            state_revision: 2,
            paused: false,
            anchor_position_seconds: 80.0,
            anchor_observed_at_seconds: 2.0,
            force_seek: true,
        },
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );
    assert_eq!(coordinator.authoritative_alignment_guard_revision, Some(2));
    assert_eq!(coordinator.required_seek_dispatch_revision, Some(2));

    let replacement = coordinator.observe(
        PlayerTransportObservation::new(generation, 2.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_seeking(false)
            .with_seekable(true)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(70.0, 90.0)]),
    );
    assert!(replacement.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(80.0),
            ..
        }
    )));

    coordinator.observe(
        PlayerTransportObservation::new(generation, 2.1)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(80.1)
            .with_logical_pause(true)
            .with_seeking(false)
            .with_seekable(true)
            .with_buffered_ahead_seconds(4.0),
    );

    let late_old_seek = coordinator.observe(
        PlayerTransportObservation::new(generation, 2.2)
            .with_phase(PlayerTransportPhase::Playing)
            .with_position(40.0)
            .with_logical_pause(false)
            .with_seeking(false)
            .with_seekable(true),
    );
    assert!(late_old_seek.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if (*position - 80.2).abs() <= f64::EPSILON
    )));
    assert!(!late_old_seek.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Started {
            state_revision: 2,
            ..
        }
    )));
}

#[test]
fn newer_explicit_seek_fences_a_late_superseded_cached_seek() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    coordinator.observe(
        PlayerTransportObservation::new(generation, 0.5)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_seeking(false)
            .with_seekable(true)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(0.0, 100.0)]),
    );
    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            media_generation: generation,
            state_revision: 1,
            paused: false,
            anchor_position_seconds: 40.0,
            anchor_observed_at_seconds: 1.0,
            force_seek: true,
        },
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );
    assert!(coordinator.seek_preparation_snapshot().is_none());
    let old = coordinator.observe(
        PlayerTransportObservation::new(generation, 1.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_seeking(false)
            .with_seekable(true)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(0.0, 100.0)]),
    );
    assert!(old.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(40.0),
            ..
        }
    )));

    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            media_generation: generation,
            state_revision: 2,
            paused: false,
            anchor_position_seconds: 80.0,
            anchor_observed_at_seconds: 2.0,
            force_seek: true,
        },
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );
    assert_eq!(coordinator.authoritative_alignment_guard_revision, Some(2));
    let replacement = coordinator.observe(
        PlayerTransportObservation::new(generation, 2.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_seeking(false)
            .with_seekable(true)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(0.0, 100.0)]),
    );
    assert!(replacement.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(80.0),
            ..
        }
    )));
    coordinator.observe(
        PlayerTransportObservation::new(generation, 2.1)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(80.1)
            .with_logical_pause(true)
            .with_seeking(false)
            .with_seekable(true),
    );

    let late_old_seek = coordinator.observe(
        PlayerTransportObservation::new(generation, 2.2)
            .with_phase(PlayerTransportPhase::Playing)
            .with_position(40.0)
            .with_logical_pause(false)
            .with_seeking(false)
            .with_seekable(true),
    );
    assert!(late_old_seek.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if (*position - 80.2).abs() <= f64::EPSILON
    )));
    assert!(!late_old_seek.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Started {
            state_revision: 2,
            ..
        }
    )));
}

#[test]
fn forced_barrier_revision_fences_a_late_superseded_direct_seek() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::LocalFile);
    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            media_generation: generation,
            state_revision: 1,
            paused: false,
            anchor_position_seconds: 40.0,
            anchor_observed_at_seconds: 1.0,
            force_seek: true,
        },
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );
    let old = coordinator.observe(
        PlayerTransportObservation::new(generation, 1.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_seeking(false)
            .with_seekable(true),
    );
    assert!(old.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(40.0),
            ..
        }
    )));

    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            media_generation: generation,
            state_revision: 2,
            paused: false,
            anchor_position_seconds: 20.0,
            anchor_observed_at_seconds: 2.0,
            force_seek: true,
        },
        DesiredRoomPlaybackUpdateKind::Ordinary,
    );
    assert_eq!(coordinator.authoritative_alignment_guard_revision, Some(2));
    let replacement = coordinator.observe(
        PlayerTransportObservation::new(generation, 2.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_seeking(false)
            .with_seekable(true),
    );
    assert!(replacement.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(20.0),
            ..
        }
    )));
    coordinator.observe(
        PlayerTransportObservation::new(generation, 2.1)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(20.1)
            .with_logical_pause(true)
            .with_seeking(false)
            .with_seekable(true),
    );

    let late_old_seek = coordinator.observe(
        PlayerTransportObservation::new(generation, 2.2)
            .with_phase(PlayerTransportPhase::Playing)
            .with_position(40.0)
            .with_logical_pause(false)
            .with_seeking(false)
            .with_seekable(true),
    );
    assert!(late_old_seek.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if (*position - 20.2).abs() <= f64::EPSILON
    )));
    assert!(!late_old_seek.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Started {
            state_revision: 2,
            ..
        }
    )));
}

#[test]
fn supersession_fence_survives_unknown_timeline_and_barrier_revision_advance() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    begin_fetch_required_seek(&mut coordinator, generation, 1, 40.0);
    assert!(coordinator.cancel_seek_preparation_for_lifecycle());
    coordinator.clear_seek_preparation_terminal();
    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            force_seek: true,
            ..desired(generation, 2, true, 20.0)
        },
        DesiredRoomPlaybackUpdateKind::AuthoritativeSeekAfterSupersededDispatch,
    );
    let waiting = coordinator.observe(
        PlayerTransportObservation::new(generation, 2.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(20.0)
            .with_logical_pause(true)
            .with_seeking(false)
            .with_seekable(true)
            .with_timeline_kind(PlayerTimelineKind::Unknown)
            .with_seekable_ranges(Vec::new()),
    );
    assert!(coordinator.seek_preparation_snapshot().is_some());
    assert!(!waiting.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_),
            ..
        }
    )));

    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            force_seek: false,
            ..desired(generation, 3, true, 20.0)
        },
        DesiredRoomPlaybackUpdateKind::Ordinary,
    );
    assert_eq!(coordinator.required_seek_dispatch_revision, Some(3));
    assert_eq!(coordinator.authoritative_alignment_guard_revision, Some(3));
    let still_waiting = coordinator.observe(
        PlayerTransportObservation::new(generation, 2.1)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(20.0)
            .with_logical_pause(true)
            .with_seeking(false)
            .with_seekable(true)
            .with_timeline_kind(PlayerTimelineKind::Unknown)
            .with_seekable_ranges(Vec::new()),
    );
    assert!(!still_waiting.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_),
            ..
        }
    )));

    let classified = coordinator.observe(
        PlayerTransportObservation::new(generation, 2.2)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(20.0)
            .with_logical_pause(true)
            .with_seeking(false)
            .with_seekable(true)
            .with_timeline_kind(PlayerTimelineKind::Unknown)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(15.0, 25.0)]),
    );
    assert_eq!(
        classified
            .iter()
            .filter(|action| matches!(
                action,
                PlaybackCoordinatorAction::Execute {
                    command: CoordinatorPlayerCommand::SetPosition(20.0),
                    ..
                }
            ))
            .count(),
        1
    );
}

#[test]
fn supersession_fence_survives_timeout_interrupt_and_same_barrier_reinstall() {
    let mut coordinator = PlaybackCoordinator::new(PlaybackCoordinatorConfig {
        command_timeout_seconds: 1.0,
        seek_preparation_timeout_seconds: 2.0,
        ..PlaybackCoordinatorConfig::default()
    });
    let generation = coordinator
        .prepare_media(
            LogicalMediaId::new("guard-through-timeout").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;
    begin_fetch_required_seek(&mut coordinator, generation, 1, 40.0);
    assert!(coordinator.cancel_seek_preparation_for_lifecycle());
    coordinator.clear_seek_preparation_terminal();
    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            media_generation: generation,
            state_revision: 2,
            paused: false,
            anchor_position_seconds: 20.0,
            anchor_observed_at_seconds: 2.0,
            force_seek: true,
        },
        DesiredRoomPlaybackUpdateKind::AuthoritativeSeekAfterSupersededDispatch,
    );
    coordinator.observe(
        PlayerTransportObservation::new(generation, 2.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(20.0)
            .with_logical_pause(true)
            .with_seekable(true)
            .with_timeline_kind(PlayerTimelineKind::Unknown)
            .with_seekable_ranges(Vec::new()),
    );
    coordinator.tick(4.1);
    assert!(matches!(
        coordinator.last_seek_preparation_terminal_outcome(),
        Some(SeekPreparationTerminalOutcome::Degraded(
            SeekPreparationDegradedReason::TimelineWindowUnavailable
        ))
    ));
    coordinator.interrupt_recovery();
    assert!(coordinator.desired.is_none());
    assert_eq!(coordinator.authoritative_alignment_guard_revision, Some(2));

    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            media_generation: generation,
            state_revision: 3,
            paused: false,
            anchor_position_seconds: 20.0,
            anchor_observed_at_seconds: 5.0,
            force_seek: false,
        },
        DesiredRoomPlaybackUpdateKind::Ordinary,
    );
    assert_eq!(coordinator.authoritative_alignment_guard_revision, Some(3));
    let replacement = coordinator.observe(
        PlayerTransportObservation::new(generation, 5.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(20.0)
            .with_logical_pause(true)
            .with_seekable(true)
            .with_timeline_kind(PlayerTimelineKind::Unknown)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(15.0, 25.0)]),
    );
    assert!(replacement.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(20.0),
            ..
        }
    )));
    coordinator.observe(
        PlayerTransportObservation::new(generation, 5.1)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(20.1)
            .with_logical_pause(true)
            .with_seekable(true)
            .with_timeline_kind(PlayerTimelineKind::Unknown),
    );
    let late_old = coordinator.observe(
        PlayerTransportObservation::new(generation, 5.2)
            .with_phase(PlayerTransportPhase::Playing)
            .with_position(40.0)
            .with_logical_pause(false)
            .with_seekable(true)
            .with_timeline_kind(PlayerTimelineKind::Unknown)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(35.0, 45.0)]),
    );
    assert!(late_old.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPaused(true),
            ..
        }
    )));
    assert!(!late_old.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_) | CoordinatorPlayerCommand::Play(_),
            ..
        } | PlaybackCoordinatorAction::Started {
            state_revision: 3,
            ..
        }
    )));
}

#[test]
fn accepted_command_without_matching_observation_times_out_and_stays_pending() {
    let config = PlaybackCoordinatorConfig {
        command_timeout_seconds: 1.0,
        command_retry_budget: 0,
        ..PlaybackCoordinatorConfig::default()
    };
    let mut coordinator = PlaybackCoordinator::new(config);
    let generation = coordinator
        .prepare_media(
            LogicalMediaId::new("timeout-media").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;
    coordinator.update_desired_room_state(desired(generation, 9, false, 0.0));
    let actions = coordinator.observe(
        PlayerTransportObservation::new(generation, 0.1)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(0.0)
            .with_logical_pause(true)
            .with_seekable(true),
    );
    let command_id = actions
        .iter()
        .find_map(|action| match action {
            PlaybackCoordinatorAction::Execute {
                command_id,
                command: CoordinatorPlayerCommand::Play(_),
            } => Some(*command_id),
            _ => None,
        })
        .expect("retained desired play should emit a tracked command");
    assert!(coordinator.command_accepted(command_id));

    let timed_out = coordinator.tick(1.2);

    assert!(timed_out.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::CommandTimedOut { command_id: id } if *id == command_id
    )));
    assert!(timed_out.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Degraded {
            reason: DegradedPlaybackReason::RecoveryCommandTimedOut,
            ..
        }
    )));
    assert_eq!(coordinator.desired_revision_pending(), Some(9));
    assert_eq!(coordinator.metrics().command_timeouts, 1);
}

#[test]
fn rejected_command_exhausts_budget_once_and_waits_for_new_room_intent() {
    let config = PlaybackCoordinatorConfig {
        command_retry_budget: 0,
        command_retry_cooldown_seconds: 0.1,
        ..PlaybackCoordinatorConfig::default()
    };
    let mut coordinator = PlaybackCoordinator::new(config);
    let generation = coordinator
        .prepare_media(
            LogicalMediaId::new("rejected-command").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;
    coordinator.update_desired_room_state(desired(generation, 1, false, 0.0));
    let ready_paused = PlayerTransportObservation::new(generation, 0.1)
        .with_phase(PlayerTransportPhase::ReadyPaused)
        .with_position(0.0)
        .with_logical_pause(true)
        .with_seekable(true);
    let first = coordinator.observe(ready_paused.clone());
    let command_id = first
        .iter()
        .find_map(|action| match action {
            PlaybackCoordinatorAction::Execute { command_id, .. } => Some(*command_id),
            _ => None,
        })
        .expect("play command should be issued");
    assert!(coordinator.command_failed(command_id, 0.2));

    let degraded = coordinator.tick(0.2);
    assert_eq!(
        degraded
            .iter()
            .filter(|action| matches!(
                action,
                PlaybackCoordinatorAction::Degraded {
                    reason: DegradedPlaybackReason::RecoveryCommandTimedOut,
                    ..
                }
            ))
            .count(),
        1
    );
    assert!(coordinator.tick(1.0).is_empty());
    assert!(coordinator.observe(ready_paused).is_empty());

    coordinator.update_desired_room_state(desired(generation, 2, false, 0.0));
    assert!(
        coordinator
            .observe(
                PlayerTransportObservation::new(generation, 1.1)
                    .with_phase(PlayerTransportPhase::ReadyPaused)
                    .with_position(0.0)
                    .with_logical_pause(true)
                    .with_seekable(true),
            )
            .iter()
            .any(|action| matches!(action, PlaybackCoordinatorAction::Execute { .. }))
    );
}

#[test]
fn playback_restart_without_advancement_does_not_acknowledge_desired_play() {
    let config = PlaybackCoordinatorConfig {
        command_timeout_seconds: 1.0,
        command_retry_budget: 0,
        ..PlaybackCoordinatorConfig::default()
    };
    let mut coordinator = PlaybackCoordinator::new(config);
    let generation = coordinator
        .prepare_media(
            LogicalMediaId::new("restart-without-advance").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;
    coordinator.update_desired_room_state(desired(generation, 3, false, 4.0));
    let actions = coordinator.observe(
        PlayerTransportObservation::new(generation, 0.1)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(4.0)
            .with_logical_pause(true)
            .with_seekable(true),
    );
    let play_id = actions
        .iter()
        .find_map(|action| match action {
            PlaybackCoordinatorAction::Execute {
                command_id,
                command: CoordinatorPlayerCommand::Play(_),
            } => Some(*command_id),
            _ => None,
        })
        .expect("ready transport should receive retained play");
    coordinator.command_accepted(play_id);

    let restarted = coordinator.observe(playing(generation, 0.2, 4.0).with_restart_sequence(1));
    assert!(!restarted.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::RevisionApplied { .. }
            | PlaybackCoordinatorAction::Started { .. }
    )));
    assert_eq!(coordinator.desired_revision_pending(), Some(3));

    let timed_out = coordinator.tick(1.2);
    assert!(timed_out.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::CommandTimedOut { command_id } if *command_id == play_id
    )));
    assert!(timed_out.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Degraded {
            reason: DegradedPlaybackReason::RecoveryCommandTimedOut,
            ..
        }
    )));
}

#[test]
fn non_seekable_large_lag_degrades_instead_of_chasing_the_room() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NonSeekable);
    coordinator.update_desired_room_state(DesiredRoomPlayback {
        anchor_observed_at_seconds: 10.0,
        ..desired(generation, 1, false, 40.0)
    });
    coordinator.observe(
        PlayerTransportObservation::new(generation, 10.0)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(10.0)
            .with_logical_pause(false)
            .with_cache_pause(true)
            .with_seekable(false),
    );
    coordinator.observe(playing(generation, 11.0, 10.2).with_seekable(false));
    let actions = coordinator.observe(playing(generation, 12.0, 10.5).with_seekable(false));

    assert!(actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Degraded {
            reason: DegradedPlaybackReason::NonSeekableLag,
            ..
        }
    )));
    assert!(!actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_),
            ..
        }
    )));
    assert_eq!(coordinator.metrics().hard_seek_count, 0);
}

#[test]
fn non_seekable_moderate_lag_terminates_recovery_explicitly() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NonSeekable);
    coordinator.update_desired_room_state(DesiredRoomPlayback {
        anchor_observed_at_seconds: 10.0,
        ..desired(generation, 1, false, 25.0)
    });
    coordinator.observe(
        PlayerTransportObservation::new(generation, 10.0)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(20.0)
            .with_logical_pause(false)
            .with_cache_pause(true)
            .with_seekable(false),
    );
    coordinator.observe(playing(generation, 11.0, 20.5).with_seekable(false));
    let actions = coordinator.observe(playing(generation, 12.0, 21.0).with_seekable(false));

    assert!(actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Degraded {
            reason: DegradedPlaybackReason::NonSeekableLag,
            ..
        }
    )));
    assert!(
        coordinator
            .recovery_episode()
            .is_some_and(|episode| episode.degraded)
    );
    assert!(!actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_)
                | CoordinatorPlayerCommand::SetPlaybackRate(_),
            ..
        }
    )));
}

#[test]
fn preserve_content_reports_degraded_lag_and_keeps_seek_correction_blocked() {
    let config = PlaybackCoordinatorConfig {
        recovery_policy: RecoveryPolicy::PreserveContent,
        stability_interval_seconds: 1.0,
        ..PlaybackCoordinatorConfig::default()
    };
    let mut coordinator = PlaybackCoordinator::new(config);
    let generation = coordinator
        .prepare_media(
            LogicalMediaId::new("preserve-content").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;
    coordinator.update_desired_room_state(DesiredRoomPlayback {
        anchor_observed_at_seconds: 10.0,
        ..desired(generation, 1, false, 40.0)
    });
    coordinator.observe(
        PlayerTransportObservation::new(generation, 10.0)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(10.0)
            .with_logical_pause(false)
            .with_cache_pause(true),
    );
    coordinator.observe(playing(generation, 11.0, 10.2));
    coordinator.observe(playing(generation, 12.0, 10.5));
    let actions = coordinator.observe(playing(generation, 13.1, 11.0));

    assert!(actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Degraded {
            reason: DegradedPlaybackReason::NonSeekableLag,
            ..
        }
    )));
    assert!(!actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_),
            ..
        }
    )));
    assert!(coordinator.ordinary_correction_blocked());
}

#[test]
fn recovery_stability_requires_continuous_position_advancement() {
    let config = PlaybackCoordinatorConfig {
        recovery_policy: RecoveryPolicy::PreserveContent,
        stability_interval_seconds: 2.0,
        ..PlaybackCoordinatorConfig::default()
    };
    let mut coordinator = PlaybackCoordinator::new(config);
    let generation = coordinator
        .prepare_media(
            LogicalMediaId::new("stable-media").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;
    coordinator.update_desired_room_state(desired(generation, 1, false, 0.0));
    coordinator.observe(
        PlayerTransportObservation::new(generation, 0.0)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(0.0)
            .with_logical_pause(false)
            .with_cache_pause(true),
    );
    coordinator.observe(playing(generation, 1.0, 1.0));
    coordinator.observe(playing(generation, 2.0, 2.0));
    assert_eq!(
        coordinator
            .recovery_episode()
            .and_then(|episode| episode.stable_since_seconds),
        Some(2.0)
    );

    // A sparse lifecycle observation is neutral, but a fresh position
    // sample without advancement restarts the stable interval.
    coordinator.observe(
        PlayerTransportObservation::new(generation, 2.5).with_phase(PlayerTransportPhase::Playing),
    );
    assert_eq!(
        coordinator
            .recovery_episode()
            .and_then(|episode| episode.stable_since_seconds),
        Some(2.0)
    );
    coordinator.observe(playing(generation, 3.0, 2.0));
    assert_eq!(
        coordinator
            .recovery_episode()
            .and_then(|episode| episode.stable_since_seconds),
        None
    );

    coordinator.observe(playing(generation, 4.0, 4.0));
    coordinator.observe(playing(generation, 5.9, 5.9));
    assert!(coordinator.recovery_episode().is_some());
    coordinator.observe(playing(generation, 6.1, 6.1));
    assert!(coordinator.recovery_episode().is_none());
}

#[test]
fn ahead_after_recovery_releases_to_ordinary_correction_after_stable_playback() {
    let config = PlaybackCoordinatorConfig {
        stability_interval_seconds: 1.0,
        ..PlaybackCoordinatorConfig::default()
    };
    let mut coordinator = PlaybackCoordinator::new(config);
    let generation = coordinator
        .prepare_media(
            LogicalMediaId::new("ahead-after-recovery").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;
    coordinator.update_desired_room_state(desired(generation, 1, false, 0.0));
    coordinator.observe(
        PlayerTransportObservation::new(generation, 0.0)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(0.0)
            .with_logical_pause(false)
            .with_cache_pause(true),
    );

    coordinator.observe(playing(generation, 1.0, 3.0));
    coordinator.observe(playing(generation, 2.0, 4.0));
    let released = coordinator.observe(playing(generation, 3.1, 5.1));

    assert!(!released.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_)
                | CoordinatorPlayerCommand::SetPlaybackRate(_),
            ..
        }
    )));
    assert!(coordinator.recovery_episode().is_none());
    assert!(
        !coordinator.ordinary_correction_blocked(),
        "stable ahead playback must be handed back to ordinary slowdown correction"
    );
}

#[test]
fn live_recovery_seek_is_clamped_behind_the_latest_seekable_edge() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::LiveSliding);
    coordinator.update_desired_room_state(DesiredRoomPlayback {
        anchor_observed_at_seconds: 10.0,
        ..desired(generation, 1, false, 200.0)
    });
    coordinator.observe(
        PlayerTransportObservation::new(generation, 10.0)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(85.0)
            .with_logical_pause(false)
            .with_cache_pause(true)
            .with_seekable(true)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(80.0, 100.0)])
            .with_known_live_seekable_window(PlayerSeekableRange::new(80.0, 100.0)),
    );
    coordinator.observe(playing(generation, 11.0, 85.1));
    let actions = coordinator.observe(playing(generation, 12.0, 85.3));

    assert!(actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if (*position - 99.0).abs() < f64::EPSILON
    )));
}

#[test]
fn offset_shifted_seekable_window_keeps_its_nonnegative_portion() {
    assert_eq!(
        latest_valid_seekable_window(&[PlayerSeekableRange::new(-5.0, 95.0)]),
        Some((0.0, 95.0))
    );
}

#[test]
fn vod_forced_seek_is_not_clamped_to_the_current_cache_range() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    coordinator.update_desired_room_state(DesiredRoomPlayback {
        force_seek: true,
        ..desired(generation, 2, true, 5.0)
    });

    let actions = coordinator.observe(
        PlayerTransportObservation::new(generation, 1.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(1.0)
            .with_logical_pause(true)
            .with_seekable(true)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(0.0, 2.0)])
            .with_core_idle(true),
    );

    assert!(actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if (*position - 5.0).abs() < f64::EPSILON
    )));
}

#[test]
fn stale_generation_observation_cannot_mutate_current_state() {
    let (mut coordinator, old_generation) = coordinator(MediaTransportKind::NetworkVod);
    let current_generation = coordinator
        .prepare_media(
            LogicalMediaId::new("episode-2").unwrap(),
            MediaTransportKind::NetworkVod,
            1.0,
        )
        .media_generation;
    coordinator.update_desired_room_state(desired(current_generation, 1, false, 0.0));

    let actions = coordinator.observe(playing(old_generation, 2.0, 100.0));
    assert!(actions.is_empty());
    assert_eq!(coordinator.metrics().stale_generation_observations, 1);
    assert_eq!(coordinator.desired_revision_pending(), Some(1));
}

#[test]
fn url_refresh_preserves_logical_generation() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    coordinator.update_desired_room_state(desired(generation, 1, false, 0.0));
    coordinator.observe(playing(generation, 0.1, 0.0));
    coordinator.observe(playing(generation, 0.2, 0.1));
    assert_eq!(coordinator.desired_revision_pending(), None);

    let refreshed = coordinator.prepare_media(
        LogicalMediaId::new("episode-1").unwrap(),
        MediaTransportKind::NetworkVod,
        10.0,
    );

    assert_eq!(refreshed.media_generation, generation);
    assert_eq!(refreshed.load_attempt, 2);
    assert!(!refreshed.logical_media_changed);
    assert_eq!(coordinator.desired_revision_pending(), Some(1));

    let actions = coordinator.observe(
        PlayerTransportObservation::new(generation, 10.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(0.0)
            .with_logical_pause(true)
            .with_seekable(true),
    );
    assert!(actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if (*position - 10.0).abs() < f64::EPSILON
    )));
}

#[test]
fn replay_of_same_logical_media_allocates_a_new_playback_generation() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    let replay = coordinator.prepare_media_with_intent(
        LogicalMediaId::new("episode-1").unwrap(),
        MediaTransportKind::NetworkVod,
        MediaLoadIntent::Replay,
        10.0,
    );

    assert_ne!(replay.media_generation, generation);
    assert_eq!(replay.load_attempt, 1);
    assert!(!replay.logical_media_changed);
    assert!(replay.playback_episode_changed);
    assert_eq!(replay.load_intent, MediaLoadIntent::Replay);
}

#[test]
fn adapter_epoch_reset_retains_rate_ownership_until_baseline_is_observed() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    coordinator.update_desired_room_state(DesiredRoomPlayback {
        anchor_observed_at_seconds: 10.0,
        ..desired(generation, 1, false, 25.0)
    });
    coordinator.observe(
        PlayerTransportObservation::new(generation, 10.0)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(20.0)
            .with_logical_pause(false)
            .with_cache_pause(true),
    );
    coordinator.observe(playing(generation, 11.0, 20.5));
    coordinator.observe(playing(generation, 12.0, 21.0));
    coordinator.observe(
        playing(generation, 13.0, 21.5)
            .with_playback_rate(CONSERVATIVE_CATCHUP_RATE_WITHOUT_HEADROOM),
    );

    coordinator.reset_transport_adapter_epoch(14.0);
    let reset = coordinator.observe(
        PlayerTransportObservation::new(generation, 14.1)
            .with_phase(PlayerTransportPhase::Loading)
            .with_playback_rate(CONSERVATIVE_CATCHUP_RATE_WITHOUT_HEADROOM)
            .with_core_idle(true),
    );
    assert!(reset.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
            ..
        } if (*rate - NORMAL_PLAYBACK_RATE).abs() < f64::EPSILON
    )));
    assert!(coordinator.ordinary_correction_blocked());

    coordinator.observe(
        PlayerTransportObservation::new(generation, 14.2)
            .with_phase(PlayerTransportPhase::Loading)
            .with_playback_rate(NORMAL_PLAYBACK_RATE)
            .with_core_idle(true),
    );
    assert!(coordinator.rate_override.is_none());
}

#[test]
fn failed_rate_reset_retries_without_reenabling_legacy_correction() {
    let config = PlaybackCoordinatorConfig {
        command_retry_cooldown_seconds: 0.1,
        ..PlaybackCoordinatorConfig::default()
    };
    let (mut coordinator, generation, target_rate, catchup_command_id) =
        begin_catchup_override(config);
    observe_catchup_override(
        &mut coordinator,
        generation,
        target_rate,
        catchup_command_id,
    );

    let reset = coordinator.interrupt_recovery();
    let reset_command_id = reset
        .iter()
        .find_map(|action| match action {
            PlaybackCoordinatorAction::Execute {
                command_id,
                command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
            } if (*rate - NORMAL_PLAYBACK_RATE).abs() <= 0.001 => Some(*command_id),
            _ => None,
        })
        .expect("interrupting catch-up should restore the baseline rate");
    assert!(coordinator.command_failed(reset_command_id, 13.01));
    assert!(
        coordinator.ordinary_correction_blocked(),
        "failed cleanup must retain exclusive coordinator ownership"
    );

    assert!(coordinator.tick(13.05).is_empty());
    let retry = coordinator.tick(13.12);
    assert!(retry.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
            ..
        } if (*rate - NORMAL_PLAYBACK_RATE).abs() <= 0.001
    )));
}

#[test]
fn timed_out_rate_reset_retries_after_its_cooldown() {
    let config = PlaybackCoordinatorConfig {
        command_timeout_seconds: 0.2,
        command_retry_budget: 5,
        command_retry_cooldown_seconds: 0.1,
        ..PlaybackCoordinatorConfig::default()
    };
    let (mut coordinator, generation, target_rate, catchup_command_id) =
        begin_catchup_override(config);
    observe_catchup_override(
        &mut coordinator,
        generation,
        target_rate,
        catchup_command_id,
    );

    let reset = coordinator.interrupt_recovery();
    let reset_command_id = reset
        .iter()
        .find_map(|action| match action {
            PlaybackCoordinatorAction::Execute {
                command_id,
                command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
            } if (*rate - NORMAL_PLAYBACK_RATE).abs() <= 0.001 => Some(*command_id),
            _ => None,
        })
        .expect("interrupting catch-up should restore the baseline rate");
    assert!(coordinator.command_accepted(reset_command_id));

    let timed_out = coordinator.tick(13.21);
    assert!(timed_out.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::CommandTimedOut { command_id }
            if *command_id == reset_command_id
    )));
    assert!(coordinator.ordinary_correction_blocked());
    let retry = coordinator.tick(13.32);
    assert!(retry.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
            ..
        } if (*rate - NORMAL_PLAYBACK_RATE).abs() <= 0.001
    )));
}

#[test]
fn command_budget_degradation_cannot_strand_a_catchup_rate() {
    let config = PlaybackCoordinatorConfig {
        command_timeout_seconds: 0.2,
        command_retry_budget: 0,
        command_retry_cooldown_seconds: 0.1,
        ..PlaybackCoordinatorConfig::default()
    };
    let (mut coordinator, _generation, _target_rate, catchup_command_id) =
        begin_catchup_override(config);
    assert!(coordinator.command_accepted(catchup_command_id));

    let degraded = coordinator.tick(12.21);
    assert!(degraded.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Degraded {
            reason: DegradedPlaybackReason::RecoveryCommandTimedOut,
            ..
        }
    )));
    assert!(coordinator.command_budget_degraded);
    assert!(
        coordinator
            .rate_override
            .is_some_and(|rate| rate.reset_requested)
    );

    let reset = coordinator.tick(12.32);
    assert!(
        reset.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
                ..
            } if (*rate - NORMAL_PLAYBACK_RATE).abs() <= 0.001
        )),
        "baseline reset must bypass the exhausted recovery-command budget"
    );
}

#[test]
fn decision_time_baseline_does_not_release_catchup_or_its_later_cleanup() {
    let (mut coordinator, generation, target_rate, catchup_command_id) =
        begin_catchup_override_with_decision_rate(
            PlaybackCoordinatorConfig::default(),
            Some(NORMAL_PLAYBACK_RATE),
        );
    assert!(
        coordinator.rate_override.is_some(),
        "the pre-command baseline sample must not discard catch-up ownership"
    );
    assert!(coordinator.ordinary_correction_blocked());
    observe_catchup_override(
        &mut coordinator,
        generation,
        target_rate,
        catchup_command_id,
    );

    let reset = coordinator.interrupt_recovery();
    let reset_command_id = reset
        .iter()
        .find_map(|action| match action {
            PlaybackCoordinatorAction::Execute {
                command_id,
                command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
            } if (*rate - NORMAL_PLAYBACK_RATE).abs() <= 0.001 => Some(*command_id),
            _ => None,
        })
        .expect("interrupting catch-up should restore the baseline rate");
    assert!(coordinator.command_accepted(reset_command_id));
    assert!(coordinator.ordinary_correction_blocked());

    coordinator.observe(playing(generation, 13.1, 21.6));
    assert!(
        coordinator.ordinary_correction_blocked(),
        "a sparse post-command sample must not reuse the pre-command baseline rate"
    );

    coordinator.observe(playing(generation, 13.2, 21.7).with_playback_rate(target_rate));
    assert!(
        coordinator.ordinary_correction_blocked(),
        "target-rate telemetry cannot release cleanup ownership"
    );

    coordinator.observe(playing(generation, 13.3, 21.8).with_playback_rate(NORMAL_PLAYBACK_RATE));
    assert!(coordinator.rate_override.is_none());
    assert!(
        !coordinator.ordinary_correction_blocked(),
        "only a fresh post-reset baseline observation releases ownership"
    );
}

#[test]
fn repeated_recovery_interruption_keeps_the_existing_baseline_reset_tracked() {
    let (mut coordinator, generation, target_rate, catchup_command_id) =
        begin_catchup_override(PlaybackCoordinatorConfig::default());
    observe_catchup_override(
        &mut coordinator,
        generation,
        target_rate,
        catchup_command_id,
    );

    let first_reset = coordinator.interrupt_recovery();
    let reset_command_id = first_reset
        .iter()
        .find_map(|action| match action {
            PlaybackCoordinatorAction::Execute {
                command_id,
                command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
            } if (*rate - NORMAL_PLAYBACK_RATE).abs() <= 0.001 => Some(*command_id),
            _ => None,
        })
        .expect("the first interruption should request baseline speed");

    assert!(
        coordinator.interrupt_recovery().is_empty(),
        "repeated lifecycle reconciliation must not replace a pending reset"
    );
    assert!(coordinator.pending_commands.iter().any(|command| {
        command.id == reset_command_id
            && matches!(
                command.kind,
                PendingCommandKind::Rate { target_rate }
                    if (target_rate - NORMAL_PLAYBACK_RATE).abs() <= 0.001
            )
    }));

    coordinator.observe(playing(generation, 13.1, 21.6).with_playback_rate(NORMAL_PLAYBACK_RATE));
    assert!(coordinator.rate_override.is_none());
}

#[test]
fn barrier_supersession_resets_catchup_before_applying_its_forced_seek() {
    let (mut coordinator, generation, target_rate, catchup_command_id) =
        begin_catchup_override(PlaybackCoordinatorConfig::default());
    observe_catchup_override(
        &mut coordinator,
        generation,
        target_rate,
        catchup_command_id,
    );

    let cleanup = coordinator.update_desired_room_state(DesiredRoomPlayback {
        media_generation: generation,
        state_revision: 2,
        paused: true,
        anchor_position_seconds: 30.0,
        anchor_observed_at_seconds: 14.0,
        force_seek: true,
    });
    assert!(cleanup.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
            ..
        } if (*rate - NORMAL_PLAYBACK_RATE).abs() <= 0.001
    )));
    assert!(coordinator.recovery_episode().is_none());
    assert!(coordinator.rate_override.is_some());

    let forced_seek =
        coordinator.observe(playing(generation, 14.1, 21.8).with_playback_rate(target_rate));
    assert!(forced_seek.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if (*position - 30.0).abs() <= f64::EPSILON
    )));
}

#[test]
fn pause_and_transport_refresh_reset_owned_catchup_rate() {
    let (mut pause_coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    pause_coordinator.update_desired_room_state(DesiredRoomPlayback {
        anchor_observed_at_seconds: 10.0,
        ..desired(generation, 1, false, 25.0)
    });
    pause_coordinator.observe(
        PlayerTransportObservation::new(generation, 10.0)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(20.0)
            .with_logical_pause(false)
            .with_cache_pause(true),
    );
    pause_coordinator.observe(playing(generation, 11.0, 20.5));
    pause_coordinator.observe(playing(generation, 12.0, 21.0));
    pause_coordinator.observe(
        playing(generation, 13.0, 21.5)
            .with_playback_rate(CONSERVATIVE_CATCHUP_RATE_WITHOUT_HEADROOM),
    );

    let pause_actions = pause_coordinator.update_desired_room_state(DesiredRoomPlayback {
        anchor_observed_at_seconds: 13.0,
        ..desired(generation, 2, true, 26.5)
    });
    assert!(pause_actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
            ..
        } if (*rate - NORMAL_PLAYBACK_RATE).abs() < f64::EPSILON
    )));

    // Re-enter catch-up, then refresh the same transport identity. The
    // new attempt must still restore the baseline before it can resume.
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    coordinator.update_desired_room_state(DesiredRoomPlayback {
        anchor_observed_at_seconds: 10.0,
        ..desired(generation, 1, false, 25.0)
    });
    coordinator.observe(
        PlayerTransportObservation::new(generation, 10.0)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(20.0)
            .with_logical_pause(false)
            .with_cache_pause(true),
    );
    coordinator.observe(playing(generation, 11.0, 20.5));
    coordinator.observe(playing(generation, 12.0, 21.0));
    coordinator.observe(
        playing(generation, 13.0, 21.5)
            .with_playback_rate(CONSERVATIVE_CATCHUP_RATE_WITHOUT_HEADROOM),
    );
    let refresh = coordinator.prepare_media_with_intent(
        LogicalMediaId::new("episode-1").unwrap(),
        MediaTransportKind::NetworkVod,
        MediaLoadIntent::TransportRefresh,
        14.0,
    );
    let refresh_actions = coordinator.observe(
        PlayerTransportObservation::new(refresh.media_generation, 14.1)
            .with_phase(PlayerTransportPhase::Loading)
            .with_playback_rate(CONSERVATIVE_CATCHUP_RATE_WITHOUT_HEADROOM)
            .with_core_idle(true),
    );
    assert!(refresh_actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
            ..
        } if (*rate - NORMAL_PLAYBACK_RATE).abs() < f64::EPSILON
    )));
}

#[test]
fn local_file_reaches_started_without_recovery_actions() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::LocalFile);
    coordinator.update_desired_room_state(desired(generation, 1, false, 0.0));
    coordinator.observe(
        PlayerTransportObservation::new(generation, 0.1)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(0.0)
            .with_logical_pause(true)
            .with_seekable(true),
    );
    coordinator.observe(playing(generation, 0.2, 0.0).with_restart_sequence(1));
    let actions = coordinator.observe(playing(generation, 0.3, 0.1).with_restart_sequence(1));

    assert!(
        actions
            .iter()
            .any(|action| matches!(action, PlaybackCoordinatorAction::Started { .. }))
    );
    assert_eq!(coordinator.metrics().buffer_episode_count, 0);
}

fn begin_fetch_required_seek(
    coordinator: &mut PlaybackCoordinator,
    generation: u64,
    revision: u64,
    target: f64,
) -> Vec<PlaybackCoordinatorAction> {
    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            force_seek: true,
            ..desired(generation, revision, false, target)
        },
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );
    coordinator.observe(
        PlayerTransportObservation::new(generation, revision as f64)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(0.0, 10.0)]),
    )
}

fn begin_fetch_required_seek_after_pre_seek_cache_metrics(
    coordinator: &mut PlaybackCoordinator,
    generation: u64,
    target: f64,
) -> Vec<PlaybackCoordinatorAction> {
    let mut pre_seek = PlayerTransportObservation::new(generation, 0.5)
        .with_phase(PlayerTransportPhase::ReadyPaused)
        .with_position(5.0)
        .with_logical_pause(true)
        .with_cache_pause(false)
        .with_seeking(false)
        .with_seekable(true)
        .with_seekable_ranges(vec![PlayerSeekableRange::new(0.0, 10.0)])
        .with_cache_buffering_percent(100.0)
        .with_buffered_ahead_seconds(10.0);
    pre_seek.input_rate_bytes_per_second = Some(9_000_000);
    coordinator.observe(pre_seek);
    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            force_seek: true,
            ..desired(generation, 1, false, target)
        },
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );
    coordinator.observe(
        PlayerTransportObservation::new(generation, 1.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true),
    )
}

#[test]
fn seekable_ranges_are_normalized_while_absent_and_present_empty_remain_distinct() {
    assert_eq!(
        normalize_seekable_ranges(&[
            PlayerSeekableRange::new(8.0, 12.0),
            PlayerSeekableRange::new(f64::NAN, 2.0),
            PlayerSeekableRange::new(-5.0, 3.0),
            PlayerSeekableRange::new(2.0, 9.0),
            PlayerSeekableRange::new(30.0, 20.0),
        ]),
        vec![PlayerSeekableRange::new(0.0, 12.0)]
    );
    assert_eq!(
        classify_seek_target(MediaTransportKind::NetworkVod, Some(true), None, None, 20.0,),
        SeekTargetAvailability::Unknown
    );
    assert_eq!(
        classify_seek_target(
            MediaTransportKind::NetworkVod,
            Some(true),
            Some(&[]),
            None,
            20.0,
        ),
        SeekTargetAvailability::FetchRequired
    );
}

#[test]
fn unbuffered_vod_seek_freezes_one_primary_target_while_room_time_advances() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    let first = begin_fetch_required_seek(&mut coordinator, generation, 1, 40.0);
    assert_eq!(
        first
            .iter()
            .filter(|action| matches!(
                action,
                PlaybackCoordinatorAction::Execute {
                    command: CoordinatorPlayerCommand::SetPosition(position),
                    ..
                } if (*position - 40.0).abs() <= f64::EPSILON
            ))
            .count(),
        1
    );
    let snapshot = coordinator
        .seek_preparation_snapshot()
        .expect("uncached VOD target should enter preparation");
    assert_eq!(snapshot.availability, SeekTargetAvailability::FetchRequired);
    assert_eq!(snapshot.frozen_target_seconds, 40.0);

    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            media_generation: generation,
            state_revision: 1,
            paused: false,
            anchor_position_seconds: 45.0,
            anchor_observed_at_seconds: 5.0,
            force_seek: false,
        },
        DesiredRoomPlaybackUpdateKind::Ordinary,
    );
    let fetching = coordinator.observe(
        PlayerTransportObservation::new(generation, 5.0)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(40.0)
            .with_logical_pause(true)
            .with_cache_pause(true)
            .with_cache_buffering_percent(35.0)
            .with_seeking(false)
            .with_seekable(true)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(0.0, 10.0)]),
    );
    assert!(!fetching.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_),
            ..
        }
    )));
    let snapshot = coordinator.seek_preparation_snapshot().unwrap();
    assert_eq!(snapshot.frozen_target_seconds, 40.0);
    assert_eq!(snapshot.latest_room_position_seconds, 45.0);
    assert_eq!(snapshot.cache_buffering_percent, Some(35.0));
    assert_eq!(snapshot.phase, SeekPreparationPhase::Refilling);
}

#[test]
fn already_satisfied_unknown_target_requires_stable_post_seek_observation() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            force_seek: true,
            ..desired(generation, 1, false, 40.0)
        },
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );

    let actions = coordinator.observe(
        PlayerTransportObservation::new(generation, 1.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(40.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true)
            .with_timeline_kind(PlayerTimelineKind::Unknown),
    );
    assert!(!actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_),
            ..
        }
    )));
    assert_eq!(coordinator.last_seek_preparation_terminal_outcome(), None);
    assert!(coordinator.seek_preparation_snapshot().is_some());
    assert!(!actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::Play(_),
            ..
        }
    )));

    let first_post_seek = coordinator.observe(
        PlayerTransportObservation::new(generation, 2.1)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(40.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true)
            .with_timeline_kind(PlayerTimelineKind::Unknown),
    );
    assert_eq!(coordinator.last_seek_preparation_terminal_outcome(), None);
    assert!(!first_post_seek.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::Play(_),
            ..
        }
    )));

    let stable = coordinator.observe(
        PlayerTransportObservation::new(generation, 3.2)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(40.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true)
            .with_timeline_kind(PlayerTimelineKind::Unknown),
    );
    assert_eq!(
        coordinator.last_seek_preparation_terminal_outcome(),
        Some(SeekPreparationTerminalOutcome::Ready)
    );
    assert!(coordinator.recovery_episode().is_some());
    assert!(stable.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::Play(_),
            ..
        }
    )));
}

#[test]
fn explicitly_unknown_timeline_still_allows_a_cached_target_seek() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    coordinator.observe(
        PlayerTransportObservation::new(generation, 0.1)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true)
            .with_timeline_kind(PlayerTimelineKind::Unknown)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(0.0, 50.0)]),
    );
    let mut actions = coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            force_seek: true,
            ..desired(generation, 1, false, 40.0)
        },
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );
    actions.extend(
        coordinator.observe(
            PlayerTransportObservation::new(generation, 0.2)
                .with_phase(PlayerTransportPhase::ReadyPaused)
                .with_position(5.0)
                .with_logical_pause(true)
                .with_cache_pause(false)
                .with_seeking(false)
                .with_seekable(true)
                .with_timeline_kind(PlayerTimelineKind::Unknown),
        ),
    );
    assert!(actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if (*position - 40.2).abs() <= f64::EPSILON
    )));
    assert!(coordinator.seek_preparation_snapshot().is_none());
}

#[test]
fn unsafe_authoritative_seek_waits_boundedly_for_timeline_classification() {
    let mut coordinator = PlaybackCoordinator::new(PlaybackCoordinatorConfig {
        command_timeout_seconds: 1.0,
        seek_preparation_timeout_seconds: 2.0,
        ..PlaybackCoordinatorConfig::default()
    });
    let generation = coordinator
        .prepare_media(
            LogicalMediaId::new("unknown-authoritative-seek").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;
    coordinator.update_desired_room_state(DesiredRoomPlayback {
        force_seek: true,
        ..desired(generation, 1, false, 40.0)
    });

    let waiting = coordinator.observe(
        PlayerTransportObservation::new(generation, 0.1)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true)
            .with_timeline_kind(PlayerTimelineKind::Unknown),
    );
    assert!(coordinator.seek_preparation_snapshot().is_some());
    assert!(!waiting.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_) | CoordinatorPlayerCommand::Play(_),
            ..
        } | PlaybackCoordinatorAction::Started { .. }
    )));

    let timed_out = coordinator.tick(2.1);
    assert!(timed_out.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Degraded {
            reason: DegradedPlaybackReason::TimelineWindowUnavailable,
            ..
        }
    )));
    assert_eq!(
        coordinator.last_seek_preparation_terminal_outcome(),
        Some(SeekPreparationTerminalOutcome::Degraded(
            SeekPreparationDegradedReason::TimelineWindowUnavailable
        ))
    );

    let held = coordinator.observe(
        PlayerTransportObservation::new(generation, 2.2)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true)
            .with_timeline_kind(PlayerTimelineKind::Unknown),
    );
    assert!(!held.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_) | CoordinatorPlayerCommand::Play(_),
            ..
        } | PlaybackCoordinatorAction::Started { .. }
    )));
    assert!(coordinator.ordinary_correction_blocked());
}

#[test]
fn unresolved_unpaused_alignment_latches_pause_while_timeline_is_unknown() {
    let mut coordinator = PlaybackCoordinator::new(PlaybackCoordinatorConfig {
        command_timeout_seconds: 1.0,
        seek_preparation_timeout_seconds: 2.0,
        ..PlaybackCoordinatorConfig::default()
    });
    let generation = coordinator
        .prepare_media(
            LogicalMediaId::new("pause-unknown-alignment").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;
    coordinator.update_desired_room_state(DesiredRoomPlayback {
        force_seek: true,
        ..desired(generation, 1, false, 40.0)
    });

    let waiting = coordinator.observe(
        PlayerTransportObservation::new(generation, 0.1)
            .with_phase(PlayerTransportPhase::Playing)
            .with_position(5.0)
            .with_logical_pause(false)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true)
            .with_timeline_kind(PlayerTimelineKind::Unknown)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(0.0, 10.0)]),
    );
    assert!(waiting.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPaused(true),
            ..
        }
    )));
    assert!(!waiting.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_) | CoordinatorPlayerCommand::Play(_),
            ..
        } | PlaybackCoordinatorAction::Started { .. }
    )));

    coordinator.tick(2.1);
    let held = coordinator.observe(
        PlayerTransportObservation::new(generation, 2.2)
            .with_phase(PlayerTransportPhase::Playing)
            .with_position(6.0)
            .with_logical_pause(false)
            .with_seekable(true)
            .with_timeline_kind(PlayerTimelineKind::Unknown),
    );
    assert!(!held.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_) | CoordinatorPlayerCommand::Play(_),
            ..
        } | PlaybackCoordinatorAction::Started { .. }
    )));
    assert_eq!(coordinator.diagnostic(), PlaybackDiagnostic::Degraded);
}

#[test]
fn confirmed_live_timeline_waits_for_a_window_then_seeks_once_to_its_safe_edge() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    coordinator.update_desired_room_state(DesiredRoomPlayback {
        force_seek: true,
        ..desired(generation, 1, false, 200.0)
    });

    let before_window = coordinator.observe(
        PlayerTransportObservation::new(generation, 0.1)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(85.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true)
            .with_timeline_kind(PlayerTimelineKind::SlidingLive),
    );
    assert!(coordinator.seek_preparation_snapshot().is_some());
    assert!(!before_window.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_),
            ..
        }
    )));

    let after_window = coordinator.observe(
        PlayerTransportObservation::new(generation, 0.2)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(85.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true)
            .with_timeline_kind(PlayerTimelineKind::SlidingLive)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(80.0, 100.0)])
            .with_known_live_seekable_window(PlayerSeekableRange::new(80.0, 100.0)),
    );
    assert_eq!(
        after_window
            .iter()
            .filter(|action| matches!(
                action,
                PlaybackCoordinatorAction::Execute {
                    command: CoordinatorPlayerCommand::SetPosition(position),
                    ..
                } if (*position - 99.0).abs() <= f64::EPSILON
            ))
            .count(),
        1
    );
    let repeated = coordinator.observe(
        PlayerTransportObservation::new(generation, 0.3)
            .with_phase(PlayerTransportPhase::Seeking)
            .with_position(85.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(true)
            .with_seekable(true)
            .with_timeline_kind(PlayerTimelineKind::SlidingLive)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(80.0, 100.0)])
            .with_known_live_seekable_window(PlayerSeekableRange::new(80.0, 100.0)),
    );
    assert!(!repeated.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_),
            ..
        }
    )));
}

#[test]
fn unknown_nonseekable_authoritative_target_degrades_without_starting() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    coordinator.update_desired_room_state(DesiredRoomPlayback {
        force_seek: true,
        ..desired(generation, 1, false, 40.0)
    });

    let actions = coordinator.observe(
        PlayerTransportObservation::new(generation, 0.1)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(false)
            .with_timeline_kind(PlayerTimelineKind::Unknown),
    );
    assert!(actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Degraded {
            reason: DegradedPlaybackReason::NonSeekableLag,
            ..
        }
    )));
    assert!(!actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_) | CoordinatorPlayerCommand::Play(_),
            ..
        } | PlaybackCoordinatorAction::Started { .. }
    )));
    assert_eq!(
        coordinator.last_seek_preparation_terminal_outcome(),
        Some(SeekPreparationTerminalOutcome::Degraded(
            SeekPreparationDegradedReason::NonSeekable
        ))
    );
}

#[test]
fn cached_seek_stall_is_timed_out_not_misreported_as_missing_timeline() {
    let mut coordinator = PlaybackCoordinator::new(PlaybackCoordinatorConfig {
        command_timeout_seconds: 1.0,
        seek_preparation_timeout_seconds: 2.0,
        ..PlaybackCoordinatorConfig::default()
    });
    let generation = coordinator
        .prepare_media(
            LogicalMediaId::new("cached-unknown-stall").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;
    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            force_seek: true,
            ..desired(generation, 1, false, 40.0)
        },
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );
    let dispatched = coordinator.observe(
        PlayerTransportObservation::new(generation, 0.1)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true)
            .with_timeline_kind(PlayerTimelineKind::Unknown)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(0.0, 50.0)]),
    );
    assert!(dispatched.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(40.0),
            ..
        }
    )));

    let timed_out = coordinator.tick(2.1);
    assert!(timed_out.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Degraded {
            reason: DegradedPlaybackReason::RecoveryCommandTimedOut,
            ..
        }
    )));
    assert_eq!(
        coordinator.last_seek_preparation_terminal_outcome(),
        Some(SeekPreparationTerminalOutcome::Degraded(
            SeekPreparationDegradedReason::TimedOut
        ))
    );
}

#[test]
fn interrupting_a_degraded_wait_invalidates_its_unreached_desire() {
    let mut coordinator = PlaybackCoordinator::new(PlaybackCoordinatorConfig {
        command_timeout_seconds: 1.0,
        seek_preparation_timeout_seconds: 2.0,
        ..PlaybackCoordinatorConfig::default()
    });
    let generation = coordinator
        .prepare_media(
            LogicalMediaId::new("interrupted-degraded-wait").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;
    coordinator.update_desired_room_state(DesiredRoomPlayback {
        force_seek: true,
        ..desired(generation, 1, false, 40.0)
    });
    coordinator.observe(
        PlayerTransportObservation::new(generation, 0.1)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_seekable(true)
            .with_timeline_kind(PlayerTimelineKind::Unknown),
    );
    coordinator.tick(2.1);
    coordinator.interrupt_recovery();
    assert!(coordinator.desired.is_none());

    let actions = coordinator.observe(
        PlayerTransportObservation::new(generation, 2.2)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_seekable(true)
            .with_timeline_kind(PlayerTimelineKind::Unknown),
    );
    assert!(!actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_) | CoordinatorPlayerCommand::Play(_),
            ..
        } | PlaybackCoordinatorAction::RevisionApplied { .. }
            | PlaybackCoordinatorAction::Started { .. }
    )));
}

#[test]
fn pre_seek_refill_and_headroom_are_not_attributed_to_the_frozen_target() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    coordinator.observe(
        PlayerTransportObservation::new(generation, 0.5)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_cache_buffering_percent(100.0)
            .with_buffered_ahead_seconds(10.0)
            .with_seeking(false)
            .with_seekable(true)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(0.0, 10.0)]),
    );
    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            force_seek: true,
            ..desired(generation, 1, false, 40.0)
        },
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );
    coordinator.observe(
        PlayerTransportObservation::new(generation, 1.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true),
    );
    let dispatched = coordinator.seek_preparation_snapshot().unwrap();
    assert_eq!(dispatched.cache_buffering_percent, None);
    assert_eq!(dispatched.buffered_ahead_seconds, None);
    assert_eq!(coordinator.metrics().last_buffered_ahead_seconds, None);

    coordinator.observe(
        PlayerTransportObservation::new(generation, 2.0)
            .with_phase(PlayerTransportPhase::Prebuffering)
            .with_position(40.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true),
    );
    let fetching = coordinator.seek_preparation_snapshot().unwrap();
    assert_eq!(fetching.cache_buffering_percent, None);
    assert_eq!(fetching.buffered_ahead_seconds, None);
    assert_ne!(
        coordinator.last_seek_preparation_terminal_outcome(),
        Some(SeekPreparationTerminalOutcome::Ready)
    );
}

#[test]
fn stale_pre_seek_cache_metrics_keep_post_seek_catchup_conservative() {
    let mut coordinator = PlaybackCoordinator::new(PlaybackCoordinatorConfig {
        maximum_catchup_rate: 1.25,
        ..PlaybackCoordinatorConfig::default()
    });
    let generation = coordinator
        .prepare_media(
            LogicalMediaId::new("stale-pre-seek-recovery-metrics").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;
    let dispatched =
        begin_fetch_required_seek_after_pre_seek_cache_metrics(&mut coordinator, generation, 40.0);
    assert!(dispatched.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if (*position - 40.0).abs() <= f64::EPSILON
    )));
    assert_eq!(coordinator.metrics().last_buffered_ahead_seconds, None);
    assert_eq!(coordinator.metrics().last_input_rate_bytes_per_second, None);

    let mut delayed_old_position = PlayerTransportObservation::new(generation, 1.1)
        .with_phase(PlayerTransportPhase::ReadyPaused)
        .with_position(5.0)
        .with_logical_pause(true)
        .with_cache_pause(false)
        .with_seeking(false)
        .with_seekable(true)
        .with_buffered_ahead_seconds(10.0);
    delayed_old_position.input_rate_bytes_per_second = Some(9_000_000);
    coordinator.observe(delayed_old_position);
    assert_eq!(coordinator.metrics().last_buffered_ahead_seconds, None);
    assert_eq!(coordinator.metrics().last_input_rate_bytes_per_second, None);

    coordinator.observe(
        PlayerTransportObservation::new(generation, 2.0)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(40.0)
            .with_logical_pause(true)
            .with_cache_pause(true)
            .with_seeking(false)
            .with_seekable(true),
    );
    let mut delayed_after_target = PlayerTransportObservation::new(generation, 2.1)
        .with_seekable_ranges(vec![PlayerSeekableRange::new(0.0, 10.0)])
        .with_cache_buffering_percent(100.0)
        .with_buffered_ahead_seconds(10.0);
    delayed_after_target.input_rate_bytes_per_second = Some(9_000_000);
    coordinator.observe(delayed_after_target);
    let active = coordinator.seek_preparation_snapshot().unwrap();
    assert_eq!(active.cache_buffering_percent, None);
    assert_eq!(active.buffered_ahead_seconds, None);
    assert_eq!(coordinator.metrics().last_buffered_ahead_seconds, None);
    assert_eq!(coordinator.metrics().last_input_rate_bytes_per_second, None);

    let mut stale_merged_replay = PlayerTransportObservation::new(generation, 2.2)
        .with_phase(PlayerTransportPhase::Rebuffering)
        .with_position(40.0)
        .with_logical_pause(true)
        .with_cache_pause(true)
        .with_seeking(false)
        .with_seekable(true)
        .with_cache_buffering_percent(100.0)
        .with_buffered_ahead_seconds(10.0);
    stale_merged_replay.input_rate_bytes_per_second = Some(9_000_000);
    coordinator.replay_observation(stale_merged_replay);
    let active = coordinator.seek_preparation_snapshot().unwrap();
    assert_eq!(active.cache_buffering_percent, None);
    assert_eq!(active.buffered_ahead_seconds, None);
    assert_eq!(coordinator.metrics().last_buffered_ahead_seconds, None);
    assert_eq!(coordinator.metrics().last_input_rate_bytes_per_second, None);

    coordinator.observe(
        PlayerTransportObservation::new(generation, 3.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(40.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true),
    );
    assert_eq!(
        coordinator.last_seek_preparation_terminal_outcome(),
        Some(SeekPreparationTerminalOutcome::Ready)
    );
    assert_eq!(coordinator.metrics().last_buffered_ahead_seconds, None);
    assert_eq!(coordinator.metrics().last_input_rate_bytes_per_second, None);

    let mut stale_after_ready = PlayerTransportObservation::new(generation, 3.1)
        .with_phase(PlayerTransportPhase::ReadyPaused)
        .with_position(40.0)
        .with_logical_pause(true)
        .with_cache_pause(false)
        .with_seeking(false)
        .with_seekable(true)
        .with_cache_buffering_percent(100.0)
        .with_buffered_ahead_seconds(10.0);
    stale_after_ready.input_rate_bytes_per_second = Some(9_000_000);
    coordinator.replay_observation(stale_after_ready);
    assert_eq!(coordinator.metrics().last_buffered_ahead_seconds, None);
    assert_eq!(coordinator.metrics().last_input_rate_bytes_per_second, None);

    let mut fresh_delayed_after_ready = PlayerTransportObservation::new(generation, 3.2)
        .with_seekable_ranges(vec![PlayerSeekableRange::new(0.0, 10.0)])
        .with_cache_buffering_percent(100.0)
        .with_buffered_ahead_seconds(10.0);
    fresh_delayed_after_ready.input_rate_bytes_per_second = Some(9_000_000);
    coordinator.observe(fresh_delayed_after_ready);
    assert_eq!(coordinator.metrics().last_buffered_ahead_seconds, None);
    assert_eq!(coordinator.metrics().last_input_rate_bytes_per_second, None);

    let actions = coordinator.observe(playing(generation, 4.0, 40.5));
    assert!(actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
            ..
        } if (*rate - CONSERVATIVE_CATCHUP_RATE_WITHOUT_HEADROOM).abs() < f64::EPSILON
    )));
}

#[test]
fn fresh_target_headroom_allows_configured_post_seek_catchup() {
    let mut coordinator = PlaybackCoordinator::new(PlaybackCoordinatorConfig {
        maximum_catchup_rate: 1.25,
        ..PlaybackCoordinatorConfig::default()
    });
    let generation = coordinator
        .prepare_media(
            LogicalMediaId::new("target-scoped-recovery-metrics").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;
    begin_fetch_required_seek_after_pre_seek_cache_metrics(&mut coordinator, generation, 40.0);

    let mut target_refill = PlayerTransportObservation::new(generation, 2.0)
        .with_phase(PlayerTransportPhase::Rebuffering)
        .with_position(40.0)
        .with_logical_pause(true)
        .with_cache_pause(true)
        .with_seeking(false)
        .with_seekable(true)
        .with_buffered_ahead_seconds(2.0);
    target_refill.input_rate_bytes_per_second = Some(2_000_000);
    coordinator.observe(target_refill);
    assert_eq!(coordinator.metrics().last_buffered_ahead_seconds, None);
    assert_eq!(coordinator.metrics().last_input_rate_bytes_per_second, None);

    coordinator.observe(
        PlayerTransportObservation::new(generation, 3.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(40.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true),
    );
    assert_eq!(coordinator.metrics().last_buffered_ahead_seconds, Some(2.0));
    assert_eq!(
        coordinator.metrics().last_input_rate_bytes_per_second,
        Some(2_000_000)
    );

    let actions = coordinator.observe(playing(generation, 4.0, 40.5));
    assert!(actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
            ..
        } if (*rate - 1.25).abs() < f64::EPSILON
    )));
}

#[test]
fn transient_ready_paused_waits_for_delayed_refill_and_its_release() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    begin_fetch_required_seek(&mut coordinator, generation, 1, 40.0);

    let transient = coordinator.observe(
        PlayerTransportObservation::new(generation, 2.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(40.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true)
            .with_restart_sequence(1),
    );
    assert!(coordinator.seek_preparation_snapshot().is_some());
    assert!(!transient.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::Play(_),
            ..
        }
    )));

    coordinator.observe(
        PlayerTransportObservation::new(generation, 2.1)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(40.0)
            .with_logical_pause(true)
            .with_cache_pause(true)
            .with_seeking(false)
            .with_seekable(true),
    );
    assert!(coordinator.seek_preparation_snapshot().is_some());
    assert_eq!(coordinator.last_seek_preparation_terminal_outcome(), None);

    let released = coordinator.observe(
        PlayerTransportObservation::new(generation, 3.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(40.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true),
    );
    assert_eq!(
        coordinator.last_seek_preparation_terminal_outcome(),
        Some(SeekPreparationTerminalOutcome::Ready)
    );
    assert!(released.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::Play(_),
            ..
        }
    )));
}

#[test]
fn target_scoped_headroom_must_cross_the_configured_threshold() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    begin_fetch_required_seek(&mut coordinator, generation, 1, 40.0);

    coordinator.observe(
        PlayerTransportObservation::new(generation, 2.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(40.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true)
            .with_buffered_ahead_seconds(1.9),
    );
    assert!(coordinator.seek_preparation_snapshot().is_some());
    assert_eq!(coordinator.last_seek_preparation_terminal_outcome(), None);

    coordinator.observe(
        PlayerTransportObservation::new(generation, 3.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(40.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true)
            .with_buffered_ahead_seconds(2.0),
    );
    assert_eq!(
        coordinator.last_seek_preparation_terminal_outcome(),
        Some(SeekPreparationTerminalOutcome::Ready)
    );
}

#[test]
fn target_scoped_refill_progress_can_complete_without_headroom_telemetry() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    begin_fetch_required_seek(&mut coordinator, generation, 1, 40.0);

    coordinator.observe(
        PlayerTransportObservation::new(generation, 2.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(40.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true)
            .with_cache_buffering_percent(100.0),
    );
    assert_eq!(
        coordinator.last_seek_preparation_terminal_outcome(),
        Some(SeekPreparationTerminalOutcome::Ready)
    );
}

#[test]
fn newer_explicit_seek_supersedes_preparation_but_ordinary_pause_does_not() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    let first = begin_fetch_required_seek(&mut coordinator, generation, 1, 40.0);
    let first_id = coordinator.seek_preparation_snapshot().unwrap().id;
    assert_eq!(
        first
            .iter()
            .filter(|action| matches!(
                action,
                PlaybackCoordinatorAction::Execute {
                    command: CoordinatorPlayerCommand::SetPosition(_),
                    ..
                }
            ))
            .count(),
        1
    );

    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            force_seek: true,
            ..desired(generation, 2, true, 41.0)
        },
        DesiredRoomPlaybackUpdateKind::Ordinary,
    );
    let pause_actions = coordinator.observe(
        PlayerTransportObservation::new(generation, 2.0)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(40.0)
            .with_logical_pause(false)
            .with_cache_pause(true)
            .with_seeking(false)
            .with_seekable(true)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(0.0, 10.0)]),
    );
    assert!(pause_actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPaused(true),
            ..
        }
    )));
    assert!(!pause_actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_),
            ..
        }
    )));
    let retained = coordinator.seek_preparation_snapshot().unwrap();
    assert_eq!(retained.id, first_id);
    assert_eq!(retained.frozen_target_seconds, 40.0);
    assert_eq!(retained.latest_room_revision, 2);

    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            force_seek: true,
            ..desired(generation, 3, false, 80.0)
        },
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );
    let replacement = coordinator.seek_preparation_snapshot().unwrap();
    assert_ne!(replacement.id, first_id);
    assert_eq!(replacement.frozen_target_seconds, 80.0);
    assert_eq!(
        coordinator.last_seek_preparation_terminal_outcome(),
        Some(SeekPreparationTerminalOutcome::Superseded)
    );
}

#[test]
fn preparation_latches_pause_but_defers_its_primary_seek_during_cache_stall() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            force_seek: true,
            ..desired(generation, 1, true, 40.0)
        },
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );

    let stalled = coordinator.observe(
        PlayerTransportObservation::new(generation, 1.0)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(5.0)
            .with_logical_pause(false)
            .with_cache_pause(true)
            .with_seeking(false)
            .with_seekable(true)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(0.0, 10.0)]),
    );
    assert!(stalled.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPaused(true),
            ..
        }
    )));
    assert!(!stalled.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_),
            ..
        }
    )));
    assert!(
        coordinator
            .seek_preparation
            .as_ref()
            .is_some_and(|episode| !episode.primary_seek_issued),
        "the frozen target must remain pending until correction is transport-safe"
    );

    let released = coordinator.observe(
        PlayerTransportObservation::new(generation, 2.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(0.0, 10.0)]),
    );
    assert_eq!(
        released
            .iter()
            .filter(|action| matches!(
                action,
                PlaybackCoordinatorAction::Execute {
                    command: CoordinatorPlayerCommand::SetPosition(position),
                    ..
                } if (*position - 40.0).abs() <= f64::EPSILON
            ))
            .count(),
        1
    );
}

#[test]
fn nearby_pause_revision_rebinds_primary_completion_without_a_second_seek() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    begin_fetch_required_seek(&mut coordinator, generation, 1, 40.0);
    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            force_seek: true,
            ..desired(generation, 2, true, 40.2)
        },
        DesiredRoomPlaybackUpdateKind::Ordinary,
    );

    let completion = coordinator.observe(
        PlayerTransportObservation::new(generation, 2.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(40.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(35.0, 50.0)])
            .with_buffered_ahead_seconds(4.0),
    );
    assert!(!completion.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_),
            ..
        }
    )));
    assert!(completion.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::RevisionApplied {
            state_revision: 2,
            ..
        }
    )));
    assert_eq!(coordinator.desired_revision_pending(), None);
    assert_eq!(
        coordinator.last_seek_preparation_terminal_outcome(),
        Some(SeekPreparationTerminalOutcome::Ready)
    );
}

#[test]
fn paused_room_position_is_aligned_once_after_frozen_target_becomes_ready() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    begin_fetch_required_seek(&mut coordinator, generation, 1, 40.0);
    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            force_seek: true,
            ..desired(generation, 2, true, 60.0)
        },
        DesiredRoomPlaybackUpdateKind::Ordinary,
    );

    let ready = coordinator.observe(
        PlayerTransportObservation::new(generation, 2.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(40.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(35.0, 65.0)])
            .with_buffered_ahead_seconds(4.0),
    );
    assert_eq!(
        ready
            .iter()
            .filter(|action| matches!(
                action,
                PlaybackCoordinatorAction::Execute {
                    command: CoordinatorPlayerCommand::SetPosition(position),
                    ..
                } if (*position - 60.0).abs() <= f64::EPSILON
            ))
            .count(),
        1,
        "the latest paused room position needs one bounded post-fetch alignment"
    );
    assert_eq!(coordinator.desired_revision_pending(), Some(2));

    let repeated = coordinator.observe(
        PlayerTransportObservation::new(generation, 2.1)
            .with_phase(PlayerTransportPhase::Seeking)
            .with_position(40.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(true)
            .with_seekable(true),
    );
    assert!(!repeated.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_),
            ..
        }
    )));
}

#[test]
fn preparation_waits_for_refill_then_hands_off_one_recovery_episode() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    begin_fetch_required_seek(&mut coordinator, generation, 1, 40.0);
    let stalled = coordinator.observe(
        PlayerTransportObservation::new(generation, 2.0)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(40.0)
            .with_logical_pause(true)
            .with_cache_pause(true)
            .with_cache_buffering_percent(68.0)
            .with_seeking(false)
            .with_seekable(true)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(35.0, 50.0)]),
    );
    assert!(!stalled.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::Play(_),
            ..
        }
    )));
    assert_eq!(
        coordinator.seek_preparation_snapshot().unwrap().phase,
        SeekPreparationPhase::Refilling
    );
    assert!(coordinator.recovery_episode().is_none());

    let ready = coordinator.observe(
        PlayerTransportObservation::new(generation, 3.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(40.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_cache_buffering_percent(100.0)
            .with_seeking(false)
            .with_seekable(true)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(35.0, 55.0)])
            .with_buffered_ahead_seconds(5.0),
    );
    assert!(ready.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::Play(_),
            ..
        }
    )));
    assert_eq!(
        coordinator.last_seek_preparation_terminal_outcome(),
        Some(SeekPreparationTerminalOutcome::Ready)
    );
    assert_eq!(
        coordinator.seek_preparation_snapshot().unwrap().phase,
        SeekPreparationPhase::ReadyToJoin
    );
    let recovery_id = coordinator.recovery_episode().unwrap().id;
    coordinator.recovery.as_mut().unwrap().catchup_active = true;
    assert_eq!(
        coordinator.seek_preparation_snapshot().unwrap().phase,
        SeekPreparationPhase::CatchingUp
    );
    coordinator.observe(
        PlayerTransportObservation::new(generation, 3.1)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(40.0)
            .with_logical_pause(false)
            .with_cache_pause(true)
            .with_seeking(false),
    );
    assert_eq!(coordinator.recovery_episode().unwrap().id, recovery_id);
    assert_eq!(
        coordinator.recovery_episode().unwrap().hard_seek_attempts,
        0
    );
    let mut degraded = Vec::new();
    coordinator
        .enter_degraded_recovery(DegradedPlaybackReason::CatchupDidNotConverge, &mut degraded);
    assert!(coordinator.seek_preparation_snapshot().is_none());
    assert!(coordinator.recovery_episode().unwrap().degraded);
    assert_eq!(
        coordinator.last_seek_preparation_terminal_outcome(),
        Some(SeekPreparationTerminalOutcome::Ready)
    );
}

#[test]
fn preparation_rejects_non_playable_readiness_and_media_end_is_terminal() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    begin_fetch_required_seek(&mut coordinator, generation, 1, 40.0);

    coordinator.observe(
        PlayerTransportObservation::new(generation, 2.0)
            .with_phase(PlayerTransportPhase::Prebuffering)
            .with_position(40.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_cache_buffering_percent(100.0)
            .with_buffered_ahead_seconds(10.0)
            .with_seeking(false)
            .with_seekable(true)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(35.0, 50.0)]),
    );
    assert!(coordinator.seek_preparation.is_some());
    assert_ne!(
        coordinator.last_seek_preparation_terminal_outcome(),
        Some(SeekPreparationTerminalOutcome::Ready)
    );

    let ended = coordinator.observe(
        PlayerTransportObservation::new(generation, 3.0)
            .with_phase(PlayerTransportPhase::Ended)
            .with_position(40.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true),
    );
    assert_eq!(
        coordinator.last_seek_preparation_terminal_outcome(),
        Some(SeekPreparationTerminalOutcome::Degraded(
            SeekPreparationDegradedReason::TransportFailed
        ))
    );
    assert!(ended.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Degraded {
            reason: DegradedPlaybackReason::TransportFailed,
            ..
        }
    )));
    assert!(coordinator.recovery_episode().is_none());
}

#[test]
fn live_outside_window_clamps_and_missing_ranges_remain_unknown() {
    let (mut vod, vod_generation) = coordinator(MediaTransportKind::NetworkVod);
    vod.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            force_seek: true,
            ..desired(vod_generation, 1, false, 40.0)
        },
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );
    assert_eq!(
        vod.seek_preparation_snapshot().unwrap().availability,
        SeekTargetAvailability::Unknown
    );

    let (mut live, live_generation) = coordinator(MediaTransportKind::LiveSliding);
    live.observe(
        PlayerTransportObservation::new(live_generation, 0.5)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(85.0)
            .with_logical_pause(true)
            .with_seekable(true)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(80.0, 100.0)])
            .with_known_live_seekable_window(PlayerSeekableRange::new(80.0, 100.0)),
    );
    live.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            force_seek: true,
            ..desired(live_generation, 1, false, 200.0)
        },
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );
    let snapshot = live.seek_preparation_snapshot().unwrap();
    assert_eq!(
        snapshot.availability,
        SeekTargetAvailability::OutsideLiveWindow
    );
    assert_eq!(snapshot.requested_target_seconds, 200.0);
    assert_eq!(snapshot.frozen_target_seconds, 99.0);
    let actions = live.observe(
        PlayerTransportObservation::new(live_generation, 1.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(85.0)
            .with_logical_pause(true)
            .with_seekable(true)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(80.0, 100.0)])
            .with_known_live_seekable_window(PlayerSeekableRange::new(80.0, 100.0)),
    );
    assert!(actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if (*position - 99.0).abs() <= f64::EPSILON
    )));

    let (mut empty_live, empty_generation) = coordinator(MediaTransportKind::LiveSliding);
    empty_live.observe(
        PlayerTransportObservation::new(empty_generation, 0.1)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_seekable(true)
            .with_timeline_kind(PlayerTimelineKind::SlidingLive)
            .with_seekable_ranges(Vec::new()),
    );
    empty_live.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            force_seek: true,
            ..desired(empty_generation, 1, false, 40.0)
        },
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );
    let rejected = empty_live.observe(
        PlayerTransportObservation::new(empty_generation, 1.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_seekable(true)
            .with_timeline_kind(PlayerTimelineKind::SlidingLive)
            .with_seekable_ranges(Vec::new()),
    );
    assert!(!rejected.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_),
            ..
        }
    )));
    assert_eq!(
        empty_live.seek_preparation_snapshot().unwrap().availability,
        SeekTargetAvailability::Unknown
    );
    assert_eq!(empty_live.last_seek_preparation_terminal_outcome(), None);
    empty_live.tick(100.0);
    assert_eq!(
        empty_live.last_seek_preparation_terminal_outcome(),
        Some(SeekPreparationTerminalOutcome::Degraded(
            SeekPreparationDegradedReason::TimelineWindowUnavailable
        ))
    );

    let (mut late_empty_live, late_empty_generation) = coordinator(MediaTransportKind::LiveSliding);
    late_empty_live.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            force_seek: true,
            ..desired(late_empty_generation, 1, false, 40.0)
        },
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );
    let late_rejected = late_empty_live.observe(
        PlayerTransportObservation::new(late_empty_generation, 1.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_seekable(true)
            .with_timeline_kind(PlayerTimelineKind::SlidingLive)
            .with_seekable_ranges(Vec::new()),
    );
    assert!(!late_rejected.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_),
            ..
        }
    )));
}

#[test]
fn positive_player_evidence_promotes_network_vod_to_a_sliding_live_window() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    coordinator.observe(
        PlayerTransportObservation::new(generation, 0.5)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(90.0)
            .with_logical_pause(true)
            .with_seekable(true)
            .with_timeline_kind(PlayerTimelineKind::SlidingLive)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(80.0, 100.0)])
            .with_known_live_seekable_window(PlayerSeekableRange::new(80.0, 100.0)),
    );
    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            force_seek: true,
            ..desired(generation, 1, false, 20.0)
        },
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );

    let snapshot = coordinator.seek_preparation_snapshot().unwrap();
    assert_eq!(
        snapshot.availability,
        SeekTargetAvailability::OutsideLiveWindow
    );
    assert_eq!(snapshot.requested_target_seconds, 20.0);
    assert_eq!(snapshot.frozen_target_seconds, 80.0);

    let actions = coordinator.observe(
        PlayerTransportObservation::new(generation, 1.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(90.0)
            .with_logical_pause(true)
            .with_seekable(true)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(80.0, 100.0)]),
    );
    assert!(actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if (*position - 80.0).abs() <= f64::EPSILON
    )));
    assert!(!actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if (*position - 20.0).abs() <= f64::EPSILON
    )));
}

#[test]
fn cached_disjoint_live_target_is_not_rewritten_to_the_rightmost_interval() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::LiveSliding);
    coordinator.observe(
        PlayerTransportObservation::new(generation, 0.1)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(90.0)
            .with_logical_pause(true)
            .with_seekable(true)
            .with_timeline_kind(PlayerTimelineKind::SlidingLive)
            .with_seekable_ranges(vec![
                PlayerSeekableRange::new(0.0, 10.0),
                PlayerSeekableRange::new(80.0, 100.0),
            ])
            .with_known_live_seekable_window(PlayerSeekableRange::new(80.0, 100.0)),
    );
    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            force_seek: true,
            ..desired(generation, 1, true, 5.0)
        },
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );

    let actions = coordinator.observe(
        PlayerTransportObservation::new(generation, 0.2)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(90.0)
            .with_logical_pause(true)
            .with_seekable(true)
            .with_timeline_kind(PlayerTimelineKind::SlidingLive),
    );
    assert!(actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(5.0),
            ..
        }
    )));
    assert!(!actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if *position >= 80.0
    )));
}

#[test]
fn late_live_window_does_not_rewrite_a_cached_disjoint_preparation_target() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::LiveSliding);
    coordinator.observe(
        PlayerTransportObservation::new(generation, 0.1)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(90.0)
            .with_logical_pause(true)
            .with_seekable(true)
            .with_timeline_kind(PlayerTimelineKind::Unknown),
    );
    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            force_seek: true,
            ..desired(generation, 1, true, 5.0)
        },
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );
    assert_eq!(
        coordinator
            .seek_preparation_snapshot()
            .map(|snapshot| snapshot.availability),
        Some(SeekTargetAvailability::Unknown)
    );

    let actions = coordinator.observe(
        PlayerTransportObservation::new(generation, 0.2)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(90.0)
            .with_logical_pause(true)
            .with_seekable(true)
            .with_timeline_kind(PlayerTimelineKind::SlidingLive)
            .with_seekable_ranges(vec![
                PlayerSeekableRange::new(0.0, 10.0),
                PlayerSeekableRange::new(80.0, 100.0),
            ])
            .with_known_live_seekable_window(PlayerSeekableRange::new(80.0, 100.0)),
    );
    let snapshot = coordinator
        .seek_preparation_snapshot()
        .expect("the cached target should remain in its preparation episode");
    assert_eq!(snapshot.availability, SeekTargetAvailability::Cached);
    assert_eq!(snapshot.requested_target_seconds, 5.0);
    assert_eq!(snapshot.frozen_target_seconds, 5.0);
    assert!(actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(5.0),
            ..
        }
    )));
    assert!(!actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if *position >= 80.0
    )));
}

#[test]
fn stale_live_classification_cannot_promote_a_newer_vod_observation() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    coordinator.observe(
        PlayerTransportObservation::new(generation, 2.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(10.0)
            .with_logical_pause(true)
            .with_timeline_kind(PlayerTimelineKind::Vod),
    );

    coordinator.observe(
        PlayerTransportObservation::new(generation, 1.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(90.0)
            .with_logical_pause(true)
            .with_timeline_kind(PlayerTimelineKind::SlidingLive)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(80.0, 100.0)])
            .with_known_live_seekable_window(PlayerSeekableRange::new(80.0, 100.0)),
    );

    assert_eq!(
        coordinator.media.as_ref().map(|media| media.kind),
        Some(MediaTransportKind::NetworkVod)
    );
    assert_eq!(
        coordinator
            .observed
            .and_then(|observed| observed.timeline_kind),
        Some(PlayerTimelineKind::Vod)
    );
    assert_eq!(coordinator.metrics().stale_timestamp_observations, 1);
}

#[test]
fn empty_live_cache_snapshot_clears_the_previous_usable_interval() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    coordinator.observe(
        PlayerTransportObservation::new(generation, 1.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(90.0)
            .with_logical_pause(true)
            .with_timeline_kind(PlayerTimelineKind::SlidingLive)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(80.0, 100.0)])
            .with_known_live_seekable_window(PlayerSeekableRange::new(80.0, 100.0)),
    );
    assert_eq!(
        coordinator
            .observed
            .and_then(|observed| observed.known_live_seekable_window),
        Some(PlayerSeekableRange::new(80.0, 100.0))
    );

    coordinator.observe(
        PlayerTransportObservation::new(generation, 2.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(90.0)
            .with_logical_pause(true)
            .with_timeline_kind(PlayerTimelineKind::SlidingLive)
            .with_seekable_ranges(Vec::new()),
    );
    assert_eq!(
        coordinator
            .observed
            .and_then(|observed| observed.known_live_seekable_window),
        None
    );
    assert_eq!(coordinator.cached_seekable_ranges.as_deref(), Some(&[][..]));
}

#[test]
fn local_explicit_seek_keeps_the_existing_direct_path() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::LocalFile);
    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            force_seek: true,
            ..desired(generation, 1, true, 40.0)
        },
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );
    let actions = coordinator.observe(
        PlayerTransportObservation::new(generation, 1.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_seekable(true),
    );
    assert!(coordinator.seek_preparation_snapshot().is_none());
    assert!(actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if (*position - 40.0).abs() <= f64::EPSILON
    )));
}

#[test]
fn transport_refresh_preserves_frozen_episode_and_recovery_budget() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    begin_fetch_required_seek(&mut coordinator, generation, 1, 40.0);
    let original = coordinator.seek_preparation_snapshot().unwrap();
    let refresh = coordinator.prepare_media_with_intent(
        LogicalMediaId::new("episode-1").unwrap(),
        MediaTransportKind::NetworkVod,
        MediaLoadIntent::TransportRefresh,
        2.0,
    );
    let refreshed = coordinator.seek_preparation_snapshot().unwrap();
    assert_eq!(refresh.media_generation, generation);
    assert_eq!(refresh.load_attempt, 2);
    assert_eq!(refreshed.id, original.id);
    assert_eq!(refreshed.frozen_target_seconds, 40.0);
    assert_eq!(refreshed.load_attempt, 2);
    assert_eq!(refreshed.availability, SeekTargetAvailability::Unknown);

    coordinator.recovery = Some(RecoveryEpisode {
        id: 99,
        media_generation: generation,
        entered_at_seconds: 2.0,
        hard_seek_attempts: 1,
        post_cache_baseline_at_seconds: None,
        stable_since_seconds: None,
        catchup_deadline_seconds: None,
        decision_made: true,
        cache_metrics_frozen_until_decision: false,
        catchup_active: false,
        seek_active: false,
        gentle_catchup_only: false,
        degraded: false,
    });
    coordinator.prepare_media_with_intent(
        LogicalMediaId::new("episode-1").unwrap(),
        MediaTransportKind::NetworkVod,
        MediaLoadIntent::TransportRefresh,
        3.0,
    );
    assert_eq!(coordinator.recovery_episode().unwrap().id, 99);
    assert_eq!(
        coordinator.recovery_episode().unwrap().hard_seek_attempts,
        1
    );
}

#[test]
fn transport_refresh_rearms_ready_handoff_instead_of_projecting_stale_readiness() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    begin_fetch_required_seek(&mut coordinator, generation, 1, 40.0);
    coordinator.observe(
        PlayerTransportObservation::new(generation, 2.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(40.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(35.0, 50.0)])
            .with_buffered_ahead_seconds(4.0),
    );
    let ready = coordinator
        .last_seek_preparation_terminal_snapshot()
        .unwrap();
    assert_eq!(
        ready.terminal_outcome,
        Some(SeekPreparationTerminalOutcome::Ready)
    );
    coordinator.recovery.as_mut().unwrap().hard_seek_attempts = 1;

    let refresh = coordinator.prepare_media_with_intent(
        LogicalMediaId::new("episode-1").unwrap(),
        MediaTransportKind::NetworkVod,
        MediaLoadIntent::TransportRefresh,
        3.0,
    );
    let rearmed = coordinator.seek_preparation_snapshot().unwrap();
    assert_eq!(refresh.load_attempt, 2);
    assert_eq!(rearmed.id, ready.id);
    assert_eq!(rearmed.load_attempt, 2);
    assert_eq!(rearmed.frozen_target_seconds, 40.0);
    assert_eq!(rearmed.phase, SeekPreparationPhase::Seeking);
    assert_eq!(rearmed.terminal_outcome, None);
    assert_eq!(rearmed.cache_buffering_percent, None);
    assert_eq!(rearmed.buffered_ahead_seconds, None);
    assert_eq!(coordinator.last_seek_preparation_terminal_outcome(), None);
    assert_eq!(
        coordinator.recovery_episode().unwrap().hard_seek_attempts,
        1
    );
}

#[test]
fn adapter_epoch_reset_never_reissues_the_same_primary_seek() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    let initial = begin_fetch_required_seek(&mut coordinator, generation, 1, 40.0);
    assert_eq!(
        initial
            .iter()
            .filter(|action| matches!(
                action,
                PlaybackCoordinatorAction::Execute {
                    command: CoordinatorPlayerCommand::SetPosition(_),
                    ..
                }
            ))
            .count(),
        1
    );
    coordinator.reset_transport_adapter_epoch(2.0);
    let replacement_observation = coordinator.observe(
        PlayerTransportObservation::new(generation, 2.1)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(0.0, 10.0)]),
    );
    assert!(!replacement_observation.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_),
            ..
        }
    )));
    assert!(coordinator.seek_preparation.is_some_and(|episode| {
        episode.primary_seek_issued && episode.primary_seek_command_id.is_none()
    }));
}

#[test]
fn adapter_epoch_reset_preserves_consumed_hard_seek_budget() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    coordinator.update_desired_room_state(desired(generation, 1, false, 80.0));
    coordinator.recovery = Some(RecoveryEpisode {
        id: 1,
        media_generation: generation,
        entered_at_seconds: 0.0,
        hard_seek_attempts: 1,
        post_cache_baseline_at_seconds: Some(0.0),
        stable_since_seconds: None,
        catchup_deadline_seconds: None,
        decision_made: true,
        cache_metrics_frozen_until_decision: false,
        catchup_active: false,
        seek_active: false,
        gentle_catchup_only: false,
        degraded: false,
    });

    coordinator.reset_transport_adapter_epoch(1.0);
    assert_eq!(
        coordinator.recovery_episode().unwrap().hard_seek_attempts,
        1
    );
    coordinator.observe(
        PlayerTransportObservation::new(generation, 1.1)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(10.0)
            .with_logical_pause(false)
            .with_cache_pause(true)
            .with_seeking(false)
            .with_seekable(true),
    );
    coordinator.observe(playing(generation, 2.0, 10.0));
    let actions = coordinator.observe(playing(generation, 3.0, 10.5));
    assert!(!actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_),
            ..
        }
    )));
    assert!(actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Degraded {
            reason: DegradedPlaybackReason::HardSeekBudgetExhausted,
            ..
        }
    )));
    assert!(coordinator.recovery_episode().unwrap().degraded);
}

#[test]
fn manual_supersession_cancels_dispatched_preparation_and_new_media_clears_terminal() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    begin_fetch_required_seek(&mut coordinator, generation, 1, 40.0);
    coordinator.interrupt_recovery();
    assert!(coordinator.seek_preparation_snapshot().is_none());
    assert_eq!(
        coordinator.last_seek_preparation_terminal_outcome(),
        Some(SeekPreparationTerminalOutcome::Cancelled)
    );

    coordinator.prepare_media(
        LogicalMediaId::new("unrelated-media").unwrap(),
        MediaTransportKind::NetworkVod,
        2.0,
    );
    assert_eq!(coordinator.last_seek_preparation_terminal_outcome(), None);
    assert!(
        coordinator
            .last_seek_preparation_terminal_snapshot()
            .is_none()
    );
}

#[test]
fn preparation_actions_and_terminal_outcomes_are_bounded_and_truthful() {
    let (mut cancel, generation) = coordinator(MediaTransportKind::NetworkVod);
    cancel.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            force_seek: true,
            ..desired(generation, 1, false, 40.0)
        },
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );
    assert!(
        cancel
            .seek_preparation_snapshot()
            .unwrap()
            .can_cancel_and_remain
    );
    cancel.cancel_seek_preparation(0.1);
    assert_eq!(
        cancel.last_seek_preparation_terminal_outcome(),
        Some(SeekPreparationTerminalOutcome::Cancelled)
    );

    let (mut dispatched, generation) = coordinator(MediaTransportKind::NetworkVod);
    begin_fetch_required_seek(&mut dispatched, generation, 1, 20.0);
    assert!(
        !dispatched
            .seek_preparation_snapshot()
            .unwrap()
            .can_cancel_and_remain
    );
    dispatched.cancel_seek_preparation(1.1);
    assert!(dispatched.seek_preparation_snapshot().is_some());

    let timeout_at = dispatched
        .seek_preparation
        .as_ref()
        .unwrap()
        .deadline_seconds;
    let timeout = dispatched.tick(timeout_at + 0.1);
    assert!(timeout.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Degraded {
            reason: DegradedPlaybackReason::RecoveryCommandTimedOut,
            ..
        }
    )));
    assert_eq!(
        dispatched.last_seek_preparation_terminal_outcome(),
        Some(SeekPreparationTerminalOutcome::Degraded(
            SeekPreparationDegradedReason::TimedOut
        ))
    );

    let post_timeout = dispatched.observe(
        PlayerTransportObservation::new(generation, timeout_at + 0.2)
            .with_phase(PlayerTransportPhase::Playing)
            .with_position(5.0)
            .with_logical_pause(false)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true),
    );
    assert!(
        !post_timeout.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPosition(_),
                ..
            }
        )),
        "a degraded preparation must not fall back into an automatic seek loop"
    );
    assert!(dispatched.ordinary_correction_blocked());

    dispatched.interrupt_recovery();
    assert_eq!(dispatched.last_seek_preparation_terminal_outcome(), None);
}

#[test]
fn rejected_preparation_seek_projects_transport_degradation_once() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    let dispatched = begin_fetch_required_seek(&mut coordinator, generation, 1, 40.0);
    let command_id = dispatched
        .iter()
        .find_map(|action| match action {
            PlaybackCoordinatorAction::Execute {
                command_id,
                command: CoordinatorPlayerCommand::SetPosition(40.0),
            } => Some(*command_id),
            _ => None,
        })
        .expect("preparation seek should be dispatched");

    assert!(coordinator.command_failed(command_id, 1.1));
    assert_eq!(coordinator.diagnostic(), PlaybackDiagnostic::Degraded);
    assert_eq!(
        coordinator.last_seek_preparation_terminal_outcome(),
        Some(SeekPreparationTerminalOutcome::Degraded(
            SeekPreparationDegradedReason::TransportFailed
        ))
    );
    let projected = coordinator.tick(1.1);
    assert_eq!(
        projected
            .iter()
            .filter(|action| matches!(
                action,
                PlaybackCoordinatorAction::Degraded {
                    reason: DegradedPlaybackReason::TransportFailed,
                    ..
                }
            ))
            .count(),
        1
    );
    assert!(coordinator.tick(1.2).is_empty());
    let held = coordinator.observe(
        PlayerTransportObservation::new(generation, 1.3)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_seekable(true),
    );
    assert!(!held.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_) | CoordinatorPlayerCommand::Play(_),
            ..
        } | PlaybackCoordinatorAction::Started { .. }
    )));
    assert_eq!(coordinator.diagnostic(), PlaybackDiagnostic::Degraded);
}

#[test]
fn preparation_seek_uses_the_extendable_fetch_deadline_not_generic_command_timeout() {
    let mut coordinator = PlaybackCoordinator::new(PlaybackCoordinatorConfig {
        command_timeout_seconds: 1.0,
        seek_preparation_timeout_seconds: 20.0,
        ..PlaybackCoordinatorConfig::default()
    });
    let generation = coordinator
        .prepare_media(
            LogicalMediaId::new("slow-seek").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;
    let initial = begin_fetch_required_seek(&mut coordinator, generation, 1, 40.0);
    let command_id = initial
        .iter()
        .find_map(|action| match action {
            PlaybackCoordinatorAction::Execute {
                command_id,
                command: CoordinatorPlayerCommand::SetPosition(40.0),
            } => Some(*command_id),
            _ => None,
        })
        .expect("preparation should dispatch one primary seek");
    assert!(coordinator.command_accepted(command_id));
    coordinator.observe(
        PlayerTransportObservation::new(generation, 2.0)
            .with_phase(PlayerTransportPhase::Seeking)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(true)
            .with_seekable(true)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(0.0, 10.0)]),
    );

    let before_original_deadline = coordinator.tick(12.0);
    assert!(before_original_deadline.is_empty());
    assert!(coordinator.seek_preparation_snapshot().is_some());
    assert!(
        coordinator
            .pending_commands
            .iter()
            .any(|command| command.id == command_id)
    );

    coordinator.keep_waiting_for_seek_preparation(19.0);
    assert!(coordinator.tick(21.0).is_empty());
    assert!(coordinator.seek_preparation_snapshot().is_some());

    let extended_timeout = coordinator.tick(39.1);
    assert!(extended_timeout.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Degraded {
            reason: DegradedPlaybackReason::RecoveryCommandTimedOut,
            ..
        }
    )));
    assert_eq!(
        coordinator.last_seek_preparation_terminal_outcome(),
        Some(SeekPreparationTerminalOutcome::Degraded(
            SeekPreparationDegradedReason::TimedOut
        ))
    );
}

#[test]
fn cancel_and_remain_holds_the_current_room_revision_until_explicit_supersession() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            force_seek: true,
            ..desired(generation, 1, false, 40.0)
        },
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );
    coordinator.cancel_seek_preparation(0.1);

    for (at, position) in [(1.0, 5.0), (2.0, 6.0)] {
        let actions = coordinator.observe(playing(generation, at, position));
        assert!(!actions.iter().any(|action| matches!(
            action,
            PlaybackCoordinatorAction::Execute {
                command: CoordinatorPlayerCommand::SetPosition(_),
                ..
            }
        )));
    }
    assert_eq!(
        coordinator.last_seek_preparation_terminal_outcome(),
        Some(SeekPreparationTerminalOutcome::Cancelled)
    );
    assert!(coordinator.ordinary_correction_blocked());

    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            force_seek: true,
            ..desired(generation, 2, false, 50.0)
        },
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );
    assert!(coordinator.seek_preparation_snapshot().is_some());
}

#[test]
fn ordinary_reconnect_revision_rebinds_degraded_hold_without_direct_reseek() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    begin_fetch_required_seek(&mut coordinator, generation, 1, 40.0);
    let deadline = coordinator
        .seek_preparation
        .as_ref()
        .unwrap()
        .deadline_seconds;
    coordinator.tick(deadline + 0.1);

    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            force_seek: true,
            ..desired(generation, 2, false, 40.0)
        },
        DesiredRoomPlaybackUpdateKind::Ordinary,
    );
    let rebound = coordinator
        .last_seek_preparation_terminal_snapshot()
        .unwrap();
    assert_eq!(rebound.latest_room_revision, 2);
    assert!(matches!(
        rebound.terminal_outcome,
        Some(SeekPreparationTerminalOutcome::Degraded(
            SeekPreparationDegradedReason::TimedOut
        ))
    ));
    let actions = coordinator.observe(
        PlayerTransportObservation::new(generation, deadline + 0.2)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true),
    );
    assert!(!actions.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_),
            ..
        }
    )));
    assert!(coordinator.ordinary_correction_blocked());

    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            force_seek: true,
            ..desired(generation, 3, false, 60.0)
        },
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );
    assert!(coordinator.seek_preparation_snapshot().is_some());
}

#[test]
fn authoritative_alignment_guard_rejects_nearest_buffered_alternative() {
    let (mut coordinator, generation) = coordinator(MediaTransportKind::NetworkVod);
    begin_fetch_required_seek(&mut coordinator, generation, 1, 20.0);
    assert!(
        coordinator
            .seek_preparation
            .as_ref()
            .is_some_and(|episode| {
                episode.primary_seek_issued
                    && episode.nearest_safe_buffered_position_seconds == Some(10.0)
            })
    );

    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            media_generation: generation,
            state_revision: 2,
            paused: false,
            anchor_position_seconds: 20.0,
            anchor_observed_at_seconds: 2.0,
            force_seek: true,
        },
        DesiredRoomPlaybackUpdateKind::AuthoritativeSeekAfterSupersededDispatch,
    );
    assert_eq!(coordinator.authoritative_alignment_guard_revision, Some(2));
    let guarded = coordinator
        .seek_preparation_snapshot()
        .expect("guarded preparation should remain active");
    assert_eq!(guarded.nearest_safe_buffered_position_seconds, Some(10.0));
    assert!(!guarded.can_join_nearest_buffered);
    assert!(
        coordinator
            .join_nearest_buffered_seek_preparation(2.1)
            .is_empty(),
        "a stale UI request must not bypass authoritative alignment"
    );
    assert_eq!(coordinator.authoritative_alignment_guard_revision, Some(2));
}

#[test]
fn join_nearest_before_primary_seek_invalidates_old_headroom_and_stays_conservative() {
    let mut coordinator = PlaybackCoordinator::new(PlaybackCoordinatorConfig {
        maximum_catchup_rate: 1.25,
        ..PlaybackCoordinatorConfig::default()
    });
    let generation = coordinator
        .prepare_media(
            LogicalMediaId::new("nearest-before-primary").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;

    let mut old_position = PlayerTransportObservation::new(generation, 0.5)
        .with_phase(PlayerTransportPhase::ReadyPaused)
        .with_position(5.0)
        .with_logical_pause(true)
        .with_cache_pause(false)
        .with_seeking(false)
        .with_seekable(true)
        .with_seekable_ranges(vec![PlayerSeekableRange::new(0.0, 35.0)])
        .with_buffered_ahead_seconds(30.0);
    old_position.input_rate_bytes_per_second = Some(9_000_000);
    coordinator.observe(old_position);
    assert_eq!(
        coordinator.metrics().last_buffered_ahead_seconds,
        Some(30.0)
    );
    assert_eq!(
        coordinator.metrics().last_input_rate_bytes_per_second,
        Some(9_000_000)
    );

    coordinator.update_desired_room_state_with_kind(
        DesiredRoomPlayback {
            force_seek: true,
            ..desired(generation, 1, false, 40.0)
        },
        DesiredRoomPlaybackUpdateKind::ExplicitSeek,
    );
    let blocked = coordinator.observe(
        PlayerTransportObservation::new(generation, 1.0)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(5.0)
            .with_logical_pause(true)
            .with_cache_pause(true)
            .with_seeking(false)
            .with_seekable(true)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(0.0, 35.0)]),
    );
    assert!(!blocked.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_),
            ..
        }
    )));
    assert!(
        coordinator
            .seek_preparation
            .as_ref()
            .is_some_and(|episode| {
                !episode.primary_seek_issued
                    && episode.nearest_safe_buffered_position_seconds == Some(35.0)
            })
    );
    assert!(
        coordinator
            .seek_preparation_snapshot()
            .is_some_and(|preparation| preparation.can_join_nearest_buffered)
    );
    assert_eq!(
        coordinator.metrics().last_buffered_ahead_seconds,
        Some(30.0)
    );

    let join = coordinator.join_nearest_buffered_seek_preparation(1.1);
    assert_eq!(
        join.iter()
            .filter(|action| matches!(
                action,
                PlaybackCoordinatorAction::Execute {
                    command: CoordinatorPlayerCommand::SetPosition(position),
                    ..
                } if (*position - 35.0).abs() <= f64::EPSILON
            ))
            .count(),
        1
    );
    assert!(!join.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(position),
            ..
        } if (*position - 40.0).abs() <= f64::EPSILON
    )));
    assert_eq!(coordinator.metrics().last_buffered_ahead_seconds, None);
    assert_eq!(coordinator.metrics().last_input_rate_bytes_per_second, None);
    assert_eq!(
        coordinator.last_seek_preparation_terminal_outcome(),
        Some(SeekPreparationTerminalOutcome::Superseded)
    );
    assert!(
        coordinator.recovery.as_ref().is_some_and(|episode| {
            episode.gentle_catchup_only && episode.hard_seek_attempts == 0
        })
    );
    assert!(
        coordinator
            .join_nearest_buffered_seek_preparation(1.2)
            .is_empty(),
        "the consumed alternative must not emit a duplicate seek"
    );

    let nearest_applied = coordinator.observe(
        PlayerTransportObservation::new(generation, 2.0)
            .with_phase(PlayerTransportPhase::ReadyPaused)
            .with_position(35.0)
            .with_logical_pause(true)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true),
    );
    let baseline = coordinator.observe(playing(generation, 3.0, 35.0));
    let convergence = coordinator.observe(playing(generation, 4.0, 35.5));
    assert_eq!(coordinator.metrics().last_buffered_ahead_seconds, None);
    assert_eq!(coordinator.metrics().last_input_rate_bytes_per_second, None);
    assert!(convergence.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
            ..
        } if (*rate - CONSERVATIVE_CATCHUP_RATE_WITHOUT_HEADROOM).abs() < f64::EPSILON
    )));
    assert!(
        nearest_applied
            .iter()
            .chain(&baseline)
            .chain(&convergence)
            .all(|action| !matches!(
                action,
                PlaybackCoordinatorAction::Execute {
                    command: CoordinatorPlayerCommand::SetPosition(_),
                    ..
                }
            )),
        "nearest recovery must not hard-seek back to the original target"
    );
}

#[test]
fn join_nearest_stays_coordinator_owned_and_uses_gentle_convergence() {
    let mut coordinator = PlaybackCoordinator::new(PlaybackCoordinatorConfig {
        maximum_catchup_rate: 1.25,
        ..PlaybackCoordinatorConfig::default()
    });
    let generation = coordinator
        .prepare_media(
            LogicalMediaId::new("nearest-with-headroom").unwrap(),
            MediaTransportKind::NetworkVod,
            0.0,
        )
        .media_generation;
    begin_fetch_required_seek(&mut coordinator, generation, 1, 40.0);
    coordinator.observe(
        PlayerTransportObservation::new(generation, 2.0)
            .with_phase(PlayerTransportPhase::Rebuffering)
            .with_position(40.0)
            .with_logical_pause(false)
            .with_cache_pause(true)
            .with_seeking(false)
            .with_seekable(true)
            .with_seekable_ranges(vec![PlayerSeekableRange::new(0.0, 35.0)]),
    );
    let preparation = coordinator.seek_preparation_snapshot().unwrap();
    assert_eq!(
        preparation.nearest_safe_buffered_position_seconds,
        Some(35.0)
    );
    assert!(preparation.can_join_nearest_buffered);

    let join = coordinator.join_nearest_buffered_seek_preparation(2.1);
    assert_eq!(
        join.iter()
            .filter(|action| matches!(
                action,
                PlaybackCoordinatorAction::Execute {
                    command: CoordinatorPlayerCommand::SetPosition(position),
                    ..
                } if (*position - 35.0).abs() <= f64::EPSILON
            ))
            .count(),
        1
    );
    assert!(
        coordinator.recovery.as_ref().is_some_and(|episode| {
            episode.gentle_catchup_only && episode.hard_seek_attempts == 0
        })
    );

    coordinator.observe(
        PlayerTransportObservation::new(generation, 3.0)
            .with_phase(PlayerTransportPhase::Playing)
            .with_position(35.0)
            .with_logical_pause(false)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true)
            .with_buffered_ahead_seconds(4.0),
    );
    let convergence = coordinator.observe(
        PlayerTransportObservation::new(generation, 4.0)
            .with_phase(PlayerTransportPhase::Playing)
            .with_position(35.5)
            .with_logical_pause(false)
            .with_cache_pause(false)
            .with_seeking(false)
            .with_seekable(true)
            .with_buffered_ahead_seconds(4.0),
    );
    assert!(convergence.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPlaybackRate(rate),
            ..
        } if (*rate - CONSERVATIVE_CATCHUP_RATE_WITHOUT_HEADROOM).abs() < f64::EPSILON
    )));
    assert!(!convergence.iter().any(|action| matches!(
        action,
        PlaybackCoordinatorAction::Execute {
            command: CoordinatorPlayerCommand::SetPosition(_),
            ..
        }
    )));
    assert!(coordinator.ordinary_correction_blocked());
}

#[test]
fn nearest_buffered_target_is_only_offered_within_the_configured_limit() {
    let ranges = [PlayerSeekableRange::new(0.0, 30.0)];
    assert_eq!(
        nearest_buffered_target(MediaTransportKind::NetworkVod, 40.0, Some(&ranges), 15.0),
        Some(30.0)
    );
    assert_eq!(
        nearest_buffered_target(MediaTransportKind::NetworkVod, 50.0, Some(&ranges), 15.0),
        None
    );
    assert_eq!(
        nearest_buffered_target(
            MediaTransportKind::NetworkVod,
            40.0,
            Some(&[
                PlayerSeekableRange::new(0.0, 30.0),
                PlayerSeekableRange::new(40.0, 50.0),
            ]),
            15.0,
        ),
        None,
        "a target already present in any disjoint range needs no alternative"
    );
    assert_eq!(
        nearest_buffered_target(
            MediaTransportKind::LiveSliding,
            105.0,
            Some(&[PlayerSeekableRange::new(80.0, 100.0)]),
            15.0,
        ),
        Some(99.0),
        "a live alternative must stay behind the range's write edge"
    );
}

#[test]
fn seek_preparation_config_is_normalized_and_bounded() {
    let config = PlaybackCoordinatorConfig {
        command_timeout_seconds: 5.0,
        seek_preparation_timeout_seconds: f64::INFINITY,
        seek_preparation_minimum_headroom_seconds: -1.0,
        nearest_buffered_target_limit_seconds: 500.0,
        ..PlaybackCoordinatorConfig::default()
    }
    .normalized();
    assert_eq!(
        config.seek_preparation_timeout_seconds,
        SEEK_PREPARATION_TIMEOUT_SECONDS
    );
    assert_eq!(
        config.seek_preparation_minimum_headroom_seconds,
        HEALTHY_CATCHUP_BUFFER_SECONDS
    );
    assert_eq!(config.nearest_buffered_target_limit_seconds, 60.0);

    let large_command_timeout = PlaybackCoordinatorConfig {
        command_timeout_seconds: 600.0,
        seek_preparation_timeout_seconds: 60.0,
        ..PlaybackCoordinatorConfig::default()
    }
    .normalized();
    assert_eq!(large_command_timeout.command_timeout_seconds, 600.0);
    assert_eq!(
        large_command_timeout.seek_preparation_timeout_seconds,
        MAXIMUM_CATCHUP_EPISODE_SECONDS
    );
}
