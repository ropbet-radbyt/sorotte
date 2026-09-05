//! IPC attachment initialization owns the version gate and observer installation.
//! Inputs are an explicit endpoint plus a connected client; success installs one
//! fresh physical epoch, while failure keeps the adapter detached. New attachment
//! resets load/event identity, recovery evidence and bridge supervision together.
//! Retry scheduling stays in reconnection and starts after initialization ends.
use super::*;

impl MpvAdapter {
    pub fn with_json_ipc(path: impl AsRef<Path>) -> Result<Self, PlayerError> {
        let mut adapter = Self::default();
        adapter.connect_json_ipc(path)?;
        Ok(adapter)
    }

    pub fn connect_json_ipc(&mut self, path: impl AsRef<Path>) -> Result<(), PlayerError> {
        let endpoint = path.as_ref().to_path_buf();
        let client = MpvJsonIpcClient::connect(&endpoint).map_err(PlayerError::OperationFailed)?;
        self.initialize_json_ipc_attachment(endpoint, client)
    }

    pub(super) fn initialize_json_ipc_attachment(
        &mut self,
        endpoint: PathBuf,
        mut client: MpvJsonIpcClient,
    ) -> Result<(), PlayerError> {
        let replacing_attachment = self.ipc_client.is_some();
        let attachment_epoch = self.lifecycle_epoch().get();
        if replacing_attachment {
            emit_player_lifecycle_transition(
                "PLAYER-LOSS-001",
                "player-attachment",
                Trigger::Recovery,
                Disposition::Superseded,
                &[("attachment-epoch", attachment_epoch)],
            );
            emit_player_lifecycle_transition(
                "PLAYER-RELAUNCH-001",
                "player-attachment",
                Trigger::Recovery,
                Disposition::Accepted,
                &[("attachment-epoch", attachment_epoch)],
            );
        } else {
            emit_player_lifecycle_transition(
                "PLAYER-LAUNCH-001",
                "player-attachment",
                Trigger::Startup,
                Disposition::Submitted,
                &[("attachment-epoch", attachment_epoch)],
            );
        }
        emit_player_lifecycle_transition(
            "PLAYER-CONNECT-001",
            "player-attachment",
            if replacing_attachment {
                Trigger::Recovery
            } else {
                Trigger::Startup
            },
            Disposition::Submitted,
            &[("attachment-epoch", attachment_epoch)],
        );
        if let Err(error) = Self::require_supported_mpv_version(&mut client) {
            emit_player_lifecycle_transition(
                "PLAYER-LOSS-001",
                "player-attachment",
                Trigger::Fault,
                Disposition::Rejected,
                &[("attachment-epoch", attachment_epoch)],
            );
            return Err(error);
        }
        self.release_sorotte_bridge_best_effort();
        self.collect_ipc_connection_events();
        if replacing_attachment {
            self.reset_player_state_for_new_attachment();
        }
        self.apply_lifecycle_input(PlayerLifecycleInput::AttachmentReplaced);
        self.next_lifecycle_transcript_ingress_sequence = 1;
        self.fail_all_accepted_tracked_commands(PlayerCommandFailureKind::TransportDisconnected);
        self.pending_tracked_commands.clear();
        self.last_finished_tracked_command_debug = None;
        self.pending_load_request = None;
        self.pending_load_generation = None;
        self.stream_recovery.interrupted_network_stream_recovery = None;
        self.stream_recovery.network_stream_recovery_evidence = None;
        self.stream_recovery.network_cache_stall = None;
        self.clear_physical_projection();
        self.latest_start_file_observation = None;
        self.deferred_start_file_observation = None;
        self.deferred_file_loaded_observation = None;
        self.active_generation_has_restarted = false;
        self.pending_local_file_update = None;
        self.last_polled_local_file_update = None;
        self.last_paused_position_poll_at = None;
        self.last_paused_position_telemetry_at = None;
        self.last_ipc_event_fence_at = None;
        self.pending_ipc_event_fence_command_id = None;
        self.invalidate_cache_pause_readback_scope();
        self.pending_playback_telemetry_update = None;
        self.pending_transport_telemetry_updates.clear();
        self.pending_cache_telemetry_updates.clear();
        self.pending_media_load_outcomes.clear();
        self.observed_state = MpvObservedState::default();
        self.transport_phase = PlayerTransportPhase::Empty;
        self.reset_timeline_metadata();
        self.simulation_mode = false;
        self.ipc_client = Some(client);
        self.ipc_endpoint = Some(endpoint);
        self.ipc_reconnect_not_before = None;
        self.reset_legacy_syncplayintf_attachment_for_new_ipc();
        self.observers_registered = false;
        self.transport_observers_registered = false;
        self.reset_network_media_options_attachment_state();
        self.legacy_syncplay_osd_placement_restore = None;
        #[cfg(not(test))]
        {
            self.ensure_observers_registered_if_attached();
            self.reconcile_lifecycle_from_authority();
        }
        emit_player_lifecycle_transition(
            "PLAYER-ATTACH-001",
            "player-attachment",
            Trigger::PlayerEvent,
            Disposition::Applied,
            &[("attachment-epoch", self.lifecycle_epoch().get())],
        );
        Ok(())
    }

