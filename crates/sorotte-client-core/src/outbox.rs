use std::{cell::Cell, collections::VecDeque};

use sorotte_protocol::{ProtocolMessage, SOROTTE_PLAYBACK_BARRIER_V1, StatePayload};

fn merge_playback_barrier_observations(previous: &StatePayload, latest: &mut StatePayload) {
    let Ok(Some(previous_extension)) = previous.playback_barrier_v1() else {
        return;
    };
    let Ok(Some(latest_extension)) = latest.playback_barrier_v1() else {
        return;
    };
    let Some(latest_object) = latest
        .extra
        .get_mut(SOROTTE_PLAYBACK_BARRIER_V1)
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };

    if latest_extension.ready.is_none()
        && let Some(ready) = previous_extension.ready
    {
        latest_object.insert(
            "ready".to_owned(),
            serde_json::to_value(ready).expect("playback barrier readiness must serialize to JSON"),
        );
    }
    if latest_extension.started.is_none()
        && let Some(started) = previous_extension.started
    {
        latest_object.insert(
            "started".to_owned(),
            serde_json::to_value(started)
                .expect("playback barrier started acknowledgement must serialize to JSON"),
        );
    }
    if latest_extension.transport.is_none()
        && let Some(transport) = previous_extension.transport
    {
        latest_object.insert(
            "transport".to_owned(),
            serde_json::to_value(transport)
                .expect("playback barrier transport observation must serialize to JSON"),
        );
    }
}

fn merge_pending_state_obligations(previous: &StatePayload, latest: &mut StatePayload) {
    if let Some(previous_playstate) = previous.playstate.as_ref() {
        if let Some(latest_playstate) = latest.playstate.as_mut() {
            latest_playstate.position = latest_playstate.position.or(previous_playstate.position);
            latest_playstate.paused = latest_playstate.paused.or(previous_playstate.paused);
            latest_playstate.do_seek = if previous_playstate.do_seek == Some(true) {
                Some(true)
            } else {
                latest_playstate.do_seek.or(previous_playstate.do_seek)
            };
            if latest_playstate.set_by.is_none() {
                latest_playstate
                    .set_by
                    .clone_from(&previous_playstate.set_by);
            }
            merge_missing_entries(&previous_playstate.extra, &mut latest_playstate.extra);
        } else {
            latest.playstate = Some(previous_playstate.clone());
        }
    }

    if let Some(previous_ping) = previous.ping.as_ref() {
        if let Some(latest_ping) = latest.ping.as_mut() {
            latest_ping.latency_calculation = latest_ping
                .latency_calculation
                .or(previous_ping.latency_calculation);
            latest_ping.client_latency_calculation = latest_ping
                .client_latency_calculation
                .or(previous_ping.client_latency_calculation);
            latest_ping.client_rtt = latest_ping.client_rtt.or(previous_ping.client_rtt);
            latest_ping.server_rtt = latest_ping.server_rtt.or(previous_ping.server_rtt);
            merge_missing_entries(&previous_ping.extra, &mut latest_ping.extra);
        } else {
            latest.ping = Some(previous_ping.clone());
        }
    }

    if let Some(previous_ignore) = previous.ignoring_on_the_fly.as_ref() {
        if let Some(latest_ignore) = latest.ignoring_on_the_fly.as_mut() {
            latest_ignore.server = latest_ignore.server.or(previous_ignore.server);
            latest_ignore.client = latest_ignore.client.or(previous_ignore.client);
            merge_missing_entries(&previous_ignore.extra, &mut latest_ignore.extra);
        } else {
            latest.ignoring_on_the_fly = Some(previous_ignore.clone());
        }
    }

    merge_playback_barrier_observations(previous, latest);
    merge_missing_entries(&previous.extra, &mut latest.extra);
}

