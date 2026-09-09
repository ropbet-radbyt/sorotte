use super::*;

#[test]
fn a_new_pause_survives_the_delayed_acknowledgement_of_its_prior_play() {
    let mut fixture = SeekFixture::new();
    assert!(fixture.runtime.run_set_paused(false).unwrap());
    fixture.queue_observation(false, 0.1);
    assert!(
        fixture
            .runtime
            .run_state_sync_heartbeat_legacy_ping_compatible(false)
    );
    let play = fixture.take_response();
    assert_eq!(play.playstate.as_ref().unwrap().paused, Some(false));
    let counter = play
        .ignoring_on_the_fly
        .as_ref()
        .and_then(|ignore| ignore.client);
    let mut acknowledgement = IgnoringOnTheFlyPayload::new().with_server(1);
    if let Some(counter) = counter {
        acknowledgement = acknowledgement.with_client(counter);
    }

    // The server has accepted Play, but its echo has not yet reached this
    // controller when the user presses Pause. The new command is based on
    // the preceding revision until that exact acknowledgement arrives.
    assert!(fixture.runtime.run_set_paused(true).unwrap());
    fixture.queue_observation(true, 0.1);
    fixture.reconcile(
        StatePayload::new()
            .with_playstate(
                PlaystatePayload::new()
                    .with_position(0.1)
                    .with_paused(false)
                    .with_do_seek(false)
                    .with_set_by("alice")
                    .with_transport_revision(35),
            )
            .with_ignoring_on_the_fly(acknowledgement),
    );
    fixture
        .runtime
        .drain_player_transport_coordination(1.4)
        .unwrap();
    assert_eq!(
        fixture
            .runtime
            .playback_coordination_snapshot()
            .pending_local_pause_intent,
        Some(true),
        "the older Play acknowledgement must not erase the newer Pause"
    );
    fixture.queue_observation(true, 0.1);
    assert!(
        fixture
            .runtime
            .run_state_sync_heartbeat_legacy_ping_compatible(false)
    );
    let pause = fixture.take_response();
    let playstate = pause
        .playstate
        .expect("the next heartbeat must publish Pause");
    assert_eq!(playstate.paused, Some(true));
    assert_eq!(playstate.transport_revision().unwrap(), Some(35));
    assert_ne!(playstate.do_seek, Some(true));
}

type TestRuntime = ClientRuntime<CoordinatedTestPlayer, QueuedRuntimeControl>;

fn emit_pause_mutation(fixture: &mut SeekFixture, paused: bool) -> StatePayload {
    assert!(fixture.runtime.run_set_paused(paused).unwrap());
    fixture.queue_observation(paused, 0.1);
    assert!(
        fixture
            .runtime
            .run_state_sync_heartbeat_legacy_ping_compatible(false)
    );
    let state = fixture.take_response();
    assert_eq!(state.playstate.as_ref().unwrap().paused, Some(paused));
    assert!(state.ignoring_on_the_fly.as_ref().unwrap().client.unwrap() > 0);
    assert!(
        fixture
            .runtime
            .playback_coordination
            .pending_local_transport_echo
            .is_some()
    );
    state
}

fn pause_mutation_echo(state: &StatePayload) -> StatePayload {
    let playstate = state.playstate.as_ref().unwrap();
    StatePayload::new()
        .with_playstate(
            PlaystatePayload::new()
                .with_position(0.2)
                .with_paused(playstate.paused.unwrap())
                .with_do_seek(false)
                .with_set_by("alice")
                .with_transport_revision(playstate.transport_revision().unwrap().unwrap() + 1),
        )
        .with_ignoring_on_the_fly(
            IgnoringOnTheFlyPayload::new()
                .with_server(1)
                .with_client(state.ignoring_on_the_fly.as_ref().unwrap().client.unwrap()),
        )
}

#[test]
fn both_pause_directions_and_player_acknowledgement_orders_preserve_the_newer_command() {
    for initial_paused in [false, true] {
        for player_before_echo in [false, true] {
            let mut fixture = SeekFixture::with_paused(initial_paused);
            let preceding = emit_pause_mutation(&mut fixture, !initial_paused);
            assert!(fixture.runtime.run_set_paused(initial_paused).unwrap());
            if player_before_echo {
                fixture.queue_observation(initial_paused, 0.2);
            }
            fixture.reconcile(pause_mutation_echo(&preceding));
            fixture
                .runtime
                .drain_player_transport_coordination(1.4)
                .unwrap();
            assert_eq!(
                fixture
                    .runtime
                    .playback_coordination_snapshot()
                    .pending_local_pause_intent,
                Some(initial_paused),
                "initial paused={initial_paused}, physical acknowledgement first={player_before_echo}"
            );
            fixture.queue_observation(initial_paused, 0.2);
            fixture
                .runtime
                .drain_player_transport_coordination(1.5)
                .unwrap();
            assert!(
                fixture
                    .runtime
                    .run_state_sync_heartbeat_legacy_ping_compatible(false)
            );
            let newer = fixture.take_response();
            let playstate = newer.playstate.as_ref().unwrap();
            assert_eq!(playstate.paused, Some(initial_paused));
            assert_eq!(playstate.transport_revision().unwrap(), Some(35));
            assert_ne!(playstate.do_seek, Some(true));

            fixture.reconcile(pause_mutation_echo(&newer));
            fixture
                .runtime
                .drain_player_transport_coordination(1.6)
                .unwrap();
            assert_eq!(
                fixture
                    .runtime
                    .playback_coordination_snapshot()
                    .pending_local_pause_intent,
                None
            );
            assert_eq!(
                fixture.runtime.session.current_room_transport_revision(),
                Some(36)
            );
        }
    }
}

