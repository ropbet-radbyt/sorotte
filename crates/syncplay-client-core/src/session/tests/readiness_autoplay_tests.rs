use super::*;

#[test]
fn handle_disconnect_clears_readiness_support_until_next_hello() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true,"setOthersReadiness":true}}}"#,
            )
            .expect("hello should apply");
    assert_eq!(session.server_readiness_supported(), Some(true));
    assert_eq!(session.server_set_others_readiness_supported(), Some(true));
    assert_eq!(
        session.runtime_actions_for_local_ready_toggle(true),
        vec![ClientRuntimeAction::SetReady {
            ready: true,
            manually_initiated: true,
        }]
    );
    assert_eq!(
        session.runtime_actions_for_local_user_ready_set("bob".to_owned(), true, true),
        vec![ClientRuntimeAction::SetReadyForUser {
            username: "bob".to_owned(),
            ready: true,
            manually_initiated: true,
        }]
    );

    let _ = session.handle_disconnect(200.0);
    assert_eq!(session.server_readiness_supported(), None);
    assert_eq!(session.server_set_others_readiness_supported(), None);
    assert!(
        session
            .runtime_actions_for_local_ready_toggle(true)
            .is_empty()
    );
    assert!(
        session
            .runtime_actions_for_local_user_ready_set(String::new(), true, true)
            .is_empty()
    );
    assert!(
        session
            .runtime_actions_for_local_user_ready_set("bob".to_owned(), true, true)
            .is_empty()
    );
}

#[test]
fn client_ready_setby_does_not_become_target_username() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");

    session
        .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"setBy":"bob"}}}"#)
        .expect("ready state should apply");

    assert_eq!(session.user_ready("alice"), Some(true));
    assert_eq!(
        session.user_ready("bob"),
        None,
        "setBy is metadata and should not create or update the target user"
    );
}

#[test]
fn client_ready_missing_username_targets_local_user() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");

    session
        .apply_message_json(r#"{"Set":{"ready":{"isReady":true}}}"#)
        .expect("ready state should apply");

    assert_eq!(session.user_ready("alice"), Some(true));
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
        session.instaplay_conditions_met(true, false),
        "default IfOthersReady mode should allow unpause when another ready user is present"
    );

    session.readiness_autoplay_config_mut().unpause_action = UnpauseActionMode::IfAlreadyReady;
    assert!(
        !session.instaplay_conditions_met(true, false),
        "IfAlreadyReady mode should require local ready=true"
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
        .apply_message_json(r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"isReady":false}}}}"#)
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
        .apply_message_json(r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"isReady":true}}}}"#)
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
        .apply_message_json(r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"isReady":true}}}}"#)
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
fn readiness_autoplay_config_defaults_include_legacy_duration_comparison_settings() {
    let config = ReadinessAutoplayConfig::default();
    assert!(config.show_duration_notification);
    assert_eq!(
        config.different_duration_threshold_seconds,
        LEGACY_DIFFERENT_DURATION_THRESHOLD_SECONDS
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
fn local_media_open_marks_local_user_not_ready_when_readiness_is_supported() {
    let mut session = ClientSession::default();
    session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true}}}"#,
            )
            .expect("hello should apply");

    let actions = session.runtime_actions_for_local_media_opened_not_ready();

    assert_eq!(
        actions,
        vec![ClientRuntimeAction::SetReady {
            ready: false,
            manually_initiated: false,
        }]
    );
    assert_eq!(session.user_ready("alice"), Some(false));
}

