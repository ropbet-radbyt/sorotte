use sorotte_client_core::{
    CoordinatorCommandId, CoordinatorPlayerCommand, DesiredRoomPlayback,
    DesiredRoomPlaybackUpdateKind, LogicalMediaId, MediaTransportKind, PlaybackCoordinator,
    PlaybackCoordinatorAction, PlaybackCoordinatorConfig, PlayerTransportObservation,
};

#[derive(Debug, Clone, PartialEq)]
pub struct RecordedPlaybackCommand {
    pub client_index: usize,
    pub command_id: CoordinatorCommandId,
    pub command: CoordinatorPlayerCommand,
    pub recovery_episode_id: Option<u64>,
}

#[derive(Debug)]
struct SimulatedPlaybackClient {
    coordinator: PlaybackCoordinator,
    media_generation: u64,
}

#[derive(Debug)]
pub struct MultiClientPlaybackHarness {
    clients: Vec<SimulatedPlaybackClient>,
    recorded_commands: Vec<RecordedPlaybackCommand>,
}

impl MultiClientPlaybackHarness {
    pub fn new(
        client_count: usize,
        kind: MediaTransportKind,
        config: PlaybackCoordinatorConfig,
    ) -> Self {
        let clients = (0..client_count)
            .map(|index| {
                let mut coordinator = PlaybackCoordinator::new(config.clone());
                let media_generation = coordinator
                    .prepare_media(
                        LogicalMediaId::new(format!("shared-media-{index}"))
                            .expect("generated logical ID is valid"),
                        kind,
                        0.0,
                    )
                    .media_generation;
                SimulatedPlaybackClient {
                    coordinator,
                    media_generation,
                }
            })
            .collect();
        Self {
            clients,
            recorded_commands: Vec::new(),
        }
    }

    pub fn client_count(&self) -> usize {
        self.clients.len()
    }

    pub fn media_generation(&self, client_index: usize) -> u64 {
        self.clients[client_index].media_generation
    }

    pub fn set_desired_for_all(
        &mut self,
        state_revision: u64,
        paused: bool,
        anchor_position_seconds: f64,
        anchor_observed_at_seconds: f64,
        force_seek: bool,
    ) {
        let update_kind = if force_seek {
            DesiredRoomPlaybackUpdateKind::ExplicitSeek
        } else {
            DesiredRoomPlaybackUpdateKind::Ordinary
        };
        for client in &mut self.clients {
            client.coordinator.update_desired_room_state_with_kind(
                DesiredRoomPlayback {
                    media_generation: client.media_generation,
                    state_revision,
                    paused,
                    anchor_position_seconds,
                    anchor_observed_at_seconds,
                    force_seek,
                },
                update_kind,
            );
        }
    }

    pub fn observe(
        &mut self,
        client_index: usize,
        observation: PlayerTransportObservation,
    ) -> Vec<PlaybackCoordinatorAction> {
        let actions = self.clients[client_index].coordinator.observe(observation);
        let episode = self.clients[client_index]
            .coordinator
            .recovery_episode()
            .map(|episode| episode.id);
        for action in &actions {
            if let PlaybackCoordinatorAction::Execute {
                command_id,
                command,
            } = action
            {
                self.recorded_commands.push(RecordedPlaybackCommand {
                    client_index,
                    command_id: *command_id,
                    command: *command,
                    recovery_episode_id: episode,
                });
            }
        }
        actions
    }

    pub fn accept_recorded_commands(&mut self, client_index: usize) {
        let command_ids = self
            .recorded_commands
            .iter()
            .filter(|command| command.client_index == client_index)
            .map(|command| command.command_id)
            .collect::<Vec<_>>();
        for command_id in command_ids {
            let _ = self.clients[client_index]
                .coordinator
                .command_accepted(command_id);
        }
    }

    pub fn recorded_commands(&self) -> &[RecordedPlaybackCommand] {
        &self.recorded_commands
    }

    pub fn position_command_count_for_episode(
        &self,
        client_index: usize,
        recovery_episode_id: u64,
    ) -> usize {
        self.recorded_commands
            .iter()
            .filter(|command| {
                command.client_index == client_index
                    && command.recovery_episode_id == Some(recovery_episode_id)
                    && matches!(command.command, CoordinatorPlayerCommand::SetPosition(_))
            })
            .count()
    }

    pub fn coordinator(&self, client_index: usize) -> &PlaybackCoordinator {
        &self.clients[client_index].coordinator
    }
}

#[cfg(test)]
mod tests {
    use sorotte_player_api::{PlayerSeekableRange, PlayerTransportPhase};

    use super::*;

