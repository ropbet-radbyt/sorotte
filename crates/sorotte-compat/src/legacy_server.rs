use super::*;

pub fn run_legacy_server_fanout_roundtrip(
    steps: &[ServerRuntimeScenarioStep],
) -> Result<Vec<ServerRuntimeScenarioEvent>, InteropError> {
    run_legacy_server_fanout_roundtrip_with_salt(steps, DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT)
}

pub fn run_legacy_server_fanout_roundtrip_with_salt(
    steps: &[ServerRuntimeScenarioStep],
    controlled_room_salt: &str,
) -> Result<Vec<ServerRuntimeScenarioEvent>, InteropError> {
    run_legacy_server_fanout_roundtrip_with_salt_and_motd_template(
        steps,
        controlled_room_salt,
        None,
    )
}

pub fn run_legacy_server_fanout_roundtrip_with_salt_and_motd_template(
    steps: &[ServerRuntimeScenarioStep],
    controlled_room_salt: &str,
    motd_template: Option<&str>,
) -> Result<Vec<ServerRuntimeScenarioEvent>, InteropError> {
    run_legacy_server_fanout_roundtrip_with_overrides(
        steps,
        controlled_room_salt,
        motd_template,
        false,
    )
}

pub fn run_legacy_server_fanout_roundtrip_with_overrides(
    steps: &[ServerRuntimeScenarioStep],
    controlled_room_salt: &str,
    motd_template: Option<&str>,
    persistent_rooms_enabled: bool,
) -> Result<Vec<ServerRuntimeScenarioEvent>, InteropError> {
    run_legacy_server_fanout_roundtrip_with_full_overrides(
        steps,
        controlled_room_salt,
        motd_template,
        persistent_rooms_enabled,
        &[],
    )
}

pub(crate) fn run_legacy_server_fanout_roundtrip_with_full_overrides(
    steps: &[ServerRuntimeScenarioStep],
    controlled_room_salt: &str,
    motd_template: Option<&str>,
    persistent_rooms_enabled: bool,
    permanent_rooms: &[&str],
) -> Result<Vec<ServerRuntimeScenarioEvent>, InteropError> {
    if steps.is_empty() {
        return Ok(Vec::new());
    }

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
    let motd_template_file_path = motd_template
        .map(str::trim)
        .filter(|template| !template.is_empty())
        .map(write_legacy_motd_template_file)
        .transpose()?;
    let persistent_rooms_db_path = if persistent_rooms_enabled {
        Some(create_temporary_legacy_rooms_db_file_path()?)
    } else {
        None
    };
    let permanent_rooms_file_path = if permanent_rooms.is_empty() {
        None
    } else {
        Some(create_temporary_legacy_permanent_rooms_file_path(
            permanent_rooms,
        )?)
    };
    let mut command = Command::new(&python_bin);
    command
        .arg(&legacy_server_entry)
        .arg("--port")
        .arg(port.to_string())
        .arg("--ipv4-only")
        .arg("--interface-ipv4")
        .arg("127.0.0.1")
        .arg("--salt")
        .arg(controlled_room_salt)
        .current_dir(legacy_checkout)
        .env("PYTHONUNBUFFERED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(motd_file_path) = motd_template_file_path.as_ref() {
        command.arg("--motd-file").arg(motd_file_path);
    }
    if let Some(rooms_db_path) = persistent_rooms_db_path.as_ref() {
        command.arg("--rooms-db-file").arg(rooms_db_path);
    }
    if let Some(permanent_rooms_path) = permanent_rooms_file_path.as_ref() {
        command
            .arg("--permanent-rooms-file")
            .arg(permanent_rooms_path);
    }
    let child_spawn = command.spawn();
    let mut child = match child_spawn {
        Ok(child) => child,
        Err(source) => {
            if let Some(motd_file_path) = motd_template_file_path {
                let _ = fs::remove_file(motd_file_path);
            }
            if let Some(rooms_db_path) = persistent_rooms_db_path {
                let _ = fs::remove_file(rooms_db_path);
            }
            if let Some(permanent_rooms_path) = permanent_rooms_file_path {
                let _ = fs::remove_file(permanent_rooms_path);
            }
            return Err(InteropError::PythonSpawn {
                python: python_bin_display,
                source,
            });
        }
    };

    let result = (|| {
        wait_for_legacy_server_startup(port, &mut child)?;

        let mut clients: BTreeMap<String, LegacyServerClientConnection> = BTreeMap::new();
        let mut events = Vec::with_capacity(steps.len());
        for step in steps {
            ensure_legacy_server_is_running(&mut child)?;
            if !clients.contains_key(&step.client_id) {
                let stream = connect_legacy_client_stream(port, &step.client_id)?;
                clients.insert(
                    step.client_id.clone(),
                    LegacyServerClientConnection {
                        stream,
                        pending_bytes: Vec::new(),
                    },
                );
            }

            let legacy_advance_seconds =
                step.legacy_advance_seconds.unwrap_or(step.advance_seconds);
            if legacy_advance_seconds > 0.0 {
                thread::sleep(Duration::from_secs_f64(legacy_advance_seconds));
            }

            let stream = clients
                .get_mut(&step.client_id)
                .ok_or_else(|| InteropError::MissingLegacyClient(step.client_id.clone()))?;
            let legacy_request_line = prepare_legacy_server_request_line(&step.request_line)?;
            stream.stream.write_all(legacy_request_line.as_bytes())?;
            // Twisted LineReceiver defaults to CRLF framing.
            stream.stream.write_all(b"\r\n")?;
            stream.stream.flush()?;

            let outbound_lines = collect_legacy_server_step_outputs(&mut clients)?;
            events.push(ServerRuntimeScenarioEvent {
                client_id: step.client_id.clone(),
                request_line: step.request_line.clone(),
                outbound_lines,
            });
        }

        Ok(events)
    })();

    terminate_legacy_server_process(&mut child);
    if let Some(motd_file_path) = motd_template_file_path {
        let _ = fs::remove_file(motd_file_path);
    }
    if let Some(rooms_db_path) = persistent_rooms_db_path {
        let _ = fs::remove_file(rooms_db_path);
    }
    if let Some(permanent_rooms_path) = permanent_rooms_file_path {
        let _ = fs::remove_file(permanent_rooms_path);
    }
    result
}
