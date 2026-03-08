use std::{
    collections::VecDeque,
    fmt,
    io::{self, Read, Write},
    path::Path,
};

use serde_json::{Value, json};
use syncplay_player_api::{
    LocalFileUpdate, PlayerAdapter, PlayerError, PlayerPlaybackTelemetryUpdate,
};

const MPV_COMMAND_SET_PROPERTY: &str = "set_property";
const MPV_COMMAND_SET: &str = "set";
const MPV_COMMAND_GET_PROPERTY: &str = "get_property";
const MPV_COMMAND_OBSERVE_PROPERTY: &str = "observe_property";
const MPV_COMMAND_LOADFILE: &str = "loadfile";
const MPV_COMMAND_APPLY_PROFILE: &str = "apply-profile";
const MPV_COMMAND_SHOW_TEXT: &str = "show-text";
const MPV_COMMAND_LOAD_SCRIPT: &str = "load-script";
const MPV_COMMAND_SCRIPT_MESSAGE_TO: &str = "script-message-to";
const MPV_LOADFILE_REPLACE: &str = "replace";
const MPV_PROPERTY_PAUSE: &str = "pause";
const MPV_PROPERTY_TIME_POS: &str = "time-pos";
const MPV_PROPERTY_SPEED: &str = "speed";
const MPV_PROPERTY_MUTE: &str = "mute";
const MPV_PROPERTY_VOLUME: &str = "volume";
const MPV_PROPERTY_DEINTERLACE: &str = "deinterlace";
const MPV_PROPERTY_KEEPASPECT: &str = "keepaspect";
const MPV_PROPERTY_KEEPASPECT_WINDOW: &str = "keepaspect-window";
const MPV_PROPERTY_FULLSCREEN: &str = "fullscreen";
const MPV_PROPERTY_ONTOP: &str = "ontop";
const MPV_PROPERTY_BORDER: &str = "border";
const MPV_PROPERTY_FORCE_WINDOW: &str = "force-window";
const MPV_PROPERTY_KEEP_OPEN: &str = "keep-open";
const MPV_PROPERTY_KEEP_OPEN_PAUSE: &str = "keep-open-pause";
const MPV_PROPERTY_CURSOR_AUTOHIDE_FS_ONLY: &str = "cursor-autohide-fs-only";
const MPV_PROPERTY_STOP_SCREENSAVER: &str = "stop-screensaver";
const MPV_PROPERTY_SUB_VISIBILITY: &str = "sub-visibility";
const MPV_PROPERTY_OSD_BAR: &str = "osd-bar";
const MPV_PROPERTY_WINDOW_MAXIMIZED: &str = "window-maximized";
const MPV_PROPERTY_WINDOW_MINIMIZED: &str = "window-minimized";
const MPV_PROPERTY_OSD_ALIGN_Y: &str = "osd-align-y";
const MPV_PROPERTY_OSD_MARGIN_Y: &str = "osd-margin-y";
const MPV_PROPERTY_PATH: &str = "path";
const MPV_PROPERTY_DURATION: &str = "duration";
const MPV_PROPERTY_FILE_SIZE: &str = "file-size";
const MPV_RESPONSE_SUCCESS: &str = "success";
const MPV_EVENT_PROPERTY_CHANGE: &str = "property-change";
const MPV_OBS_PATH_ID: u64 = 1;
const MPV_OBS_DURATION_ID: u64 = 2;
const MPV_OBS_FILE_SIZE_ID: u64 = 3;
const MPV_OBS_PAUSE_ID: u64 = 4;
const MPV_OBS_TIME_POS_ID: u64 = 5;
const MPV_OBS_SPEED_ID: u64 = 6;
const LEGACY_SYNCPLAY_SHOW_TEXT_OSD_LEVEL: i64 = 1;
const LEGACY_SYNCPLAYINTF_SCRIPT_NAME: &str = "syncplayintf";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacySyncplayOsdKind {
    Notification,
    Alert,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacySyncplayUiSettings {
    pub show_osd: bool,
    pub chat_output_enabled: bool,
    pub chat_input_enabled: bool,
    pub chat_input_font_underline: bool,
    pub chat_input_font_family: String,
    pub chat_input_relative_font_size: i64,
    pub chat_input_font_weight: i64,
    pub chat_input_font_color: String,
    pub chat_input_position: String,
    pub chat_direct_input: bool,
    pub chat_output_font_underline: bool,
    pub chat_output_font_family: String,
    pub chat_output_relative_font_size: i64,
    pub chat_output_font_weight: i64,
    pub chat_output_mode: String,
    pub chat_max_lines: i64,
    pub chat_top_margin: i64,
    pub chat_left_margin: i64,
    pub chat_bottom_margin: i64,
    pub chat_move_osd: bool,
    pub chat_osd_margin: i64,
    pub notification_timeout_ms: u64,
    pub alert_timeout_ms: u64,
    pub chat_timeout_ms: u64,
}

impl LegacySyncplayUiSettings {
    fn chat_input_position_top(&self) -> bool {
        self.chat_input_position.trim().eq_ignore_ascii_case("Top")
    }

    fn should_move_osd(&self) -> bool {
        self.chat_move_osd
            && (self.chat_output_enabled
                || (self.chat_input_enabled && self.chat_input_position_top()))
    }

    fn syncplayintf_options_payload(&self) -> String {
        let options = [
            (
                "chatInputEnabled",
                legacy_syncplay_bool_string_compatible(false),
            ),
            (
                "chatInputFontFamily",
                self.chat_input_font_family.trim().to_owned(),
            ),
            (
                "chatInputRelativeFontSize",
                self.chat_input_relative_font_size.to_string(),
            ),
            (
                "chatInputFontWeight",
                self.chat_input_font_weight.to_string(),
            ),
            (
                "chatInputFontUnderline",
                legacy_syncplay_bool_string_compatible(self.chat_input_font_underline),
            ),
            (
                "chatInputFontColor",
                self.chat_input_font_color.trim().to_owned(),
            ),
            (
                "chatInputPosition",
                self.chat_input_position.trim().to_owned(),
            ),
            (
                "chatOutputFontFamily",
                self.chat_output_font_family.trim().to_owned(),
            ),
            (
                "chatOutputRelativeFontSize",
                self.chat_output_relative_font_size.to_string(),
            ),
            (
                "chatOutputFontWeight",
                self.chat_output_font_weight.to_string(),
            ),
            (
                "chatOutputFontUnderline",
                legacy_syncplay_bool_string_compatible(self.chat_output_font_underline),
            ),
            ("chatOutputMode", self.chat_output_mode.trim().to_owned()),
            ("chatMaxLines", self.chat_max_lines.to_string()),
            ("chatTopMargin", self.chat_top_margin.to_string()),
            ("chatLeftMargin", self.chat_left_margin.to_string()),
            ("chatBottomMargin", self.chat_bottom_margin.to_string()),
            (
                "chatDirectInput",
                legacy_syncplay_bool_string_compatible(false),
            ),
            (
                "notificationTimeout",
                legacy_syncplay_timeout_seconds_string_compatible(self.notification_timeout_ms),
            ),
            (
                "alertTimeout",
                legacy_syncplay_timeout_seconds_string_compatible(self.alert_timeout_ms),
            ),
            (
                "chatTimeout",
                legacy_syncplay_timeout_seconds_string_compatible(self.chat_timeout_ms),
            ),
            (
                "chatOutputEnabled",
                legacy_syncplay_bool_string_compatible(self.chat_output_enabled),
            ),
        ];

        options
            .into_iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl Default for LegacySyncplayUiSettings {
    fn default() -> Self {
        Self {
            show_osd: true,
            chat_output_enabled: true,
            chat_input_enabled: true,
            chat_input_font_underline: false,
            chat_input_font_family: "sans-serif".to_owned(),
            chat_input_relative_font_size: 24,
            chat_input_font_weight: 1,
            chat_input_font_color: "#FFFF00".to_owned(),
            chat_input_position: "Top".to_owned(),
            chat_direct_input: false,
            chat_output_font_underline: false,
            chat_output_font_family: "sans-serif".to_owned(),
            chat_output_relative_font_size: 24,
            chat_output_font_weight: 1,
            chat_output_mode: "Chatroom".to_owned(),
            chat_max_lines: 7,
            chat_top_margin: 25,
            chat_left_margin: 20,
            chat_bottom_margin: 30,
            chat_move_osd: true,
            chat_osd_margin: 110,
            notification_timeout_ms: 3_000,
            alert_timeout_ms: 5_000,
            chat_timeout_ms: 7_000,
        }
    }
}

fn legacy_syncplay_bool_string_compatible(value: bool) -> String {
    if value {
        "True".to_owned()
    } else {
        "False".to_owned()
    }
}

fn legacy_syncplay_timeout_seconds_string_compatible(duration_ms: u64) -> String {
    if duration_ms.is_multiple_of(1_000) {
        return (duration_ms / 1_000).to_string();
    }

    let seconds = duration_ms as f64 / 1_000.0;
    let mut formatted = format!("{seconds:.3}");
    while formatted.contains('.') && formatted.ends_with('0') {
        formatted.pop();
    }
    if formatted.ends_with('.') {
        formatted.pop();
    }
    formatted
}

fn sanitize_legacy_syncplay_script_message_text(message: &str) -> String {
    message
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\\n")
        .replace('%', "%%")
        .replace('{', "\\{")
        .replace('}', "\\}")
}

pub struct MpvAdapter {
    paused: bool,
    position_seconds: f64,
    playback_rate: f64,
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
    last_polled_local_file_update: Option<LocalFileUpdate>,
    observed_state: MpvObservedState,
    observers_registered: bool,
    legacy_syncplay_ui_settings: LegacySyncplayUiSettings,
    legacy_syncplayintf_script_loaded: bool,
    ipc_client: Option<MpvJsonIpcClient>,
}

impl fmt::Debug for MpvAdapter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MpvAdapter")
            .field("paused", &self.paused)
            .field("position_seconds", &self.position_seconds)
            .field("playback_rate", &self.playback_rate)
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
                "last_polled_local_file_update",
                &self.last_polled_local_file_update,
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
            .field("ipc_attached", &self.ipc_client.is_some())
            .finish()
    }
}

impl Default for MpvAdapter {
    fn default() -> Self {
        Self {
            paused: false,
            position_seconds: 0.0,
            playback_rate: 0.0,
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
            last_polled_local_file_update: None,
            observed_state: MpvObservedState::default(),
            observers_registered: false,
            legacy_syncplay_ui_settings: LegacySyncplayUiSettings::default(),
            legacy_syncplayintf_script_loaded: false,
            ipc_client: None,
        }
    }
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
        self.ipc_client = Some(client);
        Ok(())
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
        self.send_ipc_command_if_attached(json!([MPV_COMMAND_LOAD_SCRIPT, script_path]))?;
        self.legacy_syncplayintf_script_loaded = true;
        self.send_legacy_syncplayintf_options_if_loaded()
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
        self.send_legacy_syncplayintf_options_if_loaded()
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
            && self.legacy_syncplayintf_script_loaded
        {
            let script_message_name = match kind {
                LegacySyncplayOsdKind::Notification => "notification-osd-neutral",
                LegacySyncplayOsdKind::Alert => "alert-osd-neutral",
            };
            return self.send_syncplayintf_script_message(
                script_message_name,
                &sanitize_legacy_syncplay_script_message_text(message),
            );
        }
        self.show_text(message, duration_ms, LEGACY_SYNCPLAY_SHOW_TEXT_OSD_LEVEL)
    }

    pub fn show_syncplay_legacy_chat_message(&mut self, message: &str) -> Result<(), PlayerError> {
        if message.trim().is_empty() {
            return Ok(());
        }

        if self.legacy_syncplay_ui_settings.chat_output_enabled
            && self.legacy_syncplayintf_script_loaded
        {
            return self.send_syncplayintf_script_message(
                "chat",
                &sanitize_legacy_syncplay_script_message_text(message),
            );
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
            LEGACY_SYNCPLAYINTF_SCRIPT_NAME,
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
            return Ok(());
        }

        self.send_syncplayintf_script_message("set_syncplayintf_options", &payload)
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
        ];

        for (observer_id, property_name) in registrations {
            let registration_result = self
                .ipc_client
                .as_mut()
                .expect("checked is_some above")
                .observe_property(observer_id, property_name);
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

        self.observed_state.path = polled_update.path.clone();
        self.observed_state.duration_seconds = polled_update.duration_seconds;
        self.observed_state.size_bytes = polled_update.size_bytes;
        self.current_path = polled_update.path.clone();
        self.record_local_file_update_if_changed(polled_update);
        self.drain_ipc_events_if_attached();
    }

    fn poll_local_file_update_from_mpv(
        ipc_client: &mut MpvJsonIpcClient,
    ) -> Result<Option<LocalFileUpdate>, String> {
        let Some(path) = ipc_client
            .get_property_string(MPV_PROPERTY_PATH)
            .unwrap_or(None)
        else {
            return Ok(None);
        };

        let mut local_file_update = Self::local_file_update_for_path(path.as_str());

        if let Some(duration_seconds) = ipc_client
            .get_property_f64(MPV_PROPERTY_DURATION)
            .unwrap_or(None)
        {
            local_file_update = local_file_update.with_duration_seconds(duration_seconds);
        }

        if let Some(size_bytes) = ipc_client
            .get_property_u64(MPV_PROPERTY_FILE_SIZE)
            .unwrap_or(None)
        {
            local_file_update = local_file_update.with_size_bytes(size_bytes);
        }

        Ok(Some(local_file_update))
    }

    fn record_local_file_update_if_changed(&mut self, update: LocalFileUpdate) {
        if self.last_polled_local_file_update.as_ref() != Some(&update) {
            self.last_polled_local_file_update = Some(update.clone());
            self.pending_local_file_update = Some(update);
        }
    }

    fn queue_playback_telemetry_update(&mut self, update: PlayerPlaybackTelemetryUpdate) {
        match self.pending_playback_telemetry_update.as_mut() {
            Some(pending) => {
                if let Some(paused) = update.paused {
                    pending.paused = Some(paused);
                }
                if let Some(position_seconds) = update.position_seconds {
                    pending.position_seconds = Some(position_seconds);
                }
                if let Some(playback_rate) = update.playback_rate {
                    pending.playback_rate = Some(playback_rate);
                }
            }
            None => {
                self.pending_playback_telemetry_update = Some(update);
            }
        }
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
        if event_name != MPV_EVENT_PROPERTY_CHANGE {
            return;
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
            _ => false,
        };

        if file_metadata_changed {
            self.maybe_emit_local_file_update_from_observed_state();
        }
    }

    fn maybe_emit_local_file_update_from_observed_state(&mut self) {
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
        self.record_local_file_update_if_changed(update);
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

    #[cfg(test)]
    fn with_test_transport(transport: impl MpvJsonIpcTransport + 'static) -> Self {
        Self {
            ipc_client: Some(MpvJsonIpcClient::new(Box::new(transport))),
            ..Self::default()
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
struct MpvObservedState {
    path: Option<String>,
    duration_seconds: Option<f64>,
    size_bytes: Option<u64>,
    paused: Option<bool>,
    position_seconds: Option<f64>,
    playback_rate: Option<f64>,
}

impl PlayerAdapter for MpvAdapter {
    fn name(&self) -> &'static str {
        "mpv"
    }

    fn open_file(&mut self, path: &str) -> Result<(), PlayerError> {
        self.send_ipc_command_if_attached(json!([
            MPV_COMMAND_LOADFILE,
            path,
            MPV_LOADFILE_REPLACE
        ]))?;
        self.current_path = Some(path.to_owned());
        self.pending_local_file_update = Some(Self::local_file_update_for_path(path));
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
        self.ensure_observers_registered_if_attached();
        self.drain_ipc_events_if_attached();
        self.pending_playback_telemetry_update.take()
    }
}

trait MpvJsonIpcTransport: Send + Sync {
    fn send_line(&mut self, line: &str) -> io::Result<()>;
    fn read_line(&mut self, line: &mut String) -> io::Result<usize>;
}

struct MpvJsonIpcClient {
    transport: Box<dyn MpvJsonIpcTransport>,
    next_request_id: u64,
    pending_events: VecDeque<Value>,
}

impl MpvJsonIpcClient {
    fn new(transport: Box<dyn MpvJsonIpcTransport>) -> Self {
        Self {
            transport,
            next_request_id: 1,
            pending_events: VecDeque::new(),
        }
    }

    fn connect(path: &Path) -> Result<Self, String> {
        let transport = MpvPipeTransport::connect(path)
            .map_err(|err| format!("failed to connect mpv IPC at {}: {err}", path.display()))?;
        Ok(Self::new(Box::new(transport)))
    }

    fn send_command_expect_success(&mut self, command: Value) -> Result<(), String> {
        self.send_command(command).map(|_| ())
    }

    fn observe_property(&mut self, observer_id: u64, property_name: &str) -> Result<(), String> {
        self.send_command_expect_success(json!([
            MPV_COMMAND_OBSERVE_PROPERTY,
            observer_id,
            property_name
        ]))
    }

    fn get_property(&mut self, property_name: &str) -> Result<Option<Value>, String> {
        let response = self.send_command(json!([MPV_COMMAND_GET_PROPERTY, property_name]))?;
        Ok(response
            .get("data")
            .cloned()
            .filter(|value| !value.is_null()))
    }

    fn get_property_string(&mut self, property_name: &str) -> Result<Option<String>, String> {
        let value = self.get_property(property_name)?;
        Ok(value
            .as_ref()
            .and_then(Value::as_str)
            .map(ToOwned::to_owned))
    }

    fn get_property_f64(&mut self, property_name: &str) -> Result<Option<f64>, String> {
        let value = self.get_property(property_name)?;
        Ok(value.as_ref().and_then(Value::as_f64))
    }

    fn get_property_u64(&mut self, property_name: &str) -> Result<Option<u64>, String> {
        let value = self.get_property(property_name)?;
        Ok(value
            .as_ref()
            .and_then(|value| value.as_u64().or_else(|| value.as_i64()?.try_into().ok())))
    }

    fn send_command(&mut self, command: Value) -> Result<Value, String> {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);

        let request = json!({
            "command": command,
            "request_id": request_id,
        });
        let mut line = serde_json::to_string(&request)
            .map_err(|err| format!("failed to serialize mpv IPC request: {err}"))?;
        line.push('\n');
        self.transport
            .send_line(&line)
            .map_err(|err| format!("failed to write mpv IPC request: {err}"))?;

        let mut response_line = String::new();
        loop {
            let bytes_read = self
                .transport
                .read_line(&mut response_line)
                .map_err(|err| format!("failed to read mpv IPC response: {err}"))?;
            if bytes_read == 0 {
                return Err(format!(
                    "unexpected EOF while waiting for mpv IPC response (request_id={request_id})"
                ));
            }

            let trimmed = response_line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                continue;
            }

            let parsed: Value = serde_json::from_str(trimmed)
                .map_err(|err| format!("invalid mpv IPC JSON line '{trimmed}': {err}"))?;
            if parsed.get("event").and_then(Value::as_str).is_some() {
                self.pending_events.push_back(parsed);
                continue;
            }
            let Some(parsed_request_id) = parsed.get("request_id").and_then(Value::as_u64) else {
                // Ignore non-event lines without request_id while waiting for the response.
                continue;
            };
            if parsed_request_id != request_id {
                continue;
            }

            let error = parsed
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("<missing error>");
            if error != MPV_RESPONSE_SUCCESS {
                return Err(format!(
                    "mpv command failed for request_id={request_id}: {error}"
                ));
            }

            return Ok(parsed);
        }
    }

    fn take_pending_events(&mut self) -> Vec<Value> {
        self.pending_events.drain(..).collect()
    }
}

struct MpvPipeTransport {
    stream: MpvPipeStream,
}

impl MpvPipeTransport {
    fn connect(path: &Path) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::net::UnixStream;
            let stream = UnixStream::connect(path)?;
            return Ok(Self {
                stream: MpvPipeStream::Unix(stream),
            });
        }

        #[cfg(windows)]
        {
            let stream = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)?;
            return Ok(Self {
                stream: MpvPipeStream::Windows(stream),
            });
        }

        #[allow(unreachable_code)]
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "mpv IPC transport not implemented for this platform",
        ))
    }
}