#[test]
fn a_pause_echo_requires_its_counter_author_revision_and_transport_kind() {
    for mismatch in [
        "missing-counter",
        "wrong-counter",
        "other-user",
        "seek",
        "wrong-pause",
        "skipped-revision",
        "missing-revision",
    ] {
        let mut fixture = SeekFixture::new();
        let preceding = emit_pause_mutation(&mut fixture, false);
        assert!(fixture.runtime.run_set_paused(true).unwrap());
        fixture.queue_observation(true, 0.2);
        let mut echo = pause_mutation_echo(&preceding);
        match mismatch {
            "missing-counter" => echo.ignoring_on_the_fly.as_mut().unwrap().client = None,
            "wrong-counter" => echo.ignoring_on_the_fly.as_mut().unwrap().client = Some(99),
            "other-user" => echo.playstate.as_mut().unwrap().set_by = Some("bob".into()),
            "seek" => echo.playstate.as_mut().unwrap().do_seek = Some(true),
            "wrong-pause" => echo.playstate.as_mut().unwrap().paused = Some(true),
            "skipped-revision" => {
                echo.playstate = Some(echo.playstate.take().unwrap().with_transport_revision(36))
            }
            "missing-revision" => {
                echo.playstate = Some(
                    PlaystatePayload::new()
                        .with_position(0.2)
                        .with_paused(false)
                        .with_do_seek(false)
                        .with_set_by("alice"),
                );
            }
            _ => unreachable!(),
        }
        fixture.reconcile(echo);
        fixture
            .runtime
            .drain_player_transport_coordination(1.4)
            .unwrap();
        assert!(
            fixture
                .runtime
                .playback_coordination
                .pending_local_pause_intent
                .as_ref()
                .is_none_or(|intent| intent.base_transport_revision != Some(35)),
            "{mismatch} must not grant the newer command the predecessor's revision"
        );
    }
}

#[test]
fn an_older_pause_frame_cannot_replace_the_predecessor_of_a_newer_command() {
    for canonical_paused in [false, true] {
        let mut fixture = SeekFixture::with_paused(canonical_paused);
        let mut older_frame = emit_pause_mutation(&mut fixture, !canonical_paused);
        assert!(fixture.runtime.run_set_paused(canonical_paused).unwrap());
        assert_eq!(
            fixture
                .runtime
                .playback_coordination
                .active_local_pause_intent(&fixture.runtime.session),
            Some(canonical_paused),
            "the newer command has returned to the old canonical value"
        );
        let predecessor = fixture
            .runtime
            .playback_coordination
            .pending_local_transport_echo
            .clone();
        assert!(predecessor.is_some());
        let counter = older_frame.ignoring_on_the_fly.as_mut().unwrap();
        counter.client = Some(counter.client.unwrap() + 1);
        // A larger counter cannot make an older frame match the newer
        // pending command, even when both share the current room revision.
        fixture
            .runtime
            .playback_coordination
            .record_emitted_local_transport(&fixture.runtime.session, &older_frame);
        assert_eq!(
            fixture
                .runtime
                .playback_coordination
                .pending_local_transport_echo,
            predecessor,
            "a superseded pause frame cannot replace the actual issued predecessor"
        );
    }
}

#[test]
fn an_explicit_pause_mutation_cannot_be_coalesced_away_by_heartbeats() {
    let mut fixture = SeekFixture::new();
    assert!(fixture.runtime.run_set_paused(false).unwrap());
    fixture.runtime.flush_queued_protocol_messages();
    assert!(
        fixture
            .runtime
            .run_state_sync_heartbeat_legacy_ping_compatible(false)
    );
    let first = fixture
        .runtime
        .control()
        .outbound_messages()
        .front()
        .unwrap()
        .clone();
    assert!(
        matches!(&first, ProtocolMessage::State(state) if state.state.playstate.as_ref().is_some_and(|playstate| playstate.paused == Some(false)))
    );
    for _ in 0..8 {
        assert!(
            fixture
                .runtime
                .run_state_sync_heartbeat_legacy_ping_compatible(false)
        );
    }
    let messages = fixture.runtime.flush_queued_protocol_messages();
    assert_eq!(
        messages.len(),
        2,
        "one causal command plus one coalesced heartbeat"
    );
    assert_eq!(
        messages[0], first,
        "the older command must retain its exact FIFO bytes"
    );
    assert!(
        matches!(&messages[0], ProtocolMessage::State(state) if state.state.playstate.as_ref().is_some_and(|playstate| playstate.paused == Some(false)))
    );
    assert!(
        matches!(&messages[1], ProtocolMessage::State(state) if state.state.playstate.is_none())
    );
}

#[test]
fn a_causal_pause_keeps_participant_reports_cancellable() {
    assert_causal_pause_keeps_participant_reports_cancellable();
}

pub(super) fn assert_causal_pause_keeps_participant_reports_cancellable() {
    let mut fixture = SeekFixture::new();
    assert!(fixture.runtime.run_set_paused(false).unwrap());
    fixture.runtime.flush_queued_protocol_messages();
    fixture
        .runtime
        .playback_coordination
        .participant_status
        .last_participant_status_fingerprint = None;
    assert!(
        fixture
            .runtime
            .run_state_sync_heartbeat_legacy_ping_compatible(false)
    );
    let pending: Vec<_> = fixture
        .runtime
        .control()
        .outbound_messages()
        .iter()
        .cloned()
        .collect();
    assert_eq!(reports_in(pending.clone()).len(), 1);
    assert!(matches!(&pending[0], ProtocolMessage::State(state)
        if state.state.playstate.as_ref().is_some_and(|playstate| playstate.paused == Some(false))
            && state.state.participant_status_v1().unwrap().is_none()));

    fixture
        .runtime
        .control
        .cancel_protocol_participant_status_reports();
    let remaining = fixture.runtime.flush_queued_protocol_messages();
    assert!(reports_in(remaining.clone()).is_empty());
    assert_eq!(
        remaining,
        vec![pending[0].clone()],
        "status withdrawal must preserve the exact causal Play frame"
    );
}

#[test]
fn pause_queue_keeps_only_current_command_frames_in_order() {
    assert_pause_queue_keeps_only_current_command_frames_in_order();
}

