use super::*;

impl LegacyServerPythonPeerHarness {
    pub(super) fn spawn_server(peer_username: &str, room: &str) -> Result<Self, InteropError> {
        let legacy_checkout = ensure_legacy_syncplay_checkout_available()?;

        let legacy_server_entry = legacy_syncplay_server_entry_script_path();
        if !legacy_server_entry.is_file() {
            return Err(InteropError::LegacyServerEntryScriptMissing(
                legacy_server_entry,
            ));
        }

        let port = reserve_ephemeral_tcp_port()?;
        let python_bin = python_bin_from_env();
        let python_bin_display = python_bin.to_string_lossy().to_string();

        let mut server_command = Command::new(&python_bin);
        server_command
            .arg(&legacy_server_entry)
            .arg("--port")
            .arg(port.to_string())
            .arg("--ipv4-only")
            .arg("--interface-ipv4")
            .arg("127.0.0.1")
            .arg("--salt")
            .arg(DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT)
            .current_dir(&legacy_checkout)
            .env("PYTHONUNBUFFERED", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut server_child =
            server_command
                .spawn()
                .map_err(|source| InteropError::PythonSpawn {
                    python: python_bin_display.clone(),
                    source,
                })?;

        if let Err(error) = wait_for_legacy_server_startup(port, &mut server_child) {
            terminate_legacy_server_process(&mut server_child);
            return Err(error);
        }
        if let Err(error) = ensure_legacy_server_is_running(&mut server_child) {
            terminate_legacy_server_process(&mut server_child);
            return Err(error);
        }

        let host = "127.0.0.1".to_owned();
        let address = format!("{host}:{port}");
        Ok(Self {
            host,
            address,
            port,
            room: room.to_owned(),
            peer_username: peer_username.to_owned(),
            server_child,
            peer_child: None,
            peer_stdin: None,
            peer_status_rx: None,
            peer_stdout_lines: Arc::new(Mutex::new(Vec::new())),
            peer_stderr_lines: Arc::new(Mutex::new(Vec::new())),
        })
    }
}