impl MpvJsonIpcTransport for MpvPipeTransport {
    fn send_line(&mut self, line: &str) -> io::Result<()> {
        match &mut self.stream {
            #[cfg(unix)]
            MpvPipeStream::Unix(stream) => write_line_to_stream(stream, line),
            #[cfg(windows)]
            MpvPipeStream::Windows(stream) => write_line_to_stream(stream, line),
        }
    }

    fn read_line(&mut self, line: &mut String) -> io::Result<usize> {
        match &mut self.stream {
            #[cfg(unix)]
            MpvPipeStream::Unix(stream) => read_line_from_stream(stream, line),
            #[cfg(windows)]
            MpvPipeStream::Windows(stream) => read_line_from_stream(stream, line),
        }
    }
}

enum MpvPipeStream {
    #[cfg(unix)]
    Unix(std::os::unix::net::UnixStream),
    #[cfg(windows)]
    Windows(std::fs::File),
}

fn write_line_to_stream(stream: &mut impl Write, line: &str) -> io::Result<()> {
    stream.write_all(line.as_bytes())?;
    stream.flush()
}

fn read_line_from_stream(stream: &mut impl Read, line: &mut String) -> io::Result<usize> {
    let mut bytes = Vec::new();
    loop {
        let mut one = [0_u8; 1];
        match stream.read(&mut one) {
            Ok(0) => {
                if bytes.is_empty() {
                    line.clear();
                    return Ok(0);
                }
                break;
            }
            Ok(_) => {
                bytes.push(one[0]);
                if one[0] == b'\n' {
                    break;
                }
            }
            Err(err) => return Err(err),
        }
    }

    let decoded = String::from_utf8(bytes).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("mpv IPC response was not valid UTF-8: {err}"),
        )
    })?;
    line.clear();
    line.push_str(&decoded);
    Ok(line.len())
}

