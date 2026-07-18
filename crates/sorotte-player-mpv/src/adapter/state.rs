use std::fmt;

use super::*;

impl fmt::Debug for MpvAdapter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MpvAdapter")
            .field("paused", &self.paused)
            .field("logical_pause_explicit", &self.logical_pause_explicit)
            .field("position_seconds", &self.position_seconds)
            .field("playback_rate", &self.playback_rate)
            .field("paused_for_cache", &self.paused_for_cache)
            .field("cache_buffering_percent", &self.cache_buffering_percent)
            .field("muted", &self.muted)
            .field("volume", &self.volume)
            .field("deinterlace", &self.deinterlace)
            .field("keepaspect", &self.keepaspect)
            .field("keepaspect_window", &self.keepaspect_window)
            .field("fullscreen", &self.fullscreen)
            .field("ontop", &self.ontop)
            .field("border", &self.border)
            .field("force_window", &self.force_window)
            .field("keep_open", &self.keep_open)
            .field("keep_open_pause", &self.keep_open_pause)
            .field("cursor_autohide_fs_only", &self.cursor_autohide_fs_only)
            .field("stop_screensaver", &self.stop_screensaver)
            .field("sub_visibility", &self.sub_visibility)
            .field("osd_bar", &self.osd_bar)
            .field("window_maximized", &self.window_maximized)
            .field("window_minimized", &self.window_minimized)
            .field(
                "current_path",
                &self
                    .current_path
                    .as_ref()
                    .map(|_| sorotte_secret::REDACTED_SECRET),
            )
            .field(
                "network_media_option_count",
                &self.network_media_options.len(),
            )
            .field(
                "network_media_options_embedded_generation",
                &self
                    .network_media_options_embedded_load
                    .as_ref()
                    .map(|embedded| embedded.media_generation),
            )
            .field(
                "network_media_options_apply_identity_present",
                &self.network_media_options_apply_identity.is_some(),
            )
            .field(
                "pending_network_media_options_transition_outcome_count",
                &self.pending_network_media_options_transition_outcomes.len(),
            )
            .field("loadfile_options_syntax", &self.loadfile_options_syntax)
            .field("mpv_version", &self.mpv_version)
            .field("pending_local_file_update", &self.pending_local_file_update)
            .field(
                "pending_playback_telemetry_update",
                &self.pending_playback_telemetry_update,
            )
            .field(
                "pending_transport_telemetry_updates",
                &self.pending_transport_telemetry_updates,
            )
            .field("pending_tracked_commands", &self.pending_tracked_commands)
            .field(
                "pending_command_progress_updates",
                &self.pending_command_progress_updates,
            )
            .field(
                "pending_media_load_outcomes",
                &self.pending_media_load_outcomes,
            )
            .field("pending_chat_requests", &self.pending_chat_requests)
            .field(
                "pending_load_request",
                &self
                    .pending_load_request
                    .as_ref()
                    .map(|_| sorotte_secret::REDACTED_SECRET),
            )
            .field(
                "last_polled_local_file_update",
                &self.last_polled_local_file_update,
            )
            .field(
                "last_paused_position_poll_at",
                &self.last_paused_position_poll_at,
            )
            .field("observed_state", &self.observed_state)
            .field("observers_registered", &self.observers_registered)
            .field(
                "transport_observers_registered",
                &self.transport_observers_registered,
            )
            .field("next_media_generation", &self.next_media_generation)
            .field("active_media_generation", &self.active_media_generation)
            .field("pending_load_generation", &self.pending_load_generation)
            .field("active_playlist_entry_id", &self.active_playlist_entry_id)
            .field("transport_phase", &self.transport_phase)
            .field("active_file_loaded", &self.active_file_loaded)
            .field(
                "active_generation_has_restarted",
                &self.active_generation_has_restarted,
            )
            .field("timeline_kind", &self.timeline_kind)
            .field("ytdl_is_live", &self.ytdl_is_live)
            .field(
                "ytdl_is_live_metadata_generation",
                &self.ytdl_is_live_metadata_generation,
            )
            .field(
                "latest_cached_seekable_window",
                &self.latest_cached_seekable_window,
            )
            .field(
                "ytdl_live_probe_executable_configured",
                &self.ytdl_live_probe_executable.is_some(),
            )
            .field(
                "ytdl_live_probe_path_prefix_count",
                &self.ytdl_live_probe_path_prefixes.len(),
            )
            .field(
                "ytdl_live_probe_started",
                &self.ytdl_live_probe_identity.is_some(),
            )
            .field(
                "pending_ytdl_live_probe",
                &self.pending_ytdl_live_probe.is_some(),
            )
            .field("playback_restart_sequence", &self.playback_restart_sequence)
            .field("next_command_id", &self.next_command_id)
            .field(
                "legacy_syncplay_ui_settings",
                &self.legacy_syncplay_ui_settings,
            )
            .field(
                "last_simulated_legacy_syncplay_osd_message",
                &self.last_simulated_legacy_syncplay_osd_message,
            )
            .field(
                "legacy_syncplay_osd_placement_overridden",
                &self.legacy_syncplay_osd_placement_restore.is_some(),
            )
            .field(
                "legacy_syncplayintf_script_loaded",
                &self.legacy_syncplayintf_script_loaded,
            )
            .field(
                "legacy_syncplayintf_options_applied",
                &self.legacy_syncplayintf_options_applied,
            )
            .field(
                "legacy_syncplayintf_script_name",
                &self.legacy_syncplayintf_script_name,
            )
            .field(
                "legacy_syncplayintf_bridge_instance_id",
                &self.legacy_syncplayintf_bridge_instance_id,
            )
            .field(
                "legacy_syncplayintf_pending_options_generation",
                &self.legacy_syncplayintf_pending_options_generation,
            )
            .field(
                "legacy_syncplayintf_acknowledged_options_generation",
                &self.legacy_syncplayintf_acknowledged_options_generation,
            )
            .field(
                "legacy_syncplayintf_lease_reacquire_required",
                &self.legacy_syncplayintf_lease_reacquire_required,
            )
            .field("sorotte_bridge_health", &self.sorotte_bridge_health)
            .field("ipc_endpoint", &self.ipc_endpoint)
            .field("simulation_mode", &self.simulation_mode)
            .field("ipc_attached", &self.ipc_client.is_some())
            .field(
                "pending_ipc_connection_events",
                &self.pending_ipc_connection_events,
            )
            .finish()
    }
}

