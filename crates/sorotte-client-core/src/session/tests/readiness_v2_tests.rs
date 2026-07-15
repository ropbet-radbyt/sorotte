use super::*;
use crate::{LogicalMediaId, MediaTransportKind};
use sorotte_protocol::{
    DirectReadinessSurface, ParticipantReadinessUpdate, ReadinessMutationSource,
    ReadinessRequestResultPayload, ReadinessRequestResultStatus, ReadinessSetExtension,
    RoomPauseOwner, RoomReadinessSnapshot, RoomStartGatePhase, StartParticipationRole,
    TechnicalBlockCause, TechnicalPlayabilityPhase, TechnicalPlayabilitySummary,
    UserReadinessIntent, UserReadinessMutationSource,
};

fn participant(
    revision: u64,
    membership_epoch: u64,
    intent: UserReadinessIntent,
    accepted_operation_id: Option<String>,
) -> ParticipantReadinessUpdate {
    ParticipantReadinessUpdate {
        room_readiness_revision: revision,
        membership_epoch,
        last_technical_report_sequence: 0,
        username: "alice".to_owned(),
        user_intent: intent,
        user_intent_revision: revision,
        user_intent_source: ReadinessMutationSource::Initialization,
        last_user_mutation: None,
        terminal_technical_block: None,
        technical_state: TechnicalPlayabilitySummary {
            phase: TechnicalPlayabilityPhase::Playable,
            media_generation: Some(7),
            reason: None,
            recovery: None,
        },
        participation_role: StartParticipationRole::Required,
        room_ready: intent == UserReadinessIntent::Ready,
        start_eligible: intent == UserReadinessIntent::Ready,
        accepted_operation_id,
    }
}

fn snapshot(
    revision: u64,
    membership_epoch: u64,
    intent: UserReadinessIntent,
    accepted_operation_id: Option<String>,
) -> RoomReadinessSnapshot {
    RoomReadinessSnapshot {
        room_readiness_revision: revision,
        media_generation: Some(7),
        start_gate_phase: RoomStartGatePhase::WaitingForIntent {
            media_generation: 7,
        },
        pause_owner: RoomPauseOwner::ReadinessStartGate {
            media_generation: 7,
        },
        mixed_readiness_policy: Default::default(),
        participants: [(
            "alice".to_owned(),
            participant(revision, membership_epoch, intent, accepted_operation_id),
        )]
        .into_iter()
        .collect(),
    }
}

fn active_v2_session() -> ClientSession {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true,"sorotteReadinessV2":true,"sorottePlaybackBarrierV1":true}}}"#,
        )
        .expect("V2 Hello should apply");
    session.apply_readiness_v2_extension(ReadinessSetExtension::new().with_snapshot(snapshot(
        1,
        41,
        UserReadinessIntent::NotReady,
        None,
    )));
    session
}

#[derive(Default)]
struct RejectingReadinessControl;

impl ClientEffectSink for RejectingReadinessControl {
    fn emit(&mut self, effect: ClientEffect) -> Result<(), ClientEffectError> {
        if matches!(effect, ClientEffect::SendReadinessIntent { .. }) {
            return Err(ClientEffectError::OperationFailed(
                "forced readiness delivery failure".to_owned(),
            ));
        }
        Ok(())
    }
}

