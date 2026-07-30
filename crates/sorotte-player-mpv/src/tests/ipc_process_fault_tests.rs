use crate::ipc::{MpvIpcConnectionEvent, MpvJsonIpcClient};
use serde_json::{Value, json};
use std::{
    ffi::OsStr,
    fs,
    io::{self, Read as _, Write as _},
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Path, PathBuf},
    process::{Child, ExitStatus, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread::JoinHandle,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{FlushFileBuffers, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile},
    System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT,
    },
};

const FIXTURE_TEST: &str =
    "tests::ipc_process_fault_tests::mpv_external_process_fixture_entrypoint";
const FIXTURE_ROLE_ENV: &str = "SOROTTE_TEST_MPV_PROCESS_FIXTURE_ROLE";
const FIXTURE_PIPE_ENV: &str = "SOROTTE_TEST_MPV_PROCESS_FIXTURE_PIPE";
const FIXTURE_ROOT_ENV: &str = "SOROTTE_TEST_MPV_PROCESS_FIXTURE_ROOT";
const LARGE_STDIO_BYTES: usize = 256 * 1024;
const LARGE_IPC_VALUE_BYTES: usize = 512 * 1024;
const FIXTURE_READY_BUDGET: Duration = Duration::from_secs(5);
const COMMAND_COMPLETION_BUDGET: Duration = Duration::from_secs(5);
static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(1);

struct ProcessPipeServer {
    handle: OwnedHandle,
}

impl ProcessPipeServer {
    fn create(pipe_name: &str) -> Self {
        let wide_pipe_name = OsStr::new(pipe_name)
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        // SAFETY: the pipe name is NUL-terminated, buffer sizes are bounded
        // test constants, and no optional security attributes are supplied.
        let raw_handle = unsafe {
            CreateNamedPipeW(
                wide_pipe_name.as_ptr(),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                1,
                64 * 1024,
                64 * 1024,
                0,
                std::ptr::null(),
            )
        };
        assert_ne!(
            raw_handle,
            INVALID_HANDLE_VALUE,
            "external fixture failed creating named pipe {pipe_name}: {}",
            io::Error::last_os_error()
        );
        // SAFETY: ownership of the valid handle transfers exactly once.
        let handle = unsafe { OwnedHandle::from_raw_handle(raw_handle as _) };
        Self { handle }
    }

    fn raw_handle(&self) -> HANDLE {
        self.handle.as_raw_handle() as HANDLE
    }

    fn connect(&self) {
        // SAFETY: this is a live synchronous server handle. The production
        // client opening the pipe releases this blocking call.
        let connected = unsafe { ConnectNamedPipe(self.raw_handle(), std::ptr::null_mut()) };
        if connected == 0 {
            let error = io::Error::last_os_error();
            assert_eq!(
                error.raw_os_error(),
                Some(ERROR_PIPE_CONNECTED as i32),
                "external fixture failed connecting named pipe: {error}"
            );
        }
    }

    fn read_request(&self) -> Value {
        let mut bytes = Vec::new();
        loop {
            let mut chunk = [0_u8; 8 * 1024];
            let mut bytes_read = 0_u32;
            // SAFETY: the synchronous pipe and output buffers remain valid for
            // the duration of this call.
            let succeeded = unsafe {
                ReadFile(
                    self.raw_handle(),
                    chunk.as_mut_ptr(),
                    chunk.len() as u32,
                    &mut bytes_read,
                    std::ptr::null_mut(),
                )
            };
            assert_ne!(
                succeeded,
                0,
                "external fixture failed reading request: {}",
                io::Error::last_os_error()
            );
            assert_ne!(bytes_read, 0, "production client closed before its request");
            bytes.extend_from_slice(&chunk[..bytes_read as usize]);
            if bytes.contains(&b'\n') {
                let newline = bytes
                    .iter()
                    .position(|byte| *byte == b'\n')
                    .expect("newline presence was checked");
                assert_eq!(
                    newline + 1,
                    bytes.len(),
                    "fixture expects exactly one request frame"
                );
                return serde_json::from_slice(&bytes[..newline])
                    .expect("production request must be valid JSON");
            }
        }
    }

