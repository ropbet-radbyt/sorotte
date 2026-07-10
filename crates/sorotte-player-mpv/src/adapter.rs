mod player_adapter;
mod state;

use std::{
    collections::VecDeque,
    path::Path,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use sorotte_player_api::{
    LocalFileUpdate, PlayerError, PlayerMediaLoadFailureKind, PlayerMediaLoadOutcome,
    PlayerPlaybackTelemetryUpdate,
};

use crate::constants::*;
#[cfg(test)]
use crate::ipc::MpvJsonIpcTransport;
use crate::ipc::{MpvIpcConnectionEvent, MpvJsonIpcClient};
use crate::legacy_ui::{
    LegacySyncplayOsdKind, LegacySyncplayUiSettings, legacy_syncplayintf_script_name_for_path,
    sanitize_legacy_syncplay_script_message_text,
};

use self::state::MpvObservedState;

const PAUSED_POSITION_POLL_INTERVAL: Duration = Duration::from_millis(100);

pub struct MpvAdapter {
    paused: bool,
    position_seconds: f64,
    playback_rate: f64,
    paused_for_cache: bool,
    cache_buffering_percent: Option<f64>,
    muted: bool,
    volume: Option<f64>,
    deinterlace: bool,
    keepaspect: bool,
    keepaspect_window: bool,
    fullscreen: bool,
    ontop: bool,
    border: bool,
    force_window: bool,
    keep_open: bool,
    keep_open_pause: bool,
    cursor_autohide_fs_only: bool,
    stop_screensaver: bool,
    sub_visibility: bool,
    osd_bar: bool,
    window_maximized: bool,
    window_minimized: bool,
    current_path: Option<String>,
    pending_local_file_update: Option<LocalFileUpdate>,
    pending_playback_telemetry_update: Option<PlayerPlaybackTelemetryUpdate>,
    pending_media_load_outcomes: VecDeque<PlayerMediaLoadOutcome>,
    pending_chat_requests: VecDeque<String>,
    pending_load_request: Option<String>,
    last_polled_local_file_update: Option<LocalFileUpdate>,
    last_paused_position_poll_at: Option<Instant>,
    observed_state: MpvObservedState,
    observers_registered: bool,
    legacy_syncplay_ui_settings: LegacySyncplayUiSettings,
    legacy_syncplayintf_script_loaded: bool,
    legacy_syncplayintf_options_applied: bool,
    legacy_syncplayintf_script_name: String,
    ipc_client: Option<MpvJsonIpcClient>,
    pending_ipc_connection_events: VecDeque<MpvIpcConnectionEvent>,
}

impl MpvAdapter {
    pub fn with_json_ipc(path: impl AsRef<Path>) -> Result<Self, PlayerError> {
        let mut adapter = Self::default();
        adapter.connect_json_ipc(path)?;
        Ok(adapter)
    }

    pub fn connect_json_ipc(&mut self, path: impl AsRef<Path>) -> Result<(), PlayerError> {
        let client =
            MpvJsonIpcClient::connect(path.as_ref()).map_err(PlayerError::OperationFailed)?;
        self.collect_ipc_connection_events();
        self.ipc_client = Some(client);
        Ok(())
    }

    pub fn take_ipc_connection_events(&mut self) -> Vec<MpvIpcConnectionEvent> {
        self.collect_ipc_connection_events();
        self.pending_ipc_connection_events.drain(..).collect()
    }

    fn collect_ipc_connection_events(&mut self) {
        let Some(ipc_client) = self.ipc_client.as_mut() else {
            return;
        };
        self.pending_ipc_connection_events
            .extend(ipc_client.take_connection_events());
    }

    pub fn current_path(&self) -> Option<&str> {
        self.current_path.as_deref()
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn position_seconds(&self) -> f64 {
        self.position_seconds
    }

    pub fn playback_rate(&self) -> f64 {
        if self.playback_rate == 0.0 {
            1.0
        } else {
            self.playback_rate
        }
    }

    pub fn paused_for_cache(&self) -> bool {
        self.paused_for_cache
    }

    pub fn cache_buffering_percent(&self) -> Option<f64> {
        self.cache_buffering_percent
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    pub fn volume(&self) -> f64 {
        self.volume.unwrap_or(100.0)
    }

    pub fn deinterlace(&self) -> bool {
        self.deinterlace
    }

    pub fn keepaspect(&self) -> bool {
        self.keepaspect
    }

    pub fn keepaspect_window(&self) -> bool {
        self.keepaspect_window
    }

    pub fn fullscreen(&self) -> bool {
        self.fullscreen
    }

    pub fn ontop(&self) -> bool {
        self.ontop
    }

    pub fn border(&self) -> bool {
        self.border
    }

    pub fn force_window(&self) -> bool {
        self.force_window
    }

    pub fn keep_open(&self) -> bool {
        self.keep_open
    }

    pub fn keep_open_pause(&self) -> bool {
        self.keep_open_pause
    }

    pub fn cursor_autohide_fs_only(&self) -> bool {
        self.cursor_autohide_fs_only
    }

    pub fn stop_screensaver(&self) -> bool {
        self.stop_screensaver
    }

    pub fn sub_visibility(&self) -> bool {
        self.sub_visibility
    }

    pub fn osd_bar(&self) -> bool {
        self.osd_bar
    }

    pub fn window_maximized(&self) -> bool {
        self.window_maximized
    }

    pub fn window_minimized(&self) -> bool {
        self.window_minimized
    }

    pub fn queue_local_file_update(&mut self, update: LocalFileUpdate) {
        self.pending_local_file_update = Some(update);
    }

    pub fn legacy_syncplay_ui_settings(&self) -> &LegacySyncplayUiSettings {
        &self.legacy_syncplay_ui_settings
    }

    pub fn set_property_string(&mut self, name: &str, value: &str) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([MPV_COMMAND_SET_PROPERTY, name, value]))
    }

    pub fn set_property_i64(&mut self, name: &str, value: i64) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([MPV_COMMAND_SET_PROPERTY, name, value]))
    }

    pub fn show_text(
        &mut self,
        text: &str,
        duration_ms: u64,
        level: i64,
    ) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([MPV_COMMAND_SHOW_TEXT, text, duration_ms, level]))
    }

    pub fn load_legacy_syncplayintf_script(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<(), PlayerError> {
        if self.ipc_client.is_none() {
            return Ok(());
        }

        let script_path = path.as_ref().to_string_lossy().into_owned();
        let script_name = legacy_syncplayintf_script_name_for_path(path.as_ref());
        self.send_ipc_command_if_attached(json!([MPV_COMMAND_LOAD_SCRIPT, script_path]))?;
        self.legacy_syncplayintf_script_name = script_name;
        self.legacy_syncplayintf_script_loaded = true;
        self.legacy_syncplayintf_options_applied = false;
        self.try_send_legacy_syncplayintf_options_if_pending();
        Ok(())
    }

    pub fn configure_legacy_syncplay_ui_settings(
        &mut self,
        settings: LegacySyncplayUiSettings,
    ) -> Result<(), PlayerError> {
        self.legacy_syncplay_ui_settings = settings;
        if self.legacy_syncplay_ui_settings.should_move_osd() {
            self.set_property_string(MPV_PROPERTY_OSD_ALIGN_Y, "bottom")?;
            self.set_property_i64(
                MPV_PROPERTY_OSD_MARGIN_Y,
                self.legacy_syncplay_ui_settings.chat_osd_margin,
            )?;
        }
        self.legacy_syncplayintf_options_applied = false;
        self.try_send_legacy_syncplayintf_options_if_pending();
        Ok(())
    }

    pub fn show_syncplay_legacy_message(
        &mut self,
        message: &str,
        kind: LegacySyncplayOsdKind,
    ) -> Result<(), PlayerError> {
        if message.trim().is_empty() || !self.legacy_syncplay_ui_settings.show_osd {
            return Ok(());
        }

        let duration_ms = match kind {
            LegacySyncplayOsdKind::Notification => {
                self.legacy_syncplay_ui_settings.notification_timeout_ms
            }
            LegacySyncplayOsdKind::Alert => self.legacy_syncplay_ui_settings.alert_timeout_ms,
        };
        if self.legacy_syncplay_ui_settings.chat_output_enabled
            && self.ensure_legacy_syncplayintf_ready()
        {
            let script_message_name = match kind {
                LegacySyncplayOsdKind::Notification => "notification-osd-neutral",
                LegacySyncplayOsdKind::Alert => "alert-osd-neutral",
            };
            if self
                .send_syncplayintf_script_message(
                    script_message_name,
                    &sanitize_legacy_syncplay_script_message_text(message),
                )
                .is_ok()
            {
                return Ok(());
            }
            self.legacy_syncplayintf_options_applied = false;
        }
        self.show_text(message, duration_ms, LEGACY_SYNCPLAY_SHOW_TEXT_OSD_LEVEL)
    }

    pub fn show_syncplay_legacy_chat_message(&mut self, message: &str) -> Result<(), PlayerError> {
        if message.trim().is_empty() {
            return Ok(());
        }

        if self.legacy_syncplay_ui_settings.chat_output_enabled
            && self.ensure_legacy_syncplayintf_ready()
        {
            if self
                .send_syncplayintf_script_message(
                    "chat",
                    &sanitize_legacy_syncplay_script_message_text(message),
                )
                .is_ok()
            {
                return Ok(());
            }
            self.legacy_syncplayintf_options_applied = false;
        }

        let maybe_duration_ms = if self.legacy_syncplay_ui_settings.chat_output_enabled {
            Some(self.legacy_syncplay_ui_settings.chat_timeout_ms)
        } else if self.legacy_syncplay_ui_settings.show_osd {
            Some(self.legacy_syncplay_ui_settings.notification_timeout_ms)
        } else {
            None
        };

        let Some(duration_ms) = maybe_duration_ms else {
            return Ok(());
        };
        self.show_text(message, duration_ms, LEGACY_SYNCPLAY_SHOW_TEXT_OSD_LEVEL)
    }

    fn send_syncplayintf_script_message(
        &mut self,
        message_name: &str,
        payload: &str,
    ) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([
            MPV_COMMAND_SCRIPT_MESSAGE_TO,
            self.legacy_syncplayintf_script_name.as_str(),
            message_name,
            payload
        ]))
    }

    fn send_legacy_syncplayintf_options_if_loaded(&mut self) -> Result<(), PlayerError> {
        if !self.legacy_syncplayintf_script_loaded {
            return Ok(());
        }

        let payload = self
            .legacy_syncplay_ui_settings
            .syncplayintf_options_payload();
        if payload.trim().is_empty() {
            self.legacy_syncplayintf_options_applied = true;
            return Ok(());
        }

        self.send_syncplayintf_script_message("set_syncplayintf_options", &payload)?;
        self.legacy_syncplayintf_options_applied = true;
        Ok(())
    }

    fn try_send_legacy_syncplayintf_options_if_pending(&mut self) {
        if self.legacy_syncplayintf_options_applied {
            return;
        }

        let _ = self.send_legacy_syncplayintf_options_if_loaded();
    }

    fn ensure_legacy_syncplayintf_ready(&mut self) -> bool {
        self.try_send_legacy_syncplayintf_options_if_pending();
        self.legacy_syncplayintf_script_loaded && self.legacy_syncplayintf_options_applied
    }

    fn ensure_observers_registered_if_attached(&mut self) {
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

    fn poll_ipc_local_file_update_if_attached(&mut self) {
        self.ensure_observers_registered_if_attached();
        self.drain_ipc_events_if_attached();
        if self.pending_local_file_update.is_some() {
            return;
        }

        let Some(ipc_client) = self.ipc_client.as_mut() else {
            return;
        };

        let Ok(polled_update) = Self::poll_local_file_update_from_mpv(ipc_client) else {
            return;
        };
        let Some(polled_update) = polled_update else {
            return;
        };

        if self.pending_load_request.is_some() {
            self.complete_pending_load_request_from_polled_update_if_ready(polled_update);
            self.drain_ipc_events_if_attached();
            return;
        }

        self.observed_state.path = polled_update.path.clone();
        self.observed_state.duration_seconds = polled_update.duration_seconds;
        self.observed_state.size_bytes = polled_update.size_bytes;
        self.current_path = polled_update.path.clone();
        if Self::local_file_update_ready_for_sync(&polled_update) {
            self.record_local_file_update_if_changed(polled_update);
        }
        self.drain_ipc_events_if_attached();
    }

    fn poll_local_file_update_from_mpv(
        ipc_client: &mut MpvJsonIpcClient,
    ) -> Result<Option<LocalFileUpdate>, String> {
        let Some(path) = ipc_client.get_property_string(MPV_PROPERTY_PATH)? else {
            return Ok(None);
        };

        let mut local_file_update = Self::local_file_update_for_path(path.as_str());

        if let Some(duration_seconds) = ipc_client.get_property_f64(MPV_PROPERTY_DURATION)? {
            local_file_update = local_file_update.with_duration_seconds(duration_seconds);
        }

        if let Some(size_bytes) = ipc_client.get_property_u64(MPV_PROPERTY_FILE_SIZE)? {
            local_file_update = local_file_update.with_size_bytes(size_bytes);
        }

        Ok(Some(local_file_update))
    }

    fn poll_paused_position_telemetry_if_attached(&mut self) {
        if !self.paused {
            return;
        }

        let now = Instant::now();
        if self
            .last_paused_position_poll_at
            .is_some_and(|last_poll| now.duration_since(last_poll) < PAUSED_POSITION_POLL_INTERVAL)
        {
            return;
        }
        self.last_paused_position_poll_at = Some(now);

        let polled_position = {
            let Some(ipc_client) = self.ipc_client.as_mut() else {
                return;
            };
            ipc_client.get_property_f64(MPV_PROPERTY_TIME_POS)
        };
        self.drain_ipc_events_if_attached();

        let Ok(Some(position_seconds)) = polled_position else {
            return;
        };
        if !position_seconds.is_finite() || (self.position_seconds - position_seconds).abs() < 1e-6
        {
            return;
        }

        self.position_seconds = position_seconds;
        self.observed_state.position_seconds = Some(position_seconds);
        self.queue_playback_telemetry_update(
            PlayerPlaybackTelemetryUpdate::default().with_position_seconds(position_seconds),
        );
    }

    fn record_local_file_update_if_changed(&mut self, update: LocalFileUpdate) {
        if self.last_polled_local_file_update.as_ref() != Some(&update) {
            self.last_polled_local_file_update = Some(update.clone());
            self.pending_local_file_update = Some(update);
        }
    }

    fn complete_pending_load_request_from_polled_update_if_ready(
        &mut self,
        polled_update: LocalFileUpdate,
    ) {
        let Some(requested_target) = self.pending_load_request.as_deref() else {
            return;
        };
        if !Self::local_file_update_matches_request(&polled_update, requested_target)
            || !Self::local_file_update_ready_for_sync(&polled_update)
        {
            return;
        }
        let requested_target = self
            .pending_load_request
            .take()
            .expect("pending request should still be present");
        self.current_path = polled_update.path.clone();
        self.observed_state.path = polled_update.path.clone();
        self.observed_state.duration_seconds = polled_update.duration_seconds;
        self.observed_state.size_bytes = polled_update.size_bytes;
        self.record_local_file_update_if_changed(polled_update.clone());
        self.pending_media_load_outcomes
            .push_back(PlayerMediaLoadOutcome::success(
                requested_target,
                polled_update.path,
            ));
    }

    fn queue_playback_telemetry_update(&mut self, update: PlayerPlaybackTelemetryUpdate) {
        match self.pending_playback_telemetry_update.as_mut() {
            Some(pending) => {
                if let Some(paused) = update.paused
                    && !(paused && pending.paused_for_cache == Some(true))
                {
                    pending.paused = Some(paused);
                }
                if let Some(position_seconds) = update.position_seconds {
                    pending.position_seconds = Some(position_seconds);
                }
                if let Some(playback_rate) = update.playback_rate {
                    pending.playback_rate = Some(playback_rate);
                }
                if let Some(paused_for_cache) = update.paused_for_cache {
                    pending.paused_for_cache = Some(paused_for_cache);
                    if paused_for_cache && pending.paused == Some(true) {
                        pending.paused = None;
                    }
                }
                if let Some(cache_buffering_percent) = update.cache_buffering_percent {
                    pending.cache_buffering_percent = Some(cache_buffering_percent);
                }
            }
            None => {
                let mut update = update;
                if update.paused_for_cache == Some(true) && update.paused == Some(true) {
                    update.paused = None;
                }
                self.pending_playback_telemetry_update = Some(update);
            }
        }
    }

    fn chat_input_polling_enabled(&self) -> bool {
        self.legacy_syncplayintf_script_loaded
            && self.legacy_syncplayintf_options_applied
            && self.legacy_syncplay_ui_settings.chat_input_enabled
    }

    fn poll_ipc_events_for_chat_input_if_enabled(&mut self) {
        if !self.chat_input_polling_enabled() {
            return;
        }

        let Some(ipc_client) = self.ipc_client.as_mut() else {
            return;
        };

        let _ = ipc_client.get_property(MPV_PROPERTY_PAUSE);
        self.drain_ipc_events_if_attached();
    }

    fn drain_ipc_events_if_attached(&mut self) {
        let pending_events = match self.ipc_client.as_mut() {
            Some(ipc_client) => ipc_client.take_pending_events(),
            None => return,
        };
        for event in pending_events {
            self.handle_ipc_event(&event);
        }
    }

    fn handle_ipc_event(&mut self, event: &Value) {
        let Some(event_name) = event.get("event").and_then(Value::as_str) else {
            return;
        };

        match event_name {
            MPV_EVENT_FILE_LOADED => {
                self.handle_file_loaded_event();
                return;
            }
            MPV_EVENT_END_FILE => {
                self.handle_end_file_event(event);
                return;
            }
            MPV_EVENT_PROPERTY_CHANGE => {}
            MPV_EVENT_CLIENT_MESSAGE => {
                self.handle_client_message_event(event);
                return;
            }
            _ => return,
        }

        let Some(property_name) = event.get("name").and_then(Value::as_str) else {
            return;
        };
        let data = event.get("data");

        let file_metadata_changed = match property_name {
            MPV_PROPERTY_PATH => {
                let next_path = data.and_then(Value::as_str).map(ToOwned::to_owned);
                self.current_path = next_path.clone();
                self.observed_state.path = next_path;
                true
            }
            MPV_PROPERTY_DURATION => {
                self.observed_state.duration_seconds = data.and_then(Value::as_f64);
                true
            }
            MPV_PROPERTY_FILE_SIZE => {
                self.observed_state.size_bytes = data
                    .and_then(|value| value.as_u64().or_else(|| value.as_i64()?.try_into().ok()));
                true
            }
            MPV_PROPERTY_PAUSE => {
                if let Some(paused) = data.and_then(Value::as_bool) {
                    self.paused = paused;
                    self.observed_state.paused = Some(paused);
                    self.queue_playback_telemetry_update(
                        PlayerPlaybackTelemetryUpdate::default().with_paused(paused),
                    );
                } else {
                    self.observed_state.paused = None;
                }
                false
            }
            MPV_PROPERTY_TIME_POS => {
                if let Some(position_seconds) = data.and_then(Value::as_f64) {
                    self.position_seconds = position_seconds;
                    self.observed_state.position_seconds = Some(position_seconds);
                    self.queue_playback_telemetry_update(
                        PlayerPlaybackTelemetryUpdate::default()
                            .with_position_seconds(position_seconds),
                    );
                } else {
                    self.observed_state.position_seconds = None;
                }
                false
            }
            MPV_PROPERTY_SPEED => {
                if let Some(speed) = data.and_then(Value::as_f64) {
                    self.playback_rate = speed;
                    self.observed_state.playback_rate = Some(speed);
                    self.queue_playback_telemetry_update(
                        PlayerPlaybackTelemetryUpdate::default().with_playback_rate(speed),
                    );
                } else {
                    self.observed_state.playback_rate = None;
                }
                false
            }
            MPV_PROPERTY_PAUSED_FOR_CACHE => {
                if let Some(paused_for_cache) = data.and_then(Value::as_bool) {
                    self.paused_for_cache = paused_for_cache;
                    self.observed_state.paused_for_cache = Some(paused_for_cache);
                    self.queue_playback_telemetry_update(
                        PlayerPlaybackTelemetryUpdate::default()
                            .with_paused_for_cache(paused_for_cache),
                    );
                } else {
                    self.observed_state.paused_for_cache = None;
                }
                false
            }
            MPV_PROPERTY_CACHE_BUFFERING_STATE => {
                if let Some(cache_buffering_percent) = data.and_then(Value::as_f64) {
                    self.cache_buffering_percent = Some(cache_buffering_percent);
                    self.observed_state.cache_buffering_percent = Some(cache_buffering_percent);
                    self.queue_playback_telemetry_update(
                        PlayerPlaybackTelemetryUpdate::default()
                            .with_cache_buffering_percent(cache_buffering_percent),
                    );
                } else {
                    self.cache_buffering_percent = None;
                    self.observed_state.cache_buffering_percent = None;
                }
                false
            }
            _ => false,
        };

        if file_metadata_changed {
            self.maybe_emit_local_file_update_from_observed_state();
        }
    }

    fn handle_file_loaded_event(&mut self) {
        let Some(requested_target) = self.pending_load_request.take() else {
            return;
        };

        let loaded_update = self
            .ipc_client
            .as_mut()
            .and_then(|ipc_client| Self::poll_local_file_update_from_mpv(ipc_client).ok())
            .flatten()
            .unwrap_or_else(|| Self::local_file_update_for_path(&requested_target));
        self.current_path = loaded_update.path.clone();
        self.observed_state.path = loaded_update.path.clone();
        self.observed_state.duration_seconds = loaded_update.duration_seconds;
        self.observed_state.size_bytes = loaded_update.size_bytes;
        if Self::local_file_update_ready_for_sync(&loaded_update) {
            self.record_local_file_update_if_changed(loaded_update.clone());
        }
        self.pending_media_load_outcomes
            .push_back(PlayerMediaLoadOutcome::success(
                requested_target,
                loaded_update.path.clone(),
            ));
    }

    fn handle_end_file_event(&mut self, event: &Value) {
        let reason = event
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if reason != MPV_END_FILE_REASON_ERROR {
            return;
        }

        let Some(requested_target) = self.pending_load_request.take() else {
            return;
        };
        let message = event
            .get("file_error")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| {
                event.get("error").and_then(|value| match value {
                    Value::String(message) => Some(message.trim().to_owned()),
                    Value::Number(number) => Some(format!("mpv error code {number}")),
                    _ => None,
                })
            })
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "mpv failed to load the requested media.".to_owned());
        self.current_path = None;
        self.pending_local_file_update = None;
        self.last_polled_local_file_update = None;
        self.observed_state.path = None;
        self.observed_state.duration_seconds = None;
        self.observed_state.size_bytes = None;
        self.pending_media_load_outcomes
            .push_back(PlayerMediaLoadOutcome::failure(
                requested_target,
                None,
                Self::media_load_failure_kind_from_message(&message),
                message,
            ));
    }

    fn handle_client_message_event(&mut self, event: &Value) {
        let Some(args) = event.get("args").and_then(Value::as_array) else {
            return;
        };
        let Some(message_name) = args.first().and_then(Value::as_str) else {
            return;
        };
        if message_name != LEGACY_SYNCPLAYINTF_CLIENT_MESSAGE_CHAT {
            return;
        }
        let Some(message) = args.get(1).and_then(Value::as_str) else {
            return;
        };
        self.pending_chat_requests.push_back(message.to_owned());
    }

    fn maybe_emit_local_file_update_from_observed_state(&mut self) {
        if self.pending_load_request.is_some() {
            return;
        }
        let Some(path) = self.observed_state.path.as_deref() else {
            return;
        };

        let mut update = Self::local_file_update_for_path(path);
        if let Some(duration_seconds) = self.observed_state.duration_seconds {
            update = update.with_duration_seconds(duration_seconds);
        }
        if let Some(size_bytes) = self.observed_state.size_bytes {
            update = update.with_size_bytes(size_bytes);
        }
        if Self::local_file_update_ready_for_sync(&update) {
            self.record_local_file_update_if_changed(update);
        }
    }

    fn send_ipc_command_if_attached(&mut self, command: Value) -> Result<(), PlayerError> {
        if let Some(ipc_client) = self.ipc_client.as_mut() {
            ipc_client
                .send_command_expect_success(command)
                .map_err(PlayerError::OperationFailed)?;
        }
        self.drain_ipc_events_if_attached();
        Ok(())
    }

    fn local_file_update_for_path(path: &str) -> LocalFileUpdate {
        let name = if path.contains("://") {
            path.to_owned()
        } else {
            Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(path)
                .to_owned()
        };
        let size_bytes = if path.contains("://") {
            0
        } else {
            std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
        };

        LocalFileUpdate::new(name)
            .with_size_bytes(size_bytes)
            .with_path(path.to_owned())
    }

    fn local_file_update_ready_for_sync(update: &LocalFileUpdate) -> bool {
        match update.path.as_deref() {
            Some(path) if !path.contains("://") => update.duration_seconds.is_some(),
            _ => true,
        }
    }

    fn local_file_update_matches_request(update: &LocalFileUpdate, requested_target: &str) -> bool {
        if requested_target.trim().is_empty() {
            return false;
        }

        if let Some(path) = update.path.as_deref()
            && Self::media_target_matches(path, requested_target)
        {
            return true;
        }

        if Self::media_target_matches(&update.name, requested_target) {
            return true;
        }

        Path::new(requested_target)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|requested_name| Self::media_target_matches(&update.name, requested_name))
    }

    fn media_target_matches(left: &str, right: &str) -> bool {
        if cfg!(windows) {
            left.eq_ignore_ascii_case(right)
        } else {
            left == right
        }
    }

    fn media_load_failure_kind_from_message(message: &str) -> PlayerMediaLoadFailureKind {
        let normalized = message.to_ascii_lowercase();
        if normalized.contains("failed to recognize file format")
            || normalized.contains("unsupported")
        {
            return PlayerMediaLoadFailureKind::FormatUnsupported;
        }
        if (normalized.contains("yt-dlp")
            || normalized.contains("youtube-dl")
            || normalized.contains("deno"))
            && (normalized.contains("not found")
                || normalized.contains("not enough permissions")
                || normalized.contains("no such file"))
        {
            return PlayerMediaLoadFailureKind::HelperMissing;
        }
        if normalized.contains("yt-dlp")
            || normalized.contains("youtube-dl")
            || normalized.contains("deno")
        {
            return PlayerMediaLoadFailureKind::HelperBroken;
        }
        if normalized.contains("connection")
            || normalized.contains("network")
            || normalized.contains("http")
            || normalized.contains("timed out")
        {
            return PlayerMediaLoadFailureKind::Network;
        }
        if normalized.contains("aborted") || normalized.contains("interrupt") {
            return PlayerMediaLoadFailureKind::LoadAborted;
        }
        PlayerMediaLoadFailureKind::Unknown
    }

    #[cfg(test)]
    pub(crate) fn with_test_transport(transport: impl MpvJsonIpcTransport + 'static) -> Self {
        Self {
            ipc_client: Some(MpvJsonIpcClient::new(Box::new(transport))),
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn with_test_transport_and_ipc_timeout(
        transport: impl MpvJsonIpcTransport + 'static,
        command_timeout: std::time::Duration,
    ) -> Self {
        Self {
            ipc_client: Some(MpvJsonIpcClient::new_with_command_timeout(
                Box::new(transport),
                command_timeout,
            )),
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn enable_test_legacy_chat_input(&mut self) {
        self.legacy_syncplayintf_script_loaded = true;
        self.legacy_syncplayintf_options_applied = true;
        self.legacy_syncplay_ui_settings.chat_input_enabled = true;
    }
}
