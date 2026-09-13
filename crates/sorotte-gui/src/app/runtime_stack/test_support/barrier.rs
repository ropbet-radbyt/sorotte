use crate::app::runtime_stack::{GuiAttachedPlayerRuntimeAction, GuiClientSession};
use sorotte_client_core::{PlaybackBarrierStartConfig, PlaybackBarrierTimeoutAction};
use sorotte_player_api::{
    PlayerMediaGeneration, PlayerObservationTimestamp, PlayerTransportPhase,
    PlayerTransportTelemetryUpdate,
};
use sorotte_protocol::{
    PlaybackBarrierPhase, PlaybackBarrierPolicy, PlaybackBarrierStateExtension,
    PlaybackBarrierStatusPayload, PrepareMediaPayload, ProtocolMessage, decode_message_line_items,
    encode_message_line,
};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

pub(in crate::app) const LOGICAL_MEDIA_ID: &str = "sha256:gui-barrier-integration";
pub(in crate::app) const ROOM_MEDIA_GENERATION: u64 = 41;
pub(in crate::app) const ROOM_STATE_REVISION: u64 = 7;

pub(in crate::app) fn apply_protocol_message(
    adapter: &mut GuiClientSession,
    message: ProtocolMessage,
) {
    let line = encode_message_line(&message).expect("test protocol message should encode");
    adapter
        .apply_message_json(&line)
        .expect("test protocol message should apply through the real GUI adapter");
}

pub(in crate::app) fn barrier_status(
    policy: PlaybackBarrierPolicy,
    phase: PlaybackBarrierPhase,
    state_revision: Option<u64>,
) -> PlaybackBarrierStatusPayload {
    PlaybackBarrierStatusPayload {
        media_generation: ROOM_MEDIA_GENERATION,
        state_revision,
        phase,
        policy,
        quorum: None,
        deadline: 120.0,
        participants: BTreeMap::new(),
        excluded_unsupported_clients: BTreeSet::new(),
    }
}

pub(in crate::app) fn transport(
    observed_at_seconds: f64,
    phase: PlayerTransportPhase,
    position_seconds: f64,
    logical_pause: bool,
    playback_restart_sequence: u64,
) -> PlayerTransportTelemetryUpdate {
    let mut update = PlayerTransportTelemetryUpdate::new(
        PlayerMediaGeneration::new(1),
        PlayerObservationTimestamp::from_adapter_start(Duration::from_secs_f64(
            observed_at_seconds,
        )),
    )
    .with_phase(phase)
    .with_position_seconds(position_seconds)
    .with_logical_pause(logical_pause);
    update.paused_for_cache = Some(phase == PlayerTransportPhase::Rebuffering);
    update.seeking = Some(phase == PlayerTransportPhase::Seeking);
    update.seekable = Some(true);
    update.core_idle = Some(phase == PlayerTransportPhase::ReadyPaused);
    update.playback_restart_sequence = Some(playback_restart_sequence);
    update
}

pub(in crate::app) fn accept_coordinator_commands(
    adapter: &mut GuiClientSession,
    actions: &[GuiAttachedPlayerRuntimeAction],
    now_seconds: f64,
) {
    for action in actions {
        if let GuiAttachedPlayerRuntimeAction::Coordinator { command_id, .. } = action {
            adapter.report_attached_coordinator_command_dispatch(*command_id, true, now_seconds);
        }
    }
}

pub(in crate::app) fn drain_barrier_state_extensions(
    adapter: &mut GuiClientSession,
) -> Vec<PlaybackBarrierStateExtension> {
    adapter
        .deliver_outbound_protocol_lines()
        .expect("GUI adapter outbox should encode")
        .into_iter()
        .flat_map(|line| {
            decode_message_line_items(&line)
                .expect("GUI adapter outbox line should decode")
                .into_iter()
        })
        .filter_map(|item| item.message.ok())
        .filter_map(|message| match message {
            ProtocolMessage::State(state) => state
                .state
                .playback_barrier_v1()
                .expect("GUI barrier State extension should decode"),
            _ => None,
        })
        .collect()
}

pub(in crate::app) fn barrier_request(adapter: &mut GuiClientSession) -> PrepareMediaPayload {
    adapter
        .deliver_outbound_protocol_lines()
        .expect("GUI adapter outbox should encode")
        .into_iter()
        .flat_map(|line| {
            decode_message_line_items(&line)
                .expect("GUI adapter outbox line should decode")
                .into_iter()
        })
        .filter_map(|item| item.message.ok())
        .find_map(|message| match message {
            ProtocolMessage::Set(set) => set
                .set
                .playback_barrier_v1()
                .expect("GUI barrier Set extension should decode")
                .and_then(|extension| extension.prepare),
            _ => None,
        })
        .expect("controller media preparation should emit a PrepareMedia request")
}

pub(in crate::app) fn barrier_aware_controller(policy: PlaybackBarrierPolicy) -> GuiClientSession {
    let mut adapter = GuiClientSession::new("alice", "room1");
    let startup = adapter
        .deliver_outbound_protocol_lines()
        .expect("startup Hello should encode");
    assert_eq!(startup.len(), 1);
    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"sorottePlaybackBarrierV1":true}}}"#,
        )
        .expect("barrier-aware server Hello should apply");
    adapter
        .apply_message_json(
            r#"{"Set":{"user":{"alice":{"room":{"name":"room1"},"controller":true}}}}"#,
        )
        .expect("local controller projection should apply");
    assert_eq!(adapter.runtime.session().local_can_control(), Some(true));
    adapter
        .runtime
        .set_playback_barrier_start_config(PlaybackBarrierStartConfig {
            policy: Some(policy),
            timeout_action: PlaybackBarrierTimeoutAction::Continue,
            ..PlaybackBarrierStartConfig::default()
        });
    adapter
}
