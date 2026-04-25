use super::*;

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
            unpause_action: UnpauseActionMode::IfOthersReady,
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