#[test]
fn v2_play_stages_ready_without_dispatching_a_physical_resume() {
    let mut session = active_v2_session();
    session.model.playback.local_paused = Some(true);
    let player = RecordingPlayer {
        fail_set_paused: true,
        ..RecordingPlayer::default()
    };
    let mut runtime = ClientRuntime::new(session, player, QueuedRuntimeControl::default());
    runtime.prepare_playback_media(
        LogicalMediaId::new("readiness-failure-media").expect("logical ID should be valid"),
        MediaTransportKind::LocalFile,
        1.0,
    );
    runtime.flush_queued_protocol_messages();

    assert!(
        runtime
            .run_set_paused(false)
            .expect("V2 Play should stage semantic intent without touching the player")
    );
    assert_eq!(
        runtime
            .session()
            .pending_readiness_intent()
            .map(|intent| intent.desired()),
        Some(UserReadinessIntent::Ready),
        "the user's Play intent should remain pending until server acknowledgement"
    );
    assert_eq!(runtime.session().user_ready("alice"), Some(false));
    assert_eq!(
        runtime.session().displayed_user_readiness_intent("alice"),
        Some(UserReadinessIntent::Ready),
        "optimistic presentation must not overwrite the canonical readiness projection"
    );
    assert!(
        !runtime
            .player()
            .player_effects
            .contains(&ClientEffect::SetPlayerPaused(false)),
        "V2 Play must wait for CommitStart before resuming the player"
    );

    let messages = runtime.control().outbound_messages();
    assert!(!messages.iter().any(|message| {
        let ProtocolMessage::State(state) = message else {
            return false;
        };
        state
            .state
            .playstate
            .as_ref()
            .and_then(|state| state.paused)
            == Some(false)
    }));

    let intent = messages.iter().find_map(|message| {
        let ProtocolMessage::Set(set) = message else {
            return None;
        };
        set.set
            .readiness_v2()
            .expect("readiness extension should decode")
            .and_then(|extension| extension.intent)
    });
    assert_eq!(
        intent.map(|intent| intent.desired),
        Some(UserReadinessIntent::Ready)
    );

    runtime
        .session_mut_for_test()
        .reset_sync_state_for_reconnect();
    assert_eq!(
        runtime.session().user_ready("alice"),
        Some(false),
        "reconnect must not copy the pending Ready overlay into canonical user state"
    );
    assert_eq!(
        runtime.session().displayed_user_readiness_intent("alice"),
        Some(UserReadinessIntent::Ready),
        "the pending intent remains presentation state while awaiting reconciliation"
    );
}

#[test]
fn direct_intent_uses_membership_revision_and_operation_identity() {
    let mut session = active_v2_session();
    let actions = session.runtime_actions_for_direct_readiness_intent(
        UserReadinessIntent::Ready,
        DirectReadinessSurface::GuiButton,
        None,
    );

    let [ClientRuntimeAction::SetReadinessIntent { request, .. }] = actions.as_slice() else {
        panic!("V2 readiness should create one semantic intent action");
    };
    assert_eq!(request.membership_epoch, 41);
    assert_eq!(request.expected_user_intent_revision, Some(1));
    assert_eq!(request.desired, UserReadinessIntent::Ready);
    assert_eq!(
        request.source,
        UserReadinessMutationSource::DirectUser {
            surface: DirectReadinessSurface::GuiButton
        }
    );
    assert_eq!(
        session
            .pending_readiness_intent()
            .expect("intent should remain pending")
            .operation_id(),
        request.operation_id
    );
}

#[test]
fn intent_cas_uses_target_user_revision_not_room_technical_revision() {
    let mut session = active_v2_session();
    let mut technical_update = snapshot(2, 41, UserReadinessIntent::NotReady, None);
    let alice = technical_update
        .participants
        .get_mut("alice")
        .expect("fixture should include Alice");
    alice.user_intent_revision = 1;
    alice.technical_state.phase = TechnicalPlayabilityPhase::TemporarilyBlocked;
    alice.technical_state.reason = Some(TechnicalBlockCause::Rebuffering);
    session
        .apply_readiness_v2_extension(ReadinessSetExtension::new().with_snapshot(technical_update));

    let actions = session.runtime_actions_for_direct_readiness_intent(
        UserReadinessIntent::Ready,
        DirectReadinessSurface::GuiButton,
        None,
    );
    let [ClientRuntimeAction::SetReadinessIntent { request, .. }] = actions.as_slice() else {
        panic!("V2 readiness should create one semantic intent action");
    };
    assert_eq!(request.expected_user_intent_revision, Some(1));
    assert_ne!(
        request.expected_user_intent_revision,
        session
            .readiness_snapshot()
            .map(|snapshot| snapshot.room_readiness_revision)
    );
}