    fn observation(
        generation: u64,
        at: f64,
        phase: PlayerTransportPhase,
        position: f64,
        cache_paused: bool,
    ) -> PlayerTransportObservation {
        PlayerTransportObservation::new(generation, at)
            .with_phase(phase)
            .with_position(position)
            .with_logical_pause(false)
            .with_cache_pause(cache_paused)
            .with_seeking(false)
            .with_seekable(true)
    }

    #[test]
    fn one_buffer_episode_never_emits_unbounded_position_commands() {
        let config = PlaybackCoordinatorConfig {
            maximum_hard_seeks_per_episode: 1,
            stability_interval_seconds: 5.0,
            ..PlaybackCoordinatorConfig::default()
        };
        let mut harness =
            MultiClientPlaybackHarness::new(3, MediaTransportKind::NetworkVod, config);
        harness.set_desired_for_all(1, false, 40.0, 10.0, false);
        let generation = harness.media_generation(0);

        for at in 10..20 {
            let actions = harness.observe(
                0,
                observation(
                    generation,
                    at as f64,
                    PlayerTransportPhase::Rebuffering,
                    10.0,
                    true,
                ),
            );
            assert!(actions.is_empty());
        }
        harness.observe(
            0,
            observation(generation, 20.0, PlayerTransportPhase::Playing, 10.1, false),
        );
        harness.observe(
            0,
            observation(generation, 21.0, PlayerTransportPhase::Playing, 10.3, false),
        );
        let episode = harness
            .coordinator(0)
            .recovery_episode()
            .expect("recovery should remain active")
            .id;
        harness.accept_recorded_commands(0);
        let target = harness
            .recorded_commands()
            .iter()
            .find_map(|command| match command.command {
                CoordinatorPlayerCommand::SetPosition(target) if command.client_index == 0 => {
                    Some(target)
                }
                _ => None,
            })
            .expect("large lag should spend one hard seek");

        harness.observe(
            0,
            observation(generation, 21.5, PlayerTransportPhase::Seeking, 10.3, false)
                .with_seeking(true),
        );
        harness.observe(
            0,
            observation(
                generation,
                22.0,
                PlayerTransportPhase::Rebuffering,
                target,
                true,
            ),
        );
        for at in 23..35 {
            let phase = if at % 2 == 0 {
                PlayerTransportPhase::Rebuffering
            } else {
                PlayerTransportPhase::Playing
            };
            harness.observe(
                0,
                observation(
                    generation,
                    at as f64,
                    phase,
                    target + (at - 22) as f64 * 0.1,
                    phase == PlayerTransportPhase::Rebuffering,
                ),
            );
        }

        assert_eq!(
            harness.position_command_count_for_episode(0, episode),
            1,
            "a buffering episode must have a finite hard-seek budget"
        );
        assert_eq!(harness.coordinator(0).metrics().hard_seek_count, 1);
    }

    #[test]
    fn one_stalled_client_does_not_generate_commands_for_healthy_peers() {
        let mut harness = MultiClientPlaybackHarness::new(
            3,
            MediaTransportKind::NetworkVod,
            PlaybackCoordinatorConfig::default(),
        );
        harness.set_desired_for_all(1, false, 10.0, 0.0, false);
        for client_index in 0..3 {
            let generation = harness.media_generation(client_index);
            harness.observe(
                client_index,
                observation(
                    generation,
                    1.0,
                    if client_index == 0 {
                        PlayerTransportPhase::Rebuffering
                    } else {
                        PlayerTransportPhase::Playing
                    },
                    10.0,
                    client_index == 0,
                ),
            );
            if client_index != 0 {
                harness.observe(
                    client_index,
                    observation(generation, 2.0, PlayerTransportPhase::Playing, 11.0, false),
                );
            }
        }

        assert!(harness.recorded_commands().is_empty());
        assert_eq!(harness.coordinator(0).metrics().buffer_episode_count, 1);
        assert_eq!(harness.coordinator(1).metrics().buffer_episode_count, 0);
        assert_eq!(harness.coordinator(2).metrics().buffer_episode_count, 0);
    }

