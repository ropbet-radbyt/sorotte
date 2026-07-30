use super::*;

pub(in crate::tests) fn run_legacy_server_tls_upgrade_roundtrip_with_cert_path(
    tls_cert_path: &Path,
) -> Result<(String, String), InteropError> {
    let legacy_checkout = super::ensure_legacy_syncplay_checkout_available()?;

    let legacy_server_entry = super::legacy_syncplay_server_entry_script_path();
    if !legacy_server_entry.is_file() {
        return Err(InteropError::LegacyServerEntryScriptMissing(
            legacy_server_entry,
        ));
    }

    let port = super::reserve_ephemeral_tcp_port()?;
    let python_bin = super::python_bin_from_env();
    let python_bin_display = python_bin.to_string_lossy().to_string();
    let mut command = Command::new(&python_bin);
    command
        .arg(&legacy_server_entry)
        .arg("--port")
        .arg(port.to_string())
        .arg("--ipv4-only")
        .arg("--interface-ipv4")
        .arg("127.0.0.1")
        .arg("--salt")
        .arg(super::DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT)
        .arg("--tls")
        .arg(tls_cert_path)
        .current_dir(legacy_checkout)
        .env("PYTHONUNBUFFERED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|source| InteropError::PythonSpawn {
            python: python_bin_display,
            source,
        })?;

    let result = (|| {
        super::wait_for_legacy_server_startup(port, &mut child)?;
        super::ensure_legacy_server_is_running(&mut child)?;

        let stream = super::connect_legacy_client_stream(port, "legacy-tls-client")?;
        let mut connection = super::LegacyServerClientConnection {
            stream,
            pending_bytes: Vec::new(),
        };
        let request_line =
            super::prepare_legacy_server_request_line(r#"{"TLS":{"startTLS":"send"}}"#)?;
        connection.stream.write_all(request_line.as_bytes())?;
        connection.stream.write_all(b"\r\n")?;
        connection.stream.flush()?;

        let tls_response_line = read_plaintext_legacy_protocol_line_with_timeout(
            &mut connection,
            Duration::from_secs(2),
        )?;
        let tls_message = decode_message_line(&tls_response_line)?;
        let ProtocolMessage::Tls(tls_payload) = tls_message else {
            return Err(InteropError::InvalidPythonBatchResponse(format!(
                "expected legacy TLS response before upgrade, got: {tls_response_line}"
            )));
        };
        if tls_payload.tls.start_tls != "true" {
            return Err(InteropError::InvalidPythonBatchResponse(format!(
                "legacy TLS upgrade denied by server response: {tls_response_line}"
            )));
        }

        connection.pending_bytes.clear();
        connection.stream.set_nonblocking(false)?;
        connection
            .stream
            .set_read_timeout(Some(Duration::from_secs(3)))?;
        connection
            .stream
            .set_write_timeout(Some(Duration::from_secs(3)))?;
        let mut tls_stream = open_legacy_tls_client_stream(connection.stream, tls_cert_path)?;

        let hello_line = encode_message_line(&ProtocolMessage::hello(
            super::default_rust_client_hello_for_legacy_live_tls(),
        ))?;
        tls_stream.write_all(hello_line.as_bytes())?;
        tls_stream.write_all(b"\r\n")?;
        tls_stream.flush()?;
        let mut tls_pending_bytes = Vec::new();

        let mut hello_response_line = None;
        for _ in 0..8 {
            let candidate_line = read_tls_protocol_line_with_timeout(
                &mut tls_stream,
                &mut tls_pending_bytes,
                Duration::from_secs(3),
            )?;
            let candidate_message = decode_message_line(&candidate_line)?;
            if extract_hello_from_message(candidate_message).is_ok() {
                hello_response_line = Some(candidate_line);
                break;
            }
        }
        let hello_response_line = hello_response_line.ok_or_else(|| {
            InteropError::InvalidPythonBatchResponse(
                "timed out waiting for legacy hello response over upgraded TLS socket".to_owned(),
            )
        })?;

        Ok((tls_response_line, hello_response_line))
    })();

    super::terminate_legacy_server_process(&mut child);
    result
}

