use super::*;

use crate::app::support::system_time_seconds;
use sorotte_client_core::{
    PlaybackBarrierStartConfig, PlaybackBarrierTimeoutAction, RoomPlaystateAuthority,
};
use sorotte_player_api::{
    PlayerMediaGeneration, PlayerObservationTimestamp, PlayerPlayIntent, PlayerTransportPhase,
    PlayerTransportTelemetryUpdate,
};
use sorotte_protocol::{
    CommitStartPayload, PlaybackBarrierPhase, PlaybackBarrierPolicy,
    PlaybackBarrierRequestResultPayload, PlaybackBarrierSetExtension,
    PlaybackBarrierStateExtension, PlaybackBarrierStatusPayload, PlaystatePayload,
    PrepareMediaPayload, ProtocolMessage, RoomBufferingPhase, RoomBufferingPolicy,
    RoomBufferingPolicyPayload, RoomBufferingStatusPayload, SetPayload, StatePayload,
    decode_message_line_items, encode_message_line,
};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

const LOGICAL_MEDIA_ID: &str = "sha256:gui-barrier-integration";
const ROOM_MEDIA_GENERATION: u64 = 41;
const ROOM_STATE_REVISION: u64 = 7;

fn apply_protocol_message(
    adapter: &mut GuiClientCoreChatSessionRuntimeAdapter,
    message: ProtocolMessage,
) {
    let line = encode_message_line(&message).expect("test protocol message should encode");
    adapter
        .apply_message_json(&line)
        .expect("test protocol message should apply through the real GUI adapter");
}

fn barrier_status(
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
        excluded_legacy_clients: BTreeSet::new(),
    }
}

