use super::*;
use crate::normalize_client_state_payload;

const SCHEDULE_EPSILON: f64 = 1e-8;

#[derive(Clone, Copy, Debug, Default)]
struct ReferencePingOracle {
    client_rtt_seconds: f64,
    average_rtt_seconds: f64,
    server_rtt_seconds: f64,
    forward_delay_seconds: f64,
}

impl ReferencePingOracle {
    fn observe(
        &mut self,
        client_send_seconds: f64,
        server_rtt_seconds: f64,
        client_receive_seconds: f64,
    ) -> bool {
        if !client_send_seconds.is_finite()
            || !server_rtt_seconds.is_finite()
            || server_rtt_seconds < 0.0
        {
            return false;
        }

        let client_rtt_seconds = client_receive_seconds - client_send_seconds;
        if !client_rtt_seconds.is_finite() || client_rtt_seconds < 0.0 {
            return false;
        }

        self.client_rtt_seconds = client_rtt_seconds;
        self.server_rtt_seconds = server_rtt_seconds;
        if self.average_rtt_seconds == 0.0 {
            self.average_rtt_seconds = client_rtt_seconds;
        }
        self.average_rtt_seconds = self.average_rtt_seconds * 0.85 + client_rtt_seconds * 0.15;
        self.forward_delay_seconds =
            self.average_rtt_seconds / 2.0 + (client_rtt_seconds - server_rtt_seconds).max(0.0);
        true
    }

    fn snapshot(self) -> (f64, f64, f64) {
        (
            self.client_rtt_seconds,
            self.server_rtt_seconds,
            self.forward_delay_seconds,
        )
    }
}

fn production_ping_snapshot(metrics: ClientPingMetricsLegacyCompatible) -> (f64, f64, f64) {
    (
        metrics.client_rtt_seconds(),
        metrics.server_rtt_seconds(),
        metrics.forward_delay_seconds(),
    )
}

fn assert_close(actual: f64, expected: f64, context: &str) {
    assert!(
        (actual - expected).abs() <= SCHEDULE_EPSILON,
        "{context}: expected {expected:.12}, got {actual:.12}"
    );
}

fn assert_ping_snapshots_match(actual: (f64, f64, f64), expected: (f64, f64, f64), context: &str) {
    assert_close(actual.0, expected.0, &format!("{context} client RTT"));
    assert_close(actual.1, expected.1, &format!("{context} server RTT"));
    assert_close(actual.2, expected.2, &format!("{context} forward delay"));
}

fn observe_production_ping(
    metrics: &mut ClientPingMetricsLegacyCompatible,
    client_send_seconds: f64,
    server_rtt_seconds: f64,
    client_receive_seconds: f64,
) {
    metrics.observe_inbound_state_at(
        &StatePayload::new().with_ping(
            PingPayload::new()
                .with_client_latency_calculation(client_send_seconds)
                .with_server_rtt(server_rtt_seconds),
        ),
        client_receive_seconds,
    );
}

#[test]
fn ping_jitter_outlier_and_nonmonotonic_observation_schedule_matches_reference_oracle() {
    let schedule = [
        ("baseline", 1_000.0, 0.08, 1_000.10, true),
        ("moderate jitter", 1_001.0, 0.09, 1_001.45, true),
        ("large finite outlier", 1_002.0, 0.15, 1_004.75, true),
        ("post-outlier recovery", 1_005.0, 0.08, 1_005.12, true),
        (
            "backward wall-clock step with valid same-sample RTT",
            900.0,
            0.10,
            900.20,
            true,
        ),
        (
            "echo from the future after a backward wall-clock step",
            901.0,
            0.10,
            900.90,
            false,
        ),
        ("non-finite receive clock", 902.0, 0.10, f64::NAN, false),
        (
            "negative cross-host RTT duration",
            903.0,
            -0.10,
            903.20,
            false,
        ),
    ];

    let mut production = ClientPingMetricsLegacyCompatible::default();
    let mut reference = ReferencePingOracle::default();

    for (name, client_send, server_rtt, client_receive, expected_to_apply) in schedule {
        let before = production_ping_snapshot(production);
        let reference_applied = reference.observe(client_send, server_rtt, client_receive);
        assert_eq!(
            reference_applied, expected_to_apply,
            "{name}: schedule acceptance precondition drifted"
        );

        observe_production_ping(&mut production, client_send, server_rtt, client_receive);
        let after = production_ping_snapshot(production);
        if expected_to_apply {
            assert_ping_snapshots_match(after, reference.snapshot(), name);
        } else {
            assert_eq!(
                after, before,
                "{name}: a rejected sample must preserve every metric atomically"
            );
            assert_ping_snapshots_match(after, reference.snapshot(), name);
        }
    }

    assert!(
        production.forward_delay_seconds() > 0.1,
        "the finite outlier must remain represented in the smoothed estimate"
    );
    assert!(
        production.forward_delay_seconds() < 1.0,
        "later valid samples must damp rather than retain the full outlier"
    );
}