pub(in crate::tests) fn run_legacy_server_tls_logged_client_send_denied_roundtrip_with_cert_path(
    tls_cert_path: &Path,
) -> Result<String, InteropError> {
    let legacy_checkout = super::ensure_legacy_syncplay_checkout_available()?;

    let legacy_server_entry = super::legacy_syncplay_server_entry_script_path();
    if !legacy_server_entry.is_file() {
        return Err(InteropError::LegacyServerEntryScriptMissing(
            legacy_server_entry,
        ));
    }

    let port = super::reserve_ephemeral_tcp_port()?;
    let python_bin = super::python_bin_from_env();
    let python_bin_display = python_bin.to_string_lossy().to_string();
    let mut command = Command::new(&python_bin);
    command
        .arg(&legacy_server_entry)
        .arg("--port")
        .arg(port.to_string())
        .arg("--ipv4-only")
        .arg("--interface-ipv4")
        .arg("127.0.0.1")
        .arg("--salt")
        .arg(super::DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT)
        .arg("--tls")
        .arg(tls_cert_path)
        .current_dir(legacy_checkout)
        .env("PYTHONUNBUFFERED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|source| InteropError::PythonSpawn {
            python: python_bin_display,
            source,
        })?;

    let result = (|| {
        super::wait_for_legacy_server_startup(port, &mut child)?;
        super::ensure_legacy_server_is_running(&mut child)?;

        // First verify TLS is actually available for unlogged clients in this legacy setup.
        let probe_stream = super::connect_legacy_client_stream(port, "legacy-tls-probe")?;
        let mut probe_connection = super::LegacyServerClientConnection {
            stream: probe_stream,
            pending_bytes: Vec::new(),
        };
        let tls_request_line =
            super::prepare_legacy_server_request_line(r#"{"TLS":{"startTLS":"send"}}"#)?;
        probe_connection
            .stream
            .write_all(tls_request_line.as_bytes())?;
        probe_connection.stream.write_all(b"\r\n")?;
        probe_connection.stream.flush()?;
        let probe_response_line = read_plaintext_legacy_protocol_line_with_timeout(
            &mut probe_connection,
            Duration::from_secs(2),
        )?;
        let probe_message = decode_message_line(&probe_response_line)?;
        let ProtocolMessage::Tls(probe_payload) = probe_message else {
            return Err(InteropError::InvalidPythonBatchResponse(format!(
                "expected legacy TLS probe response, got: {probe_response_line}"
            )));
        };
        if probe_payload.tls.start_tls != "true" {
            return Err(InteropError::InvalidPythonBatchResponse(format!(
                "legacy tls availability probe returned non-true response: {probe_response_line}"
            )));
        }

        let stream = super::connect_legacy_client_stream(port, "legacy-tls-logged-client")?;
        let mut connection = super::LegacyServerClientConnection {
            stream,
            pending_bytes: Vec::new(),
        };
        let hello_line = encode_message_line(&ProtocolMessage::hello(
            super::default_rust_client_hello_for_legacy_live_tls(),
        ))?;
        connection.stream.write_all(hello_line.as_bytes())?;
        connection.stream.write_all(b"\r\n")?;
        connection.stream.flush()?;

        let mut saw_hello = false;
        for _ in 0..8 {
            let line = read_plaintext_legacy_protocol_line_with_timeout(
                &mut connection,
                Duration::from_secs(2),
            )?;
            let message = decode_message_line(&line)?;
            if matches!(message, ProtocolMessage::Hello(_)) {
                saw_hello = true;
                break;
            }
        }
        if !saw_hello {
            return Err(InteropError::InvalidPythonBatchResponse(
                "timed out waiting for legacy hello response before logged TLS probe".to_owned(),
            ));
        }

        connection.stream.write_all(tls_request_line.as_bytes())?;
        connection.stream.write_all(b"\r\n")?;
        connection.stream.flush()?;
        let logged_tls_response_line = read_plaintext_legacy_protocol_line_with_timeout(
            &mut connection,
            Duration::from_secs(2),
        )?;
        let logged_tls_message = decode_message_line(&logged_tls_response_line)?;
        let ProtocolMessage::Tls(logged_tls_payload) = logged_tls_message else {
            return Err(InteropError::InvalidPythonBatchResponse(format!(
                "expected legacy logged TLS response, got: {logged_tls_response_line}"
            )));
        };
        if logged_tls_payload.tls.start_tls != "false" {
            return Err(InteropError::InvalidPythonBatchResponse(format!(
                "legacy tls send was not denied for logged client: {logged_tls_response_line}"
            )));
        }

        Ok(logged_tls_response_line)
    })();

    super::terminate_legacy_server_process(&mut child);
    result
}

