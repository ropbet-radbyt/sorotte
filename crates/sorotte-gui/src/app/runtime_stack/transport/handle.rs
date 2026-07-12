use std::{
    collections::VecDeque,
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Clone, PartialEq, Eq)]
pub(in crate::app) struct GuiOutboundProtocolDelivery {
    pub(in crate::app) token: u64,
    pub(in crate::app) line: String,
}

impl GuiOutboundProtocolDelivery {
    pub(in crate::app) fn new(token: u64, line: impl Into<String>) -> Self {
        Self {
            token,
            line: line.into(),
        }
    }

    pub(in crate::app) fn token(&self) -> u64 {
        self.token
    }

    #[allow(
        dead_code,
        reason = "Delivery content inspection is used by deterministic transport tests."
    )]
    pub(in crate::app) fn line(&self) -> &str {
        &self.line
    }
}

impl fmt::Debug for GuiOutboundProtocolDelivery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuiOutboundProtocolDelivery")
            .field("token", &self.token)
            .field("line_bytes", &self.line.len())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(in crate::app) enum GuiOutboundProtocolDeliveryResult {
    FrameWritten {
        token: u64,
    },
    FrameFailed {
        token: u64,
        bytes_written: usize,
        message: String,
    },
}

impl GuiOutboundProtocolDeliveryResult {
    fn token(&self) -> u64 {
        match self {
            Self::FrameWritten { token } | Self::FrameFailed { token, .. } => *token,
        }
    }
}

impl fmt::Debug for GuiOutboundProtocolDeliveryResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FrameWritten { token } => formatter
                .debug_struct("FrameWritten")
                .field("token", token)
                .finish(),
            Self::FrameFailed {
                token,
                bytes_written,
                message,
            } => formatter
                .debug_struct("FrameFailed")
                .field("token", token)
                .field("bytes_written", bytes_written)
                .field("message_bytes", &message.len())
                .finish(),
        }
    }
}

#[derive(Default)]
struct GuiTrackedOutboundProtocolDeliveryState {
    pending: Option<GuiTrackedOutboundProtocolDelivery>,
}

struct GuiTrackedOutboundProtocolDelivery {
    token: u64,
    queued_line: Option<String>,
    result: Option<GuiOutboundProtocolDeliveryResult>,
}

#[derive(Clone, Default)]
pub(in crate::app) struct GuiQueuedSessionTransportHandle {
    queued_inbound_protocol_lines: Arc<Mutex<VecDeque<String>>>,
    queued_outbound_protocol_lines: Arc<Mutex<VecDeque<String>>>,
    queued_outbound_liveness_protocol_line: Arc<Mutex<Option<String>>>,
    tracked_outbound_protocol_delivery: Arc<Mutex<GuiTrackedOutboundProtocolDeliveryState>>,
    queued_outbound_protocol_activity_revision: Arc<AtomicU64>,
}

impl GuiQueuedSessionTransportHandle {
    pub(in crate::app) fn push_inbound_protocol_line(&self, line: impl Into<String>) {
        self.push_inbound_protocol_lines([line.into()]);
    }

    pub(in crate::app) fn push_inbound_protocol_lines<I>(&self, lines: I)
    where
        I: IntoIterator<Item = String>,
    {
        let mut queue = self
            .queued_inbound_protocol_lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        queue.extend(lines);
    }

    pub(in crate::app) fn drain_inbound_protocol_lines(&self) -> Vec<String> {
        let mut queue = self
            .queued_inbound_protocol_lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        queue.drain(..).collect()
    }

    pub(in crate::app) fn clear_inbound_protocol_lines(&self) {
        self.queued_inbound_protocol_lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }

