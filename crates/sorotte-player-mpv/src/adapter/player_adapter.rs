use super::*;
use sorotte_player_api::{
    PlayerAdapter, PlayerCapabilities, PlayerCommand, PlayerCommandId, PlayerCommandProgress,
};

const NETWORK_OPTIONS_HEARTBEAT_COMMAND_TOKEN: u64 = 1;
const NETWORK_OPTIONS_EVENT_POLL_COMMAND_TOKEN: u64 = 2;
const LEGACY_SYNCPLAYINTF_HEARTBEAT_COMMAND_TOKEN: u64 = 3;

impl MpvAdapter {
    fn is_nonblocking_runtime_lease_event(event: &Value) -> bool {
        crate::ipc::is_sorotte_control_event(event)
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
                        .value
                        .get("args")
                        .and_then(Value::as_array)
                        .and_then(|args| args.first())
                        .and_then(Value::as_str)
                        == Some(SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_TRANSITION_RESULT);
                    let previous_observed_at = self
                        .current_ipc_event_observed_at
                        .replace(self.observation_timestamp_for(event.received_at));
                    self.handle_client_message_event(&event.value);
                    self.current_ipc_event_observed_at = previous_observed_at;
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
                            command_id,
                            token: LEGACY_SYNCPLAYINTF_HEARTBEAT_COMMAND_TOKEN,
                        } if self.legacy_syncplayintf_pending_heartbeat_command_id
                            == Some(command_id) =>
                        {
                            self.legacy_syncplayintf_pending_heartbeat_command_id = None;
                            self.legacy_syncplayintf_last_heartbeat_at = Some(Instant::now());
                        }
                        crate::ipc::MpvIpcNonblockingCommandCompletion::Succeeded {
                            command_id,
                            token: NETWORK_OPTIONS_HEARTBEAT_COMMAND_TOKEN,
                        } => {
                            if let Some(pending) =
                                self.network_media_options_hook_pending_heartbeat.as_mut()
                                && pending.command_id == Some(command_id)
                            {
                                pending.sent_at = Some(Instant::now());
                            }
                        }
                        crate::ipc::MpvIpcNonblockingCommandCompletion::Succeeded {
                            command_id,
                            token: NETWORK_OPTIONS_EVENT_POLL_COMMAND_TOKEN,
                        } if self.network_media_options_hook_pending_event_poll_command_id
                            == Some(command_id) =>
                        {
                            self.network_media_options_hook_pending_event_poll_command_id = None;
                        }
                        crate::ipc::MpvIpcNonblockingCommandCompletion::Succeeded { .. } => {}
                        crate::ipc::MpvIpcNonblockingCommandCompletion::Failed {
                            command_id,
                            token,
                            message,
                        } if (token == NETWORK_OPTIONS_HEARTBEAT_COMMAND_TOKEN
                            && self
                                .network_media_options_hook_pending_heartbeat
                                .is_some_and(|pending| pending.command_id == Some(command_id)))
                            || (token == NETWORK_OPTIONS_EVENT_POLL_COMMAND_TOKEN
                                && self
                                    .network_media_options_hook_pending_event_poll_command_id
                                    == Some(command_id)) =>
                        {
                            self.invalidate_network_media_options_hook_delivery();
                            self.queue_network_media_options_hook_degraded(
                                PlayerError::OperationFailed(message),
                            );
                        }
                        crate::ipc::MpvIpcNonblockingCommandCompletion::Failed {
                            command_id,
                            token: LEGACY_SYNCPLAYINTF_HEARTBEAT_COMMAND_TOKEN,
                            message,
                        } if self.legacy_syncplayintf_pending_heartbeat_command_id
                            == Some(command_id) =>
                        {
                            self.legacy_syncplayintf_pending_heartbeat_command_id = None;
                            self.begin_sorotte_bridge_runtime_recovery(
                                SorotteBridgeFailureKind::IpcCommand,
                                format!("failed to renew Sorotte's mpv bridge lease: {message}"),
                                true,
                            );
                        }
                        crate::ipc::MpvIpcNonblockingCommandCompletion::Failed { .. } => {}
                    }
                }
                crate::ipc::MpvIpcNonblockingRuntimeItem::ControlQueueOverflow => {
                    let lifecycle_epoch = self.lifecycle_epoch();
                    self.apply_lifecycle_input(PlayerLifecycleInput::EventGapDetected {
                        attachment_epoch: lifecycle_epoch,
                    });
                    self.invalidate_network_media_options_hook_delivery();
                    self.set_network_media_policy_state(MpvNetworkMediaPolicyState::Unknown);
                    self.queue_network_media_options_hook_degraded(PlayerError::OperationFailed(
                        "Sorotte's mpv IPC control queue overflowed; hook state must be reacquired"
                            .to_owned(),
                    ));
                    self.legacy_syncplayintf_pending_heartbeat_command_id = None;
                    self.begin_sorotte_bridge_runtime_recovery(
                        SorotteBridgeFailureKind::IpcCommand,
                        "Sorotte's mpv IPC control queue overflowed; bridge state must be reacquired",
                        true,
                    );
                }
                crate::ipc::MpvIpcNonblockingRuntimeItem::OrdinaryQueueOverflow => {
                    let lifecycle_epoch = self.lifecycle_epoch();
                    self.apply_lifecycle_input(PlayerLifecycleInput::EventGapDetected {
                        attachment_epoch: lifecycle_epoch,
                    });
                    self.invalidate_network_media_options_hook_delivery();
                    self.set_network_media_policy_state(MpvNetworkMediaPolicyState::Unknown);
                    self.queue_network_media_options_hook_degraded(
                        PlayerError::OperationFailed(
                            "Sorotte's mpv ordinary event queue overflowed; media and hook state must be reacquired"
                                .to_owned(),
                        ),
                    );
                    self.legacy_syncplayintf_pending_heartbeat_command_id = None;
                    self.begin_sorotte_bridge_runtime_recovery(
                        SorotteBridgeFailureKind::IpcCommand,
                        "Sorotte's mpv ordinary event queue overflowed; bridge state must be reacquired",
                        true,
                    );
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

            if self
                .network_media_options_hook_pending_event_poll_command_id
                .is_some()
            {
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
            match poll_result {
                Some(Ok(Some(command_id))) => {
                    self.network_media_options_hook_pending_event_poll_command_id =
                        Some(command_id);
                }
                Some(Ok(None)) | None => {}
                Some(Err(error)) => {
                    self.invalidate_network_media_options_hook_delivery();
                    self.queue_network_media_options_hook_degraded(PlayerError::OperationFailed(
                        error,
                    ));
                }
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
            Some(Ok(Some(command_id))) => {
                self.next_network_media_options_hook_heartbeat_nonce = self
                    .next_network_media_options_hook_heartbeat_nonce
                    .wrapping_add(1)
                    .max(1);
                self.network_media_options_hook_pending_heartbeat =
                    Some(PendingNetworkOptionsHookHeartbeat {
                        nonce,
                        command_id: Some(command_id),
                        sent_at: None,
                    });
            }
            Some(Ok(None)) => {}
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
            self.legacy_syncplayintf_pending_heartbeat_command_id = None;
            return;
        }
        if self
            .legacy_syncplayintf_pending_heartbeat_command_id
            .is_some()
        {
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
            Some(Ok(Some(command_id))) => {
                self.legacy_syncplayintf_pending_heartbeat_command_id = Some(command_id);
            }
            Some(Ok(None)) => {}
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
                self.interrupted_network_stream_recovery = None;
                self.network_cache_stall = None;
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
            TrackedCommandSupersession::Load => {
                debug_assert!(
                    self.player_lifecycle
                        .load_attempts
                        .values()
                        .any(|attempt| attempt.command_id == Some(command_id))
                        || self
                            .unacknowledged_terminal_command_progress
                            .contains_key(&command_id),
                    "an accepted tracked load must retain either its transition or terminal result"
                );
                self.supersede_tracked_commands(Some(command_id), |kind| {
                    kind.is_load_seek_or_play()
                });
            }
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
        let baseline_playlist_entry_ids = self.capture_authoritative_playlist_baseline();
        let generation = self.allocate_media_generation();
        self.interrupted_network_stream_recovery = None;
        self.network_cache_stall = None;
        let lifecycle_command_id = self.tracked_load_command_id_for_generation(generation);
        let lifecycle_attempt_id = self.submit_lifecycle_load(
            lifecycle_command_id,
            generation,
            path,
            baseline_playlist_entry_ids,
        );
        let lifecycle_epoch = self.lifecycle_epoch();
        let previous_phase = self.transport_phase;
        self.pending_load_request = Some(path.to_owned());
        self.pending_load_generation = Some(generation);
        self.network_media_options_embedded_load = None;
        self.transport_phase = PlayerTransportPhase::Loading;
        let loading_update = self
            .transport_update_for(generation)
            .with_phase(PlayerTransportPhase::Loading);
        self.queue_transport_telemetry_update_for_attempt(
            loading_update,
            Some(lifecycle_attempt_id),
        );

        let load_result =
            if uses_network_media_options(path) && !self.network_media_options.is_empty() {
                self.network_media_options_embedded_load = Some(EmbeddedNetworkMediaOptions {
                    media_generation: generation,
                    requested_target: path.to_owned(),
                });
                self.send_network_media_loadfile(path)
            } else {
                self.send_ipc_command_if_attached_without_draining_events(json!([
                    MPV_COMMAND_LOADFILE,
                    path,
                    MPV_LOADFILE_REPLACE
                ]))
            };
        if let Err(error) = load_result {
            self.apply_lifecycle_input(PlayerLifecycleInput::LoadAttemptRejected {
                attachment_epoch: lifecycle_epoch,
                attempt_id: lifecycle_attempt_id,
                failure: PlayerCommandFailureKind::Unknown,
            });
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
            self.queue_transport_telemetry_update_for_attempt(
                failure_update,
                Some(lifecycle_attempt_id),
            );
            return Err(error);
        }

        self.apply_lifecycle_input(PlayerLifecycleInput::LoadAttemptAccepted {
            attachment_epoch: lifecycle_epoch,
            attempt_id: lifecycle_attempt_id,
        });
        if let Some(command_id) = lifecycle_command_id {
            // The loadfile response has been accepted and its authoritative playlist identity is
            // bound. Publish that acceptance before superseding B, but retire B before reducing
            // lifecycle events buffered while C's command response was in flight.
            self.accept_tracked_command(command_id);
        }
        self.supersede_tracked_commands(lifecycle_command_id, |kind| kind.is_load_seek_or_play());
        if self.ipc_client.is_some() {
            // Acceptance must be recorded before any buffered start/file-loaded
            // event can be reduced.
            #[cfg(not(test))]
            self.reconcile_lifecycle_from_authority();
            self.drain_ipc_events_if_attached();
            // A fast mpv load can deliver start-file/file-loaded before the
            // loadfile command reply. Do not erase those observations after
            // the command returns.
            if self.pending_load_generation == Some(generation) {
                self.current_path = Some(path.to_owned());
                self.pending_local_file_update = None;
                self.pending_local_file_generation = None;
                self.pending_local_file_observed_at = None;
                self.observed_state.path = None;
                self.observed_state.duration_seconds = None;
                self.observed_state.size_bytes = None;
                self.paused_for_cache = false;
                self.cache_buffering_percent = None;
                self.observed_state.paused_for_cache = None;
                self.observed_state.cache_buffering_percent = None;
            }
        } else {
            let simulated_entry_id = i64::try_from(lifecycle_attempt_id.get()).unwrap_or(i64::MAX);
            self.apply_lifecycle_input(PlayerLifecycleInput::PlaylistSnapshot {
                attachment_epoch: lifecycle_epoch,
                entries: vec![AuthoritativePlaylistEntry::new(
                    simulated_entry_id,
                    Some(path.to_owned()),
                    true,
                )],
                current_path: Some(path.to_owned()),
            });
            self.apply_lifecycle_input(PlayerLifecycleInput::FileLoaded {
                attachment_epoch: lifecycle_epoch,
                playlist_entry_id: Some(simulated_entry_id),
                loaded_target: Some(path.to_owned()),
            });
            self.active_media_generation = Some(generation);
            self.pending_load_generation = None;
            self.pending_load_request = None;
            self.active_file_loaded = true;
            self.active_generation_has_restarted = !self.paused;
            self.current_path = Some(path.to_owned());
            self.queue_local_file_update(Self::local_file_update_for_path(path));
            self.queue_media_load_outcome(PlayerMediaLoadOutcome::success(
                path,
                Some(path.to_owned()),
            ));
            let phase = if self.paused {
                PlayerTransportPhase::ReadyPaused
            } else {
                PlayerTransportPhase::Playing
            };
            self.set_transport_phase(phase);
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
        self.interrupted_network_stream_recovery = None;
        self.network_cache_stall = None;
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
        self.take_local_file_observation()
            .map(|observation| observation.update)
    }

    fn take_local_file_observation(&mut self) -> Option<PlayerLocalFileObservation> {
        self.maintain_runtime_integrations();
        self.poll_ipc_local_file_update_if_attached();
        let update = self.pending_local_file_update.take()?;
        let media_generation = self.pending_local_file_generation.take();
        let observed_at = self
            .pending_local_file_observed_at
            .take()
            .map(|observed_at| {
                PlayerObservationTimestamp::from_adapter_observation(
                    observed_at.elapsed_since_adapter_start(),
                    self.observation_clock_origin.elapsed(),
                )
            });
        Some(PlayerLocalFileObservation::new(
            update,
            media_generation,
            observed_at,
        ))
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
        let mut update = self.pending_transport_telemetry_updates.pop_front()?;
        if let Some(observed_at) = update.observed_at {
            update.observed_at = Some(PlayerObservationTimestamp::from_adapter_observation(
                observed_at.elapsed_since_adapter_start(),
                self.observation_clock_origin.elapsed(),
            ));
        }
        Some(update)
    }

    fn take_cache_telemetry_update(&mut self) -> Option<PlayerCacheTelemetryUpdate> {
        self.maintain_runtime_integrations();
        self.ensure_transport_observers_registered_if_attached();
        self.drain_ipc_events_if_attached();
        self.observe_unhealthy_ipc_transport();
        let mut update = self.pending_cache_telemetry_updates.pop_front()?;
        if let Some(observed_at) = update.observed_at {
            update.observed_at = Some(PlayerObservationTimestamp::from_adapter_observation(
                observed_at.elapsed_since_adapter_start(),
                self.observation_clock_origin.elapsed(),
            ));
        }
        Some(update)
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
        self.take_media_load_observation()
            .map(|observation| observation.outcome)
    }

    fn take_media_load_observation(&mut self) -> Option<PlayerMediaLoadObservation> {
        self.maintain_runtime_integrations();
        self.ensure_observers_registered_if_attached();
        self.drain_ipc_events_if_attached();
        let mut observation = self.pending_media_load_outcomes.pop_front()?;
        observation.observed_at = observation.observed_at.map(|observed_at| {
            PlayerObservationTimestamp::from_adapter_observation(
                observed_at.elapsed_since_adapter_start(),
                self.observation_clock_origin.elapsed(),
            )
        });
        Some(observation)
    }

    fn take_ordered_event_batch(&mut self) -> Option<PlayerObservationBatch> {
        // A later pump without a consumer reacquisition request acknowledges the previously
        // returned semantic terminals. Keep them until this boundary so a rejected batch can be
        // reconstructed exactly, independent of the smaller legacy progress queue.
        self.acknowledge_last_delivered_ordered_semantic_outcomes();
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
        if self.pending_playback_telemetry_update.is_none() {
            self.poll_paused_position_telemetry_if_attached();
        }
        self.poll_ipc_local_file_update_if_attached();

        let (dropped_events_through, authoritative_snapshot) = if self
            .ordered_player_event_reacquisition_required
        {
            let dropped_events_through =
                PlayerEventSequence::new(self.next_ordered_player_event_sequence - 1);
            let interrupted_command_progress = self.authoritative_reacquisition_command_progress();
            let interrupted_media_load_outcomes = self
                .unacknowledged_media_load_outcomes
                .iter()
                .cloned()
                .collect::<Vec<_>>();
            let authoritative_generation = self
                .observation_media_generation()
                .or_else(|| {
                    interrupted_media_load_outcomes
                        .iter()
                        .rev()
                        .find_map(|observation| observation.media_generation)
                })
                .or_else(|| {
                    interrupted_command_progress
                        .iter()
                        .rev()
                        .find_map(|progress| progress.media_generation)
                });
            let authoritative_local_file = self
                .pending_local_file_update
                .clone()
                .or_else(|| self.last_polled_local_file_update.clone())
                .filter(|update| {
                    self.active_file_loaded
                        && self.observed_state.path.as_deref().is_some_and(|path| {
                            Self::local_file_update_matches_request(update, path)
                        })
                });
            self.pending_ordered_player_events.clear();
            self.pending_command_progress_updates.clear();
            self.pending_transport_telemetry_updates.clear();
            self.pending_media_load_outcomes.clear();
            self.pending_local_file_update = None;
            self.pending_local_file_generation = None;
            self.pending_local_file_observed_at = None;
            self.pending_playback_telemetry_update = None;
            let authoritative_snapshot = self.authoritative_ordered_player_snapshot(
                authoritative_local_file,
                interrupted_command_progress,
                interrupted_media_load_outcomes,
                authoritative_generation,
            );
            (Some(dropped_events_through), Some(authoritative_snapshot))
        } else {
            (None, None)
        };
        self.ordered_player_event_reacquisition_required = false;
        self.ordered_player_event_reacquisition_requested_by_consumer = false;
        let delivery_reference = self.observation_clock_origin.elapsed();
        let mut ordered_events: Vec<_> =
            if let Some(authoritative_snapshot) = authoritative_snapshot {
                authoritative_snapshot
                    .into_iter()
                    .map(|kind| {
                        let sequence =
                            PlayerEventSequence::new(self.next_ordered_player_event_sequence);
                        self.next_ordered_player_event_sequence = self
                            .next_ordered_player_event_sequence
                            .checked_add(1)
                            .expect("mpv ordered player event sequence exhausted");
                        PlayerOrderedEvent::new(sequence, kind)
                    })
                    .collect()
            } else {
                self.pending_ordered_player_events.drain(..).collect()
            };
        for event in &mut ordered_events {
            let retag = |observed_at: PlayerObservationTimestamp| {
                PlayerObservationTimestamp::from_adapter_observation(
                    observed_at.elapsed_since_adapter_start(),
                    delivery_reference,
                )
            };
            match &mut event.kind {
                PlayerOrderedEventKind::CommandProgress(progress) => {
                    progress.observed_at = progress.observed_at.map(retag);
                }
                PlayerOrderedEventKind::LocalFile(observation) => {
                    observation.observed_at = observation.observed_at.map(retag);
                }
                PlayerOrderedEventKind::MediaLoad(observation) => {
                    observation.observed_at = observation.observed_at.map(retag);
                }
                PlayerOrderedEventKind::Transport(update) => {
                    update.observed_at = update.observed_at.map(retag);
                }
            }
        }
        self.last_delivered_ordered_command_progress = ordered_events
            .iter()
            .filter_map(|event| match &event.kind {
                PlayerOrderedEventKind::CommandProgress(progress) => Some(*progress),
                _ => None,
            })
            .collect();
        self.last_delivered_ordered_media_load_outcomes = ordered_events
            .iter()
            .filter_map(|event| match &event.kind {
                PlayerOrderedEventKind::MediaLoad(observation) => Some(observation.clone()),
                _ => None,
            })
            .collect();

        self.pending_command_progress_updates.clear();
        self.pending_transport_telemetry_updates.clear();
        self.pending_media_load_outcomes.clear();
        self.pending_local_file_update = None;
        self.pending_local_file_generation = None;
        self.pending_local_file_observed_at = None;

        Some(PlayerObservationBatch {
            dropped_events_through,
            ordered_events,
            legacy_playback_telemetry: self.pending_playback_telemetry_update.take(),
        })
    }

    fn request_ordered_event_reacquisition(&mut self) {
        self.ordered_player_event_reacquisition_requested_by_consumer = true;
        self.ordered_player_event_reacquisition_required = true;
    }

    fn take_player_event_batch(&mut self) -> Option<sorotte_player_api::PlayerEventBatch> {
        self.maintain_runtime_integrations();
        self.player_lifecycle.peek_event_batch()
    }

    fn player_event_delivery_mode(&self) -> sorotte_player_api::PlayerEventDeliveryMode {
        sorotte_player_api::PlayerEventDeliveryMode::OrderedAcknowledgedBatches
    }

    fn acknowledge_player_event_batch(
        &mut self,
        token: sorotte_player_api::PlayerEventAcknowledgementToken,
    ) -> Result<(), PlayerError> {
        if self.player_lifecycle.acknowledge_event_batch(token) {
            Ok(())
        } else {
            Err(PlayerError::OperationFailed(
                "player event acknowledgement did not match the in-flight batch".to_owned(),
            ))
        }
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
    use sorotte_player_api::PlayerCommandProgressState;
    use std::{
        collections::VecDeque,
        io,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

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

    #[derive(Clone, Copy)]
    enum HeartbeatEventOrdering {
        PropertyThenAck,
        AckThenProperty,
        StartPathThenAck,
    }

    struct RuntimeLeaseControlLaneTransport {
        responses: VecDeque<String>,
        ordering: HeartbeatEventOrdering,
        network_heartbeats: Arc<AtomicUsize>,
        legacy_heartbeats: Arc<AtomicUsize>,
        ordinary_sequence: usize,
    }

    #[test]
    fn ordered_event_batch_is_atomic_and_preserves_adapter_ingress_order() {
        let mut adapter = MpvAdapter::simulated();
        let generation = PlayerMediaGeneration::new(4);
        adapter.active_media_generation = Some(generation);
        adapter.queue_local_file_update(LocalFileUpdate::new("ordered.mkv"));
        let transport = adapter
            .transport_update_for(generation)
            .with_position_seconds(12.0);
        adapter.queue_transport_telemetry_update(transport);
        adapter.queue_media_load_outcome(PlayerMediaLoadOutcome::success(
            "ordered.mkv",
            Some("ordered.mkv".to_owned()),
        ));
        let observed_at = Some(adapter.observation_timestamp());
        adapter.queue_command_progress(PlayerCommandProgress::finished(
            PlayerCommandId::new(11),
            Some(generation),
            observed_at,
            Some(12.0),
            PlayerCommandResult::Completed,
        ));

        let batch = adapter
            .take_ordered_event_batch()
            .expect("mpv supports ordered event batches");
        assert_eq!(batch.ordered_events.len(), 4);
        assert!(
            batch
                .ordered_events
                .windows(2)
                .all(|events| events[0].sequence < events[1].sequence)
        );
        assert!(matches!(
            batch.ordered_events[0].kind,
            PlayerOrderedEventKind::LocalFile(_)
        ));
        assert!(matches!(
            batch.ordered_events[1].kind,
            PlayerOrderedEventKind::Transport(_)
        ));
        assert!(matches!(
            batch.ordered_events[2].kind,
            PlayerOrderedEventKind::MediaLoad(_)
        ));
        assert!(matches!(
            batch.ordered_events[3].kind,
            PlayerOrderedEventKind::CommandProgress(_)
        ));
        assert!(adapter.pending_command_progress_updates.is_empty());
        assert!(adapter.pending_transport_telemetry_updates.is_empty());
        assert!(adapter.pending_media_load_outcomes.is_empty());
        assert!(adapter.pending_local_file_update.is_none());
    }

    #[test]
    fn ordered_event_overflow_rebases_file_command_and_terminal_transport_to_snapshot() {
        let mut adapter = MpvAdapter::simulated();
        let generation = PlayerMediaGeneration::new(4);
        adapter.active_media_generation = Some(generation);
        adapter.active_file_loaded = true;
        adapter.transport_phase = PlayerTransportPhase::Playing;
        adapter.observed_state.path = Some("current.mkv".to_owned());
        adapter.observed_state.position_seconds = Some(32.0);
        adapter.observed_state.playback_rate = Some(1.0);
        adapter.observed_state.logical_pause = Some(false);
        adapter.observed_state.paused_for_cache = Some(false);
        adapter.observed_state.eof_reached = Some(false);
        adapter.queue_local_file_update(LocalFileUpdate::new("current.mkv"));
        adapter.queue_command_progress(PlayerCommandProgress::finished(
            PlayerCommandId::new(11),
            Some(generation),
            Some(adapter.observation_timestamp()),
            Some(32.0),
            PlayerCommandResult::Completed,
        ));
        let mut ended = adapter
            .transport_update_for(generation)
            .with_phase(PlayerTransportPhase::Ended);
        ended.eof_reached = Some(true);
        adapter.queue_transport_telemetry_update(ended);
        for position in 0..MAX_PENDING_ORDERED_PLAYER_EVENTS {
            let update = adapter
                .transport_update_for(generation)
                .with_position_seconds(position as f64);
            adapter.queue_transport_telemetry_update(update);
        }

        let batch = adapter
            .take_ordered_event_batch()
            .expect("mpv supports ordered event batches");
        let dropped_events_through = batch
            .dropped_events_through
            .expect("overflow must be explicit");
        assert_eq!(
            batch
                .ordered_events
                .first()
                .map(|event| event.sequence.get()),
            Some(dropped_events_through.get() + 1)
        );
        assert_eq!(batch.ordered_events.len(), 3);
        assert!(batch.ordered_events.windows(2).all(|events| {
            events[0]
                .sequence
                .get()
                .checked_add(1)
                .is_some_and(|expected| events[1].sequence.get() == expected)
        }));
        assert!(matches!(
            &batch.ordered_events[0].kind,
            PlayerOrderedEventKind::CommandProgress(progress)
                if progress.command_id == PlayerCommandId::new(11)
                    && progress.state
                        == PlayerCommandProgressState::Finished(PlayerCommandResult::Completed)
        ));
        assert!(matches!(
            &batch.ordered_events[1].kind,
            PlayerOrderedEventKind::LocalFile(observation)
                if observation.update.name == "current.mkv"
        ));
        assert!(matches!(
            &batch.ordered_events[2].kind,
            PlayerOrderedEventKind::Transport(update)
                if update.media_generation == Some(generation)
                    && update.phase == Some(PlayerTransportPhase::Playing)
                    && update.position_seconds == Some(32.0)
                    && update.eof_reached == Some(false)
                    && update.seekable_ranges == Some(Vec::new())
        ));
        assert!(batch.ordered_events.iter().all(|event| {
            !matches!(
                &event.kind,
                PlayerOrderedEventKind::Transport(PlayerTransportTelemetryUpdate {
                    phase: Some(PlayerTransportPhase::Ended | PlayerTransportPhase::Failed),
                    ..
                })
            )
        }));
        assert_eq!(batch.legacy_playback_telemetry, None);
    }

    #[test]
    fn authoritative_reacquisition_can_replay_more_events_than_the_ingress_queue_capacity() {
        let mut adapter = MpvAdapter::simulated();
        let generation = PlayerMediaGeneration::new(4);
        adapter.active_media_generation = Some(generation);
        adapter.active_file_loaded = true;
        adapter.transport_phase = PlayerTransportPhase::Playing;
        adapter.observed_state.position_seconds = Some(32.0);
        adapter.observed_state.eof_reached = Some(false);

        let terminal_count = MAX_PENDING_ORDERED_PLAYER_EVENTS + 44;
        for id in 1..=terminal_count {
            adapter.queue_command_progress(PlayerCommandProgress::finished(
                PlayerCommandId::new(id as u64),
                Some(generation),
                Some(adapter.observation_timestamp()),
                Some(32.0),
                PlayerCommandResult::Completed,
            ));
        }

        let batch = adapter
            .take_ordered_event_batch()
            .expect("mpv supports ordered event batches");
        let dropped_events_through = batch
            .dropped_events_through
            .expect("overflow must request authoritative reacquisition");
        assert_eq!(batch.ordered_events.len(), terminal_count + 1);
        assert_eq!(
            batch
                .ordered_events
                .first()
                .map(|event| event.sequence.get()),
            Some(dropped_events_through.get() + 1)
        );
        assert!(batch.ordered_events.windows(2).all(|events| {
            events[0]
                .sequence
                .get()
                .checked_add(1)
                .is_some_and(|expected| events[1].sequence.get() == expected)
        }));
        assert_eq!(
            batch
                .ordered_events
                .iter()
                .filter(|event| matches!(event.kind, PlayerOrderedEventKind::CommandProgress(_)))
                .count(),
            terminal_count
        );
        assert!(matches!(
            batch.ordered_events.last().map(|event| &event.kind),
            Some(PlayerOrderedEventKind::Transport(update))
                if update.media_generation == Some(generation)
                    && update.phase == Some(PlayerTransportPhase::Playing)
        ));
        assert!(!adapter.ordered_player_event_reacquisition_required);

        let acknowledged = adapter
            .take_ordered_event_batch()
            .expect("mpv supports ordered event batches");
        assert_eq!(acknowledged.dropped_events_through, None);
        assert!(adapter.unacknowledged_terminal_command_progress.is_empty());
    }

    #[test]
    fn ordered_event_reacquisition_replays_the_latest_empty_seekable_range_snapshot() {
        let mut adapter = MpvAdapter::simulated();
        let generation = PlayerMediaGeneration::new(4);
        let attachment_epoch = adapter.lifecycle_epoch();
        adapter.apply_lifecycle_input(PlayerLifecycleInput::ExternalLoadObserved {
            attachment_epoch,
            media_generation: generation,
            playlist_entry_id: 4,
            observed_target: "current.mkv".to_owned(),
            file_loaded: true,
        });
        adapter.active_media_generation = Some(generation);
        adapter.active_file_loaded = true;
        adapter.transport_phase = PlayerTransportPhase::Playing;
        adapter.observed_state.path = Some("current.mkv".to_owned());
        adapter.cache_state_telemetry_update(&json!({
            "seekable-ranges": [{ "start": 10.0, "end": 20.0 }],
        }));
        adapter.cache_state_telemetry_update(&json!({
            "seekable-ranges": [],
        }));
        for position in 0..=MAX_PENDING_ORDERED_PLAYER_EVENTS {
            adapter.queue_transport_telemetry_update(
                adapter
                    .transport_update_for(generation)
                    .with_position_seconds(position as f64),
            );
        }

        let batch = adapter
            .take_ordered_event_batch()
            .expect("mpv supports ordered event batches");

        assert!(batch.dropped_events_through.is_some());
        assert!(batch.ordered_events.iter().any(|event| matches!(
            &event.kind,
            PlayerOrderedEventKind::Transport(update)
                if update.media_generation == Some(generation)
                    && update.seekable_ranges == Some(Vec::new())
                    && update.known_live_seekable_window.is_none()
        )));
    }

    #[test]
    fn ordered_event_overflow_during_start_file_does_not_reacquire_previous_file_identity() {
        let mut adapter = MpvAdapter::simulated();
        let old_generation = PlayerMediaGeneration::new(4);
        adapter.active_media_generation = Some(old_generation);
        adapter.active_file_loaded = true;
        adapter.current_path = Some("old.mkv".to_owned());
        adapter.observed_state.path = Some("old.mkv".to_owned());
        adapter.last_polled_local_file_update =
            Some(LocalFileUpdate::new("old.mkv").with_path("old.mkv"));
        adapter.handle_start_file_event(&json!({ "playlist_entry_id": 9 }));
        let new_generation = adapter
            .active_media_generation
            .expect("start-file generation");
        assert_ne!(new_generation, old_generation);
        for _ in 0..MAX_PENDING_ORDERED_PLAYER_EVENTS {
            let update = adapter.transport_update_for(new_generation);
            adapter.queue_transport_telemetry_update(update);
        }

        let batch = adapter
            .take_ordered_event_batch()
            .expect("mpv supports ordered event batches");
        assert!(batch.dropped_events_through.is_some());
        assert!(
            batch
                .ordered_events
                .iter()
                .all(|event| !matches!(event.kind, PlayerOrderedEventKind::LocalFile(_)))
        );
        assert!(batch.ordered_events.iter().any(|event| matches!(
            &event.kind,
            PlayerOrderedEventKind::Transport(update)
                if update.media_generation == Some(new_generation)
                    && update.phase == Some(PlayerTransportPhase::Loading)
        )));
    }

    impl RuntimeLeaseControlLaneTransport {
        fn push(&mut self, value: Value) {
            self.responses.push_back(value.to_string() + "\n");
        }

        fn ordinary_property_event(&mut self) -> Value {
            self.ordinary_sequence += 1;
            json!({
                "event": "property-change",
                "name": "time-pos",
                "data": self.ordinary_sequence,
            })
        }

        fn heartbeat_ack(payload: &Value) -> Value {
            let payload = json!({
                "protocol": SOROTTE_NETWORK_OPTIONS_PROTOCOL,
                "ownerId": "test-owner",
                "attachmentId": "test-attachment",
                "hookInstanceId": "lease-hook",
                "configurationGeneration": 1,
                "status": "renewed",
                "heartbeatNonce": payload.get("heartbeatNonce").cloned().unwrap_or(Value::Null),
            });
            json!({
                "event": MPV_EVENT_CLIENT_MESSAGE,
                "args": [
                    SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_HEARTBEAT,
                    payload.to_string(),
                ],
            })
        }
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

    impl MpvJsonIpcTransport for RuntimeLeaseControlLaneTransport {
        fn send_line_until(&mut self, line: &str, _deadline: Instant) -> io::Result<()> {
            let request: Value = serde_json::from_str(line.trim())
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            let request_id = request.get("request_id").cloned().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "test request omitted request_id",
                )
            })?;
            let command = request
                .get("command")
                .and_then(Value::as_array)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid command"))?;
            let message = command.get(2).and_then(Value::as_str);
            if message == Some(SOROTTE_NETWORK_OPTIONS_HEARTBEAT_MESSAGE) {
                self.network_heartbeats.fetch_add(1, Ordering::Relaxed);
                let payload = command
                    .get(3)
                    .and_then(Value::as_str)
                    .and_then(|payload| serde_json::from_str::<Value>(payload).ok())
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidData, "invalid heartbeat payload")
                    })?;
                let ack = Self::heartbeat_ack(&payload);
                match self.ordering {
                    HeartbeatEventOrdering::PropertyThenAck => {
                        let event = self.ordinary_property_event();
                        self.push(event);
                        self.push(ack);
                    }
                    HeartbeatEventOrdering::AckThenProperty => {
                        self.push(ack);
                        let event = self.ordinary_property_event();
                        self.push(event);
                    }
                    HeartbeatEventOrdering::StartPathThenAck => {
                        self.push(json!({"event": "start-file", "playlist_entry_id": 71}));
                        self.push(json!({
                            "event": "property-change",
                            "name": "path",
                            "data": "https://media.example.test/live.m3u8",
                        }));
                        self.push(ack);
                    }
                }
            } else if message == Some(LEGACY_SYNCPLAYINTF_HEARTBEAT_MESSAGE) {
                self.legacy_heartbeats.fetch_add(1, Ordering::Relaxed);
                let event = self.ordinary_property_event();
                self.push(event);
            } else {
                let event = self.ordinary_property_event();
                self.push(event);
            }
            self.push(json!({"request_id": request_id, "error": "success", "data": false}));
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

    fn ready_adapter_with_control_lane_transport(
        ordering: HeartbeatEventOrdering,
    ) -> (MpvAdapter, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let network_heartbeats = Arc::new(AtomicUsize::new(0));
        let legacy_heartbeats = Arc::new(AtomicUsize::new(0));
        let mut adapter = MpvAdapter::with_test_transport_and_ipc_timeout(
            RuntimeLeaseControlLaneTransport {
                responses: VecDeque::new(),
                ordering,
                network_heartbeats: Arc::clone(&network_heartbeats),
                legacy_heartbeats: Arc::clone(&legacy_heartbeats),
                ordinary_sequence: 0,
            },
            Duration::from_millis(250),
        );
        adapter.enable_test_legacy_chat_input();
        adapter.sorotte_bridge_health = SorotteBridgeHealth::Ready;
        adapter.legacy_syncplayintf_last_heartbeat_at =
            Some(Instant::now() - LEGACY_SYNCPLAYINTF_HEARTBEAT_INTERVAL);
        adapter.network_media_options_hook_enabled = true;
        adapter.network_media_options_hook_loaded = true;
        adapter.network_media_options_hook_instance_id = Some("lease-hook".to_owned());
        adapter.network_media_options_hook_configured_generation =
            Some(adapter.network_media_options_generation);
        adapter.set_network_options_hook_health(MpvNetworkOptionsHookHealth::Ready);
        adapter.network_media_options_hook_last_heartbeat_at =
            Some(Instant::now() - NETWORK_OPTIONS_HOOK_HEARTBEAT_INTERVAL);
        adapter
            .pending_network_options_hook_health_transitions
            .clear();
        (adapter, network_heartbeats, legacy_heartbeats)
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

    fn assert_ordinary_event_order_does_not_block_hook_ack(
        ordering: HeartbeatEventOrdering,
        expected_event_names: &[&str],
    ) {
        let (mut adapter, network_heartbeats, _legacy_heartbeats) =
            ready_adapter_with_control_lane_transport(ordering);
        adapter.legacy_syncplay_ui_settings.chat_input_enabled = false;
        adapter.sorotte_bridge_health = SorotteBridgeHealth::Disabled;
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline
            && (network_heartbeats.load(Ordering::Relaxed) == 0
                || adapter
                    .network_media_options_hook_pending_heartbeat
                    .is_some())
        {
            PlayerAdapter::maintain_runtime_leases_nonblocking(&mut adapter);
            std::thread::yield_now();
        }

        assert!(network_heartbeats.load(Ordering::Relaxed) >= 1);
        assert!(
            adapter
                .network_media_options_hook_pending_heartbeat
                .is_none()
        );
        assert_eq!(
            adapter.network_media_options_hook_health,
            MpvNetworkOptionsHookHealth::Ready
        );
        assert!(
            adapter
                .pending_network_options_hook_health_transitions
                .iter()
                .all(|transition| !matches!(
                    transition.value,
                    MpvNetworkOptionsHookHealthTransition::Degraded(_)
                ))
        );

        let ordinary_events = adapter
            .ipc_client
            .as_mut()
            .expect("test adapter should remain attached")
            .take_pending_events();
        let event_names = ordinary_events
            .iter()
            .filter_map(|event| event.get("event").and_then(Value::as_str))
            .collect::<Vec<_>>();
        assert_eq!(
            event_names, expected_event_names,
            "ordinary event order must remain unchanged for the full pump"
        );
    }

    #[test]
    fn property_before_heartbeat_ack_does_not_block_nonblocking_lease_maintenance() {
        assert_ordinary_event_order_does_not_block_hook_ack(
            HeartbeatEventOrdering::PropertyThenAck,
            &["property-change"],
        );
    }

    #[test]
    fn property_between_heartbeat_ack_and_response_remains_full_pump_visible() {
        assert_ordinary_event_order_does_not_block_hook_ack(
            HeartbeatEventOrdering::AckThenProperty,
            &["property-change"],
        );
    }

    #[test]
    fn start_and_path_before_heartbeat_ack_do_not_block_the_control_lane() {
        assert_ordinary_event_order_does_not_block_hook_ack(
            HeartbeatEventOrdering::StartPathThenAck,
            &["start-file", "property-change"],
        );
    }

    #[test]
    fn both_runtime_leases_renew_for_more_than_owner_lease_with_only_nonblocking_pumps() {
        let (mut adapter, network_heartbeats, legacy_heartbeats) =
            ready_adapter_with_control_lane_transport(HeartbeatEventOrdering::PropertyThenAck);
        let deadline = Instant::now() + Duration::from_millis(2_200);
        while Instant::now() < deadline {
            PlayerAdapter::maintain_runtime_leases_nonblocking(&mut adapter);
            std::thread::sleep(Duration::from_millis(5));
        }
        PlayerAdapter::maintain_runtime_leases_nonblocking(&mut adapter);

        assert!(
            network_heartbeats.load(Ordering::Relaxed) >= 3,
            "network hook should receive multiple acknowledged renewals"
        );
        assert!(
            legacy_heartbeats.load(Ordering::Relaxed) >= 3,
            "optional Chat/OSD bridge should renew alongside the core hook"
        );
        assert_eq!(
            adapter.network_media_options_hook_health,
            MpvNetworkOptionsHookHealth::Ready
        );
        assert!(matches!(
            adapter.sorotte_bridge_health,
            SorotteBridgeHealth::Ready
        ));
        assert!(
            adapter
                .pending_network_options_hook_health_transitions
                .iter()
                .all(|transition| !matches!(
                    transition.value,
                    MpvNetworkOptionsHookHealthTransition::Degraded(_)
                ))
        );

        let client = adapter
            .ipc_client
            .as_mut()
            .expect("test adapter should remain attached");
        let (ordinary_capacity, control_capacity) =
            MpvJsonIpcClient::test_runtime_queue_capacities();
        let (ordinary_count, control_count) = client.test_runtime_queue_sizes();
        assert!(ordinary_count > 0);
        assert!(ordinary_count <= ordinary_capacity);
        assert!(control_count <= control_capacity);
        let ordinary_events = client.take_pending_events();
        assert!(ordinary_events.iter().any(|event| {
            event.get("event").and_then(Value::as_str) == Some("property-change")
        }));
    }

    #[test]
    fn stale_heartbeat_poll_and_bridge_completions_cannot_mutate_successors() {
        let (mut adapter, _network_heartbeats, _legacy_heartbeats) =
            ready_adapter_with_control_lane_transport(HeartbeatEventOrdering::PropertyThenAck);
        adapter.network_media_options_hook_pending_heartbeat =
            Some(PendingNetworkOptionsHookHeartbeat {
                nonce: 1,
                command_id: Some(21),
                sent_at: Some(Instant::now()),
            });
        let h1_ack = json!({
            "protocol": SOROTTE_NETWORK_OPTIONS_PROTOCOL,
            "ownerId": "test-owner",
            "attachmentId": "test-attachment",
            "hookInstanceId": "lease-hook",
            "configurationGeneration": 1,
            "status": "renewed",
            "heartbeatNonce": 1,
        })
        .to_string();
        adapter.handle_network_options_hook_heartbeat(Some(&h1_ack));
        assert!(
            adapter
                .network_media_options_hook_pending_heartbeat
                .is_none(),
            "H1 acknowledgement must clear H1 before its delayed command completion"
        );

        adapter.network_media_options_hook_pending_heartbeat =
            Some(PendingNetworkOptionsHookHeartbeat {
                nonce: 2,
                command_id: Some(22),
                sent_at: None,
            });
        adapter.network_media_options_hook_pending_event_poll_command_id = Some(32);
        adapter.legacy_syncplayintf_pending_heartbeat_command_id = Some(42);
        adapter.legacy_syncplayintf_last_heartbeat_at = None;
        let client = adapter
            .ipc_client
            .as_mut()
            .expect("test adapter should remain attached");
        client.inject_test_nonblocking_completion(
            crate::ipc::MpvIpcNonblockingCommandCompletion::Succeeded {
                command_id: 21,
                token: NETWORK_OPTIONS_HEARTBEAT_COMMAND_TOKEN,
            },
        );
        client.inject_test_nonblocking_completion(
            crate::ipc::MpvIpcNonblockingCommandCompletion::Failed {
                command_id: 31,
                token: NETWORK_OPTIONS_EVENT_POLL_COMMAND_TOKEN,
                message: "stale poll failure".to_owned(),
            },
        );
        client.inject_test_nonblocking_completion(
            crate::ipc::MpvIpcNonblockingCommandCompletion::Succeeded {
                command_id: 41,
                token: LEGACY_SYNCPLAYINTF_HEARTBEAT_COMMAND_TOKEN,
            },
        );

        adapter.drain_runtime_lease_events_nonblocking();

        assert!(
            adapter
                .network_media_options_hook_pending_heartbeat
                .is_some_and(|pending| {
                    pending.nonce == 2
                        && pending.command_id == Some(22)
                        && pending.sent_at.is_none()
                })
        );
        assert_eq!(
            adapter.network_media_options_hook_pending_event_poll_command_id,
            Some(32)
        );
        assert_eq!(
            adapter.legacy_syncplayintf_pending_heartbeat_command_id,
            Some(42)
        );
        assert!(adapter.legacy_syncplayintf_last_heartbeat_at.is_none());
        assert_eq!(
            adapter.network_media_options_hook_health,
            MpvNetworkOptionsHookHealth::Ready
        );
        assert!(matches!(
            adapter.sorotte_bridge_health,
            SorotteBridgeHealth::Ready
        ));
    }

    #[test]
    fn synchronous_full_maintenance_consumes_overflow_fault_and_later_events() {
        let mut adapter = MpvAdapter::with_test_transport_and_ipc_timeout(
            RejectingHeartbeatTransport {
                responses: VecDeque::new(),
            },
            Duration::from_millis(100),
        );
        adapter.set_network_media_policy_state(MpvNetworkMediaPolicyState::NetworkMediaUpdated);
        let prior_revision = adapter.network_media_options_runtime_health_revision;
        let (ordinary_capacity, _) = MpvJsonIpcClient::test_runtime_queue_capacities();
        let client = adapter
            .ipc_client
            .as_mut()
            .expect("test adapter should remain attached");
        for playlist_entry_id in 0..=ordinary_capacity {
            client.inject_test_event(json!({
                "event": "start-file",
                "playlist_entry_id": playlist_entry_id,
            }));
        }

        adapter.maintain_runtime_integrations();

        let snapshot = adapter.network_options_runtime_health_snapshot();
        assert!(matches!(
            snapshot.hook_health,
            MpvNetworkOptionsHookHealth::Degraded(ref reason)
                if reason.contains("ordinary event queue overflowed")
        ));
        assert_eq!(snapshot.media_policy, MpvNetworkMediaPolicyState::Unknown);
        assert!(snapshot.revision > prior_revision);
        assert_eq!(
            adapter
                .ipc_client
                .as_ref()
                .expect("test adapter should remain attached")
                .test_runtime_queue_sizes(),
            (0, 0),
            "full maintenance must not leave the overflow sentinel or later events stuck"
        );
    }

    #[test]
    fn pending_position_and_rate_events_keep_their_individual_sample_clocks() {
        let generation = PlayerMediaGeneration::new(1);
        for position_then_speed in [true, false] {
            let mut adapter = MpvAdapter {
                active_media_generation: Some(generation),
                transport_phase: PlayerTransportPhase::Playing,
                ..MpvAdapter::default()
            };
            let mut position = PlayerTransportTelemetryUpdate::new(
                generation,
                PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(
                    if position_then_speed { 1 } else { 2 },
                )),
            )
            .with_position_seconds(10.0);
            position.phase = Some(PlayerTransportPhase::Playing);
            let mut speed = PlayerTransportTelemetryUpdate::new(
                generation,
                PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(
                    if position_then_speed { 2 } else { 1 },
                )),
            );
            speed.playback_rate = Some(4.0);

            if position_then_speed {
                adapter.queue_transport_telemetry_update(position);
                adapter.queue_transport_telemetry_update(speed);
            } else {
                adapter.queue_transport_telemetry_update(speed);
                adapter.queue_transport_telemetry_update(position);
            }

            assert_eq!(adapter.pending_transport_telemetry_updates.len(), 2);
            let clocks = adapter
                .pending_transport_telemetry_updates
                .iter()
                .map(|update| {
                    (
                        update.position_seconds,
                        update.playback_rate,
                        update
                            .observed_at
                            .expect("queued transport telemetry should have a clock")
                            .elapsed_since_adapter_start(),
                    )
                })
                .collect::<Vec<_>>();
            if position_then_speed {
                assert_eq!(
                    clocks,
                    vec![
                        (Some(10.0), None, Duration::from_secs(1)),
                        (None, Some(4.0), Duration::from_secs(2)),
                    ]
                );
            } else {
                assert_eq!(
                    clocks,
                    vec![
                        (None, Some(4.0), Duration::from_secs(1)),
                        (Some(10.0), None, Duration::from_secs(2)),
                    ]
                );
            }
        }
    }

    #[test]
    fn sparse_event_cannot_retimestamp_a_pending_position() {
        let generation = PlayerMediaGeneration::new(1);
        let mut adapter = MpvAdapter {
            active_media_generation: Some(generation),
            transport_phase: PlayerTransportPhase::Playing,
            ..MpvAdapter::default()
        };
        adapter.queue_transport_telemetry_update(
            PlayerTransportTelemetryUpdate::new(
                generation,
                PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(1)),
            )
            .with_position_seconds(10.0),
        );
        let mut sparse = PlayerTransportTelemetryUpdate::new(
            generation,
            PlayerObservationTimestamp::from_adapter_start(Duration::from_secs(2)),
        );
        sparse.logical_pause = Some(false);
        adapter.queue_transport_telemetry_update(sparse);

        assert_eq!(adapter.pending_transport_telemetry_updates.len(), 2);
        assert_eq!(
            adapter.pending_transport_telemetry_updates[0]
                .observed_at
                .expect("position should retain its own clock")
                .elapsed_since_adapter_start(),
            Duration::from_secs(1)
        );
    }

    #[test]
    fn delayed_ordinary_event_uses_ipc_ingress_time_not_drain_time() {
        let mut adapter = MpvAdapter::with_test_transport_and_ipc_timeout(
            RejectingHeartbeatTransport {
                responses: VecDeque::new(),
            },
            Duration::from_millis(100),
        );
        let origin = Instant::now() - Duration::from_secs(10);
        let received_at = origin + Duration::from_secs(1);
        adapter.observation_clock_origin = origin;
        let generation = PlayerMediaGeneration::new(1);
        adapter.active_media_generation = Some(generation);
        adapter.active_file_loaded = true;
        adapter.transport_phase = PlayerTransportPhase::Playing;
        adapter
            .ipc_client
            .as_mut()
            .expect("test adapter should remain attached")
            .inject_test_event_received_at(
                json!({
                    "event": MPV_EVENT_PROPERTY_CHANGE,
                    "name": MPV_PROPERTY_TIME_POS,
                    "data": 10.0,
                }),
                received_at,
            );

        assert!(adapter.drain_ipc_events_without_network_options_flush());
        let position = adapter
            .pending_transport_telemetry_updates
            .iter()
            .find(|update| update.position_seconds == Some(10.0))
            .expect("time-pos event should emit transport telemetry");
        assert_eq!(
            position
                .observed_at
                .expect("event-derived telemetry should be timestamped")
                .elapsed_since_adapter_start(),
            Duration::from_secs(1)
        );
    }

    #[test]
    fn telemetry_delivery_reference_includes_adapter_queue_dwell() {
        let mut adapter = MpvAdapter {
            observation_clock_origin: Instant::now() - Duration::from_secs(10),
            ..MpvAdapter::default()
        };
        let generation = PlayerMediaGeneration::new(1);
        adapter
            .pending_transport_telemetry_updates
            .push_back(PlayerTransportTelemetryUpdate::new(
                generation,
                PlayerObservationTimestamp::from_adapter_observation(
                    Duration::from_secs(1),
                    Duration::from_secs(2),
                ),
            ));
        adapter
            .pending_cache_telemetry_updates
            .push_back(PlayerCacheTelemetryUpdate {
                media_generation: Some(generation),
                observed_at: Some(PlayerObservationTimestamp::from_adapter_observation(
                    Duration::from_secs(1),
                    Duration::from_secs(2),
                )),
                ..PlayerCacheTelemetryUpdate::default()
            });

        let transport_timestamp = adapter
            .take_transport_telemetry_update()
            .and_then(|update| update.observed_at)
            .expect("transport telemetry should retain a timestamp");
        let cache_timestamp = adapter
            .take_cache_telemetry_update()
            .and_then(|update| update.observed_at)
            .expect("cache telemetry should retain a timestamp");

        for timestamp in [transport_timestamp, cache_timestamp] {
            assert_eq!(
                timestamp.elapsed_since_adapter_start(),
                Duration::from_secs(1),
                "popping telemetry must preserve the original observation clock"
            );
            assert!(
                timestamp.delivery_reference_since_adapter_start() >= Duration::from_secs(9),
                "popping telemetry must retag delivery after its adapter-queue dwell"
            );
        }
    }

    #[test]
    fn control_overflow_invalidates_stale_authoritative_policy_snapshot() {
        let mut adapter = MpvAdapter::with_test_transport_and_ipc_timeout(
            RejectingHeartbeatTransport {
                responses: VecDeque::new(),
            },
            Duration::from_millis(100),
        );
        adapter.set_network_media_policy_state(MpvNetworkMediaPolicyState::NetworkMediaUpdated);
        let prior_revision = adapter.network_media_options_runtime_health_revision;
        let (_, control_capacity) = MpvJsonIpcClient::test_runtime_queue_capacities();
        let client = adapter
            .ipc_client
            .as_mut()
            .expect("test adapter should remain attached");
        for nonce in 0..=control_capacity {
            client.inject_test_event(json!({
                "event": MPV_EVENT_CLIENT_MESSAGE,
                "args": [
                    SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_HEARTBEAT,
                    json!({"heartbeatNonce": nonce}).to_string(),
                ],
            }));
        }

        PlayerAdapter::maintain_runtime_leases_nonblocking(&mut adapter);

        let snapshot = adapter.network_options_runtime_health_snapshot();
        assert!(matches!(
            snapshot.hook_health,
            MpvNetworkOptionsHookHealth::Degraded(ref reason)
                if reason.contains("control queue overflowed")
        ));
        assert_eq!(snapshot.media_policy, MpvNetworkMediaPolicyState::Unknown);
        assert!(snapshot.revision > prior_revision);
        assert_eq!(
            adapter
                .ipc_client
                .as_ref()
                .expect("test adapter should remain attached")
                .test_runtime_queue_sizes(),
            (0, 0)
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
