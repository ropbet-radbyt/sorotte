//! Explicit player ingress for runtime tests. Scripts supply physical facts;
//! this queue supplies only delivery order and acknowledgement bookkeeping.

use std::collections::VecDeque;

use crate::{
    PlayerAttachmentEpoch, PlayerAuthoritativeSnapshot, PlayerError, PlayerEvent,
    PlayerEventAcknowledgementToken, PlayerEventBatch, PlayerEventOrder, PlayerSemanticOutcome,
    PlayerSequenceBoundary, SequencedPlayerEvent, SequencedPlayerSemanticOutcome,
};

#[derive(Debug)]
pub struct ScriptedPlayerEvents {
    epoch: PlayerAttachmentEpoch,
    next_sequence: u64,
    next_token: u64,
    pending: VecDeque<PlayerEventBatch>,
}

impl ScriptedPlayerEvents {
    pub fn new(epoch: PlayerAttachmentEpoch) -> Self {
        Self {
            epoch,
            next_sequence: 1,
            next_token: 1,
            pending: VecDeque::new(),
        }
    }

    pub fn push_event(&mut self, event: PlayerEvent) {
        let order = self.next_order();
        let mut batch = self.empty_batch(order.sequence);
        batch.events.push(SequencedPlayerEvent { order, event });
        self.pending.push_back(batch);
    }

    pub fn push_outcome(&mut self, outcome: PlayerSemanticOutcome) {
        let order = self.next_order();
        let mut batch = self.empty_batch(order.sequence);
        batch
            .semantic_outcomes
            .push(SequencedPlayerSemanticOutcome { order, outcome });
        self.pending.push_back(batch);
    }

    pub fn push_snapshot(&mut self, snapshot: PlayerAuthoritativeSnapshot) {
        assert_eq!(snapshot.attachment_epoch, self.epoch);
        assert_eq!(snapshot.sequence_boundary.attachment_epoch, self.epoch);
        let boundary = snapshot.sequence_boundary.through_sequence;
        self.next_sequence = self.next_sequence.max(boundary.checked_add(1).unwrap());
        let mut batch = self.empty_batch(boundary);
        batch.authoritative_snapshot = Some(snapshot);
        self.pending.push_back(batch);
    }

    pub fn peek(&self) -> Option<PlayerEventBatch> {
        self.pending.front().cloned()
    }

    pub fn acknowledge(
        &mut self,
        token: PlayerEventAcknowledgementToken,
    ) -> Result<(), PlayerError> {
        if self
            .pending
            .front()
            .is_none_or(|batch| batch.acknowledgement_token != token)
        {
            return Err(PlayerError::OperationFailed(
                "scripted batch acknowledgement mismatch".to_owned(),
            ));
        }
        self.pending.pop_front();
        Ok(())
    }

    fn next_order(&mut self) -> PlayerEventOrder {
        let sequence = self.next_sequence;
        self.next_sequence = sequence.checked_add(1).expect("script sequence exhausted");
        PlayerEventOrder::new(self.epoch, sequence)
    }

    fn empty_batch(&mut self, through_sequence: u64) -> PlayerEventBatch {
        let token = self.next_token;
        self.next_token = token.checked_add(1).expect("script token exhausted");
        PlayerEventBatch {
            attachment_epoch: self.epoch,
            sequence_boundary: PlayerSequenceBoundary::new(self.epoch, through_sequence),
            authoritative_snapshot: None,
            events: Vec::new(),
            semantic_outcomes: Vec::new(),
            acknowledgement_token: PlayerEventAcknowledgementToken::new(self.epoch, token),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PlayerCommandId, PlayerCommandOutcome, PlayerCommandSemanticResult};

    #[test]
    fn failed_acknowledgement_and_new_ingress_preserve_the_in_flight_batch() {
        let epoch = PlayerAttachmentEpoch::new(7);
        let mut events = ScriptedPlayerEvents::new(epoch);
        events.push_outcome(PlayerSemanticOutcome::Command(PlayerCommandOutcome {
            attachment_epoch: epoch,
            command_id: PlayerCommandId::new(2),
            media_generation: None,
            result: PlayerCommandSemanticResult::CompletionNotObserved,
        }));
        let first = events.peek().unwrap();
        events.push_event(PlayerEvent::EventGapDetected);
        assert!(
            events
                .acknowledge(PlayerEventAcknowledgementToken::new(epoch, 99))
                .is_err()
        );
        assert_eq!(events.peek(), Some(first.clone()));
        events.acknowledge(first.acknowledgement_token).unwrap();
        let second = events.peek().unwrap();
        assert_eq!(second.events[0].order.sequence, 2);
        assert_eq!(second.events[0].event, PlayerEvent::EventGapDetected);
        events.acknowledge(second.acknowledgement_token).unwrap();
        assert!(events.peek().is_none());
    }
}
