use std::fmt;

use super::*;

impl fmt::Debug for MpvAdapter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MpvAdapter")
            .field("paused", &self.paused)
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
            .field("current_path", &self.current_path)
            .field("pending_local_file_update", &self.pending_local_file_update)
            .field(
                "pending_playback_telemetry_update",
                &self.pending_playback_telemetry_update,
            )
            .field(
                "pending_media_load_outcomes",
                &self.pending_media_load_outcomes,
            )
            .field("pending_chat_requests", &self.pending_chat_requests)
            .field("pending_load_request", &self.pending_load_request)
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
                "legacy_syncplay_ui_settings",
                &self.legacy_syncplay_ui_settings,
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
            pending_local_file_update: None,
            pending_playback_telemetry_update: None,
            pending_media_load_outcomes: VecDeque::new(),
            pending_chat_requests: VecDeque::new(),
            pending_load_request: None,
            last_polled_local_file_update: None,
            last_paused_position_poll_at: None,
            observed_state: MpvObservedState::default(),
            observers_registered: false,
            legacy_syncplay_ui_settings: LegacySyncplayUiSettings::default(),
            legacy_syncplayintf_script_loaded: false,
            legacy_syncplayintf_options_applied: false,
            legacy_syncplayintf_script_name: LEGACY_SYNCPLAYINTF_SCRIPT_NAME.to_owned(),
            simulation_mode: false,
            ipc_client: None,
            pending_ipc_connection_events: VecDeque::new(),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct MpvObservedState {
    pub(super) path: Option<String>,
    pub(super) duration_seconds: Option<f64>,
    pub(super) size_bytes: Option<u64>,
    pub(super) paused: Option<bool>,
    pub(super) position_seconds: Option<f64>,
    pub(super) playback_rate: Option<f64>,
    pub(super) paused_for_cache: Option<bool>,
    pub(super) cache_buffering_percent: Option<f64>,
}