fn merge_missing_entries(
    previous: &std::collections::BTreeMap<String, serde_json::Value>,
    latest: &mut std::collections::BTreeMap<String, serde_json::Value>,
) {
    for (key, value) in previous {
        latest.entry(key.clone()).or_insert_with(|| value.clone());
    }
}

/// FIFO boundary between pure runtime state and fallible effect adapters.
///
/// Fallible adapters should use [`Self::try_flush`], which acknowledges an
/// effect only after its delivery succeeds. [`Self::drain`] is reserved for
/// explicit best-effort or otherwise infallible handoffs.
#[derive(Debug, Clone)]
pub(crate) struct EffectOutbox<T> {
    pending: VecDeque<T>,
}

impl<T> Default for EffectOutbox<T> {
    fn default() -> Self {
        Self {
            pending: VecDeque::new(),
        }
    }
}

impl<T> EffectOutbox<T> {
    pub(crate) fn pending(&self) -> &VecDeque<T> {
        &self.pending
    }

    pub(crate) fn front(&self) -> Option<&T> {
        self.pending.front()
    }

    pub(crate) fn back_mut(&mut self) -> Option<&mut T> {
        self.pending.back_mut()
    }

    pub(crate) fn push_back(&mut self, effect: T) {
        self.pending.push_back(effect);
    }

    pub(crate) fn acknowledge_front(&mut self) -> Option<T> {
        self.pending.pop_front()
    }

    pub(crate) fn drain(&mut self) -> Vec<T> {
        self.pending.drain(..).collect()
    }

