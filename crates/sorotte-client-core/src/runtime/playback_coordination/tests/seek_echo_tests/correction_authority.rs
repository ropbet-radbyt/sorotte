use super::*;
use crate::inbound::normalize_client_state_payload;

fn correction_reply(fixture: &SeekFixture, position: f64, revision: u64) -> StatePayload {
    let mut reply = fixture.echo();
    reply.playstate = Some(
        reply
            .playstate
            .unwrap()
            .with_position(position)
            .with_transport_revision(revision),
    );
    reply
}

fn admit(fixture: &mut SeekFixture, reply: StatePayload) {
    let line = sorotte_protocol::encode_message_line(&ProtocolMessage::state(reply)).unwrap();
    fixture
        .runtime
        .session_mut()
        .apply_message_json_at(&line, 1.3)
        .unwrap();
}

#[test]
fn malformed_or_unrelated_replies_cannot_capture_seek_correction_authority() {
    for invalid in [
        "old-revision",
        "not-seek",
        "negative-position",
        "nan-position",
        "infinite-position",
        "wrong-counter",
        "missing-counter",
    ] {
        let mut fixture = SeekFixture::new();
        fixture.seek();
        let reply = correction_reply(&fixture, 0.0, 34);
        let valid = normalize_client_state_payload(reply);
        let mut inbound = valid.clone();
        let playstate = inbound.playstate.as_mut().unwrap();
        match invalid {
            "old-revision" => playstate.transport_revision = Some(33),
            "not-seek" => playstate.do_seek = Some(false),
            "negative-position" => playstate.position = Some(-1.0),
            "nan-position" => playstate.position = Some(f64::NAN),
            "infinite-position" => playstate.position = Some(f64::INFINITY),
            "wrong-counter" => inbound.ignoring_on_the_fly.as_mut().unwrap().client = Some(99),
            "missing-counter" => inbound.ignoring_on_the_fly.as_mut().unwrap().client = None,
            _ => unreachable!(),
        }
        let owner = &mut fixture.runtime.playback_coordination;
        assert!(
            owner
                .capture_local_seek_correction(&fixture.runtime.session, &inbound)
                .is_none(),
            "{invalid} must not authorize a physical correction"
        );
        assert!(
            owner
                .capture_local_seek_correction(&fixture.runtime.session, &valid)
                .is_some(),
            "{invalid} must not consume the genuine reply's identity"
        );
    }
}

#[test]
fn captured_correction_requires_unchanged_intent_scope_and_admitted_room_state() {
    for change in ["none", "new-seek", "username", "revision", "room-position"] {
        let mut fixture = SeekFixture::new();
        fixture.seek();
        let reply = correction_reply(&fixture, 0.0, 34);
        let inbound = normalize_client_state_payload(reply.clone());
        let candidate = fixture
            .runtime
            .playback_coordination
            .capture_local_seek_correction(&fixture.runtime.session, &inbound);
        assert!(candidate.is_some());
        // Exercise the boundary between capture and Session admission without
        // prematurely invoking the runtime's normal completion hook.
        admit(&mut fixture, reply);
        match change {
            "none" => {}
            "new-seek" => assert!(fixture.runtime.run_seek_to_position(12.0).unwrap()),
            "username" => fixture
                .runtime
                .session_mut()
                .apply_message_json_at(
                    r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.7.5"}}"#,
                    1.3,
                )
                .unwrap(),
            "revision" => {
                let newer = correction_reply(&fixture, 0.0, 35);
                admit(&mut fixture, newer);
            }
            "room-position" => {
                let different = correction_reply(&fixture, 5.0, 34);
                admit(&mut fixture, different);
            }
            _ => unreachable!(),
        }
        let owner = &mut fixture.runtime.playback_coordination;
        let pending = owner.unacknowledged_local_seek.clone();
        owner.finish_local_seek_correction(&fixture.runtime.session, candidate);
        if change == "none" {
            assert!(owner.local_seek_correction_is_current(&fixture.runtime.session));
            assert!(owner.unacknowledged_local_seek.is_none());
            assert!(
                owner.pending_local_transport_echo.is_none(),
                "the corrected seek must no longer hold or rebase a later Play/Pause"
            );
        } else {
            assert!(owner.rejected_local_seek.is_none(), "{change}");
            assert_eq!(owner.unacknowledged_local_seek, pending, "{change}");
        }
    }
}

#[test]
fn correction_authority_expires_when_room_revision_or_identity_changes() {
    for change in ["revision", "username"] {
        let mut fixture = SeekFixture::new();
        fixture.seek();
        assert!(
            !fixture
                .runtime
                .playback_coordination
                .local_seek_correction_is_current(&fixture.runtime.session),
            "an outstanding local seek is not itself a server correction"
        );
        let reply = correction_reply(&fixture, 0.0, 34);
        fixture.reconcile(reply);
        assert!(
            fixture
                .runtime
                .playback_coordination
                .local_seek_correction_is_current(&fixture.runtime.session)
        );
        match change {
            "revision" => {
                let newer = correction_reply(&fixture, 0.0, 35);
                admit(&mut fixture, newer);
            }
            "username" => fixture
                .runtime
                .session_mut()
                .apply_message_json_at(
                    r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.7.5"}}"#,
                    1.3,
                )
                .unwrap(),
            _ => unreachable!(),
        }
        let owner = &mut fixture.runtime.playback_coordination;
        assert!(
            !owner.local_seek_correction_is_current(&fixture.runtime.session),
            "{change} must retire the old physical correction"
        );
        assert!(owner.rejected_local_seek.is_none(), "{change}");
    }
}

#[test]
fn unchanged_base_replay_keeps_the_later_rejection_correlated() {
    let mut fixture = SeekFixture::new();
    fixture.seek();
    fixture.queue_observation(true, 11.0);
    fixture
        .runtime
        .drain_player_transport_coordination(1.2)
        .unwrap();
    let replay = correction_reply(&fixture, 11.0, 34);
    fixture.reconcile(replay);
    assert!(
        fixture
            .runtime
            .playback_coordination
            .unacknowledged_local_seek
            .is_some(),
        "repeating the base revision cannot acknowledge the newer seek"
    );
    fixture.runtime.player.commands.clear();
    let rejection = correction_reply(&fixture, 0.0, 34);
    fixture.reconcile(rejection);
    fixture.queue_observation(true, 11.0);
    fixture
        .runtime
        .drain_player_transport_coordination(1.4)
        .unwrap();
    assert!(fixture.runtime.player.commands.iter().any(
        |command| matches!(command, PlayerCommand::SetPosition(position) if position.abs() < 0.001)
    ));
}