#[cfg(test)]
mod tests {
    use super::{LegacySyncplayOsdKind, LegacySyncplayUiSettings, MpvAdapter, MpvJsonIpcTransport};
    use serde_json::{Value, json};
    use std::{
        collections::VecDeque,
        fs::File,
        io,
        io::Write,
        sync::{Arc, Mutex},
    };
    use syncplay_player_api::{LocalFileUpdate, PlayerAdapter, PlayerPlaybackTelemetryUpdate};

    #[test]
    fn stores_opened_file_path() {
        let mut adapter = MpvAdapter::default();
        adapter
            .open_file("movie.mkv")
            .expect("mpv stub should accept file");
        assert_eq!(adapter.current_path(), Some("movie.mkv"));

        let file_update = adapter
            .take_local_file_update()
            .expect("open file should produce local file update");
        assert_eq!(file_update.name, "movie.mkv");
        assert_eq!(file_update.path.as_deref(), Some("movie.mkv"));
    }

    #[test]
    fn stores_runtime_state_updates() {
        let mut adapter = MpvAdapter::default();
        adapter
            .set_paused(true)
            .expect("mpv stub should accept paused updates");
        adapter
            .set_position(24.5)
            .expect("mpv stub should accept position updates");
        adapter
            .set_playback_rate(0.95)
            .expect("mpv stub should accept speed updates");
        adapter
            .set_muted(true)
            .expect("mpv stub should accept mute updates");
        adapter
            .set_volume(50.0)
            .expect("mpv stub should accept volume updates");
        adapter
            .set_deinterlace(true)
            .expect("mpv stub should accept deinterlace updates");
        adapter
            .set_keepaspect(true)
            .expect("mpv stub should accept keepaspect updates");
        adapter
            .set_keepaspect_window(true)
            .expect("mpv stub should accept keepaspect-window updates");
        adapter
            .set_fullscreen(true)
            .expect("mpv stub should accept fullscreen updates");
        adapter
            .set_ontop(true)
            .expect("mpv stub should accept ontop updates");
        adapter
            .set_border(true)
            .expect("mpv stub should accept border updates");
        adapter
            .set_force_window(true)
            .expect("mpv stub should accept force-window updates");
        adapter
            .set_keep_open(true)
            .expect("mpv stub should accept keep-open updates");
        adapter
            .set_keep_open_pause(true)
            .expect("mpv stub should accept keep-open-pause updates");
        adapter
            .set_cursor_autohide_fs_only(true)
            .expect("mpv stub should accept cursor-autohide-fs-only updates");
        adapter
            .set_stop_screensaver(true)
            .expect("mpv stub should accept stop-screensaver updates");
        adapter
            .set_sub_visibility(true)
            .expect("mpv stub should accept sub-visibility updates");
        adapter
            .set_osd_bar(true)
            .expect("mpv stub should accept osd-bar updates");
        adapter
            .set_window_maximized(true)
            .expect("mpv stub should accept window-maximized updates");
        adapter
            .set_window_minimized(true)
            .expect("mpv stub should accept window-minimized updates");

        assert!(adapter.paused());
        assert_eq!(adapter.position_seconds(), 24.5);
        assert_eq!(adapter.playback_rate(), 0.95);
        assert!(adapter.muted());
        assert_eq!(adapter.volume(), 50.0);
        assert!(adapter.deinterlace());
        assert!(adapter.keepaspect());
        assert!(adapter.keepaspect_window());
        assert!(adapter.fullscreen());
        assert!(adapter.ontop());
        assert!(adapter.border());
        assert!(adapter.force_window());
        assert!(adapter.keep_open());
        assert!(adapter.keep_open_pause());
        assert!(adapter.cursor_autohide_fs_only());
        assert!(adapter.stop_screensaver());
        assert!(adapter.sub_visibility());
        assert!(adapter.osd_bar());
        assert!(adapter.window_maximized());
        assert!(adapter.window_minimized());
    }

