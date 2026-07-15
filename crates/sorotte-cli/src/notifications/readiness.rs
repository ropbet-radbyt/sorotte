use std::collections::BTreeMap;

use sorotte_client_app::app_boundary::readiness::{
    ParticipantReadinessPresentation, PendingReadinessIntentPresentation,
};
use sorotte_client_core::ClientSession;
use sorotte_protocol::StartParticipationRole;

#[derive(Debug, Default)]
pub(crate) struct ReadinessNotificationState {
    visible_lines: BTreeMap<String, String>,
}

fn participation_role_label(role: Option<StartParticipationRole>) -> &'static str {
    match role {
        Some(StartParticipationRole::Required) => "required",
        Some(StartParticipationRole::Spectator) => "spectator",
        Some(StartParticipationRole::ExcludedLegacy) => "excluded-legacy",
        None => "unknown",
    }
}

fn readiness_status_line(presentation: &ParticipantReadinessPresentation) -> String {
    format!(
        "readiness {}: {}; intent: {}; technical: {}; eligibility: {}; role={}",
        presentation.username,
        presentation.status_label(),
        presentation.intent_detail_label(),
        presentation.technical_detail_label(),
        presentation.eligibility_detail_label(),
        participation_role_label(presentation.participation_role),
    )
}

pub(crate) fn next_readiness_status_lines(
    state: &mut ReadinessNotificationState,
    session: &ClientSession,
) -> Vec<String> {
    if !session.server_readiness_v2_supported() {
        state.visible_lines.clear();
        return Vec::new();
    }
    let Some(snapshot) = session.readiness_snapshot() else {
        state.visible_lines.clear();
        return Vec::new();
    };

    let local_username = session.username();
    let current_lines = snapshot
        .participants
        .iter()
        .map(|(username, canonical)| {
            let pending = session
                .pending_readiness_intent()
                .filter(|pending| {
                    pending
                        .target_username()
                        .or(local_username)
                        .is_some_and(|target| target == username)
                })
                .map(PendingReadinessIntentPresentation::from);
            let presentation = ParticipantReadinessPresentation::from_v2(canonical, pending);
            (username.clone(), readiness_status_line(&presentation))
        })
        .collect::<BTreeMap<_, _>>();

    let changed = current_lines
        .iter()
        .filter(|(username, line)| state.visible_lines.get(*username) != Some(*line))
        .map(|(_, line)| line.clone())
        .collect();
    state.visible_lines = current_lines;
    changed
}

pub(crate) fn flush_readiness_status_notifications(
    runtime: &sorotte_client_app::app_boundary::application::ClientApplication<
        sorotte_player_mpv::MpvAdapter,
    >,
    state: &mut ReadinessNotificationState,
) {
    for line in next_readiness_status_lines(state, runtime.session()) {
        println!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sorotte_protocol::{DirectReadinessSurface, UserReadinessIntent};

    fn v2_session() -> ClientSession {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true,"sorotteReadinessV2":true,"sorottePlaybackBarrierV1":true}}}"#,
            )
            .expect("V2 Hello should apply");
        session
            .apply_message_json(
                r#"{"Set":{"sorotteReadinessV2":{"snapshot":{"roomReadinessRevision":1,"mediaGeneration":7,"startGatePhase":{"phase":"waitingForTechnicalReadiness","mediaGeneration":7},"pauseOwner":{"owner":"readinessStartGate","mediaGeneration":7},"participants":{"alice":{"roomReadinessRevision":1,"membershipEpoch":41,"username":"alice","userIntent":"ready","userIntentRevision":1,"userIntentSource":{"type":"initialization"},"technicalState":{"phase":"temporarilyBlocked","mediaGeneration":7,"reason":"rebuffering","recovery":"retrying"},"participationRole":"required","roomReady":true,"startEligible":false}}}}}}"#,
            )
            .expect("V2 readiness snapshot should apply");
        session
    }

    #[test]
    fn v2_status_separates_intent_technical_state_and_eligibility_and_deduplicates() {
        let session = v2_session();
        let mut state = ReadinessNotificationState::default();

        assert_eq!(
            next_readiness_status_lines(&mut state, &session),
            vec![
                "readiness alice: Ready — buffering; intent: canonical=Ready; technical: phase=temporarily-blocked, reason=rebuffering, recovery=retrying; eligibility: room_ready=yes, start_eligible=no; role=required"
                    .to_owned(),
            ]
        );
        assert!(next_readiness_status_lines(&mut state, &session).is_empty());
    }

    #[test]
    fn pending_cli_intent_remains_distinct_from_canonical_status() {
        let mut session = v2_session();
        let mut state = ReadinessNotificationState::default();
        let _ = next_readiness_status_lines(&mut state, &session);

        let actions = session.runtime_actions_for_direct_readiness_intent(
            UserReadinessIntent::NotReady,
            DirectReadinessSurface::CliCommand,
            None,
        );
        assert_eq!(actions.len(), 1);

        assert_eq!(
            next_readiness_status_lines(&mut state, &session),
            vec![
                "readiness alice: Not Ready — buffering; intent: pending=Not Ready, canonical=Ready; technical: phase=temporarily-blocked, reason=rebuffering, recovery=retrying; eligibility: room_ready=yes, start_eligible=no; role=required"
                    .to_owned(),
            ]
        );
    }

    #[test]
    fn legacy_readiness_produces_no_new_status_output() {
        let mut session = ClientSession::default();
        session
            .apply_hello_json(
                r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"readiness":true}}}"#,
            )
            .expect("legacy Hello should apply");
        let mut state = ReadinessNotificationState {
            visible_lines: BTreeMap::from([("stale".to_owned(), "stale".to_owned())]),
        };

        assert!(next_readiness_status_lines(&mut state, &session).is_empty());
        assert!(state.visible_lines.is_empty());
    }
}
