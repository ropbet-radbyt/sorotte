use super::*;

#[derive(Clone)]
pub(crate) struct ClientLoopConfig {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) server_password: Option<SecretValue>,
    pub(crate) username: String,
    pub(crate) room: String,
    pub(crate) version: String,
    pub(crate) max_retries: u32,
    pub(crate) max_connected_runtime_seconds: f64,
    pub(crate) readiness_supported_override: Option<bool>,
    pub(crate) local_can_control_override: Option<bool>,
    pub(crate) is_playing_music_override: Option<bool>,
    pub(crate) recently_advanced_override: Option<bool>,
    pub(crate) autoplay_enabled: bool,
    pub(crate) autoplay_require_same_filenames: bool,
    pub(crate) ready_at_start_override: Option<bool>,
    pub(crate) shared_playlists_enabled_override: Option<bool>,
    pub(crate) pause_on_leave_override: Option<bool>,
    pub(crate) loop_at_end_of_playlist_override: Option<bool>,
    pub(crate) loop_single_files_override: Option<bool>,
    pub(crate) only_switch_to_trusted_domains_override: Option<bool>,
    pub(crate) trusted_domains_override: Option<Vec<String>>,
    pub(crate) rewind_on_desync_override: Option<bool>,
    pub(crate) fastforward_on_desync_override: Option<bool>,
    pub(crate) slow_on_desync_override: Option<bool>,
    pub(crate) dont_slow_down_with_me_override: Option<bool>,
    pub(crate) rewind_threshold_seconds_override: Option<f64>,
    pub(crate) fastforward_threshold_seconds_override: Option<f64>,
    pub(crate) slowdown_threshold_seconds_override: Option<f64>,
    pub(crate) unpause_action_override: Option<UnpauseActionMode>,
    pub(crate) auto_play_threshold_override: Option<AutoplayThresholdOverride>,
    pub(crate) filename_privacy_mode: PrivacyMode,
    pub(crate) filesize_privacy_mode: PrivacyMode,
    pub(crate) show_duration_notification_override: Option<bool>,
    pub(crate) different_duration_threshold_seconds_override: Option<f64>,
    pub(crate) show_same_room_osd_override: Option<bool>,
    pub(crate) show_osd_warnings_override: Option<bool>,
    pub(crate) show_noncontroller_osd_override: Option<bool>,
    pub(crate) show_different_room_osd_override: Option<bool>,
    pub(crate) controlled_room_password_override: Option<SecretValue>,
}

impl std::fmt::Debug for ClientLoopConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClientLoopConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("server_password", &self.server_password)
            .field("username", &self.username)
            .field("room_configured", &!self.room.is_empty())
            .field("version", &self.version)
            .field("max_retries", &self.max_retries)
            .field(
                "controlled_room_password_override",
                &self.controlled_room_password_override,
            )
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RuntimeLoopInputs {
    pub(crate) readiness_supported: bool,
    pub(crate) local_can_control: bool,
    pub(crate) is_playing_music: bool,
    pub(crate) recently_advanced: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ClientBehaviorOverrides {
    pub(crate) pause_on_leave: Option<bool>,
    pub(crate) loop_at_end_of_playlist: Option<bool>,
    pub(crate) loop_single_files: Option<bool>,
    pub(crate) only_switch_to_trusted_domains: Option<bool>,
    pub(crate) trusted_domains: Option<Vec<String>>,
    pub(crate) reconnect_state_restore_auto_correct: Option<bool>,
    pub(crate) reconnect_state_restore_correction_policy_mode_override:
        Option<ReconnectStateRestoreCorrectionPolicyMode>,
    pub(crate) reconnect_state_restore_position_tolerance_seconds: Option<f64>,
    pub(crate) reconnect_state_restore_correction_retry_max_attempts: Option<u32>,
    pub(crate) reconnect_state_restore_correction_retry_cooldown_ticks: Option<u32>,
    pub(crate) reconnect_state_restore_correction_retry_exponential_backoff: Option<bool>,
    pub(crate) reconnect_state_restore_correction_retry_max_cooldown_ticks: Option<u32>,
    pub(crate) reconnect_state_restore_correction_retry_adaptive_cycle_backoff: Option<bool>,
    pub(crate) reconnect_state_restore_correction_retry_adaptive_cycle_budget: Option<bool>,
    pub(crate) reconnect_state_restore_correction_retry_adaptive_cycle_budget_min_attempts:
        Option<u32>,
    pub(crate) reconnect_state_restore_correction_disable_after_mismatch_cycles: Option<u32>,
    pub(crate) reconnect_state_restore_correction_disable_after_mismatch_decay_on_success:
        Option<u32>,
    pub(crate) reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ReadinessAutoplayOverrides {
    pub(crate) unpause_action: Option<UnpauseActionMode>,
    pub(crate) auto_play_threshold: Option<AutoplayThresholdOverride>,
    pub(crate) autoplay_delay_seconds: Option<f64>,
    pub(crate) last_paused_diff_threshold_seconds: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ChatPolicyOverrides {
    pub(crate) max_chat_message_length: Option<usize>,
    pub(crate) apply_server_max_chat_message_length: Option<bool>,
}
