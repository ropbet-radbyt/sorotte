use super::*;
use sorotte_player_api::{
    PlayerAdapter, PlayerCapabilities, PlayerCommand, PlayerCommandId, PlayerCommandProgress,
};

impl PlayerAdapter for MpvAdapter {
    fn name(&self) -> &'static str {
        "mpv"
    }

    fn capabilities(&self) -> PlayerCapabilities {
        if self.is_connected() || self.simulation_mode {
            PlayerCapabilities::ALL
        } else {
            PlayerCapabilities::NONE
        }
    }

    fn execute_tracked(&mut self, command: PlayerCommand) -> Result<PlayerCommandId, PlayerError> {
        self.ensure_transport_observers_registered_if_attached();

        let (command_id, supersession, play_intent) = match &command {
            PlayerCommand::OpenFile(_) => {
                let generation = PlayerMediaGeneration::new(self.next_media_generation.max(1));
                let command_id = self.register_tracked_command(
                    Some(generation),
                    TrackedCommandKind::Load {
                        file_loaded: false,
                        ready: false,
                    },
                );
                (command_id, TrackedCommandSupersession::Load, None)
            }
            PlayerCommand::SetPosition(target_seconds) => {
                let command_id = self.register_tracked_command(
                    self.media_generation(),
                    TrackedCommandKind::Seek {
                        target_seconds: *target_seconds,
                        seeking_finished: false,
                        position_in_tolerance: false,
                    },
                );
                (command_id, TrackedCommandSupersession::Seek, None)
            }
            PlayerCommand::SetPaused(true) => {
                let command_id = self.register_tracked_command(
                    self.media_generation(),
                    TrackedCommandKind::Pause {
                        logical_pause_observed: false,
                    },
                );
                (command_id, TrackedCommandSupersession::PauseOrPlay, None)
            }
            PlayerCommand::SetPaused(false) | PlayerCommand::Play(PlayerPlayIntent::Resume) => {
                let intent = PlayerPlayIntent::Resume;
                let command_id = self.register_tracked_command(
                    self.media_generation(),
                    TrackedCommandKind::Play {
                        intent,
                        restart_sequence_baseline: self.playback_restart_sequence,
                        position_baseline: self.observed_state.position_seconds,
                        logical_play_observed: false,
                        cache_clear_observed: self.observed_state.paused_for_cache == Some(false),
                        restart_observed: false,
                        forward_advancement_observed: false,
                    },
                );
                (
                    command_id,
                    TrackedCommandSupersession::PauseOrPlay,
                    Some(intent),
                )
            }
            PlayerCommand::Play(intent) => {
                let restart_sequence_baseline = match intent {
                    PlayerPlayIntent::Resume => self.playback_restart_sequence,
                    PlayerPlayIntent::StartAfterLoad {
                        baseline_restart_sequence,
                    }
                    | PlayerPlayIntent::StartAfterSeek {
                        baseline_restart_sequence,
                    } => *baseline_restart_sequence,
                };
                let command_id = self.register_tracked_command(
                    self.media_generation(),
                    TrackedCommandKind::Play {
                        intent: *intent,
                        restart_sequence_baseline,
                        position_baseline: self.observed_state.position_seconds,
                        logical_play_observed: false,
                        cache_clear_observed: self.observed_state.paused_for_cache == Some(false),
                        restart_observed: self.playback_restart_sequence
                            > restart_sequence_baseline,
                        forward_advancement_observed: false,
                    },
                );
                (
                    command_id,
                    TrackedCommandSupersession::PauseOrPlay,
                    Some(*intent),
                )
            }
            _ => return Err(PlayerError::Unsupported("execute_tracked command")),
        };

        let result = match command {
            PlayerCommand::OpenFile(path) => self.open_file(&path),
            PlayerCommand::SetPosition(position_seconds) => {
                let result = self.send_ipc_command_if_attached(json!([
                    MPV_COMMAND_SET_PROPERTY,
                    MPV_PROPERTY_TIME_POS,
                    position_seconds
                ]));
                if result.is_ok() && self.simulation_mode {
                    self.position_seconds = position_seconds;
                }
                result
            }
            PlayerCommand::SetPaused(paused) => {
                let result = self.send_ipc_command_if_attached(json!([
                    MPV_COMMAND_SET_PROPERTY,
                    MPV_PROPERTY_PAUSE,
                    paused
                ]));
                if result.is_ok() && self.simulation_mode {
                    self.paused = paused;
                }
                result
            }
            PlayerCommand::Play(_) => {
                let result = self.send_ipc_command_if_attached(json!([
                    MPV_COMMAND_SET_PROPERTY,
                    MPV_PROPERTY_PAUSE,
                    false
                ]));
                if result.is_ok() && self.simulation_mode {
                    self.paused = false;
                }
                result
            }
            _ => unreachable!("tracked command variants were filtered above"),
        };
        if let Err(error) = result {
            self.discard_unaccepted_tracked_command(command_id);
            return Err(error);
        }

        if self.simulation_mode {
            let media_generation = self.media_generation();
            match supersession {
                TrackedCommandSupersession::Load => {
                    self.observe_tracked_commands(
                        media_generation,
                        TrackedCommandObservation::FileLoaded,
                    );
                    self.observe_tracked_commands(
                        media_generation,
                        TrackedCommandObservation::Phase(self.transport_phase),
                    );
                }
                TrackedCommandSupersession::Seek => {
                    self.observed_state.position_seconds = Some(self.position_seconds);
                    self.observed_state.seeking = Some(false);
                    self.observe_tracked_commands(
                        media_generation,
                        TrackedCommandObservation::Position(self.position_seconds),
                    );
                    self.observe_tracked_commands(
                        media_generation,
                        TrackedCommandObservation::Seeking(false),
                    );
                }
                TrackedCommandSupersession::PauseOrPlay if self.paused => {
                    self.observed_state.paused = Some(true);
                    self.observed_state.logical_pause = Some(true);
                    self.observed_state.paused_for_cache = Some(false);
                    self.observe_tracked_commands(
                        media_generation,
                        TrackedCommandObservation::CachePause(false),
                    );
                    self.observe_tracked_commands(
                        media_generation,
                        TrackedCommandObservation::LogicalPause(true),
                    );
                }
                TrackedCommandSupersession::PauseOrPlay => {
                    self.observed_state.paused = Some(false);
                    self.observed_state.logical_pause = Some(false);
                    self.observed_state.paused_for_cache = Some(false);
                    self.observe_tracked_commands(
                        media_generation,
                        TrackedCommandObservation::LogicalPause(false),
                    );
                    self.observe_tracked_commands(
                        media_generation,
                        TrackedCommandObservation::CachePause(false),
                    );
                    if let Some(intent) = play_intent
                        && !matches!(intent, PlayerPlayIntent::Resume)
                    {
                        let baseline_restart_sequence = match intent {
                            PlayerPlayIntent::Resume => unreachable!("resume was filtered above"),
                            PlayerPlayIntent::StartAfterLoad {
                                baseline_restart_sequence,
                            }
                            | PlayerPlayIntent::StartAfterSeek {
                                baseline_restart_sequence,
                            } => baseline_restart_sequence,
                        };
                        if self.playback_restart_sequence <= baseline_restart_sequence {
                            self.playback_restart_sequence =
                                self.playback_restart_sequence.wrapping_add(1).max(1);
                        }
                        self.observe_tracked_commands(
                            media_generation,
                            TrackedCommandObservation::PlaybackRestart(
                                self.playback_restart_sequence,
                            ),
                        );
                    }
                    self.position_seconds += PLAYBACK_ADVANCEMENT_EPSILON_SECONDS * 2.0;
                    self.observed_state.position_seconds = Some(self.position_seconds);
                    self.observe_tracked_commands(
                        media_generation,
                        TrackedCommandObservation::Position(self.position_seconds),
                    );
                    self.position_seconds += PLAYBACK_ADVANCEMENT_EPSILON_SECONDS * 2.0;
                    self.observed_state.position_seconds = Some(self.position_seconds);
                    self.observe_tracked_commands(
                        media_generation,
                        TrackedCommandObservation::Position(self.position_seconds),
                    );
                }
            }
        }

        self.accept_tracked_command(command_id);
        match supersession {
            TrackedCommandSupersession::Load => self
                .supersede_tracked_commands(Some(command_id), |kind| kind.is_load_seek_or_play()),
            TrackedCommandSupersession::Seek => self
                .supersede_tracked_commands(Some(command_id), |kind| {
                    matches!(kind, TrackedCommandKind::Seek { .. })
                }),
            TrackedCommandSupersession::PauseOrPlay => {
                self.supersede_tracked_commands(Some(command_id), |kind| {
                    matches!(
                        kind,
                        TrackedCommandKind::Pause { .. } | TrackedCommandKind::Play { .. }
                    )
                })
            }
        }
        Ok(command_id)
    }

    fn open_file(&mut self, path: &str) -> Result<(), PlayerError> {
        let generation = self.allocate_media_generation();
        let previous_phase = self.transport_phase;
        self.pending_load_request = Some(path.to_owned());
        self.pending_load_generation = Some(generation);
        self.transport_phase = PlayerTransportPhase::Loading;
        let loading_update = self
            .transport_update_for(generation)
            .with_phase(PlayerTransportPhase::Loading);
        self.queue_transport_telemetry_update(loading_update);

        let load_result =
            if uses_network_media_options(path) && !self.network_media_options.is_empty() {
                self.send_network_media_loadfile(path)
            } else {
                self.send_ipc_command_if_attached(json!([
                    MPV_COMMAND_LOADFILE,
                    path,
                    MPV_LOADFILE_REPLACE
                ]))
            };
        if let Err(error) = load_result {
            if self.pending_load_generation == Some(generation) {
                self.pending_load_request = None;
                self.pending_load_generation = None;
            }
            self.transport_phase = previous_phase;
            let mut failure_update = self
                .transport_update_for(generation)
                .with_phase(PlayerTransportPhase::Failed);
            failure_update.error_kind = Some(PlayerMediaLoadFailureKind::Unknown);
            self.queue_transport_telemetry_update(failure_update);
            return Err(error);
        }

        if self.ipc_client.is_some() {
            // A fast mpv load can deliver start-file/file-loaded before the
            // loadfile command reply. Do not erase those observations after
            // the command returns.
            if self.pending_load_generation == Some(generation) {
                self.current_path = Some(path.to_owned());
                self.pending_local_file_update = None;
                self.observed_state.path = None;
                self.observed_state.duration_seconds = None;
                self.observed_state.size_bytes = None;
                self.paused_for_cache = false;
                self.cache_buffering_percent = None;
                self.observed_state.paused_for_cache = None;
                self.observed_state.cache_buffering_percent = None;
            }
        } else {
            self.active_media_generation = Some(generation);
            self.pending_load_generation = None;
            self.pending_load_request = None;
            self.active_file_loaded = true;
            self.active_generation_has_restarted = !self.paused;
            self.current_path = Some(path.to_owned());
            self.pending_local_file_update = Some(Self::local_file_update_for_path(path));
            self.pending_media_load_outcomes
                .push_back(PlayerMediaLoadOutcome::success(path, Some(path.to_owned())));
            let phase = if self.paused {
                PlayerTransportPhase::ReadyPaused
            } else {
                PlayerTransportPhase::Playing
            };
            self.set_transport_phase(phase);
        }
        let belongs_to_tracked_load = self.pending_tracked_commands.iter().any(|command| {
            command.accepted_at.is_none()
                && command.media_generation == Some(generation)
                && matches!(&command.kind, TrackedCommandKind::Load { .. })
        });
        if !belongs_to_tracked_load {
            self.supersede_tracked_commands(None, |kind| kind.is_load_seek_or_play());
        }
        Ok(())
    }

    fn set_option_string(&mut self, name: &str, value: &str) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([MPV_COMMAND_SET, name, value]))?;
        Ok(())
    }

    fn apply_profile(&mut self, profile: &str) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([MPV_COMMAND_APPLY_PROFILE, profile]))?;
        Ok(())
    }

    fn set_paused(&mut self, paused: bool) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([
            MPV_COMMAND_SET_PROPERTY,
            MPV_PROPERTY_PAUSE,
            paused
        ]))?;
        self.paused = paused;
        // This records requested user/room intent only; command application is
        // still acknowledged exclusively by later property observations. It
        // lets a cache release distinguish an intentional pause from mpv's
        // transient cache-induced `pause=true`.
        self.logical_pause_explicit = paused;
        Ok(())
    }

    fn set_position(&mut self, position_seconds: f64) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([
            MPV_COMMAND_SET_PROPERTY,
            MPV_PROPERTY_TIME_POS,
            position_seconds
        ]))?;
        self.position_seconds = position_seconds;
        Ok(())
    }

    fn set_playback_rate(&mut self, rate: f64) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([
            MPV_COMMAND_SET_PROPERTY,
            MPV_PROPERTY_SPEED,
            rate
        ]))?;
        self.playback_rate = rate;
        Ok(())
    }

    fn set_muted(&mut self, muted: bool) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([
            MPV_COMMAND_SET_PROPERTY,
            MPV_PROPERTY_MUTE,
            muted
        ]))?;
        self.muted = muted;
        Ok(())
    }

    fn set_volume(&mut self, volume: f64) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([
            MPV_COMMAND_SET_PROPERTY,
            MPV_PROPERTY_VOLUME,
            volume
        ]))?;
        self.volume = Some(volume);
        Ok(())
    }

    fn set_deinterlace(&mut self, deinterlace: bool) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([
            MPV_COMMAND_SET_PROPERTY,
            MPV_PROPERTY_DEINTERLACE,
            deinterlace
        ]))?;
        self.deinterlace = deinterlace;
        Ok(())
    }

    fn set_keepaspect(&mut self, keepaspect: bool) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([
            MPV_COMMAND_SET_PROPERTY,
            MPV_PROPERTY_KEEPASPECT,
            keepaspect
        ]))?;
        self.keepaspect = keepaspect;
        Ok(())
    }

    fn set_keepaspect_window(&mut self, keepaspect_window: bool) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([
            MPV_COMMAND_SET_PROPERTY,
            MPV_PROPERTY_KEEPASPECT_WINDOW,
            keepaspect_window
        ]))?;
        self.keepaspect_window = keepaspect_window;
        Ok(())
    }

    fn set_fullscreen(&mut self, fullscreen: bool) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([
            MPV_COMMAND_SET_PROPERTY,
            MPV_PROPERTY_FULLSCREEN,
            fullscreen
        ]))?;
        self.fullscreen = fullscreen;
        Ok(())
    }

    fn set_ontop(&mut self, ontop: bool) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([
            MPV_COMMAND_SET_PROPERTY,
            MPV_PROPERTY_ONTOP,
            ontop
        ]))?;
        self.ontop = ontop;
        Ok(())
    }

    fn set_border(&mut self, border: bool) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([
            MPV_COMMAND_SET_PROPERTY,
            MPV_PROPERTY_BORDER,
            border
        ]))?;
        self.border = border;
        Ok(())
    }

    fn set_force_window(&mut self, force_window: bool) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([
            MPV_COMMAND_SET_PROPERTY,
            MPV_PROPERTY_FORCE_WINDOW,
            force_window
        ]))?;
        self.force_window = force_window;
        Ok(())
    }

    fn set_keep_open(&mut self, keep_open: bool) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([
            MPV_COMMAND_SET_PROPERTY,
            MPV_PROPERTY_KEEP_OPEN,
            keep_open
        ]))?;
        self.keep_open = keep_open;
        Ok(())
    }

    fn set_keep_open_pause(&mut self, keep_open_pause: bool) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([
            MPV_COMMAND_SET_PROPERTY,
            MPV_PROPERTY_KEEP_OPEN_PAUSE,
            keep_open_pause
        ]))?;
        self.keep_open_pause = keep_open_pause;
        Ok(())
    }

    fn set_cursor_autohide_fs_only(
        &mut self,
        cursor_autohide_fs_only: bool,
    ) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([
            MPV_COMMAND_SET_PROPERTY,
            MPV_PROPERTY_CURSOR_AUTOHIDE_FS_ONLY,
            cursor_autohide_fs_only
        ]))?;
        self.cursor_autohide_fs_only = cursor_autohide_fs_only;
        Ok(())
    }

    fn set_stop_screensaver(&mut self, stop_screensaver: bool) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([
            MPV_COMMAND_SET_PROPERTY,
            MPV_PROPERTY_STOP_SCREENSAVER,
            stop_screensaver
        ]))?;
        self.stop_screensaver = stop_screensaver;
        Ok(())
    }

    fn set_sub_visibility(&mut self, sub_visibility: bool) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([
            MPV_COMMAND_SET_PROPERTY,
            MPV_PROPERTY_SUB_VISIBILITY,
            sub_visibility
        ]))?;
        self.sub_visibility = sub_visibility;
        Ok(())
    }

    fn set_osd_bar(&mut self, osd_bar: bool) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([
            MPV_COMMAND_SET_PROPERTY,
            MPV_PROPERTY_OSD_BAR,
            osd_bar
        ]))?;
        self.osd_bar = osd_bar;
        Ok(())
    }

    fn set_window_maximized(&mut self, window_maximized: bool) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([
            MPV_COMMAND_SET_PROPERTY,
            MPV_PROPERTY_WINDOW_MAXIMIZED,
            window_maximized
        ]))?;
        self.window_maximized = window_maximized;
        Ok(())
    }

    fn set_window_minimized(&mut self, window_minimized: bool) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([
            MPV_COMMAND_SET_PROPERTY,
            MPV_PROPERTY_WINDOW_MINIMIZED,
            window_minimized
        ]))?;
        self.window_minimized = window_minimized;
        Ok(())
    }

    fn take_local_file_update(&mut self) -> Option<LocalFileUpdate> {
        self.poll_ipc_local_file_update_if_attached();
        self.pending_local_file_update.take()
    }

    fn take_playback_telemetry_update(&mut self) -> Option<PlayerPlaybackTelemetryUpdate> {
        self.ensure_transport_observers_registered_if_attached();
        self.drain_ipc_events_if_attached();
        if self.pending_playback_telemetry_update.is_none() {
            self.poll_paused_position_telemetry_if_attached();
        }
        self.pending_playback_telemetry_update.take()
    }

    fn take_transport_telemetry_update(&mut self) -> Option<PlayerTransportTelemetryUpdate> {
        self.ensure_transport_observers_registered_if_attached();
        self.drain_ipc_events_if_attached();
        self.pending_transport_telemetry_updates.pop_front()
    }

    fn take_command_progress(&mut self) -> Option<PlayerCommandProgress> {
        self.ensure_transport_observers_registered_if_attached();
        self.drain_ipc_events_if_attached();
        if self
            .ipc_client
            .as_ref()
            .is_some_and(|ipc_client| !ipc_client.is_healthy())
        {
            self.fail_all_accepted_tracked_commands(
                sorotte_player_api::PlayerCommandFailureKind::TransportDisconnected,
            );
        }
        self.expire_tracked_commands();
        self.pending_command_progress_updates.pop_front()
    }

    fn take_media_load_outcome(&mut self) -> Option<PlayerMediaLoadOutcome> {
        self.ensure_observers_registered_if_attached();
        self.drain_ipc_events_if_attached();
        self.pending_media_load_outcomes.pop_front()
    }

    fn take_pending_chat_request(&mut self) -> Option<String> {
        self.try_send_legacy_syncplayintf_options_if_pending();
        if self.pending_chat_requests.is_empty() && !self.chat_input_polling_enabled() {
            return None;
        }
        self.ensure_observers_registered_if_attached();
        self.drain_ipc_events_if_attached();
        if self.pending_chat_requests.is_empty() {
            self.poll_ipc_events_for_chat_input_if_enabled();
        }
        self.pending_chat_requests.pop_front()
    }
}