    pub(crate) fn try_flush<E>(
        &mut self,
        mut deliver: impl FnMut(&T) -> Result<(), E>,
    ) -> Result<(), E> {
        while let Some(effect) = self.pending.front() {
            deliver(effect)?;
            self.pending.pop_front();
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProtocolDelivery {
    Reliable,
    ConnectionScopedReliable {
        connection_generation: u64,
        room: String,
        local_media_generation: u64,
        request_nonce: u64,
        cancelled: bool,
    },
    ConnectionScopedState {
        generation: u64,
    },
}

/// Opaque identity for one staged outbox front.
///
/// A receipt is accepted only while this exact lease still owns the front;
/// connection replacement invalidates it so a late receipt cannot acknowledge
/// a different durable message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProtocolLineLease(u64);

/// Outbound protocol ownership split by delivery semantics.
///
/// Durable reliable commands remain in FIFO order until acknowledged.
/// Playback-barrier Set requests are reliable only within their connection,
/// room, and local-media scope. Playback State is scoped to one connection
/// generation and coalesces to the latest pending value. A State returned by
/// [`Self::front_for_delivery`] is leased until it is acknowledged; a newer
/// State may wait behind that lease without changing the bytes owned by an
/// asynchronous transport.
#[derive(Debug)]
pub(crate) struct ProtocolOutbox {
    pending: VecDeque<ProtocolMessage>,
    delivery: VecDeque<ProtocolDelivery>,
    connection_generation: u64,
    active_generation: Option<u64>,
    next_lease: Cell<u64>,
    leased_front: Cell<Option<ProtocolLineLease>>,
}

impl Default for ProtocolOutbox {
    fn default() -> Self {
        Self {
            pending: VecDeque::new(),
            delivery: VecDeque::new(),
            connection_generation: 0,
            active_generation: None,
            next_lease: Cell::new(0),
            leased_front: Cell::new(None),
        }
    }
}

impl ProtocolOutbox {
    pub(crate) fn pending(&self) -> &VecDeque<ProtocolMessage> {
        &self.pending
    }

    pub(crate) fn front_for_delivery(&self) -> Option<(ProtocolLineLease, &ProtocolMessage)> {
        let message = self.pending.front()?;
        let lease = self.leased_front.get().unwrap_or_else(|| {
            let next = self.next_lease.get().wrapping_add(1).max(1);
            self.next_lease.set(next);
            let lease = ProtocolLineLease(next);
            self.leased_front.set(Some(lease));
            lease
        });
        Some((lease, message))
    }

    pub(crate) fn push_back(&mut self, message: ProtocolMessage) {
        if matches!(message, ProtocolMessage::State(_)) {
            let _ = self.push_connection_scoped_state(message);
            return;
        }

        self.insert_reliable(message, ProtocolDelivery::Reliable);
    }

    fn insert_reliable(&mut self, message: ProtocolMessage, delivery: ProtocolDelivery) {
        // Keep reliable commands ahead of an unleased coalescible State. A
        // leased State is already owned by the transport and cannot be moved.
        let insert_at = self
            .delivery
            .iter()
            .enumerate()
            .find_map(|(index, delivery)| {
                matches!(
                    delivery,
                    ProtocolDelivery::ConnectionScopedState { generation }
                        if *generation == self.connection_generation
                            && !(index == 0 && self.leased_front.get().is_some())
                )
                .then_some(index)
            })
            .unwrap_or(self.pending.len());
        self.pending.insert(insert_at, message);
        self.delivery.insert(insert_at, delivery);
    }

    pub(crate) fn push_connection_scoped_reliable(
        &mut self,
        message: ProtocolMessage,
        room: String,
        local_media_generation: u64,
        request_nonce: u64,
    ) -> bool {
        if self.active_generation != Some(self.connection_generation) {
            return false;
        }

        // A newly serialized playback lifecycle request supersedes every older
        // one. Durable chat, playlist, and other ordinary commands are kept.
        self.cancel_connection_scoped_reliable();
        self.insert_reliable(
            message,
            ProtocolDelivery::ConnectionScopedReliable {
                connection_generation: self.connection_generation,
                room,
                local_media_generation,
                request_nonce,
                cancelled: false,
            },
        );
        true
    }

    pub(crate) fn retain_connection_scoped_reliable_scope(
        &mut self,
        room: &str,
        local_media_generation: u64,
    ) {
        let connection_generation = self.connection_generation;
        self.invalidate_connection_scoped_reliable(|delivery| {
            matches!(
                delivery,
                ProtocolDelivery::ConnectionScopedReliable {
                    connection_generation: pending_connection_generation,
                    room: pending_room,
                    local_media_generation: pending_generation,
                    request_nonce,
                    ..
                } if *pending_connection_generation != connection_generation
                    || pending_room != room
                    || *pending_generation != local_media_generation
                    || *request_nonce == 0
            )
        });
    }

    pub(crate) fn cancel_connection_scoped_reliable(&mut self) {
        self.invalidate_connection_scoped_reliable(|delivery| {
            matches!(delivery, ProtocolDelivery::ConnectionScopedReliable { .. })
        });
    }

    fn invalidate_connection_scoped_reliable(
        &mut self,
        should_cancel: impl Fn(&ProtocolDelivery) -> bool,
    ) {
        if self.leased_front.get().is_some()
            && self.delivery.front().is_some_and(&should_cancel)
            && let Some(ProtocolDelivery::ConnectionScopedReliable { cancelled, .. }) =
                self.delivery.front_mut()
        {
            *cancelled = true;
        }
        self.retain_deliveries(true, |delivery| !should_cancel(delivery));
    }

    fn retain_deliveries(
        &mut self,
        preserve_leased_front: bool,
        mut keep: impl FnMut(&ProtocolDelivery) -> bool,
    ) {
        let mut retained_messages = VecDeque::with_capacity(self.pending.len());
        let mut retained_delivery = VecDeque::with_capacity(self.delivery.len());
        let mut index = 0;
        while let (Some(message), Some(delivery)) =
            (self.pending.pop_front(), self.delivery.pop_front())
        {
            if (preserve_leased_front && index == 0 && self.leased_front.get().is_some())
                || keep(&delivery)
            {
                retained_messages.push_back(message);
                retained_delivery.push_back(delivery);
            }
            index += 1;
        }
        self.pending = retained_messages;
        self.delivery = retained_delivery;
    }

    pub(crate) fn push_connection_scoped_state(&mut self, mut message: ProtocolMessage) -> bool {
        if self.active_generation != Some(self.connection_generation) {
            return false;
        }

        let replace_at = self
            .delivery
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, delivery)| {
                matches!(
                    delivery,
                    ProtocolDelivery::ConnectionScopedState { generation }
                        if *generation == self.connection_generation
                            && !(index == 0 && self.leased_front.get().is_some())
                )
                .then_some(index)
            });
        if let Some(index) = replace_at {
            if let (ProtocolMessage::State(previous), ProtocolMessage::State(latest)) =
                (&self.pending[index], &mut message)
            {
                merge_pending_state_obligations(&previous.state, &mut latest.state);
            }
            self.pending[index] = message;
        } else {
            self.pending.push_back(message);
            self.delivery
                .push_back(ProtocolDelivery::ConnectionScopedState {
                    generation: self.connection_generation,
                });
        }
        true
    }

    pub(crate) fn begin_connection_generation(&mut self) {
        self.connection_generation = self.connection_generation.wrapping_add(1);
        self.active_generation = None;
        self.leased_front.set(None);

        self.retain_deliveries(false, |delivery| {
            matches!(delivery, ProtocolDelivery::Reliable)
        });
    }

    pub(crate) fn activate_connection_generation(&mut self) {
        self.active_generation = Some(self.connection_generation);
    }

    pub(crate) fn acknowledge_front(
        &mut self,
        lease: ProtocolLineLease,
    ) -> Option<ProtocolMessage> {
        if self.leased_front.get() != Some(lease) {
            return None;
        }
        let message = self.pending.pop_front()?;
        self.delivery
            .pop_front()
            .expect("protocol outbox delivery metadata should match pending messages");
        self.leased_front.set(None);
        Some(message)
    }

    pub(crate) fn release_front(&mut self, lease: ProtocolLineLease) -> bool {
        if self.leased_front.get() != Some(lease) {
            return false;
        }
        let discard_cancelled = matches!(
            self.delivery.front(),
            Some(ProtocolDelivery::ConnectionScopedReliable {
                cancelled: true,
                ..
            })
        );
        self.leased_front.set(None);
        if discard_cancelled {
            self.pending.pop_front();
            self.delivery.pop_front();
        }
        true
    }

    pub(crate) fn clear(&mut self) {
        self.pending.clear();
        self.delivery.clear();
        self.leased_front.set(None);
    }

    pub(crate) fn drain(&mut self) -> Vec<ProtocolMessage> {
        self.delivery.clear();
        self.leased_front.set(None);
        self.pending.drain(..).collect()
    }
}

#[cfg(test)]
mod tests {
    use sorotte_protocol::{
        IgnoringOnTheFlyPayload, MediaReadyPayload, PingPayload, PlaybackBarrierSetExtension,
        PlaybackBarrierStateExtension, PlaystatePayload, ProtocolMessage, RoomBufferingPolicy,
        RoomBufferingPolicyPayload, SetPayload, StartedAckPayload, StatePayload,
        TransportBufferingReportPayload, playlist_change_with_plex_sidecar,
    };

