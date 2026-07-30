use super::*;

pub(crate) fn prepare_legacy_server_request_line(
    request_line: &str,
) -> Result<String, InteropError> {
    let mut request_value: Value = serde_json::from_str(request_line)?;
    if let Some(hello) = request_value
        .get_mut("Hello")
        .and_then(Value::as_object_mut)
        && !hello.contains_key("features")
    {
        // Syncplay v1.7.5 normally synthesizes these values in
        // SyncServerProtocol.getFeatures(). Its first-client Hello path instead
        // reads the still-null feature map before getFeatures() runs and aborts
        // the connection. Supplying the values that getFeatures() would create
        // keeps the live reference probe behaviorally faithful without a
        // capability-changing sentinel field.
        let version = hello
            .get("realversion")
            .or_else(|| hello.get("version"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        hello.insert(
            "features".to_owned(),
            legacy_default_features_for_version(version),
        );
    }
    Ok(serde_json::to_string(&request_value)?)
}

fn legacy_default_features_for_version(version: &str) -> Value {
    json!({
        "sharedPlaylists": numeric_version_meets_minimum(version, "1.4.0"),
        "chat": numeric_version_meets_minimum(version, "1.5.0"),
        "featureList": false,
        "readiness": numeric_version_meets_minimum(version, "1.3.0"),
        "managedRooms": numeric_version_meets_minimum(version, "1.3.0"),
        "persistentRooms": false,
        "uiMode": "Unknown",
    })
}

fn numeric_version_meets_minimum(version: &str, minimum: &str) -> bool {
    fn components(value: &str) -> Option<Vec<u32>> {
        value
            .split('.')
            .map(|part| part.parse::<u32>().ok())
            .collect()
    }

    let Some(mut version) = components(version) else {
        return false;
    };
    let Some(mut minimum) = components(minimum) else {
        return false;
    };
    let width = version.len().max(minimum.len());
    version.resize(width, 0);
    minimum.resize(width, 0);
    version >= minimum
}

pub(crate) fn reserve_ephemeral_tcp_port() -> Result<u16, InteropError> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

pub(crate) fn write_legacy_motd_template_file(template: &str) -> Result<PathBuf, InteropError> {
    let unique_suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let filename = format!(
        "sorotte-motd-template-{}-{}.txt",
        std::process::id(),
        unique_suffix
    );
    let path = env::temp_dir().join(filename);
    fs::write(&path, template)?;
    Ok(path)
}

pub(crate) fn create_temporary_legacy_rooms_db_file_path() -> Result<PathBuf, InteropError> {
    let unique_suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let filename = format!(
        "sorotte-persistent-rooms-{}-{}.sqlite3",
        std::process::id(),
        unique_suffix
    );
    let path = env::temp_dir().join(filename);
    fs::write(&path, b"")?;
    Ok(path)
}

pub(crate) fn create_temporary_legacy_permanent_rooms_file_path(
    permanent_rooms: &[&str],
) -> Result<PathBuf, InteropError> {
    let unique_suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let filename = format!(
        "sorotte-permanent-rooms-{}-{}.txt",
        std::process::id(),
        unique_suffix
    );
    let path = env::temp_dir().join(filename);
    let contents = permanent_rooms.join("\n");
    fs::write(&path, contents)?;
    Ok(path)
}

pub(crate) fn wait_for_legacy_server_startup(
    port: u16,
    child: &mut Child,
) -> Result<(), InteropError> {
    let startup_deadline = Instant::now() + LEGACY_SERVER_START_TIMEOUT;
    while Instant::now() <= startup_deadline {
        if let Some(status) = child.try_wait()? {
            let (stdout, stderr) = collect_child_pipes(child);
            return Err(InteropError::LegacyServerExited {
                exit_code: status.code(),
                stdout,
                stderr,
            });
        }

        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return Ok(());
        }

        thread::sleep(Duration::from_millis(40));
    }

    Err(InteropError::LegacyServerStartTimeout {
        port,
        stdout: String::new(),
        stderr: String::new(),
    })
}

pub(crate) fn ensure_legacy_server_is_running(child: &mut Child) -> Result<(), InteropError> {
    if let Some(status) = child.try_wait()? {
        let (stdout, stderr) = collect_child_pipes(child);
        return Err(InteropError::LegacyServerExited {
            exit_code: status.code(),
            stdout,
            stderr,
        });
    }
    Ok(())
}

pub(crate) fn connect_legacy_client_stream(
    port: u16,
    client_id: &str,
) -> Result<TcpStream, InteropError> {
    let connect_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => {
                stream.set_nodelay(true).map_err(|source| {
                    InteropError::LegacyClientConnectionInit {
                        client_id: client_id.to_owned(),
                        source,
                    }
                })?;
                stream.set_nonblocking(true).map_err(|source| {
                    InteropError::LegacyClientConnectionInit {
                        client_id: client_id.to_owned(),
                        source,
                    }
                })?;
                return Ok(stream);
            }
            Err(source) => {
                if Instant::now() >= connect_deadline {
                    return Err(InteropError::LegacyClientConnectionInit {
                        client_id: client_id.to_owned(),
                        source,
                    });
                }
                thread::sleep(Duration::from_millis(25));
            }
        }
    }
}

