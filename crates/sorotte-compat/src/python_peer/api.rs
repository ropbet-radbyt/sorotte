use super::*;

impl SyncplayServerPythonPeerHarness {
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