    fn write_all_fragmented(&self, bytes: &[u8], max_chunk_bytes: usize) {
        assert!(max_chunk_bytes > 0);
        let mut offset = 0usize;
        while offset < bytes.len() {
            let end = (offset + max_chunk_bytes).min(bytes.len());
            let chunk = &bytes[offset..end];
            let mut bytes_written = 0_u32;
            // SAFETY: the synchronous pipe and input buffer remain valid for
            // the duration of this call.
            let succeeded = unsafe {
                WriteFile(
                    self.raw_handle(),
                    chunk.as_ptr(),
                    u32::try_from(chunk.len()).expect("fixture chunk should fit in u32"),
                    &mut bytes_written,
                    std::ptr::null_mut(),
                )
            };
            assert_ne!(
                succeeded,
                0,
                "external fixture failed writing response: {}",
                io::Error::last_os_error()
            );
            assert_ne!(bytes_written, 0, "external fixture write made no progress");
            offset += bytes_written as usize;
        }
    }

    fn flush(&self) {
        // SAFETY: the live synchronous pipe is being read concurrently by the
        // production worker.
        let succeeded = unsafe { FlushFileBuffers(self.raw_handle()) };
        assert_ne!(
            succeeded,
            0,
            "external fixture failed flushing pipe: {}",
            io::Error::last_os_error()
        );
    }
}

fn request_id(request: &Value) -> u64 {
    request
        .get("request_id")
        .and_then(Value::as_u64)
        .expect("production request must carry request_id")
}

fn write_large_stdio() {
    let stdout_chunk = [b'o'; 16 * 1024];
    let stderr_chunk = [b'e'; 16 * 1024];
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    for _ in 0..(LARGE_STDIO_BYTES / stdout_chunk.len()) {
        stdout
            .write_all(&stdout_chunk)
            .expect("fixture stdout should remain drained");
        stderr
            .write_all(&stderr_chunk)
            .expect("fixture stderr should remain drained");
    }
    stdout.flush().expect("fixture stdout should flush");
    stderr.flush().expect("fixture stderr should flush");
}

fn fixture_marker(root: &Path, name: &str) -> PathBuf {
    root.join(name)
}