pub(crate) fn collect_legacy_server_step_outputs(
    clients: &mut BTreeMap<String, LegacyServerClientConnection>,
) -> Result<Vec<DirectedOutboundLine>, InteropError> {
    let mut outputs = Vec::new();
    let step_start = Instant::now();
    let mut last_activity = Instant::now();
    loop {
        let mut saw_new_output = false;
        for (client_id, connection) in clients.iter_mut() {
            let lines = drain_legacy_client_lines(connection)?;
            if lines.is_empty() {
                continue;
            }
            saw_new_output = true;
            for line in lines {
                outputs.push(DirectedOutboundLine {
                    client_id: client_id.clone(),
                    line,
                    delivery: ServerOutboundDelivery::Reliable,
                });
            }
        }
        if saw_new_output {
            last_activity = Instant::now();
        }

        if step_start.elapsed() >= LEGACY_SERVER_STEP_MIN_WAIT
            && last_activity.elapsed() >= LEGACY_SERVER_STEP_IDLE_WAIT
        {
            break;
        }
        if step_start.elapsed() >= LEGACY_SERVER_STEP_MAX_WAIT {
            break;
        }

        thread::sleep(Duration::from_millis(5));
    }

    Ok(outputs)
}

pub(crate) fn drain_legacy_client_lines(
    connection: &mut LegacyServerClientConnection,
) -> Result<Vec<String>, InteropError> {
    let mut chunk = [0_u8; 4096];
    loop {
        match connection.stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => connection.pending_bytes.extend_from_slice(&chunk[..count]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(error) => return Err(InteropError::Io(error)),
        }
    }

    let mut lines = Vec::new();
    while let Some(newline_index) = connection
        .pending_bytes
        .iter()
        .position(|byte| *byte == b'\n')
    {
        let mut raw_line: Vec<u8> = connection.pending_bytes.drain(..=newline_index).collect();
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
        if decode_message_line(&line).is_err() {
            continue;
        }
        lines.push(line);
    }

    Ok(lines)
}

pub(crate) fn terminate_legacy_server_process(child: &mut Child) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = collect_child_pipes(child);
}

pub(crate) fn collect_child_pipes(child: &mut Child) -> (String, String) {
    let stdout = child
        .stdout
        .take()
        .map(read_process_pipe_to_string)
        .unwrap_or_default();
    let stderr = child
        .stderr
        .take()
        .map(read_process_pipe_to_string)
        .unwrap_or_default();
    (stdout, stderr)
}

pub(crate) fn read_process_pipe_to_string<R: Read>(mut reader: R) -> String {
    let mut buffer = Vec::new();
    let _ = reader.read_to_end(&mut buffer);
    String::from_utf8_lossy(&buffer).trim().to_owned()
}