pub(in crate::tests) fn run_legacy_server_tls_rotation_invalidates_subsequent_send_with_cert_path(
    tls_cert_path: &Path,
) -> Result<(String, String), InteropError> {
    let legacy_checkout = super::ensure_legacy_syncplay_checkout_available()?;

    let legacy_server_entry = super::legacy_syncplay_server_entry_script_path();
    if !legacy_server_entry.is_file() {
        return Err(InteropError::LegacyServerEntryScriptMissing(
            legacy_server_entry,
        ));
    }

    let port = super::reserve_ephemeral_tcp_port()?;
    let python_bin = super::python_bin_from_env();
    let python_bin_display = python_bin.to_string_lossy().to_string();
    let mut command = Command::new(&python_bin);
    command
        .arg(&legacy_server_entry)
        .arg("--port")
        .arg(port.to_string())
        .arg("--ipv4-only")
        .arg("--interface-ipv4")
        .arg("127.0.0.1")
        .arg("--salt")
        .arg(super::DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT)
        .arg("--tls")
        .arg(tls_cert_path)
        .current_dir(legacy_checkout)
        .env("PYTHONUNBUFFERED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|source| InteropError::PythonSpawn {
            python: python_bin_display,
            source,
        })?;

    let result = (|| {
        super::wait_for_legacy_server_startup(port, &mut child)?;
        super::ensure_legacy_server_is_running(&mut child)?;

        let initial_stream = super::connect_legacy_client_stream(port, "legacy-tls-initial")?;
        let mut initial_connection = super::LegacyServerClientConnection {
            stream: initial_stream,
            pending_bytes: Vec::new(),
        };
        let tls_request_line =
            super::prepare_legacy_server_request_line(r#"{"TLS":{"startTLS":"send"}}"#)?;
        initial_connection
            .stream
            .write_all(tls_request_line.as_bytes())?;
        initial_connection.stream.write_all(b"\r\n")?;
        initial_connection.stream.flush()?;
        let initial_response_line = read_plaintext_legacy_protocol_line_with_timeout(
            &mut initial_connection,
            Duration::from_secs(2),
        )?;
        let initial_message = decode_message_line(&initial_response_line)?;
        let ProtocolMessage::Tls(initial_payload) = initial_message else {
            return Err(InteropError::InvalidPythonBatchResponse(format!(
                "expected initial legacy TLS probe response, got: {initial_response_line}"
            )));
        };
        if initial_payload.tls.start_tls != "true" {
            return Err(InteropError::InvalidPythonBatchResponse(format!(
                "legacy tls availability probe returned non-true response: {initial_response_line}"
            )));
        }

        fs::remove_file(tls_cert_path.join("chain.pem"))?;
        overwrite_file_until_modified_time_changes(
            &tls_cert_path.join("cert.pem"),
            "legacy-rotated-invalid",
        )?;
        super::ensure_legacy_server_is_running(&mut child)?;

        let rotated_stream = super::connect_legacy_client_stream(port, "legacy-tls-rotated")?;
        let mut rotated_connection = super::LegacyServerClientConnection {
            stream: rotated_stream,
            pending_bytes: Vec::new(),
        };
        rotated_connection
            .stream
            .write_all(tls_request_line.as_bytes())?;
        rotated_connection.stream.write_all(b"\r\n")?;
        rotated_connection.stream.flush()?;
        let rotated_response_line = read_plaintext_legacy_protocol_line_with_timeout(
            &mut rotated_connection,
            Duration::from_secs(2),
        )?;
        let rotated_message = decode_message_line(&rotated_response_line)?;
        let ProtocolMessage::Tls(rotated_payload) = rotated_message else {
            return Err(InteropError::InvalidPythonBatchResponse(format!(
                "expected rotated legacy TLS probe response, got: {rotated_response_line}"
            )));
        };
        if rotated_payload.tls.start_tls != "false" {
            return Err(InteropError::InvalidPythonBatchResponse(format!(
                "legacy tls rotation invalidation expected false response: {rotated_response_line}"
            )));
        }

        Ok((initial_response_line, rotated_response_line))
    })();

    super::terminate_legacy_server_process(&mut child);
    result
}