#[test]
fn room_entry_initialization_has_distinct_v2_provenance() {
    let mut session = active_v2_session();
    let actions = session.runtime_actions_for_initial_readiness_intent(UserReadinessIntent::Ready);

    let [ClientRuntimeAction::SetReadinessIntent { request, .. }] = actions.as_slice() else {
        panic!("V2 initialization should create one semantic intent action");
    };
    assert_eq!(request.desired, UserReadinessIntent::Ready);
    assert_eq!(request.source, UserReadinessMutationSource::Initialization);
    assert_eq!(request.membership_epoch, 41);
    assert_eq!(request.expected_user_intent_revision, Some(1));
    assert!(
        session
            .runtime_actions_for_initial_readiness_intent(UserReadinessIntent::NotReady)
            .is_empty(),
        "canonical Not Ready initialization is a V2 no-op"
    );
}

#[test]
fn legacy_room_entry_initialization_remains_non_manual() {
    let mut session = ClientSession::default();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true}}}"#,
        )
        .expect("legacy readiness Hello should apply");

    for (desired, ready) in [
        (UserReadinessIntent::NotReady, false),
        (UserReadinessIntent::Ready, true),
    ] {
        assert_eq!(
            session.runtime_actions_for_initial_readiness_intent(desired),
            vec![ClientRuntimeAction::SetReady {
                ready,
                manually_initiated: false,
            }]
        );
    }
}

#[test]
fn hello_activates_v2_readiness_delivery_without_a_seed_state() {
    let mut runtime = ClientRuntime::new(
        ClientSession::default(),
        RecordingPlayer::default(),
        QueuedRuntimeControl::default(),
    );
    runtime.begin_protocol_connection_generation();
    runtime
        .session_mut()
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true,"sorotteReadinessV2":true,"sorottePlaybackBarrierV1":true}}}"#,
        )
        .expect("V2 Hello should activate the connection generation");
    runtime
        .session_mut()
        .apply_protocol_message(ProtocolMessage::set(
            sorotte_protocol::SetPayload::new().with_readiness_v2(
                ReadinessSetExtension::new().with_snapshot(snapshot(
                    1,
                    41,
                    UserReadinessIntent::NotReady,
                    None,
                )),
            ),
        ))
        .expect("canonical readiness snapshot should apply");

    runtime
        .run_set_ready_for_user_from("", true, true, DirectReadinessSurface::GuiButton)
        .expect("direct readiness should queue immediately after the handshake");

    let outbound = runtime.control().outbound_messages();
    assert_eq!(outbound.len(), 1, "no seed State should be required");
    let ProtocolMessage::Set(set) = &outbound[0] else {
        panic!("V2 intent should use Set");
    };
    let intent = set
        .set
        .readiness_v2()
        .expect("readiness extension should decode")
        .and_then(|extension| extension.intent)
        .expect("queued Set should contain intent");
    assert_eq!(intent.desired, UserReadinessIntent::Ready);
}

#[test]
fn failed_direct_readiness_delivery_remains_semantically_pending() {
    let mut runtime = ClientRuntime::new(
        active_v2_session(),
        RecordingPlayer::default(),
        RejectingReadinessControl,
    );

    runtime
        .run_set_ready_for_user_from("", true, true, DirectReadinessSurface::GuiButton)
        .expect_err("the injected readiness delivery failure should surface");

    let retry = runtime
        .session_mut_for_test()
        .pending_readiness_reconciliation_action()
        .expect("semantic intent should remain eligible for retry");
    let ClientRuntimeAction::SetReadinessIntent { request, .. } = retry else {
        panic!("semantic retry expected");
    };
    assert_eq!(request.desired, UserReadinessIntent::Ready);
}