impl Default for MpvAdapter {
    fn default() -> Self {
        Self {
            paused: false,
            logical_pause_explicit: false,
            position_seconds: 0.0,
            playback_rate: 0.0,
            paused_for_cache: false,
            cache_buffering_percent: None,
            muted: false,
            volume: None,
            deinterlace: false,
            keepaspect: false,
            keepaspect_window: false,
            fullscreen: false,
            ontop: false,
            border: false,
            force_window: false,
            keep_open: false,
            keep_open_pause: false,
            cursor_autohide_fs_only: false,
            stop_screensaver: false,
            sub_visibility: false,
            osd_bar: false,
            window_maximized: false,
            window_minimized: false,
            current_path: None,
            network_media_options: BTreeMap::new(),
            network_media_options_embedded_load: None,
            network_media_options_apply_identity: None,
            next_network_media_options_apply_attempt_id: 1,
            network_media_options_event_batch_depth: 0,
            deferred_network_media_options_observation: None,
            pending_network_media_options_transition_outcomes: VecDeque::new(),
            loadfile_options_syntax: None,
            mpv_version: None,
            pending_local_file_update: None,
            pending_playback_telemetry_update: None,
            pending_transport_telemetry_updates: VecDeque::new(),
            pending_tracked_commands: VecDeque::new(),
            pending_command_progress_updates: VecDeque::new(),
            pending_media_load_outcomes: VecDeque::new(),
            pending_chat_requests: VecDeque::new(),
            pending_load_request: None,
            last_polled_local_file_update: None,
            last_paused_position_poll_at: None,
            observed_state: MpvObservedState::default(),
            observers_registered: false,
            transport_observers_registered: false,
            observation_clock_origin: Instant::now(),
            next_media_generation: 1,
            active_media_generation: None,
            pending_load_generation: None,
            active_playlist_entry_id: None,
            playlist_entry_generations: HashMap::new(),
            transport_phase: PlayerTransportPhase::Empty,
            active_file_loaded: false,
            active_generation_has_restarted: false,
            timeline_kind: PlayerTimelineKind::Unknown,
            ytdl_is_live: false,
            ytdl_is_live_metadata_generation: None,
            latest_cached_seekable_window: None,
            path_metadata_generation: None,
            duration_metadata_generation: None,
            ytdl_live_probe_executable: None,
            ytdl_live_probe_path_prefixes: Vec::new(),
            ytdl_live_probe_identity: None,
            pending_ytdl_live_probe: None,
            playback_restart_sequence: 0,
            next_command_id: 1,
            legacy_syncplay_ui_settings: LegacySyncplayUiSettings::default(),
            last_simulated_legacy_syncplay_osd_message: None,
            legacy_syncplay_osd_placement_restore: None,
            legacy_syncplayintf_script_loaded: false,
            legacy_syncplayintf_options_applied: false,
            legacy_syncplayintf_script_name: LEGACY_SYNCPLAYINTF_SCRIPT_NAME.to_owned(),
            legacy_syncplayintf_bridge_instance_id: None,
            legacy_syncplayintf_owner_id: (*LEGACY_SYNCPLAYINTF_OWNER_ID).clone(),
            legacy_syncplayintf_attachment_id: format!(
                "detached-{}",
                NEXT_LEGACY_SYNCPLAYINTF_ATTACHMENT.fetch_add(1, Ordering::Relaxed)
            ),
            legacy_syncplayintf_next_options_generation: 1,
            legacy_syncplayintf_pending_options_generation: None,
            legacy_syncplayintf_acknowledged_options_generation: None,
            legacy_syncplayintf_options_ack_error: None,
            legacy_syncplayintf_next_ping_nonce: 1,
            legacy_syncplayintf_pending_ping_nonce: None,
            legacy_syncplayintf_last_heartbeat_at: None,
            legacy_syncplayintf_last_discovery_at: None,
            legacy_syncplayintf_lease_reacquire_required: false,
            legacy_syncplayintf_runtime_rediscovery_required: false,
            legacy_syncplayintf_runtime_recovery_attempts: 0,
            legacy_syncplayintf_runtime_recovery_failure: None,
            sorotte_bridge_health: SorotteBridgeHealth::Disabled,
            pending_sorotte_bridge_health_transitions: VecDeque::new(),
            ipc_endpoint: None,
            simulation_mode: false,
            ipc_client: None,
            pending_ipc_connection_events: VecDeque::new(),
        }
    }
}

