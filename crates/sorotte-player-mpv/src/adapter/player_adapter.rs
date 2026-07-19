use super::*;
use sorotte_player_api::{
    PlayerAdapter, PlayerCapabilities, PlayerCommand, PlayerCommandId, PlayerCommandProgress,
};

const NETWORK_OPTIONS_HEARTBEAT_COMMAND_TOKEN: u64 = 1;
const NETWORK_OPTIONS_EVENT_POLL_COMMAND_TOKEN: u64 = 2;
const LEGACY_SYNCPLAYINTF_HEARTBEAT_COMMAND_TOKEN: u64 = 3;

impl MpvAdapter {
    fn is_nonblocking_runtime_lease_event(event: &Value) -> bool {
        if event.get("event").and_then(Value::as_str) != Some(MPV_EVENT_CLIENT_MESSAGE) {
            return false;
        }
        matches!(
            event
                .get("args")
                .and_then(Value::as_array)
                .and_then(|args| args.first())
                .and_then(Value::as_str),
            Some(
                SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_CONFIGURED
                    | SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_OWNERSHIP
                    | SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_HEARTBEAT
                    | SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_ACTIVE_RESULT
                    | SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_TRANSITION_RESULT
                    | LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_OPTIONS_APPLIED
                    | LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_PONG
                    | LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_LEASE_EXPIRED
                    | LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_CHAT
            )
        )
    }

    pub(super) fn drain_runtime_lease_events_nonblocking(&mut self) -> bool {
        let items = self
            .ipc_client
            .as_mut()
            .map(|client| {
                client.take_nonblocking_runtime_items_matching(
                    Self::is_nonblocking_runtime_lease_event,
                )
            })
            .unwrap_or_default();
        let processed_any = !items.is_empty();
        for item in items {
            match item {
                crate::ipc::MpvIpcNonblockingRuntimeItem::Event(event) => {
                    let is_transition_result = event
                        .get("args")
                        .and_then(Value::as_array)
                        .and_then(|args| args.first())
                        .and_then(Value::as_str)
                        == Some(SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_TRANSITION_RESULT);
                    self.handle_client_message_event(&event);
                    if is_transition_result
                        && let Some(result) = self
                            .deferred_network_media_options_hook_transition_result
                            .take()
                    {
                        // Commit the earlier policy result before a later selected ownership,
                        // lease, or command-completion item mutates hook state. Path/property
                        // events can remain queued for the ordinary full pump.
                        self.apply_network_options_hook_transition_result(result, None);
                    }
                }
                crate::ipc::MpvIpcNonblockingRuntimeItem::Completion(completion) => {
                    match completion {
                        crate::ipc::MpvIpcNonblockingCommandCompletion::Succeeded {
                            token: LEGACY_SYNCPLAYINTF_HEARTBEAT_COMMAND_TOKEN,
                        } => {
                            self.legacy_syncplayintf_last_heartbeat_at = Some(Instant::now());
                        }
                        crate::ipc::MpvIpcNonblockingCommandCompletion::Succeeded {
                            token: NETWORK_OPTIONS_HEARTBEAT_COMMAND_TOKEN,
                        } => {
                            if let Some(pending) =
                                self.network_media_options_hook_pending_heartbeat.as_mut()
                            {
                                pending.sent_at = Some(Instant::now());
                            }
                        }
                        crate::ipc::MpvIpcNonblockingCommandCompletion::Succeeded { .. } => {}
                        crate::ipc::MpvIpcNonblockingCommandCompletion::Failed {
                            token,
                            message,
                        } if token == NETWORK_OPTIONS_HEARTBEAT_COMMAND_TOKEN
                            || token == NETWORK_OPTIONS_EVENT_POLL_COMMAND_TOKEN =>
                        {
                            self.invalidate_network_media_options_hook_delivery();
                            self.queue_network_media_options_hook_degraded(
                                PlayerError::OperationFailed(message),
                            );
                        }
                        crate::ipc::MpvIpcNonblockingCommandCompletion::Failed {
                            token: LEGACY_SYNCPLAYINTF_HEARTBEAT_COMMAND_TOKEN,
                            message,
                        } => self.begin_sorotte_bridge_runtime_recovery(
                            SorotteBridgeFailureKind::IpcCommand,
                            format!("failed to renew Sorotte's mpv bridge lease: {message}"),
                            true,
                        ),
                        crate::ipc::MpvIpcNonblockingCommandCompletion::Failed { .. } => {}
                    }
                }
            }
        }
        processed_any
    }

