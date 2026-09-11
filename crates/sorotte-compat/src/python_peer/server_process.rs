use super::*;

impl SyncplayServerPythonPeerHarness {
    pub(super) fn spawn_server(peer_username: &str, room: &str) -> Result<Self, InteropError> {
        let legacy_checkout = ensure_syncplay_checkout_available()?;

        let syncplay_server_entry = syncplay_server_entry_script_path();
        if !syncplay_server_entry.is_file() {
            return Err(InteropError::SyncplayServerEntryScriptMissing(
                syncplay_server_entry,
            ));
        }

        let mut port_lease = reserve_syncplay_server_port()?;
        let port = port_lease.port();
        let python_bin = python_bin_from_env();
        let python_bin_display = python_bin.to_string_lossy().to_string();

        let mut server_command = Command::new(&python_bin);
        server_command
            .arg(&syncplay_server_entry)
            .arg("--port")
            .arg(port.to_string())
            .arg("--ipv4-only")
            .arg("--interface-ipv4")
            .arg("127.0.0.1")
            .arg("--salt")
            .arg(DEFAULT_SYNCPLAY_SERVER_CONTROLLED_ROOM_SALT)
            .current_dir(&legacy_checkout)
            .env("PYTHONUNBUFFERED", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        port_lease.release_socket_for_child();
        let mut server_child =
            server_command
                .spawn()
                .map_err(|source| InteropError::PythonSpawn {
                    python: python_bin_display.clone(),
                    source,
                })?;

        if let Err(error) = wait_for_syncplay_server_startup(port, &mut server_child) {
            terminate_syncplay_server_process(&mut server_child);
            return Err(error);
        }
        drop(port_lease);
        if let Err(error) = ensure_syncplay_server_is_running(&mut server_child) {
            terminate_syncplay_server_process(&mut server_child);
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
            next_peer_request_id: 1,
        })
    }
}