    #[test]
    fn queue_local_file_update_is_drained_once() {
        let mut adapter = MpvAdapter::default();
        adapter.queue_local_file_update(
            LocalFileUpdate::new("movie.mkv")
                .with_duration_seconds(95.5)
                .with_size_bytes(123),
        );

        let first = adapter
            .take_local_file_update()
            .expect("queued local file update should be returned");
        assert_eq!(first.name, "movie.mkv");
        assert_eq!(first.duration_seconds, Some(95.5));
        assert_eq!(first.size_bytes, Some(123));
        assert_eq!(adapter.take_local_file_update(), None);
    }

    #[test]
    fn open_file_collects_filesystem_size_for_local_paths() {
        let temp_path = std::env::temp_dir().join("syncplay_mpv_adapter_size_probe.tmp");
        let mut temp_file = File::create(&temp_path).expect("temp file should be creatable");
        writeln!(temp_file, "12345").expect("temp file should be writable");
        drop(temp_file);

        let mut adapter = MpvAdapter::default();
        adapter
            .open_file(temp_path.to_string_lossy().as_ref())
            .expect("mpv stub should accept local temp file");

        let file_update = adapter
            .take_local_file_update()
            .expect("open file should queue local file metadata update");
        assert_eq!(
            file_update.path.as_deref(),
            Some(temp_path.to_string_lossy().as_ref())
        );
        assert!(
            file_update.size_bytes.is_some_and(|size| size >= 6),
            "expected local file metadata size"
        );

        std::fs::remove_file(temp_path).expect("temp file should be removable");
    }

    #[test]
    fn set_paused_sends_json_ipc_set_property_command_when_attached() {
        let (transport, state) =
            fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        adapter
            .set_paused(true)
            .expect("attached mpv transport should accept pause command");

        let writes = state.writes();
        assert_eq!(writes.len(), 1);
        let sent = &writes[0];
        assert!(sent.ends_with('\n'), "expected newline-delimited mpv IPC");
        let payload: Value = serde_json::from_str(sent.trim_end()).expect("valid json");
        assert_eq!(
            payload,
            json!({
                "command": ["set_property", "pause", true],
                "request_id": 1
            })
        );
        assert!(adapter.paused());
    }

    #[test]
    fn set_muted_sends_json_ipc_set_property_command_when_attached() {
        let (transport, state) =
            fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        adapter
            .set_muted(true)
            .expect("attached mpv transport should accept mute command");

        let writes = state.writes();
        assert_eq!(writes.len(), 1);
        let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
        assert_eq!(
            payload,
            json!({
                "command": ["set_property", "mute", true],
                "request_id": 1
            })
        );
        assert!(adapter.muted());
    }

    #[test]
    fn set_volume_sends_json_ipc_set_property_command_when_attached() {
        let (transport, state) =
            fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        adapter
            .set_volume(33.5)
            .expect("attached mpv transport should accept volume command");

        let writes = state.writes();
        assert_eq!(writes.len(), 1);
        let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
        assert_eq!(
            payload,
            json!({
                "command": ["set_property", "volume", 33.5],
                "request_id": 1
            })
        );
        assert_eq!(adapter.volume(), 33.5);
    }

    #[test]
    fn set_fullscreen_sends_json_ipc_set_property_command_when_attached() {
        let (transport, state) =
            fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        adapter
            .set_fullscreen(true)
            .expect("attached mpv transport should accept fullscreen command");

        let writes = state.writes();
        assert_eq!(writes.len(), 1);
        let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
        assert_eq!(
            payload,
            json!({
                "command": ["set_property", "fullscreen", true],
                "request_id": 1
            })
        );
        assert!(adapter.fullscreen());
    }

    #[test]
    fn set_ontop_sends_json_ipc_set_property_command_when_attached() {
        let (transport, state) =
            fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        adapter
            .set_ontop(true)
            .expect("attached mpv transport should accept ontop command");

        let writes = state.writes();
        assert_eq!(writes.len(), 1);
        let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
        assert_eq!(
            payload,
            json!({
                "command": ["set_property", "ontop", true],
                "request_id": 1
            })
        );
        assert!(adapter.ontop());
    }

    #[test]
    fn set_border_sends_json_ipc_set_property_command_when_attached() {
        let (transport, state) =
            fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        adapter
            .set_border(true)
            .expect("attached mpv transport should accept border command");

        let writes = state.writes();
        assert_eq!(writes.len(), 1);
        let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
        assert_eq!(
            payload,
            json!({
                "command": ["set_property", "border", true],
                "request_id": 1
            })
        );
        assert!(adapter.border());
    }

    #[test]
    fn set_keep_open_sends_json_ipc_set_property_command_when_attached() {
        let (transport, state) =
            fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        adapter
            .set_keep_open(true)
            .expect("attached mpv transport should accept keep-open command");

        let writes = state.writes();
        assert_eq!(writes.len(), 1);
        let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
        assert_eq!(
            payload,
            json!({
                "command": ["set_property", "keep-open", true],
                "request_id": 1
            })
        );
        assert!(adapter.keep_open());
    }

    #[test]
    fn set_force_window_sends_json_ipc_set_property_command_when_attached() {
        let (transport, state) =
            fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        adapter
            .set_force_window(true)
            .expect("attached mpv transport should accept force-window command");

        let writes = state.writes();
        assert_eq!(writes.len(), 1);
        let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
        assert_eq!(
            payload,
            json!({
                "command": ["set_property", "force-window", true],
                "request_id": 1
            })
        );
        assert!(adapter.force_window());
    }

    #[test]
    fn set_deinterlace_sends_json_ipc_set_property_command_when_attached() {
        let (transport, state) =
            fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        adapter
            .set_deinterlace(true)
            .expect("attached mpv transport should accept deinterlace command");

        let writes = state.writes();
        assert_eq!(writes.len(), 1);
        let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
        assert_eq!(
            payload,
            json!({
                "command": ["set_property", "deinterlace", true],
                "request_id": 1
            })
        );
        assert!(adapter.deinterlace());
    }

    #[test]
    fn set_keepaspect_sends_json_ipc_set_property_command_when_attached() {
        let (transport, state) =
            fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        adapter
            .set_keepaspect(true)
            .expect("attached mpv transport should accept keepaspect command");

        let writes = state.writes();
        assert_eq!(writes.len(), 1);
        let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
        assert_eq!(
            payload,
            json!({
                "command": ["set_property", "keepaspect", true],
                "request_id": 1
            })
        );
        assert!(adapter.keepaspect());
    }