    pub(in crate::app) fn try_push_outbound_protocol_delivery(
        &self,
        delivery: GuiOutboundProtocolDelivery,
    ) -> Result<(), GuiOutboundProtocolDelivery> {
        let mut state = self
            .tracked_outbound_protocol_delivery
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.pending.is_some() {
            return Err(delivery);
        }
        state.pending = Some(GuiTrackedOutboundProtocolDelivery {
            token: delivery.token,
            queued_line: Some(delivery.line),
            result: None,
        });
        self.queued_outbound_protocol_activity_revision
            .fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    pub(in crate::app) fn drain_outbound_protocol_delivery_results(
        &self,
    ) -> Vec<GuiOutboundProtocolDeliveryResult> {
        let mut state = self
            .tracked_outbound_protocol_delivery
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(result) = state
            .pending
            .as_mut()
            .and_then(|pending| pending.result.take())
        else {
            return Vec::new();
        };
        state.pending = None;
        vec![result]
    }

    pub(in crate::app) fn take_outbound_protocol_delivery_for_driver(
        &self,
    ) -> Option<GuiOutboundProtocolDelivery> {
        let mut state = self
            .tracked_outbound_protocol_delivery
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let pending = state.pending.as_mut()?;
        let line = pending.queued_line.take()?;
        Some(GuiOutboundProtocolDelivery {
            token: pending.token,
            line,
        })
    }

    pub(in crate::app) fn publish_outbound_protocol_delivery_result(
        &self,
        result: GuiOutboundProtocolDeliveryResult,
    ) {
        let mut state = self
            .tracked_outbound_protocol_delivery
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(pending) = state.pending.as_mut() else {
            return;
        };
        if pending.token != result.token() || pending.result.is_some() {
            return;
        }
        pending.queued_line = None;
        pending.result = Some(result);
    }

    pub(in crate::app) fn fail_pending_outbound_protocol_delivery(
        &self,
        bytes_written: usize,
        message: impl Into<String>,
    ) {
        let mut state = self
            .tracked_outbound_protocol_delivery
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(pending) = state.pending.as_mut() else {
            return;
        };
        if pending.result.is_some() {
            return;
        }
        pending.queued_line = None;
        pending.result = Some(GuiOutboundProtocolDeliveryResult::FrameFailed {
            token: pending.token,
            bytes_written,
            message: message.into(),
        });
    }

    #[allow(
        dead_code,
        reason = "Untracked protocol injection is retained for transport compatibility tests."
    )]
    pub(in crate::app) fn push_outbound_protocol_lines<I>(&self, lines: I)
    where
        I: IntoIterator<Item = String>,
    {
        let mut queue = self
            .queued_outbound_protocol_lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut pushed = false;
        for line in lines {
            queue.push_back(line);
            pushed = true;
        }
        if pushed {
            self.queued_outbound_protocol_activity_revision
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(in crate::app) fn push_outbound_liveness_protocol_line(&self, line: impl Into<String>) {
        *self
            .queued_outbound_liveness_protocol_line
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(line.into());
    }

    pub(in crate::app) fn take_outbound_liveness_protocol_line_for_driver(&self) -> Option<String> {
        self.queued_outbound_liveness_protocol_line
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }

    pub(in crate::app) fn clear_outbound_liveness_protocol_line(&self) {
        *self
            .queued_outbound_liveness_protocol_line
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }

    pub(in crate::app) fn drain_untracked_outbound_protocol_lines_for_driver(&self) -> Vec<String> {
        self.queued_outbound_protocol_lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .drain(..)
            .collect()
    }

    pub(in crate::app) fn outbound_protocol_activity_revision(&self) -> u64 {
        self.queued_outbound_protocol_activity_revision
            .load(Ordering::Relaxed)
    }

    pub(in crate::app) fn drain_outbound_protocol_lines(&self) -> Vec<String> {
        let tracked_delivery = self.take_outbound_protocol_delivery_for_driver();
        let mut lines = Vec::new();
        if let Some(delivery) = tracked_delivery {
            lines.push(delivery.line);
            self.publish_outbound_protocol_delivery_result(
                GuiOutboundProtocolDeliveryResult::FrameWritten {
                    token: delivery.token,
                },
            );
        }
        lines.extend(self.drain_untracked_outbound_protocol_lines_for_driver());
        if let Some(liveness_line) = self.take_outbound_liveness_protocol_line_for_driver() {
            lines.push(liveness_line);
        }
        lines
    }

    pub(in crate::app) fn clear_protocol_lines(&self) {
        self.clear_inbound_protocol_lines();
        self.queued_outbound_protocol_lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        self.clear_outbound_liveness_protocol_line();
        self.fail_pending_outbound_protocol_delivery(
            0,
            "Outbound protocol delivery was discarded while closing the session transport.",
        );
    }
}

pub(in crate::app) trait GuiSessionTransportDriver: Send {
    fn pump(&mut self, transport: &GuiQueuedSessionTransportHandle) -> Result<(), String>;

    fn set_protocol_liveness_enabled(&mut self, _enabled: bool) {}

    fn reconnect(&mut self) -> Result<(), String> {
        Err("Session transport driver does not support reconnect.".to_owned())
    }
}
