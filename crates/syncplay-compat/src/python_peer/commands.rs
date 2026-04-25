use super::*;

impl LegacyServerPythonPeerHarness {
    pub fn start_peer_connected(&mut self) -> Result<(), InteropError> {
        if self.peer_child.is_none() {
            self.spawn_peer_process()?;
        }
        self.wait_for_peer_connected(Duration::from_secs(3))
    }

    pub fn set_peer_ready(&mut self, ready: bool) -> Result<(), InteropError> {
        self.ensure_peer_connected()?;
        self.send_peer_command(&json!({
            "command": "set_ready",
            "ready": ready,
        }))?;
        self.wait_for_peer_status(Duration::from_secs(3), "ready-command-sent")?;
        Ok(())
    }

    pub fn wait_for_peer_local_ready(
        &mut self,
        ready: bool,
        timeout: Duration,
    ) -> Result<LegacyPythonPeerSnapshot, InteropError> {
        self.ensure_peer_connected()?;
        self.send_peer_command(&json!({
            "command": "wait_for_local_ready",
            "ready": ready,
            "timeoutSeconds": timeout.as_secs_f64(),
        }))?;
        let status = self.wait_for_peer_status(timeout, "local-ready")?;
        Self::parse_peer_snapshot(&status)
    }

    pub fn wait_for_peer_observed_user_ready(
        &mut self,
        username: &str,
        ready: bool,
        timeout: Duration,
    ) -> Result<LegacyPythonPeerSnapshot, InteropError> {
        self.ensure_peer_connected()?;
        self.send_peer_command(&json!({
            "command": "wait_for_user_ready",
            "username": username,
            "ready": ready,
            "timeoutSeconds": timeout.as_secs_f64(),
        }))?;
        let status = self.wait_for_peer_status(timeout, "user-ready")?;
        Self::parse_peer_snapshot(&status)
    }

    pub fn wait_for_peer_local_controller(
        &mut self,
        controller: bool,
        timeout: Duration,
    ) -> Result<LegacyPythonPeerSnapshot, InteropError> {
        self.ensure_peer_connected()?;
        self.send_peer_command(&json!({
            "command": "wait_for_local_controller",
            "controller": controller,
            "timeoutSeconds": timeout.as_secs_f64(),
        }))?;
        let status = self.wait_for_peer_status(timeout, "local-controller")?;
        Self::parse_peer_snapshot(&status)
    }

    pub fn wait_for_peer_observed_user_controller(
        &mut self,
        username: &str,
        controller: bool,
        timeout: Duration,
    ) -> Result<LegacyPythonPeerSnapshot, InteropError> {
        self.ensure_peer_connected()?;
        self.send_peer_command(&json!({
            "command": "wait_for_user_controller",
            "username": username,
            "controller": controller,
            "timeoutSeconds": timeout.as_secs_f64(),
        }))?;
        let status = self.wait_for_peer_status(timeout, "user-controller")?;
        Self::parse_peer_snapshot(&status)
    }

    pub fn send_peer_chat_message(
        &mut self,
        message: &str,
    ) -> Result<LegacyPythonPeerSnapshot, InteropError> {
        self.ensure_peer_connected()?;
        self.send_peer_command(&json!({
            "command": "send_chat_message",
            "message": message,
        }))?;
        let status = self.wait_for_peer_status(Duration::from_secs(3), "chat-command-sent")?;
        Self::parse_peer_snapshot(&status)
    }

    pub fn wait_for_peer_observed_chat_message(
        &mut self,
        username: &str,
        message: &str,
        timeout: Duration,
    ) -> Result<LegacyPythonPeerSnapshot, InteropError> {
        self.ensure_peer_connected()?;
        self.send_peer_command(&json!({
            "command": "wait_for_chat_message",
            "username": username,
            "message": message,
            "timeoutSeconds": timeout.as_secs_f64(),
        }))?;
        let status = self.wait_for_peer_status(timeout, "chat-message")?;
        Self::parse_peer_snapshot(&status)
    }

    pub fn wait_for_peer_observed_user_file_name(
        &mut self,
        username: &str,
        file_name: &str,
        timeout: Duration,
    ) -> Result<LegacyPythonPeerSnapshot, InteropError> {
        self.ensure_peer_connected()?;
        self.send_peer_command(&json!({
            "command": "wait_for_user_file_name",
            "username": username,
            "fileName": file_name,
            "timeoutSeconds": timeout.as_secs_f64(),
        }))?;
        let status = self.wait_for_peer_status(timeout, "user-file")?;
        Self::parse_peer_snapshot(&status)
    }

    pub fn set_peer_playlist(
        &mut self,
        files: &[String],
    ) -> Result<LegacyPythonPeerSnapshot, InteropError> {
        self.ensure_peer_connected()?;
        self.send_peer_command(&json!({
            "command": "set_playlist",
            "files": files,
        }))?;
        let status = self.wait_for_peer_status(Duration::from_secs(3), "playlist-command-sent")?;
        Self::parse_peer_snapshot(&status)
    }

    pub fn set_peer_playlist_index(
        &mut self,
        index: usize,
    ) -> Result<LegacyPythonPeerSnapshot, InteropError> {
        self.ensure_peer_connected()?;
        self.send_peer_command(&json!({
            "command": "set_playlist_index",
            "index": index,
        }))?;
        let status =
            self.wait_for_peer_status(Duration::from_secs(3), "playlist-index-command-sent")?;
        Self::parse_peer_snapshot(&status)
    }

    pub fn wait_for_peer_playlist(
        &mut self,
        files: &[String],
        timeout: Duration,
    ) -> Result<LegacyPythonPeerSnapshot, InteropError> {
        self.ensure_peer_connected()?;
        self.send_peer_command(&json!({
            "command": "wait_for_playlist",
            "files": files,
            "timeoutSeconds": timeout.as_secs_f64(),
        }))?;
        let status = self.wait_for_peer_status(timeout, "playlist")?;
        Self::parse_peer_snapshot(&status)
    }

    pub fn wait_for_peer_playlist_index(
        &mut self,
        index: usize,
        timeout: Duration,
    ) -> Result<LegacyPythonPeerSnapshot, InteropError> {
        self.ensure_peer_connected()?;
        self.send_peer_command(&json!({
            "command": "wait_for_playlist_index",
            "index": index,
            "timeoutSeconds": timeout.as_secs_f64(),
        }))?;
        let status = self.wait_for_peer_status(timeout, "playlist-index")?;
        Self::parse_peer_snapshot(&status)
    }

    pub fn peer_snapshot(&mut self) -> Result<LegacyPythonPeerSnapshot, InteropError> {
        self.ensure_peer_connected()?;
        self.send_peer_command(&json!({
            "command": "snapshot",
        }))?;
        let status = self.wait_for_peer_status(Duration::from_secs(3), "snapshot")?;
        Self::parse_peer_snapshot(&status)
    }

    pub fn disconnect_peer(&mut self) -> Result<(), InteropError> {
        self.stop_peer_process()
    }

    pub fn shutdown(mut self) -> Result<(), InteropError> {
        let mut errors = Vec::new();
        if let Err(error) = self.stop_peer_process() {
            errors.push(error.to_string());
        }
        terminate_legacy_server_process(&mut self.server_child);
        if errors.is_empty() {
            Ok(())
        } else {
            Err(InteropError::InvalidPythonBatchResponse(errors.join("; ")))
        }
    }
}