    #[test]
    fn set_keepaspect_window_sends_json_ipc_set_property_command_when_attached() {
        let (transport, state) =
            fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        adapter
            .set_keepaspect_window(true)
            .expect("attached mpv transport should accept keepaspect-window command");

        let writes = state.writes();
        assert_eq!(writes.len(), 1);
        let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
        assert_eq!(
            payload,
            json!({
                "command": ["set_property", "keepaspect-window", true],
                "request_id": 1
            })
        );
        assert!(adapter.keepaspect_window());
    }

    #[test]
    fn set_keep_open_pause_sends_json_ipc_set_property_command_when_attached() {
        let (transport, state) =
            fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        adapter
            .set_keep_open_pause(true)
            .expect("attached mpv transport should accept keep-open-pause command");

        let writes = state.writes();
        assert_eq!(writes.len(), 1);
        let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
        assert_eq!(
            payload,
            json!({
                "command": ["set_property", "keep-open-pause", true],
                "request_id": 1
            })
        );
        assert!(adapter.keep_open_pause());
    }

    #[test]
    fn set_cursor_autohide_fs_only_sends_json_ipc_set_property_command_when_attached() {
        let (transport, state) =
            fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        adapter
            .set_cursor_autohide_fs_only(true)
            .expect("attached mpv transport should accept cursor-autohide-fs-only command");

        let writes = state.writes();
        assert_eq!(writes.len(), 1);
        let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
        assert_eq!(
            payload,
            json!({
                "command": ["set_property", "cursor-autohide-fs-only", true],
                "request_id": 1
            })
        );
        assert!(adapter.cursor_autohide_fs_only());
    }

    #[test]
    fn set_stop_screensaver_sends_json_ipc_set_property_command_when_attached() {
        let (transport, state) =
            fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        adapter
            .set_stop_screensaver(true)
            .expect("attached mpv transport should accept stop-screensaver command");

        let writes = state.writes();
        assert_eq!(writes.len(), 1);
        let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
        assert_eq!(
            payload,
            json!({
                "command": ["set_property", "stop-screensaver", true],
                "request_id": 1
            })
        );
        assert!(adapter.stop_screensaver());
    }

    #[test]
    fn set_sub_visibility_sends_json_ipc_set_property_command_when_attached() {
        let (transport, state) =
            fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        adapter
            .set_sub_visibility(true)
            .expect("attached mpv transport should accept sub-visibility command");

        let writes = state.writes();
        assert_eq!(writes.len(), 1);
        let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
        assert_eq!(
            payload,
            json!({
                "command": ["set_property", "sub-visibility", true],
                "request_id": 1
            })
        );
        assert!(adapter.sub_visibility());
    }

    #[test]
    fn set_osd_bar_sends_json_ipc_set_property_command_when_attached() {
        let (transport, state) =
            fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        adapter
            .set_osd_bar(true)
            .expect("attached mpv transport should accept osd-bar command");

        let writes = state.writes();
        assert_eq!(writes.len(), 1);
        let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
        assert_eq!(
            payload,
            json!({
                "command": ["set_property", "osd-bar", true],
                "request_id": 1
            })
        );
        assert!(adapter.osd_bar());
    }

    #[test]
    fn set_window_maximized_sends_json_ipc_set_property_command_when_attached() {
        let (transport, state) =
            fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        adapter
            .set_window_maximized(true)
            .expect("attached mpv transport should accept window-maximized command");

        let writes = state.writes();
        assert_eq!(writes.len(), 1);
        let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
        assert_eq!(
            payload,
            json!({
                "command": ["set_property", "window-maximized", true],
                "request_id": 1
            })
        );
        assert!(adapter.window_maximized());
    }

    #[test]
    fn set_window_minimized_sends_json_ipc_set_property_command_when_attached() {
        let (transport, state) =
            fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        adapter
            .set_window_minimized(true)
            .expect("attached mpv transport should accept window-minimized command");

        let writes = state.writes();
        assert_eq!(writes.len(), 1);
        let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
        assert_eq!(
            payload,
            json!({
                "command": ["set_property", "window-minimized", true],
                "request_id": 1
            })
        );
        assert!(adapter.window_minimized());
    }

