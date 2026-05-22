use super::*;

impl LegacyServerPythonPeerHarness {
    pub fn spawn_connected(peer_username: &str, room: &str) -> Result<Self, InteropError> {
        let mut harness = Self::spawn(peer_username, room)?;
        if let Err(error) = harness.start_peer_connected() {
            let _ = harness.shutdown();
            return Err(error);
        }
        Ok(harness)
    }

    pub fn spawn(peer_username: &str, room: &str) -> Result<Self, InteropError> {
        Self::spawn_server(peer_username, room)
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn address(&self) -> &str {
        &self.address
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn room(&self) -> &str {
        &self.room
    }

    pub fn peer_username(&self) -> &str {
        &self.peer_username
    }
}
