use std::{cell::Cell, collections::VecDeque};

use sorotte_protocol::{ProtocolMessage, StatePayload};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProtocolDelivery {
    Reliable,
    ConnectionScopedState { generation: u64 },
}

/// Outbound protocol ownership split by delivery semantics.
///
/// Reliable commands remain in FIFO order until acknowledged. Playback State
/// is scoped to one connection generation and coalesces to the latest pending
/// value. A State returned by [`Self::front_for_delivery`] is leased until it is
/// acknowledged; a newer State may wait behind that lease without changing the
/// bytes owned by an asynchronous transport.
#[derive(Debug)]
pub(crate) struct ProtocolOutbox {
    pending: VecDeque<ProtocolMessage>,
    delivery: VecDeque<ProtocolDelivery>,
    connection_generation: u64,
    active_generation: Option<u64>,
    leased_front_state: Cell<bool>,
}

impl Default for ProtocolOutbox {
    fn default() -> Self {
        Self {
            pending: VecDeque::new(),
            delivery: VecDeque::new(),
            connection_generation: 0,
            active_generation: None,
            leased_front_state: Cell::new(false),
        }
    }
}

impl ProtocolOutbox {
    pub(crate) fn pending(&self) -> &VecDeque<ProtocolMessage> {
        &self.pending
    }

    pub(crate) fn front_for_delivery(&self) -> Option<&ProtocolMessage> {
        let message = self.pending.front()?;
        if matches!(
            self.delivery.front(),
            Some(ProtocolDelivery::ConnectionScopedState { generation })
                if self.active_generation == Some(*generation)
        ) {
            self.leased_front_state.set(true);
        }
        Some(message)
    }

    pub(crate) fn push_back(&mut self, message: ProtocolMessage) {
        if matches!(message, ProtocolMessage::State(_)) {
            let _ = self.push_connection_scoped_state(message);
            return;
        }

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
                            && !(index == 0 && self.leased_front_state.get())
                )
                .then_some(index)
            })
            .unwrap_or(self.pending.len());
        self.pending.insert(insert_at, message);
        self.delivery.insert(insert_at, ProtocolDelivery::Reliable);
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
                            && !(index == 0 && self.leased_front_state.get())
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
        self.leased_front_state.set(false);

        let mut reliable = VecDeque::new();
        while let (Some(message), Some(delivery)) =
            (self.pending.pop_front(), self.delivery.pop_front())
        {
            if delivery == ProtocolDelivery::Reliable {
                reliable.push_back(message);
            }
        }
        self.pending = reliable;
        self.delivery =
            std::iter::repeat_n(ProtocolDelivery::Reliable, self.pending.len()).collect();
    }

    pub(crate) fn activate_connection_generation(&mut self) {
        self.active_generation = Some(self.connection_generation);
    }

    pub(crate) fn acknowledge_front(&mut self) -> Option<ProtocolMessage> {
        let message = self.pending.pop_front()?;
        let delivery = self
            .delivery
            .pop_front()
            .expect("protocol outbox delivery metadata should match pending messages");
        if matches!(delivery, ProtocolDelivery::ConnectionScopedState { .. }) {
            self.leased_front_state.set(false);
        }
        Some(message)
    }

    pub(crate) fn clear(&mut self) {
        self.pending.clear();
        self.delivery.clear();
        self.leased_front_state.set(false);
    }

    pub(crate) fn drain(&mut self) -> Vec<ProtocolMessage> {
        self.delivery.clear();
        self.leased_front_state.set(false);
        self.pending.drain(..).collect()
    }

    pub(crate) fn try_flush<E>(
        &mut self,
        mut deliver: impl FnMut(&ProtocolMessage) -> Result<(), E>,
    ) -> Result<(), E> {
        while let Some(message) = self.pending.front() {
            deliver(message)?;
            self.pending.pop_front();
            self.delivery.pop_front();
            self.leased_front_state.set(false);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use sorotte_protocol::{
        IgnoringOnTheFlyPayload, PingPayload, PlaystatePayload, ProtocolMessage, StatePayload,
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
}