    pub(super) fn reset_player_state_for_new_attachment(&mut self) {
        let accepted_command_ids = self
            .pending_tracked_commands
            .iter()
            .filter(|command| command.accepted_at.is_some())
            .map(|command| command.id)
            .collect::<BTreeSet<_>>();
        self.fail_all_accepted_tracked_commands(PlayerCommandFailureKind::TransportDisconnected);
        let handoff_progress = accepted_command_ids
            .iter()
            .filter_map(|command_id| {
                self.unacknowledged_terminal_command_progress
                    .get(command_id)
                    .copied()
            })
            .collect::<Vec<_>>();

        self.pending_tracked_commands.clear();
        self.last_finished_tracked_command_debug = None;
        self.pending_command_progress_updates.clear();
        self.pending_media_load_outcomes.clear();
        self.pending_ordered_player_events.clear();
        self.last_delivered_ordered_command_progress.clear();
        self.last_delivered_ordered_media_load_outcomes.clear();
        self.pending_local_file_update = None;
        self.pending_local_file_generation = None;
        self.pending_local_file_observed_at = None;
        self.pending_playback_telemetry_update = None;
        self.pending_transport_telemetry_updates.clear();
        self.pending_cache_telemetry_updates.clear();
        self.pending_ipc_connection_events.clear();

        self.pending_load_request = None;
        self.pending_load_generation = None;
        self.clear_physical_projection();
        self.latest_start_file_observation = None;
        self.deferred_start_file_observation = None;
        self.deferred_file_loaded_observation = None;
        self.stream_recovery.interrupted_network_stream_recovery = None;
        self.stream_recovery.network_stream_recovery_evidence = None;
        self.stream_recovery.network_cache_stall = None;
        self.last_polled_local_file_update = None;
        self.active_generation_has_restarted = false;
        self.transport_phase = PlayerTransportPhase::Empty;
        self.observed_state = MpvObservedState::default();
        self.paused = false;
        self.logical_pause_explicit = false;
        self.position_seconds = 0.0;
        self.playback_rate = 0.0;
        self.paused_for_cache = false;
        self.cache_buffering_percent = None;
        self.last_paused_position_poll_at = None;
        self.last_paused_position_telemetry_at = None;
        self.last_ipc_event_fence_at = None;
        self.pending_ipc_event_fence_command_id = None;
        self.invalidate_cache_pause_readback_scope();
        self.playback_restart_sequence = 0;
        self.reset_timeline_metadata();
        self.ordered_player_event_reacquisition_required = true;
        self.ordered_player_event_reacquisition_requested_by_consumer = false;
        for progress in handoff_progress {
            self.queue_command_progress(progress);
        }
    }

    pub(super) fn require_supported_mpv_version(
        client: &mut MpvJsonIpcClient,
    ) -> Result<(), PlayerError> {
        let reported_version = match client.get_property_string_classified(MPV_PROPERTY_VERSION) {
            Ok(Some(version)) => version,
            Ok(None) => {
                return Err(PlayerError::OperationFailed(format!(
                    "{}{minimum} or newer, but the connected mpv did not report an mpv-version; upgrade mpv and try again",
                    crate::UNSUPPORTED_MPV_VERSION_ERROR_PREFIX,
                    minimum = crate::MINIMUM_SUPPORTED_MPV_VERSION,
                )));
            }
            Err(error) if error.is_property_unavailable() => {
                return Err(PlayerError::OperationFailed(format!(
                    "{}{minimum} or newer, but the connected mpv does not expose the mpv-version property; upgrade mpv and try again",
                    crate::UNSUPPORTED_MPV_VERSION_ERROR_PREFIX,
                    minimum = crate::MINIMUM_SUPPORTED_MPV_VERSION,
                )));
            }
            Err(error) => return Err(PlayerError::OperationFailed(error.into_message())),
        };
        let parsed_version = Self::parse_mpv_version(&reported_version).ok_or_else(|| {
            PlayerError::OperationFailed(format!(
                "{}{minimum} or newer, but the connected mpv reported an unrecognized mpv-version; install an official supported mpv build and try again",
                crate::UNSUPPORTED_MPV_VERSION_ERROR_PREFIX,
                minimum = crate::MINIMUM_SUPPORTED_MPV_VERSION,
            ))
        })?;
        if parsed_version < MINIMUM_SUPPORTED_MPV_VERSION_COMPONENTS {
            let (major, minor, patch) = parsed_version;
            return Err(PlayerError::OperationFailed(format!(
                "{}{minimum} or newer, but the connected mpv reports mpv {major}.{minor}.{patch}; upgrade mpv and try again",
                crate::UNSUPPORTED_MPV_VERSION_ERROR_PREFIX,
                minimum = crate::MINIMUM_SUPPORTED_MPV_VERSION,
            )));
        }
        Ok(())
    }