#[test]
fn mpv_external_process_fixture_entrypoint() {
    let Some(role) = std::env::var_os(FIXTURE_ROLE_ENV) else {
        return;
    };
    let pipe_name = std::env::var(FIXTURE_PIPE_ENV).expect("fixture pipe name must be provided");
    let root =
        PathBuf::from(std::env::var_os(FIXTURE_ROOT_ENV).expect("fixture root must be provided"));
    let server = ProcessPipeServer::create(&pipe_name);
    fs::write(fixture_marker(&root, "ready"), b"ready")
        .expect("fixture should publish pipe readiness");
    server.connect();

    match role.to_string_lossy().as_ref() {
        "large-traffic" => {
            write_large_stdio();
            let request = server.read_request();
            let event = json!({
                "event": "property-change",
                "name": "external-large-payload",
                "data": "x".repeat(LARGE_IPC_VALUE_BYTES),
            });
            let response = json!({
                "request_id": request_id(&request),
                "error": "success",
                "data": "external-large-ok",
            });
            let wire = format!("{event}\n{response}\n");
            server.write_all_fragmented(wire.as_bytes(), 4_093);
            server.flush();
        }
        "partial-response" => {
            let request = server.read_request();
            let prefix = format!(r#"{{"request_id":{},"error":"suc"#, request_id(&request));
            server.write_all_fragmented(prefix.as_bytes(), 3);
            server.flush();
        }
        "early-exit" => std::process::exit(23),
        "hang-after-request" => {
            server.read_request();
            fs::write(fixture_marker(&root, "request-seen"), b"request-seen")
                .expect("fixture should publish the observed request");
            loop {
                std::thread::park();
            }
        }
        unexpected => panic!("unknown external mpv fixture role {unexpected:?}"),
    }
}

struct ExternalMpvFixture {
    root: PathBuf,
    executable: PathBuf,
    pipe_name: String,
    child: Option<Child>,
    stdout_reader: Option<JoinHandle<Vec<u8>>>,
    stderr_reader: Option<JoinHandle<Vec<u8>>>,
}

struct ExternalMpvOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl ExternalMpvFixture {
    fn spawn(role: &str) -> Self {
        let fixture_id = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "sorotte-mpv-external-process-{}-{fixture_id}-{role}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("external mpv fixture root should be created");
        let source = std::env::current_exe().expect("player-mpv test image should exist");
        let extension = source
            .extension()
            .map(|extension| format!(".{}", extension.to_string_lossy()))
            .unwrap_or_default();
        let executable = root.join(format!("fake-mpv{extension}"));
        fs::hard_link(&source, &executable)
            .or_else(|_| fs::copy(&source, &executable).map(|_| ()))
            .expect("external mpv fixture image should be materialized");
        let pipe_name = format!(
            r"\\.\pipe\sorotte-mpv-external-{}-{fixture_id}",
            std::process::id()
        );

        let mut child = std::process::Command::new(&executable)
            .args(["--exact", FIXTURE_TEST, "--nocapture"])
            .env(FIXTURE_ROLE_ENV, role)
            .env(FIXTURE_PIPE_ENV, &pipe_name)
            .env(FIXTURE_ROOT_ENV, &root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("external mpv fixture should spawn");
        let stdout = child.stdout.take().expect("fixture stdout should be piped");
        let stderr = child.stderr.take().expect("fixture stderr should be piped");
        let stdout_reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let mut stdout = stdout;
            stdout
                .read_to_end(&mut bytes)
                .expect("fixture stdout should drain");
            bytes
        });
        let stderr_reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let mut stderr = stderr;
            stderr
                .read_to_end(&mut bytes)
                .expect("fixture stderr should drain");
            bytes
        });
        let mut fixture = Self {
            root,
            executable,
            pipe_name,
            child: Some(child),
            stdout_reader: Some(stdout_reader),
            stderr_reader: Some(stderr_reader),
        };
        fixture.wait_for_marker("ready");
        fixture
    }

    fn pipe_name(&self) -> &str {
        &self.pipe_name
    }

    fn wait_for_marker(&mut self, name: &str) {
        let marker = fixture_marker(&self.root, name);
        let deadline = Instant::now() + FIXTURE_READY_BUDGET;
        while !marker.exists() {
            if let Some(status) = self
                .child
                .as_mut()
                .expect("fixture child should remain present")
                .try_wait()
                .expect("fixture child state should be observable")
            {
                panic!("external mpv fixture exited before marker {name}: {status}");
            }
            assert!(
                Instant::now() < deadline,
                "external mpv fixture did not publish marker {name}"
            );
            std::thread::yield_now();
        }
    }

    fn finish(mut self, terminate: bool) -> ExternalMpvOutput {
        let mut child = self
            .child
            .take()
            .expect("fixture child should remain present");
        if terminate {
            let _ = child.kill();
        }
        let status = child.wait().expect("external mpv fixture should be reaped");
        let stdout = self
            .stdout_reader
            .take()
            .expect("fixture stdout reader should remain present")
            .join()
            .expect("fixture stdout reader should join");
        let stderr = self
            .stderr_reader
            .take()
            .expect("fixture stderr reader should remain present")
            .join()
            .expect("fixture stderr reader should join");
        fs::remove_file(&self.executable)
            .expect("reaped external mpv fixture must release its executable image");
        assert!(!self.executable.exists());
        ExternalMpvOutput {
            status,
            stdout,
            stderr,
        }
    }
}

impl Drop for ExternalMpvFixture {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(reader) = self.stdout_reader.take() {
            let _ = reader.join();
        }
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
        let _ = fs::remove_file(&self.executable);
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn connect_client(pipe_name: &str, timeout: Duration) -> MpvJsonIpcClient {
    MpvJsonIpcClient::connect_with_command_timeout(Path::new(pipe_name), timeout)
        .unwrap_or_else(|error| panic!("production client should open {pipe_name}: {error}"))
}

fn take_initial_connected(client: &mut MpvJsonIpcClient) {
    assert!(matches!(
        client.take_connection_events().as_slice(),
        [MpvIpcConnectionEvent::Connected { generation }]
            if *generation == client.generation()
    ));
}

fn take_one_terminal_disconnect(client: &mut MpvJsonIpcClient) -> Vec<MpvIpcConnectionEvent> {
    assert!(!client.is_healthy());
    let events = client.take_connection_events();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, MpvIpcConnectionEvent::CommandFailed { .. }))
            .count(),
        1,
        "{events:?}"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, MpvIpcConnectionEvent::Disconnected { .. }))
            .count(),
        1,
        "{events:?}"
    );
    events
}