pub(crate) fn capture_process_output_lines<R: Read + Send + 'static>(
    reader: R,
    sink: Arc<Mutex<Vec<String>>>,
    line_tx: Option<mpsc::Sender<String>>,
) {
    thread::spawn(move || {
        let reader = BufReader::new(reader);
        for line in reader.lines() {
            let Ok(line) = line else {
                break;
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(mut locked) = sink.lock() {
                locked.push(trimmed.to_owned());
            }
            if let Some(tx) = line_tx.as_ref() {
                let _ = tx.send(trimmed.to_owned());
            }
        }
    });
}

pub(crate) fn captured_process_output(lines: &Arc<Mutex<Vec<String>>>) -> String {
    lines
        .lock()
        .map(|locked| locked.join("\n"))
        .unwrap_or_default()
}

pub(crate) fn wait_for_child_exit_with_timeout(
    child: &mut Child,
    timeout: Duration,
) -> Result<bool, std::io::Error> {
    let deadline = Instant::now() + timeout;
    loop {
        if child.try_wait()?.is_some() {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        thread::sleep(Duration::from_millis(50));
    }
}

pub(crate) fn syncplay_repo_root_dir() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("..");
    path.push("..");
    path
}

pub(crate) fn repo_local_legacy_syncplay_checkout_dir() -> PathBuf {
    let mut path = syncplay_repo_root_dir();
    path.push(".interop-cache");
    path.push("syncplay-legacy");
    path
}

pub(crate) fn configured_legacy_syncplay_checkout_dir() -> Option<PathBuf> {
    env::var_os("SYNCPLAY_LEGACY_ROOT")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn live_interop_required_from_value(value: Option<&std::ffi::OsStr>) -> bool {
    value.is_some_and(|value| value == "1")
}

/// Returns whether optional live Python interoperability prerequisites are
/// required for this process.
///
/// Ordinary developer runs retain the historical optional behavior. Strict CI
/// and release lanes set this exact switch to make every shared prerequisite
/// classifier fail closed.
pub fn required_live_interop_enabled() -> bool {
    live_interop_required_from_value(env::var_os("SYNCPLAY_REQUIRE_LIVE_INTEROP").as_deref())
}

fn required_live_prerequisite_from_mode(required: bool, error: InteropError) -> InteropError {
    if required {
        InteropError::RequiredLivePrerequisite {
            source: Box::new(error),
        }
    } else {
        error
    }
}

pub(crate) fn required_live_prerequisite_error(error: InteropError) -> InteropError {
    required_live_prerequisite_from_mode(required_live_interop_enabled(), error)
}

pub(crate) fn legacy_syncplay_checkout_bootstrap_lock() -> &'static Mutex<()> {
    LEGACY_SYNCPLAY_BOOTSTRAP_LOCK.get_or_init(|| Mutex::new(()))
}

pub(crate) fn legacy_syncplay_checkout_is_ready(path: &Path) -> bool {
    path.join("syncplayServer.py").is_file()
}

pub(crate) fn remove_legacy_syncplay_checkout_path_if_present(
    path: &Path,
) -> Result<(), InteropError> {
    if !path.exists() {
        return Ok(());
    }
    if path.is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub(crate) fn bootstrap_repo_local_legacy_syncplay_checkout(
    path: &Path,
) -> Result<(), InteropError> {
    remove_legacy_syncplay_checkout_path_if_present(path)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let clone_result = Command::new("git")
        .arg("clone")
        .arg("--depth")
        .arg("1")
        .arg("--branch")
        .arg(LEGACY_SYNCPLAY_UPSTREAM_REF)
        .arg("--single-branch")
        .arg(LEGACY_SYNCPLAY_UPSTREAM_REPO)
        .arg(path)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    match clone_result {
        Ok(status) if status.success() && legacy_syncplay_checkout_is_ready(path) => Ok(()),
        Ok(_) | Err(_) => {
            let _ = remove_legacy_syncplay_checkout_path_if_present(path);
            Err(InteropError::LegacySyncplayCheckoutMissing(
                path.to_path_buf(),
            ))
        }
    }
}

pub(crate) fn ensure_legacy_syncplay_checkout_available() -> Result<PathBuf, InteropError> {
    let configured_checkout = configured_legacy_syncplay_checkout_dir();
    if required_live_interop_enabled() && configured_checkout.is_none() {
        return Err(required_live_prerequisite_error(
            InteropError::LegacySyncplayCheckoutMissing(repo_local_legacy_syncplay_checkout_dir()),
        ));
    }
    if let Some(legacy_checkout) = configured_checkout {
        if !legacy_checkout.is_dir() {
            return Err(required_live_prerequisite_error(
                InteropError::LegacySyncplayCheckoutMissing(legacy_checkout),
            ));
        }
        return Ok(legacy_checkout);
    }

    let _guard = legacy_syncplay_checkout_bootstrap_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let legacy_checkout = repo_local_legacy_syncplay_checkout_dir();
    if legacy_syncplay_checkout_is_ready(&legacy_checkout) {
        return Ok(legacy_checkout);
    }
    bootstrap_repo_local_legacy_syncplay_checkout(&legacy_checkout)
        .map_err(required_live_prerequisite_error)?;
    Ok(legacy_checkout)
}

pub fn legacy_syncplay_checkout_dir() -> PathBuf {
    configured_legacy_syncplay_checkout_dir()
        .unwrap_or_else(repo_local_legacy_syncplay_checkout_dir)
}

pub fn legacy_syncplay_server_entry_script_path() -> PathBuf {
    legacy_syncplay_checkout_dir().join("syncplayServer.py")
}

pub fn python_handshake_probe_script_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("scripts");
    path.push("python_handshake_probe.py");
    path
}

pub fn python_live_peer_probe_script_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("scripts");
    path.push("python_live_peer_probe.py");
    path
}