fn transport(
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

fn accept_coordinator_commands(
    adapter: &mut GuiClientCoreChatSessionRuntimeAdapter,
    actions: &[GuiAttachedPlayerRuntimeAction],
    now_seconds: f64,
) {
    for action in actions {
        if let GuiAttachedPlayerRuntimeAction::Coordinator { command_id, .. } = action {
            adapter.report_attached_coordinator_command_dispatch(*command_id, true, now_seconds);
        }
    }
}

fn drain_barrier_state_extensions(
    adapter: &mut GuiClientCoreChatSessionRuntimeAdapter,
) -> Vec<PlaybackBarrierStateExtension> {
    adapter
        .flush_outbound_protocol_lines()
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

fn barrier_request(adapter: &mut GuiClientCoreChatSessionRuntimeAdapter) -> PrepareMediaPayload {
    adapter
        .flush_outbound_protocol_lines()
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

fn barrier_aware_controller(
    policy: PlaybackBarrierPolicy,
) -> GuiClientCoreChatSessionRuntimeAdapter {
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core GUI adapter should bootstrap");
    let startup = adapter
        .flush_outbound_protocol_lines()
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

fn exercise_gui_barrier_lifecycle(policy: PlaybackBarrierPolicy) {
    let mut adapter = barrier_aware_controller(policy);
    let plan = adapter
        .prepare_attached_playback_media(
            LogicalMediaId::new(LOGICAL_MEDIA_ID).expect("logical media ID should be valid"),
            MediaTransportKind::NetworkVod,
            MediaLoadIntent::NewPlayback,
            100.0,
        )
        .expect("GUI media preparation should succeed")
        .expect("real GUI adapter should return a media load plan");
    assert_eq!(plan.media_generation, 1);

    let request = barrier_request(&mut adapter);
    assert_eq!(request.media_generation, 0);
    assert_ne!(request.request_nonce, 0);
    assert_eq!(request.logical_media_id, LOGICAL_MEDIA_ID);
    assert_eq!(request.policy, policy);

    // The server first forces the room pause, then publishes its canonical
    // generation. Reconciliation is deliberately deferred until both frames
    // have crossed the same real adapter used by the GUI runtime owner.
    apply_protocol_message(
        &mut adapter,
        ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(0.0)
                    .with_paused(true)
                    .with_do_seek(true)
                    .with_set_by("alice"),
            ),
        ),
    );
    let prepare = PrepareMediaPayload::new(ROOM_MEDIA_GENERATION, LOGICAL_MEDIA_ID, 0.0, policy)
        .with_request_nonce(request.request_nonce);
    apply_protocol_message(
        &mut adapter,
        ProtocolMessage::set(
            SetPayload::new().with_playback_barrier_v1(
                PlaybackBarrierSetExtension::new()
                    .with_prepare(prepare)
                    .with_status(barrier_status(
                        policy,
                        PlaybackBarrierPhase::Preparing,
                        None,
                    )),
            ),
        ),
    );
    assert!(matches!(
        adapter.runtime.session().current_room_playstate_authority(),
        Some(RoomPlaystateAuthority::ServerBarrier {
            media_generation: ROOM_MEDIA_GENERATION,
            state_revision: None,
        })
    ));

    let initial_actions = adapter
        .attached_player_runtime_actions(100.0)
        .expect("pre-observation reconciliation should succeed");
    accept_coordinator_commands(&mut adapter, &initial_actions, 100.0);

    let ready_actions = adapter
        .sync_attached_player_transport_telemetry(
            transport(1.0, PlayerTransportPhase::ReadyPaused, 0.0, true, 0),
            101.0,
        )
        .expect("ReadyPaused transport should reach the real GUI adapter");
    accept_coordinator_commands(&mut adapter, &ready_actions, 101.0);
    let ready_extensions = drain_barrier_state_extensions(&mut adapter);
    let ready = ready_extensions
        .iter()
        .find_map(|extension| extension.ready.as_ref())
        .expect("initiating GUI participant should emit MediaReady");
    assert_eq!(ready.media_generation, ROOM_MEDIA_GENERATION);
    assert!(ready.loaded);
    assert_eq!(ready.seekable, Some(true));
    assert!(ready.buffer_ready, "core-idle ReadyPaused is barrier-ready");

    let commit = CommitStartPayload::new(
        ROOM_MEDIA_GENERATION,
        ROOM_STATE_REVISION,
        0.0,
        102.0,
        112.0,
    );
    apply_protocol_message(
        &mut adapter,
        ProtocolMessage::set(
            SetPayload::new().with_playback_barrier_v1(
                PlaybackBarrierSetExtension::new()
                    .with_commit(commit)
                    .with_status(barrier_status(
                        policy,
                        PlaybackBarrierPhase::Committed,
                        Some(ROOM_STATE_REVISION),
                    )),
            ),
        ),
    );
    apply_protocol_message(
        &mut adapter,
        ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(0.0)
                    .with_paused(false)
                    .with_do_seek(true)
                    .with_set_by("alice"),
            ),
        ),
    );

    let commit_actions = adapter
        .attached_player_runtime_actions(102.0)
        .expect("server commit should reconcile through the GUI adapter");
    assert!(commit_actions.iter().any(|action| matches!(
        action,
        GuiAttachedPlayerRuntimeAction::Coordinator {
            command: CoordinatorPlayerCommand::Play(PlayerPlayIntent::StartAfterLoad { .. }),
            ..
        }
    )));
    accept_coordinator_commands(&mut adapter, &commit_actions, 102.0);

    let first_playing = adapter
        .sync_attached_player_transport_telemetry(
            transport(2.0, PlayerTransportPhase::Playing, 0.0, false, 1),
            102.5,
        )
        .expect("first Playing observation should apply");
    accept_coordinator_commands(&mut adapter, &first_playing, 102.5);
    assert!(
        drain_barrier_state_extensions(&mut adapter)
            .iter()
            .all(|extension| extension.started.is_none()),
        "command acceptance and restart without advancement must not emit StartedAck"
    );

    let advancing = adapter
        .sync_attached_player_transport_telemetry(
            transport(3.0, PlayerTransportPhase::Playing, 0.25, false, 1),
            103.0,
        )
        .expect("advancing Playing observation should apply");
    accept_coordinator_commands(&mut adapter, &advancing, 103.0);
    let started_extensions = drain_barrier_state_extensions(&mut adapter);
    let started = started_extensions
        .iter()
        .find_map(|extension| extension.started.as_ref())
        .expect("observed advancement should emit StartedAck");
    assert_eq!(started.media_generation, ROOM_MEDIA_GENERATION);
    assert_eq!(started.state_revision, ROOM_STATE_REVISION);
    assert!((started.observed_position - 0.25).abs() < f64::EPSILON);
}

#[test]
fn real_gui_controller_initiator_completes_controller_barrier_lifecycle() {
    exercise_gui_barrier_lifecycle(PlaybackBarrierPolicy::Controller);
}

#[test]
fn real_gui_controller_participates_in_its_all_eligible_barrier() {
    exercise_gui_barrier_lifecycle(PlaybackBarrierPolicy::AllEligible);
}

