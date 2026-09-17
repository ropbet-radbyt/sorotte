use super::*;
use sorotte_client_app::app_boundary::application::ClientCommand;
use sorotte_client_core::{LogicalMediaId, MediaTransportKind};
use sorotte_protocol::{PlaybackBarrierPolicy, PlaybackBarrierSetExtension, RoomBufferingPolicy};

fn configured_media_request(apply_application_settings: bool) -> PlaybackBarrierSetExtension {
    let stored = parse_sorotte_ini_stored_client_settings(
        "[client_settings]\nstreamingStartPolicy = wait-all\nstreamingRoomBufferingPolicy = pause-eligible\n",
    );
    // Verify the input itself and use the exact runtime constructor called at CLI startup.
    assert_eq!(stored.streaming_start_policy.as_deref(), Some("wait-all"));
    assert_eq!(
        stored.streaming_room_buffering_policy.as_deref(),
        Some("pause-eligible")
    );
    let mut loop_config = test_client_loop_config();
    apply_stored_client_settings(&mut loop_config, &stored, |_| None);
    let (mut runtime, _) = create_client_runtime_with_prepared_mpv_and_bridge_setup_for_test(
        &loop_config,
        Some(&stored),
        MpvAdapter::simulated(),
        |_, _| SorotteBridgeHealth::Disabled,
    )
    .expect("CLI factory");
    if apply_application_settings {
        let resolved = ClientConfig::resolve(&stored);
        runtime.dispatch(ClientCommand::update_settings(
            ClientApplicationSettings::new(resolved.config),
        ));
    }
    runtime.apply_protocol_line(
        r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"sorottePlaybackBarrierV1":true}}}"#,
        1.0, false, false, false,
    ).unwrap();
    runtime
        .apply_protocol_line(
            r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"controller":true}}}}"#,
            2.0,
            false,
            false,
            false,
        )
        .unwrap();
    runtime.prepare_playback_media(
        LogicalMediaId::new("sha256:cli-saved-start").unwrap(),
        MediaTransportKind::NetworkVod,
        3.0,
    );
    runtime
        .pending_protocol_messages()
        .iter()
        .find_map(|message| match message {
            ProtocolMessage::Set(set) => set.set.playback_barrier_v1().ok().flatten(),
            _ => None,
        })
        .expect("media preparation emits its coordination request")
}

#[test]
fn saved_coordinated_start_reaches_media_request() {
    let request = configured_media_request(false);
    assert_eq!(
        request.prepare.map(|prepare| prepare.policy),
        Some(PlaybackBarrierPolicy::AllEligible)
    );
}

#[test]
fn saved_room_buffering_reaches_media_request() {
    let request = configured_media_request(false);
    assert_eq!(
        request.buffering_policy.unwrap().policy,
        RoomBufferingPolicy::PauseAnyEligible
    );
}

#[test]
fn application_settings_apply_saved_start_and_buffering() {
    let request = configured_media_request(true);
    assert_eq!(
        request.prepare.map(|prepare| prepare.policy),
        Some(PlaybackBarrierPolicy::AllEligible)
    );
    assert_eq!(
        request.buffering_policy.unwrap().policy,
        RoomBufferingPolicy::PauseAnyEligible
    );
}

#[test]
fn saved_streaming_policy_preserves_cli_behavior_overrides() {
    let stored = parse_sorotte_ini_stored_client_settings(
        "[client_settings]\nstreamingStartPolicy = wait-all\npauseOnLeave = true\nrewindThreshold = 4.0\n",
    );
    let config = ClientLoopConfig {
        pause_on_leave_override: Some(false),
        rewind_threshold_seconds_override: Some(17.0),
        ..test_client_loop_config()
    };
    let (runtime, _) = create_client_runtime_with_prepared_mpv_and_bridge_setup_for_test(
        &config,
        Some(&stored),
        MpvAdapter::simulated(),
        |_, _| SorotteBridgeHealth::Disabled,
    )
    .unwrap();
    assert!(!runtime.session().behavior_config().pause_on_leave);
    assert_eq!(
        runtime.session().desync_config().rewind_threshold_seconds,
        17.0
    );
}