pub(crate) fn stderr_indicates_missing_legacy_prerequisites(stderr: &str) -> bool {
    let lowered = stderr.to_ascii_lowercase();
    lowered.contains("no module named 'twisted'")
        || lowered.contains("unable import twisted")
        || lowered.contains("unable to import twisted")
}

pub fn interop_prerequisites_missing(error: &InteropError) -> bool {
    if required_live_interop_enabled() {
        return false;
    }
    match error {
        InteropError::LegacySyncplayCheckoutMissing(_)
        | InteropError::PythonHandshakeProbeMissing(_)
        | InteropError::PythonLivePeerProbeMissing(_)
        | InteropError::LegacyServerEntryScriptMissing(_)
        | InteropError::PythonSpawn { .. } => true,
        InteropError::LegacyServerExited { stderr, .. }
        | InteropError::LegacyServerStartTimeout { stderr, .. }
        | InteropError::PythonLivePeerExited { stderr, .. }
        | InteropError::PythonLivePeerStartTimeout { stderr, .. } => {
            stderr_indicates_missing_legacy_prerequisites(stderr)
        }
        _ => false,
    }
}

#[cfg(test)]
mod required_live_tests {
    use super::{
        InteropError, live_interop_required_from_value, required_live_prerequisite_from_mode,
    };
    use std::ffi::OsStr;
    use std::path::PathBuf;

    #[test]
    fn required_live_interop_switch_accepts_only_exact_one() {
        assert!(live_interop_required_from_value(Some(OsStr::new("1"))));
        for value in [
            None,
            Some(OsStr::new("")),
            Some(OsStr::new("0")),
            Some(OsStr::new("true")),
        ] {
            assert!(!live_interop_required_from_value(value));
        }
    }

    #[test]
    fn required_live_mode_wraps_optional_prerequisite_errors() {
        let missing =
            || InteropError::LegacySyncplayCheckoutMissing(PathBuf::from("missing-oracle"));
        assert!(matches!(
            required_live_prerequisite_from_mode(false, missing()),
            InteropError::LegacySyncplayCheckoutMissing(_)
        ));
        assert!(matches!(
            required_live_prerequisite_from_mode(true, missing()),
            InteropError::RequiredLivePrerequisite { .. }
        ));
    }
}