#[test]
fn real_gui_adapter_keeps_retry_later_nonfatal_and_retries_the_same_attempt_once() {
    let mut adapter = barrier_aware_controller(PlaybackBarrierPolicy::Controller);
    adapter
        .prepare_attached_playback_media(
            LogicalMediaId::new(LOGICAL_MEDIA_ID).expect("logical media ID should be valid"),
            MediaTransportKind::NetworkVod,
            MediaLoadIntent::NewPlayback,
            10.0,
        )
        .expect("GUI media preparation should succeed");
    let original = barrier_request(&mut adapter);
    let received_before = system_time_seconds();

    apply_protocol_message(
        &mut adapter,
        ProtocolMessage::set(
            SetPayload::new().with_playback_barrier_v1(
                PlaybackBarrierSetExtension::new().with_request_result(
                    PlaybackBarrierRequestResultPayload::retry_later(
                        original
                            .request_id
                            .clone()
                            .expect("GUI barrier request should have an operation ID"),
                        original.request_nonce,
                        1_000,
                    ),
                ),
            ),
        ),
    );

    assert!(
        matches!(
            adapter.runtime.connection_phase(),
            ConnectionPhase::Active(_)
        ),
        "retryLater must leave the GUI session active"
    );
    assert!(
        !adapter.runtime.take_stop_reconnect_requested(),
        "retryLater must not terminate GUI reconnect ownership"
    );
    assert!(
        adapter
            .runtime
            .pending_playback_barrier_retry_delay_at(system_time_seconds())
            .is_some(),
        "the semantic media intent should remain pending behind a retry delay"
    );

    adapter
        .runtime
        .run_pending_playback_barrier_retry_at(received_before)
        .expect("an early GUI retry pump should be harmless");
    assert!(
        adapter
            .flush_outbound_protocol_lines()
            .expect("early GUI outbox should encode")
            .is_empty(),
        "retryLater must not be retried before its delay"
    );

    adapter
        .runtime
        .run_pending_playback_barrier_retry_at(received_before + 2.0)
        .expect("the due GUI retry should dispatch");
    let retried = barrier_request(&mut adapter);
    assert_eq!(retried.request_id, original.request_id);
    assert_eq!(retried.request_nonce, original.request_nonce);
    assert_eq!(retried.logical_media_id, original.logical_media_id);

    adapter
        .runtime
        .run_pending_playback_barrier_retry_at(received_before + 3.0)
        .expect("a repeated GUI retry pump should be harmless");
    assert!(
        adapter
            .flush_outbound_protocol_lines()
            .expect("repeated GUI retry outbox should encode")
            .is_empty(),
        "the GUI retry pump must emit exactly one attempt"
    );
}