    fn maintain_network_options_hook_lease_nonblocking(&mut self) {
        if !self.network_media_options_hook_is_ready() {
            return;
        }
        if let Some(pending) = self.network_media_options_hook_pending_heartbeat {
            if pending.sent_at.is_some_and(|sent_at| {
                sent_at.elapsed() >= NETWORK_OPTIONS_HOOK_HEARTBEAT_ACK_TIMEOUT
            }) {
                let reason = format!(
                    "Sorotte's mpv network-options hook did not acknowledge heartbeat nonce {}",
                    pending.nonce
                );
                self.invalidate_network_media_options_hook_delivery();
                self.queue_network_media_options_hook_degraded(PlayerError::OperationFailed(
                    reason,
                ));
                return;
            }

            if pending.sent_at.is_none() {
                return;
            }

            // A heartbeat acknowledgement can arrive after mpv's response to the script-message
            // command. Queue a harmless read so the IPC worker continues harvesting events while
            // the async owner is otherwise waiting on unrelated I/O.
            let poll_result = self.ipc_client.as_mut().map(|client| {
                client.try_send_command_expect_success_nonblocking(
                    json!([MPV_COMMAND_GET_PROPERTY, MPV_PROPERTY_PAUSE]),
                    NETWORK_OPTIONS_EVENT_POLL_COMMAND_TOKEN,
                )
            });
            if let Some(Err(error)) = poll_result {
                self.invalidate_network_media_options_hook_delivery();
                self.queue_network_media_options_hook_degraded(PlayerError::OperationFailed(error));
            }
            return;
        }
        if self
            .network_media_options_hook_last_heartbeat_at
            .is_some_and(|last| last.elapsed() < NETWORK_OPTIONS_HOOK_HEARTBEAT_INTERVAL)
        {
            return;
        }

        let nonce = self.next_network_media_options_hook_heartbeat_nonce;
        let payload = json!({
            "protocol": SOROTTE_NETWORK_OPTIONS_PROTOCOL,
            "ownerId": self.legacy_syncplayintf_owner_id,
            "attachmentId": self.legacy_syncplayintf_attachment_id,
            "configurationGeneration": self.network_media_options_generation,
            "heartbeatNonce": nonce,
        });
        let command = json!([
            MPV_COMMAND_SCRIPT_MESSAGE_TO,
            SOROTTE_NETWORK_OPTIONS_SCRIPT_NAME,
            SOROTTE_NETWORK_OPTIONS_HEARTBEAT_MESSAGE,
            payload.to_string(),
        ]);
        match self.ipc_client.as_mut().map(|client| {
            client.try_send_command_expect_success_nonblocking(
                command,
                NETWORK_OPTIONS_HEARTBEAT_COMMAND_TOKEN,
            )
        }) {
            Some(Ok(true)) => {
                self.next_network_media_options_hook_heartbeat_nonce = self
                    .next_network_media_options_hook_heartbeat_nonce
                    .wrapping_add(1)
                    .max(1);
                self.network_media_options_hook_pending_heartbeat =
                    Some(PendingNetworkOptionsHookHeartbeat {
                        nonce,
                        sent_at: None,
                    });
            }
            Some(Ok(false)) => {}
            Some(Err(error)) => {
                self.invalidate_network_media_options_hook_delivery();
                self.queue_network_media_options_hook_degraded(PlayerError::OperationFailed(error));
            }
            None => {}
        }
    }

