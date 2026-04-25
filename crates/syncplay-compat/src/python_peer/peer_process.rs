use super::*;

impl LegacyServerPythonPeerHarness {
    pub(super) fn spawn_peer_process(&mut self) -> Result<(), InteropError> {
        let legacy_checkout = ensure_legacy_syncplay_checkout_available()?;

        let live_peer_probe = python_live_peer_probe_script_path();
        if !live_peer_probe.is_file() {
            return Err(InteropError::PythonLivePeerProbeMissing(live_peer_probe));
        }

        let python_bin = python_bin_from_env();
        let python_bin_display = python_bin.to_string_lossy().to_string();
        let mut peer_command = Command::new(&python_bin);
        peer_command
            .arg(&live_peer_probe)
            .arg("--host")
            .arg(&self.host)
            .arg("--port")
            .arg(self.port.to_string())
            .arg("--name")
            .arg(&self.peer_username)
            .arg("--room")
            .arg(&self.room)
            .arg("--timeout-seconds")
            .arg("3")
            .current_dir(&legacy_checkout)
            .env("PYTHONUNBUFFERED", "1")
            .env("SYNCPLAY_LEGACY_ROOT", &legacy_checkout)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut peer_child = peer_command
            .spawn()
            .map_err(|source| InteropError::PythonSpawn {
                python: python_bin_display,
                source,
            })?;

        let peer_stdin = peer_child
            .stdin
            .take()
            .ok_or(InteropError::PythonStdinMissing)?;
        let peer_stdout = peer_child
            .stdout
            .take()
            .ok_or(InteropError::EmptyPythonResponse)?;
        let peer_stderr = peer_child
            .stderr
            .take()
            .ok_or(InteropError::EmptyPythonResponse)?;
        let (peer_status_tx, peer_status_rx) = mpsc::channel();
        let peer_stdout_lines = Arc::new(Mutex::new(Vec::new()));
        let peer_stderr_lines = Arc::new(Mutex::new(Vec::new()));
        capture_process_output_lines(peer_stdout, peer_stdout_lines.clone(), Some(peer_status_tx));
        capture_process_output_lines(peer_stderr, peer_stderr_lines.clone(), None);

        self.peer_child = Some(peer_child);
        self.peer_stdin = Some(peer_stdin);
        self.peer_status_rx = Some(peer_status_rx);
        self.peer_stdout_lines = peer_stdout_lines;
        self.peer_stderr_lines = peer_stderr_lines;
        Ok(())
    }

    pub(super) fn wait_for_peer_connected(
        &mut self,
        timeout: Duration,
    ) -> Result<(), InteropError> {
        self.wait_for_peer_status(timeout, "connected").map(|_| ())
    }

    pub(super) fn ensure_peer_connected(&mut self) -> Result<(), InteropError> {
        if self.peer_child.is_none() {
            self.spawn_peer_process()?;
            self.wait_for_peer_connected(Duration::from_secs(3))?;
        }
        Ok(())
    }

    pub(super) fn stop_peer_process(&mut self) -> Result<(), InteropError> {
        let mut errors = Vec::new();
        if let Some(stdin) = self.peer_stdin.take() {
            drop(stdin);
        }
        if let Some(mut peer_child) = self.peer_child.take() {
            match wait_for_child_exit_with_timeout(&mut peer_child, Duration::from_secs(1)) {
                Ok(true) => {}
                Ok(false) => {
                    if let Err(error) = peer_child.kill() {
                        errors.push(format!(
                            "failed to terminate python live peer process: {error}"
                        ));
                    }
                    if let Err(error) = peer_child.wait() {
                        errors.push(format!(
                            "failed to wait for python live peer process exit after kill: {error}"
                        ));
                    }
                }
                Err(error) => errors.push(format!(
                    "failed to wait for python live peer process exit before kill: {error}"
                )),
            }
        }
        self.peer_status_rx = None;
        self.peer_stdout_lines = Arc::new(Mutex::new(Vec::new()));
        self.peer_stderr_lines = Arc::new(Mutex::new(Vec::new()));
        if errors.is_empty() {
            Ok(())
        } else {
            Err(InteropError::InvalidPythonBatchResponse(errors.join("; ")))
        }
    }

    pub(super) fn send_peer_command(&mut self, command: &Value) -> Result<(), InteropError> {
        let stdin = self
            .peer_stdin
            .as_mut()
            .ok_or(InteropError::PythonStdinMissing)?;
        let mut payload = serde_json::to_vec(command)?;
        payload.push(b'\n');
        stdin
            .write_all(&payload)
            .map_err(InteropError::PythonStdinWrite)?;
        stdin.flush().map_err(InteropError::PythonStdinWrite)?;
        Ok(())
    }

    pub(super) fn wait_for_peer_status(
        &mut self,
        timeout: Duration,
        expected_status: &str,
    ) -> Result<Value, InteropError> {
        let Some(peer_status_rx) = self.peer_status_rx.as_ref() else {
            return Err(InteropError::InvalidPythonBatchResponse(
                "python live peer process has not been started".to_owned(),
            ));
        };
        let Some(peer_child) = self.peer_child.as_mut() else {
            return Err(InteropError::InvalidPythonBatchResponse(
                "python live peer child handle is missing".to_owned(),
            ));
        };

        let status_line = match peer_status_rx.recv_timeout(timeout) {
            Ok(status_line) => status_line,
            Err(mpsc::RecvTimeoutError::Timeout) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                let stdout = captured_process_output(&self.peer_stdout_lines);
                let stderr = captured_process_output(&self.peer_stderr_lines);
                return match peer_child.try_wait()? {
                    Some(status) => Err(InteropError::InvalidPythonBatchResponse(format!(
                        "python live peer exited before reporting status {expected_status:?} (exit code: {:?}, stdout: '{stdout}', stderr: '{stderr}')",
                        status.code()
                    ))),
                    None => Err(InteropError::InvalidPythonBatchResponse(format!(
                        "python live peer timed out waiting for status {expected_status:?} (stdout: '{stdout}', stderr: '{stderr}')"
                    ))),
                };
            }
        };

        let parsed = self.parse_peer_status_line(&status_line)?;
        let Some(actual_status) = parsed.get("status").and_then(Value::as_str) else {
            return Err(InteropError::InvalidPythonBatchResponse(format!(
                "python live peer status line did not include a status field: {status_line:?}"
            )));
        };
        if actual_status != expected_status {
            return Err(InteropError::InvalidPythonBatchResponse(format!(
                "python live peer reported unexpected status {actual_status:?}; expected {expected_status:?}: {status_line:?}"
            )));
        }
        Ok(parsed)
    }
}
