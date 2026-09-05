use std::{path::Path, time::Instant};

use crate::ipc::MpvJsonIpcClient;

use super::{IPC_RECONNECT_INTERVAL, MpvAdapter};

impl MpvAdapter {
    /// Creates a disconnected adapter that retains an explicit JSON-IPC
    /// endpoint and retries it from the normal nonblocking maintenance pump.
    /// This is intended for runtime owners that must keep Sorotte membership
    /// alive while mpv is temporarily absent.
    pub fn disconnected_with_json_ipc_retry(path: impl AsRef<Path>) -> Self {
        Self {
            ipc_endpoint: Some(path.as_ref().to_path_buf()),
            ipc_reconnect_not_before: Some(Instant::now()),
            ..Self::default()
        }
    }

    pub(super) fn maintain_json_ipc_reconnection_using<F>(&mut self, now: Instant, connect: F)
    where
        F: FnOnce(&Path) -> Result<MpvJsonIpcClient, String>,
    {
        self.maintain_json_ipc_reconnection_using_clock(now, connect, Instant::now);
    }

    pub(super) fn maintain_json_ipc_reconnection_using_clock<F, C>(
        &mut self,
        attempt_started_at: Instant,
        connect: F,
        completed_at: C,
    ) where
        F: FnOnce(&Path) -> Result<MpvJsonIpcClient, String>,
        C: FnOnce() -> Instant,
    {
        if self.simulation_mode || self.is_connected() {
            self.ipc_reconnect_not_before = None;
            return;
        }
        let Some(endpoint) = self.ipc_endpoint.clone() else {
            return;
        };
        if self
            .ipc_reconnect_not_before
            .is_some_and(|not_before| attempt_started_at < not_before)
        {
            return;
        }
        let connected = connect(&endpoint);
        let Ok(client) = connected else {
            let retry_from = completed_at().max(attempt_started_at);
            self.ipc_reconnect_not_before = Some(retry_from + IPC_RECONNECT_INTERVAL);
            return;
        };
        if self
            .initialize_json_ipc_attachment(endpoint, client)
            .is_err()
        {
            // Connecting and the fallible version/attachment initialization are
            // one attempt. A slow version response must not consume its backoff.
            let retry_from = completed_at().max(attempt_started_at);
            self.ipc_reconnect_not_before = Some(retry_from + IPC_RECONNECT_INTERVAL);
        }
    }
}
