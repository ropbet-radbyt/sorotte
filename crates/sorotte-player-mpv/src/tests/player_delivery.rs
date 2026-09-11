//! Inspection of actual acknowledged adapter deliveries. No observations are synthesized.
use sorotte_player_api::{
    LoadAttemptOutcome, LocalFileUpdate, PlayerAdapter, PlayerCommandOutcome, PlayerEvent,
    PlayerEventBatch, PlayerSemanticOutcome, PlayerTransportDelta, PlayerTransportSnapshot,
};

#[derive(Debug, Default)]
pub(crate) struct PlayerDelivery {
    pub(crate) batches: Vec<PlayerEventBatch>,
}

pub(crate) fn collect_player_delivery(player: &mut impl PlayerAdapter) -> PlayerDelivery {
    let mut delivery = PlayerDelivery::default();
    while let Some(batch) = player.take_player_event_batch() {
        player
            .acknowledge_player_event_batch(batch.acknowledgement_token)
            .expect("acknowledge the batch that was actually received");
        delivery.batches.push(batch);
        assert!(
            delivery.batches.len() < 1024,
            "adapter failed to drain acknowledged deliveries"
        );
    }
    delivery
}

impl PlayerDelivery {
    pub(crate) fn events(&self) -> impl Iterator<Item = &PlayerEvent> {
        self.batches
            .iter()
            .flat_map(|batch| &batch.events)
            .map(|item| &item.event)
    }

    pub(crate) fn transport_deltas(&self) -> impl Iterator<Item = &PlayerTransportDelta> {
        self.events().filter_map(|event| match event {
            PlayerEvent::TransportDelta(delta) => Some(delta),
            _ => None,
        })
    }

    pub(crate) fn command_outcomes(&self) -> impl Iterator<Item = &PlayerCommandOutcome> {
        self.batches
            .iter()
            .flat_map(|batch| &batch.semantic_outcomes)
            .filter_map(|item| match &item.outcome {
                PlayerSemanticOutcome::Command(outcome) => Some(outcome),
                _ => None,
            })
    }

    pub(crate) fn load_outcomes(&self) -> impl Iterator<Item = &LoadAttemptOutcome> {
        self.batches
            .iter()
            .flat_map(|batch| &batch.semantic_outcomes)
            .filter_map(|item| match &item.outcome {
                PlayerSemanticOutcome::LoadAttempt(outcome) => Some(outcome),
                _ => None,
            })
    }

    pub(crate) fn local_files(&self) -> impl Iterator<Item = &LocalFileUpdate> {
        self.events().filter_map(|event| match event {
            PlayerEvent::LocalFileChanged { update, .. } => Some(update),
            _ => None,
        })
    }

    pub(crate) fn transport_snapshot(&self) -> PlayerTransportSnapshot {
        let mut transport = PlayerTransportSnapshot::default();
        for batch in &self.batches {
            if let Some(snapshot) = &batch.authoritative_snapshot {
                transport.rebase(snapshot.transport.clone());
            }
            for item in &batch.events {
                if let PlayerEvent::TransportDelta(delta) = &item.event {
                    transport.apply_delta(delta.clone());
                }
            }
        }
        transport
    }
}