#[test]
fn affine_clock_offset_is_invariant_while_rate_drift_remains_measurement_bias() {
    let relative_samples = [(10.0, 0.20, 0.10), (20.0, 0.45, 0.12), (30.0, 0.08, 0.07)];
    let offsets = [-1_000_000.0, 0.0, 1_000_000.0];
    let clock_rates = [0.9995, 1.0, 1.0005];

    for clock_rate in clock_rates {
        let mut snapshots = Vec::new();
        for offset in offsets {
            let mut production = ClientPingMetricsLegacyCompatible::default();
            let mut reference = ReferencePingOracle::default();
            for (relative_send, relative_rtt, server_rtt) in relative_samples {
                let client_send = offset + relative_send * clock_rate;
                let client_receive = offset + (relative_send + relative_rtt) * clock_rate;
                assert!(reference.observe(client_send, server_rtt, client_receive));
                observe_production_ping(&mut production, client_send, server_rtt, client_receive);
                assert_ping_snapshots_match(
                    production_ping_snapshot(production),
                    reference.snapshot(),
                    &format!("offset {offset}, rate {clock_rate}"),
                );
            }
            snapshots.push(production_ping_snapshot(production));
        }

        for shifted in &snapshots[1..] {
            assert_ping_snapshots_match(
                *shifted,
                snapshots[0],
                &format!("common clock offset at rate {clock_rate}"),
            );
        }
        assert_close(
            snapshots[0].0,
            0.08 * clock_rate,
            "the latest client RTT should carry local clock-rate drift",
        );
        assert_close(
            snapshots[0].1,
            0.07,
            "server RTT is a received duration, not a cross-host timestamp",
        );
    }

    let baseline_rtt: f64 = 0.08;
    let fast_clock_rtt = baseline_rtt * 1.0005;
    assert!(
        (fast_clock_rtt - baseline_rtt).abs() > SCHEDULE_EPSILON,
        "a rate-skewed local clock must remain distinguishable from an offset-only transform"
    );
}

fn runtime_fixture() -> ClientRuntime<RecordingPlayer, QueuedRuntimeControl> {
    let mut session = ClientSession::default();
    session
        .apply_message_json(
            r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5"}}"#,
        )
        .expect("hello should apply");
    ClientRuntime::new(
        session,
        RecordingPlayer::default(),
        QueuedRuntimeControl::default(),
    )
}

fn inbound_playstate(
    position: f64,
    paused: bool,
    do_seek: bool,
    client_send_seconds: f64,
    server_rtt_seconds: f64,
) -> StatePayload {
    StatePayload::new()
        .with_playstate(
            PlaystatePayload::new()
                .with_position(position)
                .with_paused(paused)
                .with_do_seek(do_seek)
                .with_set_by("bob"),
        )
        .with_ping(
            PingPayload::new()
                .with_client_latency_calculation(client_send_seconds)
                .with_server_rtt(server_rtt_seconds),
        )
}

fn reference_adjusted_position(
    raw_position: f64,
    paused: bool,
    forward_delay_seconds: f64,
    received_at_seconds: f64,
    response_at_seconds: f64,
) -> f64 {
    if paused {
        return raw_position;
    }
    let scheduler_delay_seconds = response_at_seconds - received_at_seconds;
    let scheduler_delay_seconds =
        if scheduler_delay_seconds.is_finite() && scheduler_delay_seconds > 0.0 {
            scheduler_delay_seconds
        } else {
            0.0
        };
    raw_position + forward_delay_seconds.max(0.0) + scheduler_delay_seconds
}

fn adjusted_room_playstate(
    runtime: &ClientRuntime<RecordingPlayer, QueuedRuntimeControl>,
    state: StatePayload,
    received_at_seconds: f64,
    response_at_seconds: f64,
) -> RoomPlaystateView {
    runtime
        .adjusted_inbound_playstate_for_local_state_change_legacy_ping_compatible(
            &normalize_client_state_payload(state),
            received_at_seconds,
            response_at_seconds,
        )
        .expect("schedule should contain playstate")
}

