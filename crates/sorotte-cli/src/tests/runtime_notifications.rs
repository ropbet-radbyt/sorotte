use super::*;
use crate::notifications::{
    SeekPreparationNotificationState, next_seek_preparation_notification_messages,
};

#[test]
fn flush_autoplay_notifications_to_sink_dispatches_notifications() {
    let config = ClientLoopConfig {
        host: "127.0.0.1".to_owned(),
        port: 8999,
        server_password: None,
        username: "cli-user".to_owned(),
        room: "room1".to_owned(),
        version: "1.2.255".to_owned(),
        max_retries: 0,
        max_connected_runtime_seconds: 1.0,
        readiness_supported_override: None,
        local_can_control_override: None,
        is_playing_music_override: None,
        recently_advanced_override: None,
        autoplay_enabled: true,
        autoplay_require_same_filenames: false,
        ready_at_start_override: None,
        shared_playlists_enabled_override: None,
        pause_on_leave_override: None,
        loop_at_end_of_playlist_override: None,
        loop_single_files_override: None,
        only_switch_to_trusted_domains_override: None,
        trusted_domains_override: None,
        rewind_on_desync_override: None,
        fastforward_on_desync_override: None,
        slow_on_desync_override: None,
        dont_slow_down_with_me_override: None,
        rewind_threshold_seconds_override: None,
        fastforward_threshold_seconds_override: None,
        slowdown_threshold_seconds_override: None,
        unpause_action_override: None,
        auto_play_threshold_override: None,
        filename_privacy_mode: PrivacyMode::SendRaw,
        filesize_privacy_mode: PrivacyMode::SendRaw,
        show_duration_notification_override: None,
        different_duration_threshold_seconds_override: None,
        show_same_room_osd_override: None,
        show_osd_warnings_override: None,
        show_noncontroller_osd_override: None,
        show_different_room_osd_override: None,
        controlled_room_password_override: None,
    };
    let mut runtime = create_client_runtime(&config);
    runtime
        .session_mut()
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.2.255"}}"#,
        )
        .expect("hello should apply");
    runtime
        .session_mut()
        .apply_message_json(r#"{"Set":{"ready":{"isReady":true,"username":"alice"}}}"#)
        .expect("local ready should apply");
    runtime
            .session_mut()
            .apply_message_json(
                r#"{"Set":{"user":{"bob":{"room":{"name":"room1"},"file":{"name":"bob.mp4"},"isReady":true}}}}"#,
            )
            .expect("other user ready should apply");
    let mut readiness = runtime.session().readiness_autoplay_config().clone();
    readiness.auto_play_threshold = Some(2);
    runtime
        .session_mut()
        .set_readiness_autoplay_config(readiness);

    runtime
        .run_disconnect(0.0)
        .expect("disconnect should pause local player");
    runtime.update_autoplay_check(true, true, false, false);
    runtime
        .tick_autoplay(true, true, false, false)
        .expect("autoplay tick should emit countdown notification");

    let mut captured = Vec::new();
    flush_autoplay_notifications_to_sink(&mut runtime, &mut |notification| {
        captured.push(notification.clone());
        Ok(())
    })
    .expect("notification sink dispatch should succeed");

    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].ready_user_count, 2);
    assert_eq!(captured[0].seconds_left, 3);
}

#[test]
fn autoplay_countdown_notification_message_localized_legacy_compatible_localizes_user_visible_message()
 {
    let notification = AutoplayCountdownNotification {
        ready_user_count: 2,
        seconds_left: 3,
    };

    assert_eq!(
        crate::autoplay_countdown_notification_message_localized_legacy_compatible(
            &notification,
            Some("fr"),
        ),
        "Compte a rebours autoplay : utilisateurs_prets=2 secondes_restantes=3"
    );
    assert_eq!(
        crate::autoplay_countdown_notification_message_localized_legacy_compatible(
            &notification,
            None,
        ),
        "autoplay countdown: ready_users=2 seconds_left=3"
    );
}

#[test]
fn player_playback_telemetry_update_message_formats_present_fields() {
    let update = PlayerPlaybackTelemetryUpdate::default()
        .with_paused(true)
        .with_position_seconds(12.5)
        .with_playback_rate(0.95)
        .with_paused_for_cache(true)
        .with_cache_buffering_percent(42.5);

    let message = player_playback_telemetry_update_message(&update)
        .expect("expected telemetry message for populated update");
    assert_eq!(
        message,
        "player telemetry: paused=true position=12.500 speed=0.950 paused-for-cache=true cache-buffering=42.5%"
    );

    assert_eq!(
        player_playback_telemetry_update_message(&PlayerPlaybackTelemetryUpdate::default()),
        None
    );
}

#[test]
fn player_playback_telemetry_update_message_localized_legacy_compatible_localizes_prefix() {
    let update = PlayerPlaybackTelemetryUpdate::default()
        .with_paused(true)
        .with_position_seconds(12.5);

    let message = crate::player_playback_telemetry_update_message_localized_legacy_compatible(
        &update,
        Some("fr"),
    )
    .expect("expected telemetry message for populated update");
    assert_eq!(
        message,
        "Telemetrie du lecteur: paused=true position=12.500"
    );
}

