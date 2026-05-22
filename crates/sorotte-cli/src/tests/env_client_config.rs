use super::*;

#[test]
fn parse_env_bool_legacy_compatible_parses_expected_tokens() {
    assert_eq!(parse_env_bool_legacy_compatible("1"), Some(true));
    assert_eq!(parse_env_bool_legacy_compatible("true"), Some(true));
    assert_eq!(parse_env_bool_legacy_compatible("YES"), Some(true));
    assert_eq!(parse_env_bool_legacy_compatible("on"), Some(true));
    assert_eq!(parse_env_bool_legacy_compatible("0"), Some(false));
    assert_eq!(parse_env_bool_legacy_compatible("false"), Some(false));
    assert_eq!(parse_env_bool_legacy_compatible("No"), Some(false));
    assert_eq!(parse_env_bool_legacy_compatible("off"), Some(false));
}

#[test]
fn parse_env_bool_legacy_compatible_rejects_invalid_values() {
    assert_eq!(parse_env_bool_legacy_compatible(""), None);
    assert_eq!(parse_env_bool_legacy_compatible("  "), None);
    assert_eq!(parse_env_bool_legacy_compatible("maybe"), None);
    assert_eq!(parse_env_bool_legacy_compatible("2"), None);
}

#[test]
fn parse_env_port_legacy_compatible_requires_port_range_one_to_65535() {
    assert_eq!(parse_env_port_legacy_compatible("1"), Some(1));
    assert_eq!(parse_env_port_legacy_compatible("65535"), Some(65535));
    assert_eq!(parse_env_port_legacy_compatible("0"), None);
    assert_eq!(parse_env_port_legacy_compatible("65536"), None);
    assert_eq!(parse_env_port_legacy_compatible("abc"), None);
}

#[test]
fn parse_env_non_negative_f64_legacy_compatible_requires_finite_non_negative_values() {
    assert_eq!(parse_env_non_negative_f64_legacy_compatible("0"), Some(0.0));
    assert_eq!(
        parse_env_non_negative_f64_legacy_compatible("1.25"),
        Some(1.25)
    );
    assert_eq!(parse_env_non_negative_f64_legacy_compatible("-0.01"), None);
    assert_eq!(parse_env_non_negative_f64_legacy_compatible("NaN"), None);
    assert_eq!(parse_env_non_negative_f64_legacy_compatible("inf"), None);
    assert_eq!(parse_env_non_negative_f64_legacy_compatible("abc"), None);
}

#[test]
fn parse_env_string_list_legacy_compatible_splits_and_trims_entries() {
    assert_eq!(
        parse_env_string_list_legacy_compatible(" youtube.com , *.example.com/videos ; youtu.be"),
        Some(vec![
            "youtube.com".to_owned(),
            "*.example.com/videos".to_owned(),
            "youtu.be".to_owned()
        ])
    );
    assert_eq!(
        parse_env_string_list_legacy_compatible("alpha\nbeta\r\ngamma"),
        Some(vec![
            "alpha".to_owned(),
            "beta".to_owned(),
            "gamma".to_owned()
        ])
    );
}

#[test]
fn parse_env_string_list_legacy_compatible_rejects_empty_values() {
    assert_eq!(parse_env_string_list_legacy_compatible(""), None);
    assert_eq!(parse_env_string_list_legacy_compatible(" , ; \n "), None);
}

#[test]
fn parse_unpause_action_mode_legacy_compatible_accepts_known_values() {
    assert_eq!(
        parse_unpause_action_mode_legacy_compatible("IfAlreadyReady"),
        Some(UnpauseActionMode::IfAlreadyReady)
    );
    assert_eq!(
        parse_unpause_action_mode_legacy_compatible("if_others_ready"),
        Some(UnpauseActionMode::IfOthersReady)
    );
    assert_eq!(
        parse_unpause_action_mode_legacy_compatible("if-min-users-ready"),
        Some(UnpauseActionMode::IfMinUsersReady)
    );
    assert_eq!(
        parse_unpause_action_mode_legacy_compatible("always"),
        Some(UnpauseActionMode::Always)
    );
}

#[test]
fn parse_unpause_action_mode_legacy_compatible_rejects_unknown_values() {
    assert_eq!(parse_unpause_action_mode_legacy_compatible(""), None);
    assert_eq!(
        parse_unpause_action_mode_legacy_compatible("sometimes"),
        None
    );
}