    #[test]
    fn set_position_waits_for_matching_response_and_ignores_async_events() {
        let (transport, state) = fake_transport_with_reads(&[
            r#"{"event":"property-change","name":"pause","data":false}"#,
            r#"{"request_id":999,"error":"success"}"#,
            r#"{"request_id":1,"error":"success"}"#,
        ]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        adapter
            .set_position(24.5)
            .expect("attached mpv transport should accept seek command");

        let writes = state.writes();
        assert_eq!(writes.len(), 1);
        let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
        assert_eq!(
            payload,
            json!({
                "command": ["set_property", "time-pos", 24.5],
                "request_id": 1
            })
        );
        assert_eq!(adapter.position_seconds(), 24.5);
    }

    #[test]
    fn mpv_error_response_is_reported_and_local_state_is_not_updated() {
        let (transport, _state) =
            fake_transport_with_reads(&[r#"{"request_id":1,"error":"property unavailable"}"#]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        let err = adapter
            .set_position(42.0)
            .expect_err("mpv error response should fail operation");
        match err {
            syncplay_player_api::PlayerError::OperationFailed(message) => {
                assert!(
                    message.contains("property unavailable"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
        assert_eq!(adapter.position_seconds(), 0.0);
    }

    #[test]
    fn open_file_sends_mpv_loadfile_replace_command_when_attached() {
        let (transport, state) =
            fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        adapter
            .open_file("movie.mkv")
            .expect("attached mpv transport should accept loadfile");

        let writes = state.writes();
        let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
        assert_eq!(
            payload,
            json!({
                "command": ["loadfile", "movie.mkv", "replace"],
                "request_id": 1
            })
        );
    }

    #[test]
    fn set_option_string_sends_json_ipc_set_command_when_attached() {
        let (transport, state) =
            fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        adapter
            .set_option_string("script-opts", "osc=no")
            .expect("attached mpv transport should accept generic option updates");

        let writes = state.writes();
        let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
        assert_eq!(
            payload,
            json!({
                "command": ["set", "script-opts", "osc=no"],
                "request_id": 1
            })
        );
    }

    #[test]
    fn apply_profile_sends_json_ipc_command_when_attached() {
        let (transport, state) =
            fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        adapter
            .apply_profile("fast")
            .expect("attached mpv transport should accept apply-profile");

        let writes = state.writes();
        let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
        assert_eq!(
            payload,
            json!({
                "command": ["apply-profile", "fast"],
                "request_id": 1
            })
        );
    }

    #[test]
    fn show_text_sends_json_ipc_command_when_attached() {
        let (transport, state) =
            fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        adapter
            .show_text("syncplay notice", 4_000, 1)
            .expect("attached mpv transport should accept show-text");

        let writes = state.writes();
        let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
        assert_eq!(
            payload,
            json!({
                "command": ["show-text", "syncplay notice", 4_000, 1],
                "request_id": 1
            })
        );
    }

    #[test]
    fn load_legacy_syncplayintf_script_sends_load_script_and_option_message_when_attached() {
        let (transport, state) = fake_transport_with_reads(&[
            r#"{"request_id":1,"error":"success"}"#,
            r#"{"request_id":2,"error":"success"}"#,
        ]);
        let mut adapter = MpvAdapter::with_test_transport(transport);
        adapter
            .configure_legacy_syncplay_ui_settings(LegacySyncplayUiSettings {
                chat_input_enabled: true,
                chat_input_font_underline: true,
                chat_input_font_family: "serif".to_owned(),
                chat_input_relative_font_size: 18,
                chat_input_font_weight: 50,
                chat_input_font_color: "#abcdef".to_owned(),
                chat_input_position: "Bottom".to_owned(),
                chat_direct_input: true,
                chat_output_font_underline: true,
                chat_output_font_family: "monospace".to_owned(),
                chat_output_relative_font_size: 30,
                chat_output_font_weight: 75,
                chat_output_mode: "Scrolling".to_owned(),
                chat_max_lines: 9,
                chat_top_margin: 40,
                chat_left_margin: 35,
                chat_bottom_margin: 45,
                chat_move_osd: false,
                notification_timeout_ms: 4_000,
                alert_timeout_ms: 6_000,
                chat_timeout_ms: 8_000,
                ..LegacySyncplayUiSettings::default()
            })
            .expect("legacy settings application should succeed");

        adapter
            .load_legacy_syncplayintf_script(std::path::Path::new("C:/syncplay/syncplayintf.lua"))
            .expect("attached mpv transport should accept load-script");

        let writes = state.writes();
        assert_eq!(writes.len(), 2);
        let first_payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
        let second_payload: Value = serde_json::from_str(writes[1].trim_end()).expect("valid json");
        assert_eq!(
            first_payload,
            json!({
                "command": ["load-script", "C:/syncplay/syncplayintf.lua"],
                "request_id": 1
            })
        );
        assert_eq!(
            second_payload["command"][0],
            Value::String("script-message-to".to_owned())
        );
        assert_eq!(
            second_payload["command"][1],
            Value::String("syncplayintf".to_owned())
        );
        assert_eq!(
            second_payload["command"][2],
            Value::String("set_syncplayintf_options".to_owned())
        );
        let options = second_payload["command"][3]
            .as_str()
            .expect("syncplayintf options should be a string");
        assert!(options.contains("chatInputEnabled=False"));
        assert!(options.contains("chatInputFontUnderline=True"));
        assert!(options.contains("chatInputFontFamily=serif"));
        assert!(options.contains("chatInputRelativeFontSize=18"));
        assert!(options.contains("chatInputFontWeight=50"));
        assert!(options.contains("chatInputFontColor=#abcdef"));
        assert!(options.contains("chatInputPosition=Bottom"));
        assert!(options.contains("chatOutputFontUnderline=True"));
        assert!(options.contains("chatOutputFontFamily=monospace"));
        assert!(options.contains("chatOutputRelativeFontSize=30"));
        assert!(options.contains("chatOutputFontWeight=75"));
        assert!(options.contains("chatOutputMode=Scrolling"));
        assert!(options.contains("chatMaxLines=9"));
        assert!(options.contains("chatTopMargin=40"));
        assert!(options.contains("chatLeftMargin=35"));
        assert!(options.contains("chatBottomMargin=45"));
        assert!(options.contains("chatDirectInput=False"));
        assert!(options.contains("notificationTimeout=4"));
        assert!(options.contains("alertTimeout=6"));
        assert!(options.contains("chatTimeout=8"));
        assert!(options.contains("chatOutputEnabled=True"));
    }

    #[test]
    fn configure_legacy_syncplay_ui_settings_applies_osd_position_when_needed() {
        let (transport, state) = fake_transport_with_reads(&[
            r#"{"request_id":1,"error":"success"}"#,
            r#"{"request_id":2,"error":"success"}"#,
        ]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        adapter
            .configure_legacy_syncplay_ui_settings(LegacySyncplayUiSettings::default())
            .expect("legacy settings application should succeed");

        let writes = state.writes();
        assert_eq!(writes.len(), 2);
        let first_payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
        let second_payload: Value = serde_json::from_str(writes[1].trim_end()).expect("valid json");
        assert_eq!(
            first_payload,
            json!({
                "command": ["set_property", "osd-align-y", "bottom"],
                "request_id": 1
            })
        );
        assert_eq!(
            second_payload,
            json!({
                "command": ["set_property", "osd-margin-y", 110],
                "request_id": 2
            })
        );
        assert_eq!(
            adapter.legacy_syncplay_ui_settings(),
            &LegacySyncplayUiSettings::default()
        );
    }

    #[test]
    fn configure_legacy_syncplay_ui_settings_skips_osd_position_when_disabled() {
        let (transport, state) = fake_transport_with_reads(&[]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        adapter
            .configure_legacy_syncplay_ui_settings(LegacySyncplayUiSettings {
                chat_move_osd: false,
                ..LegacySyncplayUiSettings::default()
            })
            .expect("legacy settings application should succeed");

        assert!(state.writes().is_empty());
    }

    #[test]
    fn show_syncplay_legacy_message_uses_script_message_when_syncplayintf_is_loaded() {
        let (transport, state) = fake_transport_with_reads(&[
            r#"{"request_id":1,"error":"success"}"#,
            r#"{"request_id":2,"error":"success"}"#,
            r#"{"request_id":3,"error":"success"}"#,
        ]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        adapter
            .load_legacy_syncplayintf_script(std::path::Path::new("C:/syncplay/syncplayintf.lua"))
            .expect("attached mpv transport should accept load-script");

        adapter
            .show_syncplay_legacy_message("room updated", LegacySyncplayOsdKind::Notification)
            .expect("syncplayintf notification should succeed");

        let writes = state.writes();
        assert_eq!(writes.len(), 3);
        let payload: Value = serde_json::from_str(writes[2].trim_end()).expect("valid json");
        assert_eq!(
            payload,
            json!({
                "command": ["script-message-to", "syncplayintf", "notification-osd-neutral", "room updated"],
                "request_id": 3
            })
        );
    }

    #[test]
    fn show_syncplay_legacy_message_uses_notification_timeout_when_osd_is_enabled() {
        let (transport, state) =
            fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
        let mut adapter = MpvAdapter::with_test_transport(transport);
        adapter
            .configure_legacy_syncplay_ui_settings(LegacySyncplayUiSettings {
                chat_move_osd: false,
                notification_timeout_ms: 4_500,
                ..LegacySyncplayUiSettings::default()
            })
            .expect("legacy settings application should succeed");

        adapter
            .show_syncplay_legacy_message("room updated", LegacySyncplayOsdKind::Notification)
            .expect("show-text notification should succeed");

        let writes = state.writes();
        assert_eq!(writes.len(), 1);
        let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
        assert_eq!(
            payload,
            json!({
                "command": ["show-text", "room updated", 4_500, 1],
                "request_id": 1
            })
        );
    }

    #[test]
    fn show_syncplay_legacy_message_uses_alert_timeout_when_requested() {
        let (transport, state) =
            fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
        let mut adapter = MpvAdapter::with_test_transport(transport);
        adapter
            .configure_legacy_syncplay_ui_settings(LegacySyncplayUiSettings {
                chat_move_osd: false,
                alert_timeout_ms: 6_000,
                ..LegacySyncplayUiSettings::default()
            })
            .expect("legacy settings application should succeed");

        adapter
            .show_syncplay_legacy_message("autoplay", LegacySyncplayOsdKind::Alert)
            .expect("show-text alert should succeed");

        let writes = state.writes();
        assert_eq!(writes.len(), 1);
        let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
        assert_eq!(
            payload,
            json!({
                "command": ["show-text", "autoplay", 6_000, 1],
                "request_id": 1
            })
        );
    }

    #[test]
    fn show_syncplay_legacy_chat_message_uses_script_message_when_syncplayintf_is_loaded() {
        let (transport, state) = fake_transport_with_reads(&[
            r#"{"request_id":1,"error":"success"}"#,
            r#"{"request_id":2,"error":"success"}"#,
            r#"{"request_id":3,"error":"success"}"#,
        ]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        adapter
            .load_legacy_syncplayintf_script(std::path::Path::new("C:/syncplay/syncplayintf.lua"))
            .expect("attached mpv transport should accept load-script");

        adapter
            .show_syncplay_legacy_chat_message("<alice> hi")
            .expect("syncplayintf chat should succeed");

        let writes = state.writes();
        assert_eq!(writes.len(), 3);
        let payload: Value = serde_json::from_str(writes[2].trim_end()).expect("valid json");
        assert_eq!(
            payload,
            json!({
                "command": ["script-message-to", "syncplayintf", "chat", "<alice> hi"],
                "request_id": 3
            })
        );
    }

    #[test]
    fn show_syncplay_legacy_chat_message_uses_chat_timeout_even_when_show_osd_is_disabled() {
        let (transport, state) =
            fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
        let mut adapter = MpvAdapter::with_test_transport(transport);
        adapter
            .configure_legacy_syncplay_ui_settings(LegacySyncplayUiSettings {
                show_osd: false,
                chat_move_osd: false,
                chat_timeout_ms: 8_000,
                ..LegacySyncplayUiSettings::default()
            })
            .expect("legacy settings application should succeed");

        adapter
            .show_syncplay_legacy_chat_message("<alice> hi")
            .expect("chat show-text should succeed");

        let writes = state.writes();
        assert_eq!(writes.len(), 1);
        let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
        assert_eq!(
            payload,
            json!({
                "command": ["show-text", "<alice> hi", 8_000, 1],
                "request_id": 1
            })
        );
    }

    #[test]
    fn show_syncplay_legacy_chat_message_falls_back_to_notification_timeout_when_chat_output_is_disabled()
     {
        let (transport, state) =
            fake_transport_with_reads(&[r#"{"request_id":1,"error":"success"}"#]);
        let mut adapter = MpvAdapter::with_test_transport(transport);
        adapter
            .configure_legacy_syncplay_ui_settings(LegacySyncplayUiSettings {
                chat_output_enabled: false,
                chat_move_osd: false,
                notification_timeout_ms: 2_500,
                ..LegacySyncplayUiSettings::default()
            })
            .expect("legacy settings application should succeed");

        adapter
            .show_syncplay_legacy_chat_message("<alice> hi")
            .expect("chat fallback show-text should succeed");

        let writes = state.writes();
        assert_eq!(writes.len(), 1);
        let payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
        assert_eq!(
            payload,
            json!({
                "command": ["show-text", "<alice> hi", 2_500, 1],
                "request_id": 1
            })
        );
    }

    #[test]
    fn take_local_file_update_polls_mpv_properties_and_emits_changes_once() {
        let (transport, state) = fake_transport_with_reads(&[
            r#"{"request_id":1,"error":"success"}"#,
            r#"{"request_id":2,"error":"success"}"#,
            r#"{"request_id":3,"error":"success"}"#,
            r#"{"request_id":4,"error":"success"}"#,
            r#"{"request_id":5,"error":"success"}"#,
            r#"{"request_id":6,"error":"success"}"#,
            r#"{"request_id":7,"error":"success","data":"C:/media/movie.mkv"}"#,
            r#"{"request_id":8,"error":"success","data":1439.5}"#,
            r#"{"request_id":9,"error":"success","data":123456}"#,
            r#"{"request_id":10,"error":"success","data":"C:/media/movie.mkv"}"#,
            r#"{"request_id":11,"error":"success","data":1439.5}"#,
            r#"{"request_id":12,"error":"success","data":123456}"#,
        ]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        let first = adapter
            .take_local_file_update()
            .expect("first poll should emit local file update");
        assert_eq!(first.name, "movie.mkv");
        assert_eq!(first.path.as_deref(), Some("C:/media/movie.mkv"));
        assert_eq!(first.duration_seconds, Some(1439.5));
        assert_eq!(first.size_bytes, Some(123456));

        assert_eq!(
            adapter.take_local_file_update(),
            None,
            "unchanged telemetry should not re-emit a file update"
        );

        let writes = state.writes();
        assert_eq!(writes.len(), 12);
        let first_payload: Value = serde_json::from_str(writes[0].trim_end()).expect("valid json");
        let second_payload: Value = serde_json::from_str(writes[1].trim_end()).expect("valid json");
        let third_payload: Value = serde_json::from_str(writes[2].trim_end()).expect("valid json");
        let seventh_payload: Value =
            serde_json::from_str(writes[6].trim_end()).expect("valid json");
        let eighth_payload: Value = serde_json::from_str(writes[7].trim_end()).expect("valid json");
        let ninth_payload: Value = serde_json::from_str(writes[8].trim_end()).expect("valid json");
        assert_eq!(
            first_payload,
            json!({
                "command": ["observe_property", 1, "path"],
                "request_id": 1
            })
        );
        assert_eq!(
            second_payload,
            json!({
                "command": ["observe_property", 2, "duration"],
                "request_id": 2
            })
        );
        assert_eq!(
            third_payload,
            json!({
                "command": ["observe_property", 3, "file-size"],
                "request_id": 3
            })
        );
        assert_eq!(
            seventh_payload,
            json!({
                "command": ["get_property", "path"],
                "request_id": 7
            })
        );
        assert_eq!(
            eighth_payload,
            json!({
                "command": ["get_property", "duration"],
                "request_id": 8
            })
        );
        assert_eq!(
            ninth_payload,
            json!({
                "command": ["get_property", "file-size"],
                "request_id": 9
            })
        );
    }

    #[test]
    fn take_local_file_update_ignores_missing_path_until_file_is_available() {
        let (transport, _state) = fake_transport_with_reads(&[
            r#"{"request_id":1,"error":"success"}"#,
            r#"{"request_id":2,"error":"success"}"#,
            r#"{"request_id":3,"error":"success"}"#,
            r#"{"request_id":4,"error":"success"}"#,
            r#"{"request_id":5,"error":"success"}"#,
            r#"{"request_id":6,"error":"success"}"#,
            r#"{"request_id":7,"error":"property unavailable"}"#,
            r#"{"request_id":8,"error":"success","data":"C:/media/movie2.mkv"}"#,
            r#"{"request_id":9,"error":"success","data":42}"#,
            r#"{"request_id":10,"error":"success","data":1000}"#,
        ]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        assert_eq!(adapter.take_local_file_update(), None);

        let update = adapter
            .take_local_file_update()
            .expect("file should emit after path becomes available");
        assert_eq!(update.name, "movie2.mkv");
        assert_eq!(update.path.as_deref(), Some("C:/media/movie2.mkv"));
        assert_eq!(update.duration_seconds, Some(42.0));
        assert_eq!(update.size_bytes, Some(1000));
    }

    #[test]
    fn async_property_change_events_from_mpv_queue_local_file_update() {
        let (transport, state) = fake_transport_with_reads(&[
            r#"{"request_id":1,"error":"success"}"#,
            r#"{"request_id":2,"error":"success"}"#,
            r#"{"request_id":3,"error":"success"}"#,
            r#"{"request_id":4,"error":"success"}"#,
            r#"{"request_id":5,"error":"success"}"#,
            r#"{"request_id":6,"error":"success"}"#,
            r#"{"request_id":7,"error":"property unavailable"}"#,
            r#"{"event":"property-change","name":"path","data":"C:/media/from-event.mkv"}"#,
            r#"{"event":"property-change","name":"duration","data":120.0}"#,
            r#"{"event":"property-change","name":"file-size","data":987654}"#,
            r#"{"request_id":8,"error":"success"}"#,
        ]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        assert_eq!(adapter.take_local_file_update(), None);

        adapter.set_paused(true).expect("command should succeed");

        let update = adapter
            .take_local_file_update()
            .expect("async property-change events should queue a local file update");
        assert_eq!(update.name, "from-event.mkv");
        assert_eq!(update.path.as_deref(), Some("C:/media/from-event.mkv"));
        assert_eq!(update.duration_seconds, Some(120.0));
        assert_eq!(update.size_bytes, Some(987654));

        let writes = state.writes();
        assert_eq!(writes.len(), 8);
        let last_payload: Value = serde_json::from_str(writes[7].trim_end()).expect("valid json");
        assert_eq!(
            last_payload,
            json!({
                "command": ["set_property", "pause", true],
                "request_id": 8
            })
        );
    }

    #[test]
    fn async_property_change_events_queue_playback_telemetry_update() {
        let (transport, _state) = fake_transport_with_reads(&[
            r#"{"request_id":1,"error":"success"}"#,
            r#"{"request_id":2,"error":"success"}"#,
            r#"{"request_id":3,"error":"success"}"#,
            r#"{"request_id":4,"error":"success"}"#,
            r#"{"request_id":5,"error":"success"}"#,
            r#"{"request_id":6,"error":"success"}"#,
            r#"{"request_id":7,"error":"property unavailable"}"#,
            r#"{"event":"property-change","name":"pause","data":true}"#,
            r#"{"event":"property-change","name":"time-pos","data":123.25}"#,
            r#"{"event":"property-change","name":"speed","data":1.10}"#,
            r#"{"request_id":8,"error":"success"}"#,
        ]);
        let mut adapter = MpvAdapter::with_test_transport(transport);

        assert_eq!(adapter.take_local_file_update(), None);

        adapter
            .set_position(10.0)
            .expect("command should drain and process queued events");

        let telemetry = adapter
            .take_playback_telemetry_update()
            .expect("expected merged playback telemetry update from async events");
        assert_eq!(
            telemetry,
            PlayerPlaybackTelemetryUpdate {
                paused: Some(true),
                position_seconds: Some(123.25),
                playback_rate: Some(1.10),
            }
        );
        assert_eq!(adapter.take_playback_telemetry_update(), None);
        assert!(adapter.paused());
        assert_eq!(
            adapter.position_seconds(),
            10.0,
            "commanded local state currently wins over earlier async time-pos event in this slice"
        );
        assert_eq!(adapter.playback_rate(), 1.10);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "local smoke test; requires standalone mpv binary and media file"]
    fn local_standalone_mpv_smoke_reports_file_metadata() {
        use std::{
            path::{Path, PathBuf},
            process::{Child, Command, Stdio},
            thread::sleep,
            time::{Duration, Instant, SystemTime, UNIX_EPOCH},
        };

        fn repo_root() -> PathBuf {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("..")
        }

        fn default_mpv_bin(root: &Path) -> PathBuf {
            root.join("mpv").join("mpv.exe")
        }

        fn first_media_file(media_dir: &Path) -> Option<PathBuf> {
            let mut entries = std::fs::read_dir(media_dir).ok()?;
            while let Some(Ok(entry)) = entries.next() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let ext = path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| ext.to_ascii_lowercase());
                let Some(ext) = ext else { continue };
                if matches!(ext.as_str(), "mkv" | "mp4" | "avi" | "webm" | "mov" | "m4v") {
                    return Some(path);
                }
            }
            None
        }

        fn env_duration_ms(name: &str, default_ms: u64) -> Duration {
            std::env::var(name)
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .map(Duration::from_millis)
                .unwrap_or_else(|| Duration::from_millis(default_ms))
        }

        struct MpvChildGuard(Child);

        impl Drop for MpvChildGuard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        let root = repo_root();
        let mpv_bin = std::env::var_os("SYNCPLAY_MPV_SMOKE_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| default_mpv_bin(&root));
        let media_file = std::env::var_os("SYNCPLAY_MPV_SMOKE_MEDIA")
            .map(PathBuf::from)
            .or_else(|| first_media_file(&root.join("media")))
            .expect("expected media file in ./media or SYNCPLAY_MPV_SMOKE_MEDIA");

        if !mpv_bin.exists() {
            panic!(
                "mpv binary not found at {} (override with SYNCPLAY_MPV_SMOKE_BIN)",
                mpv_bin.display()
            );
        }

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_millis();
        let pipe_path = format!(
            r"\\.\pipe\syncplay-rust-mpv-smoke-{}-{unique}",
            std::process::id()
        );
        let connect_timeout = env_duration_ms("SYNCPLAY_MPV_SMOKE_CONNECT_TIMEOUT_MS", 5_000);
        let metadata_timeout = env_duration_ms("SYNCPLAY_MPV_SMOKE_METADATA_TIMEOUT_MS", 10_000);
        let poll_interval = env_duration_ms("SYNCPLAY_MPV_SMOKE_POLL_INTERVAL_MS", 50);

        let child = MpvChildGuard(
            Command::new(&mpv_bin)
                .current_dir(
                    mpv_bin
                        .parent()
                        .expect("mpv binary path should have a parent directory"),
                )
                .arg("--pause")
                .arg("--force-window=no")
                .arg("--idle=yes")
                .arg(format!("--input-ipc-server={pipe_path}"))
                .arg(&media_file)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("standalone mpv should start for local smoke test"),
        );

        let mut adapter = None;
        let connect_started = Instant::now();
        let mut last_connect_error = None;
        while connect_started.elapsed() < connect_timeout {
            match MpvAdapter::with_json_ipc(&pipe_path) {
                Ok(attached) => {
                    adapter = Some(attached);
                    break;
                }
                Err(err) => {
                    last_connect_error = Some(err.to_string());
                    sleep(poll_interval);
                }
            }
        }
        let mut adapter = match adapter {
            Some(adapter) => adapter,
            None => panic!(
                "expected to connect to mpv JSON IPC pipe within {:?} (pipe={}, mpv_bin={}, media={}); last error: {}",
                connect_timeout,
                pipe_path,
                mpv_bin.display(),
                media_file.display(),
                last_connect_error.as_deref().unwrap_or("<none>")
            ),
        };

        let mut observed_update = None;
        let mut last_update = None;
        let mut last_telemetry = None;
        let metadata_started = Instant::now();
        while metadata_started.elapsed() < metadata_timeout {
            if let Some(update) = adapter.take_local_file_update() {
                last_update = Some(update.clone());
                let has_duration = update
                    .duration_seconds
                    .is_some_and(|duration| duration > 1.0);
                let has_path = update.path.is_some();
                if has_path && has_duration {
                    observed_update = Some(update);
                    break;
                }
            }
            while let Some(telemetry) = adapter.take_playback_telemetry_update() {
                last_telemetry = Some(telemetry);
            }
            sleep(poll_interval);
        }

        drop(child);

        let update = observed_update.unwrap_or_else(|| {
            panic!(
                "expected mpv telemetry-driven LocalFileUpdate within {:?} (poll_interval={:?}, pipe={}, mpv_bin={}, media={}); last_update={:?}; last_telemetry={:?}",
                metadata_timeout,
                poll_interval,
                pipe_path,
                mpv_bin.display(),
                media_file.display(),
                last_update,
                last_telemetry
            )
        });
        let expected_name = media_file
            .file_name()
            .and_then(|name| name.to_str())
            .expect("media file should have a UTF-8 filename");
        assert_eq!(update.name, expected_name);
        assert!(
            update
                .duration_seconds
                .is_some_and(|duration| duration > 60.0),
            "expected realistic media duration from mpv telemetry, got {:?}",
            update.duration_seconds
        );
        assert!(
            update.path.is_some(),
            "expected mpv to report a path for the loaded file"
        );
    }

    fn fake_transport_with_reads(lines: &[&str]) -> (FakeTransport, FakeTransportStateHandle) {
        let shared = Arc::new(Mutex::new(FakeTransportState {
            reads: lines
                .iter()
                .map(|line| {
                    let mut owned = (*line).to_owned();
                    owned.push('\n');
                    owned
                })
                .collect(),
            writes: Vec::new(),
        }));
        (
            FakeTransport {
                shared: Arc::clone(&shared),
            },
            FakeTransportStateHandle { shared },
        )
    }

    #[derive(Debug)]
    struct FakeTransport {
        shared: Arc<Mutex<FakeTransportState>>,
    }

    impl MpvJsonIpcTransport for FakeTransport {
        fn send_line(&mut self, line: &str) -> io::Result<()> {
            self.shared
                .lock()
                .expect("fake transport mutex should not be poisoned")
                .writes
                .push(line.to_owned());
            Ok(())
        }

        fn read_line(&mut self, line: &mut String) -> io::Result<usize> {
            let mut guard = self
                .shared
                .lock()
                .expect("fake transport mutex should not be poisoned");
            let Some(next) = guard.reads.pop_front() else {
                line.clear();
                return Ok(0);
            };
            line.clear();
            line.push_str(&next);
            Ok(line.len())
        }
    }

    #[derive(Debug)]
    struct FakeTransportState {
        reads: VecDeque<String>,
        writes: Vec<String>,
    }

    #[derive(Debug)]
    struct FakeTransportStateHandle {
        shared: Arc<Mutex<FakeTransportState>>,
    }

    impl FakeTransportStateHandle {
        fn writes(&self) -> Vec<String> {
            self.shared
                .lock()
                .expect("fake transport mutex should not be poisoned")
                .writes
                .clone()
        }
    }
}
