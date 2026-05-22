use super::*;

pub(in crate::tests) fn read_next_protocol_line_from_pending(
    pending_bytes: &mut Vec<u8>,
) -> Option<String> {
    loop {
        let newline_index = pending_bytes.iter().position(|byte| *byte == b'\n')?;
        let mut raw_line: Vec<u8> = pending_bytes.drain(..=newline_index).collect();
        if raw_line.last().is_some_and(|byte| *byte == b'\n') {
            raw_line.pop();
        }
        if raw_line.last().is_some_and(|byte| *byte == b'\r') {
            raw_line.pop();
        }
        if raw_line.is_empty() {
            continue;
        }

        let line = String::from_utf8_lossy(&raw_line).trim().to_owned();
        if line.is_empty() {
            continue;
        }
        if decode_message_line(&line).is_ok() {
            return Some(line);
        }
    }
}

pub(in crate::tests) fn read_plaintext_legacy_protocol_line_with_timeout(
    connection: &mut super::LegacyServerClientConnection,
    timeout: Duration,
) -> Result<String, InteropError> {
    let deadline = Instant::now() + timeout;
    let mut chunk = [0_u8; 4096];
    loop {
        if let Some(line) = read_next_protocol_line_from_pending(&mut connection.pending_bytes) {
            return Ok(line);
        }

        match connection.stream.read(&mut chunk) {
            Ok(0) => {}
            Ok(count) => connection.pending_bytes.extend_from_slice(&chunk[..count]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(InteropError::Io(error)),
        }

        if Instant::now() >= deadline {
            return Err(InteropError::InvalidPythonBatchResponse(
                "timed out waiting for legacy plaintext protocol line".to_owned(),
            ));
        }
        thread::sleep(Duration::from_millis(5));
    }
}

pub(in crate::tests) fn read_tls_protocol_line_with_timeout(
    stream: &mut StreamOwned<ClientConnection, TcpStream>,
    pending_bytes: &mut Vec<u8>,
    timeout: Duration,
) -> Result<String, InteropError> {
    let deadline = Instant::now() + timeout;
    let mut chunk = [0_u8; 4096];
    loop {
        if let Some(line) = read_next_protocol_line_from_pending(pending_bytes) {
            return Ok(line);
        }

        match stream.read(&mut chunk) {
            Ok(0) => {}
            Ok(count) => pending_bytes.extend_from_slice(&chunk[..count]),
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut => {}
            Err(error) => return Err(InteropError::Io(error)),
        }

        if Instant::now() >= deadline {
            return Err(InteropError::InvalidPythonBatchResponse(
                "timed out waiting for legacy TLS protocol line".to_owned(),
            ));
        }
        thread::sleep(Duration::from_millis(5));
    }
}

pub(in crate::tests) fn open_legacy_tls_client_stream(
    stream: TcpStream,
    tls_cert_path: &Path,
) -> Result<StreamOwned<ClientConnection, TcpStream>, InteropError> {
    let cert_pem_path = tls_cert_path.join("cert.pem");
    let cert_pem = fs::read(&cert_pem_path)?;
    let certs = rustls_pemfile::certs(&mut Cursor::new(cert_pem))
        .collect::<Result<Vec<_>, _>>()
        .map_err(InteropError::Io)?;
    if certs.is_empty() {
        return Err(InteropError::InvalidPythonBatchResponse(format!(
            "legacy TLS cert bundle contains no certificates at {}",
            cert_pem_path.display()
        )));
    }

    let mut roots = RootCertStore::empty();
    for cert in certs {
        roots.add(cert).map_err(|error| {
            InteropError::InvalidPythonBatchResponse(format!(
                "failed to add legacy TLS root certificate: {error}"
            ))
        })?;
    }

    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let server_name = ServerName::try_from("localhost").map_err(|error| {
        InteropError::InvalidPythonBatchResponse(format!("invalid legacy TLS server name: {error}"))
    })?;
    let connection =
        ClientConnection::new(Arc::new(config), server_name.to_owned()).map_err(|error| {
            InteropError::InvalidPythonBatchResponse(format!(
                "failed to initialize legacy TLS client connection: {error}"
            ))
        })?;

    Ok(StreamOwned::new(connection, stream))
}

pub(in crate::tests) fn overwrite_file_until_modified_time_changes(
    path: &Path,
    contents: &str,
) -> Result<(), InteropError> {
    let original_modified_time = fs::metadata(path)?.modified()?;
    for attempt in 0..8 {
        fs::write(path, format!("{contents}-{attempt}"))?;
        let updated_modified_time = fs::metadata(path)?.modified()?;
        if updated_modified_time != original_modified_time {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(250));
    }
    Err(InteropError::InvalidPythonBatchResponse(
        "legacy tls cert file modified time did not change after repeated overwrite attempts"
            .to_owned(),
    ))
}

pub(in crate::tests) fn rewrite_file_until_modified_time_changes(
    path: &Path,
    contents: &str,
) -> Result<(), InteropError> {
    let original_modified_time = fs::metadata(path)?.modified()?;
    for _ in 0..8 {
        fs::write(path, contents)?;
        let updated_modified_time = fs::metadata(path)?.modified()?;
        if updated_modified_time != original_modified_time {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(250));
    }
    Err(InteropError::InvalidPythonBatchResponse(
        "legacy tls cert file modified time did not change after repeated rewrite attempts"
            .to_owned(),
    ))
}
