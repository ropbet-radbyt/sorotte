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

pub(crate) struct LegacyServerPortLease {
    port: u16,
    listener: Option<TcpListener>,
    _process_guard: fs::File,
    _thread_guard: std::sync::MutexGuard<'static, ()>,
}

impl LegacyServerPortLease {
    pub(crate) fn port(&self) -> u16 {
        self.port
    }

    pub(crate) fn release_socket_for_child(&mut self) {
        drop(self.listener.take());
    }
}

pub(crate) fn reserve_legacy_server_port() -> Result<LegacyServerPortLease, InteropError> {
    let lock_path = syncplay_repo_root_dir()
        .join("target")
        .join("sorotte-legacy-server-startup.lock");
    reserve_legacy_server_port_with_lock(&lock_path, LEGACY_SERVER_STARTUP_LOCK_WAIT, || {})
}

pub(crate) fn reserve_legacy_server_port_with_lock<F>(
    lock_path: &Path,
    lock_timeout: Duration,
    mut on_contention: F,
) -> Result<LegacyServerPortLease, InteropError>
where
    F: FnMut(),
{
    let thread_guard = LEGACY_SERVER_STARTUP_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let lock_parent = lock_path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "legacy server startup lock must have a parent directory",
        )
    })?;
    fs::create_dir_all(lock_parent)?;
    let process_guard = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(lock_path)?;
    let deadline = Instant::now() + lock_timeout;
    loop {
        match process_guard.try_lock() {
            Ok(()) => break,
            Err(fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                on_contention();
                thread::sleep(Duration::from_millis(10));
            }
            Err(fs::TryLockError::WouldBlock) => {
                return Err(InteropError::Io(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "timed out waiting for the legacy server startup lock",
                )));
            }
            Err(fs::TryLockError::Error(error)) => return Err(InteropError::Io(error)),
        }
    }
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    Ok(LegacyServerPortLease {
        port,
        listener: Some(listener),
        _process_guard: process_guard,
        _thread_guard: thread_guard,
    })
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

pub(crate) fn wait_for_legacy_permanent_rooms_startup(
    port: u16,
    child: &mut Child,
    permanent_rooms: &[&str],
) -> Result<(), InteropError> {
    if permanent_rooms.is_empty() {
        return Ok(());
    }

    // Syncplay v1.7.5 starts accepting TCP connections before its Twisted
    // adbapi callbacks have loaded the room database. A scenario client that
    // joins a configured permanent room during that window gets a transient
    // Room with playlistIndex=None instead of the permanent room's seeded
    // playlistIndex=0. Observe the GUI List contract before beginning the
    // scenario so the live oracle starts from its intended durable state.
    let mut probe_room_suffix = 0_u32;
    let probe_room = loop {
        let candidate = format!("compat-probe-{probe_room_suffix}-temp");
        if !permanent_rooms.contains(&candidate.as_str()) {
            break candidate;
        }
        probe_room_suffix += 1;
    };
    let mut connection = LegacyServerClientConnection {
        stream: connect_legacy_client_stream(port, "compat-room-startup-probe")?,
        pending_bytes: Vec::new(),
    };
    let hello = serde_json::to_string(&json!({
        "Hello": {
            "username": "compat-probe",
            "room": {"name": probe_room},
            "version": "9.9.9",
            "features": {
                "chat": false,
                "featureList": false,
                "managedRooms": true,
                "persistentRooms": false,
                "readiness": true,
                "sharedPlaylists": true,
                "uiMode": "GUI"
            }
        }
    }))?;
    connection.stream.write_all(hello.as_bytes())?;
    connection.stream.write_all(b"\r\n")?;
    connection.stream.flush()?;

    let list_request = b"{\"List\":null}\r\n";
    let startup_deadline = Instant::now() + LEGACY_SERVER_START_TIMEOUT;
    let mut next_list_request = Instant::now();
    while Instant::now() <= startup_deadline {
        ensure_legacy_server_is_running(child)?;
        for line in drain_legacy_client_lines(&mut connection)? {
            if legacy_list_contains_permanent_rooms(&line, permanent_rooms) {
                return close_legacy_startup_probe(connection, port, child);
            }
        }

        let now = Instant::now();
        if now >= next_list_request {
            connection.stream.write_all(list_request)?;
            connection.stream.flush()?;
            next_list_request = now + Duration::from_millis(40);
        }
        thread::sleep(Duration::from_millis(5));
    }

    Err(InteropError::LegacyServerPersistentRoomsStartTimeout {
        port,
        permanent_rooms: permanent_rooms
            .iter()
            .map(|room| (*room).to_owned())
            .collect(),
    })
}

fn close_legacy_startup_probe(
    mut connection: LegacyServerClientConnection,
    port: u16,
    child: &mut Child,
) -> Result<(), InteropError> {
    connection.stream.shutdown(Shutdown::Write)?;
    let disconnect_deadline = Instant::now() + LEGACY_SERVER_START_TIMEOUT;
    let mut discard = [0_u8; 4096];
    while Instant::now() <= disconnect_deadline {
        ensure_legacy_server_is_running(child)?;
        match connection.stream.read(&mut discard) {
            Ok(0) => return Ok(()),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(InteropError::Io(error)),
        }
        thread::sleep(Duration::from_millis(5));
    }
    Err(InteropError::LegacyServerStartupProbeDisconnectTimeout { port })
}

