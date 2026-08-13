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
        if self.simulation_mode || self.is_connected() {
            self.ipc_reconnect_not_before = None;
            return;
        }
        let Some(endpoint) = self.ipc_endpoint.clone() else {
            return;
        };
        if self
            .ipc_reconnect_not_before
            .is_some_and(|not_before| now < not_before)
        {
            return;
        }
        self.ipc_reconnect_not_before = Some(now + IPC_RECONNECT_INTERVAL);
        let Ok(client) = connect(&endpoint) else {
            return;
        };
        if self
            .initialize_json_ipc_attachment(endpoint, client)
            .is_err()
        {
            self.ipc_reconnect_not_before = Some(now + IPC_RECONNECT_INTERVAL);
        }
    }
}
