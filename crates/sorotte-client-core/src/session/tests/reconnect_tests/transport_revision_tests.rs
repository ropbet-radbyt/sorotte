use super::*;
use crate::session::LocalPauseMutationIntent;

fn joined_session() -> ClientSession {
    let mut session = ClientSession::default();
    session
        .apply_message_json_at(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
            1.0,
        )
        .unwrap();
    session
}

fn state(revision: Option<u64>, position: f64, paused: bool, seek: bool) -> StatePayload {
    let mut playstate = PlaystatePayload::new()
        .with_position(position)
        .with_paused(paused)
        .with_do_seek(seek)
        .with_set_by("bob");
    if let Some(revision) = revision {
        playstate = playstate.with_transport_revision(revision);
    }
    StatePayload::new().with_playstate(playstate)
}

fn reconcile(
    session: &mut ClientSession,
    state: StatePayload,
    position: f64,
    paused: bool,
    now: f64,
    intent: Option<LocalPauseMutationIntent>,
) -> StatePayload {
    session.reconcile_state_and_build_response_at_with_pause_mutation_intent(
        state, position, paused, 0.0, 0.0, now, intent,
    )
}

#[test]
fn ping_and_sampled_reconciliation_reject_retired_revisions_without_poisoning_evidence() {
    for ping_only in [true, false] {
        for baseline in [None, Some(12)] {
            for (revision, rejected) in [
                (Some(0), true),
                (Some(11), baseline.is_some()),
                (None, baseline.is_some()),
                (Some(12), false),
                (Some(13), false),
            ] {
                for seek in [false, true] {
                    let mut session = joined_session();
                    if let Some(revision) = baseline {
                        session
                            .apply_protocol_message_at(
                                ProtocolMessage::state(state(Some(revision), 10.0, true, false)),
                                2.0,
                            )
                            .unwrap();
                    }
                    let incoming = state(revision, 20.0, false, seek);
                    let response = if ping_only {
                        session.reconcile_ping_only_state_response(
                            crate::normalize_client_state_payload(incoming),
                            0.0,
                            0.0,
                            3.0,
                        )
                    } else {
                        reconcile(&mut session, incoming, 10.0, true, 3.0, None)
                    };
                    let expected_revision = if rejected { baseline } else { revision };
                    assert_eq!(session.current_room_transport_revision(), expected_revision);
                    assert_eq!(
                        session
                            .current_room_playstate()
                            .and_then(|value| value.position),
                        if rejected {
                            baseline.map(|_| 10.0)
                        } else {
                            Some(20.0)
                        },
                    );
                    let must_stage =
                        !rejected && revision.is_some() && (revision != baseline || seek);
                    assert_eq!(
                        session.pending_playstate_transport_evidence.is_some(),
                        must_stage,
                        "ping_only={ping_only}, baseline={baseline:?}, revision={revision:?}, seek={seek}",
                    );
                    if rejected || ping_only || must_stage {
                        assert!(response.playstate.is_none());
                    }
                }
            }
        }
    }
}

#[test]
fn post_seek_echo_requires_the_position_at_the_observed_playback_time() {
    for (paused, observed, now, position, accepted) in [
        (false, 10.0, 14.0, 24.0, true),
        (false, 10.0, 14.0, 23.0, true),
        (false, 10.0, 14.0, 25.0, true),
        (false, 10.0, 14.0, 22.99, false),
        (false, 10.0, 14.0, 25.01, false),
        (false, 10.0, 14.0, 20.0, false),
        (false, 10.0, 8.0, 20.0, true),
        (false, 10.0, f64::NAN, 20.0, true),
        (false, 10.0, f64::INFINITY, 20.0, true),
        (false, f64::NAN, 14.0, 20.0, true),
        (false, f64::INFINITY, 14.0, 20.0, true),
        (true, 10.0, 14.0, 20.0, true),
        (true, 10.0, 14.0, 24.0, false),
    ] {
        let mut session = joined_session();
        let first = reconcile(
            &mut session,
            state(Some(12), 20.0, paused, true),
            4.0,
            paused,
            observed,
            None,
        );
        assert!(
            first.playstate.is_none(),
            "the pre-seek sample cannot acknowledge revision 12"
        );
        let response = reconcile(
            &mut session,
            StatePayload::new(),
            position,
            paused,
            now,
            None,
        );
        assert_eq!(
            response.playstate.is_some(),
            accepted,
            "paused={paused}, observed={observed}, now={now}, position={position}",
        );
        if let Some(playstate) = response.playstate {
            assert_eq!(playstate.position, Some(position));
            assert_eq!(playstate.transport_revision().unwrap(), Some(12));
        }
    }
}

#[test]
fn local_pause_intent_supersedes_only_its_exact_opposite_canonical_revision() {
    for (base, paused, supersedes) in [
        (Some(12), true, true),
        (Some(11), true, false),
        (Some(12), false, false),
        (None, true, false),
    ] {
        let mut session = joined_session();
        let initial = reconcile(
            &mut session,
            state(Some(12), 10.0, false, false),
            10.0,
            true,
            2.0,
            None,
        );
        assert!(initial.playstate.is_none());
        let response = reconcile(
            &mut session,
            StatePayload::new(),
            10.0,
            true,
            2.1,
            Some(LocalPauseMutationIntent {
                paused,
                base_transport_revision: base,
            }),
        );
        assert_eq!(
            response.playstate.is_some(),
            supersedes,
            "base={base:?}, paused={paused}"
        );
        assert_eq!(
            session.pending_playstate_transport_evidence.is_none(),
            supersedes
        );
    }
}

#[test]
fn first_revision_can_be_superseded_only_by_matching_new_local_intent() {
    for (base, paused, supersedes) in [
        (Some(12), false, true),
        (Some(11), false, false),
        (Some(12), true, false),
        (None, false, false),
    ] {
        let mut session = joined_session();
        let response = reconcile(
            &mut session,
            state(Some(12), 10.0, true, false),
            10.0,
            false,
            2.0,
            Some(LocalPauseMutationIntent {
                paused,
                base_transport_revision: base,
            }),
        );
        assert_eq!(
            response.playstate.is_some(),
            supersedes,
            "base={base:?}, paused={paused}"
        );
        assert_eq!(
            session.pending_playstate_transport_evidence.is_none(),
            supersedes
        );
    }
}

#[test]
fn initial_legacy_self_echo_preserves_the_newer_local_position() {
    for (author, expected_position) in [("alice", 12.5), ("bob", 10.0)] {
        let mut session = joined_session();
        let incoming = StatePayload::new().with_playstate(
            PlaystatePayload::new()
                .with_position(10.0)
                .with_paused(true)
                .with_set_by(author),
        );
        let response = reconcile(&mut session, incoming, 12.5, true, 2.0, None);
        assert_eq!(
            response.playstate.unwrap().position,
            Some(expected_position)
        );
        assert_eq!(session.local_position_seconds(), Some(12.5));
    }
}