    use super::{EffectOutbox, ProtocolOutbox};

    #[test]
    fn failed_delivery_preserves_failed_effect_and_tail() {
        let mut outbox = EffectOutbox::default();
        outbox.push_back("first");
        outbox.push_back("second");
        outbox.push_back("third");
        let mut attempted = Vec::new();

        let result = outbox.try_flush(|effect| {
            attempted.push(*effect);
            if *effect == "second" {
                Err("delivery failed")
            } else {
                Ok(())
            }
        });

        assert_eq!(result, Err("delivery failed"));
        assert_eq!(attempted, vec!["first", "second"]);
        assert_eq!(outbox.drain(), vec!["second", "third"]);
    }

    #[test]
    fn coalesced_state_preserves_one_shot_obligations_behind_reliable_front() {
        let mut outbox = ProtocolOutbox::default();
        outbox.activate_connection_generation();
        outbox.push_back(ProtocolMessage::chat_text("reliable front"));

        let mut obligations = StatePayload::new()
            .with_playstate(
                PlaystatePayload::new()
                    .with_position(10.0)
                    .with_paused(true)
                    .with_do_seek(true),
            )
            .with_ping(
                PingPayload::new()
                    .with_latency_calculation(42.0)
                    .with_client_latency_calculation(1.0)
                    .with_client_rtt(0.1),
            )
            .with_ignoring_on_the_fly(IgnoringOnTheFlyPayload::new().with_server(7));
        obligations
            .extra
            .insert("oldExtension".to_owned(), serde_json::json!(true));
        assert!(outbox.push_connection_scoped_state(ProtocolMessage::state(obligations)));

        let mut newest = StatePayload::new()
            .with_playstate(
                PlaystatePayload::new()
                    .with_position(30.0)
                    .with_paused(false)
                    .with_do_seek(false),
            )
            .with_ping(
                PingPayload::new()
                    .with_client_latency_calculation(3.0)
                    .with_client_rtt(0.3),
            )
            .with_ignoring_on_the_fly(IgnoringOnTheFlyPayload::new().with_client(9));
        newest
            .extra
            .insert("newExtension".to_owned(), serde_json::json!(true));
        assert!(outbox.push_connection_scoped_state(ProtocolMessage::state(newest)));

        assert_eq!(outbox.pending().len(), 2);
        assert!(matches!(
            outbox.pending().front(),
            Some(ProtocolMessage::Chat(_))
        ));
        let ProtocolMessage::State(state) = &outbox.pending()[1] else {
            panic!("coalesced message behind reliable front should be State");
        };
        let playstate = state
            .state
            .playstate
            .as_ref()
            .expect("newest playstate should remain present");
        assert_eq!(playstate.position, Some(30.0));
        assert_eq!(playstate.paused, Some(false));
        assert_eq!(
            playstate.do_seek,
            Some(true),
            "an undelivered seek obligation must survive a heartbeat"
        );
        let ping = state
            .state
            .ping
            .as_ref()
            .expect("newest ping telemetry should remain present");
        assert_eq!(ping.client_latency_calculation, Some(3.0));
        assert_eq!(ping.client_rtt, Some(0.3));
        assert_eq!(
            ping.latency_calculation,
            Some(42.0),
            "the server latency challenge must survive until delivery"
        );
        let ignore = state
            .state
            .ignoring_on_the_fly
            .as_ref()
            .expect("ignore counters should remain present");
        assert_eq!(ignore.server, Some(7));
        assert_eq!(ignore.client, Some(9));
        assert_eq!(
            state.state.extra.get("oldExtension"),
            Some(&serde_json::json!(true))
        );
        assert_eq!(
            state.state.extra.get("newExtension"),
            Some(&serde_json::json!(true))
        );
    }

