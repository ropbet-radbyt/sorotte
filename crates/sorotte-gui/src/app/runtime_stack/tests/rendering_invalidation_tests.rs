use super::*;
use crate::app::feature_slices::GuiRuntimeInput;
use crate::app::runtime_stack::test_support::GuiSessionDeliveryTestExt;
use crate::app::support::system_time_seconds;
use crate::app::testing::support::runtime_state_for_shell;

fn playing_session() -> (
    SorotteGuiShellAppState,
    GuiClientCoreChatSessionRuntimeAdapter,
) {
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        username: Some("alice".to_owned()),
        room: Some("room1".to_owned()),
        ..StoredClientSettings::default()
    });
    let mut session = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("session should initialize");
    sync_adapter_to_saved_session_settings(&mut session, &state);
    session
        .deliver_outbound_protocol_lines()
        .expect("startup Hello should be delivered");
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true}}}"#,
        )
        .expect("server Hello should apply");
    session
        .apply_message_json(
            r#"{"State":{"playstate":{"position":42.0,"paused":false,"setBy":"alice"}}}"#,
        )
        .expect("playing room sample should apply");
    for action in session.drain_gui_actions(&runtime_state_for_shell(&state)) {
        assert!(state.apply(action));
    }
    (state, session)
}

#[test]
fn playing_room_projection_settles_without_new_external_input() {
    let (mut state, mut session) = playing_session();
    let input = GuiRuntimeInput::from_shell(&state);
    let mut main_window_updates = 0;
    for _ in 0..32 {
        for action in session.drain_gui_actions(&runtime_state_for_shell(&state)) {
            main_window_updates += usize::from(matches!(
                action,
                GuiShellAction::ApplyMainWindowRuntimeSnapshot(_)
            ));
            assert!(state.apply(action));
        }
    }
    assert_eq!(
        main_window_updates, 0,
        "sampling the same playing room must not manufacture fresh GUI output on every poll"
    );
    assert!(
        input.matches_shell(&state),
        "clock rendering must not invalidate worker input"
    );
}

#[test]
fn playing_room_clock_advances_without_replacing_its_sample() {
    let (state, _) = playing_session();
    let clock = &state.main_window.room_playback_intent;
    let sampled_at = clock
        .position_sampled_at
        .expect("playing clock has an anchor");
    let position = clock
        .position_seconds
        .expect("room position should be known");
    assert_eq!(
        clock.position_at(sampled_at + std::time::Duration::from_millis(1250)),
        Some(position + 1.25)
    );
    let input = GuiRuntimeInput::from_shell(&state);
    let _ = state.main_window.room_playback_intent.status_label();
    assert!(input.matches_shell(&state));
}

#[test]
fn room_clock_accepts_new_samples_pause_seek_and_resume_then_settles() {
    let (mut state, mut session) = playing_session();
    let old_clock = state.main_window.room_playback_intent.clone();
    // An identical payload received at a different time is still a new sample.
    session
        .apply_message_json_at(
            r#"{"State":{"playstate":{"position":42.0,"paused":false,"setBy":"alice"}}}"#,
            system_time_seconds() - 2.0,
        )
        .expect("new receipt should apply");
    for action in session.drain_gui_actions(&runtime_state_for_shell(&state)) {
        assert!(state.apply(action));
    }
    assert!(
        state
            .main_window
            .room_playback_intent
            .position_seconds
            .unwrap()
            > old_clock.position_seconds.unwrap() + 1.0
    );

    for (position, paused) in [(90.0, true), (15.0, true), (15.0, false)] {
        session.apply_message_json(&serde_json::json!({
            "State": {"playstate": {"position": position, "paused": paused, "setBy": "alice"}}
        }).to_string()).expect("room playback change should apply");
        for action in session.drain_gui_actions(&runtime_state_for_shell(&state)) {
            assert!(state.apply(action));
        }
        let clock = state.main_window.room_playback_intent.clone();
        assert_eq!(clock.paused, Some(paused));
        assert_eq!(clock.position_sampled_at.is_none(), paused);
        if paused {
            assert_eq!(clock.position_seconds, Some(position));
        }
        assert!(
            !session
                .drain_gui_actions(&runtime_state_for_shell(&state))
                .iter()
                .any(|action| {
                    matches!(action, GuiShellAction::ApplyMainWindowRuntimeSnapshot(_))
                })
        );
        assert_eq!(state.main_window.room_playback_intent, clock);
    }
}