    fn maintain_legacy_syncplayintf_lease_nonblocking(&mut self) {
        if !matches!(self.sorotte_bridge_health, SorotteBridgeHealth::Ready) {
            return;
        }
        if !self.legacy_syncplay_ui_settings.chat_input_enabled {
            self.legacy_syncplayintf_last_heartbeat_at = None;
            return;
        }
        if !self.legacy_syncplayintf_options_ready()
            || self
                .legacy_syncplayintf_last_heartbeat_at
                .is_some_and(|last| last.elapsed() < LEGACY_SYNCPLAYINTF_HEARTBEAT_INTERVAL)
        {
            return;
        }
        let Some(payload) = self.legacy_syncplayintf_controller_payload() else {
            return;
        };
        let command = json!([
            MPV_COMMAND_SCRIPT_MESSAGE_TO,
            self.legacy_syncplayintf_script_name.as_str(),
            LEGACY_SYNCPLAYINTF_HEARTBEAT_MESSAGE,
            payload,
        ]);
        match self.ipc_client.as_mut().map(|client| {
            client.try_send_command_expect_success_nonblocking(
                command,
                LEGACY_SYNCPLAYINTF_HEARTBEAT_COMMAND_TOKEN,
            )
        }) {
            Some(Ok(true)) => {}
            Some(Ok(false)) => {}
            Some(Err(error)) => self.begin_sorotte_bridge_runtime_recovery(
                SorotteBridgeFailureKind::IpcCommand,
                format!("failed to renew Sorotte's mpv bridge lease: {error}"),
                true,
            ),
            None => {}
        }
    }

    fn maintain_runtime_leases_nonblocking_inner(&mut self) {
        self.drain_runtime_lease_events_nonblocking();
        // The optional bridge uses the same two-second lease as the core hook but has no
        // acknowledgement state that naturally reserves a later slot. Service it first so
        // network-hook event polling cannot monopolize the single IPC worker command slot.
        self.maintain_legacy_syncplayintf_lease_nonblocking();
        self.maintain_network_options_hook_lease_nonblocking();
        self.drain_runtime_lease_events_nonblocking();
    }
}