    #[test]
    fn coalesced_state_merges_playback_barrier_observations_by_subfield() {
        let mut outbox = ProtocolOutbox::default();
        outbox.activate_connection_generation();

        assert!(
            outbox.push_connection_scoped_state(ProtocolMessage::state(
                StatePayload::new().with_playback_barrier_v1(
                    PlaybackBarrierStateExtension::new()
                        .with_ready(MediaReadyPayload::new(17, true, true)),
                ),
            ))
        );
        assert!(outbox.push_connection_scoped_state(ProtocolMessage::state(
            StatePayload::new().with_playback_barrier_v1(
                PlaybackBarrierStateExtension::new().with_transport(
                    TransportBufferingReportPayload::new(17, true).with_buffered_seconds(0.0),
                ),
            ),
        )));
        assert!(
            outbox.push_connection_scoped_state(ProtocolMessage::state(
                StatePayload::new().with_playback_barrier_v1(
                    PlaybackBarrierStateExtension::new()
                        .with_started(StartedAckPayload::new(17, 4, 12.0)),
                ),
            ))
        );
        assert!(
            outbox.push_connection_scoped_state(ProtocolMessage::state(
                StatePayload::new().with_playback_barrier_v1(
                    PlaybackBarrierStateExtension::new()
                        .with_ready(MediaReadyPayload::new(17, false, false)),
                ),
            ))
        );
        assert!(
            outbox.push_connection_scoped_state(ProtocolMessage::state(
                StatePayload::new().with_playback_barrier_v1(
                    PlaybackBarrierStateExtension::new()
                        .with_started(StartedAckPayload::new(17, 4, 13.0)),
                ),
            ))
        );
        assert!(outbox.push_connection_scoped_state(ProtocolMessage::state(
            StatePayload::new().with_playback_barrier_v1(
                PlaybackBarrierStateExtension::new().with_transport(
                    TransportBufferingReportPayload::new(17, false).with_buffered_seconds(8.0),
                ),
            ),
        )));

        assert_eq!(outbox.pending().len(), 1);
        let ProtocolMessage::State(state) = &outbox.pending()[0] else {
            panic!("coalesced playback barrier observation should be State");
        };
        let extension = state
            .state
            .playback_barrier_v1()
            .expect("coalesced playback barrier extension should decode")
            .expect("coalesced State should retain the playback barrier extension");
        let ready = extension
            .ready
            .expect("a later transport report must not overwrite readiness");
        assert_eq!(ready.media_generation, 17);
        assert!(!ready.loaded, "the latest readiness value must win");
        assert!(!ready.buffer_ready, "the latest readiness value must win");
        let started = extension
            .started
            .expect("a later transport report must not overwrite StartedAck");
        assert_eq!((started.media_generation, started.state_revision), (17, 4));
        assert_eq!(
            started.observed_position, 13.0,
            "the latest StartedAck value must win"
        );
        let transport = extension
            .transport
            .expect("the newest transport report should remain present");
        assert!(!transport.buffering, "the latest subfield value must win");
        assert_eq!(transport.buffered_seconds, Some(8.0));
    }

