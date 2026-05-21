use std::{
    env, fs, io,
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned, pki_types::ServerName};
use serde_json::{Value, json};
use syncplay_protocol::{
    ChatPayload, FilePayload, HelloPayload, ListPayload, PlaystatePayload, ProtocolMessage,
    ReadyPayload, SetPayload, StatePayload, TlsPayload, decode_message_line, encode_message_line,
};

pub const TEST_TLS_CERT_PEM: &str = include_str!("../../../../fixtures/tls/test_cert.pem");
pub const TEST_TLS_CHAIN_PEM: &str = include_str!("../../../../fixtures/tls/test_chain.pem");
pub const TEST_TLS_PRIVATE_KEY_PEM: &str =
    include_str!("../../../../fixtures/tls/test_privkey.pem");

pub fn strict_release_required() -> bool {
    env_flag_enabled("SYNCPLAY_REQUIRE_SERVER_RELEASE_VERIFY")
        || env_flag_enabled("SYNCPLAY_SERVER_RELEASE_VERIFY")
}

pub fn env_flag_enabled(name: &str) -> bool {
    env::var(name).ok().is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("server crate should live under workspace/crates")
        .to_path_buf()
}

pub fn python_bin_from_env() -> std::ffi::OsString {
    env::var_os("SYNCPLAY_PYTHON_BIN").unwrap_or_else(|| "python".into())
}

fn python_live_peer_prerequisites() -> Result<(), String> {
    let output = Command::new(python_bin_from_env())
        .arg("-c")
        .arg("import twisted, OpenSSL, service_identity")
        .output()
        .map_err(|error| format!("Python prerequisite check could not start: {error}"))?;
    if output.status.success() {
        return Ok(());
    }

    Err(format!(
        "Python prerequisites are missing or unusable; status={} stdout='{}' stderr='{}'",
        output.status,
        String::from_utf8_lossy(&output.stdout).trim(),
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

pub fn legacy_syncplay_root() -> Option<PathBuf> {
    if let Some(path) = env::var_os("SYNCPLAY_LEGACY_ROOT").filter(|value| !value.is_empty()) {
        let path = PathBuf::from(path);
        if path.join("syncplayServer.py").is_file() {
            return Some(path);
        }
        return None;
    }

    let repo_local = workspace_root()
        .join(".interop-cache")
        .join("syncplay-legacy");
    if repo_local.join("syncplayServer.py").is_file() {
        return Some(repo_local);
    }

    let sibling = workspace_root()
        .parent()
        .expect("workspace should have a parent")
        .join("syncplay");
    if sibling.join("syncplayServer.py").is_file() {
        return Some(sibling);
    }

    None
}

pub fn python_live_peer_probe_script() -> PathBuf {
    workspace_root()
        .join("crates")
        .join("syncplay-compat")
        .join("scripts")
        .join("python_live_peer_probe.py")
}

pub fn reserve_ipv4_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("ephemeral IPv4 port should bind");
    listener
        .local_addr()
        .expect("ephemeral IPv4 listener should have a local address")
        .port()
}

pub fn reserve_ipv6_port_or_skip() -> Option<u16> {
    let listener = match TcpListener::bind("[::1]:0") {
        Ok(listener) => listener,
        Err(error) => {
            if strict_release_required() {
                panic!(
                    "strict server release verification requires IPv6 loopback support: {error}"
                );
            }
            eprintln!("IPv6 listener test skipped; IPv6 loopback is unavailable: {error}");
            return None;
        }
    };
    Some(
        listener
            .local_addr()
            .expect("ephemeral IPv6 listener should have a local address")
            .port(),
    )
}

pub fn temporary_path(label: &str, extension: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after epoch")
        .as_nanos();
    env::temp_dir().join(format!(
        "syncplay-rs-{label}-{}-{suffix}.{extension}",
        std::process::id()
    ))
}

pub fn temporary_directory_path(label: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after epoch")
        .as_nanos();
    env::temp_dir().join(format!(
        "syncplay-rs-{label}-{}-{suffix}",
        std::process::id()
    ))
}

#[derive(Clone, Default)]
pub struct CapturedOutput {
    lines: Arc<Mutex<Vec<String>>>,
}

impl CapturedOutput {
    fn push(&self, line: String) {
        self.lines
            .lock()
            .expect("captured output lock should not be poisoned")
            .push(line);
    }

    pub fn text(&self) -> String {
        self.lines
            .lock()
            .expect("captured output lock should not be poisoned")
            .join("\n")
    }

    pub fn contains(&self, needle: &str) -> bool {
        self.text().contains(needle)
    }
}

pub fn capture_output_lines<R: Read + Send + 'static>(
    reader: R,
    captured: CapturedOutput,
    sender: Option<mpsc::Sender<String>>,
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
            captured.push(trimmed.to_owned());
            if let Some(sender) = sender.as_ref() {
                let _ = sender.send(trimmed.to_owned());
            }
        }
    });
}