#[test]
fn external_mpv_large_stdio_and_ipc_frame_do_not_block_command_completion() {
    let fixture = ExternalMpvFixture::spawn("large-traffic");
    let mut client = connect_client(fixture.pipe_name(), Duration::from_secs(3));
    take_initial_connected(&mut client);
    let started = Instant::now();
    assert_eq!(
        client
            .get_property("external-large")
            .expect("large external response should complete"),
        Some(json!("external-large-ok"))
    );
    assert!(
        started.elapsed() < COMMAND_COMPLETION_BUDGET,
        "large process traffic exceeded the bounded command budget"
    );
    let events = client.take_pending_events();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0]
            .pointer("/data")
            .and_then(Value::as_str)
            .map(str::len),
        Some(LARGE_IPC_VALUE_BYTES)
    );
    assert!(client.is_healthy());
    drop(client);

    let output = fixture.finish(false);
    assert!(output.status.success(), "{:?}", output.status);
    assert!(output.stdout.len() >= LARGE_STDIO_BYTES);
    assert!(output.stderr.len() >= LARGE_STDIO_BYTES);
}

#[test]
fn external_mpv_partial_response_and_early_exit_fail_terminally_and_boundedly() {
    for (role, expected_status) in [("partial-response", Some(0)), ("early-exit", Some(23))] {
        let fixture = ExternalMpvFixture::spawn(role);
        let mut client = connect_client(fixture.pipe_name(), Duration::from_millis(500));
        take_initial_connected(&mut client);
        let started = Instant::now();
        let error = client
            .get_property("external-terminal")
            .expect_err("external process termination must fail the command");
        assert!(
            started.elapsed() < COMMAND_COMPLETION_BUDGET,
            "{role} exceeded the bounded command budget"
        );
        assert!(
            error.contains("failed to read")
                || error.contains("unexpected EOF")
                || error.contains("invalid mpv IPC JSON"),
            "{role}: {error}"
        );
        take_one_terminal_disconnect(&mut client);
        drop(client);

        let output = fixture.finish(false);
        assert_eq!(output.status.code(), expected_status, "{role}");
    }
}

#[test]
fn external_mpv_hang_times_out_then_kill_reap_releases_process_and_pipe_handles() {
    let mut fixture = ExternalMpvFixture::spawn("hang-after-request");
    let pipe_name = fixture.pipe_name().to_owned();
    let command_timeout = Duration::from_millis(120);
    let mut client = connect_client(&pipe_name, command_timeout);
    take_initial_connected(&mut client);
    let started = Instant::now();
    let error = client
        .get_property("external-hang")
        .expect_err("hung external mpv must reach the command deadline");
    let elapsed = started.elapsed();
    assert!(error.contains("timed out"), "{error}");
    assert!(
        elapsed >= command_timeout.saturating_sub(Duration::from_millis(20)),
        "fixture did not causally withhold its response: {elapsed:?}"
    );
    assert!(
        elapsed < COMMAND_COMPLETION_BUDGET,
        "hung external command exceeded its cancellation budget"
    );
    fixture.wait_for_marker("request-seen");
    let terminal_events = take_one_terminal_disconnect(&mut client);
    assert_eq!(
        terminal_events
            .iter()
            .filter(|event| matches!(event, MpvIpcConnectionEvent::TimedOut { .. }))
            .count(),
        1,
        "{terminal_events:?}"
    );
    assert!(
        client.take_connection_events().is_empty(),
        "the terminal event batch must be delivered at most once"
    );
    drop(client);

    let output = fixture.finish(true);
    assert!(!output.status.success());
    drop(ProcessPipeServer::create(&pipe_name));
}