    fn scoped_barrier_request(request_nonce: u64) -> ProtocolMessage {
        ProtocolMessage::set(
            SetPayload::new().with_playback_barrier_v1(
                PlaybackBarrierSetExtension::new().with_buffering_policy(
                    RoomBufferingPolicyPayload::new(0, RoomBufferingPolicy::PauseController)
                        .with_request_nonce(request_nonce),
                ),
            ),
        )
    }

    fn outbox_with_scoped_barrier_and_durable_tail() -> ProtocolOutbox {
        let mut outbox = ProtocolOutbox::default();
        outbox.activate_connection_generation();
        assert!(outbox.push_connection_scoped_reliable(
            scoped_barrier_request(7),
            "room-one".to_owned(),
            31,
            7,
        ));
        outbox.push_back(ProtocolMessage::chat_text("durable chat"));
        outbox.push_back(ProtocolMessage::set(
            SetPayload::new().with_playlist_change(playlist_change_with_plex_sidecar(
                vec!["episode.mkv".to_owned()],
                true,
            )),
        ));
        outbox
    }

    fn assert_leased_barrier_acknowledges_before_durable_tail(mut outbox: ProtocolOutbox) {
        let (lease, staged) = outbox.front_for_delivery().expect("barrier should stage");
        assert!(matches!(staged, ProtocolMessage::Set(_)));
        let (same_lease, _) = outbox
            .front_for_delivery()
            .expect("staged barrier should remain visible");
        assert_eq!(same_lease, lease);

        let acknowledged = outbox
            .acknowledge_front(lease)
            .expect("the exact staged barrier lease should acknowledge");
        let ProtocolMessage::Set(set) = acknowledged else {
            panic!("the leased barrier, not its durable tail, must be acknowledged");
        };
        assert!(
            set.set
                .playback_barrier_v1()
                .expect("barrier extension should decode")
                .is_some()
        );
        assert!(matches!(outbox.pending()[0], ProtocolMessage::Chat(_)));
        let ProtocolMessage::Set(playlist) = &outbox.pending()[1] else {
            panic!("playlist command should remain behind chat");
        };
        assert!(playlist.set.playlist_change.is_some());
    }

