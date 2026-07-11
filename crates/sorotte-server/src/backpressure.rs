use std::sync::{
    Arc,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ServerOutboundBackpressureSnapshot {
    pub queue_depth: usize,
    pub peak_queue_depth: usize,
    pub full_queue_events: u64,
    pub dropped_messages: u64,
    pub coalesced_state_updates: u64,
    pub closed_channel_events: u64,
    pub overload_disconnects: u64,
}

#[derive(Debug, Default)]
struct ServerOutboundBackpressureMetricsInner {
    queue_depth: AtomicUsize,
    peak_queue_depth: AtomicUsize,
    full_queue_events: AtomicU64,
    dropped_messages: AtomicU64,
    coalesced_state_updates: AtomicU64,
    closed_channel_events: AtomicU64,
    overload_disconnects: AtomicU64,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ServerOutboundBackpressureMetrics {
    inner: Arc<ServerOutboundBackpressureMetricsInner>,
}

impl ServerOutboundBackpressureMetrics {
    pub(crate) fn enqueued(&self) {
        let depth = self.inner.queue_depth.fetch_add(1, Ordering::AcqRel) + 1;
        let _ =
            self.inner
                .peak_queue_depth
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |peak| {
                    (depth > peak).then_some(depth)
                });
    }

    pub(crate) fn dequeued(&self) {
        let _ = self
            .inner
            .queue_depth
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |depth| {
                Some(depth.saturating_sub(1))
            });
    }

    pub(crate) fn discarded(&self, count: usize) {
        if count == 0 {
            return;
        }
        self.inner
            .dropped_messages
            .fetch_add(count as u64, Ordering::Relaxed);
        let _ = self
            .inner
            .queue_depth
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |depth| {
                Some(depth.saturating_sub(count))
            });
    }

    pub(crate) fn full(&self) {
        self.inner.full_queue_events.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn dropped(&self) {
        self.inner.dropped_messages.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn coalesced(&self) {
        self.inner
            .coalesced_state_updates
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn closed(&self) {
        self.inner
            .closed_channel_events
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn overload_disconnect(&self) {
        self.inner
            .overload_disconnects
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn snapshot(&self) -> ServerOutboundBackpressureSnapshot {
        ServerOutboundBackpressureSnapshot {
            queue_depth: self.inner.queue_depth.load(Ordering::Acquire),
            peak_queue_depth: self.inner.peak_queue_depth.load(Ordering::Acquire),
            full_queue_events: self.inner.full_queue_events.load(Ordering::Relaxed),
            dropped_messages: self.inner.dropped_messages.load(Ordering::Relaxed),
            coalesced_state_updates: self.inner.coalesced_state_updates.load(Ordering::Relaxed),
            closed_channel_events: self.inner.closed_channel_events.load(Ordering::Relaxed),
            overload_disconnects: self.inner.overload_disconnects.load(Ordering::Relaxed),
        }
    }
}