#[test]
fn failed_indirect_pause_api_delivery_retries_the_same_semantic_operation() {
    for use_toggle in [false, true] {
        let mut session = active_v2_session();
        session.model.playback.local_paused = Some(true);
        let mut runtime = ClientRuntime::new(
            session,
            RecordingPlayer::default(),
            RejectingReadinessControl,
        );

        let error = if use_toggle {
            runtime.run_toggle_pause()
        } else {
            runtime.run_set_paused(false)
        }
        .expect_err("the injected indirect readiness delivery failure should surface");
        assert!(matches!(error, PlayerError::OperationFailed(_)));

        let operation_id = runtime
            .session()
            .pending_readiness_intent()
            .expect("the accepted user intent must remain pending")
            .operation_id()
            .to_owned();
        let retry = runtime
            .session_mut_for_test()
            .pending_readiness_reconciliation_action()
            .expect("the failed indirect delivery should be re-armed");
        let ClientRuntimeAction::SetReadinessIntent { request, .. } = retry else {
            panic!("semantic readiness retry expected");
        };
        assert_eq!(request.operation_id, operation_id);
        assert_eq!(request.desired, UserReadinessIntent::Ready);
    }
}

#[test]
fn canonical_boolean_match_does_not_acknowledge_an_unrelated_operation() {
    let mut session = active_v2_session();
    let actions = session.runtime_actions_for_direct_readiness_intent(
        UserReadinessIntent::Ready,
        DirectReadinessSurface::GuiButton,
        None,
    );
    let [ClientRuntimeAction::SetReadinessIntent { request, .. }] = actions.as_slice() else {
        panic!("semantic action expected");
    };
    let operation_id = request.operation_id.clone();

    session.apply_readiness_v2_extension(ReadinessSetExtension::new().with_snapshot(snapshot(
        2,
        41,
        UserReadinessIntent::Ready,
        Some("different-operation".to_owned()),
    )));
    assert_eq!(
        session
            .pending_readiness_intent()
            .expect("matching boolean must not clear pending operation")
            .operation_id(),
        operation_id
    );

    session.apply_readiness_v2_extension(ReadinessSetExtension::new().with_snapshot(snapshot(
        3,
        41,
        UserReadinessIntent::Ready,
        Some(operation_id),
    )));
    assert!(session.pending_readiness_intent().is_none());
}

#[test]
fn rapid_intents_toggle_against_pending_and_supersede_locally() {
    let mut session = active_v2_session();
    let first = session
        .runtime_actions_for_local_ready_toggle_from(true, DirectReadinessSurface::CliCommand);
    let [ClientRuntimeAction::SetReadinessIntent { request: first, .. }] = first.as_slice() else {
        panic!("first semantic action expected");
    };
    assert_eq!(first.desired, UserReadinessIntent::Ready);

    let second = session
        .runtime_actions_for_local_ready_toggle_from(true, DirectReadinessSurface::CliCommand);
    let [
        ClientRuntimeAction::SetReadinessIntent {
            request: second, ..
        },
    ] = second.as_slice()
    else {
        panic!("second semantic action expected");
    };
    assert_eq!(second.desired, UserReadinessIntent::NotReady);
    assert_ne!(first.operation_id, second.operation_id);
    assert_eq!(
        session
            .pending_readiness_intent()
            .expect("latest intent remains pending")
            .desired(),
        UserReadinessIntent::NotReady
    );
}

#[test]
fn reconnect_preserves_operation_but_reserializes_for_current_membership() {
    let mut session = active_v2_session();
    let first = session.runtime_actions_for_direct_readiness_intent(
        UserReadinessIntent::Ready,
        DirectReadinessSurface::GuiButton,
        None,
    );
    let [ClientRuntimeAction::SetReadinessIntent { request: first, .. }] = first.as_slice() else {
        panic!("first semantic action expected");
    };
    let operation_id = first.operation_id.clone();

    session.reset_sync_state_for_reconnect();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true,"sorotteReadinessV2":true}}}"#,
        )
        .expect("reconnect Hello should apply");
    session.apply_readiness_v2_extension(ReadinessSetExtension::new().with_snapshot(snapshot(
        2,
        41,
        UserReadinessIntent::NotReady,
        None,
    )));
    let resend = session
        .pending_readiness_reconciliation_action()
        .expect("unacknowledged semantic intent should resend");
    let ClientRuntimeAction::SetReadinessIntent {
        request: resend, ..
    } = resend
    else {
        panic!("semantic resend expected");
    };
    assert_eq!(resend.operation_id, operation_id);
    assert_eq!(resend.membership_epoch, 41);
    assert_eq!(resend.expected_user_intent_revision, Some(2));
}

