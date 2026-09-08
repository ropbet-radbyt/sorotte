//! Read unchanged transport properties without blocking the runtime owner.
use super::*;

pub(super) const TRANSPORT_READBACK_COMMAND_TOKEN: u64 = 6;
const READBACK_INTERVAL: Duration = Duration::from_millis(100);
const PROPERTIES: &[&str] = &[
    MPV_PROPERTY_TIME_POS,
    MPV_PROPERTY_PAUSE,
    MPV_PROPERTY_SPEED,
    MPV_PROPERTY_PAUSED_FOR_CACHE,
    MPV_PROPERTY_SEEKING,
    MPV_PROPERTY_SEEKABLE,
    MPV_PROPERTY_CORE_IDLE,
    MPV_PROPERTY_DEMUXER_CACHE_STATE,
    MPV_PROPERTY_DEMUXER_CACHE_IDLE,
    MPV_PROPERTY_CACHE_BUFFERING_STATE,
    MPV_PROPERTY_EOF_REACHED,
];

#[derive(Default)]
pub(super) struct TransportReadbackState {
    last_dispatched_at: Option<Instant>,
    next_property: usize,
    pending: Option<PendingTransportReadback>,
}

struct PendingTransportReadback {
    command_id: u64,
    attachment_epoch: PlayerAttachmentEpoch,
    media_generation: PlayerMediaGeneration,
    attempt_id: LoadAttemptId,
    property: &'static str,
}

impl MpvAdapter {
    pub(super) fn maintain_transport_readback_nonblocking(&mut self) {
        if self.simulation_mode || !self.active_file_loaded {
            return;
        }
        let Some(attempt) = self.player_lifecycle.active_attempt() else {
            return;
        };
        let now = Instant::now();
        if self.transport_readback.pending.is_some()
            || self
                .transport_readback
                .last_dispatched_at
                .is_some_and(|last| now.saturating_duration_since(last) < READBACK_INTERVAL)
        {
            return;
        }
        let index = self.transport_readback.next_property;
        // Paused mpv often omits position events, including native seeks.
        // Interleave position reads with the unchanged-state heartbeat.
        let property = if self.paused && index.is_multiple_of(2) {
            MPV_PROPERTY_TIME_POS
        } else {
            PROPERTIES[(if self.paused { index / 2 } else { index }) % PROPERTIES.len()]
        };
        let (attempt_id, media_generation) = (attempt.id, attempt.media_generation);
        let attachment_epoch = self.lifecycle_epoch();
        let Some(client) = self.ipc_client.as_mut() else {
            return;
        };
        match client.try_get_property_nonblocking(property, TRANSPORT_READBACK_COMMAND_TOKEN) {
            Ok(Some(command_id)) => {
                self.transport_readback.last_dispatched_at = Some(now);
                self.transport_readback.next_property = index.wrapping_add(1);
                self.transport_readback.pending = Some(PendingTransportReadback {
                    command_id,
                    attachment_epoch,
                    media_generation,
                    attempt_id,
                    property,
                });
            }
            Ok(None) => {}
            Err(_) => {
                self.transport_readback.last_dispatched_at = Some(now);
                self.transport_readback.next_property = index.wrapping_add(1);
            }
        }
    }

    pub(super) fn complete_transport_readback(
        &mut self,
        command_id: u64,
        response: Option<(Value, Instant)>,
    ) {
        if !self
            .transport_readback
            .pending
            .as_ref()
            .is_some_and(|pending| pending.command_id == command_id)
        {
            return;
        }
        let pending = self
            .transport_readback
            .pending
            .take()
            .expect("matched readback");
        let Some((response, received_at)) = response else {
            return;
        };
        if pending.attachment_epoch != self.lifecycle_epoch()
            || !self.active_file_loaded
            || !self
                .player_lifecycle
                .active_attempt()
                .is_some_and(|attempt| {
                    attempt.id == pending.attempt_id
                        && attempt.media_generation == pending.media_generation
                        && !attempt.logical_ownership_revoked
                })
        {
            return;
        }
        let Some(data) = response.get("data").filter(|value| !value.is_null()) else {
            return;
        };
        let valid = match pending.property {
            MPV_PROPERTY_TIME_POS | MPV_PROPERTY_SPEED | MPV_PROPERTY_CACHE_BUFFERING_STATE => {
                data.as_f64().is_some_and(f64::is_finite)
            }
            MPV_PROPERTY_DEMUXER_CACHE_STATE => data.is_object(),
            _ => data.is_boolean(),
        };
        if !valid {
            return;
        }
        let previous = self
            .current_ipc_event_observed_at
            .replace(self.observation_timestamp_for(received_at));
        // IPC delivers all earlier structural events before this response.
        // Reuse ordinary property classification at that exact boundary.
        self.handle_ipc_event(&serde_json::json!({
            "event": "property-change", "name": pending.property, "data": data,
        }));
        // Phase is inferred from the ordered property state, and must also be
        // available when the consumer has expired its pre-stall observation.
        let phase = if self.observed_state.eof_reached == Some(true) {
            PlayerTransportPhase::Ended
        } else {
            self.inferred_transport_phase()
        };
        self.queue_transport_telemetry_update(self.transport_update().with_phase(phase));
        self.current_ipc_event_observed_at = previous;
    }