pub(super) fn assert_pause_queue_keeps_only_current_command_frames_in_order() {
    for mismatch in ["none", "pause", "revision", "missing-playstate"] {
        let mut fixture = SeekFixture::new();
        assert!(fixture.runtime.run_set_paused(false).unwrap());
        fixture.runtime.flush_queued_protocol_messages();
        let mut frame = StatePayload::new()
            .with_playstate(
                PlaystatePayload::new()
                    .with_position(0.0)
                    .with_paused(false)
                    .with_transport_revision(34),
            )
            .with_ignoring_on_the_fly(IgnoringOnTheFlyPayload::new().with_client(1));
        match mismatch {
            "none" => {}
            "pause" => frame.playstate.as_mut().unwrap().paused = Some(true),
            "revision" => {
                frame.playstate = Some(frame.playstate.take().unwrap().with_transport_revision(35));
            }
            "missing-playstate" => frame.playstate = None,
            _ => unreachable!(),
        }
        assert!(
            fixture
                .runtime
                .queue_connection_scoped_state_with_participant_status(frame.clone(), true, 1.2)
        );
        // A later heartbeat may absorb only an observational frame. A State
        // matching the pending command must keep its original FIFO identity.
        let heartbeat = StatePayload::new()
            .with_ping(sorotte_protocol::PingPayload::new().with_client_rtt(0.2));
        assert!(
            fixture
                .runtime
                .queue_connection_scoped_state_with_participant_status(
                    heartbeat.clone(),
                    true,
                    1.3
                )
        );
        fixture
            .runtime
            .control
            .cancel_protocol_participant_status_reports();
        let messages = fixture.runtime.flush_queued_protocol_messages();
        if mismatch == "none" {
            assert_eq!(messages.len(), 2);
            assert_eq!(messages[0], ProtocolMessage::state(frame));
            assert_eq!(messages[1], ProtocolMessage::state(heartbeat));
        } else {
            frame.ping = heartbeat.ping;
            assert_eq!(messages, vec![ProtocolMessage::state(frame)], "{mismatch}");
        }
    }
}

struct SeekFixture {
    runtime: TestRuntime,
    generation: PlayerMediaGeneration,
    sequence: u64,
    seek_counter: u32,
    seek_paused: bool,
    seek_base_revision: u64,
}

impl SeekFixture {
    fn new() -> Self {
        Self::with_paused(true)
    }

