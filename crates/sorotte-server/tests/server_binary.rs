use std::{
    fs,
    io::{BufRead, BufReader, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde_json::Value;
use sorotte_protocol::{
    ListPayload, ProtocolMessage, decode_message_line, extract_hello_from_message,
};

struct ServerChild {
    child: Option<Child>,
}

impl ServerChild {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    fn child_mut(&mut self) -> &mut Child {
        self.child
            .as_mut()
            .expect("server child should be present while guard is alive")
    }
}

impl Drop for ServerChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

struct PeerChild {
    child: Option<Child>,
}

impl PeerChild {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }
}

impl Drop for PeerChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn reserve_local_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("ephemeral port should bind");
    listener
        .local_addr()
        .expect("ephemeral listener should have local address")
        .port()
}

fn temporary_motd_file() -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after epoch")
        .as_nanos();
    path.push(format!("sorotte-server-motd-{suffix}.txt"));
    path
}

fn wait_for_server(port: u16, child: &mut Child) -> TcpStream {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    for _ in 0..60 {
        if let Some(status) = child
            .try_wait()
            .expect("server process status should be inspectable")
        {
            panic!("sorotte-server exited before accepting connections: {status}");
        }
        if let Ok(stream) = TcpStream::connect(address) {
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .expect("read timeout should be configurable");
            stream
                .set_write_timeout(Some(Duration::from_secs(3)))
                .expect("write timeout should be configurable");
            return stream;
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!("sorotte-server did not accept a connection on 127.0.0.1:{port}");
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("server crate should live under workspace/crates")
        .to_path_buf()
}

fn legacy_syncplay_root() -> PathBuf {
    if let Some(path) = std::env::var_os("SYNCPLAY_LEGACY_ROOT").filter(|value| !value.is_empty()) {
        return PathBuf::from(path);
    }
    let repo_local = repository_root()
        .join(".interop-cache")
        .join("syncplay-legacy");
    if repo_local.join("syncplayServer.py").is_file() {
        return repo_local;
    }
    repository_root()
        .parent()
        .expect("workspace should have a parent containing the Python checkout")
        .join("syncplay")
}

fn python_live_peer_probe_script() -> PathBuf {
    repository_root()
        .join("crates")
        .join("sorotte-compat")
        .join("scripts")
        .join("python_live_peer_probe.py")
}

fn python_bin_from_env() -> std::ffi::OsString {
    std::env::var_os("SYNCPLAY_PYTHON_BIN").unwrap_or_else(|| "python".into())
}

fn capture_output_lines<R: std::io::Read + Send + 'static>(
    reader: R,
    sender: Option<mpsc::Sender<String>>,
    captured: Arc<Mutex<Vec<String>>>,
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
            captured
                .lock()
                .expect("captured process output lock should not be poisoned")
                .push(trimmed.to_owned());
            if let Some(sender) = sender.as_ref() {
                let _ = sender.send(trimmed.to_owned());
            }
        }
    });
}

fn captured_output(captured: &Arc<Mutex<Vec<String>>>) -> String {
    captured
        .lock()
        .expect("captured process output lock should not be poisoned")
        .join("\n")
}

fn read_protocol_message(reader: &mut BufReader<TcpStream>) -> ProtocolMessage {
    let mut line = String::new();
    let read = reader
        .read_line(&mut line)
        .expect("server response line should read");
    assert!(read > 0, "server should not close before responding");
    decode_message_line(line.trim_end()).expect("server response should decode")
}