#[test]
fn revision_conflict_retry_preserves_newer_server_scope_until_snapshot_catches_up() {
    let mut session = active_v2_session();
    let first = session.runtime_actions_for_direct_readiness_intent(
        UserReadinessIntent::Ready,
        DirectReadinessSurface::GuiButton,
        None,
    );
    let [ClientRuntimeAction::SetReadinessIntent { request: first, .. }] = first.as_slice() else {
        panic!("first semantic action expected");
    };

    session.apply_readiness_v2_extension(
        ReadinessSetExtension::new().with_request_result(
            ReadinessRequestResultPayload::new(
                first.operation_id.clone(),
                first.request_nonce,
                ReadinessRequestResultStatus::RejectedRevisionConflict,
            )
            .with_room_readiness_revision(2)
            .with_membership_epoch(73)
            .with_user_intent_revision(2),
        ),
    );

    let retry = session
        .pending_readiness_reconciliation_action()
        .expect("revision conflict should schedule an immediate retry");
    let ClientRuntimeAction::SetReadinessIntent {
        request: retry,
        scope,
    } = retry
    else {
        panic!("semantic retry expected");
    };
    assert_eq!(retry.operation_id, first.operation_id);
    assert!(retry.request_nonce > first.request_nonce);
    assert_eq!(retry.expected_user_intent_revision, Some(2));
    assert_eq!(retry.membership_epoch, 73);
    assert_eq!(scope, crate::ReadinessIntentScope::new("room1", 73));
}

#[test]
fn stale_membership_retry_preserves_new_epoch_at_same_revision() {
    let mut session = active_v2_session();
    let first = session.runtime_actions_for_direct_readiness_intent(
        UserReadinessIntent::Ready,
        DirectReadinessSurface::GuiButton,
        None,
    );
    let [ClientRuntimeAction::SetReadinessIntent { request: first, .. }] = first.as_slice() else {
        panic!("first semantic action expected");
    };

    session.apply_readiness_v2_extension(
        ReadinessSetExtension::new().with_request_result(
            ReadinessRequestResultPayload::new(
                first.operation_id.clone(),
                first.request_nonce,
                ReadinessRequestResultStatus::RejectedStaleMembership,
            )
            .with_room_readiness_revision(1)
            .with_membership_epoch(73),
        ),
    );

    let retry = session
        .pending_readiness_reconciliation_action()
        .expect("stale membership should schedule an immediate retry");
    let ClientRuntimeAction::SetReadinessIntent {
        request: retry,
        scope,
    } = retry
    else {
        panic!("semantic retry expected");
    };
    assert_eq!(retry.operation_id, first.operation_id);
    assert!(retry.request_nonce > first.request_nonce);
    assert_eq!(retry.expected_user_intent_revision, Some(1));
    assert_eq!(retry.membership_epoch, 73);
    assert_eq!(scope, crate::ReadinessIntentScope::new("room1", 73));
}