    fn with_paused(paused: bool) -> Self {
        let mut runtime = ClientRuntime::new(
            participant_status_session(),
            CoordinatedTestPlayer {
                ordered_delivery: true,
                ..CoordinatedTestPlayer::default()
            },
            QueuedRuntimeControl::default(),
        );
        let plan = runtime.prepare_playback_media(
            LogicalMediaId::new("ordered-issued-seek-then-play").unwrap(),
            MediaTransportKind::NetworkVod,
            1.0,
        );
        let mut fixture = Self {
            runtime,
            generation: PlayerMediaGeneration::new(plan.media_generation),
            sequence: 0,
            seek_counter: 0,
            seek_paused: paused,
            seek_base_revision: 34,
        };
        fixture.queue_observation(paused, 0.0);
        fixture
            .runtime
            .drain_player_transport_coordination(1.0)
            .unwrap();
        fixture
            .runtime
            .session_mut()
            .apply_message_json_at(
                &format!(r#"{{"State":{{"playstate":{{"position":0.0,"paused":{paused},"doSeek":false,"setBy":"alice","sorotteTransportRevision":34}}}}}}"#),
                1.0,
            )
            .unwrap();
        fixture.runtime.flush_queued_protocol_messages();
        fixture
    }

    fn queue_observation(&mut self, paused: bool, position: f64) {
        self.sequence += 1;
        let epoch = PlayerAttachmentEpoch::new(1);
        let observed = PlayerObservationTimestamp::from_adapter_start(Duration::from_millis(
            900 + self.sequence * 100,
        ));
        let mut transport = if paused {
            ordered_paused_transport(self.generation, observed, position)
        } else {
            ordered_playing_transport(self.generation, observed, position)
        };
        // These ordering contracts use a warm target, so a legitimate cold
        // media preparation pause cannot be mistaken for the old Seek echo.
        transport.seekable_ranges =
            SnapshotField::Known(vec![sorotte_player_api::PlayerSeekableRange::new(
                0.0, 30.0,
            )]);
        transport.buffered_duration_seconds = SnapshotField::Known(30.0 - position);
        self.runtime.player.ordered_batches.push_back(ordered_batch(
            epoch,
            self.sequence,
            self.sequence,
            Some(active_snapshot(
                epoch,
                self.sequence,
                LoadAttemptId::new(1),
                self.generation,
                transport,
            )),
            Vec::new(),
            Vec::new(),
        ));
    }

    fn seek(&mut self) {
        self.emit_seek();
        assert!(
            self.runtime
                .playback_coordination
                .pending_local_transport_echo
                .is_some()
        );
    }

    fn emit_seek(&mut self) -> StatePayload {
        self.seek_base_revision = self
            .runtime
            .session
            .current_room_transport_revision()
            .unwrap();
        assert!(self.runtime.run_seek_to_position(11.0).unwrap());
        let state = self
            .runtime
            .flush_queued_protocol_messages()
            .into_iter()
            .find_map(|message| match message {
                ProtocolMessage::State(state)
                    if state
                        .state
                        .playstate
                        .as_ref()
                        .is_some_and(|playstate| playstate.do_seek == Some(true)) =>
                {
                    Some(state.state)
                }
                _ => None,
            })
            .expect("local Seek must be queued before Play");
        let playstate = state.playstate.as_ref().unwrap();
        assert_eq!(playstate.paused, Some(self.seek_paused));
        assert_eq!(
            playstate.transport_revision().unwrap(),
            Some(self.seek_base_revision)
        );
        self.seek_counter = state.ignoring_on_the_fly.as_ref().unwrap().client.unwrap();
        state
    }

    fn play_after_seek(&mut self) {
        self.seek();
        self.queue_observation(true, 11.0);
        assert!(self.runtime.run_set_paused(false).unwrap());
        assert_eq!(
            self.runtime
                .playback_coordination_snapshot()
                .pending_local_pause_intent,
            Some(false)
        );
        assert!(
            self.runtime
                .run_state_sync_heartbeat_legacy_ping_compatible(false)
        );
        let response = self.take_response();
        assert!(
            response.playstate.is_none(),
            "the outstanding Seek still fences the immediate Play heartbeat"
        );
    }

    fn echo(&self) -> StatePayload {
        StatePayload::new()
            .with_playstate(
                PlaystatePayload::new()
                    .with_position(11.0)
                    .with_paused(self.seek_paused)
                    .with_do_seek(true)
                    .with_set_by("alice")
                    .with_transport_revision(self.seek_base_revision + 1),
            )
            .with_ignoring_on_the_fly(
                IgnoringOnTheFlyPayload::new()
                    .with_server(1)
                    .with_client(self.seek_counter),
            )
    }

    fn reconcile(&mut self, state: StatePayload) -> StatePayload {
        assert!(
            self.runtime
                .run_state_sync_reconcile_with_inbound_state_legacy_ping_compatible_at(
                    state, false, 1.3
                )
        );
        self.take_response()
    }

    fn take_response(&mut self) -> StatePayload {
        self.runtime
            .flush_queued_protocol_messages()
            .into_iter()
            .find_map(|message| match message {
                ProtocolMessage::State(state) if state.state.ping.is_some() => Some(state.state),
                _ => None,
            })
            .expect("the State exchange must emit a response")
    }

    fn heartbeat_play(&mut self) -> StatePayload {
        self.queue_observation(false, 11.2);
        assert!(
            self.runtime
                .run_state_sync_heartbeat_legacy_ping_compatible(false)
        );
        let response = self.take_response();
        let playstate = response
            .playstate
            .as_ref()
            .expect("the ordinary heartbeat must deliver the newer Play");
        assert_eq!(playstate.paused, Some(false));
        assert_eq!(
            playstate.transport_revision().unwrap(),
            Some(self.seek_base_revision + 1)
        );
        assert_ne!(
            playstate.do_seek,
            Some(true),
            "Play must not duplicate the acknowledged Seek"
        );
        response
    }
}

#[test]
fn later_seek_with_a_new_counter_delivers_its_own_following_play() {
    let mut fixture = SeekFixture::new();
    fixture.seek();
    let first_counter = fixture.seek_counter;
    fixture.queue_observation(true, 11.0);
    fixture.play_after_seek();
    assert_eq!(fixture.seek_counter, first_counter + 1);
    assert_eq!(fixture.seek_base_revision, 34);
    fixture.queue_observation(false, 11.1);
    let later_echo = fixture.echo();
    assert!(fixture.reconcile(later_echo).playstate.is_none());
    fixture.heartbeat_play();
}

#[test]
fn seek_after_canonical_acknowledgement_can_reuse_counter_on_its_new_revision() {
    let mut fixture = SeekFixture::new();
    fixture.seek();
    let first_counter = fixture.seek_counter;
    fixture.queue_observation(true, 11.0);
    let first_echo = fixture.echo();
    fixture.reconcile(first_echo);
    assert_eq!(
        fixture.runtime.session.current_room_transport_revision(),
        Some(35)
    );
    assert_eq!(
        fixture
            .runtime
            .session
            .model
            .playback
            .client_ignoring_on_the_fly,
        0
    );

    fixture.queue_observation(true, 11.0);
    fixture.play_after_seek();
    assert_eq!(fixture.seek_counter, first_counter);
    assert_eq!(fixture.seek_base_revision, 35);
    fixture.queue_observation(false, 11.1);
    let next_echo = fixture.echo();
    assert!(fixture.reconcile(next_echo).playstate.is_none());
    fixture.heartbeat_play();
}

#[test]
fn already_admitted_canonical_seek_cannot_be_replayed_as_a_fresh_acknowledgement() {
    for unavailable_projection in [false, true] {
        let mut fixture = SeekFixture::new();
        fixture.play_after_seek();
        fixture.runtime.session_mut().apply_message_json_at(
            r#"{"State":{"playstate":{"position":11.0,"paused":true,"doSeek":true,"setBy":"alice","sorotteTransportRevision":35}}}"#,
            1.25,
        ).unwrap();
        assert_eq!(
            fixture.runtime.session.current_room_transport_revision(),
            Some(35)
        );
        assert_eq!(
            fixture
                .runtime
                .playback_coordination
                .pending_local_pause_intent
                .as_ref()
                .unwrap()
                .base_transport_revision,
            Some(34)
        );
        if unavailable_projection {
            fixture.runtime.pending_state_sync_player_error = Some(PlayerError::OperationFailed(
                "unavailable projection after independent canonical admission".into(),
            ));
        }
        let repeated_echo = fixture.echo();
        fixture.reconcile(repeated_echo);
        assert!(
            fixture
                .runtime
                .playback_coordination
                .pending_local_transport_echo
                .is_none()
        );
        assert!(
            !fixture
                .runtime
                .playback_coordination
                .pending_local_pause_intent
                .as_ref()
                .is_some_and(|intent| intent.base_transport_revision == Some(35)),
            "already having the matching canonical State cannot prove this repeated packet was admitted"
        );
    }
}

#[test]
fn ordered_playing_seek_echo_preserves_and_delivers_the_later_explicit_pause() {
    let mut fixture = SeekFixture::with_paused(false);
    fixture.seek();
    fixture.queue_observation(false, 11.0);
    assert!(fixture.runtime.run_set_paused(true).unwrap());
    let commands_after_pause = fixture.runtime.player.commands.len();
    assert!(
        fixture
            .runtime
            .run_state_sync_heartbeat_legacy_ping_compatible(false)
    );
    assert!(fixture.take_response().playstate.is_none());

    fixture.queue_observation(true, 11.1);
    let echo = fixture.echo();
    assert!(fixture.reconcile(echo).playstate.is_none());
    assert_eq!(
        fixture
            .runtime
            .playback_coordination_snapshot()
            .pending_local_pause_intent,
        Some(true)
    );
    fixture.queue_observation(true, 11.2);
    assert!(
        fixture
            .runtime
            .run_state_sync_heartbeat_legacy_ping_compatible(false)
    );
    let response = fixture.take_response();
    let playstate = response
        .playstate
        .expect("the heartbeat must deliver the newer Pause");
    assert_eq!(playstate.paused, Some(true));
    assert_eq!(playstate.transport_revision().unwrap(), Some(35));
    assert_ne!(playstate.do_seek, Some(true));
    assert!(
        fixture.runtime.player.commands[commands_after_pause..]
            .iter()
            .all(|command| !matches!(command, PlayerCommand::SetPaused(false)))
    );
}

#[test]
fn reset_and_reused_seek_counter_has_no_special_rebase_authority() {
    let mut fixture = SeekFixture::new();
    fixture.seek();
    let earlier_echo = fixture.echo();
    let earlier_counter = fixture.seek_counter;
    fixture.reconcile(
        StatePayload::new().with_ignoring_on_the_fly(IgnoringOnTheFlyPayload::new().with_server(7)),
    );
    assert_eq!(
        fixture
            .runtime
            .session
            .model
            .playback
            .client_ignoring_on_the_fly,
        0
    );
    assert_eq!(
        fixture.runtime.session.current_room_transport_revision(),
        Some(34)
    );
    fixture.queue_observation(true, 11.0);
    fixture.emit_seek();
    assert_eq!(
        fixture.seek_counter, earlier_counter,
        "the actual legacy wire counter is reused"
    );
    assert!(
        fixture
            .runtime
            .playback_coordination
            .pending_local_transport_echo
            .is_none()
    );
    fixture.queue_observation(true, 11.0);
    fixture.runtime.run_set_paused(false).unwrap();
    fixture.reconcile(earlier_echo);
    assert!(
        !fixture
            .runtime
            .playback_coordination
            .pending_local_pause_intent
            .as_ref()
            .is_some_and(|intent| intent.base_transport_revision == Some(35))
    );
}

#[test]
fn saturated_seek_counter_has_no_special_rebase_authority() {
    for previous_counter in [u32::MAX - 1, u32::MAX] {
        let mut fixture = SeekFixture::new();
        fixture
            .runtime
            .session
            .model
            .playback
            .client_ignoring_on_the_fly = previous_counter;
        fixture.emit_seek();
        assert_eq!(fixture.seek_counter, u32::MAX);
        assert!(
            fixture
                .runtime
                .playback_coordination
                .pending_local_transport_echo
                .is_none()
        );
        fixture.queue_observation(true, 11.0);
        fixture.runtime.run_set_paused(false).unwrap();
        let echo = fixture.echo();
        fixture.reconcile(echo);
        assert!(
            !fixture
                .runtime
                .playback_coordination
                .pending_local_pause_intent
                .as_ref()
                .is_some_and(|intent| intent.base_transport_revision == Some(35))
        );
    }
}

#[test]
fn ordered_seek_echo_preserves_and_delivers_the_later_explicit_play() {
    let mut fixture = SeekFixture::new();
    fixture.play_after_seek();
    let commands_before_echo = fixture.runtime.player.commands.len();
    let assert_no_pause = |fixture: &SeekFixture, stage: &str| {
        assert!(
            fixture.runtime.player.commands[commands_before_echo..]
                .iter()
                .all(|command| !matches!(command, PlayerCommand::SetPaused(true))),
            "{stage}: an earlier Seek must not undo Play; commands={:?}; pending={:?}; desired={:?}",
            &fixture.runtime.player.commands[commands_before_echo..],
            fixture
                .runtime
                .playback_coordination
                .pending_local_pause_intent,
            fixture.runtime.playback_coordination.desired_fingerprint
        );
    };
    fixture.queue_observation(false, 11.1);
    let echo = fixture.echo();
    assert!(fixture.reconcile(echo.clone()).playstate.is_none());
    assert_no_pause(&fixture, "first Seek echo");
    assert_eq!(
        fixture.runtime.session().current_room_transport_revision(),
        Some(35)
    );
    assert_eq!(
        fixture
            .runtime
            .playback_coordination_snapshot()
            .pending_local_pause_intent,
        Some(false)
    );
    assert!(
        fixture
            .runtime
            .playback_coordination
            .pending_local_transport_echo
            .is_none()
    );
    let play = fixture.heartbeat_play();
    assert_no_pause(&fixture, "first deferred Play heartbeat");

    // A duplicate acknowledgement cannot re-arm the predecessor or cancel
    // the already rebound intent before its own canonical Play arrives.
    fixture.reconcile(echo.clone());
    assert_no_pause(&fixture, "duplicate Seek before Play commit");
    fixture.heartbeat_play();
    assert_no_pause(&fixture, "heartbeat following duplicate Seek");
    let mut play_ack = IgnoringOnTheFlyPayload::new().with_server(2);
    if let Some(counter) = play.ignoring_on_the_fly.and_then(|ignore| ignore.client) {
        play_ack = play_ack.with_client(counter);
    }
    fixture.queue_observation(false, 11.3);
    fixture.reconcile(
        StatePayload::new()
            .with_playstate(
                PlaystatePayload::new()
                    .with_position(11.3)
                    .with_paused(false)
                    .with_do_seek(false)
                    .with_set_by("alice")
                    .with_transport_revision(36),
            )
            .with_ignoring_on_the_fly(play_ack),
    );
    fixture
        .runtime
        .drain_player_transport_coordination(1.4)
        .unwrap();
    assert_no_pause(&fixture, "canonical Play commit");
    assert_eq!(
        fixture
            .runtime
            .playback_coordination_snapshot()
            .pending_local_pause_intent,
        None
    );
    fixture.reconcile(echo);
    assert_no_pause(&fixture, "stale duplicate Seek after Play commit");
    fixture.queue_observation(false, 11.4);
    fixture
        .runtime
        .drain_player_transport_coordination(1.5)
        .unwrap();
    assert_eq!(
        fixture.runtime.session().current_room_transport_revision(),
        Some(36)
    );
    assert_no_pause(&fixture, "physical Playing after stale duplicate Seek");
}

#[test]
fn seek_echo_rebases_after_ping_only_admission_with_unavailable_player_projection() {
    let mut fixture = SeekFixture::new();
    fixture.play_after_seek();
    fixture.runtime.pending_state_sync_player_error = Some(PlayerError::OperationFailed(
        "retained player refresh failure".into(),
    ));
    let echo = fixture.echo();
    assert!(fixture.reconcile(echo).playstate.is_none());
    assert_eq!(
        fixture.runtime.session().current_room_transport_revision(),
        Some(35)
    );
    assert_eq!(
        fixture
            .runtime
            .playback_coordination_snapshot()
            .pending_local_pause_intent,
        Some(false)
    );
    assert!(
        fixture
            .runtime
            .drain_player_transport_coordination(1.31)
            .is_err(),
        "the player error must remain visible"
    );
    fixture.heartbeat_play();
}

#[test]
fn protocol_suppressed_matching_echo_waits_for_actual_admission() {
    for unavailable_projection in [false, true] {
        let mut fixture = SeekFixture::new();
        fixture.play_after_seek();
        fixture
            .runtime
            .session
            .model
            .playback
            .client_ignoring_on_the_fly = fixture.seek_counter + 1;
        if unavailable_projection {
            fixture.runtime.pending_state_sync_player_error = Some(PlayerError::OperationFailed(
                "unavailable projection".into(),
            ));
        }
        let mut suppressed = fixture.echo();
        suppressed.ignoring_on_the_fly.as_mut().unwrap().server = None;
        fixture.reconcile(suppressed);
        assert_eq!(
            fixture.runtime.session().current_room_transport_revision(),
            Some(34)
        );
        let intent = fixture
            .runtime
            .playback_coordination
            .pending_local_pause_intent
            .as_ref()
            .unwrap();
        assert_eq!(intent.base_transport_revision, Some(34));
        assert!(intent.preceding_local_transport.is_some());
        assert!(
            fixture
                .runtime
                .playback_coordination
                .pending_local_transport_echo
                .is_some()
        );
        let echo = fixture.echo();
        fixture.reconcile(echo);
        assert_eq!(
            fixture
                .runtime
                .playback_coordination_snapshot()
                .pending_local_pause_intent,
            Some(false)
        );
        assert_eq!(
            fixture
                .runtime
                .playback_coordination
                .pending_local_pause_intent
                .as_ref()
                .unwrap()
                .base_transport_revision,
            Some(35)
        );
        assert!(
            fixture
                .runtime
                .playback_coordination
                .pending_local_transport_echo
                .is_none()
        );
    }
}

#[test]
fn seek_echo_requires_every_recorded_wire_identity_axis() {
    for mismatch in [
        "actor",
        "nonce",
        "missing-nonce",
        "target",
        "pause",
        "seek",
        "old-revision",
        "skipped-revision",
        "zero-revision",
        "max-revision",
        "missing-revision",
        "missing-position",
        "missing-pause",
        "missing-actor",
    ] {
        let mut fixture = SeekFixture::new();
        fixture.play_after_seek();
        let mut echo = fixture.echo();
        let playstate = echo.playstate.as_mut().unwrap();
        match mismatch {
            "actor" => playstate.set_by = Some("bob".into()),
            "nonce" => {
                echo.ignoring_on_the_fly.as_mut().unwrap().client = Some(fixture.seek_counter + 1)
            }
            "missing-nonce" => echo.ignoring_on_the_fly.as_mut().unwrap().client = None,
            "target" => playstate.position = Some(12.0),
            "pause" => playstate.paused = Some(false),
            "seek" => playstate.do_seek = Some(false),
            "old-revision" => *playstate = playstate.clone().with_transport_revision(34),
            "skipped-revision" => *playstate = playstate.clone().with_transport_revision(36),
            "zero-revision" => *playstate = playstate.clone().with_transport_revision(0),
            "max-revision" => *playstate = playstate.clone().with_transport_revision(u64::MAX),
            "missing-revision" => {
                *playstate = PlaystatePayload::new()
                    .with_position(11.0)
                    .with_paused(true)
                    .with_do_seek(true)
                    .with_set_by("alice")
            }
            "missing-position" => playstate.position = None,
            "missing-pause" => playstate.paused = None,
            "missing-actor" => playstate.set_by = None,
            _ => unreachable!(),
        }
        fixture.reconcile(echo);
        let advanced = fixture.runtime.session().current_room_transport_revision() != Some(34);
        assert_eq!(
            fixture
                .runtime
                .playback_coordination
                .pending_local_transport_echo
                .is_none(),
            advanced,
            "{mismatch}"
        );
        if let Some(intent) = fixture
            .runtime
            .playback_coordination
            .pending_local_pause_intent
            .as_ref()
        {
            assert_eq!(intent.base_transport_revision, Some(34), "{mismatch}");
            assert_eq!(
                intent.preceding_local_transport.is_none(),
                advanced,
                "{mismatch}"
            );
        }
        if !advanced {
            let valid_echo = fixture.echo();
            fixture.reconcile(valid_echo);
            assert_eq!(
                fixture
                    .runtime
                    .playback_coordination_snapshot()
                    .pending_local_pause_intent,
                Some(false),
                "{mismatch}: rejected traffic must not swallow the later valid acknowledgement"
            );
        }
    }
}

#[test]
fn seek_predecessor_expires_on_room_connection_and_media_boundaries() {
    for boundary in [
        "requested-room",
        "canonical-room",
        "username",
        "connection",
        "media",
        "retired-media",
        "adapter",
        "authorization",
    ] {
        let mut fixture = SeekFixture::new();
        fixture.play_after_seek();
        let echo = fixture.echo();
        match boundary {
            "requested-room" => {
                assert!(fixture.runtime.run_set_room("room2").unwrap());
                assert_eq!(fixture.runtime.session().room(), Some("room1"));
            }
            "canonical-room" => {
                fixture.runtime.session_mut().apply_message_json(r#"{"Hello":{"username":"alice","room":{"name":"room2"},"version":"1.7.5"}}"#).unwrap();
            }
            "username" => {
                fixture
                    .runtime
                    .session_mut()
                    .apply_message_json(
                        r#"{"Hello":{"username":"bob","room":{"name":"room1"},"version":"1.7.5"}}"#,
                    )
                    .unwrap();
            }
            "connection" => fixture
                .runtime
                .playback_coordination
                .begin_protocol_connection_generation(&fixture.runtime.session),
            "media" => {
                fixture.runtime.prepare_playback_media(
                    LogicalMediaId::new("other-media").unwrap(),
                    MediaTransportKind::LocalFile,
                    1.2,
                );
            }
            "retired-media" => {
                fixture.runtime.playback_coordination.retire_media();
            }
            "adapter" => {
                fixture
                    .runtime
                    .playback_coordination
                    .reset_adapter_epoch(1.2);
            }
            "authorization" => {
                fixture
                    .runtime
                    .playback_coordination
                    .pending_local_pause_intent
                    .as_mut()
                    .unwrap()
                    .authorization =
                    LocalIntentAuthorization::AwaitingControlledRoomReauthentication
            }
            _ => unreachable!(),
        }
        fixture.reconcile(echo);
        assert!(
            fixture
                .runtime
                .playback_coordination
                .pending_local_transport_echo
                .is_none(),
            "{boundary}"
        );
        assert!(
            !fixture
                .runtime
                .playback_coordination
                .pending_local_pause_intent
                .as_ref()
                .is_some_and(|intent| intent.base_transport_revision == Some(35)),
            "{boundary}"
        );
    }
}

#[test]
fn every_new_seek_attempt_invalidates_the_earlier_play_association_before_refresh() {
    for operation in ["absolute", "offset", "undo"] {
        let mut fixture = SeekFixture::new();
        fixture.play_after_seek();
        fixture.queue_observation(true, 11.0);
        fixture.runtime.player.reject_next_acknowledgement = true;
        let result = match operation {
            "absolute" => fixture.runtime.run_seek_to_position(12.0),
            "offset" => fixture.runtime.run_seek_by_offset(1.0),
            "undo" => fixture.runtime.run_undo_seek(),
            _ => unreachable!(),
        };
        assert!(
            result.is_err(),
            "{operation}: pre-dispatch refresh must really fail"
        );
        assert!(
            fixture
                .runtime
                .playback_coordination
                .pending_local_transport_echo
                .is_none(),
            "{operation}"
        );
        assert!(
            fixture
                .runtime
                .playback_coordination
                .pending_local_pause_intent
                .as_ref()
                .is_none_or(|intent| intent.preceding_local_transport.is_none()),
            "{operation}"
        );
    }
}

#[test]
fn later_seek_and_failed_player_dispatch_cannot_reuse_an_older_echo() {
    for fail_player in [false, true] {
        let mut fixture = SeekFixture::new();
        fixture.play_after_seek();
        let old_echo = fixture.echo();
        fixture.runtime.player.reject_seek_commands = fail_player;
        let result = fixture.runtime.run_seek_to_position(12.0);
        assert_eq!(result.is_err(), fail_player);
        fixture.runtime.player.reject_seek_commands = false;
        fixture.reconcile(old_echo);
        assert!(
            !fixture
                .runtime
                .playback_coordination
                .pending_local_pause_intent
                .as_ref()
                .is_some_and(|intent| intent.base_transport_revision == Some(35))
        );
        assert!(
            fixture
                .runtime
                .playback_coordination
                .pending_local_transport_echo
                .is_none()
        );
    }
}

#[test]
fn failed_play_dispatch_cannot_leave_an_intent_to_rebase() {
    let mut fixture = SeekFixture::new();
    fixture.seek();
    fixture.queue_observation(true, 11.0);
    fixture.runtime.player.reject_pause_commands = true;
    assert!(fixture.runtime.run_set_paused(false).is_err());
    fixture.runtime.player.reject_pause_commands = false;
    let echo = fixture.echo();
    fixture.reconcile(echo);
    assert!(
        fixture
            .runtime
            .playback_coordination
            .pending_local_pause_intent
            .is_none()
    );
    assert!(
        fixture
            .runtime
            .playback_coordination
            .pending_local_transport_echo
            .is_none()
    );
}

#[test]
fn acknowledged_seek_without_following_intent_cannot_authorize_a_future_play() {
    let mut fixture = SeekFixture::new();
    fixture.seek();
    let echo = fixture.echo();
    fixture.reconcile(echo.clone());
    assert!(
        fixture
            .runtime
            .playback_coordination
            .pending_local_transport_echo
            .is_none()
    );
    fixture.queue_observation(true, 11.0);
    assert!(fixture.runtime.run_set_paused(false).unwrap());
    assert!(
        fixture
            .runtime
            .playback_coordination
            .pending_local_pause_intent
            .as_ref()
            .unwrap()
            .preceding_local_transport
            .is_none()
    );
    fixture.reconcile(echo);
    assert!(
        fixture
            .runtime
            .playback_coordination
            .pending_local_transport_echo
            .is_none()
    );
}

#[test]
fn unsolicited_self_named_seek_cannot_rebase_an_existing_play() {
    let mut fixture = SeekFixture::new();
    assert!(fixture.runtime.run_set_paused(false).unwrap());
    fixture.seek_counter = 1;
    let echo = fixture.echo();
    fixture.reconcile(echo);
    assert!(
        !fixture
            .runtime
            .playback_coordination
            .pending_local_pause_intent
            .as_ref()
            .is_some_and(|intent| intent.base_transport_revision == Some(35))
    );
}

#[test]
fn later_explicit_pause_wins_over_play_waiting_for_its_seek_echo() {
    let mut fixture = SeekFixture::new();
    fixture.play_after_seek();
    fixture.queue_observation(false, 11.1);
    assert!(fixture.runtime.run_set_paused(true).unwrap());
    let echo = fixture.echo();
    fixture.reconcile(echo);
    fixture.queue_observation(true, 11.2);
    assert!(
        fixture
            .runtime
            .run_state_sync_heartbeat_legacy_ping_compatible(false)
    );
    let response = fixture.take_response();
    assert!(
        response
            .playstate
            .as_ref()
            .is_none_or(|playstate| playstate.paused != Some(false))
    );
    assert_ne!(
        fixture
            .runtime
            .playback_coordination_snapshot()
            .pending_local_pause_intent,
        Some(false)
    );
}

#[test]
fn queued_previous_seek_does_not_consume_the_newer_seek_acknowledgement() {
    let mut fixture = SeekFixture::new();
    fixture.play_after_seek();
    let previous = StatePayload::new()
        .with_playstate(
            PlaystatePayload::new()
                .with_position(0.0)
                .with_paused(true)
                .with_do_seek(true)
                .with_set_by("alice")
                .with_transport_revision(34),
        )
        .with_ignoring_on_the_fly(IgnoringOnTheFlyPayload::new().with_server(7));
    fixture.reconcile(previous);
    assert!(
        fixture
            .runtime
            .playback_coordination
            .pending_local_transport_echo
            .is_some()
    );
    assert_eq!(
        fixture
            .runtime
            .playback_coordination
            .pending_local_pause_intent
            .as_ref()
            .unwrap()
            .base_transport_revision,
        Some(34)
    );
    let echo = fixture.echo();
    fixture.queue_observation(false, 11.1);
    fixture.reconcile(echo);
    fixture.heartbeat_play();
}

#[test]
fn public_room_effect_roundtrip_cannot_resurrect_an_old_seek_predecessor() {
    assert_public_room_effect_roundtrip_cannot_resurrect_an_old_seek_predecessor();
}

pub(super) fn assert_public_room_effect_roundtrip_cannot_resurrect_an_old_seek_predecessor() {
    let mut fixture = SeekFixture::new();
    fixture.play_after_seek();
    fixture
        .runtime
        .emit_effect(ClientEffect::SetRoom("room2".into()))
        .unwrap();
    fixture
        .runtime
        .emit_effect(ClientEffect::SetRoom("room1".into()))
        .unwrap();
    assert_eq!(fixture.runtime.session().room(), Some("room1"));
    assert!(
        fixture
            .runtime
            .playback_coordination
            .participant_status
            .pending_participant_status_room_switch_target
            .is_none()
    );
    assert!(
        fixture
            .runtime
            .playback_coordination
            .pending_local_transport_echo
            .is_none()
    );
    let echo = fixture.echo();
    fixture.reconcile(echo);
    assert!(
        !fixture
            .runtime
            .playback_coordination
            .pending_local_pause_intent
            .as_ref()
            .is_some_and(|intent| intent.base_transport_revision == Some(35))
    );
}

#[test]
fn public_same_room_effect_preserves_newer_play_after_seek() {
    assert_public_same_room_effect_preserves_newer_play_after_seek();
}

pub(super) fn assert_public_same_room_effect_preserves_newer_play_after_seek() {
    let mut fixture = SeekFixture::new();
    fixture.play_after_seek();
    fixture
        .runtime
        .emit_effect(ClientEffect::SetRoom("room1".into()))
        .unwrap();
    fixture.queue_observation(false, 11.1);
    let echo = fixture.echo();
    fixture.reconcile(echo);
    fixture.heartbeat_play();
}

#[derive(Default)]
struct SwitchableCausalSink {
    fail_causal: bool,
    accepted: Vec<StatePayload>,
}

impl ClientEffectSink for SwitchableCausalSink {
    fn emit(&mut self, _: ClientEffect) -> Result<(), ClientEffectError> {
        Ok(())
    }

    fn emit_causal_state(&mut self, state: StatePayload) -> Result<(), ClientEffectError> {
        if self.fail_causal {
            return Err(ClientEffectError::OperationFailed(
                "deliberately rejected causal State".into(),
            ));
        }
        self.accepted.push(state);
        Ok(())
    }
}

#[test]
fn failed_causal_delivery_does_not_arm_or_restore_a_seek_predecessor() {
    for had_predecessor in [false, true] {
        let mut session = participant_status_session();
        session.apply_message_json_at(
            r#"{"State":{"playstate":{"position":0.0,"paused":true,"doSeek":false,"setBy":"alice","sorotteTransportRevision":34}}}"#,
            1.0,
        ).unwrap();
        session.model.playback.local_position = Some(0.0);
        session.model.playback.local_paused = Some(true);
        let mut runtime = ClientRuntime::new(
            session,
            CoordinatedTestPlayer::default(),
            SwitchableCausalSink::default(),
        );
        runtime.prepare_playback_media(
            LogicalMediaId::new("causal-state-delivery-failure").unwrap(),
            MediaTransportKind::LocalFile,
            1.0,
        );
        if had_predecessor {
            assert!(runtime.run_seek_to_position(11.0).unwrap());
            assert!(runtime.run_set_paused(false).unwrap());
            assert!(
                runtime
                    .playback_coordination
                    .pending_local_transport_echo
                    .is_some()
            );
        }
        let previous_counter = runtime.session.model.playback.client_ignoring_on_the_fly;
        let accepted_count = runtime.control.accepted.len();
        runtime.control.fail_causal = true;
        assert!(runtime.run_seek_to_position(12.0).is_err());
        assert_eq!(runtime.control.accepted.len(), accepted_count);
        assert_eq!(
            runtime.session.model.playback.client_ignoring_on_the_fly,
            previous_counter
        );
        assert!(runtime.player.commands.iter().any(|command| matches!(command, PlayerCommand::SetPosition(position) if *position == 12.0)), "the error must occur after the player accepted its operation");
        assert!(
            runtime
                .playback_coordination
                .pending_local_transport_echo
                .is_none()
        );
        assert!(
            runtime
                .playback_coordination
                .pending_local_pause_intent
                .as_ref()
                .is_none_or(|intent| intent.preceding_local_transport.is_none())
        );
    }
}

#[test]
fn malformed_or_unversioned_emitted_states_cannot_arm_seek_correlation() {
    for invalid in [
        "missing-state",
        "missing-session",
        "missing-media",
        "unversioned",
        "max-revision",
        "negative-target",
        "nonfinite-target",
        "missing-target",
        "missing-pause",
        "missing-counter",
        "zero-counter",
        "not-seek",
        "wrong-revision",
        "inactive",
    ] {
        let mut fixture = SeekFixture::new();
        let mut state = StatePayload::new()
            .with_playstate(
                PlaystatePayload::new()
                    .with_position(11.0)
                    .with_paused(true)
                    .with_do_seek(true)
                    .with_transport_revision(34),
            )
            .with_ignoring_on_the_fly(IgnoringOnTheFlyPayload::new().with_client(1));
        match invalid {
            "missing-state" => state.playstate = None,
            "missing-session" => fixture.runtime.session = ClientSession::default(),
            "missing-media" => {
                fixture.runtime.playback_coordination.retire_media();
            }
            "unversioned" => {
                state.playstate = Some(
                    PlaystatePayload::new()
                        .with_position(11.0)
                        .with_paused(true)
                        .with_do_seek(true),
                )
            }
            "max-revision" => {
                fixture.runtime.session_mut().apply_message_json_at(&format!(r#"{{"State":{{"playstate":{{"position":0.0,"paused":true,"setBy":"alice","sorotteTransportRevision":{}}}}}}}"#, u64::MAX), 1.1).unwrap();
                state.playstate = Some(state.playstate.unwrap().with_transport_revision(u64::MAX));
            }
            "negative-target" => state.playstate.as_mut().unwrap().position = Some(-1.0),
            "nonfinite-target" => state.playstate.as_mut().unwrap().position = Some(f64::NAN),
            "missing-target" => state.playstate.as_mut().unwrap().position = None,
            "missing-pause" => state.playstate.as_mut().unwrap().paused = None,
            "missing-counter" => state.ignoring_on_the_fly = None,
            "zero-counter" => state.ignoring_on_the_fly.as_mut().unwrap().client = Some(0),
            "not-seek" => state.playstate.as_mut().unwrap().do_seek = Some(false),
            "wrong-revision" => {
                state.playstate = Some(state.playstate.unwrap().with_transport_revision(35))
            }
            "inactive" => {
                fixture.runtime.session.handle_disconnect(1.1);
            }
            _ => unreachable!(),
        }
        fixture
            .runtime
            .playback_coordination
            .record_emitted_local_transport(&fixture.runtime.session, &state);
        assert!(
            fixture
                .runtime
                .playback_coordination
                .pending_local_transport_echo
                .is_none(),
            "{invalid}"
        );
    }
}