#[test]
fn sorotte_server_binary_handles_legacy_password_motd_hello_and_list() {
    let port = reserve_local_port();
    let motd_file = temporary_motd_file();
    fs::write(
        &motd_file,
        "\u{feff}Server=$version IP=$userIp User=$username Room=$room",
    )
    .expect("temporary MOTD file should write");

    let mut child = ServerChild::new(
        Command::new(env!("CARGO_BIN_EXE_sorotte-server"))
            .args([
                "--port",
                &port.to_string(),
                "--password",
                "secret",
                "--motd-file",
                motd_file
                    .to_str()
                    .expect("temporary MOTD path should be UTF-8"),
                "--interface-ipv4",
                "127.0.0.1",
                "--interface-ipv6",
                "::1",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("sorotte-server binary should spawn"),
    );

    let stream = wait_for_server(port, child.child_mut());
    let mut reader = BufReader::new(stream);
    reader
        .get_mut()
        .write_all(
            br#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","password":"5ebe2294ecd0e0f08eab7690d2a6ee69"}}"#,
        )
        .expect("hello should write");
    reader
        .get_mut()
        .write_all(b"\r\n")
        .expect("hello newline should write");
    reader.get_mut().flush().expect("hello should flush");

    let mut saw_hello = false;
    for _ in 0..4 {
        let message = read_protocol_message(&mut reader);
        if let ProtocolMessage::Hello(_) = message {
            let hello = extract_hello_from_message(message).expect("hello should extract");
            assert_eq!(hello.username, "alice");
            assert_eq!(hello.room.name, "room1");
            assert_eq!(
                hello.extra.get("motd").and_then(Value::as_str),
                Some("Server=1.7.5 IP=127.0.0.1 User=alice Room=room1")
            );
            saw_hello = true;
            break;
        }
    }
    assert!(saw_hello, "server should send a Hello response");

    reader
        .get_mut()
        .write_all(br#"{"List":null}"#)
        .expect("list request should write");
    reader
        .get_mut()
        .write_all(b"\r\n")
        .expect("list newline should write");
    reader.get_mut().flush().expect("list request should flush");

    let list_message = read_protocol_message(&mut reader);
    let ProtocolMessage::List(list_payload) = list_message else {
        panic!("server should respond to List request");
    };
    match list_payload.list {
        ListPayload::Rooms(rooms) => assert!(rooms.contains_key("room1")),
        ListPayload::Request(_) => panic!("server should send room snapshot, not request shape"),
    }

    fs::remove_file(motd_file).expect("temporary MOTD file should be removable");
}

#[test]
fn sorotte_server_binary_accepts_legacy_python_client_hello() {
    let legacy_root = legacy_syncplay_root();
    if !legacy_root.join("syncplayServer.py").is_file() {
        eprintln!(
            "legacy Python client smoke test skipped; missing {}",
            legacy_root.display()
        );
        return;
    }
    let live_peer_probe = python_live_peer_probe_script();
    if !live_peer_probe.is_file() {
        eprintln!(
            "legacy Python client smoke test skipped; missing {}",
            live_peer_probe.display()
        );
        return;
    }

    let port = reserve_local_port();
    let mut server = ServerChild::new(
        Command::new(env!("CARGO_BIN_EXE_sorotte-server"))
            .args([
                "--port",
                &port.to_string(),
                "--ipv4-only",
                "--interface-ipv4",
                "127.0.0.1",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("sorotte-server binary should spawn"),
    );
    let stream = wait_for_server(port, server.child_mut());
    drop(stream);

    let mut peer_child = Command::new(python_bin_from_env())
        .arg(&live_peer_probe)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("--name")
        .arg("python-client")
        .arg("--room")
        .arg("python-room")
        .arg("--timeout-seconds")
        .arg("3")
        .current_dir(&legacy_root)
        .env("PYTHONUNBUFFERED", "1")
        .env("SYNCPLAY_LEGACY_ROOT", &legacy_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("legacy Python live peer probe should spawn");

    let stdout = peer_child
        .stdout
        .take()
        .expect("peer stdout should be captured");
    let stderr = peer_child
        .stderr
        .take()
        .expect("peer stderr should be captured");
    let (status_tx, status_rx) = mpsc::channel();
    let stdout_lines = Arc::new(Mutex::new(Vec::new()));
    let stderr_lines = Arc::new(Mutex::new(Vec::new()));
    capture_output_lines(stdout, Some(status_tx), stdout_lines.clone());
    capture_output_lines(stderr, None, stderr_lines.clone());

    let mut peer = PeerChild::new(peer_child);
    let status_line = status_rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap_or_else(|_| {
            panic!(
                "legacy Python live peer did not report connected status; stdout='{}' stderr='{}'",
                captured_output(&stdout_lines),
                captured_output(&stderr_lines)
            )
        });
    let status: Value =
        serde_json::from_str(&status_line).expect("peer status line should be valid JSON");
    assert_eq!(
        status.get("status").and_then(Value::as_str),
        Some("connected")
    );
    assert_eq!(
        status.get("username").and_then(Value::as_str),
        Some("python-client")
    );
    assert_eq!(
        status.get("room").and_then(Value::as_str),
        Some("python-room")
    );

    drop(peer.child.as_mut().and_then(|child| child.stdin.take()));
}