pub struct ServerProcess {
    child: Option<Child>,
    pub stdout: CapturedOutput,
    pub stderr: CapturedOutput,
}

impl ServerProcess {
    pub fn spawn(args: &[String]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_syncplay-server"))
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|error| panic!("syncplay-server binary should spawn: {error}"));

        let stdout = CapturedOutput::default();
        let stderr = CapturedOutput::default();
        capture_output_lines(
            child
                .stdout
                .take()
                .expect("server stdout should be captured"),
            stdout.clone(),
            None,
        );
        capture_output_lines(
            child
                .stderr
                .take()
                .expect("server stderr should be captured"),
            stderr.clone(),
            None,
        );

        Self {
            child: Some(child),
            stdout,
            stderr,
        }
    }

    pub fn wait_for_ipv4(&mut self, port: u16) -> ProtocolClient {
        self.wait_for_address(SocketAddr::from(([127, 0, 0, 1], port)))
    }

    pub fn wait_for_ipv6(&mut self, port: u16) -> ProtocolClient {
        self.wait_for_address(SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], port)))
    }

    fn wait_for_address(&mut self, address: SocketAddr) -> ProtocolClient {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            self.assert_running();
            if let Ok(stream) = TcpStream::connect(address) {
                return ProtocolClient::from_stream(stream);
            }
            if Instant::now() >= deadline {
                panic!(
                    "syncplay-server did not accept connections on {address}; stdout='{}' stderr='{}'",
                    self.stdout.text(),
                    self.stderr.text()
                );
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    pub fn assert_running(&mut self) {
        let Some(child) = self.child.as_mut() else {
            panic!("server child should still be present");
        };
        if let Some(status) = child
            .try_wait()
            .expect("server child status should be inspectable")
        {
            panic!(
                "syncplay-server exited early with status {status}; stdout='{}' stderr='{}'",
                self.stdout.text(),
                self.stderr.text()
            );
        }
    }

    pub fn wait_for_stderr_contains(&self, needle: &str) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() <= deadline {
            if self.stderr.contains(needle) {
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
        panic!(
            "server stderr did not contain {needle:?}; stderr='{}'",
            self.stderr.text()
        );
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

pub struct ProtocolClient {
    reader: BufReader<TcpStream>,
}

impl ProtocolClient {
    pub fn connect_ipv4(port: u16) -> Self {
        let stream =
            TcpStream::connect(("127.0.0.1", port)).expect("client should connect over IPv4");
        Self::from_stream(stream)
    }

    pub fn from_stream(stream: TcpStream) -> Self {
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .expect("read timeout should be configurable");
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .expect("write timeout should be configurable");
        Self {
            reader: BufReader::new(stream),
        }
    }

    pub fn write_raw_line(&mut self, line: &[u8]) {
        self.reader
            .get_mut()
            .write_all(line)
            .expect("protocol line should write");
        self.reader
            .get_mut()
            .write_all(b"\r\n")
            .expect("protocol newline should write");
        self.reader
            .get_mut()
            .flush()
            .expect("protocol line should flush");
    }

    pub fn write_json_line(&mut self, line: &str) {
        self.write_raw_line(line.as_bytes());
    }

    pub fn write_message(&mut self, message: &ProtocolMessage) {
        let line = encode_message_line(message).expect("protocol message should encode");
        self.write_json_line(&line);
    }

    pub fn hello(&mut self, username: &str, room: &str) {
        self.write_message(&ProtocolMessage::hello_basic(username, room, "1.7.5"));
        self.read_until_kind("Hello");
    }

    pub fn read_message(&mut self) -> Option<ProtocolMessage> {
        let mut line = String::new();
        let read = self
            .reader
            .read_line(&mut line)
            .expect("server response should read");
        if read == 0 {
            return None;
        }
        Some(decode_message_line(line.trim_end()).expect("server response should decode"))
    }

    pub fn read_until_kind(&mut self, kind: &str) -> ProtocolMessage {
        self.read_until(|message| message.kind() == kind)
    }

    pub fn read_until(&mut self, predicate: impl Fn(&ProtocolMessage) -> bool) -> ProtocolMessage {
        for _ in 0..64 {
            let message = self
                .read_message()
                .expect("server should not close before expected response");
            if predicate(&message) {
                return message;
            }
        }
        panic!("server did not produce expected response before read limit");
    }

    pub fn upgrade_to_tls(mut self) -> TlsProtocolClient {
        self.write_message(&ProtocolMessage::tls(TlsPayload::new("send")));
        let response = self.read_until_kind("TLS");
        let ProtocolMessage::Tls(payload) = response else {
            unreachable!("read_until_kind returned non-TLS response");
        };
        assert_eq!(payload.tls.start_tls, "true");

        let stream = self.reader.into_inner();
        let connection = ClientConnection::new(
            tls_client_config(),
            ServerName::try_from("localhost").expect("test server name should parse"),
        )
        .expect("TLS client connection should initialize");
        TlsProtocolClient {
            stream: StreamOwned::new(connection, stream),
        }
    }
}

pub struct TlsProtocolClient {
    stream: StreamOwned<ClientConnection, TcpStream>,
}

impl TlsProtocolClient {
    pub fn write_message(&mut self, message: &ProtocolMessage) {
        let line = encode_message_line(message).expect("protocol message should encode");
        self.stream
            .write_all(line.as_bytes())
            .expect("TLS protocol line should write");
        self.stream
            .write_all(b"\r\n")
            .expect("TLS protocol newline should write");
        self.stream.flush().expect("TLS protocol line should flush");
    }

    pub fn read_message(&mut self) -> Option<ProtocolMessage> {
        let line = read_tls_line(&mut self.stream).expect("TLS server response should read");
        line.map(|line| decode_message_line(&line).expect("TLS server response should decode"))
    }

    pub fn read_until_kind(&mut self, kind: &str) -> ProtocolMessage {
        for _ in 0..64 {
            let message = self
                .read_message()
                .expect("TLS server should not close before expected response");
            if message.kind() == kind {
                return message;
            }
        }
        panic!("TLS server did not produce {kind} response before read limit");
    }
}

fn read_tls_line(
    stream: &mut StreamOwned<ClientConnection, TcpStream>,
) -> io::Result<Option<String>> {
    let mut raw = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) if raw.is_empty() => return Ok(None),
            Ok(0) => break,
            Ok(_) => {
                raw.push(byte[0]);
                if byte[0] == b'\n' {
                    break;
                }
            }
            Err(error) => return Err(error),
        }
    }
    while raw
        .last()
        .is_some_and(|byte| *byte == b'\n' || *byte == b'\r')
    {
        raw.pop();
    }
    Ok(Some(String::from_utf8_lossy(&raw).to_string()))
}

pub fn hello_message(username: &str, room: &str) -> ProtocolMessage {
    ProtocolMessage::hello(HelloPayload::new(username, room, "1.7.5"))
}

pub fn set_file_message(name: &str) -> ProtocolMessage {
    ProtocolMessage::set(
        SetPayload::new().with_file(
            FilePayload::new()
                .with_name(name)
                .with_duration(12.0)
                .with_size(json!(1234)),
        ),
    )
}

pub fn set_ready_message(ready: bool) -> ProtocolMessage {
    ProtocolMessage::set(
        SetPayload::new().with_ready(ReadyPayload::new(ready).with_manually_initiated(true)),
    )
}

pub fn set_playlist_message(files: &[&str]) -> ProtocolMessage {
    ProtocolMessage::set(SetPayload::new().with_playlist_change(
        syncplay_protocol::PlaylistChangePayload::new(files.iter().copied()),
    ))
}

pub fn set_playlist_index_message(index: usize) -> ProtocolMessage {
    ProtocolMessage::set(
        SetPayload::new()
            .with_playlist_index(syncplay_protocol::PlaylistIndexPayload::new(index as i64)),
    )
}

pub fn state_message(position: f64, paused: bool) -> ProtocolMessage {
    ProtocolMessage::state(
        StatePayload::new().with_playstate(
            PlaystatePayload::new()
                .with_position(position)
                .with_paused(paused)
                .with_do_seek(false),
        ),
    )
}

pub fn chat_message(text: &str) -> ProtocolMessage {
    ProtocolMessage::chat(ChatPayload::text(text))
}

pub fn expect_list_rooms(
    message: ProtocolMessage,
) -> std::collections::BTreeMap<
    String,
    std::collections::BTreeMap<String, syncplay_protocol::ListUserEntry>,
> {
    let ProtocolMessage::List(payload) = message else {
        panic!("expected List response, got {}", message.kind());
    };
    match payload.list {
        ListPayload::Rooms(rooms) => rooms,
        ListPayload::Request(_) => panic!("expected List room snapshot, got request shape"),
    }
}

pub fn write_valid_tls_bundle(path: &Path) {
    fs::create_dir_all(path).expect("TLS fixture directory should be creatable");
    fs::write(path.join("privkey.pem"), TEST_TLS_PRIVATE_KEY_PEM)
        .expect("valid private key fixture should write");
    fs::write(path.join("cert.pem"), TEST_TLS_CERT_PEM)
        .expect("valid certificate fixture should write");
    fs::write(path.join("chain.pem"), TEST_TLS_CHAIN_PEM)
        .expect("valid chain fixture should write");
}

pub fn tls_client_config() -> Arc<ClientConfig> {
    let mut cert_reader = io::BufReader::new(TEST_TLS_CERT_PEM.as_bytes());
    let certs = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .expect("test certificate fixture should parse");
    let mut roots = RootCertStore::empty();
    for cert in certs {
        roots
            .add(cert)
            .expect("test certificate should be addable to root store");
    }
    Arc::new(
        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

pub struct PythonPeer {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    status_rx: mpsc::Receiver<String>,
    pub stdout: CapturedOutput,
    pub stderr: CapturedOutput,
}

const PYTHON_PEER_COMMAND_TIMEOUT: Duration = Duration::from_secs(6);
const PYTHON_PEER_OBSERVATION_TIMEOUT: Duration = Duration::from_secs(15);

impl PythonPeer {
    pub fn spawn_or_skip(
        host: &str,
        port: u16,
        username: &str,
        room: &str,
        password: Option<&str>,
    ) -> Option<Self> {
        Self::spawn_with_options(host, port, username, room, password, None)
    }

    pub fn spawn_tls_or_skip(
        host: &str,
        port: u16,
        username: &str,
        room: &str,
        tls_ca_file: &Path,
    ) -> Option<Self> {
        Self::spawn_with_options(host, port, username, room, None, Some(tls_ca_file))
    }

    fn spawn_with_options(
        host: &str,
        port: u16,
        username: &str,
        room: &str,
        password: Option<&str>,
        tls_ca_file: Option<&Path>,
    ) -> Option<Self> {
        let Some(legacy_root) = legacy_syncplay_root() else {
            if strict_release_required() {
                panic!(
                    "strict server release verification requires legacy Syncplay checkout at SYNCPLAY_LEGACY_ROOT or .interop-cache/syncplay-legacy"
                );
            }
            eprintln!("legacy Python client test skipped; missing legacy Syncplay checkout");
            return None;
        };
        let probe = python_live_peer_probe_script();
        if !probe.is_file() {
            if strict_release_required() {
                panic!(
                    "strict server release verification requires {}",
                    probe.display()
                );
            }
            eprintln!(
                "legacy Python client test skipped; missing {}",
                probe.display()
            );
            return None;
        }

        if let Err(reason) = python_live_peer_prerequisites() {
            if strict_release_required() {
                panic!(
                    "strict server release verification requires Python live peer support: {reason}"
                );
            }
            eprintln!("legacy Python client test skipped; {reason}");
            return None;
        }

        let mut command = Command::new(python_bin_from_env());
        command
            .arg(&probe)
            .arg("--host")
            .arg(host)
            .arg("--port")
            .arg(port.to_string())
            .arg("--name")
            .arg(username)
            .arg("--room")
            .arg(room)
            .arg("--timeout-seconds")
            .arg("5")
            .current_dir(&legacy_root)
            .env("PYTHONUNBUFFERED", "1")
            .env("SYNCPLAY_LEGACY_ROOT", &legacy_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(password) = password {
            command.arg("--password").arg(password);
        }
        if let Some(tls_ca_file) = tls_ca_file {
            command
                .arg("--tls")
                .arg("--tls-ca-file")
                .arg(tls_ca_file)
                .arg("--tls-hostname")
                .arg("localhost");
        }
        let mut child = command
            .spawn()
            .unwrap_or_else(|error| panic!("legacy Python live peer should spawn: {error}"));
        let stdin = child.stdin.take().expect("peer stdin should be captured");
        let stdout = CapturedOutput::default();
        let stderr = CapturedOutput::default();
        let (status_tx, status_rx) = mpsc::channel();
        capture_output_lines(
            child.stdout.take().expect("peer stdout should be captured"),
            stdout.clone(),
            Some(status_tx),
        );
        capture_output_lines(
            child.stderr.take().expect("peer stderr should be captured"),
            stderr.clone(),
            None,
        );
        let mut peer = Self {
            child: Some(child),
            stdin: Some(stdin),
            status_rx,
            stdout,
            stderr,
        };
        peer.wait_for_status("connected");
        Some(peer)
    }

    pub fn command(&mut self, value: Value) {
        let stdin = self
            .stdin
            .as_mut()
            .expect("peer stdin should remain open while peer is active");
        let mut bytes = serde_json::to_vec(&value).expect("peer command should encode");
        bytes.push(b'\n');
        stdin
            .write_all(&bytes)
            .expect("peer command should write to stdin");
        stdin.flush().expect("peer command should flush");
    }

    pub fn wait_for_status(&mut self, expected: &str) -> Value {
        self.wait_for_status_with_timeout(expected, PYTHON_PEER_COMMAND_TIMEOUT)
    }

    fn wait_for_observation_status(&mut self, expected: &str) -> Value {
        self.wait_for_status_with_timeout(
            expected,
            PYTHON_PEER_OBSERVATION_TIMEOUT + PYTHON_PEER_COMMAND_TIMEOUT,
        )
    }

    fn wait_for_status_with_timeout(&mut self, expected: &str, timeout: Duration) -> Value {
        let status_line = self.status_rx.recv_timeout(timeout).unwrap_or_else(|_| {
            panic!(
                "legacy Python peer timed out waiting for status {expected:?}; stdout='{}' stderr='{}'",
                self.stdout.text(),
                self.stderr.text()
            )
        });
        let parsed: Value =
            serde_json::from_str(&status_line).expect("peer status line should be valid JSON");
        let actual = parsed.get("status").and_then(Value::as_str);
        assert_eq!(
            actual,
            Some(expected),
            "legacy Python peer reported unexpected status: {status_line}"
        );
        parsed
    }

    pub fn snapshot(&mut self) -> Value {
        self.command(json!({"command": "snapshot"}));
        self.wait_for_status("snapshot")
    }

    pub fn set_ready(&mut self, ready: bool) {
        self.command(json!({"command": "set_ready", "ready": ready}));
        self.wait_for_status("ready-command-sent");
    }

    pub fn send_chat_message(&mut self, message: &str) {
        self.command(json!({"command": "send_chat_message", "message": message}));
        self.wait_for_status("chat-command-sent");
    }

    pub fn set_room(&mut self, room: &str) {
        self.command(json!({"command": "set_room", "room": room}));
        self.wait_for_status("room-command-sent");
    }

    pub fn set_file(&mut self, name: &str) {
        self.command(json!({
            "command": "set_file",
            "file": {"name": name, "duration": 12.0, "size": 1234}
        }));
        self.wait_for_status("file-command-sent");
    }

    pub fn set_playlist(&mut self, files: &[&str]) {
        self.command(json!({"command": "set_playlist", "files": files}));
        self.wait_for_status("playlist-command-sent");
    }

    pub fn set_playlist_index(&mut self, index: usize) {
        self.command(json!({"command": "set_playlist_index", "index": index}));
        self.wait_for_status("playlist-index-command-sent");
    }

    pub fn request_controlled_room(&mut self, room: &str, password: &str) {
        self.command(json!({
            "command": "request_controlled_room",
            "room": room,
            "password": password
        }));
        self.wait_for_status("controlled-room-command-sent");
    }

    pub fn wait_for_user_ready(&mut self, username: &str, ready: bool) {
        self.command(json!({
            "command": "wait_for_user_ready",
            "username": username,
            "ready": ready,
            "timeoutSeconds": PYTHON_PEER_OBSERVATION_TIMEOUT.as_secs_f64()
        }));
        self.wait_for_observation_status("user-ready");
    }

    pub fn wait_for_user_room(&mut self, username: &str, room: &str) {
        self.command(json!({
            "command": "wait_for_user_room",
            "username": username,
            "room": room,
            "timeoutSeconds": PYTHON_PEER_OBSERVATION_TIMEOUT.as_secs_f64()
        }));
        self.wait_for_observation_status("user-room");
    }

    pub fn wait_for_user_file_name(&mut self, username: &str, file_name: &str) {
        self.command(json!({
            "command": "wait_for_user_file_name",
            "username": username,
            "fileName": file_name,
            "timeoutSeconds": PYTHON_PEER_OBSERVATION_TIMEOUT.as_secs_f64()
        }));
        self.wait_for_observation_status("user-file");
    }

    pub fn wait_for_chat_message(&mut self, username: &str, message: &str) {
        self.command(json!({
            "command": "wait_for_chat_message",
            "username": username,
            "message": message,
            "timeoutSeconds": PYTHON_PEER_OBSERVATION_TIMEOUT.as_secs_f64()
        }));
        self.wait_for_observation_status("chat-message");
    }

    pub fn wait_for_playlist(&mut self, files: &[&str]) {
        self.command(json!({
            "command": "wait_for_playlist",
            "files": files,
            "timeoutSeconds": PYTHON_PEER_OBSERVATION_TIMEOUT.as_secs_f64()
        }));
        self.wait_for_observation_status("playlist");
    }

    pub fn wait_for_playlist_index(&mut self, index: usize) {
        self.command(json!({
            "command": "wait_for_playlist_index",
            "index": index,
            "timeoutSeconds": PYTHON_PEER_OBSERVATION_TIMEOUT.as_secs_f64()
        }));
        self.wait_for_observation_status("playlist-index");
    }

    pub fn wait_for_local_controller(&mut self, controller: bool) {
        self.command(json!({
            "command": "wait_for_local_controller",
            "controller": controller,
            "timeoutSeconds": PYTHON_PEER_OBSERVATION_TIMEOUT.as_secs_f64()
        }));
        self.wait_for_observation_status("local-controller");
    }
}

impl Drop for PythonPeer {
    fn drop(&mut self) {
        drop(self.stdin.take());
        if let Some(mut child) = self.child.take() {
            let deadline = Instant::now() + Duration::from_secs(1);
            while Instant::now() < deadline {
                if child.try_wait().ok().flatten().is_some() {
                    return;
                }
                thread::sleep(Duration::from_millis(50));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