    #[test]
    fn unbuffered_seek_preparation_freezes_one_target_until_an_explicit_seek_supersedes_it() {
        use sorotte_client_core::{
            SeekPreparationPhase, SeekPreparationTerminalOutcome, SeekTargetAvailability,
        };

        let mut harness = MultiClientPlaybackHarness::new(
            1,
            MediaTransportKind::NetworkVod,
            PlaybackCoordinatorConfig::default(),
        );
        let generation = harness.media_generation(0);
        let cached_range = PlayerSeekableRange::new(0.0, 20.0);
        harness.observe(
            0,
            observation(
                generation,
                0.0,
                PlayerTransportPhase::ReadyPaused,
                10.0,
                false,
            )
            .with_seekable_ranges(vec![cached_range]),
        );

        harness.set_desired_for_all(1, false, 80.0, 1.0, true);
        harness.observe(
            0,
            observation(
                generation,
                1.0,
                PlayerTransportPhase::ReadyPaused,
                10.0,
                false,
            )
            .with_seekable_ranges(vec![cached_range]),
        );
        harness.accept_recorded_commands(0);

        let first = harness
            .coordinator(0)
            .seek_preparation_snapshot()
            .expect("out-of-cache VOD seek should begin preparation");
        assert_eq!(first.availability, SeekTargetAvailability::FetchRequired);
        assert_eq!(first.phase, SeekPreparationPhase::Seeking);
        assert_eq!(first.frozen_target_seconds, 80.0);
        let first_episode_id = first.id;

        harness.set_desired_for_all(2, false, 85.0, 2.0, false);
        harness.observe(
            0,
            observation(
                generation,
                2.0,
                PlayerTransportPhase::Rebuffering,
                80.0,
                true,
            )
            .with_seekable_ranges(vec![cached_range])
            .with_cache_buffering_percent(45.0)
            .with_buffered_ahead_seconds(1.5),
        );
        let fetching = harness
            .coordinator(0)
            .seek_preparation_snapshot()
            .expect("ordinary room aging must retain preparation");
        assert_eq!(fetching.id, first_episode_id);
        assert_eq!(fetching.frozen_target_seconds, 80.0);
        assert_eq!(fetching.latest_room_position_seconds, 85.0);
        assert_eq!(fetching.phase, SeekPreparationPhase::Refilling);
        assert_eq!(fetching.cache_buffering_percent, Some(45.0));
        assert_eq!(fetching.buffered_ahead_seconds, Some(1.5));
        assert_eq!(
            harness
                .recorded_commands()
                .iter()
                .filter(|command| matches!(
                    command.command,
                    CoordinatorPlayerCommand::SetPosition(_)
                ))
                .count(),
            1,
            "advancing room timestamps must not restart the primary seek"
        );

        harness.set_desired_for_all(3, false, 100.0, 3.0, true);
        assert_eq!(
            harness
                .coordinator(0)
                .last_seek_preparation_terminal_outcome(),
            Some(SeekPreparationTerminalOutcome::Superseded)
        );
        harness.observe(
            0,
            observation(
                generation,
                3.0,
                PlayerTransportPhase::ReadyPaused,
                80.0,
                false,
            )
            .with_seekable_ranges(vec![cached_range]),
        );
        let replacement = harness
            .coordinator(0)
            .seek_preparation_snapshot()
            .expect("new explicit seek should create a replacement preparation");
        assert_ne!(replacement.id, first_episode_id);
        assert_eq!(replacement.frozen_target_seconds, 100.0);
        assert_eq!(
            harness
                .recorded_commands()
                .iter()
                .filter(|command| matches!(
                    command.command,
                    CoordinatorPlayerCommand::SetPosition(_)
                ))
                .count(),
            2,
            "only a newer explicit seek may issue a replacement primary seek"
        );
    }

    #[test]
    fn missing_cache_ranges_are_unknown_and_local_files_bypass_seek_preparation() {
        use sorotte_client_core::SeekTargetAvailability;

        let mut network = MultiClientPlaybackHarness::new(
            1,
            MediaTransportKind::NetworkVod,
            PlaybackCoordinatorConfig::default(),
        );
        let generation = network.media_generation(0);
        network.observe(
            0,
            observation(
                generation,
                0.0,
                PlayerTransportPhase::ReadyPaused,
                5.0,
                false,
            ),
        );
        network.set_desired_for_all(1, false, 50.0, 1.0, true);
        network.observe(
            0,
            observation(
                generation,
                1.0,
                PlayerTransportPhase::ReadyPaused,
                5.0,
                false,
            ),
        );
        assert_eq!(
            network
                .coordinator(0)
                .seek_preparation_snapshot()
                .expect("network seek without range telemetry still needs preparation")
                .availability,
            SeekTargetAvailability::Unknown
        );

        let mut local = MultiClientPlaybackHarness::new(
            1,
            MediaTransportKind::LocalFile,
            PlaybackCoordinatorConfig::default(),
        );
        let generation = local.media_generation(0);
        local.observe(
            0,
            observation(
                generation,
                0.0,
                PlayerTransportPhase::ReadyPaused,
                5.0,
                false,
            ),
        );
        local.set_desired_for_all(1, false, 50.0, 1.0, true);
        local.observe(
            0,
            observation(
                generation,
                1.0,
                PlayerTransportPhase::ReadyPaused,
                5.0,
                false,
            ),
        );
        assert!(
            local.coordinator(0).seek_preparation_snapshot().is_none(),
            "local files must keep ordinary seek behavior"
        );
    }
}