pub(in crate::tests) fn run_legacy_server_tls_rotation_recovers_after_bundle_restored_with_cert_path(
    tls_cert_path: &Path,
) -> Result<(String, String, String), InteropError> {
    let legacy_checkout = super::ensure_legacy_syncplay_checkout_available()?;

    let legacy_server_entry = super::legacy_syncplay_server_entry_script_path();
    if !legacy_server_entry.is_file() {
        return Err(InteropError::LegacyServerEntryScriptMissing(
            legacy_server_entry,
        ));
    }

    let port = super::reserve_ephemeral_tcp_port()?;
    let python_bin = super::python_bin_from_env();
    let python_bin_display = python_bin.to_string_lossy().to_string();
    let mut command = Command::new(&python_bin);
    command
        .arg(&legacy_server_entry)
        .arg("--port")
        .arg(port.to_string())
        .arg("--ipv4-only")
        .arg("--interface-ipv4")
        .arg("127.0.0.1")
        .arg("--salt")
        .arg(super::DEFAULT_LEGACY_SERVER_CONTROLLED_ROOM_SALT)
        .arg("--tls")
        .arg(tls_cert_path)
        .current_dir(legacy_checkout)
        .env("PYTHONUNBUFFERED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|source| InteropError::PythonSpawn {
            python: python_bin_display,
            source,
        })?;

    let result = (|| {
        super::wait_for_legacy_server_startup(port, &mut child)?;
        super::ensure_legacy_server_is_running(&mut child)?;

        let tls_request_line =
            super::prepare_legacy_server_request_line(r#"{"TLS":{"startTLS":"send"}}"#)?;

        let initial_stream = super::connect_legacy_client_stream(port, "legacy-tls-initial")?;
        let mut initial_connection = super::LegacyServerClientConnection {
            stream: initial_stream,
            pending_bytes: Vec::new(),
        };
        initial_connection
            .stream
            .write_all(tls_request_line.as_bytes())?;
        initial_connection.stream.write_all(b"\r\n")?;
        initial_connection.stream.flush()?;
        let initial_response_line = read_plaintext_legacy_protocol_line_with_timeout(
            &mut initial_connection,
            Duration::from_secs(2),
        )?;
        let initial_message = decode_message_line(&initial_response_line)?;
        let ProtocolMessage::Tls(initial_payload) = initial_message else {
            return Err(InteropError::InvalidPythonBatchResponse(format!(
                "expected initial legacy TLS probe response, got: {initial_response_line}"
            )));
        };
        if initial_payload.tls.start_tls != "true" {
            return Err(InteropError::InvalidPythonBatchResponse(format!(
                "legacy tls availability probe returned non-true response: {initial_response_line}"
            )));
        }

        fs::remove_file(tls_cert_path.join("chain.pem"))?;
        overwrite_file_until_modified_time_changes(
            &tls_cert_path.join("cert.pem"),
            "legacy-rotated-invalid",
        )?;
        super::ensure_legacy_server_is_running(&mut child)?;

        let rotated_stream = super::connect_legacy_client_stream(port, "legacy-tls-rotated")?;
        let mut rotated_connection = super::LegacyServerClientConnection {
            stream: rotated_stream,
            pending_bytes: Vec::new(),
        };
        rotated_connection
            .stream
            .write_all(tls_request_line.as_bytes())?;
        rotated_connection.stream.write_all(b"\r\n")?;
        rotated_connection.stream.flush()?;
        let rotated_response_line = read_plaintext_legacy_protocol_line_with_timeout(
            &mut rotated_connection,
            Duration::from_secs(2),
        )?;
        let rotated_message = decode_message_line(&rotated_response_line)?;
        let ProtocolMessage::Tls(rotated_payload) = rotated_message else {
            return Err(InteropError::InvalidPythonBatchResponse(format!(
                "expected rotated legacy TLS probe response, got: {rotated_response_line}"
            )));
        };
        if rotated_payload.tls.start_tls != "false" {
            return Err(InteropError::InvalidPythonBatchResponse(format!(
                "legacy tls rotation invalidation expected false response: {rotated_response_line}"
            )));
        }

        write_valid_tls_bundle(tls_cert_path);
        rewrite_file_until_modified_time_changes(
            &tls_cert_path.join("cert.pem"),
            TEST_TLS_CERT_PEM,
        )?;
        super::ensure_legacy_server_is_running(&mut child)?;

        let recovered_stream = super::connect_legacy_client_stream(port, "legacy-tls-recovered")?;
        let mut recovered_connection = super::LegacyServerClientConnection {
            stream: recovered_stream,
            pending_bytes: Vec::new(),
        };
        recovered_connection
            .stream
            .write_all(tls_request_line.as_bytes())?;
        recovered_connection.stream.write_all(b"\r\n")?;
        recovered_connection.stream.flush()?;
        let recovered_response_line = read_plaintext_legacy_protocol_line_with_timeout(
            &mut recovered_connection,
            Duration::from_secs(2),
        )?;
        let recovered_message = decode_message_line(&recovered_response_line)?;
        let ProtocolMessage::Tls(recovered_payload) = recovered_message else {
            return Err(InteropError::InvalidPythonBatchResponse(format!(
                "expected recovered legacy TLS probe response, got: {recovered_response_line}"
            )));
        };
        if recovered_payload.tls.start_tls != "true" {
            return Err(InteropError::InvalidPythonBatchResponse(format!(
                "legacy tls recovery expected true response after valid bundle restore: {recovered_response_line}"
            )));
        }

        Ok((
            initial_response_line,
            rotated_response_line,
            recovered_response_line,
        ))
    })();

    super::terminate_legacy_server_process(&mut child);
    result
}

pub(in crate::tests) fn legacy_server_tls_prerequisites_missing(error: &InteropError) -> bool {
    if required_live_interop_enabled() {
        return false;
    }
    if legacy_server_prerequisites_missing(error) {
        return true;
    }
    match error {
        InteropError::LegacyServerExited { stdout, stderr, .. } => {
            let lowered = format!("{stdout}\n{stderr}").to_ascii_lowercase();
            lowered.contains("no module named 'openssl'")
                || lowered.contains("unable import openssl")
                || lowered.contains("unable to import openssl")
                || lowered.contains("error while loading the tls certificates")
                || lowered.contains("tls support is not enabled")
        }
        InteropError::InvalidPythonBatchResponse(message) => {
            let lowered = message.to_ascii_lowercase();
            lowered.contains("legacy tls upgrade denied by server response")
                    || lowered.contains("legacy tls availability probe returned non-true response")
                    || lowered.contains(
                        "legacy tls cert file modified time did not change after repeated overwrite attempts",
                    )
                    || lowered.contains(
                        "legacy tls cert file modified time did not change after repeated rewrite attempts",
                    )
        }
        _ => false,
    }
}