#[test]
fn player_playback_drift_diagnostic_messages_localized_legacy_compatible_localize_labels() {
    let update = PlayerPlaybackTelemetryUpdate::default()
        .with_paused(true)
        .with_position_seconds(12.5);
    let room = sorotte_client_core::RoomPlaystateView {
        paused: Some(false),
        position: Some(10.0),
        ..sorotte_client_core::RoomPlaystateView::default()
    };

    let messages = crate::player_playback_drift_diagnostic_messages_localized_legacy_compatible(
        &update,
        Some(&room),
        Some("de"),
    );
    assert_eq!(messages.len(), 2);
    assert!(messages[0].starts_with("Player-Abweichung: Pause-Abweichung "));
    assert!(messages[1].starts_with("Player-Abweichung: Positionsabweichung "));
}

fn seek_preparation_snapshot(
    phase: sorotte_client_core::SeekPreparationPhase,
) -> sorotte_client_core::SeekPreparationSnapshot {
    sorotte_client_core::SeekPreparationSnapshot {
        id: 1,
        media_generation: 2,
        load_attempt: 1,
        room_revision: 3,
        latest_room_revision: 4,
        requested_target_seconds: 2_533.0,
        frozen_target_seconds: 2_533.0,
        frozen_room_anchor_position_seconds: 2_533.0,
        frozen_room_anchor_observed_at_seconds: 10.0,
        latest_room_position_seconds: 2_537.2,
        availability: sorotte_client_core::SeekTargetAvailability::FetchRequired,
        phase,
        cache_buffering_percent: None,
        buffered_ahead_seconds: None,
        nearest_safe_buffered_position_seconds: None,
        started_at_seconds: 10.0,
        terminal_outcome: None,
        can_keep_waiting: true,
        can_cancel_and_remain: true,
        can_join_nearest_buffered: false,
    }
}

#[test]
fn seek_preparation_diagnostics_use_truthful_nonfatal_status_labels() {
    use sorotte_client_core::{SeekPreparationPhase, SeekPreparationTerminalOutcome};

    for (phase, expected) in [
        (SeekPreparationPhase::Seeking, "Seeking to 42:13"),
        (
            SeekPreparationPhase::Fetching,
            "Fetching stream data for 42:13",
        ),
        (
            SeekPreparationPhase::ReadyToJoin,
            "Ready - joining the room",
        ),
        (SeekPreparationPhase::CatchingUp, "Catching up to the room"),
    ] {
        let messages =
            seek_preparation_diagnostic_messages(Some(&seek_preparation_snapshot(phase)), None);
        assert!(messages[0].contains(expected));
        assert!(!messages.iter().any(|line| line.contains("ETA")));
        assert!(!messages.iter().any(|line| line.contains("download")));
    }

    let mut refilling = seek_preparation_snapshot(SeekPreparationPhase::Refilling);
    refilling.cache_buffering_percent = Some(68.4);
    refilling.buffered_ahead_seconds = Some(3.8);
    refilling.nearest_safe_buffered_position_seconds = Some(2_530.0);
    refilling.can_join_nearest_buffered = true;
    let messages = seek_preparation_diagnostic_messages(Some(&refilling), None);
    assert_eq!(
        messages,
        vec![
            "seek preparation: Buffer refill: 68%; availability=fetch-required",
            "seek preparation: 3.8 seconds buffered ahead",
            "seek preparation actions: keep-waiting,join-nearest-buffered-position,cancel-and-remain",
        ]
    );

    let terminal = seek_preparation_diagnostic_messages(
        None,
        Some(SeekPreparationTerminalOutcome::Degraded(
            sorotte_client_core::SeekPreparationDegradedReason::TimedOut,
        )),
    );
    assert_eq!(
        terminal,
        vec!["seek preparation: terminal=degraded (TimedOut)"]
    );
}

#[test]
fn normal_seek_preparation_notifications_emit_changed_states_once() {
    use sorotte_client_core::{SeekPreparationPhase, SeekPreparationTerminalOutcome};

    let mut state = SeekPreparationNotificationState::default();
    let fetching = seek_preparation_snapshot(SeekPreparationPhase::Fetching);
    let first = next_seek_preparation_notification_messages(Some(&fetching), None, &mut state);
    assert!(first[0].contains("Fetching stream data"));
    assert!(first.iter().any(|line| line.contains("keep-waiting")));
    assert!(
        next_seek_preparation_notification_messages(Some(&fetching), None, &mut state).is_empty(),
        "an unchanged default-CLI projection must not flood the terminal"
    );

    let mut refilling = fetching.clone();
    refilling.phase = SeekPreparationPhase::Refilling;
    refilling.cache_buffering_percent = Some(60.0);
    let changed = next_seek_preparation_notification_messages(Some(&refilling), None, &mut state);
    assert!(changed[0].contains("Buffer refill: 60%"));

    let mut terminal = refilling;
    terminal.terminal_outcome = Some(SeekPreparationTerminalOutcome::Cancelled);
    terminal.can_keep_waiting = false;
    terminal.can_cancel_and_remain = false;
    let completed = next_seek_preparation_notification_messages(None, Some(&terminal), &mut state);
    assert_eq!(completed, vec!["seek preparation: terminal=cancelled"]);
    assert!(
        next_seek_preparation_notification_messages(None, Some(&terminal), &mut state).is_empty(),
        "a terminal outcome must be announced exactly once"
    );

    assert!(next_seek_preparation_notification_messages(None, None, &mut state).is_empty());
    terminal.id += 1;
    assert_eq!(
        next_seek_preparation_notification_messages(None, Some(&terminal), &mut state),
        vec!["seek preparation: terminal=cancelled"],
        "the same outcome from a later episode remains visible"
    );
}