#[test]
fn reconnect_accepts_first_full_snapshot_as_a_new_revision_baseline() {
    let mut session = active_v2_session();
    session.apply_readiness_v2_extension(ReadinessSetExtension::new().with_snapshot(snapshot(
        50,
        41,
        UserReadinessIntent::Ready,
        None,
    )));

    session.reset_sync_state_for_reconnect();
    session
        .apply_hello_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true,"sorotteReadinessV2":true}}}"#,
        )
        .expect("same-room reconnect Hello should apply");
    session.apply_readiness_v2_extension(ReadinessSetExtension::new().with_snapshot(snapshot(
        1,
        73,
        UserReadinessIntent::NotReady,
        None,
    )));

    let accepted = session
        .readiness_snapshot()
        .expect("the first post-reconnect snapshot should establish the baseline");
    assert_eq!(accepted.room_readiness_revision, 1);
    assert_eq!(accepted.participants["alice"].membership_epoch, 73);

    session.apply_readiness_v2_extension(ReadinessSetExtension::new().with_snapshot(snapshot(
        0,
        99,
        UserReadinessIntent::Ready,
        None,
    )));
    let retained = session
        .readiness_snapshot()
        .expect("the reconciled snapshot should remain canonical");
    assert_eq!(retained.room_readiness_revision, 1);
    assert_eq!(retained.participants["alice"].membership_epoch, 73);
}

#[test]
fn lower_revision_full_snapshot_is_stale_without_a_reconnect_boundary() {
    let mut session = active_v2_session();
    session.apply_readiness_v2_extension(ReadinessSetExtension::new().with_snapshot(snapshot(
        50,
        41,
        UserReadinessIntent::Ready,
        None,
    )));
    session.apply_readiness_v2_extension(ReadinessSetExtension::new().with_snapshot(snapshot(
        1,
        73,
        UserReadinessIntent::NotReady,
        None,
    )));

    let retained = session
        .readiness_snapshot()
        .expect("the newer snapshot should remain canonical");
    assert_eq!(retained.room_readiness_revision, 50);
    assert_eq!(retained.participants["alice"].membership_epoch, 41);
}

#[test]
fn explicit_room_switch_invalidates_old_membership_intent() {
    let mut session = active_v2_session();
    let _ = session.runtime_actions_for_direct_readiness_intent(
        UserReadinessIntent::Ready,
        DirectReadinessSurface::GuiButton,
        None,
    );
    assert!(session.pending_readiness_intent().is_some());

    let actions = session.runtime_actions_for_local_room_switch("room2".to_owned());
    assert!(matches!(
        actions.as_slice(),
        [ClientRuntimeAction::SetRoom { room }, ClientRuntimeAction::RequestUserList]
            if room == "room2"
    ));
    assert!(session.pending_readiness_intent().is_none());
    assert!(session.readiness_snapshot().is_none());
}

#[test]
fn local_room_echo_fences_old_snapshot_before_new_membership_snapshot() {
    let mut session = active_v2_session();
    let _ = session.runtime_actions_for_local_room_switch("room2".to_owned());

    // A final old-room fanout can legitimately precede the ordered room echo.
    session.apply_readiness_v2_extension(ReadinessSetExtension::new().with_snapshot(snapshot(
        99,
        41,
        UserReadinessIntent::Ready,
        None,
    )));
    assert_eq!(
        session
            .readiness_snapshot()
            .map(|snapshot| snapshot.room_readiness_revision),
        Some(99)
    );

    session
        .apply_message_json(r#"{"Set":{"user":{"alice":{"room":{"name":"room2"}}}}}"#)
        .expect("local room echo should apply");
    assert!(
        session.readiness_snapshot().is_none(),
        "the room boundary must discard the old membership revision"
    );

    session.apply_readiness_v2_extension(ReadinessSetExtension::new().with_snapshot(snapshot(
        1,
        73,
        UserReadinessIntent::NotReady,
        None,
    )));
    assert_eq!(
        session
            .canonical_participant_readiness("alice")
            .map(|participant| participant.membership_epoch),
        Some(73),
        "a lower revision is valid in the genuinely new room membership"
    );
}

#[test]
fn v2_rooms_never_run_the_legacy_client_autoplay_countdown() {
    let session = active_v2_session();
    assert!(!session.autoplay_conditions_met(true, true, false, true));
}
