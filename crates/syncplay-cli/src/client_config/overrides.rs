use super::*;

pub(super) fn behavior_overrides_from_env() -> ClientBehaviorOverrides {
    ClientBehaviorOverrides {
        pause_on_leave: env_flag_override("SYNCPLAY_CLIENT_PAUSE_ON_LEAVE"),
        loop_at_end_of_playlist: env_flag_override("SYNCPLAY_CLIENT_LOOP_AT_END_OF_PLAYLIST"),
        loop_single_files: env_flag_override("SYNCPLAY_CLIENT_LOOP_SINGLE_FILES"),
        only_switch_to_trusted_domains: env_flag_override(
            "SYNCPLAY_CLIENT_ONLY_SWITCH_TO_TRUSTED_DOMAINS",
        ),
        trusted_domains: env_string_list("SYNCPLAY_CLIENT_TRUSTED_DOMAINS"),
        reconnect_state_restore_auto_correct: env_flag_override(
            "SYNCPLAY_CLIENT_RECONNECT_RESTORE_AUTOCORRECT",
        ),
        reconnect_state_restore_correction_policy_mode_override: env_trimmed(
            "SYNCPLAY_CLIENT_RECONNECT_RESTORE_CORRECTION_POLICY",
        )
        .and_then(|value| {
            parse_reconnect_state_restore_correction_policy_mode_legacy_compatible(&value)
        }),
        reconnect_state_restore_position_tolerance_seconds: env_non_negative_f64(
            "SYNCPLAY_CLIENT_RECONNECT_RESTORE_POSITION_TOLERANCE_SECONDS",
        ),
        reconnect_state_restore_correction_retry_max_attempts: env_u32(
            "SYNCPLAY_CLIENT_RECONNECT_RESTORE_CORRECTION_RETRY_MAX_ATTEMPTS",
        ),
        reconnect_state_restore_correction_retry_cooldown_ticks: env_u32(
            "SYNCPLAY_CLIENT_RECONNECT_RESTORE_CORRECTION_RETRY_COOLDOWN_TICKS",
        ),
        reconnect_state_restore_correction_retry_exponential_backoff: env_flag_override(
            "SYNCPLAY_CLIENT_RECONNECT_RESTORE_CORRECTION_RETRY_EXPONENTIAL_BACKOFF",
        ),
        reconnect_state_restore_correction_retry_max_cooldown_ticks: env_u32(
            "SYNCPLAY_CLIENT_RECONNECT_RESTORE_CORRECTION_RETRY_MAX_COOLDOWN_TICKS",
        ),
        reconnect_state_restore_correction_retry_adaptive_cycle_backoff: env_flag_override(
            "SYNCPLAY_CLIENT_RECONNECT_RESTORE_CORRECTION_RETRY_ADAPTIVE_CYCLE_BACKOFF",
        ),
        reconnect_state_restore_correction_retry_adaptive_cycle_budget: env_flag_override(
            "SYNCPLAY_CLIENT_RECONNECT_RESTORE_CORRECTION_RETRY_ADAPTIVE_CYCLE_BUDGET",
        ),
        reconnect_state_restore_correction_retry_adaptive_cycle_budget_min_attempts: env_u32(
            "SYNCPLAY_CLIENT_RECONNECT_RESTORE_CORRECTION_RETRY_ADAPTIVE_CYCLE_BUDGET_MIN_ATTEMPTS",
        ),
        reconnect_state_restore_correction_disable_after_mismatch_cycles: env_u32(
            "SYNCPLAY_CLIENT_RECONNECT_RESTORE_CORRECTION_DISABLE_AFTER_MISMATCHES",
        ),
        reconnect_state_restore_correction_disable_after_mismatch_decay_on_success: env_u32(
            "SYNCPLAY_CLIENT_RECONNECT_RESTORE_CORRECTION_DISABLE_AFTER_MISMATCH_DECAY_ON_SUCCESS",
        ),
        reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles: env_u32(
            "SYNCPLAY_CLIENT_RECONNECT_RESTORE_CORRECTION_RECOVERY_COOLDOWN_RECONNECT_CYCLES",
        ),
    }
}

