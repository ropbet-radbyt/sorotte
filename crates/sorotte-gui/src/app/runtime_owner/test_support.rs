use std::sync::{Arc, Mutex};

use super::GuiPersistedConfigRuntimeOwner;
use crate::app::runtime_stack::{GuiQueuedSessionTransportHandle, GuiSessionTransportDriver};

#[derive(Clone)]
pub(in crate::app) struct RecordedSessionTransport {
    transport: GuiQueuedSessionTransportHandle,
    written: Arc<Mutex<Vec<String>>>,
}

impl std::ops::Deref for RecordedSessionTransport {
    type Target = GuiQueuedSessionTransportHandle;

    fn deref(&self) -> &Self::Target {
        &self.transport
    }
}

impl RecordedSessionTransport {
    /// Inspect completed writes without changing the live transport's state.
    pub(in crate::app) fn take_written_lines(&self) -> Vec<String> {
        std::mem::take(&mut *self.written.lock().expect("write capture should lock"))
    }
}

struct RecordingSessionTransportDriver {
    written: Arc<Mutex<Vec<String>>>,
}

impl GuiSessionTransportDriver for RecordingSessionTransportDriver {
    fn pump(&mut self, transport: &GuiQueuedSessionTransportHandle) -> Result<(), String> {
        self.written
            .lock()
            .expect("write capture should lock")
            .extend(transport.complete_outbound_protocol_write());
        Ok(())
    }
}

impl GuiPersistedConfigRuntimeOwner {
    pub(in crate::app) fn with_recording_chat_session_runtime(
        self,
        username: impl Into<String>,
        room: impl Into<String>,
    ) -> Result<(Self, RecordedSessionTransport), String> {
        let (owner, transport) = self.with_client_core_chat_session_runtime(username, room)?;
        let written = Arc::default();
        let owner =
            owner.with_session_transport_driver(Box::new(RecordingSessionTransportDriver {
                written: Arc::clone(&written),
            }));
        Ok((owner, RecordedSessionTransport { transport, written }))
    }
}