    #[cfg(test)]
    pub(super) fn force_transport_readback_due_for_test(&mut self) {
        self.transport_readback.last_dispatched_at = None;
        self.transport_readback.next_property = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sorotte_player_api::{PlayerAdapter, PlayerEvent};
    use std::{collections::VecDeque, io};

    struct PropertyTransport {
        responses: VecDeque<String>,
        paused: bool,
    }

    impl MpvJsonIpcTransport for PropertyTransport {
        fn send_line_until(&mut self, line: &str, _deadline: Instant) -> io::Result<()> {
            let request: Value = serde_json::from_str(line).map_err(io::Error::other)?;
            let property = request["command"][1].as_str().unwrap_or_default();
            let data = match property {
                MPV_PROPERTY_TIME_POS => json!(12.0),
                MPV_PROPERTY_PAUSE => json!(self.paused),
                MPV_PROPERTY_SPEED => json!(1.0),
                MPV_PROPERTY_SEEKABLE => json!(true),
                MPV_PROPERTY_PAUSED_FOR_CACHE
                | MPV_PROPERTY_SEEKING
                | MPV_PROPERTY_CORE_IDLE
                | MPV_PROPERTY_EOF_REACHED => json!(false),
                MPV_PROPERTY_CACHE_BUFFERING_STATE => json!(100.0),
                _ => Value::Null,
            };
            self.responses.push_back(
                json!({"request_id": request["request_id"], "error":"success", "data":data})
                    .to_string()
                    + "\n",
            );
            Ok(())
        }
        fn read_line_until(&mut self, line: &mut String, _deadline: Instant) -> io::Result<usize> {
            *line = self.responses.pop_front().unwrap_or_default();
            Ok(line.len())
        }
    }

    fn adapter(paused: bool) -> MpvAdapter {
        let mut adapter = MpvAdapter::with_test_transport_and_ipc_timeout(
            PropertyTransport {
                responses: VecDeque::new(),
                paused,
            },
            Duration::from_secs(1),
        );
        let generation = PlayerMediaGeneration::new(7);
        adapter.apply_lifecycle_input(PlayerLifecycleInput::ExternalLoadObserved {
            attachment_epoch: adapter.lifecycle_epoch(),
            media_generation: generation,
            playlist_entry_id: 70,
            observed_target: "heartbeat.mkv".to_owned(),
            file_loaded: true,
        });
        adapter.active_media_generation = Some(generation);
        adapter.active_file_loaded = true;
        adapter.active_generation_has_restarted = true;
        adapter.paused = paused;
        adapter
    }

    #[test]
    fn unchanged_transport_fields_are_resampled_in_both_playing_and_paused_rooms() {
        for paused in [false, true] {
            let mut adapter = adapter(paused);
            for _ in 0..2 {
                let mut seen = [false; 7];
                let deadline = Instant::now() + Duration::from_secs(2);
                while !seen.iter().all(|seen| *seen) {
                    adapter.transport_readback.last_dispatched_at = None;
                    if let Some(batch) = adapter.take_player_event_batch() {
                        for event in &batch.events {
                            if let PlayerEvent::TransportDelta(delta) = &event.event {
                                seen[0] |= delta.position_seconds == Some(12.0);
                                seen[1] |= delta.logical_pause == Some(paused);
                                seen[2] |= delta.playback_rate == Some(1.0);
                                seen[3] |= delta.paused_for_cache == Some(false);
                                seen[4] |= delta.seekable == Some(true);
                                seen[5] |= delta.seeking == Some(false);
                                seen[6] |= delta.phase
                                    == Some(if paused {
                                        PlayerTransportPhase::ReadyPaused
                                    } else {
                                        PlayerTransportPhase::Playing
                                    });
                            }
                        }
                        adapter
                            .acknowledge_player_event_batch(batch.acknowledgement_token)
                            .unwrap();
                    }
                    assert!(
                        Instant::now() < deadline,
                        "missing unchanged telemetry {seen:?}, paused={paused}"
                    );
                    std::thread::yield_now();
                }
            }
        }
    }

    #[test]
    fn readback_rejects_retired_media_and_attachment_and_preserves_ingress_age() {
        let mut adapter = adapter(false);
        let attempt = adapter.player_lifecycle.active_attempt().unwrap();
        let (attempt_id, media_generation) = (attempt.id, attempt.media_generation);
        let epoch = adapter.lifecycle_epoch();
        let ingress = Instant::now() - Duration::from_secs(20);
        adapter.observation_clock_origin = ingress - Duration::from_secs(1);
        for (command_id, attachment_epoch, generation, expected) in [
            (
                1,
                PlayerAttachmentEpoch::new(epoch.get() + 1),
                media_generation,
                false,
            ),
            (2, epoch, PlayerMediaGeneration::new(99), false),
            (3, epoch, media_generation, true),
        ] {
            adapter.transport_readback.pending = Some(PendingTransportReadback {
                command_id,
                attachment_epoch,
                media_generation: generation,
                attempt_id,
                property: MPV_PROPERTY_TIME_POS,
            });
            adapter.complete_transport_readback(command_id + 10, None);
            assert!(adapter.transport_readback.pending.is_some());
            adapter.complete_transport_readback(command_id, Some((json!({"data":42.0}), ingress)));
            assert_eq!(
                adapter.observed_state.position_seconds == Some(42.0),
                expected
            );
        }
        let batch = adapter.take_player_event_batch().unwrap();
        let timestamp = batch
            .events
            .iter()
            .find_map(|event| match &event.event {
                PlayerEvent::TransportDelta(delta) if delta.position_seconds == Some(42.0) => {
                    delta.observed_at
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(
            timestamp.elapsed_since_adapter_start(),
            Duration::from_secs(1)
        );
        assert!(timestamp.delivery_reference_since_adapter_start() >= Duration::from_secs(21));
    }
}