#[test]
fn parse_reconnect_state_restore_correction_policy_mode_legacy_compatible_accepts_known_values() {
    assert_eq!(
        parse_reconnect_state_restore_correction_policy_mode_legacy_compatible("auto"),
        Some(ReconnectStateRestoreCorrectionPolicyMode::AutoCorrect)
    );
    assert_eq!(
        parse_reconnect_state_restore_correction_policy_mode_legacy_compatible("notify-only"),
        Some(ReconnectStateRestoreCorrectionPolicyMode::NotifyOnly)
    );
    assert_eq!(
        parse_reconnect_state_restore_correction_policy_mode_legacy_compatible(
            "warn-only-on-exhaustion"
        ),
        Some(ReconnectStateRestoreCorrectionPolicyMode::WarnOnlyOnExhaustion)
    );
    assert_eq!(
        parse_reconnect_state_restore_correction_policy_mode_legacy_compatible(
            "disable-after-n-mismatches"
        ),
        Some(ReconnectStateRestoreCorrectionPolicyMode::DisableAfterNMismatches)
    );
}

#[test]
fn parse_reconnect_state_restore_correction_policy_mode_legacy_compatible_rejects_unknown_values() {
    assert_eq!(
        parse_reconnect_state_restore_correction_policy_mode_legacy_compatible(""),
        None
    );
    assert_eq!(
        parse_reconnect_state_restore_correction_policy_mode_legacy_compatible("retry-forever"),
        None
    );
}

#[test]
fn parse_autoplay_min_users_override_legacy_compatible_maps_legacy_ranges() {
    assert_eq!(
        parse_autoplay_min_users_override_legacy_compatible("-1"),
        Some(AutoplayThresholdOverride::Disable)
    );
    assert_eq!(
        parse_autoplay_min_users_override_legacy_compatible("0"),
        Some(AutoplayThresholdOverride::Disable)
    );
    assert_eq!(
        parse_autoplay_min_users_override_legacy_compatible("1"),
        Some(AutoplayThresholdOverride::Set(1))
    );
    assert_eq!(
        parse_autoplay_min_users_override_legacy_compatible("3"),
        Some(AutoplayThresholdOverride::Set(3))
    );
    assert_eq!(
        parse_autoplay_min_users_override_legacy_compatible("abc"),
        None
    );
}

#[test]
fn apply_client_behavior_overrides_updates_playlist_behavior_fields() {
    let mut session = ClientSession::default();
    let overrides = ClientBehaviorOverrides {
        pause_on_leave: Some(false),
        loop_at_end_of_playlist: Some(true),
        loop_single_files: Some(true),
        only_switch_to_trusted_domains: Some(false),
        trusted_domains: Some(vec![
            "youtube.com".to_owned(),
            "*.example.com/videos".to_owned(),
        ]),
        reconnect_state_restore_auto_correct: None,
        reconnect_state_restore_correction_policy_mode_override: None,
        reconnect_state_restore_position_tolerance_seconds: None,
        reconnect_state_restore_correction_retry_max_attempts: None,
        reconnect_state_restore_correction_retry_cooldown_ticks: None,
        reconnect_state_restore_correction_retry_exponential_backoff: None,
        reconnect_state_restore_correction_retry_max_cooldown_ticks: None,
        reconnect_state_restore_correction_retry_adaptive_cycle_backoff: None,
        reconnect_state_restore_correction_retry_adaptive_cycle_budget: None,
        reconnect_state_restore_correction_retry_adaptive_cycle_budget_min_attempts: None,
        reconnect_state_restore_correction_disable_after_mismatch_cycles: None,
        reconnect_state_restore_correction_disable_after_mismatch_decay_on_success: None,
        reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles: None,
    };
    apply_client_behavior_overrides(&mut session, &overrides);

    let behavior = session.behavior_config();
    assert!(!behavior.pause_on_leave);
    assert!(behavior.loop_at_end_of_playlist);
    assert!(behavior.loop_single_files);
    assert!(!behavior.only_switch_to_trusted_domains);
    assert_eq!(
        behavior.trusted_domains,
        vec!["youtube.com".to_owned(), "*.example.com/videos".to_owned()]
    );
}