#[test]
fn scheduler_latency_and_nonmonotonic_reply_clocks_have_bounded_projection() {
    let mut runtime = runtime_fixture();
    observe_production_ping(
        &mut runtime.ping_metrics_legacy_compatible,
        100.0,
        0.10,
        100.20,
    );
    let forward_delay = runtime
        .ping_metrics_legacy_compatible
        .forward_delay_seconds();
    assert_close(forward_delay, 0.20, "schedule forward delay precondition");

    let schedules = [
        ("same instant", 100.0, false),
        ("twenty millisecond scheduler delay", 100.02, false),
        (
            "bounded seven hundred fifty millisecond delay",
            100.75,
            false,
        ),
        ("reply clock moved backward", 99.75, false),
        ("non-finite reply clock", f64::INFINITY, false),
        ("paused room ignores every projection", 100.75, true),
    ];
    for (name, response_at_seconds, paused) in schedules {
        let room = adjusted_room_playstate(
            &runtime,
            inbound_playstate(10.0, paused, false, 100.0, 0.10),
            100.0,
            response_at_seconds,
        );
        assert_close(
            room.position.expect("adjusted room position"),
            reference_adjusted_position(10.0, paused, forward_delay, 100.0, response_at_seconds),
            name,
        );
    }

    let mut reconciled = runtime_fixture();
    assert!(
        reconciled.run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible_at_clocks(
            inbound_playstate(10.0, false, false, 100.0, 0.10),
            false,
            100.0,
            100.75,
            100.20,
        ),
        "ping-only reconcile should emit a response"
    );
    assert_close(
        reconciled
            .current_room_playstate_legacy_ping_compatible_at(100.50)
            .and_then(|playstate| playstate.position)
            .expect("monotonic room projection"),
        10.70,
        "room state should age from receipt and add forward delay exactly once",
    );
    assert_close(
        reconciled
            .current_room_playstate_legacy_ping_compatible_at(99.50)
            .and_then(|playstate| playstate.position)
            .expect("backward-clock room projection"),
        10.20,
        "a backward observation must not negatively age the room snapshot",
    );
}

#[derive(Debug, Default)]
struct ReferencePlaybackOracle {
    behind_first_detected_at_seconds: Option<f64>,
}

impl ReferencePlaybackOracle {
    fn actions(
        &mut self,
        room: &RoomPlaystateView,
        now_seconds: f64,
        local_position: f64,
        local_can_control: bool,
        dont_slow_down_with_me: bool,
        speed_supported: bool,
    ) -> Vec<ClientRuntimeAction> {
        let (Some(global_position), Some(global_paused)) = (room.position, room.paused) else {
            self.behind_first_detected_at_seconds = None;
            return Vec::new();
        };
        if room.do_seek == Some(true) {
            self.behind_first_detected_at_seconds = None;
            return Vec::new();
        }

        let difference = local_position - global_position;
        if difference > 4.0 {
            self.behind_first_detected_at_seconds = None;
            return vec![ClientRuntimeAction::SetPosition(global_position)];
        }

        if !local_can_control || dont_slow_down_with_me {
            if difference < -1.75 {
                if let Some(first_detected_at) = self.behind_first_detected_at_seconds {
                    if now_seconds - first_detected_at > 3.25 && difference < -5.0 {
                        self.behind_first_detected_at_seconds = Some(now_seconds + 3.0);
                        return vec![ClientRuntimeAction::SetPosition(global_position + 0.25)];
                    }
                } else {
                    self.behind_first_detected_at_seconds = Some(now_seconds);
                }
            } else {
                self.behind_first_detected_at_seconds = None;
            }
        } else {
            self.behind_first_detected_at_seconds = None;
        }

        if speed_supported && !global_paused && difference > 0.75 {
            return vec![ClientRuntimeAction::SetPlaybackRate(0.95)];
        }
        Vec::new()
    }
}

fn runtime_with_fixed_ping() -> ClientRuntime<RecordingPlayer, QueuedRuntimeControl> {
    let mut runtime = runtime_fixture();
    observe_production_ping(
        &mut runtime.ping_metrics_legacy_compatible,
        100.0,
        0.10,
        100.20,
    );
    runtime
}

struct PlaybackScheduleStep {
    now_seconds: f64,
    local_position: f64,
    local_can_control: bool,
    speed_supported: bool,
    context: &'static str,
}

fn assert_playback_step(
    runtime: &mut ClientRuntime<RecordingPlayer, QueuedRuntimeControl>,
    reference: &mut ReferencePlaybackOracle,
    room: RoomPlaystateView,
    step: PlaybackScheduleStep,
) {
    let expected = reference.actions(
        &room,
        step.now_seconds,
        step.local_position,
        step.local_can_control,
        false,
        step.speed_supported,
    );
    let actual = runtime
        .session_mut()
        .runtime_actions_for_desync_correction_against_room_playstate(
            room,
            step.now_seconds,
            step.local_position,
            step.local_can_control,
            false,
            step.speed_supported,
        );
    assert_eq!(actual, expected, "{}", step.context);
}