fn legacy_list_contains_permanent_rooms(line: &str, permanent_rooms: &[&str]) -> bool {
    let Ok(message) = serde_json::from_str::<Value>(line) else {
        return false;
    };
    let Some(list) = message.get("List").and_then(Value::as_object) else {
        return false;
    };
    permanent_rooms
        .iter()
        .all(|room_name| list.get(*room_name).is_some_and(Value::is_object))
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
    required_first_output_client: Option<&str>,
) -> Result<Vec<DirectedOutboundLine>, InteropError> {
    let mut outputs = Vec::new();
    let step_start = Instant::now();
    let mut last_activity = Instant::now();
    let mut saw_required_output = false;
    loop {
        let mut saw_new_output = false;
        for (client_id, connection) in clients.iter_mut() {
            let lines = drain_legacy_client_lines(connection)?;
            if lines.is_empty() {
                continue;
            }
            saw_new_output = true;
            if required_first_output_client == Some(client_id.as_str()) {
                saw_required_output = true;
            }
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

        let step_elapsed = step_start.elapsed();
        if legacy_server_step_collection_is_complete(
            required_first_output_client.is_some(),
            saw_required_output,
            step_elapsed,
            last_activity.elapsed(),
        ) {
            break;
        }

        thread::sleep(Duration::from_millis(5));
    }

    Ok(outputs)
}

pub(crate) fn legacy_server_step_collection_is_complete(
    wait_for_first_output: bool,
    saw_required_output: bool,
    step_elapsed: Duration,
    idle_elapsed: Duration,
) -> bool {
    step_elapsed >= LEGACY_SERVER_STEP_MAX_WAIT
        || ((!wait_for_first_output || saw_required_output)
            && step_elapsed >= LEGACY_SERVER_STEP_MIN_WAIT
            && idle_elapsed >= LEGACY_SERVER_STEP_IDLE_WAIT)
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

pub(crate) fn acquire_legacy_syncplay_checkout_process_lock<F>(
    checkout_path: &Path,
    timeout: Duration,
    mut on_contention: F,
) -> Result<fs::File, InteropError>
where
    F: FnMut(),
{
    let parent = checkout_path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "legacy Syncplay checkout must have a parent directory",
        )
    })?;
    fs::create_dir_all(parent)?;
    let checkout_name = checkout_path.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "legacy Syncplay checkout must have a file name",
        )
    })?;
    let lock_path = parent.join(format!(
        "{}.bootstrap.lock",
        checkout_name.to_string_lossy()
    ));
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(lock_path)?;
    let deadline = Instant::now() + timeout;
    loop {
        match lock_file.try_lock() {
            Ok(()) => return Ok(lock_file),
            Err(fs::TryLockError::WouldBlock) => {
                on_contention();
                if Instant::now() >= deadline {
                    return Err(InteropError::Io(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "timed out waiting for the legacy Syncplay checkout bootstrap lock",
                    )));
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(fs::TryLockError::Error(error)) => return Err(InteropError::Io(error)),
        }
    }
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

pub(crate) fn ensure_repo_local_legacy_syncplay_checkout_with<F, G>(
    legacy_checkout: &Path,
    lock_timeout: Duration,
    on_lock_contention: G,
    bootstrap: F,
) -> Result<PathBuf, InteropError>
where
    F: FnOnce(&Path) -> Result<(), InteropError>,
    G: FnMut(),
{
    let _process_guard = acquire_legacy_syncplay_checkout_process_lock(
        legacy_checkout,
        lock_timeout,
        on_lock_contention,
    )?;
    if legacy_syncplay_checkout_is_ready(legacy_checkout) {
        return Ok(legacy_checkout.to_path_buf());
    }
    bootstrap(legacy_checkout)?;
    if !legacy_syncplay_checkout_is_ready(legacy_checkout) {
        return Err(InteropError::LegacySyncplayCheckoutMissing(
            legacy_checkout.to_path_buf(),
        ));
    }
    Ok(legacy_checkout.to_path_buf())
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
    ensure_repo_local_legacy_syncplay_checkout_with(
        &legacy_checkout,
        LEGACY_SYNCPLAY_BOOTSTRAP_LOCK_WAIT,
        || {},
        bootstrap_repo_local_legacy_syncplay_checkout,
    )
    .map_err(required_live_prerequisite_error)
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
        InteropError, legacy_list_contains_permanent_rooms, live_interop_required_from_value,
        required_live_prerequisite_from_mode,
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

    #[test]
    fn permanent_room_startup_requires_a_gui_list_snapshot_with_every_room() {
        let expected_rooms = ["permanent-room", "second-room"];
        for line in [
            "not-json",
            r#"{"Set":{"playlistIndex":{"index":0}}}"#,
            r#"{"List":{"permanent-room":{" ":{}}}}"#,
            r#"{"List":{"permanent-room":null,"second-room":null}}"#,
        ] {
            assert!(!legacy_list_contains_permanent_rooms(line, &expected_rooms));
        }
        assert!(legacy_list_contains_permanent_rooms(
            r#"{"List":{"permanent-room":{" ":{}},"second-room":{" ":{}}}}"#,
            &expected_rooms
        ));
    }
}