#[test]
fn apply_client_behavior_overrides_updates_reconnect_restore_policy_fields() {
    let mut session = ClientSession::default();
    let overrides = ClientBehaviorOverrides {
        reconnect_state_restore_auto_correct: Some(false),
        reconnect_state_restore_correction_policy_mode_override: Some(
            ReconnectStateRestoreCorrectionPolicyMode::WarnOnlyOnExhaustion,
        ),
        reconnect_state_restore_position_tolerance_seconds: Some(2.75),
        reconnect_state_restore_correction_retry_max_attempts: Some(5),
        reconnect_state_restore_correction_retry_cooldown_ticks: Some(2),
        reconnect_state_restore_correction_retry_exponential_backoff: Some(true),
        reconnect_state_restore_correction_retry_max_cooldown_ticks: Some(9),
        reconnect_state_restore_correction_retry_adaptive_cycle_backoff: Some(true),
        reconnect_state_restore_correction_retry_adaptive_cycle_budget: Some(true),
        reconnect_state_restore_correction_retry_adaptive_cycle_budget_min_attempts: Some(2),
        reconnect_state_restore_correction_disable_after_mismatch_cycles: Some(4),
        reconnect_state_restore_correction_disable_after_mismatch_decay_on_success: Some(2),
        reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles: Some(3),
        ..ClientBehaviorOverrides::default()
    };

    apply_client_behavior_overrides(&mut session, &overrides);

    let behavior = session.behavior_config();
    assert!(
        behavior.reconnect_state_restore_auto_correct,
        "explicit correction policy override should supersede legacy auto-correct boolean"
    );
    assert_eq!(
        behavior.reconnect_state_restore_correction_policy_mode_override,
        Some(ReconnectStateRestoreCorrectionPolicyMode::WarnOnlyOnExhaustion)
    );
    assert_eq!(
        behavior.reconnect_state_restore_position_tolerance_seconds,
        2.75
    );
    assert_eq!(
        behavior.reconnect_state_restore_correction_retry_max_attempts,
        5
    );
    assert_eq!(
        behavior.reconnect_state_restore_correction_retry_cooldown_ticks,
        2
    );
    assert!(behavior.reconnect_state_restore_correction_retry_exponential_backoff);
    assert_eq!(
        behavior.reconnect_state_restore_correction_retry_max_cooldown_ticks,
        9
    );
    assert!(behavior.reconnect_state_restore_correction_retry_adaptive_cycle_backoff);
    assert!(behavior.reconnect_state_restore_correction_retry_adaptive_cycle_budget);
    assert_eq!(
        behavior.reconnect_state_restore_correction_retry_adaptive_cycle_budget_min_attempts,
        2
    );
    assert_eq!(
        behavior.reconnect_state_restore_correction_disable_after_mismatch_cycles,
        4
    );
    assert_eq!(
        behavior.reconnect_state_restore_correction_disable_after_mismatch_decay_on_success,
        2
    );
    assert_eq!(
        behavior.reconnect_state_restore_correction_recovery_cooldown_reconnect_cycles,
        3
    );
}

#[test]
fn apply_readiness_autoplay_overrides_updates_fields() {
    let mut readiness = ReadinessAutoplayConfig::default();
    let overrides = ReadinessAutoplayOverrides {
        unpause_action: Some(UnpauseActionMode::IfMinUsersReady),
        auto_play_threshold: Some(AutoplayThresholdOverride::Set(3)),
        autoplay_delay_seconds: Some(4.5),
        last_paused_diff_threshold_seconds: Some(2.25),
    };
    apply_readiness_autoplay_overrides(&mut readiness, &overrides);

    assert_eq!(readiness.unpause_action, UnpauseActionMode::IfMinUsersReady);
    assert_eq!(readiness.auto_play_threshold, Some(3));
    assert_eq!(readiness.autoplay_delay_seconds, 4.5);
    assert_eq!(readiness.last_paused_diff_threshold_seconds, 2.25);

    let disable_threshold_overrides = ReadinessAutoplayOverrides {
        auto_play_threshold: Some(AutoplayThresholdOverride::Disable),
        ..ReadinessAutoplayOverrides::default()
    };
    apply_readiness_autoplay_overrides(&mut readiness, &disable_threshold_overrides);
    assert_eq!(readiness.auto_play_threshold, None);
}

#[test]
fn apply_chat_policy_overrides_sets_max_and_disables_server_sync_by_default() {
    let mut session = ClientSession::default();
    let overrides = ChatPolicyOverrides {
        max_chat_message_length: Some(12),
        apply_server_max_chat_message_length: None,
    };
    apply_chat_policy_overrides(&mut session, &overrides);

    let chat_config = session.chat_config();
    assert_eq!(chat_config.max_chat_message_length, 12);
    assert!(!chat_config.apply_server_max_chat_message_length);
}

#[test]
fn apply_chat_policy_overrides_allows_explicit_server_sync_override() {
    let mut session = ClientSession::default();
    let overrides = ChatPolicyOverrides {
        max_chat_message_length: Some(12),
        apply_server_max_chat_message_length: Some(true),
    };
    apply_chat_policy_overrides(&mut session, &overrides);

    let chat_config = session.chat_config();
    assert_eq!(chat_config.max_chat_message_length, 12);
    assert!(chat_config.apply_server_max_chat_message_length);

    let overrides = ChatPolicyOverrides {
        max_chat_message_length: None,
        apply_server_max_chat_message_length: Some(false),
    };
    apply_chat_policy_overrides(&mut session, &overrides);
    assert!(!session.chat_config().apply_server_max_chat_message_length);
}