#[test]
fn projected_jitter_and_scheduler_latency_drive_reference_playback_outcomes() {
    let mut rewind_runtime = runtime_with_fixed_ping();
    let rewind_room = adjusted_room_playstate(
        &rewind_runtime,
        inbound_playstate(10.0, false, false, 100.0, 0.10),
        100.0,
        100.0,
    );
    let mut rewind_reference = ReferencePlaybackOracle::default();
    assert_playback_step(
        &mut rewind_runtime,
        &mut rewind_reference,
        rewind_room,
        PlaybackScheduleStep {
            now_seconds: 0.0,
            local_position: 14.5,
            local_can_control: true,
            speed_supported: false,
            context: "forward delay still leaves an immediate rewind above four seconds",
        },
    );

    let mut delayed_runtime = runtime_with_fixed_ping();
    let delayed_room = adjusted_room_playstate(
        &delayed_runtime,
        inbound_playstate(10.0, false, false, 100.0, 0.10),
        100.0,
        100.5,
    );
    let mut delayed_reference = ReferencePlaybackOracle::default();
    assert_playback_step(
        &mut delayed_runtime,
        &mut delayed_reference,
        delayed_room,
        PlaybackScheduleStep {
            now_seconds: 0.0,
            local_position: 14.5,
            local_can_control: true,
            speed_supported: false,
            context: "bounded scheduler projection should move the sample below rewind threshold",
        },
    );

    let mut slowdown_runtime = runtime_with_fixed_ping();
    let slowdown_room = adjusted_room_playstate(
        &slowdown_runtime,
        inbound_playstate(10.0, false, false, 100.0, 0.10),
        100.0,
        100.0,
    );
    let mut slowdown_reference = ReferencePlaybackOracle::default();
    assert_playback_step(
        &mut slowdown_runtime,
        &mut slowdown_reference,
        slowdown_room,
        PlaybackScheduleStep {
            now_seconds: 0.0,
            local_position: 11.1,
            local_can_control: true,
            speed_supported: true,
            context: "ahead jitter should select legacy slowdown",
        },
    );

    let mut caught_up_runtime = runtime_with_fixed_ping();
    let caught_up_room = adjusted_room_playstate(
        &caught_up_runtime,
        inbound_playstate(10.0, false, false, 100.0, 0.10),
        100.0,
        100.2,
    );
    let mut caught_up_reference = ReferencePlaybackOracle::default();
    assert_playback_step(
        &mut caught_up_runtime,
        &mut caught_up_reference,
        caught_up_room,
        PlaybackScheduleStep {
            now_seconds: 0.0,
            local_position: 11.1,
            local_can_control: true,
            speed_supported: true,
            context: "reply latency should move the same sample below slowdown threshold",
        },
    );

    let mut fastforward_runtime = runtime_with_fixed_ping();
    let fastforward_room = adjusted_room_playstate(
        &fastforward_runtime,
        inbound_playstate(10.0, false, false, 100.0, 0.10),
        100.0,
        100.1,
    );
    let mut fastforward_reference = ReferencePlaybackOracle::default();
    for (now_seconds, context) in [
        (0.0, "first behind observation starts the sustain window"),
        (
            3.30,
            "sustained behind observation seeks to the projected room target",
        ),
    ] {
        assert_playback_step(
            &mut fastforward_runtime,
            &mut fastforward_reference,
            fastforward_room.clone(),
            PlaybackScheduleStep {
                now_seconds,
                local_position: 5.0,
                local_can_control: false,
                speed_supported: false,
                context,
            },
        );
    }

    let mut seek_runtime = runtime_with_fixed_ping();
    let seek_room = adjusted_room_playstate(
        &seek_runtime,
        inbound_playstate(10.0, false, true, 100.0, 0.10),
        100.0,
        100.5,
    );
    let mut seek_reference = ReferencePlaybackOracle::default();
    assert_playback_step(
        &mut seek_runtime,
        &mut seek_reference,
        seek_room,
        PlaybackScheduleStep {
            now_seconds: 0.0,
            local_position: 30.0,
            local_can_control: true,
            speed_supported: true,
            context: "an authoritative doSeek frame suppresses ordinary drift correction",
        },
    );

    let mut paused_runtime = runtime_with_fixed_ping();
    let paused_room = adjusted_room_playstate(
        &paused_runtime,
        inbound_playstate(10.0, true, false, 100.0, 0.10),
        100.0,
        100.5,
    );
    assert_close(
        paused_room.position.expect("paused room position"),
        10.0,
        "paused room state must not absorb ping or scheduler projection",
    );
    let mut paused_reference = ReferencePlaybackOracle::default();
    assert_playback_step(
        &mut paused_runtime,
        &mut paused_reference,
        paused_room,
        PlaybackScheduleStep {
            now_seconds: 0.0,
            local_position: 14.5,
            local_can_control: true,
            speed_supported: false,
            context: "paused room retains the raw rewind target",
        },
    );
}