#[test]
fn real_gui_adapter_obeys_self_attributed_server_buffering_and_adopts_local_echoes() {
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core GUI adapter should bootstrap");
    adapter
        .flush_outbound_protocol_lines()
        .expect("startup Hello should encode");
    adapter
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"chat":true,"sorottePlaybackBarrierV1":true}}}"#,
        )
        .expect("barrier-aware server Hello should apply");
    adapter
        .prepare_attached_playback_media(
            LogicalMediaId::new(LOGICAL_MEDIA_ID).expect("logical media ID should be valid"),
            MediaTransportKind::NetworkVod,
            MediaLoadIntent::NewPlayback,
            10.0,
        )
        .expect("GUI media preparation should succeed");

    apply_protocol_message(
        &mut adapter,
        ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(10.0)
                    .with_paused(false)
                    .with_do_seek(false)
                    .with_set_by("bob"),
            ),
        ),
    );
    let baseline = adapter
        .sync_attached_player_transport_telemetry(
            transport(1.0, PlayerTransportPhase::Playing, 10.0, false, 1),
            10.0,
        )
        .expect("baseline transport should apply");
    accept_coordinator_commands(&mut adapter, &baseline, 10.0);
    let reconcile = adapter
        .attached_player_runtime_actions(10.0)
        .expect("baseline desired state should reconcile");
    accept_coordinator_commands(&mut adapter, &reconcile, 10.0);
    let baseline_advancement = adapter
        .sync_attached_player_transport_telemetry(
            transport(1.5, PlayerTransportPhase::Playing, 10.25, false, 1),
            10.5,
        )
        .expect("baseline advancement should apply");
    accept_coordinator_commands(&mut adapter, &baseline_advancement, 10.5);

    let policy = RoomBufferingPolicyPayload::new(
        ROOM_MEDIA_GENERATION,
        RoomBufferingPolicy::PauseAnyEligible,
    )
    .with_debounce_ms(1)
    .with_resume_hysteresis_ms(1)
    .with_max_pause_ms(30_000);
    let mut buffering_clients = BTreeSet::new();
    buffering_clients.insert("alice".to_owned());
    let paused_status = RoomBufferingStatusPayload {
        config: policy.clone(),
        phase: RoomBufferingPhase::Paused,
        eligible_clients: 1,
        required_buffering_clients: 1,
        buffering_clients,
        pause_deadline: Some(40.0),
    };
    apply_protocol_message(
        &mut adapter,
        ProtocolMessage::set(
            SetPayload::new().with_playback_barrier_v1(
                PlaybackBarrierSetExtension::new()
                    .with_buffering_policy(policy.clone())
                    .with_buffering_status(paused_status),
            ),
        ),
    );
    apply_protocol_message(
        &mut adapter,
        ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(10.25)
                    .with_paused(true)
                    .with_do_seek(false)
                    .with_set_by("alice"),
            ),
        ),
    );
    assert!(matches!(
        adapter.runtime.session().current_room_playstate_authority(),
        Some(RoomPlaystateAuthority::ServerBufferingPolicy {
            media_generation: ROOM_MEDIA_GENERATION,
        })
    ));
    let pause_actions = adapter
        .attached_player_runtime_actions(11.0)
        .expect("server-owned buffering pause should reconcile");
    assert!(pause_actions.iter().any(|action| matches!(
        action,
        GuiAttachedPlayerRuntimeAction::Coordinator {
            command: CoordinatorPlayerCommand::SetPaused(true),
            ..
        }
    )));
    accept_coordinator_commands(&mut adapter, &pause_actions, 11.0);
    let paused_observation = adapter
        .sync_attached_player_transport_telemetry(
            transport(2.0, PlayerTransportPhase::ReadyPaused, 10.0, true, 1),
            11.5,
        )
        .expect("policy pause observation should apply");
    accept_coordinator_commands(&mut adapter, &paused_observation, 11.5);

    // The server publishes the forced resume before its trailing Monitoring
    // status. The still-active Paused status gives this self-attributed state
    // semantic server authority during reconciliation.
    apply_protocol_message(
        &mut adapter,
        ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(10.0)
                    .with_paused(false)
                    .with_do_seek(false)
                    .with_set_by("alice"),
            ),
        ),
    );
    let resume_actions = adapter
        .attached_player_runtime_actions(12.0)
        .expect("server-owned buffering resume should reconcile");
    assert!(
        resume_actions.iter().any(|action| matches!(
            action,
            GuiAttachedPlayerRuntimeAction::Coordinator {
                command: CoordinatorPlayerCommand::Play(PlayerPlayIntent::Resume),
                ..
            }
        )),
        "server buffering resume actions were {resume_actions:?}"
    );
    accept_coordinator_commands(&mut adapter, &resume_actions, 12.0);
    apply_protocol_message(
        &mut adapter,
        ProtocolMessage::set(SetPayload::new().with_playback_barrier_v1(
            PlaybackBarrierSetExtension::new().with_buffering_status(RoomBufferingStatusPayload {
                config: policy,
                phase: RoomBufferingPhase::Monitoring,
                eligible_clients: 1,
                required_buffering_clients: 1,
                buffering_clients: BTreeSet::new(),
                pause_deadline: None,
            }),
        )),
    );
    let resumed_observation = adapter
        .sync_attached_player_transport_telemetry(
            transport(3.0, PlayerTransportPhase::Playing, 10.25, false, 1),
            12.5,
        )
        .expect("policy resume observation should apply");
    accept_coordinator_commands(&mut adapter, &resumed_observation, 12.5);

    // Once the policy is merely monitoring, a self-attributed ordinary state
    // is a local echo. It is adopted as the next desired fingerprint without
    // replaying a stale server desired state back into the player.
    apply_protocol_message(
        &mut adapter,
        ProtocolMessage::state(
            StatePayload::new().with_playstate(
                PlaystatePayload::new()
                    .with_position(25.0)
                    .with_paused(true)
                    .with_do_seek(false)
                    .with_set_by("alice"),
            ),
        ),
    );
    assert_eq!(
        adapter.runtime.session().current_room_playstate_authority(),
        Some(RoomPlaystateAuthority::LegacyLocalEcho)
    );
    assert!(
        adapter
            .attached_player_runtime_actions(13.0)
            .expect("ordinary local echo should be adopted")
            .is_empty(),
        "ordinary local echo must not replay a coordinator command"
    );
    let local_pause_observation = adapter
        .sync_attached_player_transport_telemetry(
            transport(4.0, PlayerTransportPhase::ReadyPaused, 25.0, true, 1),
            13.5,
        )
        .expect("local pause observation should apply");
    assert!(
        local_pause_observation.is_empty(),
        "a stale pre-echo desired state must not undo the observed local pause"
    );
}