    #[test]
    fn leased_scoped_barrier_survives_explicit_cancellation_until_exact_acknowledgement() {
        let mut outbox = outbox_with_scoped_barrier_and_durable_tail();
        let _ = outbox.front_for_delivery().expect("barrier should stage");
        outbox.cancel_connection_scoped_reliable();
        assert_leased_barrier_acknowledges_before_durable_tail(outbox);
    }

    #[test]
    fn released_cancelled_barrier_is_discarded_without_touching_durable_tail() {
        let mut outbox = outbox_with_scoped_barrier_and_durable_tail();
        let (lease, _) = outbox.front_for_delivery().expect("barrier should stage");
        outbox.cancel_connection_scoped_reliable();

        assert!(outbox.release_front(lease));
        assert_eq!(outbox.pending().len(), 2);
        assert!(matches!(outbox.pending()[0], ProtocolMessage::Chat(_)));
        let ProtocolMessage::Set(playlist) = &outbox.pending()[1] else {
            panic!("playlist command should remain after cancelled barrier release");
        };
        assert!(playlist.set.playlist_change.is_some());
    }

    #[test]
    fn leased_scoped_barrier_survives_media_supersession_until_exact_acknowledgement() {
        let mut outbox = outbox_with_scoped_barrier_and_durable_tail();
        let _ = outbox.front_for_delivery().expect("barrier should stage");
        outbox.retain_connection_scoped_reliable_scope("room-one", 32);
        assert_leased_barrier_acknowledges_before_durable_tail(outbox);
    }

    #[test]
    fn leased_scoped_barrier_survives_room_change_until_exact_acknowledgement() {
        let mut outbox = outbox_with_scoped_barrier_and_durable_tail();
        let _ = outbox.front_for_delivery().expect("barrier should stage");
        outbox.retain_connection_scoped_reliable_scope("room-two", 31);
        assert_leased_barrier_acknowledges_before_durable_tail(outbox);
    }

    #[test]
    fn connection_generation_invalidates_old_lease_without_acknowledging_durable_tail() {
        let mut outbox = outbox_with_scoped_barrier_and_durable_tail();
        let (old_lease, _) = outbox
            .front_for_delivery()
            .expect("old barrier should stage");

        outbox.begin_connection_generation();
        assert_eq!(outbox.pending().len(), 2);
        assert!(matches!(outbox.pending()[0], ProtocolMessage::Chat(_)));
        assert_eq!(
            outbox.acknowledge_front(old_lease),
            None,
            "a stale transport receipt must not pop the next durable command"
        );

        outbox.activate_connection_generation();
        let (chat_lease, staged) = outbox
            .front_for_delivery()
            .expect("durable chat should stage on the new generation");
        assert!(matches!(staged, ProtocolMessage::Chat(_)));
        assert_eq!(outbox.acknowledge_front(old_lease), None);
        assert!(matches!(
            outbox.acknowledge_front(chat_lease),
            Some(ProtocolMessage::Chat(_))
        ));
        let ProtocolMessage::Set(playlist) = &outbox.pending()[0] else {
            panic!("playlist command should remain after chat acknowledgement");
        };
        assert!(playlist.set.playlist_change.is_some());
    }
}
