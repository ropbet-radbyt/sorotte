use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Clone, Default)]
pub(in crate::app) struct GuiQueuedSessionTransportHandle {
    queued_inbound_protocol_lines: Arc<Mutex<VecDeque<String>>>,
    queued_outbound_protocol_lines: Arc<Mutex<VecDeque<String>>>,
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
        self.queued_outbound_protocol_lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push_back(line.into());
    }

    pub(in crate::app) fn outbound_protocol_activity_revision(&self) -> u64 {
        self.queued_outbound_protocol_activity_revision
            .load(Ordering::Relaxed)
    }

    pub(in crate::app) fn drain_outbound_protocol_lines(&self) -> Vec<String> {
        let mut queue = self
            .queued_outbound_protocol_lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        queue.drain(..).collect()
    }

    pub(in crate::app) fn clear_protocol_lines(&self) {
        self.queued_inbound_protocol_lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        self.queued_outbound_protocol_lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }
}

pub(in crate::app) trait GuiSessionTransportDriver: Send {
    fn pump(&mut self, transport: &GuiQueuedSessionTransportHandle) -> Result<(), String>;

    fn set_protocol_liveness_enabled(&mut self, _enabled: bool) {}

    fn reconnect(&mut self) -> Result<(), String> {
        Err("Session transport driver does not support reconnect.".to_owned())
    }
}