pub(crate) fn parse_reconnect_state_restore_correction_policy_mode_legacy_compatible(
    value: &str,
) -> Option<ReconnectStateRestoreCorrectionPolicyMode> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "auto" | "autocorrect" | "auto_correct" | "auto-correct" => {
            Some(ReconnectStateRestoreCorrectionPolicyMode::AutoCorrect)
        }
        "notifyonly" | "notify_only" | "notify-only" | "warning_only" | "warning-only" => {
            Some(ReconnectStateRestoreCorrectionPolicyMode::NotifyOnly)
        }
        "warnonlyonexhaustion"
        | "warn_only_on_exhaustion"
        | "warn-only-on-exhaustion"
        | "warning_only_on_exhaustion"
        | "warning-only-on-exhaustion" => {
            Some(ReconnectStateRestoreCorrectionPolicyMode::WarnOnlyOnExhaustion)
        }
        "disableafternmismatches" | "disable_after_n_mismatches" | "disable-after-n-mismatches" => {
            Some(ReconnectStateRestoreCorrectionPolicyMode::DisableAfterNMismatches)
        }
        _ => None,
    }
}

pub(super) fn readiness_overrides_from_env() -> ReadinessAutoplayOverrides {
    ReadinessAutoplayOverrides {
        unpause_action: env_trimmed("SYNCPLAY_CLIENT_UNPAUSE_ACTION")
            .and_then(|value| parse_unpause_action_mode_legacy_compatible(&value)),
        auto_play_threshold: env_trimmed("SYNCPLAY_CLIENT_AUTOPLAY_MIN_USERS")
            .and_then(|value| parse_autoplay_min_users_override_legacy_compatible(&value)),
        autoplay_delay_seconds: env_non_negative_f64("SYNCPLAY_CLIENT_AUTOPLAY_DELAY_SECONDS"),
        last_paused_diff_threshold_seconds: env_non_negative_f64(
            "SYNCPLAY_CLIENT_LAST_PAUSED_DIFF_THRESHOLD_SECONDS",
        ),
    }
}

pub(crate) fn apply_readiness_autoplay_overrides(
    readiness_config: &mut ReadinessAutoplayConfig,
    overrides: &ReadinessAutoplayOverrides,
) {
    if let Some(unpause_action) = overrides.unpause_action.clone() {
        readiness_config.unpause_action = unpause_action;
    }
    if let Some(auto_play_threshold) = overrides.auto_play_threshold.as_ref() {
        readiness_config.auto_play_threshold = match auto_play_threshold {
            AutoplayThresholdOverride::Disable => None,
            AutoplayThresholdOverride::Set(value) => Some(*value),
        };
    }
    if let Some(autoplay_delay_seconds) = overrides.autoplay_delay_seconds {
        readiness_config.autoplay_delay_seconds = autoplay_delay_seconds;
    }
    if let Some(last_paused_diff_threshold_seconds) = overrides.last_paused_diff_threshold_seconds {
        readiness_config.last_paused_diff_threshold_seconds = last_paused_diff_threshold_seconds;
    }
}

pub(super) fn chat_policy_overrides_from_env() -> ChatPolicyOverrides {
    ChatPolicyOverrides {
        max_chat_message_length: env_usize("SYNCPLAY_CLIENT_CHAT_MAX_LENGTH"),
        apply_server_max_chat_message_length: env_flag_override(
            "SYNCPLAY_CLIENT_APPLY_SERVER_CHAT_MAX_LENGTH",
        ),
    }
}

pub(crate) fn apply_chat_policy_overrides(
    session: &mut ClientSession,
    overrides: &ChatPolicyOverrides,
) {
    let chat_config = session.chat_config_mut();
    if let Some(max_chat_message_length) = overrides.max_chat_message_length {
        chat_config.max_chat_message_length = max_chat_message_length;
        if overrides.apply_server_max_chat_message_length.is_none() {
            chat_config.apply_server_max_chat_message_length = false;
        }
    }
    if let Some(apply_server_max_chat_message_length) =
        overrides.apply_server_max_chat_message_length
    {
        chat_config.apply_server_max_chat_message_length = apply_server_max_chat_message_length;
    }
}