#[derive(Default, Clone, PartialEq)]
pub(super) struct MpvObservedState {
    pub(super) path: Option<String>,
    pub(super) duration_seconds: Option<f64>,
    pub(super) size_bytes: Option<u64>,
    pub(super) paused: Option<bool>,
    pub(super) logical_pause: Option<bool>,
    pub(super) position_seconds: Option<f64>,
    pub(super) playback_rate: Option<f64>,
    pub(super) paused_for_cache: Option<bool>,
    pub(super) cache_buffering_percent: Option<f64>,
    pub(super) seeking: Option<bool>,
    pub(super) seekable: Option<bool>,
    pub(super) core_idle: Option<bool>,
    pub(super) demuxer_cache_idle: Option<bool>,
    pub(super) eof_reached: Option<bool>,
}

impl std::fmt::Debug for MpvObservedState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MpvObservedState")
            .field(
                "path",
                &self.path.as_ref().map(|_| sorotte_secret::REDACTED_SECRET),
            )
            .field("duration_seconds", &self.duration_seconds)
            .field("size_bytes", &self.size_bytes)
            .field("paused", &self.paused)
            .field("logical_pause", &self.logical_pause)
            .field("position_seconds", &self.position_seconds)
            .field("playback_rate", &self.playback_rate)
            .field("paused_for_cache", &self.paused_for_cache)
            .field("cache_buffering_percent", &self.cache_buffering_percent)
            .field("seeking", &self.seeking)
            .field("seekable", &self.seekable)
            .field("core_idle", &self.core_idle)
            .field("demuxer_cache_idle", &self.demuxer_cache_idle)
            .field("eof_reached", &self.eof_reached)
            .finish()
    }
}

#[cfg(test)]
mod credential_debug_tests {
    use super::{MpvAdapter, MpvObservedState};

    #[test]
    fn observed_path_debug_redacts_tokenized_urls() {
        let secret = "mpv-observed-path-token-canary";
        let state = MpvObservedState {
            path: Some(format!("https://plex.invalid/video?X-Plex-Token={secret}")),
            ..MpvObservedState::default()
        };

        let debug = format!("{state:?}");
        assert!(debug.contains(sorotte_secret::REDACTED_SECRET));
        assert!(!debug.contains(secret));
    }

    #[test]
    fn adapter_debug_redacts_retained_media_targets() {
        let secret = "mpv-adapter-target-token-canary";
        let target = format!("https://plex.invalid/video?X-Plex-Token={secret}");
        let mut adapter = MpvAdapter {
            current_path: Some(target.clone()),
            pending_load_request: Some(target.clone()),
            ..MpvAdapter::default()
        };
        adapter.pending_local_file_update =
            Some(sorotte_player_api::LocalFileUpdate::new(target.clone()).with_path(target));

        let debug = format!("{adapter:?}");
        assert!(debug.contains(sorotte_secret::REDACTED_SECRET));
        assert!(!debug.contains(secret));
    }
}