impl PlayerAdapter for MpvAdapter {
    fn name(&self) -> &'static str {
        "mpv"
    }

    fn maintain_runtime_leases_nonblocking(&mut self) {
        self.maintain_runtime_leases_nonblocking_inner();
    }

    fn maintain_runtime_integrations(&mut self) {
        MpvAdapter::maintain_runtime_integrations(self);
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
                self.begin_seek_cache_evidence_epoch();
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
        self.network_media_options_embedded_load = None;
        self.transport_phase = PlayerTransportPhase::Loading;
        let loading_update = self
            .transport_update_for(generation)
            .with_phase(PlayerTransportPhase::Loading);
        self.queue_transport_telemetry_update(loading_update);

        let load_result =
            if uses_network_media_options(path) && !self.network_media_options.is_empty() {
                self.network_media_options_embedded_load = Some(EmbeddedNetworkMediaOptions {
                    media_generation: generation,
                    requested_target: path.to_owned(),
                });
                self.send_network_media_loadfile(path)
            } else {
                self.send_ipc_command_if_attached(json!([
                    MPV_COMMAND_LOADFILE,
                    path,
                    MPV_LOADFILE_REPLACE
                ]))
            };
        if let Err(error) = load_result {
            if self
                .network_media_options_embedded_load
                .as_ref()
                .is_some_and(|embedded| embedded.media_generation == generation)
            {
                self.network_media_options_embedded_load = None;
            }
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
        self.begin_seek_cache_evidence_epoch();
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
        self.maintain_runtime_integrations();
        self.poll_ipc_local_file_update_if_attached();
        self.pending_local_file_update.take()
    }

    fn take_playback_telemetry_update(&mut self) -> Option<PlayerPlaybackTelemetryUpdate> {
        self.maintain_runtime_integrations();
        self.ensure_transport_observers_registered_if_attached();
        self.drain_ipc_events_if_attached();
        if self.pending_playback_telemetry_update.is_none() {
            self.poll_paused_position_telemetry_if_attached();
        }
        self.pending_playback_telemetry_update.take()
    }

    fn take_transport_telemetry_update(&mut self) -> Option<PlayerTransportTelemetryUpdate> {
        self.maintain_runtime_integrations();
        self.ensure_transport_observers_registered_if_attached();
        self.drain_ipc_events_if_attached();
        self.observe_unhealthy_ipc_transport();
        self.poll_ytdl_live_probe_completion();
        self.pending_transport_telemetry_updates.pop_front()
    }

    fn take_command_progress(&mut self) -> Option<PlayerCommandProgress> {
        self.maintain_runtime_integrations();
        self.ensure_transport_observers_registered_if_attached();
        self.drain_ipc_events_if_attached();
        self.observe_unhealthy_ipc_transport();
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
        self.maintain_runtime_integrations();
        self.ensure_observers_registered_if_attached();
        self.drain_ipc_events_if_attached();
        self.pending_media_load_outcomes.pop_front()
    }

    fn take_pending_chat_request(&mut self) -> Option<String> {
        self.maintain_runtime_integrations();
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

#[cfg(test)]
mod nonblocking_maintenance_tests {
    use super::*;
    use std::{collections::VecDeque, io};

    struct RejectingHeartbeatTransport {
        responses: VecDeque<String>,
    }

    struct DelayedSuccessTransport {
        responses: VecDeque<String>,
        first_response_delay: Option<Duration>,
    }

    struct OrderedHookEventsTransport {
        responses: VecDeque<String>,
        emitted_events: bool,
        emit_ownership_loss: bool,
        response_error: &'static str,
    }

    impl MpvJsonIpcTransport for RejectingHeartbeatTransport {
        fn send_line_until(&mut self, line: &str, _deadline: Instant) -> io::Result<()> {
            let request: Value = serde_json::from_str(line.trim())
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            let request_id = request.get("request_id").cloned().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "test request omitted request_id",
                )
            })?;
            self.responses.push_back(
                json!({"request_id": request_id, "error": "client not found"}).to_string() + "\n",
            );
            Ok(())
        }

        fn read_line_until(&mut self, line: &mut String, _deadline: Instant) -> io::Result<usize> {
            let response = self.responses.pop_front().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "test response queue was empty",
                )
            })?;
            line.clear();
            line.push_str(&response);
            Ok(line.len())
        }
    }

    impl MpvJsonIpcTransport for DelayedSuccessTransport {
        fn send_line_until(&mut self, line: &str, _deadline: Instant) -> io::Result<()> {
            let request: Value = serde_json::from_str(line.trim())
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            let request_id = request.get("request_id").cloned().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "test request omitted request_id",
                )
            })?;
            self.responses.push_back(
                json!({"request_id": request_id, "error": "success"}).to_string() + "\n",
            );
            Ok(())
        }

        fn read_line_until(&mut self, line: &mut String, _deadline: Instant) -> io::Result<usize> {
            if let Some(delay) = self.first_response_delay.take() {
                std::thread::sleep(delay);
            }
            let response = self.responses.pop_front().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "test response queue was empty",
                )
            })?;
            line.clear();
            line.push_str(&response);
            Ok(line.len())
        }
    }

    impl MpvJsonIpcTransport for OrderedHookEventsTransport {
        fn send_line_until(&mut self, line: &str, _deadline: Instant) -> io::Result<()> {
            let request: Value = serde_json::from_str(line.trim())
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            let request_id = request.get("request_id").cloned().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "test request omitted request_id",
                )
            })?;
            if !self.emitted_events {
                let transition_payload = json!({
                    "protocol": SOROTTE_NETWORK_OPTIONS_PROTOCOL,
                    "ownerId": "ordered-owner",
                    "attachmentId": "ordered-attachment",
                    "hookInstanceId": "ordered-hook",
                    "configurationGeneration": 7,
                    "loadSequence": 2,
                    "sourcePath": "https://example.invalid/video",
                    "streamOpenFilename": "https://example.invalid/video",
                    "status": "network-updated",
                });
                self.responses.push_back(
                    json!({
                        "event": MPV_EVENT_CLIENT_MESSAGE,
                        "args": [
                            SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_TRANSITION_RESULT,
                            transition_payload.to_string(),
                        ],
                    })
                    .to_string()
                        + "\n",
                );
                if self.emit_ownership_loss {
                    let ownership_payload = json!({
                        "protocol": SOROTTE_NETWORK_OPTIONS_PROTOCOL,
                        "ownerId": "ordered-owner",
                        "attachmentId": "ordered-attachment",
                        "hookInstanceId": "ordered-hook",
                        "status": "ownership-lost",
                    });
                    self.responses.push_back(
                        json!({
                            "event": MPV_EVENT_CLIENT_MESSAGE,
                            "args": [
                                SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_OWNERSHIP,
                                ownership_payload.to_string(),
                            ],
                        })
                        .to_string()
                            + "\n",
                    );
                }
                self.emitted_events = true;
            }
            self.responses.push_back(
                json!({"request_id": request_id, "error": self.response_error}).to_string() + "\n",
            );
            Ok(())
        }

        fn read_line_until(&mut self, line: &mut String, _deadline: Instant) -> io::Result<usize> {
            let response = self.responses.pop_front().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "test response queue was empty",
                )
            })?;
            line.clear();
            line.push_str(&response);
            Ok(line.len())
        }
    }

    #[test]
    fn rejected_nonblocking_legacy_heartbeat_enters_recovery_without_blocking() {
        let command_timeout = Duration::from_millis(100);
        let mut adapter = MpvAdapter::with_test_transport_and_ipc_timeout(
            RejectingHeartbeatTransport {
                responses: VecDeque::new(),
            },
            command_timeout,
        );
        adapter.enable_test_legacy_chat_input();
        adapter.sorotte_bridge_health = SorotteBridgeHealth::Ready;
        adapter.legacy_syncplayintf_last_heartbeat_at =
            Some(Instant::now() - LEGACY_SYNCPLAYINTF_HEARTBEAT_INTERVAL);

        let queued_at = Instant::now();
        PlayerAdapter::maintain_runtime_leases_nonblocking(&mut adapter);
        assert!(
            queued_at.elapsed() < command_timeout / 2,
            "lease maintenance must not wait for mpv's command response"
        );

        let observation_deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < observation_deadline
            && matches!(adapter.sorotte_bridge_health(), SorotteBridgeHealth::Ready)
        {
            PlayerAdapter::maintain_runtime_leases_nonblocking(&mut adapter);
            std::thread::yield_now();
        }
        assert!(matches!(
            adapter.sorotte_bridge_health(),
            SorotteBridgeHealth::Recovering
        ));
        assert!(adapter.legacy_syncplayintf_last_heartbeat_at.is_none());
    }

    #[test]
    fn slow_successful_network_heartbeat_starts_ack_timeout_after_command_delivery() {
        let response_delay = NETWORK_OPTIONS_HOOK_HEARTBEAT_ACK_TIMEOUT
            .checked_add(Duration::from_millis(400))
            .expect("test delay should fit in Duration");
        let command_timeout = response_delay
            .checked_add(Duration::from_secs(2))
            .expect("test timeout should fit in Duration");
        let mut adapter = MpvAdapter::with_test_transport_and_ipc_timeout(
            DelayedSuccessTransport {
                responses: VecDeque::new(),
                first_response_delay: Some(response_delay),
            },
            command_timeout,
        );
        adapter.network_media_options_hook_enabled = true;
        adapter.network_media_options_hook_loaded = true;
        adapter.network_media_options_hook_configured_generation =
            Some(adapter.network_media_options_generation);
        adapter.set_network_options_hook_health(MpvNetworkOptionsHookHealth::Ready);
        adapter.network_media_options_hook_last_heartbeat_at =
            Some(Instant::now() - NETWORK_OPTIONS_HOOK_HEARTBEAT_INTERVAL);

        let queued_at = Instant::now();
        PlayerAdapter::maintain_runtime_leases_nonblocking(&mut adapter);
        assert!(
            queued_at.elapsed() < NETWORK_OPTIONS_HOOK_HEARTBEAT_ACK_TIMEOUT / 2,
            "queueing a network heartbeat must not wait for mpv's command response"
        );
        assert!(
            adapter
                .network_media_options_hook_pending_heartbeat
                .is_some_and(|pending| pending.sent_at.is_none())
        );

        let pre_delivery_deadline = Instant::now()
            + NETWORK_OPTIONS_HOOK_HEARTBEAT_ACK_TIMEOUT
            + Duration::from_millis(100);
        while Instant::now() < pre_delivery_deadline {
            PlayerAdapter::maintain_runtime_leases_nonblocking(&mut adapter);
            assert_eq!(
                adapter.network_media_options_hook_health,
                MpvNetworkOptionsHookHealth::Ready,
                "an in-flight command must not consume the hook acknowledgement window"
            );
            assert!(
                adapter
                    .network_media_options_hook_pending_heartbeat
                    .is_some_and(|pending| pending.sent_at.is_none())
            );
            std::thread::sleep(Duration::from_millis(5));
        }

        let delivery_deadline = Instant::now() + command_timeout;
        while Instant::now() < delivery_deadline
            && adapter
                .network_media_options_hook_pending_heartbeat
                .is_none_or(|pending| pending.sent_at.is_none())
        {
            PlayerAdapter::maintain_runtime_leases_nonblocking(&mut adapter);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            adapter
                .network_media_options_hook_pending_heartbeat
                .is_some_and(|pending| pending.sent_at.is_some())
        );
        assert_eq!(
            adapter.network_media_options_hook_health,
            MpvNetworkOptionsHookHealth::Ready
        );
    }

    #[test]
    fn nonblocking_transition_result_commits_before_later_ownership_loss() {
        let mut adapter = MpvAdapter::with_test_transport_and_ipc_timeout(
            OrderedHookEventsTransport {
                responses: VecDeque::new(),
                emitted_events: false,
                emit_ownership_loss: true,
                response_error: "success",
            },
            Duration::from_millis(100),
        );
        adapter.legacy_syncplayintf_owner_id = "ordered-owner".to_owned();
        adapter.legacy_syncplayintf_attachment_id = "ordered-attachment".to_owned();
        adapter.network_media_options_generation = 7;
        adapter.network_media_options_hook_enabled = true;
        adapter.network_media_options_hook_loaded = true;
        adapter.network_media_options_hook_instance_id = Some("ordered-hook".to_owned());
        adapter.network_media_options_hook_configured_generation = Some(7);
        adapter.network_media_options_hook_last_accepted_load_sequence = Some(1);
        adapter.set_network_options_hook_health(MpvNetworkOptionsHookHealth::Ready);
        adapter
            .set_network_media_policy_state(MpvNetworkMediaPolicyState::AwaitingAuthoritativeLoad);
        adapter.network_media_options_hook_last_heartbeat_at =
            Some(Instant::now() - NETWORK_OPTIONS_HOOK_HEARTBEAT_INTERVAL);

        let observation_deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < observation_deadline {
            PlayerAdapter::maintain_runtime_leases_nonblocking(&mut adapter);
            let snapshot = adapter.network_options_runtime_health_snapshot();
            if matches!(
                snapshot.hook_health,
                MpvNetworkOptionsHookHealth::Degraded(_)
            ) && snapshot.media_policy == MpvNetworkMediaPolicyState::NetworkMediaUpdated
            {
                break;
            }
            std::thread::yield_now();
        }

        let snapshot = adapter.network_options_runtime_health_snapshot();
        assert_eq!(
            snapshot.media_policy,
            MpvNetworkMediaPolicyState::NetworkMediaUpdated,
            "the earlier transition result must commit before ownership is invalidated"
        );
        assert!(
            matches!(
                snapshot.hook_health,
                MpvNetworkOptionsHookHealth::Degraded(ref reason)
                    if reason.contains("ownership was replaced")
            ),
            "the later ownership loss must remain the final hook state: {snapshot:?}"
        );
        assert!(
            adapter
                .pending_network_options_hook_health_transitions
                .iter()
                .all(|event| !matches!(
                    event.value,
                    MpvNetworkOptionsHookHealthTransition::Recovered
                )),
            "an earlier transition result must not recover the hook after later ownership loss"
        );
        assert!(matches!(
            adapter
                .pending_network_media_policy_outcomes
                .front()
                .map(|event| &event.value),
            Some(MpvNetworkMediaPolicyOutcome::NetworkMediaUpdated)
        ));
        assert!(
            adapter
                .deferred_network_media_options_hook_transition_result
                .is_none()
        );
    }

    #[test]
    fn nonblocking_transition_result_precedes_later_rejected_command_response() {
        let mut adapter = MpvAdapter::with_test_transport_and_ipc_timeout(
            OrderedHookEventsTransport {
                responses: VecDeque::new(),
                emitted_events: false,
                emit_ownership_loss: false,
                response_error: "client not found",
            },
            Duration::from_millis(100),
        );
        adapter.legacy_syncplayintf_owner_id = "ordered-owner".to_owned();
        adapter.legacy_syncplayintf_attachment_id = "ordered-attachment".to_owned();
        adapter.network_media_options_generation = 7;
        adapter.network_media_options_hook_enabled = true;
        adapter.network_media_options_hook_loaded = true;
        adapter.network_media_options_hook_instance_id = Some("ordered-hook".to_owned());
        adapter.network_media_options_hook_configured_generation = Some(7);
        adapter.network_media_options_hook_last_accepted_load_sequence = Some(1);
        adapter.set_network_options_hook_health(MpvNetworkOptionsHookHealth::Ready);
        adapter
            .set_network_media_policy_state(MpvNetworkMediaPolicyState::AwaitingAuthoritativeLoad);
        adapter.network_media_options_hook_last_heartbeat_at =
            Some(Instant::now() - NETWORK_OPTIONS_HOOK_HEARTBEAT_INTERVAL);

        let observation_deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < observation_deadline {
            PlayerAdapter::maintain_runtime_leases_nonblocking(&mut adapter);
            let snapshot = adapter.network_options_runtime_health_snapshot();
            if matches!(
                snapshot.hook_health,
                MpvNetworkOptionsHookHealth::Degraded(_)
            ) && snapshot.media_policy == MpvNetworkMediaPolicyState::NetworkMediaUpdated
            {
                break;
            }
            std::thread::yield_now();
        }

        let snapshot = adapter.network_options_runtime_health_snapshot();
        assert_eq!(
            snapshot.media_policy,
            MpvNetworkMediaPolicyState::NetworkMediaUpdated,
            "the earlier transition result must remain the authoritative media-policy result"
        );
        assert!(
            matches!(
                snapshot.hook_health,
                MpvNetworkOptionsHookHealth::Degraded(ref reason)
                    if reason.contains("client not found")
            ),
            "the later rejected response must remain the final hook state: {snapshot:?}"
        );
        assert!(matches!(
            adapter
                .pending_network_media_policy_outcomes
                .front()
                .map(|event| &event.value),
            Some(MpvNetworkMediaPolicyOutcome::NetworkMediaUpdated)
        ));
    }
}