    pub(super) fn parse_mpv_version(version: &str) -> Option<(u64, u64, u64)> {
        version
            .split(|character: char| !(character.is_ascii_digit() || character == '.'))
            .filter(|part| part.bytes().filter(|byte| *byte == b'.').count() >= 2)
            .find_map(|part| {
                let mut components = part.split('.');
                Some((
                    components.next()?.parse::<u64>().ok()?,
                    components.next()?.parse::<u64>().ok()?,
                    components.next()?.parse::<u64>().ok()?,
                ))
            })
    }

    pub(super) fn ensure_observers_registered_if_attached(&mut self) {
        if self.observers_registered {
            return;
        }
        if self.ipc_client.is_none() {
            return;
        }

        let registrations = [
            (MPV_OBS_PATH_ID, MPV_PROPERTY_PATH),
            (MPV_OBS_DURATION_ID, MPV_PROPERTY_DURATION),
            (MPV_OBS_FILE_SIZE_ID, MPV_PROPERTY_FILE_SIZE),
            (MPV_OBS_PAUSE_ID, MPV_PROPERTY_PAUSE),
            (MPV_OBS_TIME_POS_ID, MPV_PROPERTY_TIME_POS),
            (MPV_OBS_SPEED_ID, MPV_PROPERTY_SPEED),
            (MPV_OBS_PAUSED_FOR_CACHE_ID, MPV_PROPERTY_PAUSED_FOR_CACHE),
            (
                MPV_OBS_CACHE_BUFFERING_STATE_ID,
                MPV_PROPERTY_CACHE_BUFFERING_STATE,
            ),
        ];

        for (observer_id, property_name) in registrations {
            let Some(ipc_client) = self.ipc_client.as_mut() else {
                return;
            };
            let registration_result = ipc_client.observe_property(observer_id, property_name);
            if registration_result.is_err() {
                return;
            }
            self.drain_ipc_events_if_attached();
        }
        self.observers_registered = true;
    }

    pub(super) fn ensure_transport_observers_registered_if_attached(&mut self) {
        self.ensure_observers_registered_if_attached();
        if self.transport_observers_registered || self.ipc_client.is_none() {
            return;
        }

        let registrations = [
            (MPV_OBS_SEEKING_ID, MPV_PROPERTY_SEEKING),
            (MPV_OBS_SEEKABLE_ID, MPV_PROPERTY_SEEKABLE),
            (MPV_OBS_CORE_IDLE_ID, MPV_PROPERTY_CORE_IDLE),
            (
                MPV_OBS_DEMUXER_CACHE_STATE_ID,
                MPV_PROPERTY_DEMUXER_CACHE_STATE,
            ),
            (
                MPV_OBS_DEMUXER_CACHE_IDLE_ID,
                MPV_PROPERTY_DEMUXER_CACHE_IDLE,
            ),
            // Observe both forms: the full metadata map provides a resilient
            // fallback while the narrower subproperty avoids retransmitting
            // unrelated tags.
            (MPV_OBS_YTDL_IS_LIVE_ID, MPV_PROPERTY_YTDL_IS_LIVE),
            (MPV_OBS_METADATA_ID, MPV_PROPERTY_METADATA),
            (MPV_OBS_EOF_REACHED_ID, MPV_PROPERTY_EOF_REACHED),
        ];

        for (observer_id, property_name) in registrations {
            let Some(ipc_client) = self.ipc_client.as_mut() else {
                return;
            };
            // Individual properties can be unavailable for a particular
            // media source or build. One rejection must not prevent the
            // remaining lifecycle properties from registering.
            let _ = ipc_client.observe_property(observer_id, property_name);
            self.drain_ipc_events_if_attached();
        }
        self.transport_observers_registered = true;
    }
}