#[test]
fn runtime_actions_for_readiness_unpause_attempt_blocks_and_sets_ready_when_instaplay_fails() {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    session.readiness_autoplay_config_mut().unpause_action = UnpauseActionMode::IfAlreadyReady;

    let actions = session.runtime_actions_for_readiness_unpause_attempt(10.0, true, true, false);
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

    let actions = session.runtime_actions_for_readiness_unpause_attempt(20.0, true, true, false);
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
fn cache_pause_blocks_readiness_unpause_without_changing_ready_or_manual_pause_state() {
    let mut session = ClientSession::default();
    session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255","features":{"readiness":true}}}"#,
            )
            .expect("hello should apply");
    session
        .apply_message_json(
            r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"bob.mp4"},"isReady":true}}}}"#,
        )
        .expect("other user ready state should apply");
    session
        .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
        .expect("local ready state should apply");
    session.local_paused = Some(false);
    session.apply_player_playback_telemetry_update(
        &PlayerPlaybackTelemetryUpdate::default()
            .with_paused(true)
            .with_paused_for_cache(true)
            .with_cache_buffering_percent(42.5),
    );

    assert_eq!(session.local_paused(), Some(false));
    assert_eq!(session.local_paused_for_cache(), Some(true));
    assert_eq!(session.local_cache_buffering_percent(), Some(42.5));

    let actions = session.runtime_actions_for_readiness_unpause_attempt(20.0, true, true, false);
    assert!(actions.is_empty());
    assert_eq!(session.local_paused(), Some(false));
    assert_eq!(session.user_ready("alice"), Some(true));

    let unpause_actions = session.runtime_actions_for_local_pause_set(false);
    assert!(unpause_actions.is_empty());
    assert_eq!(session.user_ready("alice"), Some(true));

    session.apply_player_playback_telemetry_update(
        &PlayerPlaybackTelemetryUpdate::default()
            .with_paused_for_cache(false)
            .with_paused(false),
    );
    let resumed_actions =
        session.runtime_actions_for_readiness_unpause_attempt(21.0, true, true, false);
    assert!(resumed_actions.is_empty());
    assert_eq!(session.local_paused(), Some(false));
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

    let actions = session.runtime_actions_for_readiness_unpause_attempt(101.0, true, true, false);
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

    let blocked = session.runtime_actions_for_readiness_unpause_attempt(30.0, true, true, false);
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
    let allowed = session.runtime_actions_for_readiness_unpause_attempt(31.0, true, true, false);
    assert!(allowed.is_empty());
    assert_eq!(session.user_ready("alice"), Some(true));
    assert_eq!(session.local_paused(), Some(false));
}

#[test]
fn local_pause_marks_local_user_not_ready_when_readiness_is_supported() {
    let mut session = ClientSession::default();
    session
            .apply_message_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true}}}"#,
            )
            .expect("hello should apply");
    session
        .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
        .expect("local ready should apply");
    session.local_paused = Some(false);

    let actions = session.runtime_actions_for_local_pause_set(true);

    assert_eq!(
        actions,
        vec![
            ClientRuntimeAction::SetPaused(true),
            ClientRuntimeAction::SetReady {
                ready: false,
                manually_initiated: false
            }
        ]
    );
    assert_eq!(session.local_paused(), Some(true));
    assert_eq!(session.user_ready("alice"), Some(false));
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
fn autoplay_check_does_not_start_countdown_while_paused_for_cache() {
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
    session.apply_player_playback_telemetry_update(
        &PlayerPlaybackTelemetryUpdate::default().with_paused_for_cache(true),
    );

    session.autoplay_check(true, true, false, false);

    assert!(!session.autoplay_timer_is_running());
    assert_eq!(session.local_paused_for_cache(), Some(true));

    session.apply_player_playback_telemetry_update(
        &PlayerPlaybackTelemetryUpdate::default().with_paused_for_cache(false),
    );
    session.autoplay_check(true, true, false, false);

    assert!(session.autoplay_timer_is_running());
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
fn client_runtime_set_ready_for_user_is_omitted_when_remote_readiness_is_unsupported() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.1","features":{"readiness":true}}}"#,
            )
            .expect("hello should apply");
    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        !runtime
            .run_set_ready_for_user("bob", true, true)
            .expect("set ready for other user should not fail"),
        "set ready for other user should be suppressed when remote readiness changes are unavailable"
    );
    assert!(runtime.control().outbound_messages().is_empty());
}

#[test]
fn client_runtime_set_room_preserves_autoplay_state_on_room_change() {
    let mut session = ClientSession::default();
    session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .expect("hello should apply");
    session.set_autoplay_enabled(true);
    session.start_autoplay_countdown();

    let player = RecordingPlayer::default();
    let control = QueuedRuntimeControl::default();
    let mut runtime = ClientRuntime::new(session, player, control);
    assert!(
        runtime
            .run_set_room("room2")
            .expect("set room should not fail"),
        "room changes should still dispatch Set.room"
    );

    assert!(
        runtime.session().autoplay_enabled(),
        "room changes should preserve autoplay"
    );
    assert!(
        runtime.session().autoplay_timer_is_running(),
        "room changes should preserve any running autoplay countdown"
    );
    assert_eq!(
        runtime
            .session()
            .pending_local_room_switch_target
            .as_deref(),
        Some("room2")
    );
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

    assert_eq!(player.paused, None);
    assert_eq!(control.outbound_messages().len(), 1);
    let ProtocolMessage::Set(set_message) = &control.outbound_messages()[0] else {
        panic!("expected queued ready protocol message");
    };
    let ready = set_message
        .set
        .ready
        .as_ref()
        .expect("Set message should contain ready payload");
    assert_eq!(ready.is_ready, Some(true));
    assert_eq!(ready.manually_initiated, Some(false));
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
