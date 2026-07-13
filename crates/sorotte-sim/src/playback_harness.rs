use sorotte_client_core::{
    CoordinatorCommandId, CoordinatorPlayerCommand, DesiredRoomPlayback, LogicalMediaId,
    MediaTransportKind, PlaybackCoordinator, PlaybackCoordinatorAction, PlaybackCoordinatorConfig,
    PlayerTransportObservation,
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
        for client in &mut self.clients {
            client
                .coordinator
                .update_desired_room_state(DesiredRoomPlayback {
                    media_generation: client.media_generation,
                    state_revision,
                    paused,
                    anchor_position_seconds,
                    anchor_observed_at_seconds,
                    force_seek,
                });
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
    use sorotte_player_api::PlayerTransportPhase;

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
}
