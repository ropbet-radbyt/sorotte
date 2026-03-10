use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use syncplay_core::SyncDomain;
use syncplay_player_api::{
    LocalFileUpdate, PlayerAdapter, PlayerError, PlayerPlaybackTelemetryUpdate,
};
use syncplay_protocol::{
    ChatPayload, ControllerAuthPayload, FilePayload, IgnoringOnTheFlyPayload, ListPayload,
    PingPayload, PlaylistChangePayload, PlaylistIndexPayload, PlaystatePayload, ProtocolError,
    ProtocolMessage, ReadyPayload, RoomRef, SetPayload, StatePayload, decode_message_line,
    encode_message_line, extract_hello_from_message,
};

const SEEK_THRESHOLD_SECONDS: f64 = 1.0;
const DEFAULT_REWIND_THRESHOLD_SECONDS: f64 = 4.0;
const DEFAULT_FASTFORWARD_THRESHOLD_SECONDS: f64 = 5.0;
const FASTFORWARD_BEHIND_THRESHOLD_SECONDS: f64 = 1.75;
const FASTFORWARD_EXTRA_SECONDS: f64 = 0.25;
const FASTFORWARD_RESET_THRESHOLD_SECONDS: f64 = 3.0;
const DEFAULT_SLOWDOWN_THRESHOLD_SECONDS: f64 = 1.5;
const SLOWDOWN_RESET_THRESHOLD_SECONDS: f64 = 0.1;
const SLOWDOWN_RATE: f64 = 0.95;
const NORMAL_PLAYBACK_RATE: f64 = 1.0;
const DEFAULT_MAX_RECONNECT_RETRIES: u32 = 999;
const DEFAULT_RECONNECT_BASE_DELAY_SECONDS: f64 = 0.1;
const DEFAULT_RECONNECT_BACKOFF_MAX_EXPONENT: u32 = 5;
const DEFAULT_LAST_PAUSED_DIFF_THRESHOLD_SECONDS: f64 = 2.0;
const DEFAULT_AUTOPLAY_DELAY_SECONDS: f64 = 3.0;
const AUTOPLAY_COUNTDOWN_STEP_SECONDS: f64 = 1.0;
const RECENTLY_ADVANCED_GRACE_SECONDS: f64 = 5.0;
const LEGACY_SHOW_DURATION_NOTIFICATION: bool = true;
const LEGACY_DIFFERENT_DURATION_THRESHOLD_SECONDS: f64 = 2.5;
const LEGACY_CHAT_MAX_MESSAGE_LENGTH: usize = 150;
const LEGACY_FALLBACK_MAX_CHAT_MESSAGE_LENGTH: usize = 50;
const LEGACY_CHAT_MIN_VERSION: &str = "1.5.0";
const LEGACY_SHOW_SAME_ROOM_OSD: bool = true;
const LEGACY_SHOW_OSD_WARNINGS: bool = true;
const LEGACY_SHOW_NONCONTROLLER_OSD: bool = false;
const LEGACY_SHOW_DIFFERENT_ROOM_OSD: bool = false;
const LEGACY_ONLY_SWITCH_TO_TRUSTED_DOMAINS: bool = true;
const LEGACY_DEFAULT_TRUSTED_DOMAINS: [&str; 2] = ["youtube.com", "youtu.be"];
const DEFAULT_RECONNECT_STATE_RESTORE_AUTOCORRECT: bool = true;
const DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RETRY_MAX_ATTEMPTS: u32 = 3;
const DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RETRY_COOLDOWN_TICKS: u32 = 1;
const DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RETRY_EXPONENTIAL_BACKOFF: bool = false;
const DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RETRY_MAX_COOLDOWN_TICKS: u32 = 8;
const DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RETRY_ADAPTIVE_CYCLE_BACKOFF: bool = false;
const DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RETRY_ADAPTIVE_CYCLE_BUDGET: bool = false;
const DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RETRY_ADAPTIVE_CYCLE_BUDGET_MIN_ATTEMPTS: u32 = 0;
const DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_DISABLE_AFTER_MISMATCHES: u32 = 0;
const DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_DISABLE_AFTER_MISMATCH_DECAY_ON_SUCCESS: u32 = 0;
const DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RECOVERY_COOLDOWN_RECONNECT_CYCLES: u32 = 0;
const ROUND_HALF_EPSILON: f64 = 1e-12;
const PRIVACY_HIDDEN_FILENAME: &str = "**Hidden filename**";
const MUSIC_FORMATS: [&str; 8] = [
    ".mp3", ".m4a", ".m4p", ".wav", ".aiff", ".r", ".ogg", ".flac",
];
pub const AUTOPLAY_TICK_INTERVAL_SECONDS: f64 = AUTOPLAY_COUNTDOWN_STEP_SECONDS;
const LEGACY_PING_MOVING_AVERAGE_WEIGHT: f64 = 0.85;

#[derive(Debug, Clone, Copy, Default)]
pub struct ClientPingMetricsLegacyCompatible {
    client_rtt_seconds: f64,
    average_rtt_seconds: f64,
    server_rtt_seconds: f64,
    forward_delay_seconds: f64,
}

impl ClientPingMetricsLegacyCompatible {
    pub fn observe_inbound_state(&mut self, state: &StatePayload) {
        let now_seconds = unix_wall_clock_time_seconds_legacy_compatible();
        self.observe_inbound_state_at(state, now_seconds);
    }

    fn observe_inbound_state_at(&mut self, state: &StatePayload, now_seconds: f64) {
        let Some(ping) = state.ping.as_ref() else {
            return;
        };
        let Some(latency_calculation) = ping.latency_calculation else {
            return;
        };
        let sender_rtt = ping.client_rtt.unwrap_or(0.0);
        if !sender_rtt.is_finite() || sender_rtt < 0.0 {
            return;
        }
        let server_rtt = ping.server_rtt;
        if let Some(server_rtt_value) = server_rtt
            && (!server_rtt_value.is_finite() || server_rtt_value < 0.0)
        {
            return;
        }

        let current_rtt = now_seconds - latency_calculation;
        if !current_rtt.is_finite() || current_rtt < 0.0 {
            return;
        }
        self.client_rtt_seconds = current_rtt;
        if let Some(server_rtt_value) = server_rtt {
            self.server_rtt_seconds = server_rtt_value;
        }
        if self.average_rtt_seconds == 0.0 {
            self.average_rtt_seconds = current_rtt;
        }
        self.average_rtt_seconds = self.average_rtt_seconds * LEGACY_PING_MOVING_AVERAGE_WEIGHT
            + current_rtt * (1.0 - LEGACY_PING_MOVING_AVERAGE_WEIGHT);
        self.forward_delay_seconds = if let Some(server_rtt_value) = server_rtt {
            if server_rtt_value < current_rtt {
                self.average_rtt_seconds / 2.0 + (current_rtt - server_rtt_value)
            } else {
                self.average_rtt_seconds / 2.0
            }
        } else {
            self.average_rtt_seconds / 2.0
        };
    }

    pub fn client_rtt_seconds(self) -> f64 {
        self.client_rtt_seconds
    }

    pub fn server_rtt_seconds(self) -> f64 {
        self.server_rtt_seconds
    }

    pub fn forward_delay_seconds(self) -> f64 {
        self.forward_delay_seconds
    }

    pub fn client_latency_calculation_now(self) -> f64 {
        let _ = self;
        unix_wall_clock_time_seconds_legacy_compatible()
    }
}

fn unix_wall_clock_time_seconds_legacy_compatible() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoplayCountdownNotification {
    pub ready_user_count: usize,
    pub seconds_left: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReconnectTransitionNotification {
    Attempting {
        retries: u32,
        delay_seconds: f64,
    },
    Connected,
    Disconnected,
    RestoringState,
    StateRestoreValidationMismatch {
        local_paused: bool,
        room_paused: bool,
        local_position: f64,
        room_position: f64,
        position_diff_seconds: f64,
    },
    StateRestoreValidationCorrectionRetryScheduled {
        attempt: u32,
        max_attempts: u32,
        cooldown_ticks: u32,
    },
    StateRestoreValidationCorrectionRetriesExhausted {
        attempts: u32,
        max_attempts: u32,
    },
    StateRestoreValidationCorrectionDisabledAfterRepeatedMismatches {
        consecutive_mismatch_cycles: u32,
        disable_after_mismatch_cycles: u32,
    },
    StateRestoreValidationCorrectionRecoveryCooldownSuppressed {
        remaining_reconnect_cycles_after_this_cycle: u32,
    },
    StateRestoreValidationCorrectionRecoveryCooldownReenabled,
    RestoringPlaylist,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControllerAuthTransitionNotification {
    Attempting {
        room: String,
    },
    Succeeded {
        username: String,
        room: String,
        hide_from_osd: bool,
    },
    Failed {
        username: String,
        room: String,
        hide_from_osd: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserChangeNotification {
    Joined {
        username: String,
        room: String,
        hide_from_osd: bool,
    },
    Playing {
        username: String,
        room: String,
        file_name: Option<String>,
        file_duration: Option<Value>,
        include_room_addendum: bool,
        hide_from_osd: bool,
    },
    Left {
        username: String,
        hide_from_osd: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatNotification {
    Message {
        username: Option<String>,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FileDifferenceSummary {
    pub filename: bool,
    pub filesize: bool,
    pub fileduration: bool,
}

impl FileDifferenceSummary {
    pub fn has_differences(&self) -> bool {
        self.filename || self.filesize || self.fileduration
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconnectPlaylistRestoreIntent {
    pub files: Vec<String>,
    pub index: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DesyncCorrectionConfig {
    pub rewind_on_desync: bool,
    pub rewind_threshold_seconds: f64,
    pub fastforward_on_desync: bool,
    pub fastforward_threshold_seconds: f64,
    pub fastforward_behind_threshold_seconds: f64,
    pub fastforward_extra_seconds: f64,
    pub fastforward_reset_threshold_seconds: f64,
    pub slow_on_desync: bool,
    pub slowdown_threshold_seconds: f64,
    pub slowdown_rate: f64,
    pub slowdown_reset_threshold_seconds: f64,
}

impl Default for DesyncCorrectionConfig {
    fn default() -> Self {
        Self {
            rewind_on_desync: true,
            rewind_threshold_seconds: DEFAULT_REWIND_THRESHOLD_SECONDS,
            fastforward_on_desync: true,
            fastforward_threshold_seconds: DEFAULT_FASTFORWARD_THRESHOLD_SECONDS,
            fastforward_behind_threshold_seconds: FASTFORWARD_BEHIND_THRESHOLD_SECONDS,
            fastforward_extra_seconds: FASTFORWARD_EXTRA_SECONDS,
            fastforward_reset_threshold_seconds: FASTFORWARD_RESET_THRESHOLD_SECONDS,
            slow_on_desync: true,
            slowdown_threshold_seconds: DEFAULT_SLOWDOWN_THRESHOLD_SECONDS,
            slowdown_rate: SLOWDOWN_RATE,
            slowdown_reset_threshold_seconds: SLOWDOWN_RESET_THRESHOLD_SECONDS,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DesyncCorrectionAction {
    None,
    Rewind {
        target_position: f64,
        set_by: Option<String>,
    },
    FastForward {
        target_position: f64,
        set_by: Option<String>,
    },
    SlowDown {
        rate: f64,
        set_by: Option<String>,
    },
    RestoreSpeed {
        rate: f64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReconnectPolicyConfig {
    pub max_retries: u32,
    pub base_delay_seconds: f64,
    pub max_backoff_exponent: u32,
}

impl Default for ReconnectPolicyConfig {
    fn default() -> Self {
        Self {
            max_retries: DEFAULT_MAX_RECONNECT_RETRIES,
            base_delay_seconds: DEFAULT_RECONNECT_BASE_DELAY_SECONDS,
            max_backoff_exponent: DEFAULT_RECONNECT_BACKOFF_MAX_EXPONENT,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReconnectRetryDecision {
    pub should_retry: bool,
    pub delay_seconds: Option<f64>,
    pub should_reset_state: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconnectStateRestoreCorrectionPolicyMode {
    AutoCorrect,
    NotifyOnly,
    WarnOnlyOnExhaustion,
    DisableAfterNMismatches,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReconnectStateRestoreCorrectionMetrics {
    pub validation_cycles_started: u64,
    pub validation_cycles_completed_without_mismatch: u64,
    pub validation_cycles_completed_with_successful_correction: u64,
    pub mismatch_cycles_detected: u64,
    pub mismatch_notifications_emitted: u64,
    pub correction_actions_attempted: u64,
    pub correction_actions_succeeded: u64,
    pub correction_action_failures: u64,
    pub correction_retries_scheduled: u64,
    pub correction_retry_exhaustions: u64,
    pub correction_disables_after_repeated_mismatches: u64,
    pub correction_recovery_cooldown_suppressed_cycles: u64,
    pub correction_recovery_cooldown_reenabled_cycles: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReconnectStateRestoreCorrectionStateSnapshot {
    pub validation_pending: bool,
    pub retry_attempts: u32,
    pub retry_cooldown_ticks: u32,
    pub mismatch_notified_in_cycle: bool,
    pub mismatch_seen_in_cycle: bool,
    pub effective_policy_mode: ReconnectStateRestoreCorrectionPolicyMode,
    pub position_tolerance_seconds: f64,
    pub effective_retry_max_attempts: u32,
    pub consecutive_mismatch_cycles: u32,
    pub consecutive_retry_exhaustions: u32,
    pub recovery_cooldown_reconnect_cycles_remaining: u32,
    pub correction_suppressed_for_recovery_cycle: bool,
    pub correction_reenabled_for_recovery_cycle: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionBehaviorConfig {
    pub pause_on_leave: bool,
    pub show_same_room_osd: bool,
    pub show_osd_warnings: bool,
    pub show_noncontroller_osd: bool,
    pub show_different_room_osd: bool,
    pub loop_at_end_of_playlist: bool,
    pub loop_single_files: bool,
    pub only_switch_to_trusted_domains: bool,
    pub trusted_domains: Vec<String>,
    pub reconnect_state_restore_auto_correct: bool,
    pub reconnect_state_restore_correction_policy_mode_override:
        Option<ReconnectStateRestoreCorrectionPolicyMode>,
    pub reconnect_state_restore_position_tolerance_seconds: f64,
    pub reconnect_state_restore_correction_retry_max_attempts: u32,
    pub reconnect_state_restore_correction_retry_cooldown_ticks: u32,
    pub reconnect_state_restore_correction_retry_exponential_backoff: bool,
    pub reconnect_state_restore_correction_retry_max_cooldown_ticks: u32,
    pub reconnect_state_restore_correction_retry_adaptive_cycle_backoff: bool,
    pub reconnect_state_restore_correction_retry_adaptive_cycle_budget: bool,
    pub reconnect_state_restore_correction_retry_adaptive_cycle_budget_min_attempts: u32,
    pub reconnect_state_restore_correction_disable_after_mismatch_cycles: u32,
    pub reconnect_state_restore_correction_disable_after_mismatch_decay_on_success: u32,
    pub reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles: u32,
}

impl Default for SessionBehaviorConfig {
    fn default() -> Self {
        Self {
            pause_on_leave: true,
            show_same_room_osd: LEGACY_SHOW_SAME_ROOM_OSD,
            show_osd_warnings: LEGACY_SHOW_OSD_WARNINGS,
            show_noncontroller_osd: LEGACY_SHOW_NONCONTROLLER_OSD,
            show_different_room_osd: LEGACY_SHOW_DIFFERENT_ROOM_OSD,
            loop_at_end_of_playlist: false,
            loop_single_files: false,
            only_switch_to_trusted_domains: LEGACY_ONLY_SWITCH_TO_TRUSTED_DOMAINS,
            trusted_domains: LEGACY_DEFAULT_TRUSTED_DOMAINS
                .iter()
                .map(|domain| (*domain).to_owned())
                .collect(),
            reconnect_state_restore_auto_correct: DEFAULT_RECONNECT_STATE_RESTORE_AUTOCORRECT,
            reconnect_state_restore_correction_policy_mode_override: None,
            reconnect_state_restore_position_tolerance_seconds: SEEK_THRESHOLD_SECONDS,
            reconnect_state_restore_correction_retry_max_attempts:
                DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RETRY_MAX_ATTEMPTS,
            reconnect_state_restore_correction_retry_cooldown_ticks:
                DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RETRY_COOLDOWN_TICKS,
            reconnect_state_restore_correction_retry_exponential_backoff:
                DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RETRY_EXPONENTIAL_BACKOFF,
            reconnect_state_restore_correction_retry_max_cooldown_ticks:
                DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RETRY_MAX_COOLDOWN_TICKS,
            reconnect_state_restore_correction_retry_adaptive_cycle_backoff:
                DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RETRY_ADAPTIVE_CYCLE_BACKOFF,
            reconnect_state_restore_correction_retry_adaptive_cycle_budget:
                DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RETRY_ADAPTIVE_CYCLE_BUDGET,
            reconnect_state_restore_correction_retry_adaptive_cycle_budget_min_attempts:
                DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RETRY_ADAPTIVE_CYCLE_BUDGET_MIN_ATTEMPTS,
            reconnect_state_restore_correction_disable_after_mismatch_cycles:
                DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_DISABLE_AFTER_MISMATCHES,
            reconnect_state_restore_correction_disable_after_mismatch_decay_on_success:
                DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_DISABLE_AFTER_MISMATCH_DECAY_ON_SUCCESS,
            reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles:
                DEFAULT_RECONNECT_STATE_RESTORE_CORRECTION_RECOVERY_COOLDOWN_RECONNECT_CYCLES,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UnpauseActionMode {
    #[default]
    IfAlreadyReady,
    IfOthersReady,
    IfMinUsersReady,
    Always,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivacyMode {
    SendRaw,
    SendHashed,
    DoNotSend,
}

impl PrivacyMode {
    pub fn from_legacy_name(mode: &str) -> Option<Self> {
        match mode {
            "SendRaw" => Some(Self::SendRaw),
            "SendHashed" => Some(Self::SendHashed),
            "DoNotSend" => Some(Self::DoNotSend),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReadinessAutoplayConfig {
    pub unpause_action: UnpauseActionMode,
    pub auto_play_threshold: Option<usize>,
    pub autoplay_require_same_filenames: bool,
    pub show_duration_notification: bool,
    pub different_duration_threshold_seconds: f64,
    pub autoplay_delay_seconds: f64,
    pub last_paused_diff_threshold_seconds: f64,
}

impl Default for ReadinessAutoplayConfig {
    fn default() -> Self {
        Self {
            unpause_action: UnpauseActionMode::IfAlreadyReady,
            auto_play_threshold: None,
            autoplay_require_same_filenames: false,
            show_duration_notification: LEGACY_SHOW_DURATION_NOTIFICATION,
            different_duration_threshold_seconds: LEGACY_DIFFERENT_DURATION_THRESHOLD_SECONDS,
            autoplay_delay_seconds: DEFAULT_AUTOPLAY_DELAY_SECONDS,
            last_paused_diff_threshold_seconds: DEFAULT_LAST_PAUSED_DIFF_THRESHOLD_SECONDS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatConfig {
    pub max_chat_message_length: usize,
    pub apply_server_max_chat_message_length: bool,
}

impl Default for ChatConfig {
    fn default() -> Self {
        Self {
            max_chat_message_length: LEGACY_CHAT_MAX_MESSAGE_LENGTH,
            apply_server_max_chat_message_length: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClientRuntimeAction {
    SetPaused(bool),
    RequestUserList,
    SetRoom {
        room: String,
    },
    SetReady {
        ready: bool,
        manually_initiated: bool,
    },
    SetReadyForUser {
        ready: bool,
        manually_initiated: bool,
        username: String,
    },
    SetFile {
        file_payload: Value,
    },
    SetPlaylist {
        files: Vec<String>,
    },
    SetPlaylistIndex {
        index: i64,
    },
    RequestControllerAuth {
        room: String,
        password: String,
    },
    SendChat {
        message: String,
    },
    NotifyChat(ChatNotification),
    NotifyControllerAuthTransition(ControllerAuthTransitionNotification),
    NotifyUserChange(UserChangeNotification),
    NotifyReconnectTransition(ReconnectTransitionNotification),
    NotifyAutoplayCountdown(AutoplayCountdownNotification),
    SetPosition(f64),
    SetPlaybackRate(f64),
    ScheduleReconnect {
        delay_seconds: f64,
    },
    StopReconnect,
}

pub trait ClientRuntimeControl {
    fn request_user_list(&mut self) {}
    fn set_room(&mut self, _room: String) {}
    fn set_ready(&mut self, ready: bool, manually_initiated: bool);
    fn set_ready_for_user(&mut self, ready: bool, manually_initiated: bool, _username: String) {
        self.set_ready(ready, manually_initiated);
    }
    fn set_file(&mut self, _file_payload: Value) {}
    fn set_playlist(&mut self, _files: Vec<String>) {}
    fn set_playlist_index(&mut self, _index: i64) {}
    fn request_controller_auth(&mut self, _room: String, _password: String) {}
    fn send_chat(&mut self, _message: String) {}
    fn notify_chat(&mut self, _notification: ChatNotification) {}
    fn notify_controller_auth_transition(
        &mut self,
        _notification: ControllerAuthTransitionNotification,
    ) {
    }
    fn notify_user_change(&mut self, _notification: UserChangeNotification) {}
    fn notify_reconnect_transition(&mut self, _notification: ReconnectTransitionNotification) {}
    fn schedule_reconnect(&mut self, delay_seconds: f64);
    fn stop_reconnect(&mut self);
    fn notify_autoplay_countdown(&mut self, _notification: AutoplayCountdownNotification) {}
}

#[derive(Debug, Default)]
pub struct QueuedRuntimeControl {
    outbound_messages: Vec<ProtocolMessage>,
    reconnect_delays: Vec<f64>,
    stop_reconnect_calls: usize,
    chat_notifications: Vec<ChatNotification>,
    controller_auth_notifications: Vec<ControllerAuthTransitionNotification>,
    user_change_notifications: Vec<UserChangeNotification>,
    reconnect_notifications: Vec<ReconnectTransitionNotification>,
    autoplay_notifications: Vec<AutoplayCountdownNotification>,
}

impl QueuedRuntimeControl {
    fn file_payload_from_value(file_payload: Value) -> Option<FilePayload> {
        let Value::Object(mut fields) = file_payload else {
            return None;
        };

        let name = fields
            .remove("name")
            .and_then(|value| value.as_str().map(str::to_owned));
        let duration = fields.remove("duration").and_then(|value| value.as_f64());
        let size = fields.remove("size");
        let path = fields
            .remove("path")
            .and_then(|value| value.as_str().map(str::to_owned));

        Some(FilePayload {
            name,
            duration,
            size,
            path,
            extra: fields.into_iter().collect(),
        })
    }

    pub fn outbound_messages(&self) -> &[ProtocolMessage] {
        &self.outbound_messages
    }

    pub fn reconnect_delays(&self) -> &[f64] {
        &self.reconnect_delays
    }

    pub fn stop_reconnect_calls(&self) -> usize {
        self.stop_reconnect_calls
    }

    pub fn autoplay_notifications(&self) -> &[AutoplayCountdownNotification] {
        &self.autoplay_notifications
    }

    pub fn chat_notifications(&self) -> &[ChatNotification] {
        &self.chat_notifications
    }

    pub fn controller_auth_notifications(&self) -> &[ControllerAuthTransitionNotification] {
        &self.controller_auth_notifications
    }

    pub fn user_change_notifications(&self) -> &[UserChangeNotification] {
        &self.user_change_notifications
    }

    pub fn reconnect_notifications(&self) -> &[ReconnectTransitionNotification] {
        &self.reconnect_notifications
    }

    pub fn drain_outbound_messages(&mut self) -> Vec<ProtocolMessage> {
        std::mem::take(&mut self.outbound_messages)
    }

    pub fn drain_outbound_message_lines(&mut self) -> Result<Vec<String>, ProtocolError> {
        let messages = self.drain_outbound_messages();
        messages
            .iter()
            .map(encode_message_line)
            .collect::<Result<Vec<_>, _>>()
    }

    pub fn drain_reconnect_delays(&mut self) -> Vec<f64> {
        std::mem::take(&mut self.reconnect_delays)
    }

    pub fn take_stop_reconnect_requested(&mut self) -> bool {
        let requested = self.stop_reconnect_calls > 0;
        self.stop_reconnect_calls = 0;
        requested
    }

    pub fn drain_autoplay_notifications(&mut self) -> Vec<AutoplayCountdownNotification> {
        std::mem::take(&mut self.autoplay_notifications)
    }

    pub fn drain_chat_notifications(&mut self) -> Vec<ChatNotification> {
        std::mem::take(&mut self.chat_notifications)
    }

    pub fn drain_controller_auth_notifications(
        &mut self,
    ) -> Vec<ControllerAuthTransitionNotification> {
        std::mem::take(&mut self.controller_auth_notifications)
    }

    pub fn drain_user_change_notifications(&mut self) -> Vec<UserChangeNotification> {
        std::mem::take(&mut self.user_change_notifications)
    }

    pub fn drain_reconnect_notifications(&mut self) -> Vec<ReconnectTransitionNotification> {
        std::mem::take(&mut self.reconnect_notifications)
    }
}

impl ClientRuntimeControl for QueuedRuntimeControl {
    fn request_user_list(&mut self) {
        self.outbound_messages.push(ProtocolMessage::list_request());
    }

    fn set_room(&mut self, room: String) {
        let set_payload = SetPayload::new().with_room(RoomRef::new(room));
        self.outbound_messages
            .push(ProtocolMessage::set(set_payload));
    }

    fn set_ready(&mut self, ready: bool, manually_initiated: bool) {
        let ready_payload = ReadyPayload::new(ready).with_manually_initiated(manually_initiated);
        let set_payload = SetPayload::new().with_ready(ready_payload);
        self.outbound_messages
            .push(ProtocolMessage::set(set_payload));
    }

    fn set_ready_for_user(&mut self, ready: bool, manually_initiated: bool, username: String) {
        let ready_payload = ReadyPayload::new(ready)
            .with_manually_initiated(manually_initiated)
            .with_username(username);
        let set_payload = SetPayload::new().with_ready(ready_payload);
        self.outbound_messages
            .push(ProtocolMessage::set(set_payload));
    }

    fn set_file(&mut self, file_payload: Value) {
        let Some(file_payload) = Self::file_payload_from_value(file_payload) else {
            return;
        };
        let set_payload = SetPayload::new().with_file(file_payload);
        self.outbound_messages
            .push(ProtocolMessage::set(set_payload));
    }

    fn set_playlist(&mut self, files: Vec<String>) {
        let set_payload = SetPayload::new().with_playlist_change(PlaylistChangePayload::new(files));
        self.outbound_messages
            .push(ProtocolMessage::set(set_payload));
    }

    fn set_playlist_index(&mut self, index: i64) {
        let set_payload = SetPayload::new().with_playlist_index(PlaylistIndexPayload::new(index));
        self.outbound_messages
            .push(ProtocolMessage::set(set_payload));
    }

    fn request_controller_auth(&mut self, room: String, password: String) {
        let payload = ControllerAuthPayload::new()
            .with_room(room)
            .with_password(password);
        let set_payload = SetPayload::new().with_controller_auth(payload);
        self.outbound_messages
            .push(ProtocolMessage::set(set_payload));
    }

    fn send_chat(&mut self, message: String) {
        self.outbound_messages
            .push(ProtocolMessage::chat_text(message));
    }

    fn notify_chat(&mut self, notification: ChatNotification) {
        self.chat_notifications.push(notification);
    }

    fn notify_controller_auth_transition(
        &mut self,
        notification: ControllerAuthTransitionNotification,
    ) {
        self.controller_auth_notifications.push(notification);
    }

    fn notify_user_change(&mut self, notification: UserChangeNotification) {
        self.user_change_notifications.push(notification);
    }

    fn schedule_reconnect(&mut self, delay_seconds: f64) {
        self.reconnect_delays.push(delay_seconds);
    }

    fn stop_reconnect(&mut self) {
        self.stop_reconnect_calls += 1;
    }

    fn notify_reconnect_transition(&mut self, notification: ReconnectTransitionNotification) {
        self.reconnect_notifications.push(notification);
    }

    fn notify_autoplay_countdown(&mut self, notification: AutoplayCountdownNotification) {
        self.autoplay_notifications.push(notification);
    }
}

#[derive(Debug)]
pub struct ClientRuntime<P, C> {
    session: ClientSession,
    player: P,
    control: C,
    ping_metrics_legacy_compatible: ClientPingMetricsLegacyCompatible,
    pending_player_playback_telemetry_updates: Vec<PlayerPlaybackTelemetryUpdate>,
}

impl<P, C> ClientRuntime<P, C>
where
    P: PlayerAdapter,
    C: ClientRuntimeControl,
{
    pub fn new(session: ClientSession, player: P, control: C) -> Self {
        Self {
            session,
            player,
            control,
            ping_metrics_legacy_compatible: ClientPingMetricsLegacyCompatible::default(),
            pending_player_playback_telemetry_updates: Vec::new(),
        }
    }

    pub fn session(&self) -> &ClientSession {
        &self.session
    }

    pub fn session_mut(&mut self) -> &mut ClientSession {
        &mut self.session
    }

    pub fn reconnect_state_restore_correction_metrics(
        &self,
    ) -> &ReconnectStateRestoreCorrectionMetrics {
        self.session.reconnect_state_restore_correction_metrics()
    }

    pub fn reconnect_state_restore_correction_state_snapshot(
        &self,
    ) -> ReconnectStateRestoreCorrectionStateSnapshot {
        self.session
            .reconnect_state_restore_correction_state_snapshot()
    }

    pub fn control(&self) -> &C {
        &self.control
    }

    pub fn player(&self) -> &P {
        &self.player
    }

    pub fn player_mut(&mut self) -> &mut P {
        &mut self.player
    }

    pub fn into_parts(self) -> (ClientSession, P, C) {
        (self.session, self.player, self.control)
    }

    pub fn run_readiness_unpause_attempt(
        &mut self,
        now_seconds: f64,
        readiness_supported: bool,
        local_can_control: bool,
        is_playing_music: bool,
    ) -> Result<(), PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let actions = self.session.runtime_actions_for_readiness_unpause_attempt(
            now_seconds,
            readiness_supported,
            local_can_control,
            is_playing_music,
        );
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn update_autoplay_check(
        &mut self,
        readiness_supported: bool,
        local_can_control: bool,
        is_playing_music: bool,
        recently_advanced: bool,
    ) {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        self.session.autoplay_check(
            readiness_supported,
            local_can_control,
            is_playing_music,
            recently_advanced,
        );
    }

    pub fn tick_autoplay(
        &mut self,
        readiness_supported: bool,
        local_can_control: bool,
        is_playing_music: bool,
        recently_advanced: bool,
    ) -> Result<(), PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let actions = self.session.autoplay_countdown_tick(
            readiness_supported,
            local_can_control,
            is_playing_music,
            recently_advanced,
        );
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn run_reconnect_retry(&mut self, retries: u32) -> Result<(), PlayerError> {
        let actions = self.session.runtime_actions_for_reconnect_retry(retries);
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn run_controller_auth_notifications_if_needed(&mut self) -> Result<(), PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_controller_auth_notifications_if_needed();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn run_chat_notifications_if_needed(&mut self) -> Result<(), PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_chat_notifications_if_needed();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn run_user_change_notifications_if_needed(&mut self) -> Result<(), PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_user_change_notifications_if_needed();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn run_reconnect_transition_if_needed(&mut self) -> Result<(), PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_reconnect_transition_if_needed();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn run_reconnect_state_restore_if_needed(&mut self) -> Result<(), PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_reconnect_state_restore_if_needed();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn run_reconnect_state_restore_validation_if_needed(&mut self) -> Result<(), PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let actions = self
            .session
            .runtime_actions_for_reconnect_state_restore_validation_if_needed();
        if actions.is_empty() {
            return Ok(());
        }

        let mut attempted_correction = false;
        for action in &actions {
            let is_correction_action = matches!(
                action,
                ClientRuntimeAction::SetPaused(_) | ClientRuntimeAction::SetPosition(_)
            );
            attempted_correction |= is_correction_action;
            if is_correction_action {
                self.session
                    .reconnect_state_restore_correction_metrics
                    .correction_actions_attempted = self
                    .session
                    .reconnect_state_restore_correction_metrics
                    .correction_actions_attempted
                    .saturating_add(1);
            }

            if let Err(err) = ClientSession::dispatch_runtime_actions(
                std::slice::from_ref(action),
                &mut self.player,
                &mut self.control,
            ) {
                if is_correction_action {
                    self.session
                        .reconnect_state_restore_correction_metrics
                        .correction_action_failures = self
                        .session
                        .reconnect_state_restore_correction_metrics
                        .correction_action_failures
                        .saturating_add(1);
                    if let Some(notification) = self
                        .session
                        .defer_reconnect_state_restore_validation_after_correction_failure()
                    {
                        self.control.notify_reconnect_transition(notification);
                    }
                    return Ok(());
                }
                return Err(err);
            }

            if is_correction_action {
                self.session
                    .reconnect_state_restore_correction_metrics
                    .correction_actions_succeeded = self
                    .session
                    .reconnect_state_restore_correction_metrics
                    .correction_actions_succeeded
                    .saturating_add(1);
            }
            self.session
                .apply_successful_reconnect_state_restore_validation_action(action);
        }

        if attempted_correction {
            self.session
                .complete_reconnect_state_restore_validation_after_success();
        }

        Ok(())
    }

    pub fn run_room_pause_sync_if_needed(&mut self) -> Result<(), PlayerError> {
        // Reconnect validation owns correction immediately after reconnect state restore.
        if self.session.reconnect_state_restore_validation_pending {
            return Ok(());
        }

        self.sync_player_playback_telemetry_into_session_and_buffer();

        let Some(room_playstate) = self.session.current_room_playstate().cloned() else {
            return Ok(());
        };
        let Some(room_paused) = room_playstate.paused else {
            return Ok(());
        };
        let Some(local_paused) = self.session.local_paused else {
            return Ok(());
        };
        if local_paused == room_paused {
            return Ok(());
        }
        let set_by_is_self = self
            .session
            .username
            .as_deref()
            .zip(room_playstate.set_by.as_deref())
            .is_some_and(|(username, set_by)| username == set_by);
        if set_by_is_self {
            return Ok(());
        }

        ClientSession::dispatch_runtime_actions(
            &[ClientRuntimeAction::SetPaused(room_paused)],
            &mut self.player,
            &mut self.control,
        )?;
        // Mirror the expected local state to avoid duplicate correction attempts until telemetry catches up.
        self.session.local_paused = Some(room_paused);
        Ok(())
    }

    pub fn run_desync_correction_if_needed(
        &mut self,
        now_seconds: f64,
        local_can_control: bool,
        dont_slow_down_with_me: bool,
        speed_supported: bool,
    ) -> Result<(), PlayerError> {
        // Reconnect validation owns the correction window immediately after reconnect restore.
        if self.session.reconnect_state_restore_validation_pending {
            return Ok(());
        }

        self.sync_player_playback_telemetry_into_session_and_buffer();
        let Some(local_position) = self.session.local_position else {
            return Ok(());
        };
        let local_position =
            self.desync_local_position_with_legacy_ping_forward_delay(local_position);

        let actions = self.session.runtime_actions_for_desync_correction(
            now_seconds,
            local_position,
            local_can_control,
            dont_slow_down_with_me,
            speed_supported,
        );
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    fn desync_local_position_with_legacy_ping_forward_delay(&self, local_position: f64) -> f64 {
        let Some(room_playstate) = self.session.current_room_playstate() else {
            return local_position;
        };
        if room_playstate.paused != Some(false) || room_playstate.do_seek == Some(true) {
            return local_position;
        }

        let forward_delay = self.ping_metrics_legacy_compatible.forward_delay_seconds();
        if !forward_delay.is_finite() || forward_delay <= 0.0 {
            return local_position;
        }

        // Compare against an estimate of "room position now" by moving local position back by
        // the inferred one-way/forward delay before evaluating threshold-based desync actions.
        local_position - forward_delay
    }

    pub fn run_reconnect_playlist_restore_if_needed(&mut self) -> Result<(), PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_reconnect_playlist_restore_if_needed();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn run_controller_reidentify_if_needed(&mut self) -> Result<(), PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_controller_reidentify_if_needed();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn run_send_chat_message(
        &mut self,
        message: impl Into<String>,
    ) -> Result<bool, PlayerError> {
        if self.session.server_chat_supported().is_none() {
            return Ok(false);
        }
        let actions = self
            .session
            .runtime_actions_for_outbound_chat_message(message.into());
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_player_chat_input_if_needed(&mut self) -> Result<usize, PlayerError> {
        let mut sent = 0usize;
        while let Some(message) = self.player.take_pending_chat_request() {
            if self.run_send_chat_message(message)? {
                sent += 1;
            }
        }
        Ok(sent)
    }

    pub fn run_toggle_ready(&mut self, manually_initiated: bool) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_ready_toggle(manually_initiated);
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_set_ready_for_user(
        &mut self,
        username: impl Into<String>,
        ready: bool,
        manually_initiated: bool,
    ) -> Result<bool, PlayerError> {
        let actions = self.session.runtime_actions_for_local_user_ready_set(
            username.into(),
            ready,
            manually_initiated,
        );
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_request_controller_auth(
        &mut self,
        room: impl Into<String>,
        password: impl Into<String>,
    ) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_controller_auth_request(room.into(), password.into());
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_set_room(&mut self, room: impl Into<String>) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_room_switch(room.into());
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_set_room_with_legacy_fallback(
        &mut self,
        default_room: impl Into<String>,
    ) -> Result<bool, PlayerError> {
        let default_room = default_room.into();
        let room = self
            .session
            .local_room_command_target_with_legacy_fallback(&default_room);
        self.run_set_room(room)
    }

    pub fn run_toggle_pause(&mut self) -> Result<bool, PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let actions = self.session.runtime_actions_for_local_pause_toggle();
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_request_user_list(&mut self) -> Result<bool, PlayerError> {
        let actions = self.session.runtime_actions_for_local_user_list_request();
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_set_playlist_index(&mut self, index: i64) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_playlist_index_set(index);
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_advance_playlist_index(&mut self) -> Result<bool, PlayerError> {
        let actions = self.session.runtime_actions_for_local_playlist_next();
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_queue_playlist_item(
        &mut self,
        file_name: impl Into<String>,
        select_after_queue: bool,
    ) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_playlist_queue(file_name.into(), select_after_queue);
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_delete_playlist_index(&mut self, index: i64) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_playlist_delete(index);
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_replace_playlist(
        &mut self,
        files: Vec<String>,
        selected_index: Option<usize>,
    ) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_playlist_replace(files, selected_index);
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_undo_playlist_change(&mut self) -> Result<bool, PlayerError> {
        let actions = self.session.runtime_actions_for_local_playlist_undo();
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_shuffle_remaining_playlist(&mut self) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_playlist_shuffle_remaining();
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_shuffle_entire_playlist(&mut self) -> Result<bool, PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_playlist_shuffle_entire();
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_seek_to_position(&mut self, target_position: f64) -> Result<bool, PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let actions = self.session.runtime_actions_for_local_seek(target_position);
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_seek_by_offset(&mut self, offset_seconds: f64) -> Result<bool, PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let actions = self
            .session
            .runtime_actions_for_local_seek_offset(offset_seconds);
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_undo_seek(&mut self) -> Result<bool, PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let actions = self.session.runtime_actions_for_local_seek_undo();
        let sent = !actions.is_empty();
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
            .map(|_| sent)
    }

    pub fn run_disconnect(&mut self, now_seconds: f64) -> Result<(), PlayerError> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        let actions = self.session.handle_disconnect(now_seconds);
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn publish_local_file_legacy_compatible(
        &mut self,
        file_payload: &Value,
        filename_privacy_mode: PrivacyMode,
        filesize_privacy_mode: PrivacyMode,
    ) -> Result<(), PlayerError> {
        let actions = self
            .session
            .runtime_actions_for_local_file_publish_legacy_compatible(
                file_payload,
                filename_privacy_mode,
                filesize_privacy_mode,
            );
        ClientSession::dispatch_runtime_actions(&actions, &mut self.player, &mut self.control)
    }

    pub fn publish_pending_local_file_update_legacy_compatible(
        &mut self,
        filename_privacy_mode: PrivacyMode,
        filesize_privacy_mode: PrivacyMode,
    ) -> Result<bool, PlayerError> {
        let Some(local_file_update) = self.player.take_local_file_update() else {
            return Ok(false);
        };

        let file_payload = Self::local_file_update_payload(&local_file_update);
        self.publish_local_file_legacy_compatible(
            &file_payload,
            filename_privacy_mode,
            filesize_privacy_mode,
        )?;
        Ok(true)
    }

    fn local_file_update_payload(local_file_update: &LocalFileUpdate) -> Value {
        let mut payload = Map::new();
        payload.insert(
            "name".to_owned(),
            Value::String(local_file_update.name.clone()),
        );
        if let Some(duration_seconds) = local_file_update.duration_seconds {
            payload.insert("duration".to_owned(), Value::from(duration_seconds));
        }
        if let Some(size_bytes) = local_file_update.size_bytes {
            payload.insert("size".to_owned(), Value::from(size_bytes));
        }
        if let Some(path) = local_file_update.path.as_ref() {
            payload.insert("path".to_owned(), Value::String(path.clone()));
        }
        Value::Object(payload)
    }

    fn sync_player_playback_telemetry_into_session_and_buffer(&mut self) {
        while let Some(update) = self.player.take_playback_telemetry_update() {
            self.session.apply_player_playback_telemetry_update(&update);
            self.pending_player_playback_telemetry_updates.push(update);
        }
    }
}

impl<P> ClientRuntime<P, QueuedRuntimeControl>
where
    P: PlayerAdapter,
{
    pub fn run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible(
        &mut self,
        inbound_state: StatePayload,
    ) -> bool {
        self.ping_metrics_legacy_compatible
            .observe_inbound_state(&inbound_state);
        self.run_state_sync_reconcile_with_inbound_state(
            inbound_state,
            self.ping_metrics_legacy_compatible
                .client_latency_calculation_now(),
            self.ping_metrics_legacy_compatible.client_rtt_seconds(),
        )
    }

    pub fn run_state_sync_reconcile_with_inbound_state(
        &mut self,
        inbound_state: StatePayload,
        client_latency_calculation: f64,
        client_rtt: f64,
    ) -> bool {
        self.sync_player_playback_telemetry_into_session_and_buffer();

        let (Some(local_position), Some(local_paused)) =
            (self.session.local_position, self.session.local_paused)
        else {
            self.session.apply_state(inbound_state);
            return false;
        };

        let outbound_state = self.session.reconcile_state_and_build_response(
            inbound_state,
            local_position,
            local_paused,
            client_latency_calculation,
            client_rtt,
        );
        self.control
            .outbound_messages
            .push(ProtocolMessage::state(outbound_state));
        true
    }

    pub fn flush_queued_protocol_messages(&mut self) -> Vec<ProtocolMessage> {
        self.control.drain_outbound_messages()
    }

    pub fn flush_queued_protocol_lines(&mut self) -> Result<Vec<String>, ProtocolError> {
        self.control.drain_outbound_message_lines()
    }

    pub fn drain_reconnect_requests(&mut self) -> Vec<f64> {
        self.control.drain_reconnect_delays()
    }

    pub fn take_stop_reconnect_requested(&mut self) -> bool {
        self.control.take_stop_reconnect_requested()
    }

    pub fn drain_autoplay_notifications(&mut self) -> Vec<AutoplayCountdownNotification> {
        self.control.drain_autoplay_notifications()
    }

    pub fn drain_chat_notifications(&mut self) -> Vec<ChatNotification> {
        self.control.drain_chat_notifications()
    }

    pub fn drain_controller_auth_notifications(
        &mut self,
    ) -> Vec<ControllerAuthTransitionNotification> {
        self.control.drain_controller_auth_notifications()
    }

    pub fn drain_user_change_notifications(&mut self) -> Vec<UserChangeNotification> {
        self.control.drain_user_change_notifications()
    }

    pub fn drain_reconnect_notifications(&mut self) -> Vec<ReconnectTransitionNotification> {
        self.control.drain_reconnect_notifications()
    }

    pub fn drain_player_playback_telemetry_updates(
        &mut self,
    ) -> Vec<PlayerPlaybackTelemetryUpdate> {
        self.sync_player_playback_telemetry_into_session_and_buffer();
        std::mem::take(&mut self.pending_player_playback_telemetry_updates)
    }

    pub fn flush_queued_protocol_lines_to_transport<F>(
        &mut self,
        mut send_line: F,
    ) -> Result<(), ProtocolError>
    where
        F: FnMut(&str) -> Result<(), ProtocolError>,
    {
        let lines = self.flush_queued_protocol_lines()?;
        for line in &lines {
            send_line(line)?;
        }
        Ok(())
    }

    pub fn drain_reconnect_intents<FS, FT>(
        &mut self,
        mut schedule_reconnect: FS,
        mut stop_reconnect: FT,
    ) where
        FS: FnMut(f64),
        FT: FnMut(),
    {
        for delay_seconds in self.drain_reconnect_requests() {
            schedule_reconnect(delay_seconds);
        }
        if self.take_stop_reconnect_requested() {
            stop_reconnect();
        }
    }

    pub fn drain_autoplay_notifications_to_sink<F, E>(&mut self, mut notify: F) -> Result<(), E>
    where
        F: FnMut(&AutoplayCountdownNotification) -> Result<(), E>,
    {
        for notification in self.drain_autoplay_notifications() {
            notify(&notification)?;
        }
        Ok(())
    }

    pub fn drain_controller_auth_notifications_to_sink<F, E>(
        &mut self,
        mut notify: F,
    ) -> Result<(), E>
    where
        F: FnMut(&ControllerAuthTransitionNotification) -> Result<(), E>,
    {
        for notification in self.drain_controller_auth_notifications() {
            notify(&notification)?;
        }
        Ok(())
    }

    pub fn drain_chat_notifications_to_sink<F, E>(&mut self, mut notify: F) -> Result<(), E>
    where
        F: FnMut(&ChatNotification) -> Result<(), E>,
    {
        for notification in self.drain_chat_notifications() {
            notify(&notification)?;
        }
        Ok(())
    }

    pub fn drain_user_change_notifications_to_sink<F, E>(&mut self, mut notify: F) -> Result<(), E>
    where
        F: FnMut(&UserChangeNotification) -> Result<(), E>,
    {
        for notification in self.drain_user_change_notifications() {
            notify(&notification)?;
        }
        Ok(())
    }

    pub fn drain_reconnect_notifications_to_sink<F, E>(&mut self, mut notify: F) -> Result<(), E>
    where
        F: FnMut(&ReconnectTransitionNotification) -> Result<(), E>,
    {
        for notification in self.drain_reconnect_notifications() {
            notify(&notification)?;
        }
        Ok(())
    }

    pub fn drain_player_playback_telemetry_updates_to_sink<F, E>(
        &mut self,
        mut notify: F,
    ) -> Result<(), E>
    where
        F: FnMut(&PlayerPlaybackTelemetryUpdate) -> Result<(), E>,
    {
        for update in self.drain_player_playback_telemetry_updates() {
            notify(&update)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClientUserView {
    pub room: Option<String>,
    pub ready: Option<bool>,
    pub has_file: bool,
    pub file_name: Option<String>,
    pub file_size: Option<Value>,
    pub file_duration: Option<Value>,
    pub controller: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RoomPlaylistView {
    pub files: Vec<String>,
    pub index: Option<i64>,
    pub set_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RoomPlaystateView {
    pub position: Option<f64>,
    pub paused: Option<bool>,
    pub do_seek: Option<bool>,
    pub set_by: Option<String>,
}

#[derive(Debug)]
pub struct ClientSession {
    pub username: Option<String>,
    pub room: Option<String>,
    pub domain: SyncDomain,
    server_readiness_supported: Option<bool>,
    server_chat_supported: Option<bool>,
    desync_config: DesyncCorrectionConfig,
    reconnect_policy: ReconnectPolicyConfig,
    behavior_config: SessionBehaviorConfig,
    readiness_autoplay_config: ReadinessAutoplayConfig,
    chat_config: ChatConfig,
    speed_changed: bool,
    behind_first_detected_at_seconds: Option<f64>,
    last_paused_on_leave_at_seconds: Option<f64>,
    last_advanced_at_seconds: Option<f64>,
    user_views: BTreeMap<String, ClientUserView>,
    known_rooms: BTreeSet<String>,
    room_playlists: BTreeMap<String, RoomPlaylistView>,
    room_playstates: BTreeMap<String, RoomPlaystateView>,
    pending_playlist: Option<RoomPlaylistView>,
    reconnect_ready_restore_snapshot: Option<bool>,
    reconnect_ready_restore_intent: Option<bool>,
    reconnect_file_restore_snapshot: Option<Value>,
    reconnect_file_restore_intent: Option<Value>,
    reconnect_controller_restore_snapshot: Option<bool>,
    reconnect_playlist_restore_snapshot: Option<ReconnectPlaylistRestoreIntent>,
    reconnect_playlist_restore_intent: Option<ReconnectPlaylistRestoreIntent>,
    reconnect_state_restore_validation_pending: bool,
    reconnect_state_restore_validation_retry_attempts: u32,
    reconnect_state_restore_validation_retry_cooldown_ticks: u32,
    reconnect_state_restore_validation_mismatch_notified: bool,
    reconnect_state_restore_validation_mismatch_seen_in_cycle: bool,
    reconnect_state_restore_correction_consecutive_mismatch_cycles: u32,
    reconnect_state_restore_correction_consecutive_retry_exhaustions: u32,
    reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining: u32,
    reconnect_state_restore_correction_recovery_suppressed_this_cycle: bool,
    reconnect_state_restore_correction_recovery_reenable_notification_pending: bool,
    reconnect_state_restore_correction_recovery_reenabled_this_cycle: bool,
    reconnect_state_restore_correction_metrics: ReconnectStateRestoreCorrectionMetrics,
    pending_chat_notifications: Vec<ChatNotification>,
    pending_controller_auth_notifications: Vec<ControllerAuthTransitionNotification>,
    pending_user_change_notifications: Vec<UserChangeNotification>,
    reconnect_in_progress: bool,
    reconnect_connected_intent: bool,
    controlled_room_switch_intent: Option<String>,
    controller_reidentify_intent: Option<(String, String)>,
    controlled_room_passwords: BTreeMap<String, String>,
    playlist_undo_snapshots: BTreeMap<String, Vec<String>>,
    playlist_shuffle_nonce: u64,
    last_seek_position_before_manual_seek: Option<f64>,
    local_position: Option<f64>,
    local_paused: Option<bool>,
    autoplay_enabled: bool,
    autoplay_timer_running: bool,
    autoplay_time_left_seconds: f64,
    client_ignoring_on_the_fly: u32,
    server_ignoring_on_the_fly: u32,
}

impl Default for ClientSession {
    fn default() -> Self {
        let readiness_autoplay_config = ReadinessAutoplayConfig::default();
        let autoplay_time_left_seconds = readiness_autoplay_config.autoplay_delay_seconds;

        Self {
            username: None,
            room: None,
            domain: SyncDomain::default(),
            server_readiness_supported: None,
            server_chat_supported: None,
            desync_config: DesyncCorrectionConfig::default(),
            reconnect_policy: ReconnectPolicyConfig::default(),
            behavior_config: SessionBehaviorConfig::default(),
            readiness_autoplay_config,
            chat_config: ChatConfig::default(),
            speed_changed: false,
            behind_first_detected_at_seconds: None,
            last_paused_on_leave_at_seconds: None,
            last_advanced_at_seconds: None,
            user_views: BTreeMap::new(),
            known_rooms: BTreeSet::new(),
            room_playlists: BTreeMap::new(),
            room_playstates: BTreeMap::new(),
            pending_playlist: None,
            reconnect_ready_restore_snapshot: None,
            reconnect_ready_restore_intent: None,
            reconnect_file_restore_snapshot: None,
            reconnect_file_restore_intent: None,
            reconnect_controller_restore_snapshot: None,
            reconnect_playlist_restore_snapshot: None,
            reconnect_playlist_restore_intent: None,
            reconnect_state_restore_validation_pending: false,
            reconnect_state_restore_validation_retry_attempts: 0,
            reconnect_state_restore_validation_retry_cooldown_ticks: 0,
            reconnect_state_restore_validation_mismatch_notified: false,
            reconnect_state_restore_validation_mismatch_seen_in_cycle: false,
            reconnect_state_restore_correction_consecutive_mismatch_cycles: 0,
            reconnect_state_restore_correction_consecutive_retry_exhaustions: 0,
            reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining: 0,
            reconnect_state_restore_correction_recovery_suppressed_this_cycle: false,
            reconnect_state_restore_correction_recovery_reenable_notification_pending: false,
            reconnect_state_restore_correction_recovery_reenabled_this_cycle: false,
            reconnect_state_restore_correction_metrics:
                ReconnectStateRestoreCorrectionMetrics::default(),
            pending_chat_notifications: Vec::new(),
            pending_controller_auth_notifications: Vec::new(),
            pending_user_change_notifications: Vec::new(),
            reconnect_in_progress: false,
            reconnect_connected_intent: false,
            controlled_room_switch_intent: None,
            controller_reidentify_intent: None,
            controlled_room_passwords: BTreeMap::new(),
            playlist_undo_snapshots: BTreeMap::new(),
            playlist_shuffle_nonce: 0,
            last_seek_position_before_manual_seek: None,
            local_position: None,
            local_paused: None,
            autoplay_enabled: false,
            autoplay_timer_running: false,
            autoplay_time_left_seconds,
            client_ignoring_on_the_fly: 0,
            server_ignoring_on_the_fly: 0,
        }
    }
}

impl ClientSession {
    pub fn apply_player_playback_telemetry_update(
        &mut self,
        update: &PlayerPlaybackTelemetryUpdate,
    ) {
        if let Some(paused) = update.paused {
            self.local_paused = Some(paused);
        }
        if let Some(position_seconds) = update.position_seconds.filter(|value| value.is_finite()) {
            self.local_position = Some(position_seconds);
        }
    }

    fn clear_reconnect_state_restore_validation_state(&mut self) {
        self.reconnect_state_restore_validation_pending = false;
        self.reconnect_state_restore_validation_retry_attempts = 0;
        self.reconnect_state_restore_validation_retry_cooldown_ticks = 0;
        self.reconnect_state_restore_validation_mismatch_notified = false;
        self.reconnect_state_restore_validation_mismatch_seen_in_cycle = false;
        self.reconnect_state_restore_correction_recovery_suppressed_this_cycle = false;
        self.reconnect_state_restore_correction_recovery_reenabled_this_cycle = false;
    }

    fn reconnect_state_restore_correction_policy_mode(
        &self,
    ) -> ReconnectStateRestoreCorrectionPolicyMode {
        self.behavior_config
            .reconnect_state_restore_correction_policy_mode_override
            .unwrap_or(
                if self.behavior_config.reconnect_state_restore_auto_correct {
                    ReconnectStateRestoreCorrectionPolicyMode::AutoCorrect
                } else {
                    ReconnectStateRestoreCorrectionPolicyMode::NotifyOnly
                },
            )
    }

    fn reconnect_state_restore_position_tolerance_seconds_effective(&self) -> f64 {
        let position_tolerance_seconds = self
            .behavior_config
            .reconnect_state_restore_position_tolerance_seconds;
        if position_tolerance_seconds.is_finite() && position_tolerance_seconds >= 0.0 {
            position_tolerance_seconds
        } else {
            SEEK_THRESHOLD_SECONDS
        }
    }

    fn reconnect_state_restore_correction_retry_cooldown_for_failed_attempt(
        &self,
        failed_attempts: u32,
    ) -> u32 {
        let base_cooldown_ticks = self
            .behavior_config
            .reconnect_state_restore_correction_retry_cooldown_ticks;
        if base_cooldown_ticks == 0 {
            return base_cooldown_ticks;
        }
        let use_exponential_backoff = self
            .behavior_config
            .reconnect_state_restore_correction_retry_exponential_backoff;
        let adaptive_cycle_backoff_shift = if self
            .behavior_config
            .reconnect_state_restore_correction_retry_adaptive_cycle_backoff
        {
            self.reconnect_state_restore_correction_consecutive_retry_exhaustions
        } else {
            0
        };
        if !use_exponential_backoff && adaptive_cycle_backoff_shift == 0 {
            return base_cooldown_ticks;
        }

        let max_cooldown_ticks = self
            .behavior_config
            .reconnect_state_restore_correction_retry_max_cooldown_ticks
            .max(base_cooldown_ticks);
        let per_attempt_shift = if use_exponential_backoff {
            failed_attempts.saturating_sub(1)
        } else {
            0
        };
        let shift = per_attempt_shift
            .saturating_add(adaptive_cycle_backoff_shift)
            .min(63);
        let multiplier = 1_u64 << shift;
        let scaled_cooldown_ticks = u64::from(base_cooldown_ticks).saturating_mul(multiplier);
        scaled_cooldown_ticks.min(u64::from(max_cooldown_ticks)) as u32
    }

    fn reconnect_state_restore_correction_effective_retry_max_attempts(&self) -> u32 {
        let configured_max_attempts = self
            .behavior_config
            .reconnect_state_restore_correction_retry_max_attempts;
        if !self
            .behavior_config
            .reconnect_state_restore_correction_retry_adaptive_cycle_budget
        {
            return configured_max_attempts;
        }

        let min_attempts = self
            .behavior_config
            .reconnect_state_restore_correction_retry_adaptive_cycle_budget_min_attempts
            .min(configured_max_attempts);
        configured_max_attempts
            .saturating_sub(self.reconnect_state_restore_correction_consecutive_retry_exhaustions)
            .max(min_attempts)
    }

    fn note_reconnect_state_restore_correction_retry_exhaustion(&mut self) {
        self.reconnect_state_restore_correction_consecutive_retry_exhaustions = self
            .reconnect_state_restore_correction_consecutive_retry_exhaustions
            .saturating_add(1);
    }

    fn reset_reconnect_state_restore_correction_retry_exhaustions(&mut self) {
        self.reconnect_state_restore_correction_consecutive_retry_exhaustions = 0;
    }

    fn activate_reconnect_state_restore_correction_recovery_cooldown_if_configured(
        &mut self,
    ) -> bool {
        let recovery_cooldown_reconnect_cycles = self
            .behavior_config
            .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles;
        if recovery_cooldown_reconnect_cycles == 0 {
            return false;
        }
        self.reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining =
            recovery_cooldown_reconnect_cycles;
        self.reconnect_state_restore_correction_recovery_reenable_notification_pending = true;
        self.reconnect_state_restore_correction_recovery_reenabled_this_cycle = false;
        true
    }

    fn begin_reconnect_state_restore_validation_cycle(&mut self) {
        self.reconnect_state_restore_correction_metrics
            .validation_cycles_started = self
            .reconnect_state_restore_correction_metrics
            .validation_cycles_started
            .saturating_add(1);
        self.reconnect_state_restore_correction_recovery_suppressed_this_cycle = false;
        self.reconnect_state_restore_correction_recovery_reenabled_this_cycle = false;
        if self.reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining > 0
        {
            self.reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining = self
                .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining
                .saturating_sub(1);
            self.reconnect_state_restore_correction_recovery_suppressed_this_cycle = true;
            return;
        }
        if self.reconnect_state_restore_correction_recovery_reenable_notification_pending {
            self.reconnect_state_restore_correction_recovery_reenabled_this_cycle = true;
            self.reconnect_state_restore_correction_recovery_reenable_notification_pending = false;
        }
    }

    fn defer_reconnect_state_restore_validation_after_correction_failure(
        &mut self,
    ) -> Option<ReconnectTransitionNotification> {
        if !self.reconnect_state_restore_validation_pending {
            return None;
        }

        let retry_max_attempts =
            self.reconnect_state_restore_correction_effective_retry_max_attempts();
        let correction_policy_mode = self.reconnect_state_restore_correction_policy_mode();
        self.reconnect_state_restore_validation_retry_attempts = self
            .reconnect_state_restore_validation_retry_attempts
            .saturating_add(1);
        let failed_attempts = self.reconnect_state_restore_validation_retry_attempts;
        if failed_attempts > retry_max_attempts {
            self.note_reconnect_state_restore_correction_retry_exhaustion();
            self.reconnect_state_restore_correction_metrics
                .correction_retry_exhaustions = self
                .reconnect_state_restore_correction_metrics
                .correction_retry_exhaustions
                .saturating_add(1);
            self.activate_reconnect_state_restore_correction_recovery_cooldown_if_configured();
            self.clear_reconnect_state_restore_validation_state();
            return Some(
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRetriesExhausted {
                    attempts: failed_attempts,
                    max_attempts: retry_max_attempts,
                },
            );
        }

        let cooldown_ticks = self
            .reconnect_state_restore_correction_retry_cooldown_for_failed_attempt(failed_attempts);
        self.reconnect_state_restore_validation_retry_cooldown_ticks = cooldown_ticks;
        self.reconnect_state_restore_correction_metrics
            .correction_retries_scheduled = self
            .reconnect_state_restore_correction_metrics
            .correction_retries_scheduled
            .saturating_add(1);
        if matches!(
            correction_policy_mode,
            ReconnectStateRestoreCorrectionPolicyMode::WarnOnlyOnExhaustion
        ) {
            return None;
        }
        Some(
            ReconnectTransitionNotification::StateRestoreValidationCorrectionRetryScheduled {
                attempt: failed_attempts,
                max_attempts: retry_max_attempts,
                cooldown_ticks,
            },
        )
    }

    fn complete_reconnect_state_restore_validation_after_success(&mut self) {
        self.reconnect_state_restore_correction_metrics
            .validation_cycles_completed_with_successful_correction = self
            .reconnect_state_restore_correction_metrics
            .validation_cycles_completed_with_successful_correction
            .saturating_add(1);
        if self.reconnect_state_restore_validation_mismatch_seen_in_cycle {
            let decay = self
                .behavior_config
                .reconnect_state_restore_correction_disable_after_mismatch_decay_on_success;
            if decay > 0 {
                self.reconnect_state_restore_correction_consecutive_mismatch_cycles = self
                    .reconnect_state_restore_correction_consecutive_mismatch_cycles
                    .saturating_sub(decay);
            }
        }
        self.reset_reconnect_state_restore_correction_retry_exhaustions();
        self.reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining = 0;
        self.reconnect_state_restore_correction_recovery_reenable_notification_pending = false;
        self.clear_reconnect_state_restore_validation_state();
    }

    fn apply_successful_reconnect_state_restore_validation_action(
        &mut self,
        action: &ClientRuntimeAction,
    ) {
        match action {
            ClientRuntimeAction::SetPaused(paused) => {
                self.local_paused = Some(*paused);
            }
            ClientRuntimeAction::SetPosition(position_seconds) => {
                if position_seconds.is_finite() {
                    self.local_position = Some(*position_seconds);
                }
            }
            _ => {}
        }
    }

    pub fn dispatch_runtime_actions<P, C>(
        actions: &[ClientRuntimeAction],
        player: &mut P,
        control: &mut C,
    ) -> Result<(), PlayerError>
    where
        P: PlayerAdapter,
        C: ClientRuntimeControl,
    {
        for action in actions {
            match action {
                ClientRuntimeAction::SetPaused(paused) => {
                    player.set_paused(*paused)?;
                }
                ClientRuntimeAction::RequestUserList => {
                    control.request_user_list();
                }
                ClientRuntimeAction::SetRoom { room } => {
                    control.set_room(room.clone());
                }
                ClientRuntimeAction::SetReady {
                    ready,
                    manually_initiated,
                } => {
                    control.set_ready(*ready, *manually_initiated);
                }
                ClientRuntimeAction::SetReadyForUser {
                    ready,
                    manually_initiated,
                    username,
                } => {
                    control.set_ready_for_user(*ready, *manually_initiated, username.clone());
                }
                ClientRuntimeAction::SetFile { file_payload } => {
                    control.set_file(file_payload.clone());
                }
                ClientRuntimeAction::SetPlaylist { files } => {
                    control.set_playlist(files.clone());
                }
                ClientRuntimeAction::SetPlaylistIndex { index } => {
                    control.set_playlist_index(*index);
                }
                ClientRuntimeAction::RequestControllerAuth { room, password } => {
                    control.request_controller_auth(room.clone(), password.clone());
                }
                ClientRuntimeAction::SendChat { message } => {
                    control.send_chat(message.clone());
                }
                ClientRuntimeAction::NotifyChat(notification) => {
                    control.notify_chat(notification.clone());
                }
                ClientRuntimeAction::NotifyControllerAuthTransition(notification) => {
                    control.notify_controller_auth_transition(notification.clone());
                }
                ClientRuntimeAction::NotifyUserChange(notification) => {
                    control.notify_user_change(notification.clone());
                }
                ClientRuntimeAction::NotifyReconnectTransition(notification) => {
                    control.notify_reconnect_transition(notification.clone());
                }
                ClientRuntimeAction::NotifyAutoplayCountdown(notification) => {
                    control.notify_autoplay_countdown(notification.clone());
                }
                ClientRuntimeAction::SetPosition(position) => {
                    player.set_position(*position)?;
                }
                ClientRuntimeAction::SetPlaybackRate(rate) => {
                    player.set_playback_rate(*rate)?;
                }
                ClientRuntimeAction::ScheduleReconnect { delay_seconds } => {
                    control.schedule_reconnect(*delay_seconds);
                }
                ClientRuntimeAction::StopReconnect => {
                    control.stop_reconnect();
                }
            }
        }
        Ok(())
    }

    pub fn apply_hello_json(&mut self, json_line: &str) -> Result<(), ProtocolError> {
        let message = decode_message_line(json_line)?;
        let hello = extract_hello_from_message(message)?;
        self.apply_hello(hello);
        Ok(())
    }

    pub fn apply_protocol_message(
        &mut self,
        message: ProtocolMessage,
    ) -> Result<(), ProtocolError> {
        self.apply_protocol_message_with_now(message, None)
    }

    pub fn apply_protocol_message_at(
        &mut self,
        message: ProtocolMessage,
        now_seconds: f64,
    ) -> Result<(), ProtocolError> {
        self.apply_protocol_message_with_now(message, Some(now_seconds))
    }

    fn apply_protocol_message_with_now(
        &mut self,
        message: ProtocolMessage,
        now_seconds: Option<f64>,
    ) -> Result<(), ProtocolError> {
        match message {
            ProtocolMessage::Hello(hello_message) => self.apply_hello(hello_message.hello),
            ProtocolMessage::Set(set_message) => self.apply_set(set_message.set, now_seconds),
            ProtocolMessage::List(list_message) => self.apply_list(list_message.list),
            ProtocolMessage::State(state_message) => self.apply_state(state_message.state),
            ProtocolMessage::Chat(chat_message) => self.apply_chat(chat_message.chat),
            ProtocolMessage::Error(_) | ProtocolMessage::Tls(_) => {}
        }
        Ok(())
    }

    pub fn apply_message_json(&mut self, json_line: &str) -> Result<(), ProtocolError> {
        let message = decode_message_line(json_line)?;
        self.apply_protocol_message(message)
    }

    pub fn apply_message_json_at(
        &mut self,
        json_line: &str,
        now_seconds: f64,
    ) -> Result<(), ProtocolError> {
        let message = decode_message_line(json_line)?;
        self.apply_protocol_message_at(message, now_seconds)
    }

    pub fn user_room(&self, username: &str) -> Option<&str> {
        self.user_views
            .get(username)
            .and_then(|user| user.room.as_deref())
    }

    pub fn room_names(&self) -> Vec<String> {
        let mut rooms = self.known_rooms.iter().cloned().collect::<Vec<_>>();
        if let Some(current_room) = self
            .room
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            && !rooms.iter().any(|room| room == current_room)
        {
            rooms.push(current_room.to_owned());
            rooms.sort();
        }
        rooms
    }

    pub fn usernames_in_room(&self, room_name: &str) -> Vec<String> {
        self.user_views
            .iter()
            .filter_map(|(username, user)| {
                (!username.trim().is_empty() && user.room.as_deref() == Some(room_name))
                    .then_some(username.clone())
            })
            .collect()
    }

    pub fn user_ready(&self, username: &str) -> Option<bool> {
        self.user_views.get(username).and_then(|user| user.ready)
    }

    pub fn user_has_file(&self, username: &str) -> Option<bool> {
        self.user_views.get(username).map(|user| user.has_file)
    }

    pub fn user_file_name(&self, username: &str) -> Option<&str> {
        self.user_views
            .get(username)
            .and_then(|user| user.file_name.as_deref())
    }

    pub fn user_file_size(&self, username: &str) -> Option<&Value> {
        self.user_views
            .get(username)
            .and_then(|user| user.file_size.as_ref())
    }

    pub fn user_file_duration(&self, username: &str) -> Option<&Value> {
        self.user_views
            .get(username)
            .and_then(|user| user.file_duration.as_ref())
    }

    pub fn user_controller(&self, username: &str) -> Option<bool> {
        self.user_views.get(username).map(|user| user.controller)
    }

    pub fn file_differences_for_user(&self, username: &str) -> Option<FileDifferenceSummary> {
        let current_username = self.username.as_deref()?;
        let current_user = self.user_views.get(current_username)?;
        let other_user = self.user_views.get(username)?;
        if current_user.room.is_none() || current_user.room != other_user.room {
            return None;
        }
        Self::file_difference_summary_for_users(current_user, other_user, self)
    }

    pub fn file_differences_for_room(&self, room_name: &str) -> Option<FileDifferenceSummary> {
        let current_username = self.username.as_deref()?;
        let current_user = self.user_views.get(current_username)?;
        if current_user.room.as_deref() != Some(room_name) {
            return None;
        }

        let mut summary = FileDifferenceSummary::default();
        let mut compared_any = false;
        for (username, user_view) in &self.user_views {
            if username == current_username {
                continue;
            }
            if user_view.room.as_deref() != Some(room_name) {
                continue;
            }
            if let Some(user_summary) =
                Self::file_difference_summary_for_users(current_user, user_view, self)
            {
                compared_any = true;
                summary.filename |= user_summary.filename;
                summary.filesize |= user_summary.filesize;
                summary.fileduration |= user_summary.fileduration;
            }
        }

        if compared_any { Some(summary) } else { None }
    }

    pub fn file_differences_for_current_room(&self) -> Option<FileDifferenceSummary> {
        let room_name = self.room.as_deref()?;
        self.file_differences_for_room(room_name)
    }

    pub fn same_filename_legacy_compatible(left: &str, right: &str) -> bool {
        Self::same_filename_legacy_like(left, right)
    }

    pub fn same_filesize_legacy_compatible(left: &Value, right: &Value) -> bool {
        Self::same_filesize_legacy_like(left, right)
    }

    pub fn same_fileduration_legacy_compatible(left: f64, right: f64) -> bool {
        Self::same_fileduration_legacy_compatible_with_overrides(
            left,
            right,
            LEGACY_SHOW_DURATION_NOTIFICATION,
            LEGACY_DIFFERENT_DURATION_THRESHOLD_SECONDS,
        )
    }

    pub fn same_fileduration_legacy_compatible_with_overrides(
        left: f64,
        right: f64,
        show_duration_notification: bool,
        different_duration_threshold_seconds: f64,
    ) -> bool {
        Self::same_fileduration_legacy_like(
            left,
            right,
            show_duration_notification,
            different_duration_threshold_seconds,
        )
    }

    pub fn same_fileduration_with_readiness_autoplay_config(&self, left: f64, right: f64) -> bool {
        Self::same_fileduration_legacy_compatible_with_overrides(
            left,
            right,
            self.readiness_autoplay_config.show_duration_notification,
            self.readiness_autoplay_config
                .different_duration_threshold_seconds,
        )
    }

    pub fn sanitize_outbound_file_payload_legacy_compatible(
        file_payload: &Value,
        filename_privacy_mode: PrivacyMode,
        filesize_privacy_mode: PrivacyMode,
    ) -> Option<Value> {
        let Value::Object(file_map) = file_payload else {
            return None;
        };

        let mut sanitized = file_map.clone();
        sanitized.remove("path");

        if let Some(name_value) = file_map.get("name") {
            let sanitized_name =
                Self::filename_with_privacy_mode_legacy_like(name_value, filename_privacy_mode);
            if let Some(sanitized_name) = sanitized_name {
                sanitized.insert("name".to_owned(), Value::String(sanitized_name));
            }
        }

        if let Some(size_value) = file_map.get("size") {
            let sanitized_size =
                Self::filesize_with_privacy_mode_legacy_like(size_value, filesize_privacy_mode);
            if let Some(sanitized_size) = sanitized_size {
                sanitized.insert("size".to_owned(), sanitized_size);
            }
        }

        Some(Value::Object(sanitized))
    }

    pub fn runtime_actions_for_local_file_publish_legacy_compatible(
        &mut self,
        file_payload: &Value,
        filename_privacy_mode: PrivacyMode,
        filesize_privacy_mode: PrivacyMode,
    ) -> Vec<ClientRuntimeAction> {
        let Some(sanitized_payload) = Self::sanitize_outbound_file_payload_legacy_compatible(
            file_payload,
            filename_privacy_mode,
            filesize_privacy_mode,
        ) else {
            return Vec::new();
        };

        if let Some(username) = self.username.clone() {
            let (has_file, file_name, file_size, file_duration) =
                Self::list_payload_file_info(Some(&sanitized_payload));
            self.set_user_file_info(&username, has_file, file_name, file_size, file_duration);
        }

        vec![ClientRuntimeAction::SetFile {
            file_payload: sanitized_payload,
        }]
    }

    pub fn room_playlist(&self, room_name: &str) -> Option<&RoomPlaylistView> {
        self.room_playlists.get(room_name)
    }

    pub fn current_room_playlist(&self) -> Option<&RoomPlaylistView> {
        self.room
            .as_deref()
            .and_then(|room_name| self.room_playlists.get(room_name))
    }

    pub fn room_playstate(&self, room_name: &str) -> Option<&RoomPlaystateView> {
        self.room_playstates.get(room_name)
    }

    pub fn current_room_playstate(&self) -> Option<&RoomPlaystateView> {
        self.room
            .as_deref()
            .and_then(|room_name| self.room_playstates.get(room_name))
    }

    pub fn client_ignoring_on_the_fly(&self) -> u32 {
        self.client_ignoring_on_the_fly
    }

    pub fn server_ignoring_on_the_fly(&self) -> u32 {
        self.server_ignoring_on_the_fly
    }

    pub fn desync_config(&self) -> &DesyncCorrectionConfig {
        &self.desync_config
    }

    pub fn desync_config_mut(&mut self) -> &mut DesyncCorrectionConfig {
        &mut self.desync_config
    }

    pub fn reconnect_policy(&self) -> &ReconnectPolicyConfig {
        &self.reconnect_policy
    }

    pub fn reconnect_policy_mut(&mut self) -> &mut ReconnectPolicyConfig {
        &mut self.reconnect_policy
    }

    pub fn behavior_config(&self) -> &SessionBehaviorConfig {
        &self.behavior_config
    }

    pub fn behavior_config_mut(&mut self) -> &mut SessionBehaviorConfig {
        &mut self.behavior_config
    }

    pub fn reconnect_state_restore_correction_metrics(
        &self,
    ) -> &ReconnectStateRestoreCorrectionMetrics {
        &self.reconnect_state_restore_correction_metrics
    }

    pub fn reconnect_state_restore_correction_state_snapshot(
        &self,
    ) -> ReconnectStateRestoreCorrectionStateSnapshot {
        ReconnectStateRestoreCorrectionStateSnapshot {
            validation_pending: self.reconnect_state_restore_validation_pending,
            retry_attempts: self.reconnect_state_restore_validation_retry_attempts,
            retry_cooldown_ticks: self.reconnect_state_restore_validation_retry_cooldown_ticks,
            mismatch_notified_in_cycle: self.reconnect_state_restore_validation_mismatch_notified,
            mismatch_seen_in_cycle: self.reconnect_state_restore_validation_mismatch_seen_in_cycle,
            effective_policy_mode: self.reconnect_state_restore_correction_policy_mode(),
            position_tolerance_seconds: self
                .reconnect_state_restore_position_tolerance_seconds_effective(),
            effective_retry_max_attempts: self
                .reconnect_state_restore_correction_effective_retry_max_attempts(),
            consecutive_mismatch_cycles: self
                .reconnect_state_restore_correction_consecutive_mismatch_cycles,
            consecutive_retry_exhaustions: self
                .reconnect_state_restore_correction_consecutive_retry_exhaustions,
            recovery_cooldown_reconnect_cycles_remaining: self
                .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining,
            correction_suppressed_for_recovery_cycle: self
                .reconnect_state_restore_correction_recovery_suppressed_this_cycle,
            correction_reenabled_for_recovery_cycle: self
                .reconnect_state_restore_correction_recovery_reenabled_this_cycle,
        }
    }

    pub fn readiness_autoplay_config(&self) -> &ReadinessAutoplayConfig {
        &self.readiness_autoplay_config
    }

    pub fn readiness_autoplay_config_mut(&mut self) -> &mut ReadinessAutoplayConfig {
        &mut self.readiness_autoplay_config
    }

    pub fn chat_config(&self) -> &ChatConfig {
        &self.chat_config
    }

    pub fn chat_config_mut(&mut self) -> &mut ChatConfig {
        &mut self.chat_config
    }

    pub fn last_paused_on_leave_at_seconds(&self) -> Option<f64> {
        self.last_paused_on_leave_at_seconds
    }

    pub fn server_readiness_supported(&self) -> Option<bool> {
        self.server_readiness_supported
    }

    pub fn server_chat_supported(&self) -> Option<bool> {
        self.server_chat_supported
    }

    pub fn local_can_control(&self) -> Option<bool> {
        let username = self.username.as_deref()?;
        let room_name = self.room.as_deref()?;
        if !Self::is_controlled_room_name(room_name) {
            return Some(true);
        }
        Some(self.user_controller(username).unwrap_or(false))
    }

    pub fn noncontroller_event_hide_from_osd_legacy_compatible(&self, username: &str) -> bool {
        !self.behavior_config.show_noncontroller_osd && self.user_controller(username) != Some(true)
    }

    fn show_user_change_event_on_osd_legacy_compatible(
        &self,
        current_room: Option<&str>,
        previous_room: Option<&str>,
        username: &str,
    ) -> bool {
        let local_room = self.room.as_deref();
        let room_matches_local = local_room.is_some_and(|local_room| {
            current_room == Some(local_room) || previous_room == Some(local_room)
        });

        let mut show_on_osd = if room_matches_local {
            self.behavior_config.show_osd_warnings
        } else {
            self.behavior_config.show_different_room_osd
        };

        if !self.behavior_config.show_noncontroller_osd
            && self.user_controller(username) != Some(true)
        {
            show_on_osd = false;
        }

        show_on_osd
    }

    fn queue_user_left_notification_if_relevant(
        &mut self,
        username: &str,
        previous_user_view: Option<ClientUserView>,
    ) {
        let Some(previous_user_view) = previous_user_view else {
            return;
        };

        let previous_room = previous_user_view.room.as_deref();
        let local_room = self.room.as_deref();
        let show_on_osd = if local_room == previous_room {
            self.behavior_config.show_same_room_osd
        } else {
            self.behavior_config.show_different_room_osd
        };

        self.pending_user_change_notifications
            .push(UserChangeNotification::Left {
                username: username.to_owned(),
                hide_from_osd: !show_on_osd,
            });
    }

    fn queue_user_change_notification_if_relevant(
        &mut self,
        username: &str,
        previous_user_view: Option<ClientUserView>,
    ) {
        let Some(current_user_view) = self.user_views.get(username).cloned() else {
            return;
        };
        let Some(room_name) = current_user_view.room.clone() else {
            return;
        };

        let room_changed = previous_user_view
            .as_ref()
            .and_then(|view| view.room.as_deref())
            != Some(room_name.as_str());
        let file_changed = match previous_user_view.as_ref() {
            Some(previous_user_view) => {
                previous_user_view.has_file != current_user_view.has_file
                    || previous_user_view.file_name != current_user_view.file_name
                    || previous_user_view.file_size != current_user_view.file_size
                    || previous_user_view.file_duration != current_user_view.file_duration
            }
            None => current_user_view.has_file,
        };

        if !room_changed && !file_changed {
            return;
        }

        let previous_room = previous_user_view
            .as_ref()
            .and_then(|view| view.room.as_deref());
        let show_on_osd = self.show_user_change_event_on_osd_legacy_compatible(
            Some(room_name.as_str()),
            previous_room,
            username,
        );
        let hide_from_osd = !show_on_osd;
        if current_user_view.has_file {
            let include_room_addendum = self.room.as_deref() != Some(room_name.as_str());
            self.pending_user_change_notifications
                .push(UserChangeNotification::Playing {
                    username: username.to_owned(),
                    room: room_name,
                    file_name: current_user_view.file_name,
                    file_duration: current_user_view.file_duration,
                    include_room_addendum,
                    hide_from_osd,
                });
        } else if room_changed {
            self.pending_user_change_notifications
                .push(UserChangeNotification::Joined {
                    username: username.to_owned(),
                    room: room_name,
                    hide_from_osd,
                });
        }
    }

    pub fn remember_control_password_for_room(&mut self, room_name: &str, password: &str) {
        if !Self::is_controlled_room_name(room_name) {
            return;
        }

        let normalized_password = Self::normalize_control_password_legacy_compatible(password);
        if normalized_password.is_empty() {
            return;
        }

        self.controlled_room_passwords
            .insert(room_name.to_owned(), normalized_password);
    }

    pub fn autoplay_enabled(&self) -> bool {
        self.autoplay_enabled
    }

    pub fn set_autoplay_enabled(&mut self, enabled: bool) {
        self.autoplay_enabled = enabled;
    }

    pub fn autoplay_timer_is_running(&self) -> bool {
        self.autoplay_timer_running
    }

    pub fn autoplay_time_left_seconds(&self) -> f64 {
        self.autoplay_time_left_seconds
    }

    pub fn is_playing_music(&self) -> bool {
        self.current_room_playlist_file_name()
            .is_some_and(Self::is_music_file_name)
    }

    fn loop_single_files_enabled_legacy_compatible(&self) -> bool {
        self.behavior_config.loop_single_files || self.is_playing_music()
    }

    fn loop_at_end_of_playlist_enabled_legacy_compatible(&self) -> bool {
        self.behavior_config.loop_at_end_of_playlist || self.is_playing_music()
    }

    fn playlist_target_switch_allowed_legacy_compatible(&self, file_name: &str) -> bool {
        if !Self::is_url(file_name) {
            return true;
        }
        self.uri_is_trusted_legacy_compatible(file_name)
    }

    fn uri_is_trusted_legacy_compatible(&self, uri: &str) -> bool {
        let Some((host, path)) = Self::parse_trustable_web_uri_host_and_path_legacy_compatible(uri)
        else {
            return false;
        };

        if !self.behavior_config.only_switch_to_trusted_domains {
            return true;
        }

        for trusted_entry in &self.behavior_config.trusted_domains {
            let trusted_entry = trusted_entry.trim();
            if trusted_entry.is_empty() {
                continue;
            }
            let (trusted_domain, required_path_prefix) =
                trusted_entry.split_once('/').unwrap_or((trusted_entry, ""));
            let trusted_domain = trusted_domain.trim().to_ascii_lowercase();
            if trusted_domain.is_empty() {
                continue;
            }
            if !Self::trusted_domain_matches_host_legacy_compatible(&host, &trusted_domain) {
                continue;
            }
            if !required_path_prefix.is_empty() {
                let path_prefix = format!("/{required_path_prefix}");
                if !path.starts_with(&path_prefix) {
                    continue;
                }
            }
            return true;
        }
        false
    }

    fn parse_trustable_web_uri_host_and_path_legacy_compatible(
        uri: &str,
    ) -> Option<(String, String)> {
        let uri = uri.trim();
        let authority_and_path = if let Some(value) = uri.strip_prefix("http://") {
            value
        } else if let Some(value) = uri.strip_prefix("https://") {
            value
        } else {
            return None;
        };
        if authority_and_path.is_empty() {
            return None;
        }

        let (authority, path_tail) = authority_and_path
            .split_once('/')
            .unwrap_or((authority_and_path, ""));
        if authority.is_empty() {
            return None;
        }

        let authority = authority
            .rsplit_once('@')
            .map_or(authority, |(_, value)| value);
        if authority.is_empty() {
            return None;
        }

        let host = authority
            .split(':')
            .next()
            .map(str::trim)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if host.is_empty() {
            return None;
        }

        let path_with_query = if path_tail.is_empty() {
            "/".to_owned()
        } else {
            format!("/{path_tail}")
        };
        let path = path_with_query
            .split(['?', '#'])
            .next()
            .unwrap_or("/")
            .to_owned();
        Some((host, path))
    }

    fn trusted_domain_matches_host_legacy_compatible(host: &str, trusted_domain: &str) -> bool {
        if host == trusted_domain || host == format!("www.{trusted_domain}") {
            return true;
        }
        if !trusted_domain.contains('*') {
            return false;
        }

        let host_parts = host.split('.').collect::<Vec<_>>();
        let pattern_parts = trusted_domain.split('.').collect::<Vec<_>>();
        if host_parts.len() != pattern_parts.len() {
            return false;
        }
        host_parts
            .iter()
            .zip(pattern_parts.iter())
            .all(|(host_part, pattern_part)| {
                if *pattern_part == "*" {
                    !host_part.is_empty()
                } else {
                    host_part.eq_ignore_ascii_case(pattern_part)
                }
            })
    }

    fn capture_playlist_undo_snapshot_legacy_compatible(
        &mut self,
        room_name: &str,
        current_files: &[String],
        new_files: &[String],
    ) {
        if current_files == new_files {
            return;
        }
        if self
            .playlist_undo_snapshots
            .get(room_name)
            .is_some_and(|snapshot| snapshot == current_files)
        {
            return;
        }
        self.playlist_undo_snapshots
            .insert(room_name.to_owned(), current_files.to_vec());
    }

    fn local_playlist_target_index_from_changed_playlist_legacy_compatible(
        current_files: &[String],
        current_index: Option<usize>,
        new_files: &[String],
    ) -> usize {
        let Some(current_index) = current_index else {
            return 0;
        };
        if new_files.len() <= 1 {
            return 0;
        }

        let mut index = current_index;
        while index <= current_files.len() {
            if let Some(file_name) = current_files.get(index)
                && let Some(valid_index) = new_files.iter().position(|entry| entry == file_name)
            {
                return valid_index;
            }
            index = index.saturating_add(1);
        }

        let mut index = current_index;
        while index > 0 {
            if let Some(file_name) = current_files.get(index)
                && let Some(valid_index) = new_files.iter().position(|entry| entry == file_name)
            {
                return if valid_index < new_files.len().saturating_sub(1) {
                    valid_index.saturating_add(1)
                } else {
                    valid_index
                };
            }
            index = index.saturating_sub(1);
        }
        0
    }

    fn next_playlist_shuffle_seed_legacy_compatible(
        &mut self,
        files: &[String],
        current_index: usize,
        shuffle_scope_remaining: bool,
    ) -> u64 {
        let mut hasher = Sha256::new();
        hasher.update(if shuffle_scope_remaining {
            &b"remaining"[..]
        } else {
            &b"entire"[..]
        });
        hasher.update((current_index as u64).to_le_bytes());
        hasher.update(self.playlist_shuffle_nonce.to_le_bytes());
        for file_name in files {
            hasher.update(file_name.as_bytes());
            hasher.update([0]);
        }
        self.playlist_shuffle_nonce = self.playlist_shuffle_nonce.wrapping_add(1);

        let digest = hasher.finalize();
        let mut seed_bytes = [0u8; 8];
        seed_bytes.copy_from_slice(&digest[..8]);
        let seed = u64::from_le_bytes(seed_bytes);
        if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        }
    }

    fn next_shuffle_state_legacy_compatible(state: &mut u64) -> u64 {
        *state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *state
    }

    fn shuffle_playlist_slice_in_place_legacy_compatible(files: &mut [String], seed: u64) {
        if files.len() <= 1 {
            return;
        }

        let mut state = seed;
        for index in (1..files.len()).rev() {
            let random_value = Self::next_shuffle_state_legacy_compatible(&mut state);
            let swap_index = (random_value as usize) % (index + 1);
            files.swap(index, swap_index);
        }
    }

    pub fn recently_advanced(&self, now_seconds: f64) -> bool {
        let threshold_seconds =
            self.readiness_autoplay_config.autoplay_delay_seconds + RECENTLY_ADVANCED_GRACE_SECONDS;
        self.last_advanced_at_seconds
            .is_some_and(|last_advanced_at_seconds| {
                let elapsed = now_seconds - last_advanced_at_seconds;
                elapsed >= 0.0 && elapsed < threshold_seconds
            })
    }

    pub fn plan_reconnect_retry(&mut self, retries: u32) -> ReconnectRetryDecision {
        self.reset_sync_state_for_reconnect();

        if retries > self.reconnect_policy.max_retries {
            self.reconnect_in_progress = false;
            self.reconnect_connected_intent = false;
            return ReconnectRetryDecision {
                should_retry: false,
                delay_seconds: None,
                should_reset_state: true,
            };
        }

        let exponent = retries.min(self.reconnect_policy.max_backoff_exponent);
        let delay_seconds = self.reconnect_policy.base_delay_seconds * 2_f64.powi(exponent as i32);
        self.reconnect_in_progress = true;

        ReconnectRetryDecision {
            should_retry: true,
            delay_seconds: Some(delay_seconds),
            should_reset_state: true,
        }
    }

    pub fn runtime_actions_for_reconnect_retry(
        &mut self,
        retries: u32,
    ) -> Vec<ClientRuntimeAction> {
        let decision = self.plan_reconnect_retry(retries);
        if decision.should_retry {
            if let Some(delay_seconds) = decision.delay_seconds {
                return vec![
                    ClientRuntimeAction::NotifyReconnectTransition(
                        ReconnectTransitionNotification::Attempting {
                            retries,
                            delay_seconds,
                        },
                    ),
                    ClientRuntimeAction::ScheduleReconnect { delay_seconds },
                ];
            }
            return Vec::new();
        }
        vec![
            ClientRuntimeAction::NotifyReconnectTransition(
                ReconnectTransitionNotification::Disconnected,
            ),
            ClientRuntimeAction::StopReconnect,
        ]
    }

    pub fn runtime_actions_for_reconnect_transition_if_needed(
        &mut self,
    ) -> Vec<ClientRuntimeAction> {
        if !self.reconnect_connected_intent {
            return Vec::new();
        }
        self.reconnect_connected_intent = false;
        vec![ClientRuntimeAction::NotifyReconnectTransition(
            ReconnectTransitionNotification::Connected,
        )]
    }

    pub fn runtime_actions_for_controller_auth_notifications_if_needed(
        &mut self,
    ) -> Vec<ClientRuntimeAction> {
        self.pending_controller_auth_notifications
            .drain(..)
            .map(ClientRuntimeAction::NotifyControllerAuthTransition)
            .collect()
    }

    pub fn runtime_actions_for_chat_notifications_if_needed(&mut self) -> Vec<ClientRuntimeAction> {
        self.pending_chat_notifications
            .drain(..)
            .map(ClientRuntimeAction::NotifyChat)
            .collect()
    }

    pub fn runtime_actions_for_user_change_notifications_if_needed(
        &mut self,
    ) -> Vec<ClientRuntimeAction> {
        self.pending_user_change_notifications
            .drain(..)
            .map(ClientRuntimeAction::NotifyUserChange)
            .collect()
    }

    pub fn runtime_actions_for_reconnect_state_restore_if_needed(
        &mut self,
    ) -> Vec<ClientRuntimeAction> {
        let mut actions = Vec::new();

        if let Some(ready) = self.reconnect_ready_restore_intent.take() {
            actions.push(ClientRuntimeAction::SetReady {
                ready,
                manually_initiated: false,
            });
        }

        if let Some(file_payload) = self.reconnect_file_restore_intent.take() {
            actions.push(ClientRuntimeAction::SetFile { file_payload });
        }

        if !actions.is_empty() {
            self.reconnect_state_restore_validation_pending = true;
            self.reconnect_state_restore_validation_retry_attempts = 0;
            self.reconnect_state_restore_validation_retry_cooldown_ticks = 0;
            self.reconnect_state_restore_validation_mismatch_notified = false;
            self.reconnect_state_restore_validation_mismatch_seen_in_cycle = false;
            self.begin_reconnect_state_restore_validation_cycle();
            actions.insert(
                0,
                ClientRuntimeAction::NotifyReconnectTransition(
                    ReconnectTransitionNotification::RestoringState,
                ),
            );
        }

        actions
    }

    pub fn runtime_actions_for_reconnect_state_restore_validation_if_needed(
        &mut self,
    ) -> Vec<ClientRuntimeAction> {
        if !self.reconnect_state_restore_validation_pending {
            return Vec::new();
        }
        if self.reconnect_state_restore_validation_retry_cooldown_ticks > 0 {
            self.reconnect_state_restore_validation_retry_cooldown_ticks = self
                .reconnect_state_restore_validation_retry_cooldown_ticks
                .saturating_sub(1);
            return Vec::new();
        }

        let Some(room_playstate) = self.current_room_playstate() else {
            return Vec::new();
        };
        let (Some(room_paused), Some(room_position)) =
            (room_playstate.paused, room_playstate.position)
        else {
            return Vec::new();
        };
        let (Some(local_paused), Some(local_position)) = (self.local_paused, self.local_position)
        else {
            return Vec::new();
        };

        let position_diff_seconds = (local_position - room_position).abs();
        let pause_matches = local_paused == room_paused;
        let position_tolerance_seconds =
            self.reconnect_state_restore_position_tolerance_seconds_effective();
        let position_matches = position_diff_seconds <= position_tolerance_seconds;
        if pause_matches && position_matches {
            self.reconnect_state_restore_correction_metrics
                .validation_cycles_completed_without_mismatch = self
                .reconnect_state_restore_correction_metrics
                .validation_cycles_completed_without_mismatch
                .saturating_add(1);
            self.reconnect_state_restore_correction_consecutive_mismatch_cycles = 0;
            self.reset_reconnect_state_restore_correction_retry_exhaustions();
            self.reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining =
                0;
            self.reconnect_state_restore_correction_recovery_reenable_notification_pending = false;
            self.clear_reconnect_state_restore_validation_state();
            return Vec::new();
        }

        let correction_policy_mode = self.reconnect_state_restore_correction_policy_mode();
        let correction_suppressed_for_recovery_cycle =
            self.reconnect_state_restore_correction_recovery_suppressed_this_cycle;
        let correction_reenabled_for_this_cycle =
            self.reconnect_state_restore_correction_recovery_reenabled_this_cycle;
        if !self.reconnect_state_restore_validation_mismatch_seen_in_cycle
            && !correction_suppressed_for_recovery_cycle
        {
            self.reconnect_state_restore_validation_mismatch_seen_in_cycle = true;
            self.reconnect_state_restore_correction_metrics
                .mismatch_cycles_detected = self
                .reconnect_state_restore_correction_metrics
                .mismatch_cycles_detected
                .saturating_add(1);
            self.reconnect_state_restore_correction_consecutive_mismatch_cycles = self
                .reconnect_state_restore_correction_consecutive_mismatch_cycles
                .saturating_add(1);
        }
        let consecutive_mismatch_cycles =
            self.reconnect_state_restore_correction_consecutive_mismatch_cycles;
        let disable_after_mismatch_cycles = self
            .behavior_config
            .reconnect_state_restore_correction_disable_after_mismatch_cycles;
        let disable_correction_due_to_repeated_mismatches = matches!(
            correction_policy_mode,
            ReconnectStateRestoreCorrectionPolicyMode::DisableAfterNMismatches
        ) && disable_after_mismatch_cycles > 0
            && consecutive_mismatch_cycles >= disable_after_mismatch_cycles;
        let mut actions = Vec::new();
        let should_emit_mismatch_notification = !matches!(
            correction_policy_mode,
            ReconnectStateRestoreCorrectionPolicyMode::WarnOnlyOnExhaustion
        ) && !disable_correction_due_to_repeated_mismatches;
        if correction_reenabled_for_this_cycle {
            self.reconnect_state_restore_correction_metrics
                .correction_recovery_cooldown_reenabled_cycles = self
                .reconnect_state_restore_correction_metrics
                .correction_recovery_cooldown_reenabled_cycles
                .saturating_add(1);
            actions.push(ClientRuntimeAction::NotifyReconnectTransition(
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownReenabled,
            ));
        }
        if should_emit_mismatch_notification
            && !self.reconnect_state_restore_validation_mismatch_notified
        {
            self.reconnect_state_restore_validation_mismatch_notified = true;
            self.reconnect_state_restore_correction_metrics
                .mismatch_notifications_emitted = self
                .reconnect_state_restore_correction_metrics
                .mismatch_notifications_emitted
                .saturating_add(1);
            actions.push(ClientRuntimeAction::NotifyReconnectTransition(
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused,
                    room_paused,
                    local_position,
                    room_position,
                    position_diff_seconds,
                },
            ));
        }

        if correction_suppressed_for_recovery_cycle {
            self.reconnect_state_restore_correction_metrics
                .correction_recovery_cooldown_suppressed_cycles = self
                .reconnect_state_restore_correction_metrics
                .correction_recovery_cooldown_suppressed_cycles
                .saturating_add(1);
            actions.push(ClientRuntimeAction::NotifyReconnectTransition(
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownSuppressed {
                    remaining_reconnect_cycles_after_this_cycle: self
                        .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining,
                },
            ));
            self.clear_reconnect_state_restore_validation_state();
            return actions;
        }

        if disable_correction_due_to_repeated_mismatches {
            if self.activate_reconnect_state_restore_correction_recovery_cooldown_if_configured() {
                self.reconnect_state_restore_correction_consecutive_mismatch_cycles = 0;
            }
            self.reconnect_state_restore_correction_metrics
                .correction_disables_after_repeated_mismatches = self
                .reconnect_state_restore_correction_metrics
                .correction_disables_after_repeated_mismatches
                .saturating_add(1);
            self.clear_reconnect_state_restore_validation_state();
            actions.push(ClientRuntimeAction::NotifyReconnectTransition(
                ReconnectTransitionNotification::StateRestoreValidationCorrectionDisabledAfterRepeatedMismatches {
                    consecutive_mismatch_cycles,
                    disable_after_mismatch_cycles,
                },
            ));
            return actions;
        }

        if matches!(
            correction_policy_mode,
            ReconnectStateRestoreCorrectionPolicyMode::NotifyOnly
        ) {
            self.clear_reconnect_state_restore_validation_state();
            return actions;
        }

        if !pause_matches {
            actions.push(ClientRuntimeAction::SetPaused(room_paused));
        }
        if !position_matches {
            actions.push(ClientRuntimeAction::SetPosition(room_position));
        }
        actions
    }

    pub fn runtime_actions_for_reconnect_playlist_restore_if_needed(
        &mut self,
    ) -> Vec<ClientRuntimeAction> {
        let Some(restore_intent) = self.reconnect_playlist_restore_intent.take() else {
            return Vec::new();
        };

        let mut actions = vec![
            ClientRuntimeAction::NotifyReconnectTransition(
                ReconnectTransitionNotification::RestoringPlaylist,
            ),
            ClientRuntimeAction::SetPlaylist {
                files: restore_intent.files,
            },
        ];
        if let Some(index) = restore_intent.index {
            actions.push(ClientRuntimeAction::SetPlaylistIndex { index });
        }
        actions
    }

    pub fn runtime_actions_for_controller_reidentify_if_needed(
        &mut self,
    ) -> Vec<ClientRuntimeAction> {
        let mut actions = Vec::new();
        if let Some(room) = self.controlled_room_switch_intent.take() {
            actions.push(ClientRuntimeAction::SetRoom { room });
        }
        if let Some((room, password)) = self.controller_reidentify_intent.take() {
            actions.push(ClientRuntimeAction::NotifyControllerAuthTransition(
                ControllerAuthTransitionNotification::Attempting { room: room.clone() },
            ));
            actions.push(ClientRuntimeAction::RequestControllerAuth { room, password });
        }
        actions
    }

    pub fn runtime_actions_for_outbound_chat_message(
        &self,
        message: String,
    ) -> Vec<ClientRuntimeAction> {
        if self.server_chat_supported != Some(true) {
            return Vec::new();
        }
        if self.chat_config.max_chat_message_length == 0 {
            return Vec::new();
        }
        let sanitized = Self::sanitize_chat_message_legacy_compatible(&message);
        let truncated = Self::truncate_chat_message_legacy_compatible(
            &sanitized,
            self.chat_config.max_chat_message_length,
        );
        vec![ClientRuntimeAction::SendChat { message: truncated }]
    }

    pub fn runtime_actions_for_local_ready_toggle(
        &self,
        manually_initiated: bool,
    ) -> Vec<ClientRuntimeAction> {
        if self.username.is_none() || self.server_readiness_supported == Some(false) {
            return Vec::new();
        }
        vec![ClientRuntimeAction::SetReady {
            ready: !self.local_user_ready(),
            manually_initiated,
        }]
    }

    pub fn runtime_actions_for_local_user_ready_set(
        &self,
        username: String,
        ready: bool,
        manually_initiated: bool,
    ) -> Vec<ClientRuntimeAction> {
        if self.username.is_none() || self.server_readiness_supported == Some(false) {
            return Vec::new();
        }
        if username.is_empty() {
            return vec![ClientRuntimeAction::SetReady {
                ready,
                manually_initiated,
            }];
        }
        vec![ClientRuntimeAction::SetReadyForUser {
            ready,
            manually_initiated,
            username,
        }]
    }

    pub fn runtime_actions_for_local_controller_auth_request(
        &self,
        room: String,
        password: String,
    ) -> Vec<ClientRuntimeAction> {
        if self.username.is_none() {
            return Vec::new();
        }
        if room.is_empty() {
            return Vec::new();
        }
        let password = Self::normalize_control_password_legacy_compatible(&password);
        vec![
            ClientRuntimeAction::NotifyControllerAuthTransition(
                ControllerAuthTransitionNotification::Attempting { room: room.clone() },
            ),
            ClientRuntimeAction::RequestControllerAuth { room, password },
        ]
    }

    pub fn runtime_actions_for_local_room_switch(&self, room: String) -> Vec<ClientRuntimeAction> {
        if self.server_chat_supported.is_none() {
            return Vec::new();
        }
        if room.is_empty() {
            return Vec::new();
        }
        vec![ClientRuntimeAction::SetRoom { room }]
    }

    pub fn local_room_command_target_with_legacy_fallback(&self, default_room: &str) -> String {
        self.username
            .as_deref()
            .and_then(|username| self.user_file_name(username))
            .filter(|file_name| !file_name.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| default_room.to_owned())
    }

    pub fn runtime_actions_for_local_pause_toggle(&mut self) -> Vec<ClientRuntimeAction> {
        let target_paused = !self.local_paused.unwrap_or(false);
        self.local_paused = Some(target_paused);
        vec![ClientRuntimeAction::SetPaused(target_paused)]
    }

    pub fn runtime_actions_for_local_user_list_request(&self) -> Vec<ClientRuntimeAction> {
        if self.username.is_none() {
            return Vec::new();
        }
        vec![ClientRuntimeAction::RequestUserList]
    }

    pub fn runtime_actions_for_local_playlist_index_set(
        &self,
        index: i64,
    ) -> Vec<ClientRuntimeAction> {
        if self.username.is_none() || index < 0 {
            return Vec::new();
        }

        let Some(playlist) = self.current_room_playlist() else {
            return Vec::new();
        };
        let Ok(index_usize) = usize::try_from(index) else {
            return Vec::new();
        };
        if index_usize >= playlist.files.len() {
            return Vec::new();
        }
        if !self.playlist_target_switch_allowed_legacy_compatible(&playlist.files[index_usize]) {
            return Vec::new();
        }

        vec![ClientRuntimeAction::SetPlaylistIndex { index }]
    }

    pub fn runtime_actions_for_local_playlist_next(&self) -> Vec<ClientRuntimeAction> {
        if self.username.is_none() {
            return Vec::new();
        }

        let Some(playlist) = self.current_room_playlist() else {
            return Vec::new();
        };
        if playlist.files.is_empty() {
            return Vec::new();
        }
        let Some(current_index) = playlist.index.and_then(|index| usize::try_from(index).ok())
        else {
            return Vec::new();
        };
        if current_index >= playlist.files.len() {
            return Vec::new();
        }

        if playlist.files.len() == 1 {
            if !self.loop_single_files_enabled_legacy_compatible() {
                return Vec::new();
            }
            return vec![
                ClientRuntimeAction::SetPosition(0.0),
                ClientRuntimeAction::SetPaused(false),
            ];
        }

        let Some(next_index) = current_index.checked_add(1) else {
            return Vec::new();
        };
        if next_index >= playlist.files.len() {
            if !self.loop_at_end_of_playlist_enabled_legacy_compatible() {
                return Vec::new();
            }
            if !self.playlist_target_switch_allowed_legacy_compatible(&playlist.files[0]) {
                return Vec::new();
            }
            return vec![ClientRuntimeAction::SetPlaylistIndex { index: 0 }];
        }
        if !self.playlist_target_switch_allowed_legacy_compatible(&playlist.files[next_index]) {
            return Vec::new();
        }

        vec![ClientRuntimeAction::SetPlaylistIndex {
            index: next_index as i64,
        }]
    }

    pub fn runtime_actions_for_local_playlist_queue(
        &mut self,
        file_name: String,
        select_after_queue: bool,
    ) -> Vec<ClientRuntimeAction> {
        if self.username.is_none() {
            return Vec::new();
        }
        let Some(room_name) = self.room.clone() else {
            return Vec::new();
        };

        if file_name.is_empty() {
            return Vec::new();
        }

        let (current_files, current_index) = self
            .current_room_playlist()
            .map(|playlist| {
                (
                    playlist.files.clone(),
                    playlist.index.and_then(|index| usize::try_from(index).ok()),
                )
            })
            .unwrap_or_default();
        let mut files = current_files.clone();
        files.push(file_name);
        if files == current_files {
            return Vec::new();
        }
        self.capture_playlist_undo_snapshot_legacy_compatible(&room_name, &current_files, &files);

        let target_index = if select_after_queue {
            files.len().saturating_sub(1)
        } else {
            current_index
                .filter(|index| *index < current_files.len())
                .unwrap_or(0)
        };

        vec![
            ClientRuntimeAction::SetPlaylist { files },
            ClientRuntimeAction::SetPlaylistIndex {
                index: target_index as i64,
            },
        ]
    }

    pub fn runtime_actions_for_local_playlist_delete(
        &mut self,
        index: i64,
    ) -> Vec<ClientRuntimeAction> {
        if self.username.is_none() || index < 0 {
            return Vec::new();
        }
        let Some(room_name) = self.room.clone() else {
            return Vec::new();
        };

        let Some(playlist) = self.current_room_playlist() else {
            return Vec::new();
        };
        let current_files = playlist.files.clone();
        let current_index = playlist
            .index
            .and_then(|current| usize::try_from(current).ok());
        let Ok(delete_index) = usize::try_from(index) else {
            return Vec::new();
        };
        if delete_index >= current_files.len() {
            return Vec::new();
        }

        let mut files = current_files.clone();
        files.remove(delete_index);
        self.capture_playlist_undo_snapshot_legacy_compatible(&room_name, &current_files, &files);

        if files.is_empty() {
            return vec![ClientRuntimeAction::SetPlaylist { files }];
        }

        let target_index = current_index
            .map(|current| {
                if current < delete_index {
                    current
                } else if current > delete_index {
                    current.saturating_sub(1)
                } else {
                    delete_index.min(files.len().saturating_sub(1))
                }
            })
            .unwrap_or(0)
            .min(files.len().saturating_sub(1));

        vec![
            ClientRuntimeAction::SetPlaylist { files },
            ClientRuntimeAction::SetPlaylistIndex {
                index: target_index as i64,
            },
        ]
    }

    pub fn runtime_actions_for_local_playlist_replace(
        &mut self,
        files: Vec<String>,
        selected_index: Option<usize>,
    ) -> Vec<ClientRuntimeAction> {
        if self.username.is_none() {
            return Vec::new();
        }
        let Some(room_name) = self.room.clone() else {
            return Vec::new();
        };
        if files.iter().any(|file| file.is_empty()) {
            return Vec::new();
        }

        let (current_files, current_index) = self
            .current_room_playlist()
            .map(|playlist| {
                (
                    playlist.files.clone(),
                    playlist.index.and_then(|index| usize::try_from(index).ok()),
                )
            })
            .unwrap_or_default();
        let playlist_changed = files != current_files;
        if playlist_changed {
            self.capture_playlist_undo_snapshot_legacy_compatible(
                &room_name,
                &current_files,
                &files,
            );
        }
        if files.is_empty() {
            return playlist_changed
                .then_some(ClientRuntimeAction::SetPlaylist { files })
                .into_iter()
                .collect();
        }

        let target_index = selected_index
            .filter(|index| *index < files.len())
            .or_else(|| {
                Some(
                    Self::local_playlist_target_index_from_changed_playlist_legacy_compatible(
                        &current_files,
                        current_index,
                        &files,
                    )
                    .min(files.len().saturating_sub(1)),
                )
            })
            .unwrap_or(0);

        if !playlist_changed && current_index == Some(target_index) {
            return Vec::new();
        }

        let mut actions = Vec::new();
        if playlist_changed {
            actions.push(ClientRuntimeAction::SetPlaylist { files });
        }
        actions.push(ClientRuntimeAction::SetPlaylistIndex {
            index: target_index as i64,
        });
        actions
    }

    pub fn runtime_actions_for_local_playlist_undo(&mut self) -> Vec<ClientRuntimeAction> {
        if self.username.is_none() {
            return Vec::new();
        }
        let Some(room_name) = self.room.clone() else {
            return Vec::new();
        };
        let Some(playlist) = self.current_room_playlist() else {
            return Vec::new();
        };

        let current_files = playlist.files.clone();
        let current_index = playlist.index.and_then(|index| usize::try_from(index).ok());
        let Some(previous_files) = self.playlist_undo_snapshots.get(&room_name).cloned() else {
            return Vec::new();
        };
        if previous_files == current_files {
            return Vec::new();
        }

        self.capture_playlist_undo_snapshot_legacy_compatible(
            &room_name,
            &current_files,
            &previous_files,
        );

        if previous_files.is_empty() {
            return vec![ClientRuntimeAction::SetPlaylist {
                files: previous_files,
            }];
        }

        let target_index =
            Self::local_playlist_target_index_from_changed_playlist_legacy_compatible(
                &current_files,
                current_index,
                &previous_files,
            )
            .min(previous_files.len().saturating_sub(1));

        vec![
            ClientRuntimeAction::SetPlaylist {
                files: previous_files,
            },
            ClientRuntimeAction::SetPlaylistIndex {
                index: target_index as i64,
            },
        ]
    }

    pub fn runtime_actions_for_local_playlist_shuffle_remaining(
        &mut self,
    ) -> Vec<ClientRuntimeAction> {
        if self.username.is_none() {
            return Vec::new();
        }
        let Some(room_name) = self.room.clone() else {
            return Vec::new();
        };
        let Some(playlist) = self.current_room_playlist() else {
            return Vec::new();
        };
        let Some(current_index) = playlist.index.and_then(|index| usize::try_from(index).ok())
        else {
            return Vec::new();
        };

        let current_files = playlist.files.clone();
        if current_index >= current_files.len() {
            return Vec::new();
        }
        let shuffle_start = current_index.saturating_add(1);
        if shuffle_start >= current_files.len() {
            return Vec::new();
        }

        let mut shuffled_files = current_files.clone();
        let seed =
            self.next_playlist_shuffle_seed_legacy_compatible(&current_files, current_index, true);
        Self::shuffle_playlist_slice_in_place_legacy_compatible(
            &mut shuffled_files[shuffle_start..],
            seed,
        );
        if shuffled_files == current_files {
            return Vec::new();
        }

        self.capture_playlist_undo_snapshot_legacy_compatible(
            &room_name,
            &current_files,
            &shuffled_files,
        );
        vec![
            ClientRuntimeAction::SetPlaylist {
                files: shuffled_files,
            },
            ClientRuntimeAction::SetPlaylistIndex {
                index: current_index as i64,
            },
        ]
    }

    pub fn runtime_actions_for_local_playlist_shuffle_entire(
        &mut self,
    ) -> Vec<ClientRuntimeAction> {
        if self.username.is_none() {
            return Vec::new();
        }
        let Some(room_name) = self.room.clone() else {
            return Vec::new();
        };
        let Some(playlist) = self.current_room_playlist() else {
            return Vec::new();
        };

        let current_files = playlist.files.clone();
        if current_files.is_empty() {
            return Vec::new();
        }
        let current_index = playlist.index.and_then(|index| usize::try_from(index).ok());
        let mut shuffled_files = current_files.clone();
        let seed = self.next_playlist_shuffle_seed_legacy_compatible(
            &current_files,
            current_index.unwrap_or(0),
            false,
        );
        Self::shuffle_playlist_slice_in_place_legacy_compatible(&mut shuffled_files, seed);

        let playlist_changed = shuffled_files != current_files;
        if playlist_changed {
            self.capture_playlist_undo_snapshot_legacy_compatible(
                &room_name,
                &current_files,
                &shuffled_files,
            );
        }

        let mut actions = Vec::new();
        if playlist_changed {
            actions.push(ClientRuntimeAction::SetPlaylist {
                files: shuffled_files,
            });
        }
        if current_index != Some(0) || playlist_changed {
            actions.push(ClientRuntimeAction::SetPlaylistIndex { index: 0 });
        }
        actions
    }

    pub fn runtime_actions_for_local_seek(
        &mut self,
        target_position: f64,
    ) -> Vec<ClientRuntimeAction> {
        if !target_position.is_finite() {
            return Vec::new();
        }
        let previous_position = self
            .local_position
            .or_else(|| {
                self.current_room_playstate()
                    .and_then(|playstate| playstate.position)
            })
            .unwrap_or(0.0);
        self.last_seek_position_before_manual_seek = Some(previous_position);
        self.local_position = Some(target_position);
        vec![ClientRuntimeAction::SetPosition(target_position)]
    }

    pub fn runtime_actions_for_local_seek_offset(
        &mut self,
        offset_seconds: f64,
    ) -> Vec<ClientRuntimeAction> {
        if !offset_seconds.is_finite() {
            return Vec::new();
        }
        let baseline_position = self
            .current_room_playstate()
            .and_then(|playstate| playstate.position)
            .or(self.local_position)
            .unwrap_or(0.0);
        self.runtime_actions_for_local_seek(baseline_position + offset_seconds)
    }

    pub fn runtime_actions_for_local_seek_undo(&mut self) -> Vec<ClientRuntimeAction> {
        let Some(target_position) = self.last_seek_position_before_manual_seek else {
            return Vec::new();
        };
        let current_position = self
            .local_position
            .or_else(|| {
                self.current_room_playstate()
                    .and_then(|playstate| playstate.position)
            })
            .unwrap_or(target_position);
        self.last_seek_position_before_manual_seek = Some(current_position);
        self.local_position = Some(target_position);
        vec![ClientRuntimeAction::SetPosition(target_position)]
    }

    pub fn evaluate_desync_correction(
        &mut self,
        now_seconds: f64,
        local_position: f64,
        local_can_control: bool,
        dont_slow_down_with_me: bool,
        speed_supported: bool,
    ) -> DesyncCorrectionAction {
        let Some(global_playstate) = self.current_room_playstate() else {
            self.behind_first_detected_at_seconds = None;
            return DesyncCorrectionAction::None;
        };

        let (Some(global_position), Some(global_paused)) =
            (global_playstate.position, global_playstate.paused)
        else {
            self.behind_first_detected_at_seconds = None;
            return DesyncCorrectionAction::None;
        };

        if global_playstate.do_seek == Some(true) {
            self.behind_first_detected_at_seconds = None;
            return DesyncCorrectionAction::None;
        }

        let diff = local_position - global_position;
        let set_by = global_playstate.set_by.clone();
        let set_by_is_self = self
            .username
            .as_deref()
            .zip(set_by.as_deref())
            .is_some_and(|(username, set_by)| username == set_by);

        if self.desync_config.rewind_on_desync && diff > self.desync_config.rewind_threshold_seconds
        {
            self.behind_first_detected_at_seconds = None;
            if set_by_is_self {
                return DesyncCorrectionAction::None;
            }
            return DesyncCorrectionAction::Rewind {
                target_position: global_position,
                set_by,
            };
        }

        if self.desync_config.fastforward_on_desync
            && (!local_can_control || dont_slow_down_with_me)
        {
            if diff < -self.desync_config.fastforward_behind_threshold_seconds {
                if let Some(first_detected_at) = self.behind_first_detected_at_seconds {
                    let duration_behind = now_seconds - first_detected_at;
                    if duration_behind
                        > (self.desync_config.fastforward_threshold_seconds
                            - self.desync_config.fastforward_behind_threshold_seconds)
                        && diff < -self.desync_config.fastforward_threshold_seconds
                    {
                        self.behind_first_detected_at_seconds = Some(
                            now_seconds + self.desync_config.fastforward_reset_threshold_seconds,
                        );
                        if set_by_is_self {
                            return DesyncCorrectionAction::None;
                        }
                        return DesyncCorrectionAction::FastForward {
                            target_position: global_position
                                + self.desync_config.fastforward_extra_seconds,
                            set_by,
                        };
                    }
                } else {
                    self.behind_first_detected_at_seconds = Some(now_seconds);
                }
            } else {
                self.behind_first_detected_at_seconds = None;
            }
        } else {
            self.behind_first_detected_at_seconds = None;
        }

        if speed_supported && !global_paused && self.desync_config.slow_on_desync {
            if diff > self.desync_config.slowdown_threshold_seconds && !self.speed_changed {
                if set_by_is_self {
                    return DesyncCorrectionAction::None;
                }
                self.speed_changed = true;
                return DesyncCorrectionAction::SlowDown {
                    rate: self.desync_config.slowdown_rate,
                    set_by,
                };
            }
            if self.speed_changed && diff < self.desync_config.slowdown_reset_threshold_seconds {
                self.speed_changed = false;
                return DesyncCorrectionAction::RestoreSpeed {
                    rate: NORMAL_PLAYBACK_RATE,
                };
            }
        }

        DesyncCorrectionAction::None
    }

    pub fn runtime_actions_for_desync_correction(
        &mut self,
        now_seconds: f64,
        local_position: f64,
        local_can_control: bool,
        dont_slow_down_with_me: bool,
        speed_supported: bool,
    ) -> Vec<ClientRuntimeAction> {
        match self.evaluate_desync_correction(
            now_seconds,
            local_position,
            local_can_control,
            dont_slow_down_with_me,
            speed_supported,
        ) {
            DesyncCorrectionAction::None => Vec::new(),
            DesyncCorrectionAction::Rewind {
                target_position, ..
            }
            | DesyncCorrectionAction::FastForward {
                target_position, ..
            } => {
                vec![ClientRuntimeAction::SetPosition(target_position)]
            }
            DesyncCorrectionAction::SlowDown { rate, .. }
            | DesyncCorrectionAction::RestoreSpeed { rate } => {
                vec![ClientRuntimeAction::SetPlaybackRate(rate)]
            }
        }
    }

    pub fn handle_disconnect(&mut self, now_seconds: f64) -> Vec<ClientRuntimeAction> {
        self.server_chat_supported = None;
        if !self.behavior_config.pause_on_leave {
            return Vec::new();
        }

        self.last_paused_on_leave_at_seconds = Some(now_seconds);
        let should_pause = self.local_paused != Some(true);
        self.local_paused = Some(true);

        if should_pause {
            vec![ClientRuntimeAction::SetPaused(true)]
        } else {
            Vec::new()
        }
    }

    pub fn instaplay_conditions_met(
        &self,
        local_can_control: bool,
        is_playing_music: bool,
    ) -> bool {
        if is_playing_music {
            return true;
        }

        if !local_can_control {
            return false;
        }

        if self.local_user_ready()
            || self.readiness_autoplay_config.unpause_action == UnpauseActionMode::Always
        {
            return true;
        }

        let all_other_users_ready = self.all_other_users_in_current_room_ready();
        match self.readiness_autoplay_config.unpause_action {
            UnpauseActionMode::IfAlreadyReady => false,
            UnpauseActionMode::IfOthersReady => all_other_users_ready,
            UnpauseActionMode::IfMinUsersReady => {
                all_other_users_ready
                    && self
                        .readiness_autoplay_config
                        .auto_play_threshold
                        .is_some_and(|threshold| {
                            self.users_in_current_room_count_for_threshold() >= threshold
                        })
            }
            UnpauseActionMode::Always => true,
        }
    }

    pub fn runtime_actions_for_readiness_unpause_attempt(
        &mut self,
        now_seconds: f64,
        readiness_supported: bool,
        local_can_control: bool,
        is_playing_music: bool,
    ) -> Vec<ClientRuntimeAction> {
        if !readiness_supported {
            return Vec::new();
        }

        let instaplay = self.instaplay_conditions_met(local_can_control, is_playing_music);
        if !instaplay {
            self.local_paused = Some(true);
            let mut actions = vec![ClientRuntimeAction::SetPaused(true)];
            if !self.local_user_ready() {
                actions.push(ClientRuntimeAction::SetReady {
                    ready: true,
                    manually_initiated: true,
                });
            }
            return actions;
        }

        if let Some(last_paused_on_leave_at_seconds) = self.last_paused_on_leave_at_seconds
            && now_seconds - last_paused_on_leave_at_seconds
                < self
                    .readiness_autoplay_config
                    .last_paused_diff_threshold_seconds
        {
            self.last_paused_on_leave_at_seconds = None;
            self.local_paused = Some(false);
            return Vec::new();
        }

        self.local_paused = Some(false);
        if self.local_user_ready() {
            return Vec::new();
        }

        vec![ClientRuntimeAction::SetReady {
            ready: true,
            manually_initiated: false,
        }]
    }

    pub fn autoplay_conditions_met(
        &self,
        readiness_supported: bool,
        local_can_control: bool,
        is_playing_music: bool,
        recently_advanced: bool,
    ) -> bool {
        if is_playing_music {
            return true;
        }

        let threshold_met = self
            .readiness_autoplay_config
            .auto_play_threshold
            .is_some_and(|threshold| self.users_in_current_room_count_for_threshold() >= threshold);

        self.local_paused.unwrap_or(true)
            && (self.autoplay_enabled || recently_advanced)
            && local_can_control
            && readiness_supported
            && self.all_users_in_current_room_ready()
            && (threshold_met || recently_advanced)
    }

    pub fn autoplay_check(
        &mut self,
        readiness_supported: bool,
        local_can_control: bool,
        is_playing_music: bool,
        recently_advanced: bool,
    ) {
        if is_playing_music {
            return;
        }

        if self.autoplay_conditions_met(
            readiness_supported,
            local_can_control,
            is_playing_music,
            recently_advanced,
        ) {
            self.start_autoplay_countdown();
        } else {
            self.stop_autoplay_countdown();
        }
    }

    pub fn autoplay_countdown_tick(
        &mut self,
        readiness_supported: bool,
        local_can_control: bool,
        is_playing_music: bool,
        recently_advanced: bool,
    ) -> Vec<ClientRuntimeAction> {
        if !self.autoplay_timer_running {
            return Vec::new();
        }

        if !self.autoplay_conditions_met(
            readiness_supported,
            local_can_control,
            is_playing_music,
            recently_advanced,
        ) {
            self.stop_autoplay_countdown();
            return Vec::new();
        }

        if self.autoplay_time_left_seconds <= 0.0 {
            self.local_paused = Some(false);
            self.stop_autoplay_countdown();
            return vec![ClientRuntimeAction::SetPaused(false)];
        }

        let notification = AutoplayCountdownNotification {
            ready_user_count: self.ready_user_count_in_current_room(),
            seconds_left: self.autoplay_time_left_seconds.max(0.0).floor() as u32,
        };
        self.autoplay_time_left_seconds -= AUTOPLAY_COUNTDOWN_STEP_SECONDS;
        vec![ClientRuntimeAction::NotifyAutoplayCountdown(notification)]
    }

    pub fn reset_sync_state_for_reconnect(&mut self) {
        let (ready_snapshot, file_snapshot, controller_snapshot) = self
            .username
            .as_deref()
            .and_then(|username| self.user_views.get(username))
            .map(|user_view| {
                let ready_snapshot = user_view.ready;
                let file_snapshot = if user_view.has_file {
                    Self::file_payload_from_user_view(user_view)
                } else {
                    None
                };
                let controller_snapshot = Some(user_view.controller);
                (ready_snapshot, file_snapshot, controller_snapshot)
            })
            .unwrap_or((None, None, None));
        let preserved_ready_snapshot = self
            .reconnect_ready_restore_snapshot
            .take()
            .or(self.reconnect_ready_restore_intent.take());
        let preserved_file_snapshot = self
            .reconnect_file_restore_snapshot
            .take()
            .or(self.reconnect_file_restore_intent.take());
        let preserved_controller_snapshot = self.reconnect_controller_restore_snapshot.take();
        let preserved_playlist_snapshot = self
            .reconnect_playlist_restore_snapshot
            .take()
            .or(self.reconnect_playlist_restore_intent.take());

        self.reconnect_ready_restore_snapshot = preserved_ready_snapshot.or(ready_snapshot);
        self.reconnect_ready_restore_intent = None;
        self.reconnect_file_restore_snapshot = preserved_file_snapshot.or(file_snapshot);
        self.reconnect_file_restore_intent = None;
        self.reconnect_controller_restore_snapshot =
            preserved_controller_snapshot.or(controller_snapshot);

        self.reconnect_playlist_restore_snapshot = preserved_playlist_snapshot.or_else(|| {
            self.current_room_playlist()
                .and_then(Self::playlist_restore_intent_from_room_playlist)
        });
        self.reconnect_playlist_restore_intent = None;
        self.reconnect_connected_intent = false;
        self.clear_reconnect_state_restore_validation_state();
        self.pending_chat_notifications.clear();
        self.pending_controller_auth_notifications.clear();
        self.pending_user_change_notifications.clear();
        self.controlled_room_switch_intent = None;
        self.controller_reidentify_intent = None;
        self.user_views.clear();
        self.known_rooms.clear();
        self.domain = SyncDomain::default();
        self.room_playlists.clear();
        self.room_playstates.clear();
        self.pending_playlist = None;
        self.playlist_undo_snapshots.clear();
        self.playlist_shuffle_nonce = 0;
        self.local_position = None;
        self.local_paused = None;
        self.last_seek_position_before_manual_seek = None;
        self.autoplay_timer_running = false;
        self.autoplay_time_left_seconds = self.readiness_autoplay_config.autoplay_delay_seconds;
        self.speed_changed = false;
        self.behind_first_detected_at_seconds = None;
        self.last_paused_on_leave_at_seconds = None;
        self.last_advanced_at_seconds = None;
        self.client_ignoring_on_the_fly = 0;
        self.server_ignoring_on_the_fly = 0;
        self.server_chat_supported = None;

        if let (Some(username), Some(room_name)) = (self.username.clone(), self.room.clone()) {
            self.set_user_room(&username, Some(room_name));
            self.set_user_ready_state(&username, Some(false));
        }
    }

    pub fn reconcile_state_and_build_response(
        &mut self,
        inbound_state: StatePayload,
        local_position: f64,
        local_paused: bool,
        client_latency_calculation: f64,
        client_rtt: f64,
    ) -> StatePayload {
        self.apply_inbound_ignore_counters(&inbound_state);

        let has_playstate_update = inbound_state
            .playstate
            .as_ref()
            .is_some_and(|playstate| playstate.position.is_some() && playstate.paused.is_some());
        if has_playstate_update && self.client_ignoring_on_the_fly == 0 {
            self.apply_state(inbound_state.clone());
        }

        let mut response = StatePayload::new();
        let has_global_playstate = self.has_global_playstate();
        let client_ignore_not_set =
            self.client_ignoring_on_the_fly == 0 || self.server_ignoring_on_the_fly != 0;

        let mut state_change = false;
        if has_global_playstate && client_ignore_not_set {
            let (pause_change, seeked) =
                self.determine_local_state_change(local_paused, local_position);

            let mut playstate = PlaystatePayload::new()
                .with_position(local_position)
                .with_paused(local_paused);
            if seeked {
                playstate = playstate.with_do_seek(true);
            }
            response.playstate = Some(playstate);
            state_change = pause_change || seeked;
        }

        self.local_position = Some(local_position);
        self.local_paused = Some(local_paused);

        let mut ping = PingPayload::new()
            .with_client_latency_calculation(client_latency_calculation)
            .with_client_rtt(client_rtt);
        if let Some(latency_calculation) = inbound_state
            .ping
            .as_ref()
            .and_then(|ping| ping.latency_calculation)
            && latency_calculation != 0.0
        {
            ping = ping.with_latency_calculation(latency_calculation);
        }
        response.ping = Some(ping);

        if state_change {
            self.client_ignoring_on_the_fly = self.client_ignoring_on_the_fly.saturating_add(1);
        }

        if self.server_ignoring_on_the_fly != 0 || self.client_ignoring_on_the_fly != 0 {
            let mut ignore = IgnoringOnTheFlyPayload::new();
            if self.server_ignoring_on_the_fly != 0 {
                ignore = ignore.with_server(self.server_ignoring_on_the_fly);
                self.server_ignoring_on_the_fly = 0;
            }
            if self.client_ignoring_on_the_fly != 0 {
                ignore = ignore.with_client(self.client_ignoring_on_the_fly);
            }
            response.ignoring_on_the_fly = Some(ignore);
        }

        response
    }

    fn apply_hello(&mut self, hello: syncplay_protocol::HelloPayload) {
        if self.reconnect_in_progress {
            self.reconnect_in_progress = false;
            self.reconnect_connected_intent = true;
        }

        self.server_readiness_supported = Self::feature_bool(hello.features.as_ref(), "readiness");
        let server_version = hello.effective_version().to_owned();
        self.server_chat_supported = Some(
            Self::feature_bool(hello.features.as_ref(), "chat").unwrap_or_else(|| {
                Self::meets_min_version_legacy_compatible(&server_version, LEGACY_CHAT_MIN_VERSION)
            }),
        );
        if self.chat_config.apply_server_max_chat_message_length {
            self.chat_config.max_chat_message_length =
                Self::feature_usize(hello.features.as_ref(), "maxChatMessageLength")
                    .unwrap_or(LEGACY_FALLBACK_MAX_CHAT_MESSAGE_LENGTH);
        }

        let username = hello.username;
        let room_name = hello.room.name;

        self.username = Some(username.clone());
        self.room = Some(room_name.clone());

        self.controller_reidentify_intent = self
            .controlled_room_passwords
            .get(&room_name)
            .cloned()
            .map(|password| (room_name.clone(), password));

        self.set_user_room(&username, Some(room_name));
        self.set_user_ready(&username, false);

        if let Some(current_room) = self.room.clone()
            && let Some(pending_playlist) = self.pending_playlist.take()
        {
            self.room_playlists.insert(current_room, pending_playlist);
        }

        if let Some(restored_ready) = self.reconnect_ready_restore_snapshot.take() {
            self.reconnect_ready_restore_intent = Some(restored_ready);
            self.set_user_ready(&username, restored_ready);
        }

        if let Some(restored_file_payload) = self.reconnect_file_restore_snapshot.take() {
            let (has_file, file_name, file_size, file_duration) =
                Self::list_payload_file_info(Some(&restored_file_payload));
            self.set_user_file_info(&username, has_file, file_name, file_size, file_duration);
            self.reconnect_file_restore_intent = Some(restored_file_payload);
        }

        if let Some(restored_controller) = self.reconnect_controller_restore_snapshot.take() {
            self.set_user_controller(&username, restored_controller);
        }
    }

    fn apply_set(&mut self, set_payload: SetPayload, now_seconds: Option<f64>) {
        if let Some(room) = set_payload.room {
            if let Some(username) = self.username.clone() {
                self.set_user_room(&username, Some(room.name.clone()));
            }
            self.room = Some(room.name);
        }

        if let Some(users) = set_payload.user {
            for (username, user_payload) in users {
                let was_local_user = self.username.as_deref() == Some(username.as_str());
                let previous_user_view = self.user_views.get(&username).cloned();

                if user_payload
                    .event
                    .as_ref()
                    .and_then(|event| event.get("left"))
                    .and_then(|value| value.as_bool())
                    == Some(true)
                {
                    if !was_local_user {
                        self.queue_user_left_notification_if_relevant(
                            &username,
                            previous_user_view,
                        );
                    }
                    self.remove_user(&username);
                    continue;
                }

                if let Some(room) = user_payload.room {
                    self.set_user_room(&username, Some(room.name.clone()));
                    if was_local_user {
                        self.room = Some(room.name);
                    }
                }

                // Legacy modUser only applies file updates when the payload is truthy.
                if let Some(file) = user_payload.file.as_ref()
                    && Self::legacy_json_value_truthy(file)
                {
                    let (file_name, file_size, file_duration) =
                        Self::file_metadata_from_payload(file);
                    self.set_user_file_info(&username, true, file_name, file_size, file_duration);
                }

                if let Some(controller) = user_payload.controller {
                    self.set_user_controller(&username, controller);
                }

                if let Some(is_ready) = user_payload.is_ready {
                    self.set_user_ready(&username, is_ready);
                }

                if !was_local_user {
                    self.queue_user_change_notification_if_relevant(&username, previous_user_view);
                }
            }
        }

        if let Some(ready) = set_payload.ready {
            let target_username = ready
                .username
                .or(ready.set_by)
                .or_else(|| self.username.clone());
            if let Some(target_username) = target_username {
                if self.user_room(&target_username).is_none()
                    && let Some(room_name) = self.room.clone()
                {
                    self.set_user_room(&target_username, Some(room_name));
                }
                self.set_user_ready(&target_username, ready.is_ready);
            }
        }

        if let Some(controller_auth) = set_payload.controller_auth {
            let target_username = controller_auth.user.or_else(|| self.username.clone());
            let target_room = controller_auth.room.or_else(|| self.room.clone());
            let target_is_local_user = target_username
                .as_deref()
                .zip(self.username.as_deref())
                .is_some_and(|(target, local)| target == local);
            let target_room_matches_local_room = target_room
                .as_deref()
                .zip(self.room.as_deref())
                .is_some_and(|(target, local)| target == local);

            match controller_auth.success {
                Some(true) => {
                    if let Some(target_username) = target_username.as_deref() {
                        self.set_user_controller(target_username, true);
                    }
                    if target_room_matches_local_room
                        && let (Some(target_username), Some(target_room)) =
                            (target_username, target_room)
                    {
                        self.pending_controller_auth_notifications.push(
                            ControllerAuthTransitionNotification::Succeeded {
                                username: target_username,
                                room: target_room,
                                hide_from_osd: !self.behavior_config.show_same_room_osd,
                            },
                        );
                    }
                }
                Some(false) => {
                    if target_is_local_user
                        && let (Some(target_username), Some(target_room)) =
                            (target_username, target_room)
                    {
                        let hide_from_osd = self
                            .noncontroller_event_hide_from_osd_legacy_compatible(&target_username);
                        self.pending_controller_auth_notifications.push(
                            ControllerAuthTransitionNotification::Failed {
                                username: target_username,
                                room: target_room,
                                hide_from_osd,
                            },
                        );
                    }
                }
                None => {}
            }
        }

        if let Some(new_controlled_room) = set_payload.new_controlled_room
            && let (Some(room_name), Some(password)) =
                (new_controlled_room.room_name, new_controlled_room.password)
        {
            let normalized_password = Self::normalize_control_password_legacy_compatible(&password);
            self.remember_control_password_for_room(&room_name, &password);

            if let Some(local_username) = self.username.clone() {
                self.room = Some(room_name.clone());
                self.set_user_room(&local_username, Some(room_name.clone()));
                self.set_user_controller(&local_username, false);
                if Self::is_controlled_room_name(&room_name) && !normalized_password.is_empty() {
                    self.controlled_room_switch_intent = Some(room_name.clone());
                    self.controller_reidentify_intent = Some((room_name, normalized_password));
                }
            }
        }

        if let Some(playlist_change) = set_payload.playlist_change {
            let mut skip_playlist_change_apply = false;
            if playlist_change.user.is_none() && playlist_change.files.is_empty() {
                if let Some(restore_intent) = self.reconnect_playlist_restore_snapshot.take() {
                    self.reconnect_playlist_restore_intent = Some(restore_intent);
                    skip_playlist_change_apply = true;
                }
            } else {
                self.reconnect_playlist_restore_snapshot = None;
            }

            if !skip_playlist_change_apply {
                let playlist_change_files = playlist_change.files;
                let playlist_change_user = playlist_change.user;
                if let Some(room_name) =
                    self.resolve_room_for_playlist_update(playlist_change_user.as_deref())
                {
                    let current_files = self
                        .room_playlists
                        .get(&room_name)
                        .map(|playlist| playlist.files.clone())
                        .unwrap_or_default();
                    self.capture_playlist_undo_snapshot_legacy_compatible(
                        &room_name,
                        &current_files,
                        &playlist_change_files,
                    );

                    let playlist = self.room_playlists.entry(room_name).or_default();
                    playlist.files = playlist_change_files;
                    playlist.set_by = playlist_change_user;
                } else {
                    let pending_playlist =
                        self.pending_playlist.get_or_insert_with(Default::default);
                    pending_playlist.files = playlist_change_files;
                    pending_playlist.set_by = playlist_change_user;
                }
            }
        }

        if let Some(playlist_index) = set_payload.playlist_index {
            if playlist_index.user.is_some() {
                self.reconnect_playlist_restore_snapshot = None;
            }

            let set_by_local = playlist_index
                .user
                .as_deref()
                .zip(self.username.as_deref())
                .is_some_and(|(set_by, local_username)| set_by == local_username);
            if set_by_local {
                self.last_advanced_at_seconds = now_seconds;
            }

            if let Some(room_name) =
                self.resolve_room_for_playlist_update(playlist_index.user.as_deref())
            {
                let playlist = self.room_playlists.entry(room_name).or_default();
                playlist.index = Some(playlist_index.index);
                if playlist_index.user.is_some() {
                    playlist.set_by = playlist_index.user;
                }
            } else {
                let pending_playlist = self.pending_playlist.get_or_insert_with(Default::default);
                pending_playlist.index = Some(playlist_index.index);
                if playlist_index.user.is_some() {
                    pending_playlist.set_by = playlist_index.user;
                }
            }
        }
    }

    fn apply_list(&mut self, list_payload: ListPayload) {
        let ListPayload::Rooms(rooms) = list_payload else {
            return;
        };

        self.user_views.clear();
        self.known_rooms.clear();
        self.domain = SyncDomain::default();

        let mut resolved_self_room = None;
        let current_username = self.username.clone();

        for (room_name, room_users) in rooms {
            self.known_rooms.insert(room_name.clone());
            for (username, user_entry) in room_users {
                if username.trim().is_empty() {
                    continue;
                }
                self.set_user_room(&username, Some(room_name.clone()));
                let (has_file, file_name, file_size, file_duration) =
                    Self::list_payload_file_info(user_entry.file.as_ref());
                self.set_user_file_info(&username, has_file, file_name, file_size, file_duration);
                self.set_user_controller(&username, user_entry.controller.unwrap_or(false));
                self.set_user_ready_state(&username, user_entry.is_ready);
                if current_username.as_deref() == Some(username.as_str()) {
                    resolved_self_room = Some(room_name.clone());
                }
            }
        }

        if resolved_self_room.is_some() {
            self.room = resolved_self_room;
        }
    }

    fn apply_state(&mut self, state_payload: StatePayload) {
        let Some(playstate) = state_payload.playstate else {
            return;
        };

        let room_name = playstate
            .set_by
            .as_deref()
            .and_then(|username| self.user_room(username).map(str::to_owned))
            .or_else(|| self.room.clone());

        if let Some(room_name) = room_name {
            self.merge_room_playstate(room_name, playstate);
        }
    }

    fn apply_chat(&mut self, chat_payload: ChatPayload) {
        let notification = match chat_payload {
            ChatPayload::Text(message) => ChatNotification::Message {
                username: None,
                message,
            },
            ChatPayload::Message(message_payload) => ChatNotification::Message {
                username: Some(message_payload.username),
                message: message_payload.message,
            },
        };
        self.pending_chat_notifications.push(notification);
    }

    fn sanitize_chat_message_legacy_compatible(message: &str) -> String {
        message
            .chars()
            .filter(|character| *character != '\n' && *character != '\r')
            .collect()
    }

    fn truncate_chat_message_legacy_compatible(message: &str, max_length: usize) -> String {
        message.chars().take(max_length).collect()
    }

    fn feature_bool(features: Option<&Value>, name: &str) -> Option<bool> {
        features
            .and_then(|feature_map| feature_map.get(name))
            .and_then(|value| value.as_bool())
    }

    fn feature_usize(features: Option<&Value>, name: &str) -> Option<usize> {
        features
            .and_then(|feature_map| feature_map.get(name))
            .and_then(|value| value.as_u64())
            .and_then(|value| usize::try_from(value).ok())
    }

    fn parse_numeric_version_components(version: &str) -> Option<Vec<u32>> {
        let trimmed = version.trim();
        if trimmed.is_empty() {
            return None;
        }

        let mut components = Vec::new();
        for part in trimmed.split('.') {
            if part.is_empty() {
                return None;
            }
            components.push(part.parse::<u32>().ok()?);
        }
        Some(components)
    }

    fn meets_min_version_legacy_compatible(version: &str, min_version: &str) -> bool {
        let Some(mut version_components) = Self::parse_numeric_version_components(version) else {
            return false;
        };
        let Some(mut min_version_components) = Self::parse_numeric_version_components(min_version)
        else {
            return false;
        };

        let width = version_components.len().max(min_version_components.len());
        version_components.resize(width, 0);
        min_version_components.resize(width, 0);
        version_components >= min_version_components
    }

    fn merge_room_playstate(&mut self, room_name: String, playstate: PlaystatePayload) {
        let room_playstate = self.room_playstates.entry(room_name).or_default();
        if let Some(position) = playstate.position {
            room_playstate.position = Some(position);
        }
        if let Some(paused) = playstate.paused {
            room_playstate.paused = Some(paused);
        }
        if let Some(do_seek) = playstate.do_seek {
            room_playstate.do_seek = Some(do_seek);
        }
        room_playstate.set_by = playstate.set_by;
    }

    fn apply_inbound_ignore_counters(&mut self, state_payload: &StatePayload) {
        let Some(ignore) = state_payload.ignoring_on_the_fly.as_ref() else {
            return;
        };

        if let Some(server) = ignore.server {
            self.server_ignoring_on_the_fly = server;
            self.client_ignoring_on_the_fly = 0;
        } else if let Some(client) = ignore.client
            && client == self.client_ignoring_on_the_fly
        {
            self.client_ignoring_on_the_fly = 0;
        }
    }

    fn has_global_playstate(&self) -> bool {
        self.current_room_playstate()
            .is_some_and(|playstate| playstate.position.is_some() && playstate.paused.is_some())
    }

    fn determine_local_state_change(
        &self,
        local_paused: bool,
        local_position: f64,
    ) -> (bool, bool) {
        let global_paused = self
            .current_room_playstate()
            .and_then(|playstate| playstate.paused)
            .unwrap_or(true);
        let global_position = self
            .current_room_playstate()
            .and_then(|playstate| playstate.position)
            .unwrap_or(0.0);
        let player_paused = self.local_paused.unwrap_or(global_paused);

        let pause_change = player_paused != local_paused && global_paused != local_paused;
        let seeked =
            (global_position - local_position).abs() > SEEK_THRESHOLD_SECONDS && !pause_change;
        (pause_change, seeked)
    }

    fn local_username_and_room(&self) -> Option<(&str, &str)> {
        let local_username = self.username.as_deref()?;
        let local_room = self.room.as_deref()?;
        Some((local_username, local_room))
    }

    fn is_controlled_room_name(room_name: &str) -> bool {
        if !room_name.starts_with('+') {
            return false;
        }
        let Some((_, hash)) = room_name.rsplit_once(':') else {
            return false;
        };
        hash.len() == 12 && hash.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    }

    fn normalize_control_password_legacy_compatible(password: &str) -> String {
        password
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect::<String>()
            .to_ascii_uppercase()
    }

    fn local_user_ready(&self) -> bool {
        self.username
            .as_deref()
            .and_then(|username| self.user_views.get(username))
            .is_some_and(|user_view| user_view.ready == Some(true))
    }

    fn user_ready_with_file(user_view: &ClientUserView) -> Option<bool> {
        if !user_view.has_file {
            return None;
        }
        user_view.ready
    }

    fn legacy_json_value_truthy(value: &Value) -> bool {
        match value {
            Value::Null => false,
            Value::Bool(flag) => *flag,
            Value::Number(number) => {
                if let Some(signed) = number.as_i64() {
                    signed != 0
                } else if let Some(unsigned) = number.as_u64() {
                    unsigned != 0
                } else {
                    number.as_f64().is_some_and(|float| float != 0.0)
                }
            }
            Value::String(text) => !text.is_empty(),
            Value::Array(items) => !items.is_empty(),
            Value::Object(entries) => !entries.is_empty(),
        }
    }

    fn list_payload_has_file(file: Option<&Value>) -> bool {
        match file {
            Some(Value::Null) | None => false,
            Some(Value::Object(entries)) => !entries.is_empty(),
            Some(_) => true,
        }
    }

    fn list_payload_file_info(
        file: Option<&Value>,
    ) -> (bool, Option<String>, Option<Value>, Option<Value>) {
        match file {
            Some(Value::Null) | None => (false, None, None, None),
            Some(Value::Object(entries)) if entries.is_empty() => (false, None, None, None),
            Some(value) => (
                Self::list_payload_has_file(Some(value)),
                Self::file_name_from_payload(value),
                Self::file_size_from_payload(value),
                Self::file_duration_from_payload(value),
            ),
        }
    }

    fn file_name_from_payload(file: &Value) -> Option<String> {
        match file {
            Value::Object(entries) => entries
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_owned),
            Value::String(name) => Some(name.to_owned()),
            _ => None,
        }
    }

    fn file_size_from_payload(file: &Value) -> Option<Value> {
        match file {
            Value::Object(entries) => entries.get("size").cloned(),
            _ => None,
        }
    }

    fn file_duration_from_payload(file: &Value) -> Option<Value> {
        match file {
            Value::Object(entries) => entries.get("duration").cloned(),
            _ => None,
        }
    }

    fn file_metadata_from_payload(file: &Value) -> (Option<String>, Option<Value>, Option<Value>) {
        (
            Self::file_name_from_payload(file),
            Self::file_size_from_payload(file),
            Self::file_duration_from_payload(file),
        )
    }

    fn file_difference_summary_for_users(
        current_user: &ClientUserView,
        other_user: &ClientUserView,
        session: &ClientSession,
    ) -> Option<FileDifferenceSummary> {
        if !current_user.has_file || !other_user.has_file {
            return None;
        }

        let filename = match (&current_user.file_name, &other_user.file_name) {
            (Some(current_name), Some(other_name)) => {
                !Self::same_filename_legacy_like(current_name, other_name)
            }
            (None, None) => false,
            _ => true,
        };

        let filesize = match (&current_user.file_size, &other_user.file_size) {
            (Some(current_size), Some(other_size)) => {
                !Self::same_filesize_legacy_like(current_size, other_size)
            }
            (None, None) => false,
            _ => true,
        };

        let fileduration = match (&current_user.file_duration, &other_user.file_duration) {
            (Some(current_duration), Some(other_duration)) => {
                match (current_duration.as_f64(), other_duration.as_f64()) {
                    (Some(current_duration), Some(other_duration)) => !session
                        .same_fileduration_with_readiness_autoplay_config(
                            current_duration,
                            other_duration,
                        ),
                    _ => true,
                }
            }
            (None, None) => false,
            _ => true,
        };

        Some(FileDifferenceSummary {
            filename,
            filesize,
            fileduration,
        })
    }

    fn current_user_file_name(&self) -> Option<&str> {
        self.username
            .as_deref()
            .and_then(|username| self.user_file_name(username))
    }

    fn hex_value(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }

    fn percent_decode_lossy(input: &str) -> String {
        let bytes = input.as_bytes();
        let mut decoded = Vec::with_capacity(bytes.len());
        let mut index = 0;

        while index < bytes.len() {
            if bytes[index] == b'%'
                && index + 2 < bytes.len()
                && let (Some(high), Some(low)) = (
                    Self::hex_value(bytes[index + 1]),
                    Self::hex_value(bytes[index + 2]),
                )
            {
                decoded.push((high << 4) | low);
                index += 3;
                continue;
            }
            decoded.push(bytes[index]);
            index += 1;
        }

        String::from_utf8_lossy(&decoded).into_owned()
    }

    fn strip_filename_for_compare(filename: &str, strip_url: bool) -> String {
        let decoded_filename = Self::percent_decode_lossy(filename);
        let normalized_name = if strip_url {
            let last_segment = decoded_filename
                .rsplit('/')
                .next()
                .unwrap_or(&decoded_filename);
            Self::percent_decode_lossy(last_segment)
        } else {
            decoded_filename
        };
        normalized_name
            .chars()
            .filter(|ch| {
                !matches!(
                    ch,
                    '-' | '~' | '_' | '.' | '[' | ']' | '(' | ')' | ':' | ' '
                )
            })
            .collect()
    }

    fn same_hashed_legacy_like(
        left_raw: &str,
        left_hash: &str,
        right_raw: &str,
        right_hash: &str,
    ) -> bool {
        left_raw.to_lowercase() == right_raw.to_lowercase()
            || left_raw == right_raw
            || left_raw == right_hash
            || left_hash == right_raw
            || left_hash == right_hash
    }

    fn is_url(filename: &str) -> bool {
        filename.contains("://")
    }

    fn hash_filename_for_compare(filename: &str) -> String {
        format!("{:x}", Sha256::digest(filename.as_bytes()))[..12].to_owned()
    }

    fn hash_filesize_for_compare(filesize_raw: &str) -> String {
        format!("{:x}", Sha256::digest(filesize_raw.as_bytes()))[..12].to_owned()
    }

    fn filename_with_privacy_mode_legacy_like(
        file_name: &Value,
        privacy_mode: PrivacyMode,
    ) -> Option<String> {
        match privacy_mode {
            PrivacyMode::SendRaw => file_name.as_str().map(str::to_owned),
            PrivacyMode::SendHashed => {
                let raw_name = file_name.as_str()?;
                let strip_url = Self::is_url(raw_name);
                let stripped_name = Self::strip_filename_for_compare(raw_name, strip_url);
                Some(Self::hash_filename_for_compare(&stripped_name))
            }
            PrivacyMode::DoNotSend => Some(PRIVACY_HIDDEN_FILENAME.to_owned()),
        }
    }

    fn filesize_raw_for_privacy(size: &Value) -> String {
        match size {
            Value::Number(number) => number.to_string(),
            Value::String(text) => text.clone(),
            Value::Bool(boolean) => boolean.to_string(),
            Value::Null => "None".to_owned(),
            Value::Array(_) | Value::Object(_) => size.to_string(),
        }
    }

    fn filesize_with_privacy_mode_legacy_like(
        size: &Value,
        privacy_mode: PrivacyMode,
    ) -> Option<Value> {
        match privacy_mode {
            PrivacyMode::SendRaw => Some(size.clone()),
            PrivacyMode::SendHashed => {
                let raw_size = Self::filesize_raw_for_privacy(size);
                Some(Value::String(Self::hash_filesize_for_compare(&raw_size)))
            }
            PrivacyMode::DoNotSend => Some(Value::from(0)),
        }
    }

    fn filesize_is_zero_legacy_like(filesize: &Value) -> bool {
        match filesize {
            Value::Number(number) => {
                if let Some(signed) = number.as_i64() {
                    signed == 0
                } else if let Some(unsigned) = number.as_u64() {
                    unsigned == 0
                } else {
                    number.as_f64().is_some_and(|float| float == 0.0)
                }
            }
            _ => false,
        }
    }

    fn filesize_raw_for_compare(filesize: &Value) -> Option<String> {
        match filesize {
            Value::Number(number) => Some(number.to_string()),
            Value::String(text) => Some(text.clone()),
            _ => None,
        }
    }

    fn same_filesize_legacy_like(left: &Value, right: &Value) -> bool {
        if Self::filesize_is_zero_legacy_like(left) || Self::filesize_is_zero_legacy_like(right) {
            return true;
        }

        let Some(left_raw) = Self::filesize_raw_for_compare(left) else {
            return false;
        };
        let Some(right_raw) = Self::filesize_raw_for_compare(right) else {
            return false;
        };

        let left_hash = Self::hash_filesize_for_compare(&left_raw);
        let right_hash = Self::hash_filesize_for_compare(&right_raw);
        Self::same_hashed_legacy_like(&left_raw, &left_hash, &right_raw, &right_hash)
    }

    fn round_half_to_even(value: f64) -> f64 {
        let floor = value.floor();
        let fraction = value - floor;

        if fraction + ROUND_HALF_EPSILON < 0.5 {
            return floor;
        }
        if fraction - ROUND_HALF_EPSILON > 0.5 {
            return floor + 1.0;
        }

        if floor.rem_euclid(2.0) == 0.0 {
            floor
        } else {
            floor + 1.0
        }
    }

    fn same_fileduration_legacy_like(
        left: f64,
        right: f64,
        show_duration_notification: bool,
        different_duration_threshold: f64,
    ) -> bool {
        if !show_duration_notification {
            return true;
        }

        (Self::round_half_to_even(left) - Self::round_half_to_even(right)).abs()
            < different_duration_threshold
    }

    fn same_filename_legacy_like(left: &str, right: &str) -> bool {
        if left == PRIVACY_HIDDEN_FILENAME || right == PRIVACY_HIDDEN_FILENAME {
            return true;
        }
        let strip_url = Self::is_url(left) ^ Self::is_url(right);
        let left_stripped = Self::strip_filename_for_compare(left, strip_url);
        let right_stripped = Self::strip_filename_for_compare(right, strip_url);
        let left_hash = Self::hash_filename_for_compare(&left_stripped);
        let right_hash = Self::hash_filename_for_compare(&right_stripped);
        Self::same_hashed_legacy_like(&left_stripped, &left_hash, &right_stripped, &right_hash)
    }

    fn all_users_in_current_room_ready(&self) -> bool {
        if !self.local_user_ready() {
            return false;
        }
        let require_same_filenames = self
            .readiness_autoplay_config
            .autoplay_require_same_filenames;
        self.all_other_users_in_current_room_ready()
            && (!require_same_filenames || self.all_users_in_current_room_match_filename())
    }

    fn all_other_users_in_current_room_ready(&self) -> bool {
        let Some((local_username, local_room)) = self.local_username_and_room() else {
            return false;
        };

        self.user_views.iter().all(|(username, user_view)| {
            if username == local_username {
                return true;
            }
            if user_view.room.as_deref() != Some(local_room) {
                return true;
            }
            Self::user_ready_with_file(user_view) != Some(false)
        })
    }

    fn users_in_current_room_count_for_threshold(&self) -> usize {
        let Some((local_username, local_room)) = self.local_username_and_room() else {
            return 0;
        };

        // Legacy usersInRoomCount adds the current user and only counts other room users
        // where isReadyWithFile() is truthy.
        let ready_others = self
            .user_views
            .iter()
            .filter(|(username, user_view)| {
                *username != local_username
                    && user_view.room.as_deref() == Some(local_room)
                    && Self::user_ready_with_file(user_view) == Some(true)
            })
            .count();
        1 + ready_others
    }

    fn all_users_in_current_room_match_filename(&self) -> bool {
        let Some((local_username, local_room)) = self.local_username_and_room() else {
            return false;
        };
        let Some(local_file_name) = self.current_user_file_name() else {
            return false;
        };

        self.user_views.iter().all(|(username, user_view)| {
            if username == local_username || user_view.room.as_deref() != Some(local_room) {
                return true;
            }
            user_view
                .file_name
                .as_deref()
                .is_some_and(|other_file_name| {
                    Self::same_filename_legacy_like(local_file_name, other_file_name)
                })
        })
    }

    fn ready_user_count_in_current_room(&self) -> usize {
        let Some((local_username, local_room)) = self.local_username_and_room() else {
            return 0;
        };

        let mut ready_count = usize::from(self.local_user_ready());
        ready_count += self
            .user_views
            .iter()
            .filter(|(username, user_view)| {
                *username != local_username
                    && user_view.room.as_deref() == Some(local_room)
                    && Self::user_ready_with_file(user_view) == Some(true)
            })
            .count();
        ready_count
    }

    fn current_room_playlist_file_name(&self) -> Option<&str> {
        let playlist = self.current_room_playlist()?;
        if playlist.files.is_empty() {
            return None;
        }

        let selected_index = playlist
            .index
            .and_then(|index| usize::try_from(index).ok())
            .filter(|index| *index < playlist.files.len())
            .unwrap_or(0);
        playlist.files.get(selected_index).map(String::as_str)
    }

    fn playlist_restore_intent_from_room_playlist(
        playlist: &RoomPlaylistView,
    ) -> Option<ReconnectPlaylistRestoreIntent> {
        if playlist.files.is_empty() {
            return None;
        }

        let index = playlist.index.filter(|index| {
            usize::try_from(*index).is_ok_and(|index| index < playlist.files.len())
        });

        Some(ReconnectPlaylistRestoreIntent {
            files: playlist.files.clone(),
            index,
        })
    }

    fn file_payload_from_user_view(user_view: &ClientUserView) -> Option<Value> {
        if !user_view.has_file {
            return None;
        }

        let mut payload = Map::new();
        if let Some(file_name) = user_view.file_name.as_ref() {
            payload.insert("name".to_owned(), Value::String(file_name.clone()));
        }
        if let Some(file_size) = user_view.file_size.as_ref() {
            payload.insert("size".to_owned(), file_size.clone());
        }
        if let Some(file_duration) = user_view.file_duration.as_ref() {
            payload.insert("duration".to_owned(), file_duration.clone());
        }

        Some(Value::Object(payload))
    }

    fn is_music_file_name(file_name: &str) -> bool {
        let lower_name = file_name.to_ascii_lowercase();
        MUSIC_FORMATS
            .iter()
            .any(|music_format| lower_name.ends_with(music_format))
    }

    fn start_autoplay_countdown(&mut self) {
        if !self.autoplay_timer_running {
            self.autoplay_time_left_seconds = self.readiness_autoplay_config.autoplay_delay_seconds;
            self.autoplay_timer_running = true;
        }
    }

    fn stop_autoplay_countdown(&mut self) {
        self.autoplay_timer_running = false;
        self.autoplay_time_left_seconds = self.readiness_autoplay_config.autoplay_delay_seconds;
    }

    fn resolve_room_for_playlist_update(&self, set_by: Option<&str>) -> Option<String> {
        set_by
            .and_then(|username| self.user_room(username).map(str::to_owned))
            .or_else(|| self.room.clone())
    }

    fn set_user_room(&mut self, username: &str, room_name: Option<String>) {
        let (previous_room, ready) = {
            let user_view = self.user_views.entry(username.to_owned()).or_default();
            let previous_room = user_view.room.clone();
            let ready = user_view.ready;
            user_view.room = room_name.clone();
            (previous_room, ready)
        };

        if previous_room != room_name
            && let Some(previous_room_name) = previous_room.as_deref()
        {
            let _ = self.domain.leave_room(username, previous_room_name);
        }

        if let Some(new_room_name) = room_name.as_deref() {
            self.known_rooms.insert(new_room_name.to_owned());
            self.domain.join_room(username, new_room_name);
            let _ = self
                .domain
                .set_ready(username, new_room_name, ready.unwrap_or(false));
        }
    }

    fn set_user_ready(&mut self, username: &str, ready: bool) {
        self.set_user_ready_state(username, Some(ready));
    }

    fn set_user_ready_state(&mut self, username: &str, ready: Option<bool>) {
        let room_name = {
            let user_view = self.user_views.entry(username.to_owned()).or_default();
            user_view.ready = ready;
            user_view.room.clone()
        };

        if let Some(room_name) = room_name {
            self.domain.join_room(username, &room_name);
            let _ = self
                .domain
                .set_ready(username, &room_name, ready.unwrap_or(false));
        }
    }

    fn set_user_file_info(
        &mut self,
        username: &str,
        has_file: bool,
        file_name: Option<String>,
        file_size: Option<Value>,
        file_duration: Option<Value>,
    ) {
        let user_view = self.user_views.entry(username.to_owned()).or_default();
        user_view.has_file = has_file;
        user_view.file_name = if has_file { file_name } else { None };
        user_view.file_size = if has_file { file_size } else { None };
        user_view.file_duration = if has_file { file_duration } else { None };
    }

    fn set_user_controller(&mut self, username: &str, controller: bool) {
        let user_view = self.user_views.entry(username.to_owned()).or_default();
        user_view.controller = controller;
    }

    fn remove_user(&mut self, username: &str) {
        if let Some(user_view) = self.user_views.remove(username)
            && let Some(room_name) = user_view.room
        {
            let _ = self.domain.leave_room(username, &room_name);
        }
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use crate::PRIVACY_HIDDEN_FILENAME;
    use crate::SEEK_THRESHOLD_SECONDS;
    use serde_json::{Value, json};
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;

    use super::{
        AutoplayCountdownNotification, ChatConfig, ChatNotification,
        ClientPingMetricsLegacyCompatible, ClientRuntime, ClientRuntimeAction,
        ClientRuntimeControl, ClientSession, ControllerAuthTransitionNotification,
        DesyncCorrectionAction, FileDifferenceSummary, LEGACY_CHAT_MAX_MESSAGE_LENGTH,
        LEGACY_DIFFERENT_DURATION_THRESHOLD_SECONDS, LEGACY_FALLBACK_MAX_CHAT_MESSAGE_LENGTH,
        PrivacyMode, QueuedRuntimeControl, ReadinessAutoplayConfig,
        ReconnectStateRestoreCorrectionMetrics, ReconnectStateRestoreCorrectionPolicyMode,
        ReconnectTransitionNotification, RoomPlaystateView, UnpauseActionMode,
        UserChangeNotification, unix_wall_clock_time_seconds_legacy_compatible,
    };
    use syncplay_player_api::{
        LocalFileUpdate, PlayerAdapter, PlayerError, PlayerPlaybackTelemetryUpdate,
    };
    use syncplay_protocol::{
        ChatPayload, IgnoringOnTheFlyPayload, ListPayload, PingPayload, PlaystatePayload,
        ProtocolMessage, StatePayload, decode_line, decode_message_line,
    };

    fn scenario_fixture_path(name: &str) -> PathBuf {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("..");
        path.push("..");
        path.push("fixtures");
        path.push("scenarios");
        path.push(name);
        path
    }

    fn replay_python_trace_fixture(name: &str) -> BTreeMap<String, ClientSession> {
        let fixture = fs::read_to_string(scenario_fixture_path(name))
            .expect("python trace fixture should be readable");
        let trace = decode_line(&fixture).expect("python trace fixture should be valid JSON");
        let steps = trace
            .get("steps")
            .and_then(|value| value.as_array())
            .expect("python trace fixture should contain steps array");

        let mut sessions = BTreeMap::new();
        for step in steps {
            let outputs = step
                .get("outputs")
                .and_then(|value| value.as_array())
                .expect("python trace step should contain outputs array");
            for output in outputs {
                let client_id = output
                    .get("client")
                    .and_then(|value| value.as_str())
                    .expect("python trace output should contain client id");
                let message_line = output
                    .get("message")
                    .expect("python trace output should contain message")
                    .to_string();
                let message = decode_message_line(&message_line)
                    .expect("python trace output message should decode");
                sessions
                    .entry(client_id.to_owned())
                    .or_insert_with(ClientSession::default)
                    .apply_protocol_message(message)
                    .expect("python trace output should apply cleanly");
            }
        }

        sessions
    }

    struct DesyncRuntimeScenarioStep {
        now_seconds: f64,
        local_position: f64,
        local_can_control: bool,
        dont_slow_down_with_me: bool,
        speed_supported: bool,
        expected_actions: Vec<ClientRuntimeAction>,
    }

    fn run_desync_runtime_scenario(
        session: &mut ClientSession,
        steps: &[DesyncRuntimeScenarioStep],
    ) {
        for (index, step) in steps.iter().enumerate() {
            let actions = session.runtime_actions_for_desync_correction(
                step.now_seconds,
                step.local_position,
                step.local_can_control,
                step.dont_slow_down_with_me,
                step.speed_supported,
            );
            assert_eq!(
                actions,
                step.expected_actions,
                "desync runtime scenario step {} actions mismatch",
                index + 1
            );
        }
    }

    fn desync_session_with_remote_state(
        global_position: f64,
        paused: bool,
        do_seek: bool,
        set_by: &str,
    ) -> ClientSession {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        let state_line = json!({
            "State": {
                "playstate": {
                    "position": global_position,
                    "paused": paused,
                    "doSeek": do_seek,
                    "setBy": set_by,
                }
            }
        })
        .to_string();
        session
            .apply_message_json(&state_line)
            .expect("remote state should apply");
        session
    }

    #[derive(Default)]
    struct RecordingPlayer {
        paused: Option<bool>,
        position: Option<f64>,
        playback_rate: Option<f64>,
        fail_set_position: bool,
        pending_local_file_update: Option<LocalFileUpdate>,
        pending_playback_telemetry_update: Option<PlayerPlaybackTelemetryUpdate>,
        pending_chat_requests: std::collections::VecDeque<String>,
    }

    impl PlayerAdapter for RecordingPlayer {
        fn name(&self) -> &'static str {
            "recording-player"
        }

        fn set_paused(&mut self, paused: bool) -> Result<(), PlayerError> {
            self.paused = Some(paused);
            Ok(())
        }

        fn set_position(&mut self, position_seconds: f64) -> Result<(), PlayerError> {
            if self.fail_set_position {
                return Err(PlayerError::Unsupported("set_position_failed"));
            }
            self.position = Some(position_seconds);
            Ok(())
        }

        fn set_playback_rate(&mut self, rate: f64) -> Result<(), PlayerError> {
            self.playback_rate = Some(rate);
            Ok(())
        }

        fn take_local_file_update(&mut self) -> Option<LocalFileUpdate> {
            self.pending_local_file_update.take()
        }

        fn take_playback_telemetry_update(&mut self) -> Option<PlayerPlaybackTelemetryUpdate> {
            self.pending_playback_telemetry_update.take()
        }

        fn take_pending_chat_request(&mut self) -> Option<String> {
            self.pending_chat_requests.pop_front()
        }
    }

    #[derive(Default)]
    struct RecordingRuntimeControl {
        room_updates: Vec<String>,
        ready_updates: Vec<(bool, bool)>,
        file_updates: Vec<Value>,
        playlist_updates: Vec<Vec<String>>,
        playlist_index_updates: Vec<i64>,
        controller_auth_requests: Vec<(String, String)>,
        chat_messages: Vec<String>,
        chat_notifications: Vec<ChatNotification>,
        controller_auth_notifications: Vec<ControllerAuthTransitionNotification>,
        user_change_notifications: Vec<UserChangeNotification>,
        reconnect_schedules: Vec<f64>,
        stop_reconnect_calls: usize,
        reconnect_notifications: Vec<ReconnectTransitionNotification>,
    }

    impl ClientRuntimeControl for RecordingRuntimeControl {
        fn set_room(&mut self, room: String) {
            self.room_updates.push(room);
        }

        fn set_ready(&mut self, ready: bool, manually_initiated: bool) {
            self.ready_updates.push((ready, manually_initiated));
        }

        fn set_file(&mut self, file_payload: Value) {
            self.file_updates.push(file_payload);
        }

        fn set_playlist(&mut self, files: Vec<String>) {
            self.playlist_updates.push(files);
        }

        fn set_playlist_index(&mut self, index: i64) {
            self.playlist_index_updates.push(index);
        }

        fn request_controller_auth(&mut self, room: String, password: String) {
            self.controller_auth_requests.push((room, password));
        }

        fn send_chat(&mut self, message: String) {
            self.chat_messages.push(message);
        }

        fn notify_chat(&mut self, notification: ChatNotification) {
            self.chat_notifications.push(notification);
        }

        fn notify_controller_auth_transition(
            &mut self,
            notification: ControllerAuthTransitionNotification,
        ) {
            self.controller_auth_notifications.push(notification);
        }

        fn notify_user_change(&mut self, notification: UserChangeNotification) {
            self.user_change_notifications.push(notification);
        }

        fn schedule_reconnect(&mut self, delay_seconds: f64) {
            self.reconnect_schedules.push(delay_seconds);
        }

        fn stop_reconnect(&mut self) {
            self.stop_reconnect_calls += 1;
        }

        fn notify_reconnect_transition(&mut self, notification: ReconnectTransitionNotification) {
            self.reconnect_notifications.push(notification);
        }
    }

    #[test]
    fn hello_populates_session_state() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("valid hello should parse");

        assert_eq!(session.username.as_deref(), Some("alice"));
        assert_eq!(session.room.as_deref(), Some("room1"));
    }

    #[test]
    fn hello_records_server_readiness_support_flag() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255","features":{"readiness":true}}}"#,
            )
            .expect("hello should apply");

        assert_eq!(session.server_readiness_supported(), Some(true));
    }

    #[test]
    fn hello_records_server_chat_support_flag() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255","features":{"chat":false}}}"#,
            )
            .expect("hello should apply");

        assert_eq!(session.server_chat_supported(), Some(false));
    }

    #[test]
    fn hello_without_features_uses_legacy_version_gate_for_chat_support() {
        let mut old_server_session = ClientSession::default();
        old_server_session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        assert_eq!(old_server_session.server_chat_supported(), Some(false));

        let mut feature_list_session = ClientSession::default();
        feature_list_session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");
        assert_eq!(feature_list_session.server_chat_supported(), Some(true));
    }

    #[test]
    fn hello_applies_server_chat_max_message_length_when_enabled() {
        let mut session = ClientSession::default();
        session.chat_config_mut().max_chat_message_length = 150;
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255","features":{"maxChatMessageLength":12}}}"#,
            )
            .expect("hello should apply");

        assert_eq!(session.chat_config().max_chat_message_length, 12);
    }

    #[test]
    fn hello_without_features_uses_legacy_fallback_chat_max_message_length() {
        let mut session = ClientSession::default();
        session.chat_config_mut().max_chat_message_length = 150;
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");

        assert_eq!(
            session.chat_config().max_chat_message_length,
            LEGACY_FALLBACK_MAX_CHAT_MESSAGE_LENGTH
        );
    }

    #[test]
    fn hello_does_not_override_chat_max_message_length_when_server_sync_disabled() {
        let mut session = ClientSession::default();
        session.chat_config_mut().max_chat_message_length = 23;
        session
            .chat_config_mut()
            .apply_server_max_chat_message_length = false;
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255","features":{"maxChatMessageLength":12}}}"#,
            )
            .expect("hello should apply");

        assert_eq!(session.chat_config().max_chat_message_length, 23);
    }

    #[test]
    fn local_can_control_is_true_for_uncontrolled_room() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        assert_eq!(session.local_can_control(), Some(true));
    }

    #[test]
    fn local_can_control_requires_controller_flag_for_controlled_room() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        assert_eq!(session.local_can_control(), Some(false));

        session
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"room":{"name":"+room:ABCDEF123456"},"controller":true}}}}"#,
            )
            .expect("controller update should apply");
        assert_eq!(session.local_can_control(), Some(true));
    }

    #[test]
    fn noncontroller_event_hide_from_osd_respects_behavior_config_and_controller_flag() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        assert!(
            session.noncontroller_event_hide_from_osd_legacy_compatible("bob"),
            "unknown users are treated as non-controllers when non-controller OSD is disabled"
        );

        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"controller":true}}}}"#,
            )
            .expect("controller update should apply");
        assert!(
            !session.noncontroller_event_hide_from_osd_legacy_compatible("bob"),
            "controllers should remain visible on OSD"
        );

        session.behavior_config_mut().show_noncontroller_osd = true;
        assert!(
            !session.noncontroller_event_hide_from_osd_legacy_compatible("carol"),
            "non-controller OSD override should keep unknown/non-controller users visible"
        );
    }

    #[test]
    fn user_change_join_notification_hides_noncontroller_when_osd_is_disabled() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"+room:ABCDEF123456"}}}}}"#,
            )
            .expect("join update should apply");

        assert_eq!(
            session.runtime_actions_for_user_change_notifications_if_needed(),
            vec![ClientRuntimeAction::NotifyUserChange(
                UserChangeNotification::Joined {
                    username: "bob".to_owned(),
                    room: "+room:ABCDEF123456".to_owned(),
                    hide_from_osd: true,
                },
            )]
        );
    }

    #[test]
    fn user_change_playing_notification_respects_controller_visibility_override() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"+room:ABCDEF123456"},"controller":true}}}}"#,
            )
            .expect("controller update should apply");
        let _ = session.runtime_actions_for_user_change_notifications_if_needed();

        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"file":{"name":"movie.mkv","duration":123.4}}}}}"#,
            )
            .expect("file update should apply");

        assert_eq!(
            session.runtime_actions_for_user_change_notifications_if_needed(),
            vec![ClientRuntimeAction::NotifyUserChange(
                UserChangeNotification::Playing {
                    username: "bob".to_owned(),
                    room: "+room:ABCDEF123456".to_owned(),
                    file_name: Some("movie.mkv".to_owned()),
                    file_duration: Some(json!(123.4)),
                    include_room_addendum: false,
                    hide_from_osd: false,
                },
            )]
        );
    }

    #[test]
    fn user_change_playing_notification_room_addendum_matches_legacy_room_scope() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"controller":true}}}}"#,
            )
            .expect("controller update should apply");
        let _ = session.runtime_actions_for_user_change_notifications_if_needed();

        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"file":{"name":"movie.mkv","duration":123.4}}}}}"#,
            )
            .expect("same-room file update should apply");
        assert_eq!(
            session.runtime_actions_for_user_change_notifications_if_needed(),
            vec![ClientRuntimeAction::NotifyUserChange(
                UserChangeNotification::Playing {
                    username: "bob".to_owned(),
                    room: "room1".to_owned(),
                    file_name: Some("movie.mkv".to_owned()),
                    file_duration: Some(json!(123.4)),
                    include_room_addendum: false,
                    hide_from_osd: false,
                },
            )]
        );

        session
            .apply_message_json(r#"{"Set":{"user":{"bob":{"room":{"name":"room2"}}}}}"#)
            .expect("different-room update should apply");
        assert_eq!(
            session.runtime_actions_for_user_change_notifications_if_needed(),
            vec![ClientRuntimeAction::NotifyUserChange(
                UserChangeNotification::Playing {
                    username: "bob".to_owned(),
                    room: "room2".to_owned(),
                    file_name: Some("movie.mkv".to_owned()),
                    file_duration: Some(json!(123.4)),
                    include_room_addendum: true,
                    hide_from_osd: false,
                },
            )]
        );
    }

    #[test]
    fn user_change_notifications_respect_different_room_visibility_toggle() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room2"},"controller":true}}}}"#,
            )
            .expect("join update should apply");
        assert_eq!(
            session.runtime_actions_for_user_change_notifications_if_needed(),
            vec![ClientRuntimeAction::NotifyUserChange(
                UserChangeNotification::Joined {
                    username: "bob".to_owned(),
                    room: "room2".to_owned(),
                    hide_from_osd: true,
                },
            )]
        );

        session.behavior_config_mut().show_different_room_osd = true;
        session
            .apply_message_json(
                r#"{"Set":{"user":{"carol":{"room":{"name":"room3"},"controller":true}}}}"#,
            )
            .expect("second join update should apply");
        assert_eq!(
            session.runtime_actions_for_user_change_notifications_if_needed(),
            vec![ClientRuntimeAction::NotifyUserChange(
                UserChangeNotification::Joined {
                    username: "carol".to_owned(),
                    room: "room3".to_owned(),
                    hide_from_osd: false,
                },
            )]
        );
    }

    #[test]
    fn user_change_notifications_respect_osd_warnings_toggle_for_same_room_events() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"controller":true}}}}"#,
            )
            .expect("same-room join should apply");
        assert_eq!(
            session.runtime_actions_for_user_change_notifications_if_needed(),
            vec![ClientRuntimeAction::NotifyUserChange(
                UserChangeNotification::Joined {
                    username: "bob".to_owned(),
                    room: "room1".to_owned(),
                    hide_from_osd: false,
                },
            )]
        );

        session.behavior_config_mut().show_osd_warnings = false;
        session
            .apply_message_json(
                r#"{"Set":{"user":{"carol":{"room":{"name":"room1"},"controller":true}}}}"#,
            )
            .expect("second same-room join should apply");
        assert_eq!(
            session.runtime_actions_for_user_change_notifications_if_needed(),
            vec![ClientRuntimeAction::NotifyUserChange(
                UserChangeNotification::Joined {
                    username: "carol".to_owned(),
                    room: "room1".to_owned(),
                    hide_from_osd: true,
                },
            )]
        );
    }

    #[test]
    fn user_change_notifications_are_not_gated_by_show_same_room_osd() {
        let mut session = ClientSession::default();
        session.behavior_config_mut().show_same_room_osd = false;
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"controller":true}}}}"#,
            )
            .expect("same-room join should apply");
        assert_eq!(
            session.runtime_actions_for_user_change_notifications_if_needed(),
            vec![ClientRuntimeAction::NotifyUserChange(
                UserChangeNotification::Joined {
                    username: "bob".to_owned(),
                    room: "room1".to_owned(),
                    hide_from_osd: false,
                },
            )]
        );
    }

    #[test]
    fn user_change_room_switch_uses_previous_room_for_visibility_scope() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"controller":true}}}}"#,
            )
            .expect("same-room join should apply");
        let _ = session.runtime_actions_for_user_change_notifications_if_needed();

        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room2"},"controller":true}}}}"#,
            )
            .expect("room switch should apply");
        assert_eq!(
            session.runtime_actions_for_user_change_notifications_if_needed(),
            vec![ClientRuntimeAction::NotifyUserChange(
                UserChangeNotification::Joined {
                    username: "bob".to_owned(),
                    room: "room2".to_owned(),
                    hide_from_osd: false,
                },
            )]
        );

        session.behavior_config_mut().show_osd_warnings = false;
        session
            .apply_message_json(
                r#"{"Set":{"user":{"carol":{"room":{"name":"room1"},"controller":true}}}}"#,
            )
            .expect("carol join should apply");
        let _ = session.runtime_actions_for_user_change_notifications_if_needed();
        session
            .apply_message_json(
                r#"{"Set":{"user":{"carol":{"room":{"name":"room2"},"controller":true}}}}"#,
            )
            .expect("carol room switch should apply");
        assert_eq!(
            session.runtime_actions_for_user_change_notifications_if_needed(),
            vec![ClientRuntimeAction::NotifyUserChange(
                UserChangeNotification::Joined {
                    username: "carol".to_owned(),
                    room: "room2".to_owned(),
                    hide_from_osd: true,
                },
            )]
        );
    }

    #[test]
    fn user_left_notifications_respect_same_and_different_room_visibility() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"controller":true}}}}"#,
            )
            .expect("same-room join should apply");
        let _ = session.runtime_actions_for_user_change_notifications_if_needed();
        session
            .apply_message_json(r#"{"Set":{"user":{"bob":{"event":{"left":true}}}}}"#)
            .expect("same-room left should apply");
        assert_eq!(
            session.runtime_actions_for_user_change_notifications_if_needed(),
            vec![ClientRuntimeAction::NotifyUserChange(
                UserChangeNotification::Left {
                    username: "bob".to_owned(),
                    hide_from_osd: false,
                },
            )]
        );

        session
            .apply_message_json(
                r#"{"Set":{"user":{"carol":{"room":{"name":"room2"},"controller":true}}}}"#,
            )
            .expect("different-room join should apply");
        let _ = session.runtime_actions_for_user_change_notifications_if_needed();
        session
            .apply_message_json(r#"{"Set":{"user":{"carol":{"event":{"left":true}}}}}"#)
            .expect("different-room left should apply");
        assert_eq!(
            session.runtime_actions_for_user_change_notifications_if_needed(),
            vec![ClientRuntimeAction::NotifyUserChange(
                UserChangeNotification::Left {
                    username: "carol".to_owned(),
                    hide_from_osd: true,
                },
            )]
        );
    }

    #[test]
    fn controller_auth_success_sets_controller_flag_for_target_user() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        assert_eq!(session.local_can_control(), Some(false));

        session
            .apply_message_json(
                r#"{"Set":{"controllerAuth":{"user":"alice","room":"+room:ABCDEF123456","success":true}}}"#,
            )
            .expect("controller auth success should apply");
        assert_eq!(session.user_controller("alice"), Some(true));
        assert_eq!(session.local_can_control(), Some(true));
    }

    #[test]
    fn controller_auth_success_emits_transition_notification() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        session
            .apply_message_json(
                r#"{"Set":{"controllerAuth":{"user":"alice","room":"+room:ABCDEF123456","success":true}}}"#,
            )
            .expect("controller auth success should apply");

        assert_eq!(
            session.runtime_actions_for_controller_auth_notifications_if_needed(),
            vec![ClientRuntimeAction::NotifyControllerAuthTransition(
                ControllerAuthTransitionNotification::Succeeded {
                    username: "alice".to_owned(),
                    room: "+room:ABCDEF123456".to_owned(),
                    hide_from_osd: false,
                },
            )]
        );
        assert!(
            session
                .runtime_actions_for_controller_auth_notifications_if_needed()
                .is_empty(),
            "controller auth notifications should drain after first retrieval"
        );
    }

    #[test]
    fn controller_auth_success_for_same_room_user_emits_transition_notification() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        session
            .apply_message_json(
                r#"{"Set":{"controllerAuth":{"user":"bob","room":"room1","success":true}}}"#,
            )
            .expect("controller auth success should apply");

        assert_eq!(
            session.runtime_actions_for_controller_auth_notifications_if_needed(),
            vec![ClientRuntimeAction::NotifyControllerAuthTransition(
                ControllerAuthTransitionNotification::Succeeded {
                    username: "bob".to_owned(),
                    room: "room1".to_owned(),
                    hide_from_osd: false,
                },
            )]
        );
        assert_eq!(session.user_controller("bob"), Some(true));
    }

    #[test]
    fn controller_auth_success_hides_from_osd_when_same_room_osd_is_disabled() {
        let mut session = ClientSession::default();
        session.behavior_config_mut().show_same_room_osd = false;
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        session
            .apply_message_json(
                r#"{"Set":{"controllerAuth":{"user":"alice","room":"room1","success":true}}}"#,
            )
            .expect("controller auth success should apply");

        assert_eq!(
            session.runtime_actions_for_controller_auth_notifications_if_needed(),
            vec![ClientRuntimeAction::NotifyControllerAuthTransition(
                ControllerAuthTransitionNotification::Succeeded {
                    username: "alice".to_owned(),
                    room: "room1".to_owned(),
                    hide_from_osd: true,
                },
            )]
        );
    }

    #[test]
    fn controller_auth_success_for_different_room_suppresses_transition_notification() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        session
            .apply_message_json(
                r#"{"Set":{"controllerAuth":{"user":"bob","room":"room2","success":true}}}"#,
            )
            .expect("controller auth success should apply");

        assert!(
            session
                .runtime_actions_for_controller_auth_notifications_if_needed()
                .is_empty(),
            "controller-auth success should only notify in local room"
        );
        assert_eq!(session.user_controller("bob"), Some(true));
    }

    #[test]
    fn controller_auth_failure_emits_transition_notification() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        session
            .apply_message_json(
                r#"{"Set":{"controllerAuth":{"user":"alice","room":"+room:ABCDEF123456","success":false}}}"#,
            )
            .expect("controller auth failure should apply");

        assert_eq!(
            session.runtime_actions_for_controller_auth_notifications_if_needed(),
            vec![ClientRuntimeAction::NotifyControllerAuthTransition(
                ControllerAuthTransitionNotification::Failed {
                    username: "alice".to_owned(),
                    room: "+room:ABCDEF123456".to_owned(),
                    hide_from_osd: true,
                },
            )]
        );
        assert!(
            session
                .runtime_actions_for_controller_auth_notifications_if_needed()
                .is_empty(),
            "controller auth notifications should drain after first retrieval"
        );
    }

    #[test]
    fn controller_auth_failure_hide_from_osd_respects_show_noncontroller_osd_setting() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        session
            .apply_message_json(
                r#"{"Set":{"controllerAuth":{"user":"alice","room":"+room:ABCDEF123456","success":false}}}"#,
            )
            .expect("controller auth failure should apply");
        assert_eq!(
            session.runtime_actions_for_controller_auth_notifications_if_needed(),
            vec![ClientRuntimeAction::NotifyControllerAuthTransition(
                ControllerAuthTransitionNotification::Failed {
                    username: "alice".to_owned(),
                    room: "+room:ABCDEF123456".to_owned(),
                    hide_from_osd: true,
                },
            )]
        );

        session.behavior_config_mut().show_noncontroller_osd = true;
        session
            .apply_message_json(
                r#"{"Set":{"controllerAuth":{"user":"alice","room":"+room:ABCDEF123456","success":false}}}"#,
            )
            .expect("controller auth failure should apply");
        assert_eq!(
            session.runtime_actions_for_controller_auth_notifications_if_needed(),
            vec![ClientRuntimeAction::NotifyControllerAuthTransition(
                ControllerAuthTransitionNotification::Failed {
                    username: "alice".to_owned(),
                    room: "+room:ABCDEF123456".to_owned(),
                    hide_from_osd: false,
                },
            )]
        );
    }

    #[test]
    fn controller_auth_failure_for_other_user_suppresses_transition_notification() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        session
            .apply_message_json(
                r#"{"Set":{"controllerAuth":{"user":"bob","room":"+room:ABCDEF123456","success":false}}}"#,
            )
            .expect("controller auth failure should apply");

        assert!(
            session
                .runtime_actions_for_controller_auth_notifications_if_needed()
                .is_empty(),
            "controller-auth failure should only notify for local user"
        );
    }

    #[test]
    fn new_controlled_room_message_queues_room_switch_and_auth_request() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        assert_eq!(session.local_can_control(), Some(true));

        session
            .apply_message_json(
                r#"{"Set":{"newControlledRoom":{"roomName":"+room:ABCDEF123456","password":"AB-123-456"}}}"#,
            )
            .expect("new controlled room message should apply");

        assert_eq!(session.room.as_deref(), Some("+room:ABCDEF123456"));
        assert_eq!(session.user_room("alice"), Some("+room:ABCDEF123456"));
        assert_eq!(session.user_controller("alice"), Some(false));
        assert_eq!(session.local_can_control(), Some(false));

        let actions = session.runtime_actions_for_controller_reidentify_if_needed();
        assert_eq!(
            actions,
            vec![
                ClientRuntimeAction::SetRoom {
                    room: "+room:ABCDEF123456".to_owned(),
                },
                ClientRuntimeAction::NotifyControllerAuthTransition(
                    ControllerAuthTransitionNotification::Attempting {
                        room: "+room:ABCDEF123456".to_owned(),
                    },
                ),
                ClientRuntimeAction::RequestControllerAuth {
                    room: "+room:ABCDEF123456".to_owned(),
                    password: "AB-123-456".to_owned(),
                },
            ]
        );
        assert!(
            session
                .runtime_actions_for_controller_reidentify_if_needed()
                .is_empty(),
            "controller reidentify actions should drain after first retrieval"
        );
    }

    #[test]
    fn controller_reidentify_action_emits_after_hello_when_password_is_stored() {
        let mut session = ClientSession::default();
        session.remember_control_password_for_room("+room:ABCDEF123456", "ab-123-456 !!");

        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        let actions = session.runtime_actions_for_controller_reidentify_if_needed();
        assert_eq!(
            actions,
            vec![
                ClientRuntimeAction::NotifyControllerAuthTransition(
                    ControllerAuthTransitionNotification::Attempting {
                        room: "+room:ABCDEF123456".to_owned(),
                    },
                ),
                ClientRuntimeAction::RequestControllerAuth {
                    room: "+room:ABCDEF123456".to_owned(),
                    password: "AB-123-456".to_owned(),
                },
            ]
        );
        assert!(
            session
                .runtime_actions_for_controller_reidentify_if_needed()
                .is_empty(),
            "controller reidentify actions should drain after first retrieval"
        );
    }

    #[test]
    fn new_controlled_room_message_stores_password_for_future_reidentify() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Set":{"newControlledRoom":{"roomName":"+room:ABCDEF123456","password":"AB-123-456"}}}"#,
            )
            .expect("new controlled room message should apply");
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        assert_eq!(
            session.runtime_actions_for_controller_reidentify_if_needed(),
            vec![
                ClientRuntimeAction::NotifyControllerAuthTransition(
                    ControllerAuthTransitionNotification::Attempting {
                        room: "+room:ABCDEF123456".to_owned(),
                    },
                ),
                ClientRuntimeAction::RequestControllerAuth {
                    room: "+room:ABCDEF123456".to_owned(),
                    password: "AB-123-456".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn non_hello_message_is_ignored() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(r#"{"Chat":"hello"}"#)
            .expect_err("chat should not be accepted by hello-only parser");
    }

    #[test]
    fn dispatch_runtime_actions_applies_player_and_control_operations() {
        let actions = vec![
            ClientRuntimeAction::SetPaused(true),
            ClientRuntimeAction::SetRoom {
                room: "room2".to_owned(),
            },
            ClientRuntimeAction::SetPosition(42.0),
            ClientRuntimeAction::SetPlaybackRate(0.95),
            ClientRuntimeAction::SetReady {
                ready: true,
                manually_initiated: false,
            },
            ClientRuntimeAction::SetFile {
                file_payload: json!({"name":"movie.mkv","size":123456789}),
            },
            ClientRuntimeAction::SetPlaylist {
                files: vec!["ep1.mkv".to_owned(), "ep2.mkv".to_owned()],
            },
            ClientRuntimeAction::SetPlaylistIndex { index: 1 },
            ClientRuntimeAction::RequestControllerAuth {
                room: "+room:ABCDEF123456".to_owned(),
                password: "AB-123-456".to_owned(),
            },
            ClientRuntimeAction::SendChat {
                message: "hello room".to_owned(),
            },
            ClientRuntimeAction::NotifyChat(ChatNotification::Message {
                username: Some("bob".to_owned()),
                message: "hi".to_owned(),
            }),
            ClientRuntimeAction::NotifyControllerAuthTransition(
                ControllerAuthTransitionNotification::Attempting {
                    room: "+room:ABCDEF123456".to_owned(),
                },
            ),
            ClientRuntimeAction::NotifyReconnectTransition(
                ReconnectTransitionNotification::Attempting {
                    retries: 3,
                    delay_seconds: 0.8,
                },
            ),
            ClientRuntimeAction::ScheduleReconnect { delay_seconds: 0.4 },
            ClientRuntimeAction::StopReconnect,
        ];

        let mut player = RecordingPlayer::default();
        let mut control = RecordingRuntimeControl::default();
        ClientSession::dispatch_runtime_actions(&actions, &mut player, &mut control)
            .expect("runtime actions should dispatch cleanly");

        assert_eq!(player.paused, Some(true));
        assert_eq!(player.position, Some(42.0));
        assert_eq!(player.playback_rate, Some(0.95));
        assert_eq!(control.room_updates, vec!["room2".to_owned()]);
        assert_eq!(control.ready_updates, vec![(true, false)]);
        assert_eq!(
            control.file_updates,
            vec![json!({"name":"movie.mkv","size":123456789})]
        );
        assert_eq!(
            control.playlist_updates,
            vec![vec!["ep1.mkv".to_owned(), "ep2.mkv".to_owned()]]
        );
        assert_eq!(control.playlist_index_updates, vec![1]);
        assert_eq!(
            control.controller_auth_requests,
            vec![("+room:ABCDEF123456".to_owned(), "AB-123-456".to_owned())]
        );
        assert_eq!(control.chat_messages, vec!["hello room".to_owned()]);
        assert_eq!(
            control.chat_notifications,
            vec![ChatNotification::Message {
                username: Some("bob".to_owned()),
                message: "hi".to_owned(),
            }]
        );
        assert_eq!(
            control.controller_auth_notifications,
            vec![ControllerAuthTransitionNotification::Attempting {
                room: "+room:ABCDEF123456".to_owned(),
            }]
        );
        assert_eq!(
            control.reconnect_notifications,
            vec![ReconnectTransitionNotification::Attempting {
                retries: 3,
                delay_seconds: 0.8,
            }]
        );
        assert_eq!(control.reconnect_schedules, vec![0.4]);
        assert_eq!(control.stop_reconnect_calls, 1);
    }

    #[test]
    fn dispatch_runtime_actions_stops_on_player_error() {
        let actions = vec![
            ClientRuntimeAction::SetPaused(true),
            ClientRuntimeAction::SetPosition(5.0),
            ClientRuntimeAction::SetReady {
                ready: true,
                manually_initiated: true,
            },
        ];

        let mut player = RecordingPlayer {
            fail_set_position: true,
            ..RecordingPlayer::default()
        };
        let mut control = RecordingRuntimeControl::default();
        let err = ClientSession::dispatch_runtime_actions(&actions, &mut player, &mut control)
            .expect_err("dispatch should bubble player failures");

        assert_eq!(err, PlayerError::Unsupported("set_position_failed"));
        assert_eq!(player.paused, Some(true));
        assert!(control.ready_updates.is_empty());
    }

    #[test]
    fn apply_protocol_message_applies_chat_without_mutating_identity_state() {
        let mut session = ClientSession::default();
        let message = ProtocolMessage::chat_text("hello");
        session
            .apply_protocol_message(message)
            .expect("chat protocol message should apply");
        assert!(session.username.is_none());
        assert!(session.room.is_none());
        assert_eq!(
            session.runtime_actions_for_chat_notifications_if_needed(),
            vec![ClientRuntimeAction::NotifyChat(ChatNotification::Message {
                username: None,
                message: "hello".to_owned(),
            })]
        );
        assert!(
            session
                .runtime_actions_for_chat_notifications_if_needed()
                .is_empty(),
            "chat notifications should drain after first retrieval"
        );
    }

    #[test]
    fn list_set_and_state_messages_reconcile_client_view() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello message should apply");

        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room2"},"file":{"name":"bob.mp4","size":"15e2b0d3c338","duration":95.5},"isReady":true,"controller":true}}}}"#,
            )
            .expect("set user message should apply");
        assert_eq!(session.user_room("bob"), Some("room2"));
        assert_eq!(session.user_ready("bob"), Some(true));
        assert_eq!(session.user_file_name("bob"), Some("bob.mp4"));
        assert_eq!(session.user_file_size("bob"), Some(&json!("15e2b0d3c338")));
        assert_eq!(session.user_file_duration("bob"), Some(&json!(95.5)));
        assert_eq!(session.user_controller("bob"), Some(true));

        session
            .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
            .expect("set ready message should apply");
        assert_eq!(session.user_ready("alice"), Some(true));

        session
            .apply_message_json(
                r#"{"List":{"room1":{"alice":{"isReady":true,"controller":false}},"room2":{"bob":{"isReady":false,"controller":true}}}}"#,
            )
            .expect("list snapshot should apply");
        assert_eq!(session.user_room("alice"), Some("room1"));
        assert_eq!(session.user_room("bob"), Some("room2"));
        assert_eq!(session.user_ready("bob"), Some(false));
        assert_eq!(session.user_file_name("bob"), None);
        assert_eq!(session.user_file_size("bob"), None);
        assert_eq!(session.user_file_duration("bob"), None);
        assert_eq!(session.user_controller("bob"), Some(true));

        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":42.0,"paused":false,"doSeek":true,"setBy":"alice"}}}"#,
            )
            .expect("state message should apply");
        let playstate = session
            .current_room_playstate()
            .expect("current room playstate should exist");
        assert_eq!(playstate.position, Some(42.0));
        assert_eq!(playstate.paused, Some(false));
        assert_eq!(playstate.do_seek, Some(true));
        assert_eq!(playstate.set_by.as_deref(), Some("alice"));
    }

    #[test]
    fn set_user_left_event_removes_user_from_view() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello message should apply");

        session
            .apply_message_json(r#"{"Set":{"user":{"bob":{"room":{"name":"room1"}}}}}"#)
            .expect("joined user should be tracked");
        assert_eq!(session.user_room("bob"), Some("room1"));

        session
            .apply_message_json(r#"{"Set":{"user":{"bob":{"event":{"left":true}}}}}"#)
            .expect("left event should be accepted");
        assert_eq!(session.user_room("bob"), None);
    }

    #[test]
    fn set_user_falsy_file_payload_does_not_clear_existing_file() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"bob.mp4","size":123456789,"duration":95.5},"isReady":true}}}}"#,
            )
            .expect("initial user file should apply");
        assert_eq!(session.user_has_file("bob"), Some(true));
        assert_eq!(session.user_file_name("bob"), Some("bob.mp4"));
        assert_eq!(session.user_file_size("bob"), Some(&json!(123456789)));
        assert_eq!(session.user_file_duration("bob"), Some(&json!(95.5)));

        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{},"isReady":true}}}}"#,
            )
            .expect("falsy file payload should be accepted");
        assert_eq!(session.user_has_file("bob"), Some(true));
        assert_eq!(session.user_file_name("bob"), Some("bob.mp4"));
        assert_eq!(session.user_file_size("bob"), Some(&json!(123456789)));
        assert_eq!(session.user_file_duration("bob"), Some(&json!(95.5)));
    }

    #[test]
    fn list_snapshot_file_payload_can_clear_existing_file_state() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"bob.mp4","size":123456789,"duration":95.5},"isReady":true}}}}"#,
            )
            .expect("initial user file should apply");
        assert_eq!(session.user_has_file("bob"), Some(true));
        assert_eq!(session.user_file_name("bob"), Some("bob.mp4"));
        assert_eq!(session.user_file_size("bob"), Some(&json!(123456789)));
        assert_eq!(session.user_file_duration("bob"), Some(&json!(95.5)));

        session
            .apply_message_json(
                r#"{"List":{"room1":{"alice":{"isReady":true,"file":{"name":"alice.mp4"}},"bob":{"isReady":true,"file":{}}}}}"#,
            )
            .expect("list snapshot should apply");
        assert_eq!(session.user_has_file("bob"), Some(false));
        assert_eq!(session.user_file_name("bob"), None);
        assert_eq!(session.user_file_size("bob"), None);
        assert_eq!(session.user_file_duration("bob"), None);
    }

    #[test]
    fn list_snapshot_file_payload_tracks_mixed_raw_and_hashed_metadata() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"List":{"room1":{"alice":{"isReady":true,"file":{"name":"**Hidden filename**","size":"15e2b0d3c338","duration":95}},"bob":{"isReady":true,"file":{"name":"movie.mkv","size":123456789,"duration":95.5}}}}}"#,
            )
            .expect("list snapshot with mixed file metadata should apply");

        assert_eq!(session.user_has_file("alice"), Some(true));
        assert_eq!(session.user_file_name("alice"), Some("**Hidden filename**"));
        assert_eq!(
            session.user_file_size("alice"),
            Some(&json!("15e2b0d3c338"))
        );
        assert_eq!(session.user_file_duration("alice"), Some(&json!(95)));

        assert_eq!(session.user_has_file("bob"), Some(true));
        assert_eq!(session.user_file_name("bob"), Some("movie.mkv"));
        assert_eq!(session.user_file_size("bob"), Some(&json!(123456789)));
        assert_eq!(session.user_file_duration("bob"), Some(&json!(95.5)));
    }

    #[test]
    fn top_level_set_file_is_ignored_for_local_user_state() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        assert_eq!(session.user_has_file("alice"), Some(false));
        assert_eq!(session.user_file_name("alice"), None);
        assert_eq!(session.user_file_size("alice"), None);
        assert_eq!(session.user_file_duration("alice"), None);

        session
            .apply_message_json(
                r#"{"Set":{"file":{"name":"movie.mkv","duration":95.5,"size":123456789}}}"#,
            )
            .expect("set file should apply");
        assert_eq!(session.user_has_file("alice"), Some(false));
        assert_eq!(session.user_file_name("alice"), None);
        assert_eq!(session.user_file_size("alice"), None);
        assert_eq!(session.user_file_duration("alice"), None);

        session
            .apply_message_json(r#"{"Set":{"file":{}}}"#)
            .expect("empty set file should apply");
        assert_eq!(session.user_has_file("alice"), Some(false));
        assert_eq!(session.user_file_name("alice"), None);
        assert_eq!(session.user_file_size("alice"), None);
        assert_eq!(session.user_file_duration("alice"), None);
    }

    #[test]
    fn is_playing_music_uses_current_playlist_item_extension() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["video.mp4","song.FLAC"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
            .expect("playlist index should apply");
        assert!(session.is_playing_music());

        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
            .expect("playlist index should apply");
        assert!(!session.is_playing_music());
    }

    #[test]
    fn recently_advanced_tracks_local_playlist_index_updates() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        session
            .apply_message_json_at(
                r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#,
                10.0,
            )
            .expect("local playlist index should apply");
        assert!(session.recently_advanced(17.9));
        assert!(!session.recently_advanced(18.1));

        session
            .apply_message_json_at(
                r#"{"Set":{"playlistIndex":{"index":2,"user":"bob"}}}"#,
                20.0,
            )
            .expect("remote playlist index should apply");
        assert!(!session.recently_advanced(20.1));
    }

    #[test]
    fn python_trace_fanout_scenario_reconciles_client_sessions() {
        let sessions = replay_python_trace_fixture("server_runtime_fanout.python_trace.json");

        let client_1 = sessions
            .get("client-1")
            .expect("fanout trace should include client-1 session");
        assert_eq!(client_1.username.as_deref(), Some("alice"));
        assert_eq!(client_1.room.as_deref(), Some("room1"));
        assert_eq!(client_1.user_room("bob"), Some("room2"));
        assert_eq!(client_1.user_ready("alice"), Some(true));
        assert_eq!(client_1.user_ready("bob"), Some(false));
        let client_1_playlist = client_1
            .current_room_playlist()
            .expect("client-1 should have current room playlist");
        assert!(client_1_playlist.files.is_empty());

        let client_2 = sessions
            .get("client-2")
            .expect("fanout trace should include client-2 session");
        assert_eq!(client_2.username.as_deref(), Some("bob"));
        assert_eq!(client_2.room.as_deref(), Some("room2"));
        assert_eq!(client_2.user_room("alice"), Some("room1"));
        assert_eq!(client_2.user_ready("alice"), Some(true));
        let client_2_playstate = client_2
            .current_room_playstate()
            .expect("client-2 should have current room playstate");
        assert_eq!(client_2_playstate.position, Some(10.0));
        assert_eq!(client_2_playstate.paused, Some(false));
        assert_eq!(client_2_playstate.do_seek, Some(false));
        assert_eq!(client_2_playstate.set_by.as_deref(), Some("bob"));
    }

    #[test]
    fn python_trace_cross_room_ready_list_reconciles_room_membership_and_readiness() {
        let sessions =
            replay_python_trace_fixture("server_runtime_cross_room_ready_list.python_trace.json");

        let client_3 = sessions
            .get("client-3")
            .expect("cross-room trace should include client-3 session");
        assert_eq!(client_3.username.as_deref(), Some("carol"));
        assert_eq!(client_3.room.as_deref(), Some("room1"));
        assert_eq!(client_3.user_room("alice"), Some("room1"));
        assert_eq!(client_3.user_room("bob"), Some("room1"));
        assert_eq!(client_3.user_room("carol"), Some("room1"));
        assert_eq!(client_3.user_ready("alice"), Some(true));
        assert_eq!(client_3.user_ready("bob"), Some(false));
        assert_eq!(client_3.user_ready("carol"), Some(true));
        let room_playstate = client_3
            .room_playstate("room1")
            .expect("client-3 should have room1 playstate");
        assert_eq!(room_playstate.position, Some(0.0));
        assert_eq!(room_playstate.paused, Some(true));
        assert_eq!(room_playstate.do_seek, Some(false));
    }

    #[test]
    fn python_trace_controlled_room_state_forced_correction_reconciles_forced_state_and_room_membership()
     {
        let sessions = replay_python_trace_fixture(
            "server_runtime_controlled_room_state_forced_correction.python_trace.json",
        );
        let controlled_room = "+room1:CB39A19549E8";

        let client_1 = sessions
            .get("client-1")
            .expect("forced-correction trace should include client-1 session");
        assert_eq!(client_1.username.as_deref(), Some("alice"));
        assert_eq!(client_1.room.as_deref(), Some(controlled_room));
        assert_eq!(client_1.user_room("alice"), Some(controlled_room));
        assert_eq!(client_1.user_room("bob"), Some(controlled_room));
        let client_1_playstate = client_1
            .current_room_playstate()
            .expect("client-1 should track controlled room playstate");
        assert_eq!(client_1_playstate.position, Some(0.0));
        assert_eq!(client_1_playstate.paused, Some(true));
        assert_eq!(client_1_playstate.do_seek, Some(true));
        let client_1_playlist = client_1
            .current_room_playlist()
            .expect("client-1 should keep controlled room playlist snapshot");
        assert!(
            client_1_playlist.files.is_empty(),
            "controlled room playlist should remain empty in forced-correction scenario"
        );

        let client_2 = sessions
            .get("client-2")
            .expect("forced-correction trace should include client-2 session");
        assert_eq!(client_2.username.as_deref(), Some("bob"));
        assert_eq!(client_2.room.as_deref(), Some(controlled_room));
        assert_eq!(client_2.user_room("alice"), Some(controlled_room));
        assert_eq!(client_2.user_room("bob"), Some(controlled_room));
        let client_2_playstate = client_2
            .current_room_playstate()
            .expect("client-2 should track controlled room playstate");
        assert_eq!(client_2_playstate.position, Some(0.0));
        assert_eq!(client_2_playstate.paused, Some(true));
        assert_eq!(client_2_playstate.do_seek, Some(true));
        let client_2_playlist = client_2
            .current_room_playlist()
            .expect("client-2 should keep controlled room playlist snapshot");
        assert!(
            client_2_playlist.files.is_empty(),
            "controlled room playlist should remain empty in forced-correction scenario"
        );
    }

    #[test]
    fn reconcile_state_builds_client_ignore_and_waits_for_ack_before_applying_new_global_state() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":true,"doSeek":false,"setBy":"alice"}}}"#,
            )
            .expect("initial state should apply");

        let outbound = session.reconcile_state_and_build_response(
            StatePayload::new(),
            12.0,
            false,
            123.0,
            0.12,
        );
        let outbound_playstate = outbound
            .playstate
            .as_ref()
            .expect("outbound state should include playstate");
        assert_eq!(outbound_playstate.position, Some(12.0));
        assert_eq!(outbound_playstate.paused, Some(false));
        assert_eq!(outbound_playstate.do_seek, None);
        assert_eq!(session.client_ignoring_on_the_fly(), 1);
        assert_eq!(
            outbound
                .ignoring_on_the_fly
                .as_ref()
                .and_then(|ignore| ignore.client),
            Some(1)
        );

        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":1.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
            )
            .expect("local state reflection should apply");

        let inbound_without_ack = StatePayload::new().with_playstate(
            PlaystatePayload::new()
                .with_position(99.0)
                .with_paused(true)
                .with_do_seek(true)
                .with_set_by("bob"),
        );
        let outbound_while_waiting = session.reconcile_state_and_build_response(
            inbound_without_ack,
            12.0,
            false,
            124.0,
            0.13,
        );
        assert!(
            outbound_while_waiting.playstate.is_none(),
            "outbound playstate should be suppressed while waiting for client ignore ack"
        );
        let preserved = session
            .current_room_playstate()
            .expect("room playstate should remain available");
        assert_eq!(preserved.position, Some(1.0));
        assert_eq!(session.client_ignoring_on_the_fly(), 1);

        let inbound_with_ack = StatePayload::new()
            .with_ignoring_on_the_fly(IgnoringOnTheFlyPayload::new().with_client(1))
            .with_playstate(
                PlaystatePayload::new()
                    .with_position(99.0)
                    .with_paused(true)
                    .with_do_seek(true)
                    .with_set_by("bob"),
            );
        let outbound_after_ack =
            session.reconcile_state_and_build_response(inbound_with_ack, 99.0, true, 125.0, 0.14);
        assert!(
            outbound_after_ack.playstate.is_some(),
            "outbound playstate should resume once ack clears client ignore"
        );
        assert_eq!(session.client_ignoring_on_the_fly(), 0);
        let updated = session
            .current_room_playstate()
            .expect("room playstate should be updated after ack");
        assert_eq!(updated.position, Some(99.0));
        assert_eq!(updated.paused, Some(true));
        assert_eq!(updated.do_seek, Some(true));
        assert_eq!(updated.set_by.as_deref(), Some("bob"));
    }

    #[test]
    fn reconcile_state_echoes_server_ignore_and_clears_server_counter_after_emit() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":true,"doSeek":false,"setBy":"alice"}}}"#,
            )
            .expect("initial state should apply");

        let inbound = StatePayload::new()
            .with_ignoring_on_the_fly(IgnoringOnTheFlyPayload::new().with_server(3));
        let outbound = session.reconcile_state_and_build_response(inbound, 0.0, true, 200.0, 0.2);

        let ignore = outbound
            .ignoring_on_the_fly
            .as_ref()
            .expect("outbound should include ignoringOnTheFly");
        assert_eq!(ignore.server, Some(3));
        assert_eq!(session.server_ignoring_on_the_fly(), 0);
    }

    #[test]
    fn desync_correction_rewinds_when_client_is_ahead_beyond_threshold() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("state should apply");

        let action = session.evaluate_desync_correction(0.0, 5.0, false, false, true);
        assert_eq!(
            action,
            DesyncCorrectionAction::Rewind {
                target_position: 0.0,
                set_by: Some("bob".to_owned())
            }
        );
    }

    #[test]
    fn desync_correction_slowdown_then_restore_speed_when_delta_recovers() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("state should apply");

        let slowdown = session.evaluate_desync_correction(0.0, 2.0, true, false, true);
        assert_eq!(
            slowdown,
            DesyncCorrectionAction::SlowDown {
                rate: 0.95,
                set_by: Some("bob".to_owned())
            }
        );

        let restore = session.evaluate_desync_correction(1.0, 0.05, true, false, true);
        assert_eq!(restore, DesyncCorrectionAction::RestoreSpeed { rate: 1.0 });
    }

    #[test]
    fn desync_correction_fastforward_requires_sustained_behind_window() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("state should apply");

        let initial = session.evaluate_desync_correction(0.0, 0.0, false, false, true);
        assert_eq!(initial, DesyncCorrectionAction::None);

        let fastforward = session.evaluate_desync_correction(4.0, 0.0, false, false, true);
        assert_eq!(
            fastforward,
            DesyncCorrectionAction::FastForward {
                target_position: 10.25,
                set_by: Some("bob".to_owned())
            }
        );
    }

    #[test]
    fn desync_correction_skips_actions_when_set_by_matches_local_user() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
            )
            .expect("state should apply");

        let action = session.evaluate_desync_correction(0.0, 6.0, false, false, true);
        assert_eq!(action, DesyncCorrectionAction::None);
    }

    #[test]
    fn desync_correction_ignores_threshold_actions_on_do_seek_messages() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":true,"setBy":"bob"}}}"#,
            )
            .expect("state should apply");

        let action = session.evaluate_desync_correction(0.0, 6.0, false, false, true);
        assert_eq!(action, DesyncCorrectionAction::None);
    }

    #[test]
    fn runtime_actions_for_desync_correction_maps_rewind_to_set_position() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("state should apply");

        let actions = session.runtime_actions_for_desync_correction(0.0, 6.0, false, false, true);
        assert_eq!(actions, vec![ClientRuntimeAction::SetPosition(0.0)]);
    }

    #[test]
    fn runtime_actions_for_desync_correction_maps_slowdown_to_rate_change() {
        let mut session = desync_session_with_remote_state(0.0, false, false, "bob");

        let actions = session.runtime_actions_for_desync_correction(0.0, 2.0, true, false, true);
        assert_eq!(actions, vec![ClientRuntimeAction::SetPlaybackRate(0.95)]);
    }

    #[test]
    fn runtime_actions_for_desync_correction_scenario_fastforward_window_reset_and_retrigger() {
        let mut session = desync_session_with_remote_state(10.0, false, false, "bob");
        let steps = vec![
            DesyncRuntimeScenarioStep {
                now_seconds: 0.0,
                local_position: 0.0,
                local_can_control: false,
                dont_slow_down_with_me: false,
                speed_supported: true,
                expected_actions: Vec::new(),
            },
            DesyncRuntimeScenarioStep {
                now_seconds: 4.0,
                local_position: 0.0,
                local_can_control: false,
                dont_slow_down_with_me: false,
                speed_supported: true,
                expected_actions: vec![ClientRuntimeAction::SetPosition(10.25)],
            },
            DesyncRuntimeScenarioStep {
                now_seconds: 5.0,
                local_position: 0.0,
                local_can_control: false,
                dont_slow_down_with_me: false,
                speed_supported: true,
                expected_actions: Vec::new(),
            },
            DesyncRuntimeScenarioStep {
                now_seconds: 11.0,
                local_position: 0.0,
                local_can_control: false,
                dont_slow_down_with_me: false,
                speed_supported: true,
                expected_actions: vec![ClientRuntimeAction::SetPosition(10.25)],
            },
        ];

        run_desync_runtime_scenario(&mut session, &steps);
    }

    #[test]
    fn runtime_actions_for_desync_correction_scenario_slowdown_restore_then_rewind() {
        let mut session = desync_session_with_remote_state(0.0, false, false, "bob");
        let steps = vec![
            DesyncRuntimeScenarioStep {
                now_seconds: 0.0,
                local_position: 2.0,
                local_can_control: true,
                dont_slow_down_with_me: false,
                speed_supported: true,
                expected_actions: vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
            },
            DesyncRuntimeScenarioStep {
                now_seconds: 0.5,
                local_position: 0.05,
                local_can_control: true,
                dont_slow_down_with_me: false,
                speed_supported: true,
                expected_actions: vec![ClientRuntimeAction::SetPlaybackRate(1.0)],
            },
            DesyncRuntimeScenarioStep {
                now_seconds: 1.0,
                local_position: 4.5,
                local_can_control: true,
                dont_slow_down_with_me: false,
                speed_supported: true,
                expected_actions: vec![ClientRuntimeAction::SetPosition(0.0)],
            },
            DesyncRuntimeScenarioStep {
                now_seconds: 1.5,
                local_position: 0.0,
                local_can_control: true,
                dont_slow_down_with_me: false,
                speed_supported: true,
                expected_actions: Vec::new(),
            },
        ];

        run_desync_runtime_scenario(&mut session, &steps);
    }

    #[test]
    fn runtime_actions_for_desync_correction_do_seek_resets_fastforward_detection_window() {
        let mut session = desync_session_with_remote_state(10.0, false, false, "bob");

        let step1 = session.runtime_actions_for_desync_correction(0.0, 0.0, false, false, true);
        assert_eq!(
            step1,
            Vec::<ClientRuntimeAction>::new(),
            "initial behind detection should only start the fastforward timer"
        );

        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":true,"setBy":"bob"}}}"#,
            )
            .expect("doSeek state update should apply");
        let step2 = session.runtime_actions_for_desync_correction(4.0, 0.0, false, false, true);
        assert_eq!(
            step2,
            Vec::<ClientRuntimeAction>::new(),
            "doSeek updates should suppress desync correction and reset behind detection timing"
        );

        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-doSeek state update should apply");
        let step3 = session.runtime_actions_for_desync_correction(4.1, 0.0, false, false, true);
        assert_eq!(
            step3,
            Vec::<ClientRuntimeAction>::new(),
            "after doSeek clears, fastforward detection window should restart from this point"
        );

        let step4 = session.runtime_actions_for_desync_correction(7.3, 0.0, false, false, true);
        assert_eq!(
            step4,
            Vec::<ClientRuntimeAction>::new(),
            "restarted fastforward window should not trigger before the threshold duration elapses again"
        );

        let step5 = session.runtime_actions_for_desync_correction(7.5, 0.0, false, false, true);
        assert_eq!(
            step5,
            vec![ClientRuntimeAction::SetPosition(10.25)],
            "fastforward should retrigger only after the post-doSeek detection window fully elapses"
        );
    }

    #[test]
    fn reset_sync_state_for_reconnect_clears_sync_runtime_state() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
            )
            .expect("state should apply");
        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("remote state should apply");

        let outbound =
            session.reconcile_state_and_build_response(StatePayload::new(), 0.0, true, 300.0, 0.3);
        assert!(
            outbound
                .ignoring_on_the_fly
                .as_ref()
                .is_some_and(|ignore| ignore.client.is_some()),
            "reconcile call should populate client ignore counter for changed local state"
        );
        assert_eq!(session.client_ignoring_on_the_fly(), 1);

        let behind_initial = session.evaluate_desync_correction(0.0, 0.0, false, false, true);
        assert_eq!(behind_initial, DesyncCorrectionAction::None);

        session
            .apply_message_json_at(
                r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#,
                10.0,
            )
            .expect("local playlist index should apply");
        assert!(session.recently_advanced(11.0));

        session.reset_sync_state_for_reconnect();

        assert_eq!(session.client_ignoring_on_the_fly(), 0);
        assert_eq!(session.server_ignoring_on_the_fly(), 0);
        assert!(session.current_room_playstate().is_none());
        let post_reset = session.evaluate_desync_correction(4.0, 0.0, false, false, true);
        assert_eq!(post_reset, DesyncCorrectionAction::None);
        assert_eq!(session.username.as_deref(), Some("alice"));
        assert_eq!(session.room.as_deref(), Some("room1"));
        assert!(!session.recently_advanced(11.0));
    }

    #[test]
    fn reset_sync_state_for_reconnect_resets_desync_transient_state_before_post_reconnect_evaluation()
     {
        let mut session = desync_session_with_remote_state(10.0, false, false, "bob");

        let pre_reset_behind_detection =
            session.runtime_actions_for_desync_correction(0.0, 0.0, false, false, true);
        assert_eq!(
            pre_reset_behind_detection,
            Vec::<ClientRuntimeAction>::new(),
            "initial behind detection should only start the fastforward timer pre-reconnect"
        );
        assert_eq!(
            session.behind_first_detected_at_seconds,
            Some(0.0),
            "precondition: reconnect reset test should prime fastforward detection timer"
        );

        session.reset_sync_state_for_reconnect();
        assert_eq!(
            session.behind_first_detected_at_seconds, None,
            "reconnect reset should clear fastforward detection timer state"
        );
        assert!(
            !session.speed_changed,
            "reconnect reset should clear slowdown state"
        );

        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-reconnect remote state should apply");
        let post_reconnect_behind_detection =
            session.runtime_actions_for_desync_correction(4.0, 0.0, false, false, true);
        assert_eq!(
            post_reconnect_behind_detection,
            Vec::<ClientRuntimeAction>::new(),
            "post-reconnect behind detection should restart fresh instead of using stale pre-reconnect timer state"
        );
        assert_eq!(
            session.behind_first_detected_at_seconds,
            Some(4.0),
            "post-reconnect fastforward timer should start from the new evaluation time"
        );

        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-reconnect ahead-state update should apply");
        let post_reconnect_slowdown =
            session.runtime_actions_for_desync_correction(5.0, 2.0, true, false, true);
        assert_eq!(
            post_reconnect_slowdown,
            vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
            "post-reconnect desync evaluation should be able to re-enter slowdown from a cleared state"
        );
        assert!(
            session.speed_changed,
            "slowdown action should set speed_changed again after reconnect reset"
        );

        session.reset_sync_state_for_reconnect();
        assert_eq!(
            session.behind_first_detected_at_seconds, None,
            "second reconnect reset should clear any restarted fastforward timer state"
        );
        assert!(
            !session.speed_changed,
            "second reconnect reset should clear the re-primed slowdown state"
        );

        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-second-reconnect ahead-state update should apply");
        let second_post_reconnect_slowdown =
            session.runtime_actions_for_desync_correction(6.0, 2.0, true, false, true);
        assert_eq!(
            second_post_reconnect_slowdown,
            vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
            "cleared slowdown state should not suppress the first post-reconnect slowdown action"
        );
    }

    #[test]
    fn reset_sync_state_for_reconnect_prevents_stale_desync_speed_restore_after_pre_reconnect_slowdown()
     {
        let mut session = desync_session_with_remote_state(0.0, false, false, "bob");

        let pre_reconnect_slowdown =
            session.runtime_actions_for_desync_correction(0.0, 2.0, true, false, true);
        assert_eq!(
            pre_reconnect_slowdown,
            vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
            "precondition: pre-reconnect desync evaluation should trigger slowdown"
        );
        assert!(
            session.speed_changed,
            "precondition: slowdown should mark speed_changed before reconnect reset"
        );

        session.reset_sync_state_for_reconnect();
        assert!(
            !session.speed_changed,
            "reconnect reset should clear slowdown state so restore-speed is not emitted from stale state"
        );

        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-reconnect synced remote state should apply");
        let post_reconnect_near_sync_actions =
            session.runtime_actions_for_desync_correction(1.0, 0.05, true, false, true);
        assert_eq!(
            post_reconnect_near_sync_actions,
            Vec::<ClientRuntimeAction>::new(),
            "post-reconnect near-sync evaluation should not emit stale restore-speed action if slowdown state was reset"
        );
        assert!(
            !session.speed_changed,
            "near-sync evaluation should keep slowdown state cleared when no slowdown is active post-reconnect"
        );

        let post_reconnect_slowdown =
            session.runtime_actions_for_desync_correction(2.0, 2.0, true, false, true);
        assert_eq!(
            post_reconnect_slowdown,
            vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
            "post-reconnect slowdown should still trigger normally from a fresh state"
        );
    }

    #[test]
    fn reset_sync_state_for_reconnect_prevents_stale_fastforward_after_pre_reconnect_behind_detection_and_post_reconnect_do_seek_transition()
     {
        let mut session = desync_session_with_remote_state(10.0, false, false, "bob");

        let pre_reconnect_behind_detection =
            session.runtime_actions_for_desync_correction(0.0, 0.0, false, false, true);
        assert_eq!(
            pre_reconnect_behind_detection,
            Vec::<ClientRuntimeAction>::new(),
            "precondition: pre-reconnect behind detection should only start fastforward timer"
        );
        assert_eq!(
            session.behind_first_detected_at_seconds,
            Some(0.0),
            "precondition: pre-reconnect behind timer should be primed before reconnect reset"
        );

        session.reset_sync_state_for_reconnect();
        assert_eq!(
            session.behind_first_detected_at_seconds, None,
            "reconnect reset should clear any pre-reconnect fastforward detection timer state"
        );

        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":true,"setBy":"carol"}}}"#,
            )
            .expect("post-reconnect doSeek state should apply");
        let do_seek_suppressed =
            session.runtime_actions_for_desync_correction(4.0, 0.0, false, false, true);
        assert_eq!(
            do_seek_suppressed,
            Vec::<ClientRuntimeAction>::new(),
            "post-reconnect doSeek state should suppress desync correction"
        );
        assert_eq!(
            session.behind_first_detected_at_seconds, None,
            "doSeek suppression after reconnect should keep fastforward timer cleared"
        );

        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"dave"}}}"#,
            )
            .expect("post-reconnect doSeek-clear state should apply");
        let restarted_after_do_seek_clear =
            session.runtime_actions_for_desync_correction(4.1, 0.0, false, false, true);
        assert_eq!(
            restarted_after_do_seek_clear,
            Vec::<ClientRuntimeAction>::new(),
            "after reconnect + doSeek clears, fastforward detection should restart fresh instead of using stale pre-reconnect timing"
        );
        assert_eq!(
            session.behind_first_detected_at_seconds,
            Some(4.1),
            "post-reconnect fastforward timer should restart from doSeek-clear evaluation time"
        );

        let before_threshold =
            session.runtime_actions_for_desync_correction(7.3, 0.0, false, false, true);
        assert_eq!(
            before_threshold,
            Vec::<ClientRuntimeAction>::new(),
            "restarted post-reconnect fastforward window should not trigger before threshold elapses"
        );

        let after_threshold =
            session.runtime_actions_for_desync_correction(7.5, 0.0, false, false, true);
        assert_eq!(
            after_threshold,
            vec![ClientRuntimeAction::SetPosition(10.25)],
            "fastforward should trigger only after the restarted post-reconnect window elapses"
        );
    }

    #[test]
    fn reset_sync_state_for_reconnect_prevents_stale_speed_restore_when_post_reconnect_state_resumes_paused_then_unpauses()
     {
        let mut session = desync_session_with_remote_state(0.0, false, false, "bob");

        let pre_reconnect_slowdown =
            session.runtime_actions_for_desync_correction(0.0, 2.0, true, false, true);
        assert_eq!(
            pre_reconnect_slowdown,
            vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
            "precondition: pre-reconnect desync evaluation should trigger slowdown"
        );
        assert!(
            session.speed_changed,
            "precondition: slowdown should mark speed_changed before reconnect reset"
        );

        session.reset_sync_state_for_reconnect();
        assert!(
            !session.speed_changed,
            "reconnect reset should clear slowdown state before post-reconnect paused/unpaused evaluations"
        );

        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":true,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-reconnect paused remote state should apply");

        let paused_near_sync =
            session.runtime_actions_for_desync_correction(1.0, 0.05, true, false, true);
        assert_eq!(
            paused_near_sync,
            Vec::<ClientRuntimeAction>::new(),
            "paused post-reconnect near-sync evaluation should not emit stale restore-speed action"
        );
        assert!(
            !session.speed_changed,
            "paused post-reconnect near-sync evaluation should keep slowdown state cleared"
        );

        let paused_ahead =
            session.runtime_actions_for_desync_correction(1.5, 2.0, true, false, true);
        assert_eq!(
            paused_ahead,
            Vec::<ClientRuntimeAction>::new(),
            "paused post-reconnect desync evaluation should not emit slowdown while room is paused"
        );
        assert!(
            !session.speed_changed,
            "paused post-reconnect desync evaluation should not re-prime slowdown state"
        );

        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-reconnect unpaused remote state should apply");

        let unpaused_near_sync =
            session.runtime_actions_for_desync_correction(2.0, 0.05, true, false, true);
        assert_eq!(
            unpaused_near_sync,
            Vec::<ClientRuntimeAction>::new(),
            "unpaused post-reconnect near-sync evaluation should still not emit stale restore-speed action"
        );
        assert!(
            !session.speed_changed,
            "unpaused near-sync evaluation should keep slowdown state cleared until a real slowdown trigger"
        );

        let unpaused_ahead =
            session.runtime_actions_for_desync_correction(3.0, 2.0, true, false, true);
        assert_eq!(
            unpaused_ahead,
            vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
            "after unpause, post-reconnect desync slowdown should trigger normally from a fresh state"
        );
    }

    #[test]
    fn reset_sync_state_for_reconnect_clears_self_setby_fastforward_suppression_window_before_post_reconnect_desync_evaluation()
     {
        let mut session = desync_session_with_remote_state(10.0, false, false, "alice");

        let pre_reconnect_timer_start =
            session.runtime_actions_for_desync_correction(0.0, 0.0, false, false, true);
        assert_eq!(
            pre_reconnect_timer_start,
            Vec::<ClientRuntimeAction>::new(),
            "precondition: initial behind detection should only start fastforward timer"
        );
        assert_eq!(
            session.behind_first_detected_at_seconds,
            Some(0.0),
            "precondition: behind timer should start at first detection time"
        );

        let pre_reconnect_self_setby_suppressed =
            session.runtime_actions_for_desync_correction(4.0, 0.0, false, false, true);
        assert_eq!(
            pre_reconnect_self_setby_suppressed,
            Vec::<ClientRuntimeAction>::new(),
            "self-attributed fastforward candidate should be suppressed before reconnect"
        );
        assert!(
            session
                .behind_first_detected_at_seconds
                .is_some_and(|t| t > 4.0),
            "self-attributed fastforward suppression should leave a future suppression-window timer"
        );

        session.reset_sync_state_for_reconnect();
        assert_eq!(
            session.behind_first_detected_at_seconds, None,
            "reconnect reset should clear stale self-setby fastforward suppression window"
        );

        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-reconnect remote state should apply");

        let post_reconnect_timer_restart =
            session.runtime_actions_for_desync_correction(4.1, 0.0, false, false, true);
        assert_eq!(
            post_reconnect_timer_restart,
            Vec::<ClientRuntimeAction>::new(),
            "post-reconnect behind detection should restart instead of inheriting stale self-setby suppression window"
        );
        assert_eq!(
            session.behind_first_detected_at_seconds,
            Some(4.1),
            "post-reconnect behind timer should restart from new detection time"
        );

        let post_reconnect_before_threshold =
            session.runtime_actions_for_desync_correction(7.3, 0.0, false, false, true);
        assert_eq!(
            post_reconnect_before_threshold,
            Vec::<ClientRuntimeAction>::new(),
            "restarted post-reconnect fastforward window should not trigger before threshold elapses"
        );

        let post_reconnect_after_threshold =
            session.runtime_actions_for_desync_correction(7.5, 0.0, false, false, true);
        assert_eq!(
            post_reconnect_after_threshold,
            vec![ClientRuntimeAction::SetPosition(10.25)],
            "post-reconnect fastforward should trigger only after restarted window elapses against non-self setBy"
        );
    }

    #[test]
    fn reset_sync_state_for_reconnect_clears_fastforward_action_cooldown_window_before_post_reconnect_desync_evaluation()
     {
        let mut session = desync_session_with_remote_state(10.0, false, false, "bob");

        let pre_reconnect_timer_start =
            session.runtime_actions_for_desync_correction(0.0, 0.0, false, false, true);
        assert_eq!(
            pre_reconnect_timer_start,
            Vec::<ClientRuntimeAction>::new(),
            "precondition: initial behind detection should only start fastforward timer"
        );
        assert_eq!(
            session.behind_first_detected_at_seconds,
            Some(0.0),
            "precondition: behind timer should start at first detection time"
        );

        let pre_reconnect_fastforward =
            session.runtime_actions_for_desync_correction(4.0, 0.0, false, false, true);
        assert_eq!(
            pre_reconnect_fastforward,
            vec![ClientRuntimeAction::SetPosition(10.25)],
            "precondition: non-self fastforward should trigger before reconnect"
        );
        assert!(
            session
                .behind_first_detected_at_seconds
                .is_some_and(|t| t > 4.0),
            "fastforward action should leave a future cooldown/suppression timer before reconnect"
        );

        session.reset_sync_state_for_reconnect();
        assert_eq!(
            session.behind_first_detected_at_seconds, None,
            "reconnect reset should clear stale fastforward action cooldown window"
        );

        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-reconnect remote state should apply");

        let post_reconnect_timer_restart =
            session.runtime_actions_for_desync_correction(4.1, 0.0, false, false, true);
        assert_eq!(
            post_reconnect_timer_restart,
            Vec::<ClientRuntimeAction>::new(),
            "post-reconnect behind detection should restart instead of inheriting stale fastforward cooldown window"
        );
        assert_eq!(
            session.behind_first_detected_at_seconds,
            Some(4.1),
            "post-reconnect behind timer should restart from new detection time"
        );

        let post_reconnect_before_threshold =
            session.runtime_actions_for_desync_correction(7.3, 0.0, false, false, true);
        assert_eq!(
            post_reconnect_before_threshold,
            Vec::<ClientRuntimeAction>::new(),
            "restarted post-reconnect fastforward window should not trigger before threshold elapses"
        );

        let post_reconnect_after_threshold =
            session.runtime_actions_for_desync_correction(7.5, 0.0, false, false, true);
        assert_eq!(
            post_reconnect_after_threshold,
            vec![ClientRuntimeAction::SetPosition(10.25)],
            "post-reconnect fastforward should trigger only after restarted window elapses"
        );
    }

    #[test]
    fn reset_sync_state_for_reconnect_preserves_rewind_suppression_ordering_across_self_setby_and_post_reconnect_do_seek_transition()
     {
        let mut session = desync_session_with_remote_state(0.0, false, false, "alice");

        let pre_reconnect_self_setby_rewind_suppressed =
            session.runtime_actions_for_desync_correction(0.0, 6.0, false, false, true);
        assert_eq!(
            pre_reconnect_self_setby_rewind_suppressed,
            Vec::<ClientRuntimeAction>::new(),
            "pre-reconnect self-attributed rewind candidate should be suppressed"
        );
        assert_eq!(
            session.behind_first_detected_at_seconds, None,
            "rewind/self-setBy suppression path should not leave a behind-detection timer"
        );
        assert!(
            !session.speed_changed,
            "rewind/self-setBy suppression path should not touch slowdown state"
        );

        session.reset_sync_state_for_reconnect();
        assert_eq!(
            session.behind_first_detected_at_seconds, None,
            "reconnect reset should keep rewind-related fastforward timer state cleared"
        );
        assert!(
            !session.speed_changed,
            "reconnect reset should keep slowdown state cleared before post-reconnect rewind evaluations"
        );

        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":true,"setBy":"bob"}}}"#,
            )
            .expect("post-reconnect doSeek state should apply");
        let post_reconnect_do_seek_rewind_suppressed =
            session.runtime_actions_for_desync_correction(1.0, 6.0, false, false, true);
        assert_eq!(
            post_reconnect_do_seek_rewind_suppressed,
            Vec::<ClientRuntimeAction>::new(),
            "post-reconnect doSeek state should suppress rewind correction before doSeek clears"
        );

        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-reconnect doSeek-clear state should apply");
        let post_reconnect_remote_rewind =
            session.runtime_actions_for_desync_correction(1.1, 6.0, false, false, true);
        assert_eq!(
            post_reconnect_remote_rewind,
            vec![ClientRuntimeAction::SetPosition(0.0)],
            "once post-reconnect doSeek clears and setBy is remote, rewind should trigger immediately"
        );
    }

    #[test]
    fn reset_sync_state_for_reconnect_prevents_stale_speed_restore_when_post_reconnect_rewind_precedes_near_sync()
     {
        let mut session = desync_session_with_remote_state(0.0, false, false, "bob");

        let pre_reconnect_slowdown =
            session.runtime_actions_for_desync_correction(0.0, 2.0, true, false, true);
        assert_eq!(
            pre_reconnect_slowdown,
            vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
            "precondition: pre-reconnect ahead-state should trigger slowdown"
        );
        assert!(
            session.speed_changed,
            "precondition: slowdown should prime speed_changed before reconnect reset"
        );

        session.reset_sync_state_for_reconnect();
        assert!(
            !session.speed_changed,
            "reconnect reset should clear slowdown state before post-reconnect rewind path"
        );

        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-reconnect remote state should apply");

        let post_reconnect_rewind =
            session.runtime_actions_for_desync_correction(1.0, 6.0, false, false, true);
        assert_eq!(
            post_reconnect_rewind,
            vec![ClientRuntimeAction::SetPosition(0.0)],
            "post-reconnect rewind should still trigger immediately on large ahead desync"
        );
        assert!(
            !session.speed_changed,
            "rewind branch should not resurrect stale slowdown state after reconnect reset"
        );

        let post_reconnect_near_sync =
            session.runtime_actions_for_desync_correction(1.1, 0.05, true, false, true);
        assert_eq!(
            post_reconnect_near_sync,
            Vec::<ClientRuntimeAction>::new(),
            "near-sync after post-reconnect rewind should not emit stale restore-speed action"
        );
        assert!(
            !session.speed_changed,
            "near-sync after rewind should keep slowdown state cleared when no slowdown is active"
        );

        let post_reconnect_slowdown =
            session.runtime_actions_for_desync_correction(2.0, 2.0, true, false, true);
        assert_eq!(
            post_reconnect_slowdown,
            vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
            "post-reconnect slowdown should still trigger normally after rewind/near-sync from a fresh state"
        );
    }

    #[test]
    fn reset_sync_state_for_reconnect_prevents_stale_speed_restore_when_post_reconnect_self_setby_rewind_is_suppressed_before_near_sync()
     {
        let mut session = desync_session_with_remote_state(0.0, false, false, "bob");

        let pre_reconnect_slowdown =
            session.runtime_actions_for_desync_correction(0.0, 2.0, true, false, true);
        assert_eq!(
            pre_reconnect_slowdown,
            vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
            "precondition: pre-reconnect ahead-state should trigger slowdown"
        );
        assert!(
            session.speed_changed,
            "precondition: slowdown should prime speed_changed before reconnect reset"
        );

        session.reset_sync_state_for_reconnect();
        assert!(
            !session.speed_changed,
            "reconnect reset should clear slowdown state before post-reconnect self-setBy rewind suppression"
        );

        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
            )
            .expect("post-reconnect self-setBy remote state should apply");

        let post_reconnect_self_setby_rewind_suppressed =
            session.runtime_actions_for_desync_correction(1.0, 6.0, false, false, true);
        assert_eq!(
            post_reconnect_self_setby_rewind_suppressed,
            Vec::<ClientRuntimeAction>::new(),
            "post-reconnect self-attributed rewind candidate should remain suppressed"
        );
        assert_eq!(
            session.behind_first_detected_at_seconds, None,
            "self-setBy rewind suppression should not prime fastforward timer state"
        );
        assert!(
            !session.speed_changed,
            "self-setBy rewind suppression should not resurrect stale slowdown state"
        );

        let post_reconnect_near_sync =
            session.runtime_actions_for_desync_correction(1.1, 0.05, true, false, true);
        assert_eq!(
            post_reconnect_near_sync,
            Vec::<ClientRuntimeAction>::new(),
            "near-sync after self-setBy rewind suppression should not emit stale restore-speed action"
        );
        assert!(
            !session.speed_changed,
            "near-sync after self-setBy rewind suppression should keep slowdown state cleared"
        );

        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-reconnect non-self remote state should apply");
        let post_reconnect_remote_slowdown =
            session.runtime_actions_for_desync_correction(2.0, 2.0, true, false, true);
        assert_eq!(
            post_reconnect_remote_slowdown,
            vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
            "post-reconnect slowdown should still trigger normally after self-setBy rewind suppression and near-sync from a fresh state"
        );
    }

    #[test]
    fn reset_sync_state_for_reconnect_prevents_stale_speed_restore_across_post_reconnect_do_seek_paused_and_self_setby_rewind_suppression_branches()
     {
        let mut session = desync_session_with_remote_state(0.0, false, false, "bob");

        let pre_reconnect_slowdown =
            session.runtime_actions_for_desync_correction(0.0, 2.0, true, false, true);
        assert_eq!(
            pre_reconnect_slowdown,
            vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
            "precondition: pre-reconnect ahead-state should trigger slowdown"
        );
        assert!(
            session.speed_changed,
            "precondition: slowdown should prime speed_changed before reconnect reset"
        );

        session.reset_sync_state_for_reconnect();
        assert!(
            !session.speed_changed,
            "reconnect reset should clear slowdown state before post-reconnect branch sequence"
        );

        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":true,"doSeek":true,"setBy":"alice"}}}"#,
            )
            .expect("post-reconnect paused doSeek self-setBy state should apply");
        let post_reconnect_do_seek_suppressed =
            session.runtime_actions_for_desync_correction(1.0, 6.0, false, false, true);
        assert_eq!(
            post_reconnect_do_seek_suppressed,
            Vec::<ClientRuntimeAction>::new(),
            "post-reconnect doSeek state should suppress desync correction before other branches"
        );
        assert_eq!(
            session.behind_first_detected_at_seconds, None,
            "doSeek suppression should keep fastforward timer state cleared"
        );
        assert!(
            !session.speed_changed,
            "doSeek suppression should not resurrect stale slowdown state"
        );

        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":true,"doSeek":false,"setBy":"alice"}}}"#,
            )
            .expect("post-reconnect paused self-setBy state should apply");
        let post_reconnect_paused_self_setby_rewind_suppressed =
            session.runtime_actions_for_desync_correction(1.1, 6.0, false, false, true);
        assert_eq!(
            post_reconnect_paused_self_setby_rewind_suppressed,
            Vec::<ClientRuntimeAction>::new(),
            "paused self-attributed rewind candidate should remain suppressed after reconnect"
        );
        assert_eq!(
            session.behind_first_detected_at_seconds, None,
            "rewind/self-setBy suppression path should not prime fastforward timer state"
        );
        assert!(
            !session.speed_changed,
            "paused self-setBy rewind suppression should not resurrect stale slowdown state"
        );

        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
            )
            .expect("post-reconnect unpaused self-setBy state should apply");
        let post_reconnect_near_sync =
            session.runtime_actions_for_desync_correction(1.2, 0.05, true, false, true);
        assert_eq!(
            post_reconnect_near_sync,
            Vec::<ClientRuntimeAction>::new(),
            "near-sync after doSeek+paused+self-setBy suppression sequence should not emit stale restore-speed action"
        );
        assert!(
            !session.speed_changed,
            "near-sync after branch sequence should keep slowdown state cleared"
        );

        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-reconnect non-self state should apply");
        let post_reconnect_remote_slowdown =
            session.runtime_actions_for_desync_correction(2.0, 2.0, true, false, true);
        assert_eq!(
            post_reconnect_remote_slowdown,
            vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
            "post-reconnect slowdown should still trigger normally after branch sequence from a fresh state"
        );
    }

    #[test]
    fn reset_sync_state_for_reconnect_prevents_stale_speed_restore_when_post_reconnect_self_setby_slowdown_is_suppressed_before_near_sync()
     {
        let mut session = desync_session_with_remote_state(0.0, false, false, "bob");

        let pre_reconnect_slowdown =
            session.runtime_actions_for_desync_correction(0.0, 2.0, true, false, true);
        assert_eq!(
            pre_reconnect_slowdown,
            vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
            "precondition: pre-reconnect ahead-state should trigger slowdown"
        );
        assert!(
            session.speed_changed,
            "precondition: slowdown should prime speed_changed before reconnect reset"
        );

        session.reset_sync_state_for_reconnect();
        assert!(
            !session.speed_changed,
            "reconnect reset should clear slowdown state before post-reconnect self-setBy slowdown suppression"
        );

        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
            )
            .expect("post-reconnect self-setBy state should apply");

        let post_reconnect_self_setby_slowdown_suppressed =
            session.runtime_actions_for_desync_correction(1.0, 2.0, true, false, true);
        assert_eq!(
            post_reconnect_self_setby_slowdown_suppressed,
            Vec::<ClientRuntimeAction>::new(),
            "post-reconnect self-attributed slowdown candidate should remain suppressed"
        );
        assert!(
            !session.speed_changed,
            "self-setBy slowdown suppression should not resurrect stale slowdown state"
        );

        let post_reconnect_near_sync =
            session.runtime_actions_for_desync_correction(1.1, 0.05, true, false, true);
        assert_eq!(
            post_reconnect_near_sync,
            Vec::<ClientRuntimeAction>::new(),
            "near-sync after self-setBy slowdown suppression should not emit stale restore-speed action"
        );
        assert!(
            !session.speed_changed,
            "near-sync after self-setBy slowdown suppression should keep slowdown state cleared"
        );

        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-reconnect non-self state should apply");
        let post_reconnect_remote_slowdown =
            session.runtime_actions_for_desync_correction(2.0, 2.0, true, false, true);
        assert_eq!(
            post_reconnect_remote_slowdown,
            vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
            "post-reconnect slowdown should still trigger normally after self-setBy slowdown suppression and near-sync from a fresh state"
        );
    }

    #[test]
    fn reset_sync_state_for_reconnect_prevents_stale_speed_restore_across_post_reconnect_do_seek_paused_and_self_setby_slowdown_suppression_branches()
     {
        let mut session = desync_session_with_remote_state(0.0, false, false, "bob");

        let pre_reconnect_slowdown =
            session.runtime_actions_for_desync_correction(0.0, 2.0, true, false, true);
        assert_eq!(
            pre_reconnect_slowdown,
            vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
            "precondition: pre-reconnect ahead-state should trigger slowdown"
        );
        assert!(
            session.speed_changed,
            "precondition: slowdown should prime speed_changed before reconnect reset"
        );

        session.reset_sync_state_for_reconnect();
        assert!(
            !session.speed_changed,
            "reconnect reset should clear slowdown state before post-reconnect branch sequence"
        );

        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":true,"setBy":"alice"}}}"#,
            )
            .expect("post-reconnect doSeek self-setBy state should apply");
        let post_reconnect_do_seek_suppressed =
            session.runtime_actions_for_desync_correction(1.0, 2.0, true, false, true);
        assert_eq!(
            post_reconnect_do_seek_suppressed,
            Vec::<ClientRuntimeAction>::new(),
            "post-reconnect doSeek state should suppress slowdown evaluation before other branches"
        );
        assert!(
            !session.speed_changed,
            "doSeek suppression should not resurrect stale slowdown state"
        );

        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":true,"doSeek":false,"setBy":"alice"}}}"#,
            )
            .expect("post-reconnect paused self-setBy state should apply");
        let post_reconnect_paused_slowdown_suppressed =
            session.runtime_actions_for_desync_correction(1.1, 2.0, true, false, true);
        assert_eq!(
            post_reconnect_paused_slowdown_suppressed,
            Vec::<ClientRuntimeAction>::new(),
            "paused post-reconnect state should suppress slowdown before self-setBy slowdown branch"
        );
        assert!(
            !session.speed_changed,
            "paused slowdown suppression should keep slowdown state cleared"
        );

        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
            )
            .expect("post-reconnect unpaused self-setBy state should apply");
        let post_reconnect_self_setby_slowdown_suppressed =
            session.runtime_actions_for_desync_correction(1.2, 2.0, true, false, true);
        assert_eq!(
            post_reconnect_self_setby_slowdown_suppressed,
            Vec::<ClientRuntimeAction>::new(),
            "post-reconnect self-attributed slowdown candidate should remain suppressed"
        );
        assert!(
            !session.speed_changed,
            "self-setBy slowdown suppression should not resurrect stale slowdown state"
        );

        let post_reconnect_near_sync =
            session.runtime_actions_for_desync_correction(1.3, 0.05, true, false, true);
        assert_eq!(
            post_reconnect_near_sync,
            Vec::<ClientRuntimeAction>::new(),
            "near-sync after doSeek+paused+self-setBy slowdown-suppression sequence should not emit stale restore-speed action"
        );
        assert!(
            !session.speed_changed,
            "near-sync after branch sequence should keep slowdown state cleared"
        );

        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":0.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("post-reconnect non-self state should apply");
        let post_reconnect_remote_slowdown =
            session.runtime_actions_for_desync_correction(2.0, 2.0, true, false, true);
        assert_eq!(
            post_reconnect_remote_slowdown,
            vec![ClientRuntimeAction::SetPlaybackRate(0.95)],
            "post-reconnect slowdown should still trigger normally after branch sequence from a fresh state"
        );
    }

    #[test]
    fn reconnect_retry_policy_uses_legacy_exponential_backoff_with_cap() {
        let mut session = ClientSession::default();

        let retry0 = session.plan_reconnect_retry(0);
        assert!(retry0.should_retry);
        assert_eq!(retry0.delay_seconds, Some(0.1));

        let retry1 = session.plan_reconnect_retry(1);
        assert!(retry1.should_retry);
        assert_eq!(retry1.delay_seconds, Some(0.2));

        let retry5 = session.plan_reconnect_retry(5);
        assert!(retry5.should_retry);
        assert_eq!(retry5.delay_seconds, Some(3.2));

        let retry8 = session.plan_reconnect_retry(8);
        assert!(retry8.should_retry);
        assert_eq!(retry8.delay_seconds, Some(3.2));
    }

    #[test]
    fn reconnect_retry_policy_respects_max_retry_cutoff() {
        let mut session = ClientSession::default();
        session.reconnect_policy_mut().max_retries = 2;

        let retry = session.plan_reconnect_retry(3);
        assert!(!retry.should_retry);
        assert_eq!(retry.delay_seconds, None);
        assert!(retry.should_reset_state);
    }

    #[test]
    fn reconnect_retry_policy_applies_sync_state_reset_before_retry() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("state should apply");

        let outbound =
            session.reconcile_state_and_build_response(StatePayload::new(), 0.0, true, 10.0, 0.1);
        assert!(
            outbound
                .ignoring_on_the_fly
                .as_ref()
                .is_some_and(|ignore| ignore.client == Some(1)),
            "precondition: outbound state should carry client ignore counter"
        );

        let retry = session.plan_reconnect_retry(0);
        assert!(retry.should_retry);
        assert_eq!(retry.delay_seconds, Some(0.1));
        assert!(retry.should_reset_state);
        assert_eq!(session.client_ignoring_on_the_fly(), 0);
        assert_eq!(session.server_ignoring_on_the_fly(), 0);
        assert!(session.current_room_playstate().is_none());
    }

    #[test]
    fn runtime_actions_for_reconnect_retry_schedule_or_stop_as_expected() {
        let mut session = ClientSession::default();

        let retry_actions = session.runtime_actions_for_reconnect_retry(0);
        assert_eq!(
            retry_actions,
            vec![
                ClientRuntimeAction::NotifyReconnectTransition(
                    ReconnectTransitionNotification::Attempting {
                        retries: 0,
                        delay_seconds: 0.1,
                    },
                ),
                ClientRuntimeAction::ScheduleReconnect { delay_seconds: 0.1 },
            ]
        );

        session.reconnect_policy_mut().max_retries = 0;
        let stop_actions = session.runtime_actions_for_reconnect_retry(1);
        assert_eq!(
            stop_actions,
            vec![
                ClientRuntimeAction::NotifyReconnectTransition(
                    ReconnectTransitionNotification::Disconnected,
                ),
                ClientRuntimeAction::StopReconnect,
            ]
        );
    }

    #[test]
    fn reconnect_transition_connected_notification_emits_after_reconnect_hello() {
        let mut session = ClientSession::default();
        session.runtime_actions_for_reconnect_retry(0);
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        assert_eq!(
            session.runtime_actions_for_reconnect_transition_if_needed(),
            vec![ClientRuntimeAction::NotifyReconnectTransition(
                ReconnectTransitionNotification::Connected,
            )]
        );
        assert!(
            session
                .runtime_actions_for_reconnect_transition_if_needed()
                .is_empty(),
            "connected reconnect notification should drain once"
        );
    }

    #[test]
    fn reconnect_state_restore_emits_ready_and_file_actions_after_hello() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
            .expect("local ready should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"file":{"name":"movie.mkv","size":123456789,"duration":95.5}}}}}"#,
            )
            .expect("local file metadata should apply");

        session.reset_sync_state_for_reconnect();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("reconnect hello should apply");

        let restore_actions = session.runtime_actions_for_reconnect_state_restore_if_needed();
        assert_eq!(
            restore_actions,
            vec![
                ClientRuntimeAction::NotifyReconnectTransition(
                    ReconnectTransitionNotification::RestoringState,
                ),
                ClientRuntimeAction::SetReady {
                    ready: true,
                    manually_initiated: false,
                },
                ClientRuntimeAction::SetFile {
                    file_payload: json!({
                        "name": "movie.mkv",
                        "size": 123456789,
                        "duration": 95.5
                    }),
                },
            ]
        );
        assert!(
            session
                .runtime_actions_for_reconnect_state_restore_if_needed()
                .is_empty(),
            "state restore actions should drain after first retrieval"
        );
    }

    #[test]
    fn reconnect_reset_restores_local_controller_flag_after_hello() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"room":{"name":"+room:ABCDEF123456"},"controller":true}}}}"#,
            )
            .expect("controller update should apply");
        assert_eq!(session.local_can_control(), Some(true));

        session.reset_sync_state_for_reconnect();
        session.reset_sync_state_for_reconnect();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("reconnect hello should apply");

        assert_eq!(session.user_controller("alice"), Some(true));
        assert_eq!(session.local_can_control(), Some(true));
    }

    #[test]
    fn repeated_reconnect_resets_preserve_cached_restore_state() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
            .expect("local ready should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"file":{"name":"movie.mkv","size":123456789,"duration":95.5}}}}}"#,
            )
            .expect("local file metadata should apply");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("local playlist should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
            .expect("local playlist index should apply");

        session.reset_sync_state_for_reconnect();
        session.reset_sync_state_for_reconnect();

        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("reconnect hello should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistChange":{"files":[]}}}"#)
            .expect("empty server playlist snapshot should apply");

        let state_restore_actions = session.runtime_actions_for_reconnect_state_restore_if_needed();
        assert_eq!(
            state_restore_actions,
            vec![
                ClientRuntimeAction::NotifyReconnectTransition(
                    ReconnectTransitionNotification::RestoringState,
                ),
                ClientRuntimeAction::SetReady {
                    ready: true,
                    manually_initiated: false,
                },
                ClientRuntimeAction::SetFile {
                    file_payload: json!({
                        "name": "movie.mkv",
                        "size": 123456789,
                        "duration": 95.5
                    }),
                },
            ]
        );

        let playlist_restore_actions =
            session.runtime_actions_for_reconnect_playlist_restore_if_needed();
        assert_eq!(
            playlist_restore_actions,
            vec![
                ClientRuntimeAction::NotifyReconnectTransition(
                    ReconnectTransitionNotification::RestoringPlaylist,
                ),
                ClientRuntimeAction::SetPlaylist {
                    files: vec!["episode1.mkv".to_owned(), "episode2.mkv".to_owned()],
                },
                ClientRuntimeAction::SetPlaylistIndex { index: 1 },
            ]
        );
    }

    #[test]
    fn reconnect_playlist_restore_emits_actions_on_empty_server_playlist_snapshot() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("local playlist should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
            .expect("local playlist index should apply");

        session.reset_sync_state_for_reconnect();
        assert!(
            session.current_room_playlist().is_none(),
            "reconnect reset should clear stale playlist state until server snapshot arrives"
        );

        session
            .apply_message_json(r#"{"Set":{"playlistChange":{"files":[]}}}"#)
            .expect("empty server playlist snapshot should apply");

        let restore_actions = session.runtime_actions_for_reconnect_playlist_restore_if_needed();
        assert_eq!(
            restore_actions,
            vec![
                ClientRuntimeAction::NotifyReconnectTransition(
                    ReconnectTransitionNotification::RestoringPlaylist,
                ),
                ClientRuntimeAction::SetPlaylist {
                    files: vec!["episode1.mkv".to_owned(), "episode2.mkv".to_owned()],
                },
                ClientRuntimeAction::SetPlaylistIndex { index: 1 },
            ]
        );
        assert!(
            session
                .runtime_actions_for_reconnect_playlist_restore_if_needed()
                .is_empty(),
            "playlist restore actions should drain after first retrieval"
        );
    }

    #[test]
    fn reconnect_playlist_restore_ignores_non_matching_playlist_updates() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv"],"user":"alice"}}}"#,
            )
            .expect("local playlist should apply");

        session.reset_sync_state_for_reconnect();
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["server_episode.mkv"],"user":"bob"}}}"#,
            )
            .expect("non-empty server playlist update should apply");
        assert!(
            session
                .runtime_actions_for_reconnect_playlist_restore_if_needed()
                .is_empty(),
            "non-empty server playlist snapshots should suppress reconnect restore"
        );
    }

    #[test]
    fn handle_disconnect_with_pause_on_leave_sets_pause_and_timestamp() {
        let mut session = ClientSession {
            local_paused: Some(false),
            ..ClientSession::default()
        };

        let actions = session.handle_disconnect(123.4);
        assert_eq!(actions, vec![ClientRuntimeAction::SetPaused(true)]);
        assert_eq!(session.last_paused_on_leave_at_seconds(), Some(123.4));
    }

    #[test]
    fn handle_disconnect_respects_pause_on_leave_toggle() {
        let mut session = ClientSession {
            local_paused: Some(false),
            ..ClientSession::default()
        };
        session.behavior_config_mut().pause_on_leave = false;

        let actions = session.handle_disconnect(200.0);
        assert!(actions.is_empty());
        assert_eq!(session.last_paused_on_leave_at_seconds(), None);
    }

    #[test]
    fn handle_disconnect_clears_chat_support_until_next_hello() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
        assert_eq!(session.server_chat_supported(), Some(true));

        let _ = session.handle_disconnect(200.0);
        assert_eq!(session.server_chat_supported(), None);
    }

    #[test]
    fn instaplay_conditions_met_respects_legacy_unpause_modes() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"bob.mp4"},"isReady":true}}}}"#,
            )
            .expect("bob ready state should apply");

        assert!(
            !session.instaplay_conditions_met(true, false),
            "default IfAlreadyReady mode should require local ready=true"
        );

        session
            .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
            .expect("local ready should apply");
        assert!(
            session.instaplay_conditions_met(true, false),
            "local ready=true should satisfy IfAlreadyReady mode"
        );

        session
            .apply_message_json(r#"{"Set":{"ready":{"isReady":false,"username":"alice"}}}"#)
            .expect("local not-ready should apply");
        session.readiness_autoplay_config_mut().unpause_action = UnpauseActionMode::Always;
        assert!(
            session.instaplay_conditions_met(true, false),
            "Always mode should allow unpause when controllable"
        );

        session.readiness_autoplay_config_mut().unpause_action = UnpauseActionMode::IfOthersReady;
        assert!(
            session.instaplay_conditions_met(true, false),
            "IfOthersReady mode should pass when all other room users are ready"
        );

        session
            .apply_message_json(r#"{"Set":{"ready":{"isReady":false,"username":"bob"}}}"#)
            .expect("other user not-ready state should apply");
        assert!(
            !session.instaplay_conditions_met(true, false),
            "IfOthersReady mode should fail when another room user is not ready"
        );
    }

    #[test]
    fn instaplay_if_others_ready_ignores_users_without_files() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"isReady":false}}}}"#,
            )
            .expect("bob state should apply");
        session.readiness_autoplay_config_mut().unpause_action = UnpauseActionMode::IfOthersReady;

        assert!(
            session.instaplay_conditions_met(true, false),
            "legacy isReadyWithFile should ignore non-ready users without file metadata"
        );
    }

    #[test]
    fn readiness_counts_only_include_other_users_ready_with_file() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
            .expect("local ready state should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"isReady":true}}}}"#,
            )
            .expect("bob state should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"carol":{"room":{"name":"room1"},"file":{"name":"carol.mp4"},"isReady":true}}}}"#,
            )
            .expect("carol state should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"dave":{"room":{"name":"room1"},"file":{"name":"dave.mp4"},"isReady":false}}}}"#,
            )
            .expect("dave state should apply");

        assert_eq!(session.users_in_current_room_count_for_threshold(), 2);
        assert_eq!(session.ready_user_count_in_current_room(), 2);
    }

    #[test]
    fn list_snapshot_empty_file_payload_does_not_block_readiness_checks() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"List":{"room1":{"alice":{"isReady":true,"file":{"name":"alice.mp4"}},"bob":{"isReady":false,"file":{}}}}}"#,
            )
            .expect("list snapshot should apply");
        assert_eq!(session.user_has_file("bob"), Some(false));
        assert_eq!(session.user_file_name("bob"), None);
        assert!(
            session.all_other_users_in_current_room_ready(),
            "empty-object file payload should match legacy no-file behavior"
        );

        session
            .apply_message_json(
                r#"{"List":{"room1":{"alice":{"isReady":true,"file":{"name":"alice.mp4"}},"bob":{"isReady":false,"file":{"name":"bob.mp4"}}}}}"#,
            )
            .expect("list snapshot should apply");
        assert_eq!(session.user_has_file("bob"), Some(true));
        assert_eq!(session.user_file_name("bob"), Some("bob.mp4"));
        assert!(
            !session.all_other_users_in_current_room_ready(),
            "non-ready users with file metadata should block readiness checks"
        );
    }

    #[test]
    fn autoplay_require_same_filenames_blocks_missing_file_metadata() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
            .expect("local ready state should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"file":{"name":"movie.mp4"},"isReady":true}}}}"#,
            )
            .expect("local file state should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"isReady":true}}}}"#,
            )
            .expect("other user state should apply");
        session.set_autoplay_enabled(true);
        session.readiness_autoplay_config_mut().auto_play_threshold = Some(2);
        session
            .readiness_autoplay_config_mut()
            .autoplay_require_same_filenames = true;
        session.local_paused = Some(true);

        assert!(
            !session.autoplay_conditions_met(true, true, false, false),
            "autoplayRequireSameFilenames should fail when room users are missing file metadata"
        );
    }

    #[test]
    fn autoplay_require_same_filenames_uses_legacy_filename_comparison() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"List":{"room1":{"alice":{"isReady":true,"file":{"name":"Movie-Name.mkv"}},"bob":{"isReady":true,"file":{"name":"moviename.mkv"}}}}}"#,
            )
            .expect("matching filenames list snapshot should apply");
        session.set_autoplay_enabled(true);
        session.readiness_autoplay_config_mut().auto_play_threshold = Some(2);
        session
            .readiness_autoplay_config_mut()
            .autoplay_require_same_filenames = true;
        session.local_paused = Some(true);

        assert!(
            session.autoplay_conditions_met(true, true, false, false),
            "legacy filename normalization should treat punctuation/case variants as same file"
        );

        session
            .apply_message_json(
                r#"{"List":{"room1":{"alice":{"isReady":true,"file":{"name":"Movie-Name.mkv"}},"bob":{"isReady":true,"file":{"name":"other.mkv"}}}}}"#,
            )
            .expect("mismatched filenames list snapshot should apply");
        assert!(
            !session.autoplay_conditions_met(true, true, false, false),
            "autoplayRequireSameFilenames should fail when filenames differ"
        );
    }

    #[test]
    fn same_filename_legacy_like_treats_hidden_filename_as_match() {
        assert!(ClientSession::same_filename_legacy_like(
            PRIVACY_HIDDEN_FILENAME,
            "anything.mkv",
        ));
        assert!(ClientSession::same_filename_legacy_like(
            "anything.mkv",
            PRIVACY_HIDDEN_FILENAME,
        ));
    }

    #[test]
    fn same_filename_legacy_like_matches_url_encoded_and_plain_names() {
        assert!(ClientSession::same_filename_legacy_like(
            "https://example.invalid/media/Movie%20Name.mkv",
            "Movie Name.mkv",
        ));
    }

    #[test]
    fn same_filename_legacy_like_matches_raw_filename_and_hash_form() {
        let raw_name = "Movie Name.mkv";
        let stripped = ClientSession::strip_filename_for_compare(raw_name, false);
        let hashed = ClientSession::hash_filename_for_compare(&stripped);
        assert!(ClientSession::same_filename_legacy_like(raw_name, &hashed));
    }

    #[test]
    fn same_filesize_legacy_like_treats_numeric_zero_as_wildcard() {
        assert!(ClientSession::same_filesize_legacy_like(
            &Value::from(0),
            &Value::from(123_456_789),
        ));
        assert!(ClientSession::same_filesize_legacy_like(
            &Value::from(123_456_789),
            &Value::from(0),
        ));
        assert!(
            !ClientSession::same_filesize_legacy_like(&Value::from("0"), &Value::from(123_456_789),),
            "legacy behavior only treats numeric 0 as wildcard, not string \"0\""
        );
    }

    #[test]
    fn same_filesize_legacy_like_matches_raw_and_hash_forms() {
        let raw_size = Value::from(123_456_789);
        let hashed = Value::from(ClientSession::hash_filesize_for_compare("123456789"));
        assert!(ClientSession::same_filesize_legacy_like(&raw_size, &hashed));
    }

    #[test]
    fn same_fileduration_legacy_like_respects_default_threshold() {
        assert!(
            ClientSession::same_fileduration_legacy_compatible(10.49, 12.49),
            "rounded duration diff of 2 should match with legacy 2.5 threshold"
        );
        assert!(
            !ClientSession::same_fileduration_legacy_compatible(10.49, 13.49),
            "rounded duration diff of 3 should fail with legacy 2.5 threshold"
        );
    }

    #[test]
    fn same_fileduration_legacy_like_uses_python_ties_to_even_rounding() {
        assert!(
            ClientSession::same_fileduration_legacy_compatible(1.5, 4.5),
            "Python round() ties-to-even should yield 2 vs 4 (diff 2), not away-from-zero"
        );
    }

    #[test]
    fn same_fileduration_legacy_like_short_circuits_when_duration_notifications_disabled() {
        assert!(ClientSession::same_fileduration_legacy_like(
            1.0, 999.0, false, 2.5
        ));
    }

    #[test]
    fn same_fileduration_legacy_compatible_with_overrides_respects_toggle_and_threshold() {
        assert!(
            ClientSession::same_fileduration_legacy_compatible_with_overrides(
                10.49, 13.49, false, 0.1
            )
        );
        assert!(
            !ClientSession::same_fileduration_legacy_compatible_with_overrides(
                10.49, 12.49, true, 1.0
            )
        );
        assert!(
            ClientSession::same_fileduration_legacy_compatible_with_overrides(
                10.49, 12.49, true, 3.0
            )
        );
    }

    #[test]
    fn readiness_autoplay_config_defaults_include_legacy_duration_comparison_settings() {
        let config = ReadinessAutoplayConfig::default();
        assert!(config.show_duration_notification);
        assert_eq!(
            config.different_duration_threshold_seconds,
            LEGACY_DIFFERENT_DURATION_THRESHOLD_SECONDS
        );
    }

    #[test]
    fn chat_config_defaults_include_legacy_max_message_length() {
        let config = ChatConfig::default();
        assert_eq!(
            config.max_chat_message_length,
            LEGACY_CHAT_MAX_MESSAGE_LENGTH
        );
        assert!(config.apply_server_max_chat_message_length);
    }

    #[test]
    fn outbound_chat_message_truncates_to_configured_max_length() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
        session.chat_config_mut().max_chat_message_length = 5;
        assert_eq!(
            session.runtime_actions_for_outbound_chat_message("hello world".to_owned()),
            vec![ClientRuntimeAction::SendChat {
                message: "hello".to_owned(),
            }]
        );
    }

    #[test]
    fn outbound_chat_message_preserves_empty_payload_legacy_compatible() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
        assert_eq!(
            session.runtime_actions_for_outbound_chat_message("".to_owned()),
            vec![ClientRuntimeAction::SendChat {
                message: "".to_owned(),
            }]
        );
        assert_eq!(
            session.runtime_actions_for_outbound_chat_message("\n\r".to_owned()),
            vec![ClientRuntimeAction::SendChat {
                message: "".to_owned(),
            }]
        );
    }

    #[test]
    fn outbound_chat_message_is_omitted_when_max_length_is_zero() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
        session.chat_config_mut().max_chat_message_length = 0;
        assert!(
            session
                .runtime_actions_for_outbound_chat_message("hello world".to_owned())
                .is_empty()
        );
    }

    #[test]
    fn outbound_chat_message_is_omitted_before_server_hello() {
        let session = ClientSession::default();
        assert!(
            session
                .runtime_actions_for_outbound_chat_message("hello world".to_owned())
                .is_empty()
        );
    }

    #[test]
    fn outbound_chat_message_is_omitted_when_server_version_is_pre_chat_min_without_features() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        assert!(
            session
                .runtime_actions_for_outbound_chat_message("hello world".to_owned())
                .is_empty()
        );
    }

    #[test]
    fn outbound_chat_message_strips_newlines_before_truncation() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
        session.chat_config_mut().max_chat_message_length = 4;
        assert_eq!(
            session.runtime_actions_for_outbound_chat_message("a\nb\rcd".to_owned()),
            vec![ClientRuntimeAction::SendChat {
                message: "abcd".to_owned(),
            }]
        );
    }

    #[test]
    fn outbound_chat_message_is_omitted_when_server_chat_feature_is_disabled() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255","features":{"chat":false}}}"#,
            )
            .expect("hello should apply");
        assert!(
            session
                .runtime_actions_for_outbound_chat_message("hello world".to_owned())
                .is_empty()
        );
    }

    #[test]
    fn same_fileduration_with_readiness_autoplay_config_uses_session_overrides() {
        let mut session = ClientSession::default();
        session
            .readiness_autoplay_config_mut()
            .show_duration_notification = false;
        assert!(session.same_fileduration_with_readiness_autoplay_config(10.0, 999.0));

        session
            .readiness_autoplay_config_mut()
            .show_duration_notification = true;
        session
            .readiness_autoplay_config_mut()
            .different_duration_threshold_seconds = 1.0;
        assert!(!session.same_fileduration_with_readiness_autoplay_config(10.49, 12.49));
    }

    #[test]
    fn file_differences_for_current_room_detects_all_mismatch_types() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"file":{"name":"movie.mkv","size":123456789,"duration":95.5}}}}}"#,
            )
            .expect("local file should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"other.mkv","size":987654321,"duration":100.0}}}}}"#,
            )
            .expect("peer file should apply");

        let summary = session
            .file_differences_for_current_room()
            .expect("current room file differences should be available");
        assert_eq!(
            summary,
            FileDifferenceSummary {
                filename: true,
                filesize: true,
                fileduration: true,
            }
        );
        assert!(summary.has_differences());
    }

    #[test]
    fn file_differences_for_current_room_respects_duration_override_toggle() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"file":{"name":"movie.mkv","size":123456789,"duration":95.0}}}}}"#,
            )
            .expect("local file should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"movie.mkv","size":123456789,"duration":99.0}}}}}"#,
            )
            .expect("peer file should apply");

        let default_summary = session
            .file_differences_for_current_room()
            .expect("file differences should be available with duration mismatch");
        assert_eq!(
            default_summary,
            FileDifferenceSummary {
                filename: false,
                filesize: false,
                fileduration: true,
            }
        );
        assert!(default_summary.has_differences());

        session
            .readiness_autoplay_config_mut()
            .show_duration_notification = false;
        let override_summary = session
            .file_differences_for_current_room()
            .expect("file differences should still be computable");
        assert_eq!(
            override_summary,
            FileDifferenceSummary {
                filename: false,
                filesize: false,
                fileduration: false,
            }
        );
        assert!(!override_summary.has_differences());
    }

    #[test]
    fn file_differences_for_user_skips_out_of_room_and_missing_file_states() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        assert_eq!(session.file_differences_for_user("bob"), None);

        session
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"file":{"name":"movie.mkv","size":123456789,"duration":95.5}}}}}"#,
            )
            .expect("local file should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room2"},"file":{"name":"other.mkv","size":123456789,"duration":95.5}}}}}"#,
            )
            .expect("out-of-room peer should apply");
        assert_eq!(session.file_differences_for_user("bob"), None);

        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"other.mkv","size":123456789,"duration":95.5}}}}}"#,
            )
            .expect("in-room peer should apply");
        assert_eq!(
            session.file_differences_for_user("bob"),
            Some(FileDifferenceSummary {
                filename: true,
                filesize: false,
                fileduration: false,
            })
        );
    }

    #[test]
    fn sanitize_outbound_file_payload_legacy_like_applies_privacy_modes_and_removes_path() {
        let payload = json!({
            "name": "https://example.invalid/media/Movie Name.mkv",
            "size": 123456789,
            "duration": 95.5,
            "path": "C:/media/movie.mkv",
            "extra": "keep-me"
        });

        let raw = ClientSession::sanitize_outbound_file_payload_legacy_compatible(
            &payload,
            PrivacyMode::SendRaw,
            PrivacyMode::SendRaw,
        )
        .expect("raw mode should return sanitized payload");
        assert_eq!(
            raw,
            json!({
                "name": "https://example.invalid/media/Movie Name.mkv",
                "size": 123456789,
                "duration": 95.5,
                "extra": "keep-me"
            })
        );

        let hashed = ClientSession::sanitize_outbound_file_payload_legacy_compatible(
            &payload,
            PrivacyMode::SendHashed,
            PrivacyMode::SendHashed,
        )
        .expect("hashed mode should return sanitized payload");
        assert_eq!(
            hashed,
            json!({
                "name": "a9858cb4803c",
                "size": "15e2b0d3c338",
                "duration": 95.5,
                "extra": "keep-me"
            })
        );

        let hidden = ClientSession::sanitize_outbound_file_payload_legacy_compatible(
            &payload,
            PrivacyMode::DoNotSend,
            PrivacyMode::DoNotSend,
        )
        .expect("hidden mode should return sanitized payload");
        assert_eq!(
            hidden,
            json!({
                "name": PRIVACY_HIDDEN_FILENAME,
                "size": 0,
                "duration": 95.5,
                "extra": "keep-me"
            })
        );
    }

    #[test]
    fn privacy_mode_from_legacy_name_maps_expected_modes() {
        assert_eq!(
            PrivacyMode::from_legacy_name("SendRaw"),
            Some(PrivacyMode::SendRaw)
        );
        assert_eq!(
            PrivacyMode::from_legacy_name("SendHashed"),
            Some(PrivacyMode::SendHashed)
        );
        assert_eq!(
            PrivacyMode::from_legacy_name("DoNotSend"),
            Some(PrivacyMode::DoNotSend)
        );
        assert_eq!(PrivacyMode::from_legacy_name("unknown"), None);
    }

    #[test]
    fn local_file_publish_runtime_actions_apply_privacy_and_update_local_user_file_view() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        let file_payload = json!({
            "name": "https://example.invalid/media/Movie Name.mkv",
            "size": 123456789,
            "duration": 95.5,
            "path": "C:/media/movie.mkv",
            "extra": "keep-me"
        });

        let actions = session.runtime_actions_for_local_file_publish_legacy_compatible(
            &file_payload,
            PrivacyMode::SendHashed,
            PrivacyMode::SendHashed,
        );

        assert_eq!(
            actions,
            vec![ClientRuntimeAction::SetFile {
                file_payload: json!({
                    "name": "a9858cb4803c",
                    "size": "15e2b0d3c338",
                    "duration": 95.5,
                    "extra": "keep-me"
                }),
            }]
        );
        assert_eq!(session.user_has_file("alice"), Some(true));
        assert_eq!(session.user_file_name("alice"), Some("a9858cb4803c"));
        assert_eq!(
            session.user_file_size("alice"),
            Some(&json!("15e2b0d3c338"))
        );
        assert_eq!(session.user_file_duration("alice"), Some(&json!(95.5)));
    }

    #[test]
    fn local_file_publish_empty_payload_clears_local_user_file_view() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"file":{"name":"movie.mkv","size":123456789,"duration":95.5}}}}}"#,
            )
            .expect("existing local file view should apply");
        assert_eq!(session.user_has_file("alice"), Some(true));

        let actions = session.runtime_actions_for_local_file_publish_legacy_compatible(
            &json!({}),
            PrivacyMode::SendRaw,
            PrivacyMode::SendRaw,
        );

        assert_eq!(
            actions,
            vec![ClientRuntimeAction::SetFile {
                file_payload: json!({}),
            }]
        );
        assert_eq!(session.user_has_file("alice"), Some(false));
        assert_eq!(session.user_file_name("alice"), None);
        assert_eq!(session.user_file_size("alice"), None);
        assert_eq!(session.user_file_duration("alice"), None);
    }

    #[test]
    fn runtime_actions_for_readiness_unpause_attempt_blocks_and_sets_ready_when_instaplay_fails() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        let actions =
            session.runtime_actions_for_readiness_unpause_attempt(10.0, true, true, false);
        assert_eq!(
            actions,
            vec![
                ClientRuntimeAction::SetPaused(true),
                ClientRuntimeAction::SetReady {
                    ready: true,
                    manually_initiated: true
                }
            ]
        );
        assert_eq!(session.local_paused, Some(true));
    }

    #[test]
    fn runtime_actions_for_readiness_unpause_attempt_sets_ready_when_if_others_ready_passes() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"bob.mp4"},"isReady":true}}}}"#,
            )
            .expect("other user ready state should apply");
        session.readiness_autoplay_config_mut().unpause_action = UnpauseActionMode::IfOthersReady;

        let actions =
            session.runtime_actions_for_readiness_unpause_attempt(20.0, true, true, false);
        assert_eq!(
            actions,
            vec![ClientRuntimeAction::SetReady {
                ready: true,
                manually_initiated: false
            }]
        );
        assert_eq!(session.local_paused, Some(false));
    }

    #[test]
    fn runtime_actions_for_readiness_unpause_attempt_honors_pause_on_leave_cooldown() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session.readiness_autoplay_config_mut().unpause_action = UnpauseActionMode::Always;

        let disconnect_actions = session.handle_disconnect(100.0);
        assert_eq!(
            disconnect_actions,
            vec![ClientRuntimeAction::SetPaused(true)]
        );
        assert_eq!(session.last_paused_on_leave_at_seconds(), Some(100.0));

        let actions =
            session.runtime_actions_for_readiness_unpause_attempt(101.0, true, true, false);
        assert!(
            actions.is_empty(),
            "legacy behavior suppresses readiness toggle right after pause-on-leave"
        );
        assert_eq!(session.last_paused_on_leave_at_seconds(), None);
        assert_eq!(session.local_paused, Some(false));
    }

    #[test]
    fn runtime_actions_for_readiness_unpause_attempt_if_min_users_ready_requires_threshold() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"bob.mp4"},"isReady":true}}}}"#,
            )
            .expect("other user ready state should apply");
        session.readiness_autoplay_config_mut().unpause_action = UnpauseActionMode::IfMinUsersReady;
        session.readiness_autoplay_config_mut().auto_play_threshold = Some(3);

        let blocked =
            session.runtime_actions_for_readiness_unpause_attempt(30.0, true, true, false);
        assert_eq!(
            blocked,
            vec![
                ClientRuntimeAction::SetPaused(true),
                ClientRuntimeAction::SetReady {
                    ready: true,
                    manually_initiated: true
                }
            ]
        );

        session.readiness_autoplay_config_mut().auto_play_threshold = Some(2);
        let allowed =
            session.runtime_actions_for_readiness_unpause_attempt(31.0, true, true, false);
        assert_eq!(
            allowed,
            vec![ClientRuntimeAction::SetReady {
                ready: true,
                manually_initiated: false
            }]
        );
    }

    #[test]
    fn autoplay_check_starts_countdown_when_conditions_are_met() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
            .expect("local ready state should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"bob.mp4"},"isReady":true}}}}"#,
            )
            .expect("other user ready state should apply");
        session.set_autoplay_enabled(true);
        session.readiness_autoplay_config_mut().auto_play_threshold = Some(2);
        session.local_paused = Some(true);

        session.autoplay_check(true, true, false, false);

        assert!(session.autoplay_timer_is_running());
        assert_eq!(
            session.autoplay_time_left_seconds(),
            session.readiness_autoplay_config().autoplay_delay_seconds
        );
    }

    #[test]
    fn autoplay_check_stops_countdown_when_conditions_fail() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
            .expect("local ready state should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"bob.mp4"},"isReady":true}}}}"#,
            )
            .expect("other user ready state should apply");
        session.set_autoplay_enabled(true);
        session.readiness_autoplay_config_mut().auto_play_threshold = Some(2);
        session.local_paused = Some(true);
        session.autoplay_check(true, true, false, false);
        assert!(session.autoplay_timer_is_running());

        session
            .apply_message_json(r#"{"Set":{"ready":{"isReady":false,"username":"bob"}}}"#)
            .expect("other user not-ready state should apply");
        session.autoplay_check(true, true, false, false);

        assert!(!session.autoplay_timer_is_running());
        assert_eq!(
            session.autoplay_time_left_seconds(),
            session.readiness_autoplay_config().autoplay_delay_seconds
        );
    }

    #[test]
    fn autoplay_countdown_tick_unpauses_when_timer_reaches_zero() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
            .expect("local ready state should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"bob.mp4"},"isReady":true}}}}"#,
            )
            .expect("other user ready state should apply");
        session.set_autoplay_enabled(true);
        session.readiness_autoplay_config_mut().auto_play_threshold = Some(2);
        session.local_paused = Some(true);
        session.autoplay_check(true, true, false, false);

        let tick_1 = session.autoplay_countdown_tick(true, true, false, false);
        let tick_2 = session.autoplay_countdown_tick(true, true, false, false);
        let tick_3 = session.autoplay_countdown_tick(true, true, false, false);
        let tick_4 = session.autoplay_countdown_tick(true, true, false, false);

        assert_eq!(
            tick_1,
            vec![ClientRuntimeAction::NotifyAutoplayCountdown(
                AutoplayCountdownNotification {
                    ready_user_count: 2,
                    seconds_left: 3
                }
            )]
        );
        assert_eq!(
            tick_2,
            vec![ClientRuntimeAction::NotifyAutoplayCountdown(
                AutoplayCountdownNotification {
                    ready_user_count: 2,
                    seconds_left: 2
                }
            )]
        );
        assert_eq!(
            tick_3,
            vec![ClientRuntimeAction::NotifyAutoplayCountdown(
                AutoplayCountdownNotification {
                    ready_user_count: 2,
                    seconds_left: 1
                }
            )]
        );
        assert_eq!(tick_4, vec![ClientRuntimeAction::SetPaused(false)]);
        assert_eq!(session.local_paused, Some(false));
        assert!(!session.autoplay_timer_is_running());
        assert_eq!(
            session.autoplay_time_left_seconds(),
            session.readiness_autoplay_config().autoplay_delay_seconds
        );
    }

    #[test]
    fn autoplay_conditions_recently_advanced_overrides_disabled_autoplay_and_threshold() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
            .expect("local ready state should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"bob.mp4"},"isReady":true}}}}"#,
            )
            .expect("other user ready state should apply");
        session.set_autoplay_enabled(false);
        session.readiness_autoplay_config_mut().auto_play_threshold = Some(5);
        session.local_paused = Some(true);

        assert!(
            !session.autoplay_conditions_met(true, true, false, false),
            "without recentlyAdvanced override autoplay should stay blocked"
        );
        assert!(
            session.autoplay_conditions_met(true, true, false, true),
            "recentlyAdvanced should allow countdown conditions even with disabled autoplay and unmet threshold"
        );
    }

    #[test]
    fn autoplay_check_ignores_playing_music_state() {
        let mut session = ClientSession {
            autoplay_timer_running: true,
            autoplay_time_left_seconds: 1.5,
            ..ClientSession::default()
        };

        session.autoplay_check(true, true, true, false);

        assert!(session.autoplay_timer_is_running());
        assert_eq!(session.autoplay_time_left_seconds(), 1.5);
    }

    #[test]
    fn queued_runtime_control_set_ready_emits_protocol_set_ready_message() {
        let mut control = QueuedRuntimeControl::default();
        control.set_ready(true, false);

        assert_eq!(control.outbound_messages().len(), 1);
        let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
            panic!("expected queued control to emit Set message");
        };
        let ready = set_message
            .set
            .ready
            .as_ref()
            .expect("Set message should contain ready payload");
        assert!(ready.is_ready);
        assert_eq!(ready.manually_initiated, Some(false));
    }

    #[test]
    fn queued_runtime_control_set_room_emits_protocol_set_room_message() {
        let mut control = QueuedRuntimeControl::default();
        control.set_room("room2".to_owned());

        assert_eq!(control.outbound_messages().len(), 1);
        let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
            panic!("expected queued control room change to emit Set message");
        };
        let room = set_message
            .set
            .room
            .as_ref()
            .expect("Set message should contain room payload");
        assert_eq!(room.name, "room2");
    }

    #[test]
    fn queued_runtime_control_set_file_emits_protocol_set_file_message() {
        let mut control = QueuedRuntimeControl::default();
        control.set_file(json!({
            "name": "movie.mkv",
            "duration": 95.5,
            "size": 123456789,
            "path": "C:/media/movie.mkv",
            "extra": "keep-me"
        }));

        assert_eq!(control.outbound_messages().len(), 1);
        let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
            panic!("expected queued control to emit Set message");
        };
        let file = set_message
            .set
            .file
            .as_ref()
            .expect("Set message should contain file payload");
        assert_eq!(file.name.as_deref(), Some("movie.mkv"));
        assert_eq!(file.duration, Some(95.5));
        assert_eq!(file.size.as_ref(), Some(&json!(123456789)));
        assert_eq!(file.path.as_deref(), Some("C:/media/movie.mkv"));
        assert_eq!(file.extra.get("extra"), Some(&json!("keep-me")));
    }

    #[test]
    fn queued_runtime_control_set_playlist_and_index_emit_protocol_messages() {
        let mut control = QueuedRuntimeControl::default();
        control.set_playlist(vec!["episode1.mkv".to_owned(), "episode2.mkv".to_owned()]);
        control.set_playlist_index(1);

        assert_eq!(control.outbound_messages().len(), 2);
        let ProtocolMessage::Set(change_message) = &control.outbound_messages()[0] else {
            panic!("expected queued control playlist change to emit Set message");
        };
        let playlist_change = change_message
            .set
            .playlist_change
            .as_ref()
            .expect("Set message should contain playlistChange payload");
        assert_eq!(playlist_change.files, vec!["episode1.mkv", "episode2.mkv"]);
        assert!(playlist_change.user.is_none());

        let ProtocolMessage::Set(index_message) = &control.outbound_messages()[1] else {
            panic!("expected queued control playlist index to emit Set message");
        };
        let playlist_index = index_message
            .set
            .playlist_index
            .as_ref()
            .expect("Set message should contain playlistIndex payload");
        assert_eq!(playlist_index.index, 1);
        assert!(playlist_index.user.is_none());
    }

    #[test]
    fn queued_runtime_control_request_controller_auth_emits_protocol_message() {
        let mut control = QueuedRuntimeControl::default();
        control.request_controller_auth("+room:ABCDEF123456".to_owned(), "AB-123-456".to_owned());

        assert_eq!(control.outbound_messages().len(), 1);
        let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
            panic!("expected queued controller auth to emit Set message");
        };
        let controller_auth = set_message
            .set
            .controller_auth
            .as_ref()
            .expect("Set message should contain controllerAuth payload");
        assert_eq!(controller_auth.room.as_deref(), Some("+room:ABCDEF123456"));
        assert_eq!(controller_auth.password.as_deref(), Some("AB-123-456"));
        assert!(controller_auth.user.is_none());
        assert!(controller_auth.success.is_none());
    }

    #[test]
    fn client_runtime_publish_local_file_dispatches_sanitized_set_file_message() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        runtime
            .publish_local_file_legacy_compatible(
                &json!({
                    "name": "movie.mkv",
                    "size": 123456789,
                    "duration": 95.5,
                    "path": "C:/media/movie.mkv"
                }),
                PrivacyMode::DoNotSend,
                PrivacyMode::DoNotSend,
            )
            .expect("file publish should dispatch");

        let (session, player, control) = runtime.into_parts();
        assert_eq!(player.paused, None);
        assert_eq!(session.user_has_file("alice"), Some(true));
        assert_eq!(
            session.user_file_name("alice"),
            Some(PRIVACY_HIDDEN_FILENAME)
        );
        assert_eq!(session.user_file_size("alice"), Some(&json!(0)));
        assert_eq!(session.user_file_duration("alice"), Some(&json!(95.5)));

        assert_eq!(control.outbound_messages().len(), 1);
        let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
            panic!("expected queued Set.file protocol message");
        };
        let file = set_message
            .set
            .file
            .as_ref()
            .expect("queued message should include file payload");
        assert_eq!(file.name.as_deref(), Some(PRIVACY_HIDDEN_FILENAME));
        assert_eq!(file.duration, Some(95.5));
        assert_eq!(file.size.as_ref(), Some(&json!(0)));
        assert!(file.path.is_none());
    }

    #[test]
    fn client_runtime_publish_pending_local_file_update_dispatches_sanitized_set_file_message() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        let player = RecordingPlayer {
            pending_local_file_update: Some(
                LocalFileUpdate::new("https://example.invalid/media/Movie Name.mkv")
                    .with_duration_seconds(95.5)
                    .with_size_bytes(123_456_789)
                    .with_path("C:/media/movie.mkv"),
            ),
            ..Default::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        let published = runtime
            .publish_pending_local_file_update_legacy_compatible(
                PrivacyMode::SendHashed,
                PrivacyMode::DoNotSend,
            )
            .expect("pending local file update should publish");
        assert!(published);
        let published_again = runtime
            .publish_pending_local_file_update_legacy_compatible(
                PrivacyMode::SendHashed,
                PrivacyMode::DoNotSend,
            )
            .expect("second pending local file update poll should not fail");
        assert!(!published_again);

        let (session, player, control) = runtime.into_parts();
        assert_eq!(player.paused, None);
        assert_eq!(session.user_has_file("alice"), Some(true));
        assert_eq!(session.user_file_name("alice"), Some("a9858cb4803c"));
        assert_eq!(session.user_file_size("alice"), Some(&json!(0)));
        assert_eq!(session.user_file_duration("alice"), Some(&json!(95.5)));

        assert_eq!(control.outbound_messages().len(), 1);
        let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
            panic!("expected queued Set.file protocol message");
        };
        let file = set_message
            .set
            .file
            .as_ref()
            .expect("queued message should include file payload");
        assert_eq!(file.name.as_deref(), Some("a9858cb4803c"));
        assert_eq!(file.duration, Some(95.5));
        assert_eq!(file.size.as_ref(), Some(&json!(0)));
        assert!(file.path.is_none());
    }

    #[test]
    fn client_runtime_reconnect_playlist_restore_dispatches_protocol_messages() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("local playlist should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
            .expect("local playlist index should apply");
        session.reset_sync_state_for_reconnect();
        session
            .apply_message_json(r#"{"Set":{"playlistChange":{"files":[]}}}"#)
            .expect("empty reconnect playlist snapshot should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        runtime
            .run_reconnect_playlist_restore_if_needed()
            .expect("reconnect playlist restore should dispatch");

        let (_, _, control) = runtime.into_parts();
        assert_eq!(control.outbound_messages().len(), 2);
        assert_eq!(
            control.reconnect_notifications(),
            &[ReconnectTransitionNotification::RestoringPlaylist]
        );
        let ProtocolMessage::Set(playlist_change_message) = &control.outbound_messages()[0] else {
            panic!("first outbound reconnect restore message should be Set.playlistChange");
        };
        let playlist_change = playlist_change_message
            .set
            .playlist_change
            .as_ref()
            .expect("first outbound message should include playlistChange");
        assert_eq!(playlist_change.files, vec!["episode1.mkv", "episode2.mkv"]);
        assert!(playlist_change.user.is_none());

        let ProtocolMessage::Set(playlist_index_message) = &control.outbound_messages()[1] else {
            panic!("second outbound reconnect restore message should be Set.playlistIndex");
        };
        let playlist_index = playlist_index_message
            .set
            .playlist_index
            .as_ref()
            .expect("second outbound message should include playlistIndex");
        assert_eq!(playlist_index.index, 1);
        assert!(playlist_index.user.is_none());
    }

    #[test]
    fn client_runtime_reconnect_state_restore_dispatches_ready_and_file_messages() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
            .expect("local ready should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"file":{"name":"movie.mkv","size":123456789,"duration":95.5}}}}}"#,
            )
            .expect("local file should apply");
        session.reset_sync_state_for_reconnect();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("reconnect hello should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        runtime
            .run_reconnect_state_restore_if_needed()
            .expect("reconnect state restore should dispatch");

        let (_, _, control) = runtime.into_parts();
        assert_eq!(control.outbound_messages().len(), 2);
        assert_eq!(
            control.reconnect_notifications(),
            &[ReconnectTransitionNotification::RestoringState]
        );

        let ProtocolMessage::Set(ready_message) = &control.outbound_messages()[0] else {
            panic!("first reconnect restore message should be Set.ready");
        };
        let ready = ready_message
            .set
            .ready
            .as_ref()
            .expect("first reconnect restore message should include ready payload");
        assert!(ready.is_ready);
        assert_eq!(ready.manually_initiated, Some(false));

        let ProtocolMessage::Set(file_message) = &control.outbound_messages()[1] else {
            panic!("second reconnect restore message should be Set.file");
        };
        let file = file_message
            .set
            .file
            .as_ref()
            .expect("second reconnect restore message should include file payload");
        assert_eq!(file.name.as_deref(), Some("movie.mkv"));
        assert_eq!(file.size.as_ref(), Some(&json!(123456789)));
        assert_eq!(file.duration, Some(95.5));
    }

    #[test]
    fn client_runtime_reconnect_state_restore_validation_uses_cached_telemetry_when_restore_starts_after_validation_tick()
     {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
            .expect("local ready should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"file":{"name":"movie.mkv","size":123456789,"duration":95.5}}}}}"#,
            )
            .expect("local file should apply");

        session.reset_sync_state_for_reconnect();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("reconnect hello should apply");
        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":120.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("reconnect room playstate should apply");

        let player = RecordingPlayer {
            pending_playback_telemetry_update: Some(
                PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(true)
                    .with_position_seconds(117.5),
            ),
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("validation tick before restore should not fail");

        assert!(
            runtime.drain_reconnect_notifications().is_empty(),
            "validation tick before restore should not emit reconnect notifications"
        );
        assert_eq!(
            runtime.player().paused,
            None,
            "validation should not issue correction commands before restore starts"
        );
        assert_eq!(
            runtime.player().position,
            None,
            "validation should not issue correction seeks before restore starts"
        );
        assert_eq!(
            runtime.session().local_paused,
            Some(true),
            "pre-restore validation tick should still pre-sync telemetry into session local state"
        );
        assert_eq!(
            runtime.session().local_position,
            Some(117.5),
            "pre-restore validation tick should still pre-sync telemetry position into session local state"
        );
        assert!(
            !runtime.session().reconnect_state_restore_validation_pending,
            "validation should remain disabled until restore dispatch starts the validation cycle"
        );
        assert_eq!(
            runtime.drain_player_playback_telemetry_updates(),
            vec![
                PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(true)
                    .with_position_seconds(117.5)
            ],
            "pre-restore validation tick should preserve telemetry for diagnostics drains"
        );

        runtime
            .run_reconnect_state_restore_if_needed()
            .expect("reconnect state restore should dispatch");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![ReconnectTransitionNotification::RestoringState],
            "restore dispatch should emit restoring-state notification before validation/correction"
        );
        assert_eq!(
            runtime.control().outbound_messages().len(),
            2,
            "restore dispatch should send ready/file restore messages"
        );
        assert!(
            runtime.session().reconnect_state_restore_validation_pending,
            "restore dispatch should enable reconnect state-restore validation"
        );

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("validation should complete using cached pre-restore telemetry");

        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: false,
                    local_position: 117.5,
                    room_position: 120.0,
                    position_diff_seconds: 2.5,
                }
            ],
            "post-restore validation should use cached telemetry-refreshed local state against cached room playstate"
        );
        assert_eq!(
            runtime.player().paused,
            Some(false),
            "post-restore validation should issue corrective pause command"
        );
        assert_eq!(
            runtime.player().position,
            Some(120.0),
            "post-restore validation should issue corrective seek command"
        );
        assert!(
            !runtime.session().reconnect_state_restore_validation_pending,
            "validation pending should clear after successful post-restore correction"
        );
        assert!(
            runtime.drain_player_playback_telemetry_updates().is_empty(),
            "no additional telemetry sample should be required once cached telemetry is used"
        );
    }

    #[test]
    fn client_runtime_room_pause_sync_applies_remote_pause_mismatch_from_playstate() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":5.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("remote state should apply");

        let player = RecordingPlayer {
            pending_playback_telemetry_update: Some(
                PlayerPlaybackTelemetryUpdate::default().with_paused(true),
            ),
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_room_pause_sync_if_needed()
            .expect("room pause sync should dispatch");

        assert_eq!(
            runtime.player().paused,
            Some(false),
            "remote room playstate pause mismatch should issue player unpause"
        );
        assert_eq!(
            runtime.session().local_paused,
            Some(false),
            "room pause sync should optimistically mirror local pause state until next telemetry sample"
        );
        assert_eq!(
            runtime.drain_player_playback_telemetry_updates(),
            vec![PlayerPlaybackTelemetryUpdate::default().with_paused(true)],
            "room pause sync should preserve synced telemetry for diagnostics drains"
        );
    }

    #[test]
    fn client_runtime_room_pause_sync_skips_when_room_playstate_set_by_local_user() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":5.0,"paused":false,"doSeek":false,"setBy":"alice"}}}"#,
            )
            .expect("self-originated state should apply");

        let player = RecordingPlayer {
            pending_playback_telemetry_update: Some(
                PlayerPlaybackTelemetryUpdate::default().with_paused(true),
            ),
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_room_pause_sync_if_needed()
            .expect("room pause sync should not fail");

        assert_eq!(
            runtime.player().paused,
            None,
            "self-originated room playstate should not trigger local pause correction"
        );
        assert_eq!(
            runtime.session().local_paused,
            Some(true),
            "telemetry sync should still update local paused snapshot"
        );
    }

    #[test]
    fn client_runtime_state_sync_reconcile_queues_outbound_state_after_inbound_state_seen() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        let player = RecordingPlayer {
            pending_playback_telemetry_update: Some(
                PlayerPlaybackTelemetryUpdate::default()
                    .with_position_seconds(12.5)
                    .with_paused(true),
            ),
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        let sent = runtime.run_state_sync_reconcile_with_inbound_state(
            StatePayload::new()
                .with_playstate(
                    PlaystatePayload::new()
                        .with_position(10.0)
                        .with_paused(false)
                        .with_set_by("bob"),
                )
                .with_ping(PingPayload::new().with_latency_calculation(42.0)),
            100.0,
            0.25,
        );

        assert!(sent, "state sync should emit after telemetry is available");
        assert_eq!(
            runtime.control().outbound_messages().len(),
            1,
            "state sync should queue one outbound state message"
        );
        let ProtocolMessage::State(state_message) = &runtime.control().outbound_messages()[0]
        else {
            panic!("queued message should be State");
        };
        assert_eq!(
            state_message
                .state
                .playstate
                .as_ref()
                .and_then(|p| p.position),
            Some(12.5),
            "outbound state should report local position"
        );
        assert_eq!(
            state_message
                .state
                .playstate
                .as_ref()
                .and_then(|p| p.paused),
            Some(true),
            "outbound state should report local paused state"
        );
        assert_eq!(
            state_message
                .state
                .ping
                .as_ref()
                .and_then(|ping| ping.latency_calculation),
            Some(42.0),
            "outbound ping should echo inbound latencyCalculation when present"
        );
        assert_eq!(
            state_message
                .state
                .ping
                .as_ref()
                .and_then(|ping| ping.client_latency_calculation),
            Some(100.0),
            "outbound ping should include client latency calculation"
        );
        assert_eq!(
            state_message
                .state
                .ping
                .as_ref()
                .and_then(|ping| ping.client_rtt),
            Some(0.25),
            "outbound ping should include client RTT"
        );
    }

    #[test]
    fn client_runtime_state_sync_reconcile_legacy_ping_wrapper_tracks_and_emits_ping_metrics() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        let player = RecordingPlayer {
            pending_playback_telemetry_update: Some(
                PlayerPlaybackTelemetryUpdate::default()
                    .with_position_seconds(12.5)
                    .with_paused(true),
            ),
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        let inbound_latency_calculation = unix_wall_clock_time_seconds_legacy_compatible() - 0.05;
        let sent = runtime.run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible(
            StatePayload::new()
                .with_playstate(
                    PlaystatePayload::new()
                        .with_position(10.0)
                        .with_paused(false)
                        .with_set_by("bob"),
                )
                .with_ping(
                    PingPayload::new().with_latency_calculation(inbound_latency_calculation),
                ),
        );

        assert!(
            sent,
            "legacy ping wrapper should emit after telemetry is available"
        );
        let ProtocolMessage::State(state_message) = &runtime.control().outbound_messages()[0]
        else {
            panic!("queued message should be State");
        };
        let ping = state_message
            .state
            .ping
            .as_ref()
            .expect("outbound state should include ping");
        assert_eq!(
            ping.latency_calculation,
            Some(inbound_latency_calculation),
            "outbound ping should echo inbound latencyCalculation"
        );
        assert!(
            ping.client_latency_calculation.unwrap_or(0.0) > 0.0,
            "outbound ping should include non-zero clientLatencyCalculation"
        );
        let client_rtt = ping
            .client_rtt
            .expect("outbound ping should include clientRtt");
        assert!(
            (0.0..2.0).contains(&client_rtt),
            "outbound ping should include a plausible clientRtt, got {client_rtt}"
        );
        assert_eq!(
            ping.server_rtt, None,
            "client outbound ping should not echo serverRtt from inbound state"
        );
    }

    #[test]
    fn client_runtime_desync_correction_legacy_ping_forward_delay_compensates_borderline_fastforward_threshold()
     {
        fn local_unpaused_telemetry(position_seconds: f64) -> PlayerPlaybackTelemetryUpdate {
            PlayerPlaybackTelemetryUpdate::default()
                .with_position_seconds(position_seconds)
                .with_paused(false)
        }

        fn runtime_fixture() -> ClientRuntime<RecordingPlayer, QueuedRuntimeControl> {
            let session = desync_session_with_remote_state(5.0, false, false, "bob");
            let player = RecordingPlayer {
                pending_playback_telemetry_update: Some(local_unpaused_telemetry(0.2)),
                ..RecordingPlayer::default()
            };
            let control = QueuedRuntimeControl::default();
            ClientRuntime::new(session, player, control)
        }

        let mut baseline_runtime = runtime_fixture();
        baseline_runtime
            .run_desync_correction_if_needed(0.0, false, false, false)
            .expect("initial behind detection should not fail");
        baseline_runtime
            .player_mut()
            .pending_playback_telemetry_update = Some(local_unpaused_telemetry(0.2));
        baseline_runtime
            .run_desync_correction_if_needed(4.0, false, false, false)
            .expect("borderline fastforward check should not fail");
        assert_eq!(
            baseline_runtime.player().position,
            None,
            "without forward-delay compensation, local position should stay just above fastforward threshold"
        );

        let mut compensated_runtime = runtime_fixture();
        compensated_runtime
            .ping_metrics_legacy_compatible
            .forward_delay_seconds = 0.35;
        compensated_runtime
            .run_desync_correction_if_needed(0.0, false, false, false)
            .expect("initial behind detection with forward delay should not fail");
        compensated_runtime
            .player_mut()
            .pending_playback_telemetry_update = Some(local_unpaused_telemetry(0.2));
        compensated_runtime
            .run_desync_correction_if_needed(4.0, false, false, false)
            .expect("compensated fastforward check should not fail");
        assert_eq!(
            compensated_runtime.player().position,
            Some(5.25),
            "forward-delay compensation should push a borderline behind client over the fastforward threshold"
        );
    }

    #[test]
    fn client_ping_metrics_legacy_compatible_tracks_rtt_from_inbound_state_ping() {
        let mut ping_metrics = ClientPingMetricsLegacyCompatible::default();
        let inbound_state = StatePayload::new().with_ping(
            PingPayload::new()
                .with_latency_calculation(100.0)
                .with_client_rtt(0.25),
        );

        ping_metrics.observe_inbound_state_at(&inbound_state, 100.2);

        assert!(
            (ping_metrics.client_rtt_seconds() - 0.2).abs() < 1e-9,
            "client RTT should be computed from now - inbound latencyCalculation"
        );
        assert!(
            (ping_metrics.forward_delay_seconds() - 0.1).abs() < 1e-9,
            "without inbound serverRtt, forward delay should default to averageRTT/2"
        );
    }

    #[test]
    fn client_ping_metrics_legacy_compatible_tracks_server_rtt_and_forward_delay_estimate() {
        let mut ping_metrics = ClientPingMetricsLegacyCompatible::default();

        ping_metrics.observe_inbound_state_at(
            &StatePayload::new().with_ping(
                PingPayload::new()
                    .with_latency_calculation(100.0)
                    .with_client_rtt(0.25)
                    .with_server_rtt(0.12),
            ),
            100.3,
        );

        assert!(
            (ping_metrics.client_rtt_seconds() - 0.3).abs() < 1e-9,
            "client RTT should track now - inbound latencyCalculation"
        );
        assert!(
            (ping_metrics.server_rtt_seconds() - 0.12).abs() < 1e-9,
            "server RTT should track inbound ping.serverRtt"
        );
        assert!(
            (ping_metrics.forward_delay_seconds() - 0.33).abs() < 1e-9,
            "forward delay should use server-like formula averageRTT/2 + (clientRTT - serverRTT) when clientRTT is larger"
        );
    }

    #[test]
    fn client_ping_metrics_legacy_compatible_ignores_invalid_negative_ping_inputs() {
        let mut ping_metrics = ClientPingMetricsLegacyCompatible::default();

        ping_metrics.observe_inbound_state_at(
            &StatePayload::new().with_ping(
                PingPayload::new()
                    .with_latency_calculation(10.0)
                    .with_client_rtt(0.1),
            ),
            10.4,
        );
        let baseline = ping_metrics.client_rtt_seconds();

        ping_metrics.observe_inbound_state_at(
            &StatePayload::new().with_ping(
                PingPayload::new()
                    .with_latency_calculation(20.0)
                    .with_client_rtt(-1.0),
            ),
            20.5,
        );
        ping_metrics.observe_inbound_state_at(
            &StatePayload::new().with_ping(
                PingPayload::new()
                    .with_latency_calculation(25.0)
                    .with_client_rtt(0.1)
                    .with_server_rtt(-1.0),
            ),
            25.4,
        );
        ping_metrics.observe_inbound_state_at(
            &StatePayload::new().with_ping(PingPayload::new().with_latency_calculation(30.0)),
            29.0,
        );

        assert_eq!(
            ping_metrics.client_rtt_seconds(),
            baseline,
            "invalid ping inputs should not overwrite the tracked RTT"
        );
    }

    #[test]
    fn client_runtime_reconnect_state_and_playlist_restore_precede_validation_mismatch_notification()
     {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
            .expect("local ready should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"file":{"name":"movie.mkv","size":123456789,"duration":95.5}}}}}"#,
            )
            .expect("local file should apply");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("local playlist should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
            .expect("local playlist index should apply");

        session.reset_sync_state_for_reconnect();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("reconnect hello should apply");
        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":120.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("reconnect room playstate should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistChange":{"files":[]}}}"#)
            .expect("empty reconnect playlist snapshot should apply");

        let player = RecordingPlayer {
            pending_playback_telemetry_update: Some(
                PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(true)
                    .with_position_seconds(117.5),
            ),
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_reconnect_state_restore_if_needed()
            .expect("reconnect state restore should dispatch");
        assert!(
            runtime.session().reconnect_state_restore_validation_pending,
            "state restore dispatch should enable reconnect validation"
        );

        runtime
            .run_reconnect_playlist_restore_if_needed()
            .expect("reconnect playlist restore should dispatch");
        assert!(
            runtime.session().reconnect_state_restore_validation_pending,
            "playlist restore should not clear reconnect validation pending state"
        );

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("reconnect validation should run after state+playlist restore dispatch");

        assert_eq!(
            runtime.control().reconnect_notifications(),
            &[
                ReconnectTransitionNotification::RestoringState,
                ReconnectTransitionNotification::RestoringPlaylist,
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: false,
                    local_position: 117.5,
                    room_position: 120.0,
                    position_diff_seconds: 2.5,
                },
            ],
            "reconnect notifications should preserve restore-state, restore-playlist, then validation-mismatch ordering"
        );
        assert_eq!(
            runtime.control().outbound_messages().len(),
            4,
            "state restore + playlist restore should enqueue ready/file/playlist/index protocol messages"
        );

        let ProtocolMessage::Set(first_outbound) = &runtime.control().outbound_messages()[0] else {
            panic!("first reconnect outbound message should be Set.ready");
        };
        assert!(first_outbound.set.ready.is_some());
        let ProtocolMessage::Set(second_outbound) = &runtime.control().outbound_messages()[1]
        else {
            panic!("second reconnect outbound message should be Set.file");
        };
        assert!(second_outbound.set.file.is_some());
        let ProtocolMessage::Set(third_outbound) = &runtime.control().outbound_messages()[2] else {
            panic!("third reconnect outbound message should be Set.playlistChange");
        };
        assert!(third_outbound.set.playlist_change.is_some());
        let ProtocolMessage::Set(fourth_outbound) = &runtime.control().outbound_messages()[3]
        else {
            panic!("fourth reconnect outbound message should be Set.playlistIndex");
        };
        assert!(fourth_outbound.set.playlist_index.is_some());

        assert_eq!(
            runtime.player().paused,
            Some(false),
            "validation mismatch should still issue corrective pause after playlist restore dispatch"
        );
        assert_eq!(
            runtime.player().position,
            Some(120.0),
            "validation mismatch should still issue corrective seek after playlist restore dispatch"
        );
        assert!(
            !runtime.session().reconnect_state_restore_validation_pending,
            "validation pending should clear after post-restore correction"
        );
        assert_eq!(
            runtime.drain_player_playback_telemetry_updates(),
            vec![
                PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(true)
                    .with_position_seconds(117.5)
            ],
            "telemetry should remain available for diagnostics drains after the ordered restore/playlist/validation sequence"
        );
    }

    #[test]
    fn client_runtime_reconnect_restore_and_validation_notifications_do_not_duplicate_on_repeated_ticks()
     {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
            .expect("local ready should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"file":{"name":"movie.mkv","size":123456789,"duration":95.5}}}}}"#,
            )
            .expect("local file should apply");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("local playlist should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
            .expect("local playlist index should apply");

        session.reset_sync_state_for_reconnect();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("reconnect hello should apply");
        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":120.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("reconnect room playstate should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistChange":{"files":[]}}}"#)
            .expect("empty reconnect playlist snapshot should apply");

        let player = RecordingPlayer {
            pending_playback_telemetry_update: Some(
                PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(true)
                    .with_position_seconds(117.5),
            ),
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_reconnect_state_restore_if_needed()
            .expect("reconnect state restore should dispatch");
        runtime
            .run_reconnect_playlist_restore_if_needed()
            .expect("reconnect playlist restore should dispatch");
        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("reconnect validation should dispatch");

        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::RestoringState,
                ReconnectTransitionNotification::RestoringPlaylist,
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: false,
                    local_position: 117.5,
                    room_position: 120.0,
                    position_diff_seconds: 2.5,
                },
            ],
            "first reconnect cycle ticks should emit restore + playlist + validation notifications once"
        );
        let outbound_messages_after_first_sequence = runtime.control().outbound_messages().len();
        assert_eq!(outbound_messages_after_first_sequence, 4);

        runtime
            .run_reconnect_state_restore_if_needed()
            .expect("repeated state restore tick should be a no-op");
        runtime
            .run_reconnect_playlist_restore_if_needed()
            .expect("repeated playlist restore tick should be a no-op");
        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("repeated validation tick should be a no-op after success");

        assert!(
            runtime.drain_reconnect_notifications().is_empty(),
            "repeated reconnect ticks in the same cycle should not duplicate reconnect notifications"
        );
        assert_eq!(
            runtime.control().outbound_messages().len(),
            outbound_messages_after_first_sequence,
            "repeated reconnect ticks in the same cycle should not enqueue duplicate restore protocol messages"
        );
        assert!(
            !runtime.session().reconnect_state_restore_validation_pending,
            "validation pending should remain cleared after repeated no-op ticks"
        );
    }

    #[test]
    fn client_runtime_reconnect_retry_and_recovery_notifications_preserve_sequence_without_noop_duplicates()
     {
        let mut session = ClientSession::default();
        session.room = Some("room1".to_owned());
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_max_attempts = 1;
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_cooldown_ticks = 1;
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles = 1;
        session.room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(120.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        session.reconnect_state_restore_validation_pending = true;
        session.local_paused = Some(true);
        session.local_position = Some(117.5);

        let player = RecordingPlayer {
            fail_set_position: true,
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("first correction failure should schedule retry");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 117.5,
                    room_position: 120.0,
                    position_diff_seconds: 2.5,
                },
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRetryScheduled {
                    attempt: 1,
                    max_attempts: 1,
                    cooldown_ticks: 1,
                },
            ],
            "first failure should emit mismatch then retry-scheduled"
        );

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("cooldown tick should defer retry");
        assert!(
            runtime.drain_reconnect_notifications().is_empty(),
            "cooldown tick should not duplicate reconnect notifications"
        );

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("second failure should exhaust retry budget and activate recovery cooldown");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRetriesExhausted {
                    attempts: 2,
                    max_attempts: 1,
                },
            ],
            "retry exhaustion should emit only give-up notification without repeating mismatch details"
        );
        assert!(
            !runtime.session().reconnect_state_restore_validation_pending,
            "retry exhaustion should clear pending validation"
        );
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining,
            1
        );

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("repeated no-op tick after exhaustion should not emit notifications");
        assert!(
            runtime.drain_reconnect_notifications().is_empty(),
            "no-op validation tick after exhaustion should not duplicate reconnect notifications"
        );

        runtime.session_mut().room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(130.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        runtime.session_mut().local_paused = Some(true);
        runtime.session_mut().local_position = Some(125.0);
        runtime
            .session_mut()
            .reconnect_state_restore_validation_pending = true;
        runtime
            .session_mut()
            .begin_reconnect_state_restore_validation_cycle();

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("recovery cooldown cycle should suppress correction");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 125.0,
                    room_position: 130.0,
                    position_diff_seconds: 5.0,
                },
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownSuppressed {
                    remaining_reconnect_cycles_after_this_cycle: 0,
                },
            ],
            "suppressed recovery cycle should emit mismatch then suppression notification"
        );
        assert!(
            !runtime.session().reconnect_state_restore_validation_pending,
            "suppressed cycle should clear pending validation"
        );

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("repeated no-op tick after suppressed cycle should not emit notifications");
        assert!(
            runtime.drain_reconnect_notifications().is_empty(),
            "no-op validation tick after suppressed cycle should not duplicate suppression notifications"
        );

        runtime.player_mut().fail_set_position = false;
        runtime.session_mut().room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(140.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        runtime.session_mut().local_paused = Some(true);
        runtime.session_mut().local_position = Some(135.0);
        runtime
            .session_mut()
            .reconnect_state_restore_validation_pending = true;
        runtime
            .session_mut()
            .begin_reconnect_state_restore_validation_cycle();

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("correction should re-enable after recovery cooldown");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownReenabled,
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 135.0,
                    room_position: 140.0,
                    position_diff_seconds: 5.0,
                },
            ],
            "reenabled cycle should emit reenabled notification before mismatch details"
        );
        assert_eq!(runtime.player().position, Some(140.0));

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("repeated no-op tick after reenabled success should not emit notifications");
        assert!(
            runtime.drain_reconnect_notifications().is_empty(),
            "no-op validation tick after reenabled correction should not duplicate reenabled/mismatch notifications"
        );
    }

    #[test]
    fn client_runtime_reconnect_warn_only_on_exhaustion_retry_and_recovery_notifications_follow_policy_specific_sequence()
     {
        let mut session = ClientSession::default();
        session.room = Some("room1".to_owned());
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_policy_mode_override =
            Some(ReconnectStateRestoreCorrectionPolicyMode::WarnOnlyOnExhaustion);
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_max_attempts = 1;
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_cooldown_ticks = 1;
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles = 1;
        session.room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(120.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        session.reconnect_state_restore_validation_pending = true;
        session.local_paused = Some(true);
        session.local_position = Some(117.5);

        let player = RecordingPlayer {
            fail_set_position: true,
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("warn-only-on-exhaustion should attempt correction but suppress early notifications");
        assert!(
            runtime.session().reconnect_state_restore_validation_pending,
            "first failure should leave validation pending for retry"
        );
        assert!(
            runtime.drain_reconnect_notifications().is_empty(),
            "warn-only-on-exhaustion should suppress mismatch and retry-scheduled notifications"
        );

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("cooldown tick should defer retry");
        assert!(
            runtime.drain_reconnect_notifications().is_empty(),
            "cooldown no-op ticks should not emit notifications"
        );

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("second failure should exhaust retry budget");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRetriesExhausted {
                    attempts: 2,
                    max_attempts: 1,
                },
            ],
            "warn-only-on-exhaustion should emit only retries-exhausted on give-up"
        );
        assert!(
            !runtime.session().reconnect_state_restore_validation_pending,
            "retry exhaustion should clear pending validation"
        );

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("post-exhaustion no-op tick should remain silent");
        assert!(
            runtime.drain_reconnect_notifications().is_empty(),
            "post-exhaustion no-op ticks should not duplicate exhaustion warnings"
        );

        runtime.session_mut().room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(130.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        runtime.session_mut().local_paused = Some(true);
        runtime.session_mut().local_position = Some(125.0);
        runtime
            .session_mut()
            .reconnect_state_restore_validation_pending = true;
        runtime
            .session_mut()
            .begin_reconnect_state_restore_validation_cycle();

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("recovery cooldown cycle should suppress correction");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownSuppressed {
                    remaining_reconnect_cycles_after_this_cycle: 0,
                },
            ],
            "warn-only-on-exhaustion should suppress mismatch visibility during recovery cooldown and emit only suppression notice"
        );
        assert_eq!(runtime.player().position, None);
        assert!(!runtime.session().reconnect_state_restore_validation_pending);

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("post-suppression no-op tick should remain silent");
        assert!(
            runtime.drain_reconnect_notifications().is_empty(),
            "post-suppression no-op ticks should not duplicate suppression notices"
        );

        runtime.player_mut().fail_set_position = false;
        runtime.session_mut().room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(140.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        runtime.session_mut().local_paused = Some(true);
        runtime.session_mut().local_position = Some(135.0);
        runtime
            .session_mut()
            .reconnect_state_restore_validation_pending = true;
        runtime
            .session_mut()
            .begin_reconnect_state_restore_validation_cycle();

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("recovery cooldown re-enable cycle should correct");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownReenabled,
            ],
            "warn-only-on-exhaustion should emit only reenabled notification (no mismatch detail) on the recovery re-enable cycle"
        );
        assert_eq!(runtime.player().position, Some(140.0));
        assert!(!runtime.session().reconnect_state_restore_validation_pending);

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("post-success no-op tick should remain silent");
        assert!(
            runtime.drain_reconnect_notifications().is_empty(),
            "post-success no-op ticks should not duplicate reenabled notifications"
        );
    }

    #[test]
    fn client_runtime_reconnect_notify_only_policy_keeps_retry_and_recovery_notifications_suppressed_across_cycles()
     {
        let mut session = ClientSession::default();
        session.room = Some("room1".to_owned());
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_policy_mode_override =
            Some(ReconnectStateRestoreCorrectionPolicyMode::NotifyOnly);
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_max_attempts = 1;
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_cooldown_ticks = 1;
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles = 1;
        session.room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(120.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        session.reconnect_state_restore_validation_pending = true;
        session.local_paused = Some(true);
        session.local_position = Some(117.5);

        let player = RecordingPlayer {
            fail_set_position: true,
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("notify-only validation should emit mismatch without correction");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 117.5,
                    room_position: 120.0,
                    position_diff_seconds: 2.5,
                },
            ],
            "notify-only policy should emit only mismatch details (no retry scheduling)"
        );
        assert!(
            !runtime.session().reconnect_state_restore_validation_pending,
            "notify-only validation should complete in one tick"
        );
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining,
            0,
            "notify-only policy should not activate recovery cooldown state"
        );
        assert_eq!(
            runtime.player().position,
            None,
            "notify-only policy should not attempt corrective seeks even if correction would fail"
        );

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("repeated no-op tick should remain silent");
        assert!(
            runtime.drain_reconnect_notifications().is_empty(),
            "no-op validation ticks should not synthesize retry/recovery notifications in notify-only mode"
        );

        runtime.session_mut().room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(130.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        runtime.session_mut().local_paused = Some(true);
        runtime.session_mut().local_position = Some(125.0);
        runtime
            .session_mut()
            .reconnect_state_restore_validation_pending = true;
        runtime
            .session_mut()
            .begin_reconnect_state_restore_validation_cycle();

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("notify-only should stay mismatch-only across reconnect validation cycles");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 125.0,
                    room_position: 130.0,
                    position_diff_seconds: 5.0,
                },
            ],
            "notify-only policy should remain mismatch-only on later reconnect validation cycles"
        );
        assert!(
            !runtime.session().reconnect_state_restore_validation_pending,
            "notify-only validation should clear pending state on later cycles too"
        );
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining,
            0,
            "notify-only policy should keep recovery cooldown disabled across cycles"
        );
        assert_eq!(
            runtime.player().position,
            None,
            "notify-only policy should not perform corrective seeks on later cycles"
        );

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("post-cycle no-op tick should remain silent");
        assert!(
            runtime.drain_reconnect_notifications().is_empty(),
            "notify-only policy should not emit retry/recovery notifications on repeated no-op ticks across cycles"
        );
    }

    #[test]
    fn client_runtime_reconnect_disable_after_n_mismatches_notifications_follow_sequence_without_noop_duplicates()
     {
        let mut session = ClientSession::default();
        session.room = Some("room1".to_owned());
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_policy_mode_override =
            Some(ReconnectStateRestoreCorrectionPolicyMode::DisableAfterNMismatches);
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_disable_after_mismatch_cycles = 2;
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles = 1;
        session.room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(120.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        session.reconnect_state_restore_validation_pending = true;
        session.local_paused = Some(true);
        session.local_position = Some(117.5);

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("first mismatch cycle should auto-correct before disable threshold");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 117.5,
                    room_position: 120.0,
                    position_diff_seconds: 2.5,
                }
            ],
            "first mismatch cycle should emit mismatch details only"
        );
        assert_eq!(runtime.player().position, Some(120.0));
        assert!(
            !runtime.session().reconnect_state_restore_validation_pending,
            "first cycle should complete validation after correction"
        );

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("no-op tick after first cycle should remain silent");
        assert!(
            runtime.drain_reconnect_notifications().is_empty(),
            "no-op tick after first cycle should not duplicate mismatch notifications"
        );

        runtime.session_mut().room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(130.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        runtime.session_mut().local_paused = Some(true);
        runtime.session_mut().local_position = Some(125.0);
        runtime
            .session_mut()
            .reconnect_state_restore_validation_pending = true;
        runtime
            .session_mut()
            .begin_reconnect_state_restore_validation_cycle();

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("threshold-reaching cycle should disable correction");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![ReconnectTransitionNotification::StateRestoreValidationCorrectionDisabledAfterRepeatedMismatches {
                consecutive_mismatch_cycles: 2,
                disable_after_mismatch_cycles: 2,
            }],
            "threshold cycle should emit only disable-after-repeated-mismatches notification"
        );
        assert_eq!(
            runtime.player().position,
            Some(120.0),
            "disable threshold cycle should not issue a corrective seek"
        );
        assert!(
            !runtime.session().reconnect_state_restore_validation_pending,
            "threshold cycle should clear pending validation"
        );
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining,
            1,
            "disable threshold cycle should activate recovery cooldown"
        );

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("no-op tick after disable notification should remain silent");
        assert!(
            runtime.drain_reconnect_notifications().is_empty(),
            "no-op tick after disable notification should not duplicate notifications"
        );

        runtime.session_mut().room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(140.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        runtime.session_mut().local_paused = Some(true);
        runtime.session_mut().local_position = Some(135.0);
        runtime
            .session_mut()
            .reconnect_state_restore_validation_pending = true;
        runtime
            .session_mut()
            .begin_reconnect_state_restore_validation_cycle();

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("recovery cooldown cycle should suppress correction once");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 135.0,
                    room_position: 140.0,
                    position_diff_seconds: 5.0,
                },
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownSuppressed {
                    remaining_reconnect_cycles_after_this_cycle: 0,
                },
            ],
            "suppressed recovery cycle should emit mismatch then recovery-cooldown-suppressed"
        );
        assert_eq!(
            runtime.player().position,
            Some(120.0),
            "suppressed recovery cycle should not issue a corrective seek"
        );
        assert!(
            !runtime.session().reconnect_state_restore_validation_pending,
            "suppressed recovery cycle should clear pending validation"
        );

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("no-op tick after suppressed recovery cycle should remain silent");
        assert!(
            runtime.drain_reconnect_notifications().is_empty(),
            "no-op tick after suppressed recovery cycle should not duplicate suppression notifications"
        );

        runtime.session_mut().room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(150.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        runtime.session_mut().local_paused = Some(true);
        runtime.session_mut().local_position = Some(145.0);
        runtime
            .session_mut()
            .reconnect_state_restore_validation_pending = true;
        runtime
            .session_mut()
            .begin_reconnect_state_restore_validation_cycle();

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("re-enabled cycle should resume correction after cooldown");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownReenabled,
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 145.0,
                    room_position: 150.0,
                    position_diff_seconds: 5.0,
                },
            ],
            "re-enabled cycle should emit recovery-cooldown-reenabled before mismatch details"
        );
        assert_eq!(runtime.player().position, Some(150.0));
        assert!(
            !runtime.session().reconnect_state_restore_validation_pending,
            "re-enabled correction cycle should clear pending validation"
        );

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("no-op tick after re-enabled cycle should remain silent");
        assert!(
            runtime.drain_reconnect_notifications().is_empty(),
            "no-op tick after re-enabled cycle should not duplicate reenabled or mismatch notifications"
        );
    }

    #[test]
    fn client_runtime_reconnect_state_restore_validation_emits_mismatch_notification_and_corrects()
    {
        let mut session = ClientSession::default();
        session.room = Some("room1".to_owned());
        session.room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(120.0),
                paused: Some(false),
                ..RoomPlaystateView::default()
            },
        );
        session.reconnect_state_restore_validation_pending = true;

        let player = RecordingPlayer {
            pending_playback_telemetry_update: Some(
                PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(true)
                    .with_position_seconds(117.5),
            ),
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("reconnect state restore telemetry validation should not fail");

        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: false,
                    local_position: 117.5,
                    room_position: 120.0,
                    position_diff_seconds: 2.5,
                }
            ]
        );
        assert_eq!(
            runtime.player().paused,
            Some(false),
            "validation mismatch policy should issue a corrective pause command toward room state"
        );
        assert_eq!(
            runtime.player().position,
            Some(120.0),
            "validation mismatch policy should issue a corrective seek toward room state"
        );
        assert_eq!(
            runtime.session().local_paused,
            Some(false),
            "session local pause state should be updated to the corrective target"
        );
        assert_eq!(
            runtime.session().local_position,
            Some(120.0),
            "session local position should be updated to the corrective target"
        );
        assert_eq!(
            runtime.drain_player_playback_telemetry_updates(),
            vec![
                PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(true)
                    .with_position_seconds(117.5)
            ],
            "validation should preserve telemetry updates for later diagnostics drains"
        );
        assert!(runtime.drain_reconnect_notifications().is_empty());
    }

    #[test]
    fn client_runtime_reconnect_state_restore_validation_waits_for_complete_state() {
        let mut session = ClientSession::default();
        session.room = Some("room1".to_owned());
        session.room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(120.0),
                paused: Some(false),
                ..RoomPlaystateView::default()
            },
        );
        session.reconnect_state_restore_validation_pending = true;

        let player = RecordingPlayer {
            pending_playback_telemetry_update: Some(
                PlayerPlaybackTelemetryUpdate::default().with_paused(false),
            ),
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("validation should wait when telemetry is incomplete");

        assert!(
            runtime.drain_reconnect_notifications().is_empty(),
            "no reconnect validation notification should emit until position is known"
        );
        assert_eq!(
            runtime.drain_player_playback_telemetry_updates(),
            vec![PlayerPlaybackTelemetryUpdate::default().with_paused(false)]
        );
        assert!(
            runtime.session().reconnect_state_restore_validation_pending,
            "pending validation should remain set until complete local/global playstate is available"
        );
    }

    #[test]
    fn client_runtime_reconnect_state_restore_validation_handles_telemetry_before_room_state() {
        let mut session = ClientSession::default();
        session.room = Some("room1".to_owned());
        session.reconnect_state_restore_validation_pending = true;

        let player = RecordingPlayer {
            pending_playback_telemetry_update: Some(
                PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(true)
                    .with_position_seconds(117.5),
            ),
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("validation should wait when room playstate is not yet available");

        assert!(
            runtime.drain_reconnect_notifications().is_empty(),
            "no reconnect validation notifications should emit before room playstate arrives"
        );
        assert_eq!(
            runtime.session().local_paused,
            Some(true),
            "telemetry should still pre-sync into session local pause state while waiting"
        );
        assert_eq!(
            runtime.session().local_position,
            Some(117.5),
            "telemetry should still pre-sync into session local position while waiting"
        );
        assert!(
            runtime.session().reconnect_state_restore_validation_pending,
            "pending validation should remain set until room playstate arrives"
        );
        assert_eq!(
            runtime.drain_player_playback_telemetry_updates(),
            vec![
                PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(true)
                    .with_position_seconds(117.5)
            ],
            "telemetry should remain available for diagnostics drains while validation is pending"
        );

        runtime.session_mut().room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(120.0),
                paused: Some(false),
                ..RoomPlaystateView::default()
            },
        );

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("validation should complete once room playstate arrives");

        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: false,
                    local_position: 117.5,
                    room_position: 120.0,
                    position_diff_seconds: 2.5,
                }
            ],
            "delayed room state arrival should trigger validation using the cached telemetry-refreshed local state"
        );
        assert_eq!(runtime.player().paused, Some(false));
        assert_eq!(runtime.player().position, Some(120.0));
        assert!(
            !runtime.session().reconnect_state_restore_validation_pending,
            "pending validation should clear after delayed room-state validation succeeds"
        );
        assert!(
            runtime.drain_player_playback_telemetry_updates().is_empty(),
            "no new telemetry should be required after room state arrival"
        );
    }

    #[test]
    fn client_runtime_reconnect_state_restore_validation_handles_room_state_before_telemetry() {
        let mut session = ClientSession::default();
        session.room = Some("room1".to_owned());
        session.room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(120.0),
                paused: Some(false),
                ..RoomPlaystateView::default()
            },
        );
        session.reconnect_state_restore_validation_pending = true;

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("validation should wait when telemetry is not yet available");

        assert!(
            runtime.drain_reconnect_notifications().is_empty(),
            "no reconnect validation notifications should emit before local telemetry arrives"
        );
        assert_eq!(
            runtime.session().local_paused,
            None,
            "session local pause should remain unknown while waiting for telemetry"
        );
        assert_eq!(
            runtime.session().local_position,
            None,
            "session local position should remain unknown while waiting for telemetry"
        );
        assert!(
            runtime.session().reconnect_state_restore_validation_pending,
            "pending validation should remain set until telemetry arrives"
        );
        assert!(
            runtime.drain_player_playback_telemetry_updates().is_empty(),
            "no telemetry should be buffered before the player reports any updates"
        );

        runtime.player_mut().pending_playback_telemetry_update = Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_paused(true)
                .with_position_seconds(117.5),
        );

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("validation should complete once telemetry arrives");

        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: false,
                    local_position: 117.5,
                    room_position: 120.0,
                    position_diff_seconds: 2.5,
                }
            ],
            "delayed telemetry arrival should trigger validation using the cached room playstate"
        );
        assert_eq!(
            runtime.player().paused,
            Some(false),
            "validation should issue a corrective pause command once telemetry arrives"
        );
        assert_eq!(
            runtime.player().position,
            Some(120.0),
            "validation should issue a corrective seek once telemetry arrives"
        );
        assert!(
            !runtime.session().reconnect_state_restore_validation_pending,
            "pending validation should clear after delayed-telemetry validation succeeds"
        );
        assert_eq!(
            runtime.drain_player_playback_telemetry_updates(),
            vec![
                PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(true)
                    .with_position_seconds(117.5)
            ],
            "telemetry should remain available for diagnostics drains after delayed-telemetry validation"
        );
    }

    #[test]
    fn client_runtime_reconnect_state_restore_validation_notify_only_mode_skips_correction() {
        let mut session = ClientSession::default();
        session.room = Some("room1".to_owned());
        session
            .behavior_config_mut()
            .reconnect_state_restore_auto_correct = false;
        session.room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(120.0),
                paused: Some(false),
                ..RoomPlaystateView::default()
            },
        );
        session.reconnect_state_restore_validation_pending = true;

        let player = RecordingPlayer {
            pending_playback_telemetry_update: Some(
                PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(true)
                    .with_position_seconds(117.5),
            ),
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("notify-only validation should not fail");

        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: false,
                    local_position: 117.5,
                    room_position: 120.0,
                    position_diff_seconds: 2.5,
                }
            ]
        );
        assert_eq!(runtime.player().paused, None);
        assert_eq!(runtime.player().position, None);
        assert_eq!(
            runtime.session().local_paused,
            Some(true),
            "notify-only mode should leave telemetry-refreshed local state unchanged"
        );
        assert_eq!(runtime.session().local_position, Some(117.5));
    }

    #[test]
    fn client_runtime_reconnect_state_restore_validation_honors_custom_position_tolerance() {
        let mut session = ClientSession::default();
        session.room = Some("room1".to_owned());
        session
            .behavior_config_mut()
            .reconnect_state_restore_position_tolerance_seconds = 3.0;
        session.room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(120.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        session.reconnect_state_restore_validation_pending = true;

        let player = RecordingPlayer {
            pending_playback_telemetry_update: Some(
                PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(true)
                    .with_position_seconds(117.5),
            ),
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("validation should respect custom tolerance");

        assert!(
            runtime.drain_reconnect_notifications().is_empty(),
            "2.5s diff should be tolerated when reconnect correction tolerance is 3.0s"
        );
        assert_eq!(runtime.player().paused, None);
        assert_eq!(runtime.player().position, None);
        assert!(!runtime.session().reconnect_state_restore_validation_pending);
    }

    #[test]
    fn client_runtime_reconnect_state_restore_validation_retries_correction_after_failure() {
        let mut session = ClientSession::default();
        session.room = Some("room1".to_owned());
        session.room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(120.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        session.reconnect_state_restore_validation_pending = true;

        let player = RecordingPlayer {
            fail_set_position: true,
            pending_playback_telemetry_update: Some(
                PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(true)
                    .with_position_seconds(117.5),
            ),
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("transient correction failure should be swallowed and retried later");

        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 117.5,
                    room_position: 120.0,
                    position_diff_seconds: 2.5,
                },
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRetryScheduled {
                    attempt: 1,
                    max_attempts: 3,
                    cooldown_ticks: 1,
                },
            ],
            "mismatch and retry-scheduled notifications should emit on the first correction failure"
        );
        assert_eq!(runtime.player().position, None);
        assert!(runtime.session().reconnect_state_restore_validation_pending);
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_validation_retry_attempts,
            1
        );
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_validation_retry_cooldown_ticks,
            1
        );

        runtime.player_mut().fail_set_position = false;

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("first retry cycle should be throttled");
        assert_eq!(
            runtime.player().position,
            None,
            "cooldown should defer retry by one validation invocation"
        );
        assert!(runtime.drain_reconnect_notifications().is_empty());
        assert!(runtime.session().reconnect_state_restore_validation_pending);

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("retry after cooldown should succeed");
        assert_eq!(runtime.player().position, Some(120.0));
        assert!(!runtime.session().reconnect_state_restore_validation_pending);
        assert!(runtime.drain_reconnect_notifications().is_empty());
        assert_eq!(
            runtime.drain_player_playback_telemetry_updates(),
            vec![
                PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(true)
                    .with_position_seconds(117.5)
            ],
            "telemetry should remain available for later diagnostics despite retry handling"
        );
    }

    #[test]
    fn client_runtime_reconnect_state_restore_validation_honors_retry_budget() {
        let mut session = ClientSession::default();
        session.room = Some("room1".to_owned());
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_max_attempts = 1;
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_cooldown_ticks = 0;
        session.room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(120.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        session.reconnect_state_restore_validation_pending = true;

        let player = RecordingPlayer {
            fail_set_position: true,
            pending_playback_telemetry_update: Some(
                PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(true)
                    .with_position_seconds(117.5),
            ),
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("first correction failure should schedule retry within configured budget");
        assert!(runtime.session().reconnect_state_restore_validation_pending);
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_validation_retry_attempts,
            1
        );
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 117.5,
                    room_position: 120.0,
                    position_diff_seconds: 2.5,
                },
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRetryScheduled {
                    attempt: 1,
                    max_attempts: 1,
                    cooldown_ticks: 0,
                },
            ]
        );

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("second failure should give up after configured retry budget");

        assert!(
            !runtime.session().reconnect_state_restore_validation_pending,
            "retry budget should clear pending validation after a repeated correction failure"
        );
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRetriesExhausted {
                    attempts: 2,
                    max_attempts: 1,
                }
            ],
            "retry exhaustion should emit a give-up notification without duplicating mismatch details"
        );
        assert_eq!(
            runtime.player().position,
            None,
            "failed correction should not claim success before retry budget is exhausted"
        );
    }

    #[test]
    fn client_runtime_reconnect_state_restore_validation_honors_custom_retry_cooldown() {
        let mut session = ClientSession::default();
        session.room = Some("room1".to_owned());
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_max_attempts = 2;
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_cooldown_ticks = 2;
        session.room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(120.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        session.reconnect_state_restore_validation_pending = true;

        let player = RecordingPlayer {
            fail_set_position: true,
            pending_playback_telemetry_update: Some(
                PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(true)
                    .with_position_seconds(117.5),
            ),
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("first correction failure should be swallowed");
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_validation_retry_cooldown_ticks,
            2
        );
        runtime.player_mut().fail_set_position = false;

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("first cooldown tick should defer retry");
        assert_eq!(runtime.player().position, None);
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_validation_retry_cooldown_ticks,
            1
        );

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("second cooldown tick should still defer retry");
        assert_eq!(runtime.player().position, None);
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_validation_retry_cooldown_ticks,
            0
        );

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("retry should run after configured cooldown expires");
        assert_eq!(runtime.player().position, Some(120.0));
        assert!(!runtime.session().reconnect_state_restore_validation_pending);
    }

    #[test]
    fn client_runtime_reconnect_state_restore_validation_honors_exponential_retry_backoff_cap() {
        let mut session = ClientSession::default();
        session.room = Some("room1".to_owned());
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_max_attempts = 3;
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_cooldown_ticks = 1;
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_exponential_backoff = true;
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_max_cooldown_ticks = 2;
        session.room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(120.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        session.reconnect_state_restore_validation_pending = true;

        let player = RecordingPlayer {
            fail_set_position: true,
            pending_playback_telemetry_update: Some(
                PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(true)
                    .with_position_seconds(117.5),
            ),
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("first failure should schedule retry with base cooldown");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 117.5,
                    room_position: 120.0,
                    position_diff_seconds: 2.5,
                },
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRetryScheduled {
                    attempt: 1,
                    max_attempts: 3,
                    cooldown_ticks: 1,
                },
            ]
        );
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_validation_retry_cooldown_ticks,
            1
        );

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("cooldown tick should defer second retry attempt");
        assert!(runtime.drain_reconnect_notifications().is_empty());
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_validation_retry_cooldown_ticks,
            0
        );

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("second failure should use exponential cooldown");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRetryScheduled {
                    attempt: 2,
                    max_attempts: 3,
                    cooldown_ticks: 2,
                },
            ]
        );
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_validation_retry_cooldown_ticks,
            2
        );

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("first cooldown tick after second failure should defer retry");
        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("second cooldown tick after second failure should defer retry");
        assert!(runtime.drain_reconnect_notifications().is_empty());
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_validation_retry_cooldown_ticks,
            0
        );

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("third failure should apply capped exponential cooldown");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRetryScheduled {
                    attempt: 3,
                    max_attempts: 3,
                    cooldown_ticks: 2,
                },
            ],
            "third exponential cooldown would be 4 ticks but should clamp to configured max"
        );
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_validation_retry_cooldown_ticks,
            2
        );
    }

    #[test]
    fn client_runtime_reconnect_state_restore_validation_adaptive_retry_backoff_scales_after_prior_exhaustion()
     {
        let mut session = ClientSession::default();
        session.room = Some("room1".to_owned());
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_max_attempts = 0;
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_cooldown_ticks = 1;
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_adaptive_cycle_backoff = true;
        session.room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(120.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        session.reconnect_state_restore_validation_pending = true;

        let player = RecordingPlayer {
            fail_set_position: true,
            pending_playback_telemetry_update: Some(
                PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(true)
                    .with_position_seconds(117.5),
            ),
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("first correction failure should exhaust retry budget");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 117.5,
                    room_position: 120.0,
                    position_diff_seconds: 2.5,
                },
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRetriesExhausted {
                    attempts: 1,
                    max_attempts: 0,
                },
            ]
        );
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_correction_consecutive_retry_exhaustions,
            1
        );

        runtime
            .session_mut()
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_max_attempts = 1;
        runtime
            .session_mut()
            .reconnect_state_restore_validation_pending = true;
        runtime.session_mut().local_paused = Some(true);
        runtime.session_mut().local_position = Some(117.5);

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("adaptive retry backoff should schedule a retry in the next restore cycle");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 117.5,
                    room_position: 120.0,
                    position_diff_seconds: 2.5,
                },
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRetryScheduled {
                    attempt: 1,
                    max_attempts: 1,
                    cooldown_ticks: 2,
                },
            ],
            "one prior retry-exhausted restore cycle should double the first retry cooldown when adaptive cycle backoff is enabled"
        );
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_validation_retry_cooldown_ticks,
            2
        );
    }

    #[test]
    fn client_runtime_reconnect_state_restore_validation_adaptive_retry_budget_reduces_after_prior_exhaustion()
     {
        let mut session = ClientSession::default();
        session.room = Some("room1".to_owned());
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_max_attempts = 0;
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_cooldown_ticks = 0;
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_adaptive_cycle_budget = true;
        session.room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(120.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        session.reconnect_state_restore_validation_pending = true;

        let player = RecordingPlayer {
            fail_set_position: true,
            pending_playback_telemetry_update: Some(
                PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(true)
                    .with_position_seconds(117.5),
            ),
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("first correction failure should exhaust zero retry budget");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 117.5,
                    room_position: 120.0,
                    position_diff_seconds: 2.5,
                },
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRetriesExhausted {
                    attempts: 1,
                    max_attempts: 0,
                },
            ]
        );
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_correction_consecutive_retry_exhaustions,
            1
        );

        runtime
            .session_mut()
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_max_attempts = 2;
        runtime
            .session_mut()
            .reconnect_state_restore_validation_pending = true;
        runtime.session_mut().local_paused = Some(true);
        runtime.session_mut().local_position = Some(117.5);

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("second restore cycle should use reduced adaptive retry budget");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 117.5,
                    room_position: 120.0,
                    position_diff_seconds: 2.5,
                },
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRetryScheduled {
                    attempt: 1,
                    max_attempts: 1,
                    cooldown_ticks: 0,
                },
            ],
            "one prior retry-exhausted restore cycle should reduce the effective retry budget by one attempt when adaptive budget is enabled"
        );

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("second failure in reduced budget cycle should exhaust retries");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRetriesExhausted {
                    attempts: 2,
                    max_attempts: 1,
                },
            ]
        );
    }

    #[test]
    fn client_runtime_reconnect_state_restore_validation_adaptive_retry_budget_honors_min_attempt_floor()
     {
        let mut session = ClientSession::default();
        session.room = Some("room1".to_owned());
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_max_attempts = 3;
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_cooldown_ticks = 0;
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_adaptive_cycle_budget = true;
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_adaptive_cycle_budget_min_attempts = 2;
        session.reconnect_state_restore_correction_consecutive_retry_exhaustions = 5;
        session.room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(120.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        session.reconnect_state_restore_validation_pending = true;

        let player = RecordingPlayer {
            fail_set_position: true,
            pending_playback_telemetry_update: Some(
                PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(true)
                    .with_position_seconds(117.5),
            ),
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("adaptive retry budget floor should still allow retries");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 117.5,
                    room_position: 120.0,
                    position_diff_seconds: 2.5,
                },
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRetryScheduled {
                    attempt: 1,
                    max_attempts: 2,
                    cooldown_ticks: 0,
                },
            ],
            "adaptive retry budget floor should cap reductions so the effective retry budget does not fall below the configured minimum"
        );
    }

    #[test]
    fn client_runtime_reconnect_state_restore_validation_recovery_cooldown_suppresses_cycle_after_retry_exhaustion_then_reenables()
     {
        let mut session = ClientSession::default();
        session.room = Some("room1".to_owned());
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_max_attempts = 0;
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_cooldown_ticks = 0;
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles = 1;
        session.room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(120.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        session.reconnect_state_restore_validation_pending = true;
        session.local_paused = Some(true);
        session.local_position = Some(117.5);

        let player = RecordingPlayer {
            fail_set_position: true,
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("first reconnect correction failure should exhaust retries");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 117.5,
                    room_position: 120.0,
                    position_diff_seconds: 2.5,
                },
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRetriesExhausted {
                    attempts: 1,
                    max_attempts: 0,
                },
            ]
        );
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining,
            1
        );

        runtime.session_mut().room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(130.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        runtime.session_mut().local_paused = Some(true);
        runtime.session_mut().local_position = Some(125.0);
        runtime
            .session_mut()
            .reconnect_state_restore_validation_pending = true;
        runtime
            .session_mut()
            .begin_reconnect_state_restore_validation_cycle();

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("recovery cooldown should suppress correction for one reconnect cycle");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 125.0,
                    room_position: 130.0,
                    position_diff_seconds: 5.0,
                },
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownSuppressed {
                    remaining_reconnect_cycles_after_this_cycle: 0,
                },
            ],
            "suppressed recovery cycle should emit mismatch visibility but skip corrective actions"
        );
        assert_eq!(runtime.player().position, None);
        assert!(!runtime.session().reconnect_state_restore_validation_pending);
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining,
            0
        );

        runtime.player_mut().fail_set_position = false;
        runtime.session_mut().room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(140.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        runtime.session_mut().local_paused = Some(true);
        runtime.session_mut().local_position = Some(135.0);
        runtime
            .session_mut()
            .reconnect_state_restore_validation_pending = true;
        runtime
            .session_mut()
            .begin_reconnect_state_restore_validation_cycle();

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("correction should re-enable after recovery cooldown cycle completes");
        assert_eq!(runtime.player().position, Some(140.0));
        assert!(!runtime.session().reconnect_state_restore_validation_pending);
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownReenabled,
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 135.0,
                    room_position: 140.0,
                    position_diff_seconds: 5.0,
                },
            ]
        );
    }

    #[test]
    fn client_runtime_reconnect_state_restore_validation_success_resets_adaptive_retry_backoff_history()
     {
        let mut session = ClientSession::default();
        session.room = Some("room1".to_owned());
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_cooldown_ticks = 1;
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_adaptive_cycle_backoff = true;
        session.room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(120.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        session.reconnect_state_restore_validation_pending = true;
        session.reconnect_state_restore_correction_consecutive_retry_exhaustions = 2;

        let player = RecordingPlayer {
            pending_playback_telemetry_update: Some(
                PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(true)
                    .with_position_seconds(117.5),
            ),
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("successful correction should complete reconnect validation");

        assert!(!runtime.session().reconnect_state_restore_validation_pending);
        assert_eq!(runtime.player().position, Some(120.0));
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_correction_consecutive_retry_exhaustions,
            0,
            "adaptive retry backoff history should reset after a successful correction"
        );
    }

    #[test]
    fn client_runtime_reconnect_state_restore_validation_warn_only_on_exhaustion_suppresses_early_notifications()
     {
        let mut session = ClientSession::default();
        session.room = Some("room1".to_owned());
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_policy_mode_override =
            Some(ReconnectStateRestoreCorrectionPolicyMode::WarnOnlyOnExhaustion);
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_max_attempts = 1;
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_cooldown_ticks = 0;
        session.room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(120.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        session.reconnect_state_restore_validation_pending = true;

        let player = RecordingPlayer {
            fail_set_position: true,
            pending_playback_telemetry_update: Some(
                PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(true)
                    .with_position_seconds(117.5),
            ),
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("warn-only-on-exhaustion policy should still attempt correction");

        assert!(runtime.session().reconnect_state_restore_validation_pending);
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_validation_retry_attempts,
            1
        );
        assert!(
            runtime.drain_reconnect_notifications().is_empty(),
            "warn-only-on-exhaustion should suppress mismatch and retry notifications before exhaustion"
        );

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("second failure should exhaust retry budget and emit a single warning");

        assert!(!runtime.session().reconnect_state_restore_validation_pending);
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRetriesExhausted {
                    attempts: 2,
                    max_attempts: 1,
                }
            ]
        );
    }

    #[test]
    fn client_runtime_reconnect_state_restore_validation_disable_after_n_mismatches_stops_correction_on_threshold()
     {
        let mut session = ClientSession::default();
        session.room = Some("room1".to_owned());
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_policy_mode_override =
            Some(ReconnectStateRestoreCorrectionPolicyMode::DisableAfterNMismatches);
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_disable_after_mismatch_cycles = 2;
        session.room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(120.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        session.reconnect_state_restore_validation_pending = true;

        let player = RecordingPlayer {
            pending_playback_telemetry_update: Some(
                PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(true)
                    .with_position_seconds(117.5),
            ),
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("first mismatch cycle should still auto-correct before threshold");
        assert_eq!(runtime.player().position, Some(120.0));
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 117.5,
                    room_position: 120.0,
                    position_diff_seconds: 2.5,
                }
            ]
        );
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_correction_consecutive_mismatch_cycles,
            1
        );

        runtime.session_mut().room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(130.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        runtime
            .session_mut()
            .reconnect_state_restore_validation_pending = true;
        runtime.player_mut().pending_playback_telemetry_update = Some(
            PlayerPlaybackTelemetryUpdate::default()
                .with_paused(true)
                .with_position_seconds(125.0),
        );

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect(
                "threshold-reaching mismatch cycle should disable correction instead of correcting",
            );

        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationCorrectionDisabledAfterRepeatedMismatches {
                    consecutive_mismatch_cycles: 2,
                    disable_after_mismatch_cycles: 2,
                }
            ]
        );
        assert_eq!(
            runtime.player().position,
            Some(120.0),
            "no corrective seek should be issued once disable-after-N-mismatches threshold is reached"
        );
        assert!(!runtime.session().reconnect_state_restore_validation_pending);
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_correction_consecutive_mismatch_cycles,
            2
        );
    }

    #[test]
    fn client_runtime_reconnect_state_restore_validation_disable_after_n_mismatches_decays_counter_after_successful_correction_when_configured()
     {
        let mut session = ClientSession::default();
        session.room = Some("room1".to_owned());
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_policy_mode_override =
            Some(ReconnectStateRestoreCorrectionPolicyMode::DisableAfterNMismatches);
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_disable_after_mismatch_cycles = 2;
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_disable_after_mismatch_decay_on_success = 1;
        session.room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(120.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        session.local_paused = Some(true);
        session.local_position = Some(117.5);
        session.reconnect_state_restore_validation_pending = true;

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("first restore mismatch should be corrected");
        assert_eq!(
            runtime.player().position,
            Some(120.0),
            "first reconnect mismatch should still auto-correct"
        );
        assert!(!runtime.session().reconnect_state_restore_validation_pending);
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 117.5,
                    room_position: 120.0,
                    position_diff_seconds: 2.5,
                }
            ]
        );
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_correction_consecutive_mismatch_cycles,
            0,
            "configured decay-on-success should recover the repeated-mismatch counter after successful correction"
        );

        runtime.session_mut().room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(130.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        runtime.session_mut().local_paused = Some(true);
        runtime.session_mut().local_position = Some(125.0);
        runtime
            .session_mut()
            .reconnect_state_restore_validation_pending = true;

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect(
                "second restore mismatch should still correct instead of hitting disable threshold",
            );
        assert_eq!(
            runtime.player().position,
            Some(130.0),
            "decay-on-success should prevent threshold accumulation across successful correction cycles"
        );
        assert!(!runtime.session().reconnect_state_restore_validation_pending);
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 125.0,
                    room_position: 130.0,
                    position_diff_seconds: 5.0,
                }
            ]
        );
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_correction_consecutive_mismatch_cycles,
            0
        );
    }

    #[test]
    fn client_runtime_reconnect_state_restore_validation_disable_after_n_mismatches_recovery_cooldown_reenables_correction()
     {
        let mut session = ClientSession::default();
        session.room = Some("room1".to_owned());
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_policy_mode_override =
            Some(ReconnectStateRestoreCorrectionPolicyMode::DisableAfterNMismatches);
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_disable_after_mismatch_cycles = 2;
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles = 1;
        session.room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(120.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        session.reconnect_state_restore_validation_pending = true;
        session.local_paused = Some(true);
        session.local_position = Some(117.5);

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("first mismatch should correct and increment mismatch-cycle counter");
        assert_eq!(runtime.player().position, Some(120.0));
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_correction_consecutive_mismatch_cycles,
            1
        );
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 117.5,
                    room_position: 120.0,
                    position_diff_seconds: 2.5,
                }
            ]
        );

        runtime.session_mut().room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(130.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        runtime.session_mut().local_paused = Some(true);
        runtime.session_mut().local_position = Some(125.0);
        runtime
            .session_mut()
            .reconnect_state_restore_validation_pending = true;
        runtime
            .session_mut()
            .begin_reconnect_state_restore_validation_cycle();

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("second mismatch should trigger disable-after-N and start recovery cooldown");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![ReconnectTransitionNotification::StateRestoreValidationCorrectionDisabledAfterRepeatedMismatches {
                consecutive_mismatch_cycles: 2,
                disable_after_mismatch_cycles: 2,
            }]
        );
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining,
            1
        );
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_correction_consecutive_mismatch_cycles,
            0,
            "disable path should reset mismatch-cycle counter when recovery cooldown is activated"
        );
        assert_eq!(runtime.player().position, Some(120.0));

        runtime.session_mut().room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(140.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        runtime.session_mut().local_paused = Some(true);
        runtime.session_mut().local_position = Some(135.0);
        runtime
            .session_mut()
            .reconnect_state_restore_validation_pending = true;
        runtime
            .session_mut()
            .begin_reconnect_state_restore_validation_cycle();

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("recovery cooldown cycle should suppress correction");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 135.0,
                    room_position: 140.0,
                    position_diff_seconds: 5.0,
                },
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownSuppressed {
                    remaining_reconnect_cycles_after_this_cycle: 0,
                },
            ]
        );
        assert_eq!(runtime.player().position, Some(120.0));
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining,
            0
        );
        assert_eq!(
            runtime
                .session()
                .reconnect_state_restore_correction_consecutive_mismatch_cycles,
            0
        );

        runtime.session_mut().room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(150.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        runtime.session_mut().local_paused = Some(true);
        runtime.session_mut().local_position = Some(145.0);
        runtime
            .session_mut()
            .reconnect_state_restore_validation_pending = true;
        runtime
            .session_mut()
            .begin_reconnect_state_restore_validation_cycle();

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("correction should re-enable after recovery cooldown cycle");
        assert_eq!(runtime.player().position, Some(150.0));
        assert!(!runtime.session().reconnect_state_restore_validation_pending);
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownReenabled,
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 145.0,
                    room_position: 150.0,
                    position_diff_seconds: 5.0,
                },
            ]
        );
    }

    #[test]
    fn reconnect_state_restore_correction_state_snapshot_reports_effective_policy_and_retry_budget()
    {
        let mut session = ClientSession::default();
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_policy_mode_override =
            Some(ReconnectStateRestoreCorrectionPolicyMode::DisableAfterNMismatches);
        session
            .behavior_config_mut()
            .reconnect_state_restore_position_tolerance_seconds = -5.0;
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_max_attempts = 5;
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_adaptive_cycle_budget = true;
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_adaptive_cycle_budget_min_attempts = 2;
        session.reconnect_state_restore_validation_pending = true;
        session.reconnect_state_restore_validation_retry_attempts = 2;
        session.reconnect_state_restore_validation_retry_cooldown_ticks = 4;
        session.reconnect_state_restore_validation_mismatch_notified = true;
        session.reconnect_state_restore_validation_mismatch_seen_in_cycle = true;
        session.reconnect_state_restore_correction_consecutive_mismatch_cycles = 3;
        session.reconnect_state_restore_correction_consecutive_retry_exhaustions = 4;
        session.reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles_remaining = 2;
        session.reconnect_state_restore_correction_recovery_suppressed_this_cycle = true;
        session.reconnect_state_restore_correction_recovery_reenabled_this_cycle = false;

        let snapshot = session.reconnect_state_restore_correction_state_snapshot();
        assert!(snapshot.validation_pending);
        assert_eq!(snapshot.retry_attempts, 2);
        assert_eq!(snapshot.retry_cooldown_ticks, 4);
        assert!(snapshot.mismatch_notified_in_cycle);
        assert!(snapshot.mismatch_seen_in_cycle);
        assert_eq!(
            snapshot.effective_policy_mode,
            ReconnectStateRestoreCorrectionPolicyMode::DisableAfterNMismatches
        );
        assert_eq!(
            snapshot.position_tolerance_seconds, SEEK_THRESHOLD_SECONDS,
            "invalid tolerance should normalize to the default seek threshold in snapshots"
        );
        assert_eq!(
            snapshot.effective_retry_max_attempts, 2,
            "adaptive retry budget snapshot should respect the configured minimum floor"
        );
        assert_eq!(snapshot.consecutive_mismatch_cycles, 3);
        assert_eq!(snapshot.consecutive_retry_exhaustions, 4);
        assert_eq!(snapshot.recovery_cooldown_reconnect_cycles_remaining, 2);
        assert!(snapshot.correction_suppressed_for_recovery_cycle);
        assert!(!snapshot.correction_reenabled_for_recovery_cycle);

        assert_eq!(
            *session.reconnect_state_restore_correction_metrics(),
            ReconnectStateRestoreCorrectionMetrics::default(),
            "metrics should start at zero until reconnect validation cycles execute"
        );
    }

    #[test]
    fn client_runtime_reconnect_state_restore_validation_metrics_and_state_snapshot_track_retry_and_recovery_progress()
     {
        let mut session = ClientSession::default();
        session.room = Some("room1".to_owned());
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_max_attempts = 0;
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_retry_cooldown_ticks = 0;
        session
            .behavior_config_mut()
            .reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles = 1;
        session.room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(120.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        session.local_paused = Some(true);
        session.local_position = Some(117.5);
        session.reconnect_state_restore_validation_pending = true;
        session.begin_reconnect_state_restore_validation_cycle();

        let player = RecordingPlayer {
            fail_set_position: true,
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("first failure should exhaust zero retry budget and start recovery cooldown");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 117.5,
                    room_position: 120.0,
                    position_diff_seconds: 2.5,
                },
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRetriesExhausted {
                    attempts: 1,
                    max_attempts: 0,
                },
            ]
        );
        let snapshot_after_exhaustion = runtime.reconnect_state_restore_correction_state_snapshot();
        assert!(!snapshot_after_exhaustion.validation_pending);
        assert_eq!(
            snapshot_after_exhaustion.recovery_cooldown_reconnect_cycles_remaining,
            1
        );
        assert_eq!(snapshot_after_exhaustion.consecutive_retry_exhaustions, 1);

        runtime.session_mut().room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(130.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        runtime.session_mut().local_paused = Some(true);
        runtime.session_mut().local_position = Some(125.0);
        runtime
            .session_mut()
            .reconnect_state_restore_validation_pending = true;
        runtime
            .session_mut()
            .begin_reconnect_state_restore_validation_cycle();
        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("recovery cooldown cycle should suppress correction");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 125.0,
                    room_position: 130.0,
                    position_diff_seconds: 5.0,
                },
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownSuppressed {
                    remaining_reconnect_cycles_after_this_cycle: 0,
                },
            ]
        );

        runtime.player_mut().fail_set_position = false;
        runtime.session_mut().room_playstates.insert(
            "room1".to_owned(),
            RoomPlaystateView {
                position: Some(140.0),
                paused: Some(true),
                ..RoomPlaystateView::default()
            },
        );
        runtime.session_mut().local_paused = Some(true);
        runtime.session_mut().local_position = Some(135.0);
        runtime
            .session_mut()
            .reconnect_state_restore_validation_pending = true;
        runtime
            .session_mut()
            .begin_reconnect_state_restore_validation_cycle();
        runtime
            .run_reconnect_state_restore_validation_if_needed()
            .expect("correction should re-enable and succeed after recovery cooldown");
        assert_eq!(runtime.player().position, Some(140.0));
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![
                ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownReenabled,
                ReconnectTransitionNotification::StateRestoreValidationMismatch {
                    local_paused: true,
                    room_paused: true,
                    local_position: 135.0,
                    room_position: 140.0,
                    position_diff_seconds: 5.0,
                },
            ]
        );

        let metrics = *runtime.reconnect_state_restore_correction_metrics();
        assert_eq!(metrics.validation_cycles_started, 3);
        assert_eq!(metrics.validation_cycles_completed_without_mismatch, 0);
        assert_eq!(
            metrics.validation_cycles_completed_with_successful_correction,
            1
        );
        assert_eq!(metrics.mismatch_cycles_detected, 2);
        assert_eq!(metrics.mismatch_notifications_emitted, 3);
        assert_eq!(metrics.correction_actions_attempted, 2);
        assert_eq!(metrics.correction_actions_succeeded, 1);
        assert_eq!(metrics.correction_action_failures, 1);
        assert_eq!(metrics.correction_retries_scheduled, 0);
        assert_eq!(metrics.correction_retry_exhaustions, 1);
        assert_eq!(metrics.correction_disables_after_repeated_mismatches, 0);
        assert_eq!(metrics.correction_recovery_cooldown_suppressed_cycles, 1);
        assert_eq!(metrics.correction_recovery_cooldown_reenabled_cycles, 1);

        let final_snapshot = runtime.reconnect_state_restore_correction_state_snapshot();
        assert!(!final_snapshot.validation_pending);
        assert_eq!(final_snapshot.retry_attempts, 0);
        assert_eq!(final_snapshot.retry_cooldown_ticks, 0);
        assert_eq!(final_snapshot.consecutive_retry_exhaustions, 0);
        assert_eq!(
            final_snapshot.recovery_cooldown_reconnect_cycles_remaining,
            0
        );
        assert!(!final_snapshot.correction_suppressed_for_recovery_cycle);
        assert!(!final_snapshot.correction_reenabled_for_recovery_cycle);
    }

    #[test]
    fn client_runtime_controller_reidentify_dispatches_controller_auth_message() {
        let mut session = ClientSession::default();
        session.remember_control_password_for_room("+room:ABCDEF123456", "ab-123-456");
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        runtime
            .run_controller_reidentify_if_needed()
            .expect("controller reidentify should dispatch");

        let (_, _, control) = runtime.into_parts();
        assert_eq!(control.outbound_messages().len(), 1);
        assert_eq!(
            control.controller_auth_notifications(),
            &[ControllerAuthTransitionNotification::Attempting {
                room: "+room:ABCDEF123456".to_owned(),
            }]
        );
        let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
            panic!("expected queued controller auth Set message");
        };
        let controller_auth = set_message
            .set
            .controller_auth
            .as_ref()
            .expect("queued message should include controllerAuth payload");
        assert_eq!(controller_auth.room.as_deref(), Some("+room:ABCDEF123456"));
        assert_eq!(controller_auth.password.as_deref(), Some("AB-123-456"));
        assert!(controller_auth.user.is_none());
        assert!(controller_auth.success.is_none());
    }

    #[test]
    fn client_runtime_new_controlled_room_dispatches_room_then_controller_auth() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"newControlledRoom":{"roomName":"+room:ABCDEF123456","password":"AB-123-456"}}}"#,
            )
            .expect("new controlled room message should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        runtime
            .run_controller_reidentify_if_needed()
            .expect("controller reidentify should dispatch");

        let (_, _, control) = runtime.into_parts();
        assert_eq!(control.outbound_messages().len(), 2);
        assert_eq!(
            control.controller_auth_notifications(),
            &[ControllerAuthTransitionNotification::Attempting {
                room: "+room:ABCDEF123456".to_owned(),
            }]
        );

        let ProtocolMessage::Set(room_set) = &control.outbound_messages()[0] else {
            panic!("first outbound message should be Set.room");
        };
        let room = room_set
            .set
            .room
            .as_ref()
            .expect("first outbound message should include room payload");
        assert_eq!(room.name, "+room:ABCDEF123456");

        let ProtocolMessage::Set(auth_set) = &control.outbound_messages()[1] else {
            panic!("second outbound message should be Set.controllerAuth");
        };
        let controller_auth = auth_set
            .set
            .controller_auth
            .as_ref()
            .expect("second outbound message should include controllerAuth payload");
        assert_eq!(controller_auth.room.as_deref(), Some("+room:ABCDEF123456"));
        assert_eq!(controller_auth.password.as_deref(), Some("AB-123-456"));
    }

    #[test]
    fn client_runtime_controller_auth_outcome_notifications_dispatch_from_inbound_set() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"controllerAuth":{"user":"alice","room":"+room:ABCDEF123456","success":true}}}"#,
            )
            .expect("controller auth success should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        runtime
            .run_controller_auth_notifications_if_needed()
            .expect("controller auth notifications should dispatch");

        assert_eq!(
            runtime.control().controller_auth_notifications(),
            &[ControllerAuthTransitionNotification::Succeeded {
                username: "alice".to_owned(),
                room: "+room:ABCDEF123456".to_owned(),
                hide_from_osd: false,
            }]
        );
    }

    #[test]
    fn client_runtime_user_change_notifications_dispatch_from_inbound_set() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"+room:ABCDEF123456"}}}}}"#,
            )
            .expect("user join should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        runtime
            .run_user_change_notifications_if_needed()
            .expect("user change notifications should dispatch");

        assert_eq!(
            runtime.control().user_change_notifications(),
            &[UserChangeNotification::Joined {
                username: "bob".to_owned(),
                room: "+room:ABCDEF123456".to_owned(),
                hide_from_osd: true,
            }]
        );
    }

    #[test]
    fn client_runtime_chat_notifications_dispatch_from_inbound_chat() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(r#"{"Chat":{"username":"bob","message":"hello everyone"}}"#)
            .expect("chat should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        runtime
            .run_chat_notifications_if_needed()
            .expect("chat notifications should dispatch");

        assert_eq!(
            runtime.control().chat_notifications(),
            &[ChatNotification::Message {
                username: Some("bob".to_owned()),
                message: "hello everyone".to_owned(),
            }]
        );
    }

    #[test]
    fn client_runtime_chat_notifications_preserve_mixed_payload_order_across_batches() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(r#"{"Chat":"plain text first"}"#)
            .expect("text chat should apply");
        session
            .apply_message_json(
                r#"{"Chat":{"username":"bob","message":"object payload second","style":"notice"}}"#,
            )
            .expect("object chat with extra fields should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        runtime
            .run_chat_notifications_if_needed()
            .expect("first batch chat notifications should dispatch");

        assert_eq!(
            runtime.control().chat_notifications(),
            &[
                ChatNotification::Message {
                    username: None,
                    message: "plain text first".to_owned(),
                },
                ChatNotification::Message {
                    username: Some("bob".to_owned()),
                    message: "object payload second".to_owned(),
                },
            ]
        );

        assert_eq!(
            runtime.drain_chat_notifications(),
            vec![
                ChatNotification::Message {
                    username: None,
                    message: "plain text first".to_owned(),
                },
                ChatNotification::Message {
                    username: Some("bob".to_owned()),
                    message: "object payload second".to_owned(),
                },
            ]
        );
        assert!(runtime.drain_chat_notifications().is_empty());

        runtime
            .session_mut()
            .apply_message_json(r#"{"Chat":{"username":"carol","message":"third batch message"}}"#)
            .expect("later object chat should apply");
        runtime
            .run_chat_notifications_if_needed()
            .expect("second batch chat notifications should dispatch");

        assert_eq!(
            runtime.drain_chat_notifications(),
            vec![ChatNotification::Message {
                username: Some("carol".to_owned()),
                message: "third batch message".to_owned(),
            }]
        );
        assert!(
            runtime
                .session_mut()
                .runtime_actions_for_chat_notifications_if_needed()
                .is_empty(),
            "chat notification actions should be fully drained after dispatch"
        );
    }

    #[test]
    fn client_runtime_interleaved_user_change_and_chat_notifications_preserve_order_with_independent_drains()
     {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"controller":true}}}}"#,
            )
            .expect("initial bob join should apply");
        let _ = session.runtime_actions_for_user_change_notifications_if_needed();

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room2"},"controller":true}}}}"#,
            )
            .expect("bob room switch should apply");
        runtime
            .session_mut()
            .apply_message_json(r#"{"Chat":{"username":"bob","message":"moved to room2"}}"#)
            .expect("bob chat after room switch should apply");

        runtime
            .run_user_change_notifications_if_needed()
            .expect("user change notifications should dispatch");
        assert_eq!(
            runtime.drain_user_change_notifications(),
            vec![UserChangeNotification::Joined {
                username: "bob".to_owned(),
                room: "room2".to_owned(),
                hide_from_osd: false,
            }],
            "room-switch notification should preserve user-change ordering before chat dispatch in first batch"
        );
        assert!(
            runtime.control().chat_notifications().is_empty(),
            "dispatching user-change notifications should not implicitly dispatch chat notifications"
        );

        runtime
            .run_chat_notifications_if_needed()
            .expect("chat notifications should dispatch");
        assert_eq!(
            runtime.drain_chat_notifications(),
            vec![ChatNotification::Message {
                username: Some("bob".to_owned()),
                message: "moved to room2".to_owned(),
            }],
            "chat notification should remain pending until chat dispatch runs"
        );
        assert!(runtime.drain_user_change_notifications().is_empty());
        assert!(runtime.drain_chat_notifications().is_empty());

        runtime
            .session_mut()
            .apply_message_json(r#"{"Chat":{"username":"bob","message":"still in room2"}}"#)
            .expect("second bob chat should apply");
        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"controller":true}}}}"#,
            )
            .expect("bob room switch back should apply");

        runtime
            .run_chat_notifications_if_needed()
            .expect("chat notifications should dispatch first in second batch");
        assert_eq!(
            runtime.drain_chat_notifications(),
            vec![ChatNotification::Message {
                username: Some("bob".to_owned()),
                message: "still in room2".to_owned(),
            }],
            "chat queue should preserve arrival order when dispatched before user-change notifications"
        );
        assert!(
            runtime.control().user_change_notifications().is_empty(),
            "dispatching chat notifications should not implicitly dispatch user-change notifications"
        );

        runtime
            .run_user_change_notifications_if_needed()
            .expect("user change notifications should dispatch after chat in second batch");
        assert_eq!(
            runtime.drain_user_change_notifications(),
            vec![UserChangeNotification::Joined {
                username: "bob".to_owned(),
                room: "room1".to_owned(),
                hide_from_osd: false,
            }],
            "user-change queue should preserve the room-switch notification independently of chat drain order"
        );

        runtime
            .run_chat_notifications_if_needed()
            .expect("repeated chat dispatch should be a no-op after drains");
        runtime
            .run_user_change_notifications_if_needed()
            .expect("repeated user-change dispatch should be a no-op after drains");
        assert!(runtime.drain_chat_notifications().is_empty());
        assert!(runtime.drain_user_change_notifications().is_empty());
    }

    #[test]
    fn client_runtime_send_chat_message_dispatches_protocol_message() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        assert!(
            runtime
                .run_send_chat_message("hello room")
                .expect("send chat should dispatch"),
            "non-empty outbound chat should produce a queued send action"
        );

        assert_eq!(runtime.control().outbound_messages().len(), 1);
        let ProtocolMessage::Chat(chat_message) = &runtime.control().outbound_messages()[0] else {
            panic!("queued outbound message should be Chat");
        };
        assert_eq!(
            chat_message.chat,
            ChatPayload::Text("hello room".to_owned())
        );
    }

    #[test]
    fn client_runtime_send_chat_message_dispatches_empty_protocol_message() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        assert!(
            runtime
                .run_send_chat_message("")
                .expect("send chat should dispatch"),
            "empty outbound chat should still produce a queued send action"
        );

        assert_eq!(runtime.control().outbound_messages().len(), 1);
        let ProtocolMessage::Chat(chat_message) = &runtime.control().outbound_messages()[0] else {
            panic!("queued outbound message should be Chat");
        };
        assert_eq!(chat_message.chat, ChatPayload::Text("".to_owned()));
    }

    #[test]
    fn client_runtime_send_chat_message_does_not_emit_local_chat_notification_before_server_echo() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        assert!(
            runtime
                .run_send_chat_message("hello room")
                .expect("send chat should dispatch"),
            "outbound chat should be queued"
        );

        runtime
            .run_chat_notifications_if_needed()
            .expect("chat notification dispatch should succeed with no pending notifications");
        assert!(
            runtime.control().chat_notifications().is_empty(),
            "sending local chat should not produce a local notification before server echo"
        );
        assert!(
            runtime.drain_chat_notifications().is_empty(),
            "runtime chat notification queue should stay empty before server echo"
        );

        runtime
            .session_mut()
            .apply_message_json(r#"{"Chat":{"username":"alice","message":"hello room"}}"#)
            .expect("server echo chat should apply");
        runtime
            .run_chat_notifications_if_needed()
            .expect("chat notifications should dispatch after server echo");

        assert_eq!(
            runtime.drain_chat_notifications(),
            vec![ChatNotification::Message {
                username: Some("alice".to_owned()),
                message: "hello room".to_owned(),
            }],
            "chat notification should appear only after inbound server echo"
        );
    }

    #[test]
    fn client_runtime_send_chat_message_is_omitted_before_server_hello() {
        let session = ClientSession::default();
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        assert!(
            !runtime
                .run_send_chat_message("hello room")
                .expect("chat send should not fail"),
            "chat send should be suppressed until server hello is applied"
        );
        assert!(runtime.control().outbound_messages().is_empty());
    }

    #[test]
    fn client_runtime_send_chat_message_is_omitted_when_server_chat_is_disabled() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255","features":{"chat":false}}}"#,
            )
            .expect("hello should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            !runtime
                .run_send_chat_message("hello room")
                .expect("chat send should not fail"),
            "disabled chat support should suppress outbound chat actions"
        );
        assert!(runtime.control().outbound_messages().is_empty());
    }

    #[test]
    fn client_runtime_send_chat_message_is_omitted_after_disconnect_until_next_hello() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_disconnect(42.0)
            .expect("disconnect should apply pause-on-leave/runtime actions");
        assert!(
            !runtime
                .run_send_chat_message("hello while disconnected")
                .expect("chat send should not fail"),
            "chat send should be suppressed after disconnect until a new hello is applied"
        );
        assert!(
            runtime.control().outbound_messages().is_empty(),
            "suppressed disconnected chat should not enqueue outbound chat payloads"
        );

        runtime
            .session_mut()
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("reconnect hello should apply");
        assert!(
            runtime
                .run_send_chat_message("hello after reconnect")
                .expect("chat send should not fail"),
            "chat send should resume after reconnect hello"
        );
        assert_eq!(runtime.control().outbound_messages().len(), 1);
    }

    #[test]
    fn client_runtime_player_chat_input_dispatches_protocol_message() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
        let player = RecordingPlayer {
            pending_chat_requests: std::collections::VecDeque::from([String::from(
                "hello from mpv",
            )]),
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        assert_eq!(
            runtime
                .run_player_chat_input_if_needed()
                .expect("player chat input should dispatch"),
            1
        );

        assert_eq!(runtime.control().outbound_messages().len(), 1);
        let ProtocolMessage::Chat(chat_message) = &runtime.control().outbound_messages()[0] else {
            panic!("queued outbound message should be Chat");
        };
        assert_eq!(
            chat_message.chat,
            ChatPayload::Text("hello from mpv".to_owned())
        );
    }

    #[test]
    fn client_runtime_player_chat_input_is_suppressed_when_server_chat_is_disabled() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255","features":{"chat":false}}}"#,
            )
            .expect("hello should apply");
        let player = RecordingPlayer {
            pending_chat_requests: std::collections::VecDeque::from([String::from(
                "hello from mpv",
            )]),
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        assert_eq!(
            runtime
                .run_player_chat_input_if_needed()
                .expect("suppressed player chat input should not fail"),
            0
        );
        assert!(runtime.control().outbound_messages().is_empty());
    }

    #[test]
    fn client_runtime_toggle_ready_dispatches_protocol_message() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true}}}"#,
            )
            .expect("hello should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_toggle_ready(true)
                .expect("toggle ready should not fail"),
            "toggle ready should emit outbound Set.ready after hello"
        );
        let (_, _, control) = runtime.into_parts();

        assert_eq!(control.outbound_messages().len(), 1);
        let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
            panic!("expected queued Set.ready protocol message");
        };
        let ready = set_message
            .set
            .ready
            .as_ref()
            .expect("Set message should contain ready payload");
        assert!(ready.is_ready);
        assert_eq!(ready.manually_initiated, Some(true));
    }

    #[test]
    fn client_runtime_toggle_ready_is_omitted_before_server_hello() {
        let session = ClientSession::default();
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            !runtime
                .run_toggle_ready(true)
                .expect("toggle ready should not fail"),
            "toggle ready should be suppressed before server hello"
        );
        assert!(runtime.control().outbound_messages().is_empty());
    }

    #[test]
    fn client_runtime_toggle_ready_is_omitted_when_server_readiness_is_disabled() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":false}}}"#,
            )
            .expect("hello should apply");
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            !runtime
                .run_toggle_ready(true)
                .expect("toggle ready should not fail"),
            "toggle ready should be suppressed when server readiness is disabled"
        );
        assert!(runtime.control().outbound_messages().is_empty());
    }

    #[test]
    fn client_runtime_set_ready_for_user_dispatches_protocol_message_with_username() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true}}}"#,
            )
            .expect("hello should apply");
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_set_ready_for_user("bob", true, true)
                .expect("set ready for user should not fail"),
            "set ready for user should emit outbound Set.ready after hello"
        );
        let (_, _, control) = runtime.into_parts();

        assert_eq!(control.outbound_messages().len(), 1);
        let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
            panic!("expected queued Set.ready protocol message");
        };
        let ready = set_message
            .set
            .ready
            .as_ref()
            .expect("Set message should contain ready payload");
        assert!(ready.is_ready);
        assert_eq!(ready.manually_initiated, Some(true));
        assert_eq!(ready.username.as_deref(), Some("bob"));
    }

    #[test]
    fn client_runtime_set_ready_for_user_without_username_dispatches_local_ready_message() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true}}}"#,
            )
            .expect("hello should apply");
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_set_ready_for_user("", true, true)
                .expect("set ready without username should not fail"),
            "set ready without username should emit local Set.ready after hello"
        );
        let (_, _, control) = runtime.into_parts();

        assert_eq!(control.outbound_messages().len(), 1);
        let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
            panic!("expected queued Set.ready protocol message");
        };
        let ready = set_message
            .set
            .ready
            .as_ref()
            .expect("Set message should contain ready payload");
        assert!(ready.is_ready);
        assert_eq!(ready.manually_initiated, Some(true));
        assert!(
            ready.username.is_none(),
            "local ready set should omit username payload field"
        );
    }

    #[test]
    fn client_runtime_set_ready_for_explicit_local_username_dispatches_username_payload() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true}}}"#,
            )
            .expect("hello should apply");
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_set_ready_for_user("alice", false, true)
                .expect("set ready for explicit local username should not fail"),
            "set ready for explicit local username should emit outbound Set.ready with username"
        );
        let (_, _, control) = runtime.into_parts();

        assert_eq!(control.outbound_messages().len(), 1);
        let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
            panic!("expected queued Set.ready protocol message");
        };
        let ready = set_message
            .set
            .ready
            .as_ref()
            .expect("Set message should contain ready payload");
        assert!(!ready.is_ready);
        assert_eq!(ready.manually_initiated, Some(true));
        assert_eq!(ready.username.as_deref(), Some("alice"));
    }

    #[test]
    fn client_runtime_set_ready_for_whitespace_username_preserves_payload() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true}}}"#,
            )
            .expect("hello should apply");
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_set_ready_for_user(" ", true, true)
                .expect("set ready for whitespace username should not fail"),
            "set ready for whitespace username should emit outbound Set.ready with username"
        );
        let (_, _, control) = runtime.into_parts();

        assert_eq!(control.outbound_messages().len(), 1);
        let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
            panic!("expected queued Set.ready protocol message");
        };
        let ready = set_message
            .set
            .ready
            .as_ref()
            .expect("Set message should contain ready payload");
        assert!(ready.is_ready);
        assert_eq!(ready.manually_initiated, Some(true));
        assert_eq!(ready.username.as_deref(), Some(" "));
    }

    #[test]
    fn client_runtime_set_ready_for_user_is_omitted_before_server_hello() {
        let session = ClientSession::default();
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            !runtime
                .run_set_ready_for_user("bob", true, true)
                .expect("set ready for user should not fail"),
            "set ready for user should be suppressed before server hello"
        );
        assert!(runtime.control().outbound_messages().is_empty());
    }

    #[test]
    fn client_runtime_request_controller_auth_dispatches_protocol_message_with_normalized_password()
    {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_request_controller_auth(" +room:ABCDEF123456 ", "ab_123-456!")
                .expect("controller auth request should not fail"),
            "manual controller auth request should emit outbound Set.controllerAuth after hello"
        );
        let (_, _, control) = runtime.into_parts();
        assert_eq!(control.outbound_messages().len(), 1);
        let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
            panic!("expected queued Set.controllerAuth protocol message");
        };
        let controller_auth = set_message
            .set
            .controller_auth
            .as_ref()
            .expect("Set message should contain controllerAuth payload");
        assert_eq!(
            controller_auth.room.as_deref(),
            Some(" +room:ABCDEF123456 ")
        );
        assert_eq!(controller_auth.password.as_deref(), Some("AB123-456"));
        assert_eq!(
            control.controller_auth_notifications(),
            &[ControllerAuthTransitionNotification::Attempting {
                room: " +room:ABCDEF123456 ".to_owned()
            }]
        );
    }

    #[test]
    fn client_runtime_request_controller_auth_without_password_dispatches_empty_password_payload() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_request_controller_auth(" +room:ABCDEF123456 ", "   ")
                .expect("controller auth request should not fail"),
            "manual controller auth request should emit outbound Set.controllerAuth even with empty password after normalization"
        );
        let (_, _, control) = runtime.into_parts();
        assert_eq!(control.outbound_messages().len(), 1);
        let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
            panic!("expected queued Set.controllerAuth protocol message");
        };
        let controller_auth = set_message
            .set
            .controller_auth
            .as_ref()
            .expect("Set message should contain controllerAuth payload");
        assert_eq!(
            controller_auth.room.as_deref(),
            Some(" +room:ABCDEF123456 ")
        );
        assert_eq!(controller_auth.password.as_deref(), Some(""));
        assert_eq!(
            control.controller_auth_notifications(),
            &[ControllerAuthTransitionNotification::Attempting {
                room: " +room:ABCDEF123456 ".to_owned()
            }]
        );
    }

    #[test]
    fn client_runtime_request_controller_auth_dispatches_for_whitespace_only_room() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_request_controller_auth(" ", "AB-123-456")
                .expect("controller auth request should not fail"),
            "controller auth request should preserve whitespace-only room names"
        );
        let (_, _, control) = runtime.into_parts();
        assert_eq!(control.outbound_messages().len(), 1);
        let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
            panic!("expected queued Set.controllerAuth protocol message");
        };
        let controller_auth = set_message
            .set
            .controller_auth
            .as_ref()
            .expect("Set message should contain controllerAuth payload");
        assert_eq!(controller_auth.room.as_deref(), Some(" "));
        assert_eq!(controller_auth.password.as_deref(), Some("AB-123-456"));
        assert_eq!(
            control.controller_auth_notifications(),
            &[ControllerAuthTransitionNotification::Attempting {
                room: " ".to_owned()
            }]
        );
    }

    #[test]
    fn client_runtime_request_controller_auth_is_omitted_before_server_hello() {
        let session = ClientSession::default();
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            !runtime
                .run_request_controller_auth("+room:ABCDEF123456", "AB-123-456")
                .expect("controller auth request should not fail"),
            "manual controller auth request should be suppressed before server hello"
        );
        assert!(runtime.control().outbound_messages().is_empty());
        assert!(runtime.control().controller_auth_notifications().is_empty());
    }

    #[test]
    fn client_runtime_set_room_dispatches_protocol_message() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_set_room("  room2  ")
                .expect("set room should not fail"),
            "set room should emit outbound Set.room after hello"
        );
        let (_, _, control) = runtime.into_parts();

        assert_eq!(control.outbound_messages().len(), 1);
        let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
            panic!("expected queued Set.room protocol message");
        };
        let room = set_message
            .set
            .room
            .as_ref()
            .expect("Set message should contain room payload");
        assert_eq!(room.name, "  room2  ");
    }

    #[test]
    fn client_runtime_set_room_is_omitted_before_server_hello() {
        let session = ClientSession::default();
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            !runtime
                .run_set_room("room2")
                .expect("set room should not fail"),
            "set room should be suppressed before server hello"
        );
        assert!(runtime.control().outbound_messages().is_empty());
    }

    #[test]
    fn client_runtime_set_room_is_omitted_when_target_is_empty() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            !runtime.run_set_room("").expect("set room should not fail"),
            "empty room switch should be ignored"
        );
        assert!(runtime.control().outbound_messages().is_empty());
    }

    #[test]
    fn client_runtime_set_room_dispatches_when_target_is_whitespace_only() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_set_room("   ")
                .expect("set room should not fail"),
            "whitespace-only room switch should still emit outbound Set.room"
        );
        let (_, _, control) = runtime.into_parts();
        assert_eq!(control.outbound_messages().len(), 1);
        let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
            panic!("expected queued Set.room protocol message");
        };
        let room = set_message
            .set
            .room
            .as_ref()
            .expect("Set message should contain room payload");
        assert_eq!(room.name, "   ");
    }

    #[test]
    fn client_runtime_set_room_dispatches_even_when_target_is_unchanged() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_set_room("room1")
                .expect("set room should not fail"),
            "unchanged room switch should still emit outbound Set.room"
        );
        let (_, _, control) = runtime.into_parts();
        assert_eq!(control.outbound_messages().len(), 1);
        let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
            panic!("expected queued Set.room protocol message");
        };
        let room = set_message
            .set
            .room
            .as_ref()
            .expect("Set message should contain room payload");
        assert_eq!(room.name, "room1");
    }

    #[test]
    fn client_runtime_set_room_with_legacy_fallback_prefers_local_file_name() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"file":{"name":"movie.mkv"}}}}}"#,
            )
            .expect("local file metadata should apply");
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_set_room_with_legacy_fallback("fallback-room")
                .expect("set room fallback should not fail"),
            "room fallback should emit outbound Set.room from local file name"
        );
        let (_, _, control) = runtime.into_parts();
        assert_eq!(control.outbound_messages().len(), 1);
        let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
            panic!("expected queued Set.room protocol message");
        };
        let room = set_message
            .set
            .room
            .as_ref()
            .expect("Set message should contain room payload");
        assert_eq!(room.name, "movie.mkv");
    }

    #[test]
    fn client_runtime_set_room_with_legacy_fallback_uses_default_when_no_file() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_set_room_with_legacy_fallback("fallback-room")
                .expect("set room fallback should not fail"),
            "room fallback should emit outbound Set.room from default room"
        );
        let (_, _, control) = runtime.into_parts();
        assert_eq!(control.outbound_messages().len(), 1);
        let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
            panic!("expected queued Set.room protocol message");
        };
        let room = set_message
            .set
            .room
            .as_ref()
            .expect("Set message should contain room payload");
        assert_eq!(room.name, "fallback-room");
    }

    #[test]
    fn client_runtime_toggle_pause_dispatches_player_state_updates() {
        let session = ClientSession::default();
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_toggle_pause()
                .expect("toggle pause should not fail"),
            "toggle pause should emit a local SetPaused action"
        );
        assert_eq!(runtime.player().paused, Some(true));
        assert!(
            runtime
                .run_toggle_pause()
                .expect("toggle pause should not fail"),
            "toggle pause should emit a second local SetPaused action"
        );
        assert_eq!(runtime.player().paused, Some(false));
        assert!(
            runtime.control().outbound_messages().is_empty(),
            "local pause toggles should not directly emit protocol lines"
        );
    }

    #[test]
    fn client_runtime_request_user_list_dispatches_protocol_message() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_request_user_list()
                .expect("list request should not fail"),
            "list request should emit outbound List request after hello"
        );

        let (_, _, control) = runtime.into_parts();
        assert_eq!(control.outbound_messages().len(), 1);
        let ProtocolMessage::List(list_message) = &control.outbound_messages()[0] else {
            panic!("expected queued List protocol message");
        };
        assert!(matches!(list_message.list, ListPayload::Request(_)));
    }

    #[test]
    fn client_runtime_request_user_list_is_omitted_before_server_hello() {
        let session = ClientSession::default();
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            !runtime
                .run_request_user_list()
                .expect("list request should not fail"),
            "list request should be suppressed before server hello"
        );
        assert!(runtime.control().outbound_messages().is_empty());
    }

    #[test]
    fn client_runtime_set_playlist_index_dispatches_protocol_message() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
            .expect("playlist index should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_set_playlist_index(1)
                .expect("set playlist index should not fail"),
            "set playlist index should emit outbound Set.playlistIndex"
        );

        let (_, _, control) = runtime.into_parts();
        assert_eq!(control.outbound_messages().len(), 1);
        let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
            panic!("expected queued Set protocol message");
        };
        let playlist_index = set_message
            .set
            .playlist_index
            .as_ref()
            .expect("Set message should contain playlistIndex payload");
        assert_eq!(playlist_index.index, 1);
        assert!(playlist_index.user.is_none());
    }

    #[test]
    fn client_runtime_set_playlist_index_is_omitted_before_server_hello() {
        let session = ClientSession::default();
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            !runtime
                .run_set_playlist_index(0)
                .expect("set playlist index should not fail"),
            "set playlist index should be suppressed before server hello"
        );
        assert!(runtime.control().outbound_messages().is_empty());
    }

    #[test]
    fn client_runtime_set_playlist_index_is_omitted_for_invalid_index() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
            .expect("playlist index should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            !runtime
                .run_set_playlist_index(3)
                .expect("set playlist index should not fail"),
            "set playlist index should be suppressed when index is out of bounds"
        );
        assert!(runtime.control().outbound_messages().is_empty());
    }

    #[test]
    fn client_runtime_set_playlist_index_is_omitted_for_untrusted_url_target() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["https://example.com/video.mp4"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
            .expect("playlist index should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            !runtime
                .run_set_playlist_index(0)
                .expect("set playlist index should not fail"),
            "set playlist index should be suppressed for untrusted URL targets"
        );
        assert!(runtime.control().outbound_messages().is_empty());
    }

    #[test]
    fn client_runtime_set_playlist_index_allows_default_trusted_youtube_domain() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["https://youtube.com/watch?v=abc"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
            .expect("playlist index should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_set_playlist_index(0)
                .expect("set playlist index should not fail"),
            "set playlist index should allow default trusted domains"
        );
        assert_eq!(runtime.control().outbound_messages().len(), 1);
    }

    #[test]
    fn client_runtime_trusted_url_matching_supports_wildcard_and_path_prefix() {
        let mut session = ClientSession::default();
        session.behavior_config_mut().trusted_domains = vec!["*.example.com/videos".to_owned()];

        assert!(session.uri_is_trusted_legacy_compatible("https://cdn.example.com/videos/a.mp4"));
        assert!(!session.uri_is_trusted_legacy_compatible("https://cdn.example.com/clips/a.mp4"));
        assert!(!session.uri_is_trusted_legacy_compatible("ftp://cdn.example.com/videos/a.mp4"));
        assert!(!session.uri_is_trusted_legacy_compatible("https://a.b.example.com/videos/a.mp4"));
    }

    #[test]
    fn client_runtime_trusted_url_matching_respects_only_switch_toggle() {
        let mut session = ClientSession::default();
        session.behavior_config_mut().only_switch_to_trusted_domains = false;
        session.behavior_config_mut().trusted_domains.clear();

        assert!(session.uri_is_trusted_legacy_compatible("http://example.com/video.mp4"));
        assert!(session.uri_is_trusted_legacy_compatible("https://example.com/video.mp4"));
        assert!(!session.uri_is_trusted_legacy_compatible("ftp://example.com/video.mp4"));
    }

    #[test]
    fn client_runtime_advance_playlist_index_dispatches_protocol_message() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
            .expect("playlist index should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_advance_playlist_index()
                .expect("next playlist command should not fail"),
            "next playlist command should emit outbound Set.playlistIndex"
        );

        let (_, _, control) = runtime.into_parts();
        assert_eq!(control.outbound_messages().len(), 1);
        let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
            panic!("expected queued Set protocol message");
        };
        let playlist_index = set_message
            .set
            .playlist_index
            .as_ref()
            .expect("Set message should contain playlistIndex payload");
        assert_eq!(playlist_index.index, 1);
        assert!(playlist_index.user.is_none());
    }

    #[test]
    fn client_runtime_advance_playlist_index_is_omitted_for_untrusted_url_target() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","https://example.com/video.mp4"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
            .expect("playlist index should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            !runtime
                .run_advance_playlist_index()
                .expect("next playlist command should not fail"),
            "next playlist command should be suppressed for untrusted URL targets"
        );
        assert!(runtime.control().outbound_messages().is_empty());
    }

    #[test]
    fn client_runtime_advance_playlist_index_is_omitted_without_next_item() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
            .expect("playlist index should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            !runtime
                .run_advance_playlist_index()
                .expect("next playlist command should not fail"),
            "next playlist command should be suppressed when no next item exists"
        );
        assert!(runtime.control().outbound_messages().is_empty());
    }

    #[test]
    fn client_runtime_advance_playlist_index_loops_to_start_when_loop_at_end_enabled() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
            .expect("playlist index should apply");
        session.behavior_config_mut().loop_at_end_of_playlist = true;

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_advance_playlist_index()
                .expect("next playlist command should not fail"),
            "next playlist command should loop back to first item when loop-at-end is enabled"
        );

        let (_, _, control) = runtime.into_parts();
        assert_eq!(control.outbound_messages().len(), 1);
        let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
            panic!("expected queued Set protocol message");
        };
        let playlist_index = set_message
            .set
            .playlist_index
            .as_ref()
            .expect("Set message should contain playlistIndex payload");
        assert_eq!(playlist_index.index, 0);
        assert!(playlist_index.user.is_none());
    }

    #[test]
    fn client_runtime_advance_playlist_index_rewinds_single_music_file_legacy_style() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["song.flac"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
            .expect("playlist index should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_advance_playlist_index()
                .expect("next playlist command should not fail"),
            "next playlist command should rewind/unpause for single music playlist entries"
        );
        assert_eq!(runtime.player().position, Some(0.0));
        assert_eq!(runtime.player().paused, Some(false));
        assert!(runtime.control().outbound_messages().is_empty());
    }

    #[test]
    fn client_runtime_advance_playlist_index_rewinds_single_file_when_loop_single_enabled() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
            .expect("playlist index should apply");
        session.behavior_config_mut().loop_single_files = true;

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_advance_playlist_index()
                .expect("next playlist command should not fail"),
            "next playlist command should rewind/unpause when loop-single-files is enabled"
        );
        assert_eq!(runtime.player().position, Some(0.0));
        assert_eq!(runtime.player().paused, Some(false));
        assert!(runtime.control().outbound_messages().is_empty());
    }

    #[test]
    fn client_runtime_queue_playlist_item_dispatches_playlist_change_and_preserves_index() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
            .expect("playlist index should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_queue_playlist_item("episode3.mkv", false)
                .expect("queue command should not fail"),
            "queue command should emit playlist change/index updates"
        );

        let (_, _, control) = runtime.into_parts();
        assert_eq!(control.outbound_messages().len(), 2);
        let ProtocolMessage::Set(change_message) = &control.outbound_messages()[0] else {
            panic!("first outbound queue message should be Set.playlistChange");
        };
        let playlist_change = change_message
            .set
            .playlist_change
            .as_ref()
            .expect("first outbound message should include playlistChange");
        assert_eq!(
            playlist_change.files,
            vec!["episode1.mkv", "episode2.mkv", "episode3.mkv"]
        );
        assert!(playlist_change.user.is_none());

        let ProtocolMessage::Set(index_message) = &control.outbound_messages()[1] else {
            panic!("second outbound queue message should be Set.playlistIndex");
        };
        let playlist_index = index_message
            .set
            .playlist_index
            .as_ref()
            .expect("second outbound message should include playlistIndex");
        assert_eq!(playlist_index.index, 0);
        assert!(playlist_index.user.is_none());
    }

    #[test]
    fn client_runtime_queue_playlist_item_preserves_whitespace_only_file_name() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
            .expect("playlist index should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_queue_playlist_item(" ", false)
                .expect("queue command should not fail"),
            "queue command should preserve whitespace-only file names"
        );

        let (_, _, control) = runtime.into_parts();
        assert_eq!(control.outbound_messages().len(), 2);
        let ProtocolMessage::Set(change_message) = &control.outbound_messages()[0] else {
            panic!("first outbound queue message should be Set.playlistChange");
        };
        let playlist_change = change_message
            .set
            .playlist_change
            .as_ref()
            .expect("first outbound message should include playlistChange");
        assert_eq!(
            playlist_change.files,
            vec!["episode1.mkv", "episode2.mkv", " "]
        );
        assert!(playlist_change.user.is_none());
    }

    #[test]
    fn client_runtime_queue_and_select_playlist_item_sets_new_item_index() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
            .expect("playlist index should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_queue_playlist_item("episode3.mkv", true)
                .expect("queue-and-select command should not fail"),
            "queue-and-select command should emit playlist change/index updates"
        );

        let (_, _, control) = runtime.into_parts();
        assert_eq!(control.outbound_messages().len(), 2);
        let ProtocolMessage::Set(index_message) = &control.outbound_messages()[1] else {
            panic!("second outbound queue-and-select message should be Set.playlistIndex");
        };
        let playlist_index = index_message
            .set
            .playlist_index
            .as_ref()
            .expect("second outbound message should include playlistIndex");
        assert_eq!(playlist_index.index, 2);
        assert!(playlist_index.user.is_none());
    }

    #[test]
    fn client_runtime_queue_playlist_item_is_omitted_before_server_hello() {
        let session = ClientSession::default();
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            !runtime
                .run_queue_playlist_item("episode1.mkv", false)
                .expect("queue command should not fail"),
            "queue command should be suppressed before server hello"
        );
        assert!(runtime.control().outbound_messages().is_empty());
    }

    #[test]
    fn client_runtime_delete_playlist_index_dispatches_playlist_change_and_index() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv","episode3.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":2,"user":"alice"}}}"#)
            .expect("playlist index should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_delete_playlist_index(1)
                .expect("delete command should not fail"),
            "delete command should emit playlist change/index updates"
        );

        let (_, _, control) = runtime.into_parts();
        assert_eq!(control.outbound_messages().len(), 2);
        let ProtocolMessage::Set(change_message) = &control.outbound_messages()[0] else {
            panic!("first outbound delete message should be Set.playlistChange");
        };
        let playlist_change = change_message
            .set
            .playlist_change
            .as_ref()
            .expect("first outbound message should include playlistChange");
        assert_eq!(playlist_change.files, vec!["episode1.mkv", "episode3.mkv"]);
        assert!(playlist_change.user.is_none());

        let ProtocolMessage::Set(index_message) = &control.outbound_messages()[1] else {
            panic!("second outbound delete message should be Set.playlistIndex");
        };
        let playlist_index = index_message
            .set
            .playlist_index
            .as_ref()
            .expect("second outbound message should include playlistIndex");
        assert_eq!(playlist_index.index, 1);
        assert!(playlist_index.user.is_none());
    }

    #[test]
    fn client_runtime_delete_playlist_index_last_item_emits_only_playlist_change() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
            .expect("playlist index should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_delete_playlist_index(0)
                .expect("delete command should not fail"),
            "delete command should emit playlist change for last item removal"
        );

        let (_, _, control) = runtime.into_parts();
        assert_eq!(control.outbound_messages().len(), 1);
        let ProtocolMessage::Set(change_message) = &control.outbound_messages()[0] else {
            panic!("outbound delete message should be Set.playlistChange");
        };
        let playlist_change = change_message
            .set
            .playlist_change
            .as_ref()
            .expect("outbound message should include playlistChange");
        assert!(playlist_change.files.is_empty());
        assert!(playlist_change.user.is_none());
    }

    #[test]
    fn client_runtime_delete_playlist_index_is_omitted_for_invalid_index() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
            .expect("playlist index should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            !runtime
                .run_delete_playlist_index(3)
                .expect("delete command should not fail"),
            "delete command should be suppressed for invalid index"
        );
        assert!(runtime.control().outbound_messages().is_empty());
    }

    #[test]
    fn client_runtime_shuffle_remaining_playlist_preserves_prefix_and_index() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv","episode3.mkv","episode4.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
            .expect("playlist index should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        let mut sent = false;
        for _ in 0..4 {
            if runtime
                .run_shuffle_remaining_playlist()
                .expect("shuffle remaining should not fail")
            {
                sent = true;
                break;
            }
        }
        assert!(
            sent,
            "shuffle remaining should eventually emit playlist change/index updates"
        );

        let (_, _, control) = runtime.into_parts();
        let outbound_messages = control.outbound_messages();
        assert_eq!(outbound_messages.len(), 2);

        let ProtocolMessage::Set(change_message) = &outbound_messages[0] else {
            panic!("first outbound shuffle-remaining message should be Set.playlistChange");
        };
        let playlist_change = change_message
            .set
            .playlist_change
            .as_ref()
            .expect("first outbound message should include playlistChange");
        assert_eq!(
            &playlist_change.files[..2],
            &["episode1.mkv".to_owned(), "episode2.mkv".to_owned()]
        );
        let mut expected_tail = vec!["episode3.mkv".to_owned(), "episode4.mkv".to_owned()];
        let mut actual_tail = playlist_change.files[2..].to_vec();
        expected_tail.sort();
        actual_tail.sort();
        assert_eq!(actual_tail, expected_tail);

        let ProtocolMessage::Set(index_message) = &outbound_messages[1] else {
            panic!("second outbound shuffle-remaining message should be Set.playlistIndex");
        };
        let playlist_index = index_message
            .set
            .playlist_index
            .as_ref()
            .expect("second outbound message should include playlistIndex");
        assert_eq!(playlist_index.index, 1);
    }

    #[test]
    fn client_runtime_shuffle_entire_playlist_resets_index_to_zero() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv","episode3.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist change should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":2,"user":"alice"}}}"#)
            .expect("playlist index should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_shuffle_entire_playlist()
                .expect("shuffle entire should not fail"),
            "shuffle entire should emit at least a playlist index reset"
        );

        let (_, _, control) = runtime.into_parts();
        let outbound_messages = control.outbound_messages();
        assert!(
            !outbound_messages.is_empty(),
            "shuffle entire should emit protocol messages"
        );

        let ProtocolMessage::Set(last_set) = outbound_messages
            .last()
            .expect("shuffle entire should emit at least one Set message")
        else {
            panic!("last outbound message should be Set.playlistIndex");
        };
        let playlist_index = last_set
            .set
            .playlist_index
            .as_ref()
            .expect("last outbound message should include playlistIndex");
        assert_eq!(playlist_index.index, 0);
    }

    #[test]
    fn client_runtime_undo_playlist_change_toggles_between_previous_and_current_playlist() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv","episode3.mkv"],"user":"alice"}}}"#,
            )
            .expect("initial playlist should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
            .expect("initial playlist index should apply");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode3.mkv"],"user":"alice"}}}"#,
            )
            .expect("updated playlist should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":1,"user":"alice"}}}"#)
            .expect("updated playlist index should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_undo_playlist_change()
                .expect("undo playlist should not fail"),
            "undo playlist should emit restore actions when a previous playlist exists"
        );

        {
            let outbound_messages = runtime.control().outbound_messages();
            assert_eq!(outbound_messages.len(), 2);
            let ProtocolMessage::Set(change_message) = &outbound_messages[0] else {
                panic!("first outbound undo message should be Set.playlistChange");
            };
            let playlist_change = change_message
                .set
                .playlist_change
                .as_ref()
                .expect("first outbound undo message should include playlistChange");
            assert_eq!(
                playlist_change.files,
                vec!["episode1.mkv", "episode2.mkv", "episode3.mkv"]
            );

            let ProtocolMessage::Set(index_message) = &outbound_messages[1] else {
                panic!("second outbound undo message should be Set.playlistIndex");
            };
            let playlist_index = index_message
                .set
                .playlist_index
                .as_ref()
                .expect("second outbound undo message should include playlistIndex");
            assert_eq!(playlist_index.index, 2);
        }

        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv","episode3.mkv"],"user":"alice"}}}"#,
            )
            .expect("restored playlist echo should apply");
        runtime
            .session_mut()
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":2,"user":"alice"}}}"#)
            .expect("restored playlist index echo should apply");

        assert!(
            runtime
                .run_undo_playlist_change()
                .expect("second undo playlist should not fail"),
            "second undo should toggle back to the most recent playlist snapshot"
        );
        let (_, _, control) = runtime.into_parts();
        assert_eq!(control.outbound_messages().len(), 4);
        let ProtocolMessage::Set(change_message) = &control.outbound_messages()[2] else {
            panic!("first outbound second-undo message should be Set.playlistChange");
        };
        let playlist_change = change_message
            .set
            .playlist_change
            .as_ref()
            .expect("second undo change message should include playlistChange");
        assert_eq!(playlist_change.files, vec!["episode1.mkv", "episode3.mkv"]);
    }

    #[test]
    fn client_runtime_undo_playlist_change_restores_initial_empty_snapshot_once() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"playlistChange":{"files":["episode1.mkv"],"user":"alice"}}}"#,
            )
            .expect("playlist should apply");
        session
            .apply_message_json(r#"{"Set":{"playlistIndex":{"index":0,"user":"alice"}}}"#)
            .expect("playlist index should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_undo_playlist_change()
                .expect("undo playlist should not fail"),
            "first undo should restore the initial empty snapshot"
        );
        assert_eq!(runtime.control().outbound_messages().len(), 1);
        let ProtocolMessage::Set(change_message) = &runtime.control().outbound_messages()[0] else {
            panic!("undo playlist message should be Set.playlistChange");
        };
        let playlist_change = change_message
            .set
            .playlist_change
            .as_ref()
            .expect("undo playlist message should include playlistChange");
        assert!(playlist_change.files.is_empty());

        assert!(
            !runtime
                .run_undo_playlist_change()
                .expect("second undo playlist should not fail"),
            "second undo without playlist echo should be suppressed"
        );
        assert_eq!(runtime.control().outbound_messages().len(), 1);
    }

    #[test]
    fn client_runtime_seek_to_position_dispatches_player_position_updates() {
        let session = ClientSession::default();
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_seek_to_position(42.5)
                .expect("seek-to should not fail"),
            "seek-to should emit a local SetPosition action"
        );
        assert_eq!(runtime.player().position, Some(42.5));
        assert!(
            runtime.control().outbound_messages().is_empty(),
            "local seek should not directly emit protocol lines"
        );
    }

    #[test]
    fn client_runtime_seek_by_offset_uses_global_position_when_available() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"State":{"playstate":{"position":10.0,"paused":false,"doSeek":false,"setBy":"bob"}}}"#,
            )
            .expect("state should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            runtime
                .run_seek_by_offset(2.25)
                .expect("seek-by should not fail"),
            "seek-by should emit a local SetPosition action"
        );
        assert_eq!(runtime.player().position, Some(12.25));
        assert!(
            runtime.control().outbound_messages().is_empty(),
            "local seek should not directly emit protocol lines"
        );
    }

    #[test]
    fn client_runtime_seek_by_offset_falls_back_to_last_local_seek_position() {
        let session = ClientSession::default();
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        runtime
            .run_seek_to_position(5.0)
            .expect("initial seek should not fail");
        assert!(
            runtime
                .run_seek_by_offset(3.0)
                .expect("seek-by should not fail"),
            "seek-by should emit a local SetPosition action"
        );
        assert_eq!(runtime.player().position, Some(8.0));
    }

    #[test]
    fn client_runtime_undo_seek_is_omitted_without_seek_history() {
        let session = ClientSession::default();
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        assert!(
            !runtime.run_undo_seek().expect("undo seek should not fail"),
            "undo seek should be suppressed when no previous seek position is available"
        );
        assert_eq!(runtime.player().position, None);
        assert!(runtime.control().outbound_messages().is_empty());
    }

    #[test]
    fn client_runtime_undo_seek_toggles_between_current_and_previous_positions() {
        let session = ClientSession::default();
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        runtime
            .run_seek_to_position(10.0)
            .expect("initial seek should not fail");
        assert_eq!(runtime.player().position, Some(10.0));

        assert!(
            runtime.run_undo_seek().expect("undo seek should not fail"),
            "undo seek should emit a local SetPosition action"
        );
        assert_eq!(runtime.player().position, Some(0.0));

        assert!(
            runtime.run_undo_seek().expect("undo seek should not fail"),
            "second undo seek should toggle to previous position"
        );
        assert_eq!(runtime.player().position, Some(10.0));
        assert!(runtime.control().outbound_messages().is_empty());
    }

    #[test]
    fn client_runtime_readiness_unpause_attempt_dispatches_ready_protocol_message() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_readiness_unpause_attempt(10.0, true, true, false)
            .expect("runtime should dispatch readiness actions");
        let (_, player, control) = runtime.into_parts();

        assert_eq!(player.paused, Some(true));
        assert_eq!(control.outbound_messages().len(), 1);
        let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
            panic!("expected queued ready protocol message");
        };
        let ready = set_message
            .set
            .ready
            .as_ref()
            .expect("Set message should contain ready payload");
        assert!(ready.is_ready);
        assert_eq!(ready.manually_initiated, Some(true));
    }

    #[test]
    fn client_runtime_tick_autoplay_dispatches_unpause_to_player() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
            .expect("local ready should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"bob.mp4"},"isReady":true}}}}"#,
            )
            .expect("other user ready should apply");
        session.set_autoplay_enabled(true);
        session.readiness_autoplay_config_mut().auto_play_threshold = Some(2);
        session.local_paused = Some(true);

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime.update_autoplay_check(true, true, false, false);
        runtime
            .tick_autoplay(true, true, false, false)
            .expect("first autoplay tick should dispatch");
        runtime
            .tick_autoplay(true, true, false, false)
            .expect("second autoplay tick should dispatch");
        runtime
            .tick_autoplay(true, true, false, false)
            .expect("third autoplay tick should dispatch");
        runtime
            .tick_autoplay(true, true, false, false)
            .expect("fourth autoplay tick should dispatch unpause");

        let (_, player, control) = runtime.into_parts();
        assert_eq!(player.paused, Some(false));
        assert!(
            control.outbound_messages().is_empty(),
            "autoplay unpause should only require local player action"
        );
        assert_eq!(
            control.autoplay_notifications(),
            &[
                AutoplayCountdownNotification {
                    ready_user_count: 2,
                    seconds_left: 3
                },
                AutoplayCountdownNotification {
                    ready_user_count: 2,
                    seconds_left: 2
                },
                AutoplayCountdownNotification {
                    ready_user_count: 2,
                    seconds_left: 1
                }
            ]
        );
    }

    #[test]
    fn client_runtime_run_disconnect_applies_pause_on_leave_action() {
        let session = ClientSession {
            local_paused: Some(false),
            ..ClientSession::default()
        };
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_disconnect(42.0)
            .expect("disconnect handling should dispatch pause action");

        let (session, player, control) = runtime.into_parts();
        assert_eq!(session.last_paused_on_leave_at_seconds(), Some(42.0));
        assert_eq!(player.paused, Some(true));
        assert!(
            control.outbound_messages().is_empty(),
            "disconnect handling should not queue outbound protocol messages"
        );
    }

    #[test]
    fn queued_runtime_control_can_drain_encoded_protocol_lines() {
        let mut control = QueuedRuntimeControl::default();
        control.set_ready(true, false);

        let lines = control
            .drain_outbound_message_lines()
            .expect("queued messages should encode");
        assert_eq!(lines.len(), 1);
        assert!(
            lines[0].contains("\"Set\""),
            "encoded line should contain Set envelope"
        );
        assert!(
            lines[0].contains("\"isReady\":true"),
            "encoded line should contain ready=true"
        );
        assert!(
            lines[0].contains("\"manuallyInitiated\":false"),
            "encoded line should preserve manuallyInitiated"
        );
        assert!(control.outbound_messages().is_empty());
    }

    #[test]
    fn client_runtime_flush_helpers_expose_protocol_and_reconnect_intents() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_readiness_unpause_attempt(10.0, true, true, false)
            .expect("readiness attempt should dispatch");
        let outbound_lines = runtime
            .flush_queued_protocol_lines()
            .expect("queued outbound lines should encode");
        assert_eq!(outbound_lines.len(), 1);
        assert!(outbound_lines[0].contains("\"isReady\":true"));

        runtime
            .run_reconnect_retry(0)
            .expect("reconnect retry should dispatch");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![ReconnectTransitionNotification::Attempting {
                retries: 0,
                delay_seconds: 0.1,
            }]
        );
        assert!(runtime.drain_reconnect_notifications().is_empty());
        assert_eq!(runtime.drain_reconnect_requests(), vec![0.1]);
        assert!(runtime.drain_reconnect_requests().is_empty());

        runtime.session_mut().reconnect_policy_mut().max_retries = 0;
        runtime
            .run_reconnect_retry(1)
            .expect("terminal reconnect retry should dispatch");
        assert_eq!(
            runtime.drain_reconnect_notifications(),
            vec![ReconnectTransitionNotification::Disconnected]
        );
        assert!(runtime.take_stop_reconnect_requested());
        assert!(!runtime.take_stop_reconnect_requested());
    }

    #[test]
    fn client_runtime_flush_queued_protocol_lines_to_transport_uses_sender_callback() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_readiness_unpause_attempt(10.0, true, true, false)
            .expect("readiness attempt should dispatch");

        let mut sent_lines = Vec::new();
        runtime
            .flush_queued_protocol_lines_to_transport(|line| {
                sent_lines.push(line.to_owned());
                Ok(())
            })
            .expect("transport sender callback should be invoked per line");

        assert_eq!(sent_lines.len(), 1);
        assert!(sent_lines[0].contains("\"Set\""));
        assert!(sent_lines[0].contains("\"isReady\":true"));
        assert!(runtime.flush_queued_protocol_messages().is_empty());
    }

    #[test]
    fn client_runtime_drain_reconnect_intents_dispatches_scheduler_callbacks() {
        let session = ClientSession::default();
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_reconnect_retry(0)
            .expect("retry dispatch should queue reconnect delay");
        runtime.session_mut().reconnect_policy_mut().max_retries = 0;
        runtime
            .run_reconnect_retry(1)
            .expect("terminal retry should queue stop intent");

        let mut scheduled = Vec::new();
        let mut stop_calls = 0_usize;
        runtime.drain_reconnect_intents(
            |delay_seconds| scheduled.push(delay_seconds),
            || stop_calls += 1,
        );

        assert_eq!(scheduled, vec![0.1]);
        assert_eq!(stop_calls, 1);

        runtime.drain_reconnect_intents(
            |delay_seconds| scheduled.push(delay_seconds),
            || stop_calls += 1,
        );
        assert_eq!(scheduled, vec![0.1]);
        assert_eq!(stop_calls, 1);
    }

    #[test]
    fn client_runtime_run_reconnect_transition_dispatches_connected_notification() {
        let session = ClientSession::default();
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_reconnect_retry(0)
            .expect("reconnect retry should dispatch");
        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        runtime
            .run_reconnect_transition_if_needed()
            .expect("reconnect transition dispatch should succeed");

        assert_eq!(
            runtime.control().reconnect_notifications(),
            &[
                ReconnectTransitionNotification::Attempting {
                    retries: 0,
                    delay_seconds: 0.1,
                },
                ReconnectTransitionNotification::Connected,
            ]
        );
    }

    #[test]
    fn client_runtime_drain_reconnect_notifications_to_sink_dispatches_callback() {
        let session = ClientSession::default();
        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        runtime
            .run_reconnect_retry(0)
            .expect("reconnect retry should dispatch");
        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        runtime
            .run_reconnect_transition_if_needed()
            .expect("reconnect transition dispatch should succeed");

        let mut captured = Vec::new();
        runtime
            .drain_reconnect_notifications_to_sink(|notification| {
                captured.push(notification.clone());
                Ok::<(), ()>(())
            })
            .expect("reconnect notification sink dispatch should succeed");

        assert_eq!(
            captured,
            vec![
                ReconnectTransitionNotification::Attempting {
                    retries: 0,
                    delay_seconds: 0.1,
                },
                ReconnectTransitionNotification::Connected,
            ]
        );
        assert!(runtime.drain_reconnect_notifications().is_empty());
    }

    #[test]
    fn client_runtime_drain_controller_auth_notifications_to_sink_dispatches_callback() {
        let mut session = ClientSession::default();
        session.remember_control_password_for_room("+room:ABCDEF123456", "ab-123-456");
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        runtime
            .run_controller_reidentify_if_needed()
            .expect("controller reidentify should dispatch");
        runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"controllerAuth":{"user":"alice","room":"+room:ABCDEF123456","success":true}}}"#,
            )
            .expect("controller auth success should apply");
        runtime
            .run_controller_auth_notifications_if_needed()
            .expect("controller auth notifications should dispatch");

        let mut captured = Vec::new();
        runtime
            .drain_controller_auth_notifications_to_sink(|notification| {
                captured.push(notification.clone());
                Ok::<(), ()>(())
            })
            .expect("controller auth notification sink dispatch should succeed");

        assert_eq!(
            captured,
            vec![
                ControllerAuthTransitionNotification::Attempting {
                    room: "+room:ABCDEF123456".to_owned(),
                },
                ControllerAuthTransitionNotification::Succeeded {
                    username: "alice".to_owned(),
                    room: "+room:ABCDEF123456".to_owned(),
                    hide_from_osd: false,
                },
            ]
        );
        assert!(runtime.drain_controller_auth_notifications().is_empty());
    }

    #[test]
    fn client_runtime_drain_user_change_notifications_to_sink_dispatches_callback() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"+room:ABCDEF123456"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"+room:ABCDEF123456"}}}}}"#,
            )
            .expect("user join should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        runtime
            .run_user_change_notifications_if_needed()
            .expect("user change notifications should dispatch");

        let mut captured = Vec::new();
        runtime
            .drain_user_change_notifications_to_sink(|notification| {
                captured.push(notification.clone());
                Ok::<(), ()>(())
            })
            .expect("user change notification sink dispatch should succeed");

        assert_eq!(
            captured,
            vec![UserChangeNotification::Joined {
                username: "bob".to_owned(),
                room: "+room:ABCDEF123456".to_owned(),
                hide_from_osd: true,
            }]
        );
        assert!(runtime.drain_user_change_notifications().is_empty());
    }

    #[test]
    fn client_runtime_drain_chat_notifications_to_sink_dispatches_callback() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(r#"{"Chat":{"username":"bob","message":"hello everyone"}}"#)
            .expect("chat should apply");

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        runtime
            .run_chat_notifications_if_needed()
            .expect("chat notifications should dispatch");

        let mut captured = Vec::new();
        runtime
            .drain_chat_notifications_to_sink(|notification| {
                captured.push(notification.clone());
                Ok::<(), ()>(())
            })
            .expect("chat notification sink dispatch should succeed");

        assert_eq!(
            captured,
            vec![ChatNotification::Message {
                username: Some("bob".to_owned()),
                message: "hello everyone".to_owned(),
            }]
        );
        assert!(runtime.drain_chat_notifications().is_empty());
    }

    #[test]
    fn client_runtime_drain_player_playback_telemetry_updates_to_sink_dispatches_callback() {
        let session = ClientSession::default();
        let player = RecordingPlayer {
            pending_playback_telemetry_update: Some(
                PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(true)
                    .with_position_seconds(12.5)
                    .with_playback_rate(0.95),
            ),
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        let mut captured = Vec::new();
        runtime
            .drain_player_playback_telemetry_updates_to_sink(|update| {
                captured.push(update.clone());
                Ok::<(), ()>(())
            })
            .expect("playback telemetry sink dispatch should succeed");

        assert_eq!(
            captured,
            vec![PlayerPlaybackTelemetryUpdate {
                paused: Some(true),
                position_seconds: Some(12.5),
                playback_rate: Some(0.95),
            }]
        );
        assert!(runtime.drain_player_playback_telemetry_updates().is_empty());
    }

    #[test]
    fn client_runtime_drain_player_playback_telemetry_updates_refreshes_local_state() {
        let mut session = ClientSession::default();
        session.local_paused = Some(true);
        session.local_position = Some(1.0);
        let player = RecordingPlayer {
            pending_playback_telemetry_update: Some(
                PlayerPlaybackTelemetryUpdate::default()
                    .with_paused(false)
                    .with_position_seconds(12.5),
            ),
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        let updates = runtime.drain_player_playback_telemetry_updates();
        assert_eq!(updates.len(), 1);
        assert_eq!(runtime.session().local_paused, Some(false));
        assert_eq!(runtime.session().local_position, Some(12.5));

        assert!(
            runtime
                .run_toggle_pause()
                .expect("toggle pause should use telemetry-refreshed local paused state"),
            "toggle pause should emit a local SetPaused action"
        );
        assert_eq!(
            runtime.player().paused,
            Some(true),
            "toggle should invert the telemetry-confirmed paused=false state"
        );
    }

    #[test]
    fn client_runtime_toggle_pause_pre_syncs_pending_telemetry_and_preserves_drain() {
        let mut session = ClientSession::default();
        session.local_paused = Some(true);
        let player = RecordingPlayer {
            pending_playback_telemetry_update: Some(
                PlayerPlaybackTelemetryUpdate::default().with_paused(false),
            ),
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        assert!(
            runtime
                .run_toggle_pause()
                .expect("toggle pause should pre-sync pending telemetry"),
            "toggle pause should emit a local SetPaused action"
        );
        assert_eq!(
            runtime.player().paused,
            Some(true),
            "toggle should invert telemetry-confirmed paused=false, not stale local_paused=true"
        );

        let drained = runtime.drain_player_playback_telemetry_updates();
        assert_eq!(
            drained,
            vec![PlayerPlaybackTelemetryUpdate::default().with_paused(false)]
        );
    }

    #[test]
    fn client_runtime_seek_by_offset_pre_syncs_pending_telemetry_position() {
        let mut session = ClientSession::default();
        session.local_position = Some(1.0);
        let player = RecordingPlayer {
            pending_playback_telemetry_update: Some(
                PlayerPlaybackTelemetryUpdate::default().with_position_seconds(12.5),
            ),
            ..RecordingPlayer::default()
        };
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);

        assert!(
            runtime
                .run_seek_by_offset(2.0)
                .expect("seek-by-offset should pre-sync pending telemetry position"),
            "seek-by-offset should emit a local SetPosition action"
        );
        assert_eq!(
            runtime.player().position,
            Some(14.5),
            "offset seek should use telemetry-confirmed local position as the baseline"
        );
        assert_eq!(
            runtime.session().local_position,
            Some(14.5),
            "local session state should reflect the commanded seek target after applying telemetry baseline"
        );

        let drained = runtime.drain_player_playback_telemetry_updates();
        assert_eq!(
            drained,
            vec![PlayerPlaybackTelemetryUpdate::default().with_position_seconds(12.5)]
        );
    }

    #[test]
    fn client_runtime_drain_autoplay_notifications_to_sink_dispatches_callback() {
        let mut session = ClientSession::default();
        session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
            )
            .expect("hello should apply");
        session
            .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
            .expect("local ready should apply");
        session
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"bob.mp4"},"isReady":true}}}}"#,
            )
            .expect("other user ready should apply");
        session.set_autoplay_enabled(true);
        session.readiness_autoplay_config_mut().auto_play_threshold = Some(2);
        session.local_paused = Some(true);

        let player = RecordingPlayer::default();
        let control = QueuedRuntimeControl::default();
        let mut runtime = ClientRuntime::new(session, player, control);
        runtime.update_autoplay_check(true, true, false, false);
        runtime
            .tick_autoplay(true, true, false, false)
            .expect("first autoplay tick should emit notification");
        runtime
            .tick_autoplay(true, true, false, false)
            .expect("second autoplay tick should emit notification");

        let mut captured = Vec::new();
        runtime
            .drain_autoplay_notifications_to_sink(|notification| {
                captured.push(notification.clone());
                Ok::<(), ()>(())
            })
            .expect("notification sink dispatch should succeed");

        assert_eq!(
            captured,
            vec![
                AutoplayCountdownNotification {
                    ready_user_count: 2,
                    seconds_left: 3
                },
                AutoplayCountdownNotification {
                    ready_user_count: 2,
                    seconds_left: 2
                }
            ]
        );
        assert!(runtime.drain_autoplay_notifications().is_empty());
    }
}