pub(crate) fn apply_client_behavior_overrides(
    session: &mut ClientSession,
    overrides: &ClientBehaviorOverrides,
) {
    let behavior = session.behavior_config_mut();
    if let Some(pause_on_leave) = overrides.pause_on_leave {
        behavior.pause_on_leave = pause_on_leave;
    }
    if let Some(loop_at_end_of_playlist) = overrides.loop_at_end_of_playlist {
        behavior.loop_at_end_of_playlist = loop_at_end_of_playlist;
    }
    if let Some(loop_single_files) = overrides.loop_single_files {
        behavior.loop_single_files = loop_single_files;
    }
    if let Some(only_switch_to_trusted_domains) = overrides.only_switch_to_trusted_domains {
        behavior.only_switch_to_trusted_domains = only_switch_to_trusted_domains;
    }
    if let Some(trusted_domains) = overrides.trusted_domains.clone() {
        behavior.trusted_domains = trusted_domains;
    }
    if let Some(reconnect_state_restore_auto_correct) =
        overrides.reconnect_state_restore_auto_correct
    {
        behavior.reconnect_state_restore_auto_correct = reconnect_state_restore_auto_correct;
    }
    if let Some(reconnect_state_restore_correction_policy_mode_override) =
        overrides.reconnect_state_restore_correction_policy_mode_override
    {
        behavior.reconnect_state_restore_correction_policy_mode_override =
            Some(reconnect_state_restore_correction_policy_mode_override);
        behavior.reconnect_state_restore_auto_correct = !matches!(
            reconnect_state_restore_correction_policy_mode_override,
            ReconnectStateRestoreCorrectionPolicyMode::NotifyOnly
        );
    }
    if let Some(reconnect_state_restore_position_tolerance_seconds) =
        overrides.reconnect_state_restore_position_tolerance_seconds
    {
        behavior.reconnect_state_restore_position_tolerance_seconds =
            reconnect_state_restore_position_tolerance_seconds;
    }
    if let Some(reconnect_state_restore_correction_retry_max_attempts) =
        overrides.reconnect_state_restore_correction_retry_max_attempts
    {
        behavior.reconnect_state_restore_correction_retry_max_attempts =
            reconnect_state_restore_correction_retry_max_attempts;
    }
    if let Some(reconnect_state_restore_correction_retry_cooldown_ticks) =
        overrides.reconnect_state_restore_correction_retry_cooldown_ticks
    {
        behavior.reconnect_state_restore_correction_retry_cooldown_ticks =
            reconnect_state_restore_correction_retry_cooldown_ticks;
    }
    if let Some(reconnect_state_restore_correction_retry_exponential_backoff) =
        overrides.reconnect_state_restore_correction_retry_exponential_backoff
    {
        behavior.reconnect_state_restore_correction_retry_exponential_backoff =
            reconnect_state_restore_correction_retry_exponential_backoff;
    }
    if let Some(reconnect_state_restore_correction_retry_max_cooldown_ticks) =
        overrides.reconnect_state_restore_correction_retry_max_cooldown_ticks
    {
        behavior.reconnect_state_restore_correction_retry_max_cooldown_ticks =
            reconnect_state_restore_correction_retry_max_cooldown_ticks;
    }
    if let Some(reconnect_state_restore_correction_retry_adaptive_cycle_backoff) =
        overrides.reconnect_state_restore_correction_retry_adaptive_cycle_backoff
    {
        behavior.reconnect_state_restore_correction_retry_adaptive_cycle_backoff =
            reconnect_state_restore_correction_retry_adaptive_cycle_backoff;
    }
    if let Some(reconnect_state_restore_correction_retry_adaptive_cycle_budget) =
        overrides.reconnect_state_restore_correction_retry_adaptive_cycle_budget
    {
        behavior.reconnect_state_restore_correction_retry_adaptive_cycle_budget =
            reconnect_state_restore_correction_retry_adaptive_cycle_budget;
    }
    if let Some(reconnect_state_restore_correction_retry_adaptive_cycle_budget_min_attempts) =
        overrides.reconnect_state_restore_correction_retry_adaptive_cycle_budget_min_attempts
    {
        behavior.reconnect_state_restore_correction_retry_adaptive_cycle_budget_min_attempts =
            reconnect_state_restore_correction_retry_adaptive_cycle_budget_min_attempts;
    }
    if let Some(reconnect_state_restore_correction_disable_after_mismatch_cycles) =
        overrides.reconnect_state_restore_correction_disable_after_mismatch_cycles
    {
        behavior.reconnect_state_restore_correction_disable_after_mismatch_cycles =
            reconnect_state_restore_correction_disable_after_mismatch_cycles;
    }
    if let Some(reconnect_state_restore_correction_disable_after_mismatch_decay_on_success) =
        overrides.reconnect_state_restore_correction_disable_after_mismatch_decay_on_success
    {
        behavior.reconnect_state_restore_correction_disable_after_mismatch_decay_on_success =
            reconnect_state_restore_correction_disable_after_mismatch_decay_on_success;
    }
    if let Some(reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles) =
        overrides.reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles
    {
        behavior.reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles =
            reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles;
    }
}
