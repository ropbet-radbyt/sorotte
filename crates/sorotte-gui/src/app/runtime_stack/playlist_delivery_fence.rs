use std::collections::VecDeque;

/// The exact reliable playlist frames that must receive terminal write
/// acknowledgements before a dependent local-player side effect may run.
///
/// Unrelated status, chat, or list traffic is intentionally absent. That
/// keeps the frontier stable when a coalescible tail is cancelled and means
/// traffic queued after the mutation cannot extend the wait.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(in crate::app) struct GuiPlaylistProtocolDeliveryFence {
    pending_lines: VecDeque<String>,
}

impl GuiPlaylistProtocolDeliveryFence {
    pub(in crate::app) fn new(pending_lines: impl IntoIterator<Item = String>) -> Self {
        Self {
            pending_lines: pending_lines.into_iter().collect(),
        }
    }

    pub(in crate::app) fn note_frame_written(&mut self, line: &str) {
        if self
            .pending_lines
            .front()
            .is_some_and(|expected| expected == line)
        {
            self.pending_lines.pop_front();
        }
    }

    pub(in crate::app) fn is_reached(&self) -> bool {
        self.pending_lines.is_empty()
    }

    #[cfg(test)]
    pub(in crate::app) fn pending_frame_count(&self) -> usize {
        self.pending_lines.len()
    }
}
