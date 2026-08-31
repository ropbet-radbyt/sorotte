use super::*;

const NATIVE_SMOKE_ARTIFACT_DIR_ENV: &str = "SOROTTE_GUI_NATIVE_SMOKE_ARTIFACT_DIR";

pub(super) fn capture_native_failure_artifacts<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    scope: &str,
    failure: &str,
) {
    let Some(artifact_directory) = std::env::var_os(NATIVE_SMOKE_ARTIFACT_DIR_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    else {
        return;
    };
    capture_native_failure_artifacts_at(driver, window, &artifact_directory, scope, failure);
}

pub(super) fn capture_native_failure_artifacts_at<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    artifact_directory: &Path,
    scope: &str,
    failure: &str,
) {
    let mut capture_errors = Vec::new();
    if let Err(error) = fs::create_dir_all(artifact_directory) {
        eprintln!(
            "failed to create native-smoke failure artifact directory {}: {error}",
            artifact_directory.display()
        );
        return;
    }

    let screenshot_path = artifact_directory.join(format!("failure-{scope}.png"));
    if let Err(error) = driver.capture_window_png(window, &screenshot_path) {
        capture_errors.push(format!("screenshot: {error}"));
    }

    let accessibility_path = artifact_directory.join(format!("failure-{scope}-accessibility.json"));
    match driver.accessibility_nodes(window) {
        Ok(nodes) => {
            let serialized_nodes = nodes
                .iter()
                .enumerate()
                .map(|(index, node)| {
                    let (name, automation_id) = redacted_native_failure_node_identity(node);
                    serde_json::json!({
                        "index": index,
                        "name": name,
                        "automation_id": automation_id,
                        "control_type": node.control_type,
                        "enabled": node.enabled,
                        "focused": node.focused,
                        "offscreen": node.offscreen,
                        "bounds": node.bounds,
                    })
                })
                .collect::<Vec<_>>();
            let safe_failure = redact_native_failure_text(failure);
            let payload = serde_json::json!({
                "schema_version": 1,
                "kind": "sorotte-gui-native-smoke-failure-accessibility",
                "scope": scope,
                "source": "Windows UI Automation / AccessKit",
                "failure": safe_failure,
                "nodes": serialized_nodes,
            });
            match serde_json::to_vec_pretty(&payload) {
                Ok(mut json) => {
                    json.push(b'\n');
                    if let Err(error) = fs::write(&accessibility_path, json) {
                        capture_errors.push(format!("accessibility write: {error}"));
                    }
                }
                Err(error) => capture_errors.push(format!("accessibility serialization: {error}")),
            }
        }
        Err(error) => capture_errors.push(format!("accessibility snapshot: {error}")),
    }

    if !capture_errors.is_empty() {
        let capture_error_path =
            artifact_directory.join(format!("failure-{scope}-capture-errors.txt"));
        let mut payload = capture_errors.join("\n");
        payload.push('\n');
        if let Err(error) = fs::write(&capture_error_path, payload) {
            eprintln!(
                "failed to write native-smoke capture errors {}: {error}",
                capture_error_path.display()
            );
        }
    }
}

fn redact_native_failure_text(value: &str) -> String {
    if sorotte_secret::text_may_contain_credentials(value) {
        sorotte_secret::REDACTED_SECRET.to_owned()
    } else {
        value.to_owned()
    }
}

fn redacted_native_failure_node_identity(node: &NativeAccessibilityNode) -> (String, String) {
    let automation_id_lower = node.automation_id.to_ascii_lowercase();
    let identity_is_secret = ["password", "secret", "credential", "authorization", "token"]
        .iter()
        .any(|marker| automation_id_lower.contains(marker));
    let name = if identity_is_secret || sorotte_secret::text_may_contain_credentials(&node.name) {
        sorotte_secret::REDACTED_SECRET.to_owned()
    } else {
        node.name.clone()
    };
    let automation_id = redact_native_failure_text(&node.automation_id);
    (name, automation_id)
}

fn user_row_wait_error<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    name: &str,
    error: String,
) -> String {
    let snapshot = driver
        .accessible_names(window)
        .map(|names| {
            render_accessible_name_snapshot_for_patterns(
                &names,
                &[
                    name,
                    "Room",
                    "Participants",
                    "No users",
                    "smoke",
                    "bob",
                    "missing",
                    "Inbound",
                    "failed",
                    "pending",
                    "view:",
                ],
            )
        })
        .unwrap_or_else(|_| "unavailable".to_owned());
    format!("{error}; participant snapshot: {snapshot}")
}

pub(super) fn wait_for_main_window_user_row_name<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    name: &str,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let initial_timeout = deadline
        .saturating_duration_since(Instant::now())
        .min(Duration::from_millis(800));
    let initial_result = wait_for_accessible_name(driver, window, name, initial_timeout);
    if initial_result.is_ok() {
        return Ok(());
    }
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return initial_result.map_err(|error| user_row_wait_error(driver, window, name, error));
    }
    if wait_for_accessible_name_with_page_up(
        driver,
        window,
        name,
        MAIN_WINDOW_LOCAL_READY_BUTTON_MAX_PAGE_DOWNS,
        remaining,
    )
    .is_ok()
    {
        return Ok(());
    }
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return initial_result.map_err(|error| user_row_wait_error(driver, window, name, error));
    }
    if wait_for_accessible_name_with_named_control_scroll_up(
        driver,
        window,
        name,
        MAIN_WINDOW_ROOM_BROWSER_NAME,
        NativeControlKind::Any,
        MAIN_WINDOW_LOCAL_READY_BUTTON_MAX_PAGE_DOWNS,
        remaining,
    )
    .is_ok()
    {
        return Ok(());
    }
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return initial_result.map_err(|error| user_row_wait_error(driver, window, name, error));
    }
    if wait_for_accessible_name_with_named_control_scroll_down(
        driver,
        window,
        name,
        MAIN_WINDOW_ROOM_BROWSER_NAME,
        NativeControlKind::Any,
        MAIN_WINDOW_LOCAL_READY_BUTTON_MAX_PAGE_DOWNS,
        remaining,
    )
    .is_ok()
    {
        return Ok(());
    }
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return initial_result.map_err(|error| user_row_wait_error(driver, window, name, error));
    }
    wait_for_accessible_name_with_page_down(
        driver,
        window,
        name,
        MAIN_WINDOW_LOCAL_READY_BUTTON_MAX_PAGE_DOWNS,
        remaining,
    )
    .map(|_| ())
    .map_err(|error| user_row_wait_error(driver, window, name, error))
}

pub(super) fn navigate_to_room_surface<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    timeout: Duration,
) -> Result<(), String> {
    navigate_to_view_with_wait(
        driver,
        window,
        ROOM_SURFACE_AUTOMATION_ID,
        "view: room",
        timeout,
    )
}

pub(super) fn wait_for_room_browser_visible<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    timeout: Duration,
) -> Result<(), String> {
    navigate_to_room_surface(driver, window, timeout)?;
    wait_for_any_accessible_name(
        driver,
        window,
        &[
            MAIN_WINDOW_ROOM_BROWSER_NAME,
            "Participants",
            "Playlist",
            "view: room",
        ],
        timeout,
    )
    .map(|_| ())
}

pub(super) fn wait_for_accessible_name_prefix<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    prefix: &str,
    timeout: Duration,
) -> Result<String, String> {
    let deadline = Instant::now() + timeout;
    let mut last_matching_names = Vec::new();
    loop {
        let current_error = match driver.accessible_names(window) {
            Ok(names) => {
                last_matching_names = names
                    .iter()
                    .filter(|name| name.starts_with("Room intent:"))
                    .take(8)
                    .cloned()
                    .collect();
                if let Some(name) = names.into_iter().find(|name| name.starts_with(prefix)) {
                    return Ok(name);
                }
                None
            }
            Err(error) => Some(error),
        };
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for accessible-name prefix {prefix:?}; last matching room intents: {last_matching_names:?}; last read error: {current_error:?}"
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

pub(super) fn join_room_from_main_window<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    room: &str,
    timeout: Duration,
) -> Result<(), String> {
    navigate_to_room_surface(driver, window, timeout)?;

    if wait_for_accessible_name(driver, window, "Join Room", Duration::from_millis(800)).is_err() {
        let _ = wait_for_accessible_name_with_page_up(
            driver,
            window,
            "Change Room",
            MAIN_WINDOW_LOCAL_READY_BUTTON_MAX_PAGE_DOWNS,
            timeout.min(Duration::from_millis(1_200)),
        );
        invoke_named_control_with_wait(
            driver,
            window,
            "Change Room",
            NativeControlKind::Button,
            timeout,
        )
        .map_err(|error| format!("failed to expand room controls: {error}"))?;
        wait_for_accessible_name(driver, window, "Join Room", timeout)?;
    }

    driver.set_named_edit_value(window, "Room", room, false)?;
    wait_for_named_edit_value(driver, window, "Room", room, timeout)?;
    invoke_named_control_with_wait(
        driver,
        window,
        "Join Room",
        NativeControlKind::Button,
        timeout,
    )
}

pub(super) fn wait_for_shared_playlist_visible<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    timeout: Duration,
) -> Result<(), String> {
    navigate_to_room_surface(driver, window, timeout)?;
    wait_for_accessible_name(driver, window, "Playlist", timeout).map(|_| ())
}

pub(super) fn wait_for_shared_playlist_controls_enabled<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    timeout: Duration,
) -> Result<(), String> {
    wait_for_shared_playlist_visible(driver, window, timeout)?;
    wait_for_named_control_enabled_state(
        driver,
        window,
        "Paste URLs...",
        NativeControlKind::Button,
        true,
        timeout,
    )
}

pub(super) fn wait_for_shared_playlist_entry<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    entry: &str,
    timeout: Duration,
) -> Result<(), String> {
    wait_for_shared_playlist_visible(driver, window, timeout)?;
    let deadline = Instant::now() + timeout;
    loop {
        let accessible_names = driver.accessible_names(window).unwrap_or_default();
        if accessible_names.iter().any(|name| name == entry) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let mut media_names = accessible_names
                .into_iter()
                .filter(|name| {
                    name.contains(".mkv")
                        || name.contains("Playlist")
                        || name.contains("Inbound")
                        || name.contains("failed")
                        || name.contains("unsupported")
                        || name.contains("bob")
                        || name.contains("smoke-user")
                        || name.contains("Ready")
                        || name.contains("pending")
                })
                .collect::<Vec<_>>();
            media_names.sort();
            media_names.dedup();
            return Err(format!(
                "timed out waiting for shared playlist entry {entry:?}; playlist-related accessible names: {}",
                media_names.join(", ")
            ));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

pub(super) fn invoke_button_or_any_named_control<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    name: &str,
    timeout: Duration,
) -> Result<(), String> {
    invoke_named_control_with_wait(driver, window, name, NativeControlKind::Button, timeout)
        .or_else(|button_error| {
            invoke_named_control_with_wait(driver, window, name, NativeControlKind::Any, timeout)
                .map_err(|any_error| {
                    format!(
                        "failed to invoke {name:?}; button failure: {button_error}; any-control failure: {any_error}"
                    )
                })
        })
}

pub(super) fn add_shared_playlist_url_entry<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    entry: &str,
    timeout: Duration,
) -> Result<(), String> {
    wait_for_shared_playlist_controls_enabled(driver, window, timeout)?;
    invoke_button_or_any_named_control(driver, window, "Paste URLs...", timeout)?;
    wait_for_accessible_name(driver, window, "Add URLs", timeout)?;
    driver.set_named_edit_value(window, "URLs", entry, false)?;
    wait_for_named_edit_value(driver, window, "URLs", entry, timeout)?;
    invoke_button_or_any_named_control(driver, window, "Add URLs To Playlist", timeout)?;
    wait_for_named_control_count(
        driver,
        window,
        "Add URLs",
        NativeControlKind::Any,
        0,
        timeout,
    )?;
    wait_for_accessible_name_fragment(driver, window, entry, timeout).map(|_| ())
}

pub(super) fn assert_chat_input_cleared<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    timeout: Duration,
) -> Result<(), String> {
    wait_for_named_edit_value(driver, window, "Chat Input", "", timeout)
}

pub(super) fn wait_for_visible_chat_message<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    sender: &str,
    message: &str,
    timeout: Duration,
) -> Result<(), String> {
    let expected_label = format!("{sender}: {message}");
    if wait_for_accessible_name(
        driver,
        window,
        &expected_label,
        timeout.min(Duration::from_millis(800)),
    )
    .is_ok()
    {
        return Ok(());
    }

    let _ = navigate_to_room_surface(driver, window, Duration::from_millis(800));

    if wait_for_accessible_name(
        driver,
        window,
        &expected_label,
        timeout.min(Duration::from_millis(800)),
    )
    .is_ok()
    {
        return Ok(());
    }

    if wait_for_accessible_name_with_named_control_scroll_up(
        driver,
        window,
        &expected_label,
        "Chat",
        NativeControlKind::Any,
        3,
        timeout,
    )
    .is_ok()
    {
        return Ok(());
    }

    wait_for_accessible_name_with_named_control_scroll_down(
        driver,
        window,
        &expected_label,
        "Chat",
        NativeControlKind::Any,
        3,
        timeout,
    )
    .map(|_| ())
}

pub(super) fn send_chat_message_and_complete<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    message: &str,
    timeout: Duration,
) -> Result<(), String> {
    navigate_to_room_surface(driver, window, timeout)?;
    wait_for_accessible_name(driver, window, "Chat Input", timeout)?;
    driver.set_named_edit_value(window, "Chat Input", message, true)?;
    assert_chat_input_cleared(driver, window, timeout)?;
    Ok(())
}

pub(super) fn wait_for_pending_operation_to_finish<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    pending_label: &str,
    timeout: Duration,
) -> Result<(), String> {
    let _ = wait_for_accessible_name(
        driver,
        window,
        pending_label,
        timeout.min(Duration::from_millis(800)),
    );
    if wait_for_named_control_count(
        driver,
        window,
        pending_label,
        NativeControlKind::Any,
        0,
        timeout.min(Duration::from_millis(800)),
    )
    .is_ok()
    {
        return Ok(());
    }
    wait_for_accessible_name(driver, window, "pending: (none)", timeout)
        .map_err(|error| format!("pending operation {pending_label:?} did not finish: {error}"))
}

pub(super) fn wait_for_named_control_enabled_state<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    name: &str,
    control_kind: NativeControlKind,
    expected_enabled: bool,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut last_error = None;
    let mut last_snapshot = None;
    loop {
        match driver.count_named_controls_with_enabled_state(
            window,
            name,
            control_kind,
            expected_enabled,
        ) {
            Ok(count) if count > 0 => return Ok(()),
            Ok(_) => {}
            Err(error) => last_error = Some(error),
        }
        if let Ok(names) = driver.accessible_names(window) {
            last_snapshot = Some(render_accessible_name_snapshot_for_patterns(
                &names,
                &[
                    name,
                    "view:",
                    "pending:",
                    "Connect",
                    "Disconnect",
                    "Status:",
                    "self=",
                    "ready=",
                    "controller=",
                ],
            ));
        }
        if Instant::now() >= deadline {
            let matching_state_summary = {
                let enabled_count = driver
                    .count_named_controls_with_enabled_state(window, name, control_kind, true)
                    .unwrap_or_default();
                let disabled_count = driver
                    .count_named_controls_with_enabled_state(window, name, control_kind, false)
                    .unwrap_or_default();
                format!("enabled={enabled_count}, disabled={disabled_count}")
            };
            return if let Some(error) = last_error {
                Err(format!(
                    "timed out waiting for a {} {} named {name:?}; last count error: {error}; matching states: {matching_state_summary}; last snapshot: {}",
                    if expected_enabled {
                        "enabled"
                    } else {
                        "disabled"
                    },
                    control_kind.label(),
                    last_snapshot.unwrap_or_else(|| "unavailable".to_owned()),
                ))
            } else {
                Err(format!(
                    "timed out waiting for a {} {} named {name:?}; matching states: {matching_state_summary}; last snapshot: {}",
                    if expected_enabled {
                        "enabled"
                    } else {
                        "disabled"
                    },
                    control_kind.label(),
                    last_snapshot.unwrap_or_else(|| "unavailable".to_owned()),
                ))
            };
        }
        thread::sleep(Duration::from_millis(50));
    }
}

impl MockSessionServer {
    pub(super) fn recv_peer(&self, timeout: Duration, label: &str) -> Result<String, String> {
        self.peer_rx.recv_timeout(timeout).map_err(|error| {
            format!("timed out waiting for {label} peer endpoint from mock TCP server: {error}")
        })
    }

    pub(super) fn recv_hello(&self, timeout: Duration, label: &str) -> Result<String, String> {
        self.hello_rx.recv_timeout(timeout).map_err(|error| {
            format!("timed out waiting for {label} hello line from mock TCP server: {error}")
        })
    }

    pub(super) fn recv_playlist_exchange(
        &self,
        timeout: Duration,
        label: &str,
    ) -> Result<(String, String, String, String, String), String> {
        let receiver = self.playlist_exchange_rx.as_ref().ok_or_else(|| {
            format!("{label} mock TCP server does not expose playlist exchange evidence")
        })?;
        receiver.recv_timeout(timeout).map_err(|error| {
            format!("timed out waiting for {label} playlist exchange evidence: {error}")
        })
    }

    pub(super) fn recv_playstate_exchange(
        &self,
        timeout: Duration,
        label: &str,
    ) -> Result<(String, String), String> {
        let receiver = self.playstate_exchange_rx.as_ref().ok_or_else(|| {
            format!("{label} mock TCP server does not expose playstate exchange evidence")
        })?;
        receiver.recv_timeout(timeout).map_err(|error| {
            format!("timed out waiting for {label} playstate exchange evidence: {error}")
        })
    }

    pub(super) fn release(mut self, label: &str) -> Result<(), String> {
        let address = self.address.clone();
        let _ = self.release_tx.send(());
        if let Some(join_handle) = self.join_handle.take() {
            match join_handle.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    return Err(format!("{label} mock TCP server failed: {error}"));
                }
                Err(_) => return Err(format!("{label} mock TCP server thread panicked")),
            }
        }
        let rebound = TcpListener::bind(&address).map_err(|error| {
            format!("{label} mock TCP server did not release exact endpoint {address}: {error}")
        })?;
        drop(rebound);
        Ok(())
    }
}

pub(super) fn read_mock_session_startup_hello_line(
    stream: &mut std::net::TcpStream,
    reader: &mut BufReader<std::net::TcpStream>,
) -> Result<String, String> {
    let mut hello_line = String::new();
    reader
        .read_line(&mut hello_line)
        .map_err(|error| format!("mock TCP server failed to read startup hello line: {error}"))?;
    if hello_line.contains("\"startTLS\":\"send\"") {
        stream
            .write_all(br#"{"TLS":{"startTLS":"false"}}"#)
            .map_err(|error| {
                format!("mock TCP server failed to decline startup TLS negotiation: {error}")
            })?;
        stream.write_all(b"\n").map_err(|error| {
            format!("mock TCP server failed to terminate TLS negotiation response: {error}")
        })?;
        hello_line.clear();
        reader.read_line(&mut hello_line).map_err(|error| {
            format!("mock TCP server failed to read post-TLS startup hello line: {error}")
        })?;
    }
    Ok(hello_line)
}

pub(super) fn start_mock_session_server(
    initial_lines: &'static [&'static str],
    first_chat_followup_lines: &'static [&'static str],
    second_chat_followup_lines: &'static [&'static str],
) -> Result<MockSessionServer, String> {
    start_mock_session_server_with_release_policy(
        initial_lines,
        first_chat_followup_lines,
        second_chat_followup_lines,
        None,
    )
}

pub(super) fn start_mock_session_server_with_hold_timeout(
    initial_lines: &'static [&'static str],
    first_chat_followup_lines: &'static [&'static str],
    second_chat_followup_lines: &'static [&'static str],
    hold_timeout: Duration,
) -> Result<MockSessionServer, String> {
    start_mock_session_server_with_release_policy(
        initial_lines,
        first_chat_followup_lines,
        second_chat_followup_lines,
        Some(hold_timeout),
    )
}

fn start_mock_session_server_with_release_policy(
    initial_lines: &'static [&'static str],
    first_chat_followup_lines: &'static [&'static str],
    second_chat_followup_lines: &'static [&'static str],
    hold_timeout: Option<Duration>,
) -> Result<MockSessionServer, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("failed to bind mock TCP listener: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("failed to set mock TCP listener nonblocking mode: {error}"))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("failed to read mock TCP listener address: {error}"))?;
    let port = address.port();

    let (peer_tx, peer_rx) = mpsc::channel();
    let (hello_tx, hello_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let join_handle = thread::spawn(move || -> Result<(), String> {
        let accept_deadline = Instant::now() + Duration::from_secs(25);
        let (mut stream, peer) = loop {
            if release_rx.try_recv().is_ok() {
                return Ok(());
            }
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    if Instant::now() >= accept_deadline {
                        return Err(
                            "mock TCP server timed out waiting for client connection".to_owned()
                        );
                    }
                    thread::sleep(Duration::from_millis(40));
                    continue;
                }
                Err(error) => {
                    return Err(format!("mock TCP server failed to accept client: {error}"));
                }
            }
        };
        if !peer.is_ipv4() || !peer.ip().is_loopback() {
            return Err(format!(
                "mock TCP server rejected non-IPv4-loopback peer {peer}"
            ));
        }
        peer_tx.send(peer.to_string()).map_err(|error| {
            format!("mock TCP server failed to report connected peer endpoint: {error}")
        })?;
        stream
            .set_nonblocking(false)
            .map_err(|error| format!("mock TCP server failed to restore blocking mode: {error}"))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .map_err(|error| format!("mock TCP server failed to set read timeout: {error}"))?;
        stream
            .set_write_timeout(Some(Duration::from_secs(10)))
            .map_err(|error| format!("mock TCP server failed to set write timeout: {error}"))?;
        let reader_stream = stream
            .try_clone()
            .map_err(|error| format!("mock TCP server failed to clone stream: {error}"))?;
        let mut reader = BufReader::new(reader_stream);
        let hello_line = read_mock_session_startup_hello_line(&mut stream, &mut reader)?;
        hello_tx.send(hello_line).map_err(|error| {
            format!("mock TCP server failed to report startup hello line: {error}")
        })?;
        for line in initial_lines {
            stream
                .write_all(line.as_bytes())
                .map_err(|error| format!("mock TCP server failed to write state line: {error}"))?;
            stream.write_all(b"\n").map_err(|error| {
                format!("mock TCP server failed to terminate state line: {error}")
            })?;
        }

        let mut process_followup = |phase_label: &str,
                                    lines: &'static [&'static str]|
         -> Result<(), String> {
            if lines.is_empty() {
                return Ok(());
            }

            let _chat_line = loop {
                let mut candidate = String::new();
                reader.read_line(&mut candidate).map_err(|error| {
                    format!("mock TCP server failed to read {phase_label} chat line: {error}")
                })?;
                if candidate.trim().is_empty() {
                    return Err(format!(
                        "mock TCP server received an empty {phase_label} chat line"
                    ));
                }
                if candidate.contains("\"ping\"") {
                    continue;
                }
                break candidate;
            };

            for line in lines {
                if let Err(error) = stream.write_all(line.as_bytes()) {
                    if matches!(
                        error.kind(),
                        ErrorKind::BrokenPipe
                            | ErrorKind::ConnectionAborted
                            | ErrorKind::ConnectionReset
                    ) {
                        break;
                    }
                    return Err(format!(
                        "mock TCP server failed to write {phase_label} follow-up state line: {error}"
                    ));
                }
                if let Err(error) = stream.write_all(b"\n") {
                    if matches!(
                        error.kind(),
                        ErrorKind::BrokenPipe
                            | ErrorKind::ConnectionAborted
                            | ErrorKind::ConnectionReset
                    ) {
                        break;
                    }
                    return Err(format!(
                        "mock TCP server failed to terminate {phase_label} follow-up state line: {error}"
                    ));
                }
            }
            Ok(())
        };

        process_followup("first", first_chat_followup_lines)?;
        process_followup("second", second_chat_followup_lines)?;

        match hold_timeout {
            Some(hold_timeout) => {
                let _ = release_rx.recv_timeout(hold_timeout);
            }
            None => {
                // Scenario-owned fixtures stay connected until the scenario
                // has closed and joined the GUI process. This prevents slow
                // native interaction from turning a test fixture's timeout
                // into a product reconnect failure.
                let _ = release_rx.recv();
            }
        }
        Ok(())
    });

    Ok(MockSessionServer {
        address: address.to_string(),
        port,
        peer_rx,
        hello_rx,
        playlist_exchange_rx: None,
        playstate_exchange_rx: None,
        release_tx,
        join_handle: Some(join_handle),
    })
}

pub(super) fn start_phased_mock_session_server(
    initial_lines: &'static [&'static str],
) -> Result<MockSessionServer, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("failed to bind mock TCP listener: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("failed to set mock TCP listener nonblocking mode: {error}"))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("failed to read mock TCP listener address: {error}"))?;
    let port = address.port();

    let (peer_tx, peer_rx) = mpsc::channel();
    let (hello_tx, hello_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let join_handle = thread::spawn(move || -> Result<(), String> {
        let accept_deadline = Instant::now() + Duration::from_secs(25);
        let (mut stream, peer) = loop {
            if release_rx.try_recv().is_ok() {
                return Ok(());
            }
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    if Instant::now() >= accept_deadline {
                        return Err(
                            "mock TCP server timed out waiting for client connection".to_owned()
                        );
                    }
                    thread::sleep(Duration::from_millis(40));
                    continue;
                }
                Err(error) => {
                    return Err(format!("mock TCP server failed to accept client: {error}"));
                }
            }
        };
        if !peer.is_ipv4() || !peer.ip().is_loopback() {
            return Err(format!(
                "mock TCP server rejected non-IPv4-loopback peer {peer}"
            ));
        }
        peer_tx.send(peer.to_string()).map_err(|error| {
            format!("mock TCP server failed to report connected peer endpoint: {error}")
        })?;
        stream
            .set_nonblocking(false)
            .map_err(|error| format!("mock TCP server failed to restore blocking mode: {error}"))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .map_err(|error| format!("mock TCP server failed to set read timeout: {error}"))?;
        stream
            .set_write_timeout(Some(Duration::from_secs(10)))
            .map_err(|error| format!("mock TCP server failed to set write timeout: {error}"))?;
        let reader_stream = stream
            .try_clone()
            .map_err(|error| format!("mock TCP server failed to clone stream: {error}"))?;
        let mut reader = BufReader::new(reader_stream);
        let hello_line = read_mock_session_startup_hello_line(&mut stream, &mut reader)?;
        hello_tx.send(hello_line).map_err(|error| {
            format!("mock TCP server failed to report startup hello line: {error}")
        })?;

        let mut write_lines = |label: &str, lines: &'static [&'static str]| -> Result<(), String> {
            for line in lines {
                stream.write_all(line.as_bytes()).map_err(|error| {
                    format!("mock TCP server failed to write {label} state line: {error}")
                })?;
                stream.write_all(b"\n").map_err(|error| {
                    format!("mock TCP server failed to terminate {label} state line: {error}")
                })?;
            }
            Ok(())
        };

        write_lines("initial", initial_lines)?;

        // This fixture owns a live transport, so its lifetime must be tied to
        // the scenario rather than an arbitrary wall-clock delay. A slow UIA
        // pass can legitimately take longer than ten seconds; closing the
        // socket here used to manufacture a connection reset and reconnect
        // attempt while the GUI was still under test.
        let _ = release_rx.recv();
        Ok(())
    });

    Ok(MockSessionServer {
        address: address.to_string(),
        port,
        peer_rx,
        hello_rx,
        playlist_exchange_rx: None,
        playstate_exchange_rx: None,
        release_tx,
        join_handle: Some(join_handle),
    })
}

fn redacted_playlist_echo_frame_shape(value: &serde_json::Value) -> &'static str {
    let Some(top_level) = value.as_object() else {
        return "non-object";
    };
    if top_level.len() != 1 {
        return "multi-field-top-level";
    }
    if let Some(set) = top_level.get("Set") {
        let Some(set) = set.as_object() else {
            return "Set.non-object";
        };
        if set.len() != 1 {
            return "Set.multi-field";
        }
        if set.contains_key("playlistChange") {
            return "Set.playlistChange";
        }
        if set.contains_key("playlistIndex") {
            return "Set.playlistIndex";
        }
        if set.contains_key("ready") {
            return "Set.ready";
        }
        return "Set.other";
    }
    if top_level.contains_key("List") {
        return "List";
    }
    if let Some(state) = top_level.get("State") {
        let Some(state) = state.as_object() else {
            return "State.non-object";
        };
        if state.len() == 1 && state.contains_key("ping") {
            return "State.ping";
        }
        return "State.other";
    }
    "other-top-level"
}

fn is_known_pre_media_state_heartbeat(value: &serde_json::Value) -> bool {
    let Some(top_level) = value.as_object() else {
        return false;
    };
    if top_level.len() != 1 {
        return false;
    }
    let Some(state) = top_level
        .get("State")
        .and_then(serde_json::Value::as_object)
    else {
        return false;
    };
    if state.len() != 1 {
        return false;
    }
    let Some(ping) = state.get("ping").and_then(serde_json::Value::as_object) else {
        return false;
    };
    if ping.len() != 2
        || !ping.contains_key("clientLatencyCalculation")
        || !ping.contains_key("clientRtt")
    {
        return false;
    }
    let Some(client_latency_calculation) = ping
        .get("clientLatencyCalculation")
        .and_then(serde_json::Value::as_f64)
    else {
        return false;
    };
    let Some(client_rtt) = ping.get("clientRtt").and_then(serde_json::Value::as_f64) else {
        return false;
    };
    client_latency_calculation.is_finite()
        && client_latency_calculation > 0.0
        && client_rtt.is_finite()
        && client_rtt >= 0.0
}

fn is_known_playlist_echo_housekeeping_frame(value: &serde_json::Value) -> bool {
    value == &serde_json::json!({"List": null})
        || value == &serde_json::json!({"State": {"ping": {}}})
        || is_known_pre_media_state_heartbeat(value)
        || value
            == &serde_json::json!({
                "Set": {
                    "ready": {
                        "isReady": false,
                        "manuallyInitiated": false,
                    }
                }
            })
}

fn validated_client_playstate_transition(
    value: &serde_json::Value,
) -> Result<Option<(bool, serde_json::Value)>, String> {
    let Some(state) = value.get("State") else {
        return Ok(None);
    };
    let Some(playstate) = state.get("playstate") else {
        return Ok(None);
    };
    let top_level = value.as_object().ok_or_else(|| {
        "playlist-echo mock TCP server received a non-object playstate frame".to_owned()
    })?;
    if top_level.len() != 1 {
        return Err(
            "playlist-echo mock TCP server received a widened playstate top-level schema"
                .to_owned(),
        );
    }
    let state = state.as_object().ok_or_else(|| {
        "playlist-echo mock TCP server received a non-object State playstate frame".to_owned()
    })?;
    if state
        .keys()
        .any(|key| !matches!(key.as_str(), "playstate" | "ping" | "ignoringOnTheFly"))
    {
        return Err(
            "playlist-echo mock TCP server received an unknown field beside client playstate"
                .to_owned(),
        );
    }
    let playstate = playstate.as_object().ok_or_else(|| {
        "playlist-echo mock TCP server received a non-object client playstate".to_owned()
    })?;
    if playstate.keys().any(|key| {
        !matches!(
            key.as_str(),
            "position" | "paused" | "doSeek" | "sorotteTransportRevision"
        )
    }) {
        return Err(
            "playlist-echo mock TCP server received a widened client playstate schema".to_owned(),
        );
    }
    let position = playstate
        .get("position")
        .and_then(serde_json::Value::as_f64)
        .filter(|position| position.is_finite() && *position >= 0.0)
        .ok_or_else(|| {
            "playlist-echo mock TCP server client playstate omitted a finite non-negative position"
                .to_owned()
        })?;
    let paused = playstate
        .get("paused")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| {
            "playlist-echo mock TCP server client playstate omitted a boolean paused state"
                .to_owned()
        })?;
    if playstate
        .get("doSeek")
        .is_some_and(|do_seek| !do_seek.is_null() && do_seek.as_bool().is_none())
    {
        return Err(
            "playlist-echo mock TCP server client playstate used a non-boolean doSeek".to_owned(),
        );
    }
    if playstate
        .get("sorotteTransportRevision")
        .is_some_and(|revision| revision.as_u64().is_none_or(|revision| revision == 0))
    {
        return Err(
            "playlist-echo mock TCP server client playstate used an invalid transport revision"
                .to_owned(),
        );
    }

    let mut authoritative_playstate = serde_json::Map::new();
    authoritative_playstate.insert("position".to_owned(), serde_json::json!(position));
    authoritative_playstate.insert("paused".to_owned(), serde_json::json!(paused));
    authoritative_playstate.insert(
        "doSeek".to_owned(),
        playstate
            .get("doSeek")
            .cloned()
            .filter(|value| !value.is_null())
            .unwrap_or_else(|| serde_json::json!(false)),
    );
    if let Some(revision) = playstate.get("sorotteTransportRevision") {
        authoritative_playstate.insert("sorotteTransportRevision".to_owned(), revision.clone());
    }
    Ok(Some((
        paused,
        serde_json::Value::Object(authoritative_playstate),
    )))
}

fn is_known_post_playlist_housekeeping_frame(
    value: &serde_json::Value,
    expected_media_url: &str,
) -> bool {
    if is_known_playlist_echo_housekeeping_frame(value) {
        return true;
    }
    if value
        == &serde_json::json!({
            "Set": {
                "playlistChange": {
                    "files": [expected_media_url],
                }
            }
        })
        || value
            == &serde_json::json!({
                "Set": {
                    "playlistIndex": {
                        "index": 0,
                    }
                }
            })
    {
        return true;
    }
    let Some(top_level) = value.as_object().filter(|value| value.len() == 1) else {
        return false;
    };
    if let Some(set) = top_level
        .get("Set")
        .and_then(serde_json::Value::as_object)
        .filter(|value| value.len() == 1)
    {
        if set.get("file").is_some_and(serde_json::Value::is_object) {
            return true;
        }
        if let Some(ready) = set.get("ready").and_then(serde_json::Value::as_object) {
            return ready
                .keys()
                .all(|key| matches!(key.as_str(), "isReady" | "manuallyInitiated"))
                && ready
                    .get("isReady")
                    .is_some_and(serde_json::Value::is_boolean)
                && ready
                    .get("manuallyInitiated")
                    .is_none_or(serde_json::Value::is_boolean);
        }
    }
    let Some(state) = top_level
        .get("State")
        .and_then(serde_json::Value::as_object)
    else {
        return false;
    };
    !state.contains_key("playstate")
        && state.keys().all(|key| {
            matches!(key.as_str(), "ping" | "ignoringOnTheFly") || key.starts_with("sorotte")
        })
}

pub(super) fn start_playlist_echo_mock_session_server(
    server_hello: &'static str,
    expected_media_url: String,
    username: &'static str,
) -> Result<MockSessionServer, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("failed to bind playlist-echo mock TCP listener: {error}"))?;
    listener.set_nonblocking(true).map_err(|error| {
        format!("failed to set playlist-echo mock TCP listener nonblocking mode: {error}")
    })?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("failed to read playlist-echo mock TCP address: {error}"))?;
    let port = address.port();

    let (peer_tx, peer_rx) = mpsc::channel();
    let (hello_tx, hello_rx) = mpsc::channel();
    let (playlist_exchange_tx, playlist_exchange_rx) = mpsc::channel();
    let (playstate_exchange_tx, playstate_exchange_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let join_handle = thread::Builder::new()
        .name("sorotte-native-playlist-echo".to_owned())
        .spawn(move || -> Result<(), String> {
            let accept_deadline = Instant::now() + Duration::from_secs(25);
            let (mut stream, peer) = loop {
                if release_rx.try_recv().is_ok() {
                    return Ok(());
                }
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        if Instant::now() >= accept_deadline {
                            return Err(
                                "playlist-echo mock TCP server timed out waiting for client connection"
                                    .to_owned(),
                            );
                        }
                        thread::sleep(Duration::from_millis(40));
                    }
                    Err(error) => {
                        return Err(format!(
                            "playlist-echo mock TCP server failed to accept client: {error}"
                        ));
                    }
                }
            };
            if !peer.is_ipv4() || !peer.ip().is_loopback() {
                return Err(format!(
                    "playlist-echo mock TCP server rejected non-IPv4-loopback peer {peer}"
                ));
            }
            peer_tx.send(peer.to_string()).map_err(|error| {
                format!(
                    "playlist-echo mock TCP server failed to report connected peer endpoint: {error}"
                )
            })?;
            stream.set_nonblocking(false).map_err(|error| {
                format!("playlist-echo mock TCP server failed to restore blocking mode: {error}")
            })?;
            stream
                .set_read_timeout(Some(Duration::from_millis(100)))
                .map_err(|error| {
                    format!(
                        "playlist-echo mock TCP server failed to set read timeout: {error}"
                    )
                })?;
            stream
                .set_write_timeout(Some(Duration::from_secs(10)))
                .map_err(|error| {
                    format!(
                        "playlist-echo mock TCP server failed to set write timeout: {error}"
                    )
                })?;
            let reader_stream = stream.try_clone().map_err(|error| {
                format!("playlist-echo mock TCP server failed to clone stream: {error}")
            })?;
            let mut reader = BufReader::new(reader_stream);
            let hello_line = read_mock_session_startup_hello_line(&mut stream, &mut reader)?;
            hello_tx.send(hello_line).map_err(|error| {
                format!(
                    "playlist-echo mock TCP server failed to report startup hello line: {error}"
                )
            })?;
            stream
                .write_all(server_hello.as_bytes())
                .map_err(|error| {
                    format!("playlist-echo mock TCP server failed to write Hello: {error}")
                })?;
            stream.write_all(b"\n").map_err(|error| {
                format!("playlist-echo mock TCP server failed to terminate Hello: {error}")
            })?;

            let exchange_deadline = Instant::now() + Duration::from_secs(25);
            let mut unrelated_frame_count = 0usize;
            let request = loop {
                if release_rx.try_recv().is_ok() {
                    return Err(
                        "playlist-echo mock TCP server was released before playlistChange"
                            .to_owned(),
                    );
                }
                let mut candidate = String::new();
                match reader.read_line(&mut candidate) {
                    Ok(0) => {
                        return Err(
                            "playlist-echo mock TCP client closed before playlistChange".to_owned()
                        );
                    }
                    Ok(_) => {}
                    Err(error)
                        if matches!(
                            error.kind(),
                            ErrorKind::WouldBlock | ErrorKind::TimedOut
                        ) =>
                    {
                        if Instant::now() >= exchange_deadline {
                            return Err(
                                "playlist-echo mock TCP server timed out waiting for playlistChange"
                                    .to_owned(),
                            );
                        }
                        continue;
                    }
                    Err(error) => {
                        return Err(format!(
                            "playlist-echo mock TCP server failed reading client frame: {error}"
                        ));
                    }
                }
                let candidate = candidate.trim();
                if candidate.is_empty() {
                    return Err(
                        "playlist-echo mock TCP server received an empty client frame".to_owned()
                    );
                }
                let parsed: serde_json::Value =
                    serde_json::from_str(candidate).map_err(|error| {
                        format!(
                            "playlist-echo mock TCP server received malformed client JSON: {error}"
                        )
                    })?;
                let expected_playlist_change = serde_json::json!({
                    "Set": {
                        "playlistChange": {
                            "files": [expected_media_url.as_str()],
                        }
                    }
                });
                if parsed == expected_playlist_change {
                    break candidate.to_owned();
                }
                if is_known_playlist_echo_housekeeping_frame(&parsed) {
                    unrelated_frame_count = unrelated_frame_count.saturating_add(1);
                    if unrelated_frame_count > 64 {
                        return Err(
                            "playlist-echo mock TCP server exceeded known startup-frame budget before playlistChange"
                                .to_owned(),
                        );
                    }
                    continue;
                }
                if parsed.pointer("/Set/playlistChange").is_some() {
                    return Err(
                        "playlist-echo mock TCP server playlistChange did not match the exact closed request schema"
                            .to_owned(),
                    );
                }
                return Err(
                    format!(
                        "playlist-echo mock TCP server received an unexpected client frame before playlistChange (redacted shape: {})",
                        redacted_playlist_echo_frame_shape(&parsed)
                    ),
                );
            };

            let playlist_change_echo = serde_json::json!({
                "Set": {
                    "playlistChange": {
                        "files": [expected_media_url.as_str()],
                        "user": username,
                    }
                }
            })
            .to_string();
            stream
                .write_all(playlist_change_echo.as_bytes())
                .map_err(|error| {
                    format!(
                        "playlist-echo mock TCP server failed to write authoritative playlistChange echo: {error}"
                    )
                })?;
            stream.write_all(b"\n").map_err(|error| {
                format!(
                    "playlist-echo mock TCP server failed to terminate authoritative playlistChange echo: {error}"
                )
            })?;
            stream.flush().map_err(|error| {
                format!(
                    "playlist-echo mock TCP server failed to flush authoritative playlistChange echo: {error}"
                )
            })?;

            let expected_playlist_index = serde_json::json!({
                "Set": {
                    "playlistIndex": {
                        "index": 0,
                    }
                }
            });
            let playlist_index_request = loop {
                if release_rx.try_recv().is_ok() {
                    return Err(
                        "playlist-echo mock TCP server was released before playlistIndex"
                            .to_owned(),
                    );
                }
                let mut candidate = String::new();
                match reader.read_line(&mut candidate) {
                    Ok(0) => {
                        return Err(
                            "playlist-echo mock TCP client closed before playlistIndex".to_owned(),
                        );
                    }
                    Ok(_) => {}
                    Err(error)
                        if matches!(
                            error.kind(),
                            ErrorKind::WouldBlock | ErrorKind::TimedOut
                        ) =>
                    {
                        if Instant::now() >= exchange_deadline {
                            return Err(
                                "playlist-echo mock TCP server timed out waiting for playlistIndex"
                                    .to_owned(),
                            );
                        }
                        continue;
                    }
                    Err(error) => {
                        return Err(format!(
                            "playlist-echo mock TCP server failed reading client frame before playlistIndex: {error}"
                        ));
                    }
                }
                let candidate = candidate.trim();
                if candidate.is_empty() {
                    return Err(
                        "playlist-echo mock TCP server received an empty client frame before playlistIndex"
                            .to_owned(),
                    );
                }
                let parsed: serde_json::Value =
                    serde_json::from_str(candidate).map_err(|error| {
                        format!(
                            "playlist-echo mock TCP server received malformed client JSON before playlistIndex: {error}"
                        )
                    })?;
                if parsed == expected_playlist_index {
                    break candidate.to_owned();
                }
                if is_known_playlist_echo_housekeeping_frame(&parsed) {
                    unrelated_frame_count = unrelated_frame_count.saturating_add(1);
                    if unrelated_frame_count > 64 {
                        return Err(
                            "playlist-echo mock TCP server exceeded known housekeeping-frame budget before playlistIndex"
                                .to_owned(),
                        );
                    }
                    continue;
                }
                if parsed.pointer("/Set/playlistIndex").is_some() {
                    return Err(
                        "playlist-echo mock TCP server playlistIndex did not match the exact closed request schema"
                            .to_owned(),
                    );
                }
                return Err(format!(
                    "playlist-echo mock TCP server received an unexpected client frame before playlistIndex (redacted shape: {})",
                    redacted_playlist_echo_frame_shape(&parsed)
                ));
            };

            let playlist_index_echo = serde_json::json!({
                "Set": {
                    "playlistIndex": {
                        "index": 0,
                        "user": username,
                    }
                }
            })
            .to_string();
            stream
                .write_all(playlist_index_echo.as_bytes())
                .map_err(|error| {
                    format!(
                        "playlist-echo mock TCP server failed to write authoritative playlistIndex echo: {error}"
                    )
                })?;
            stream.write_all(b"\n").map_err(|error| {
                format!(
                    "playlist-echo mock TCP server failed to terminate authoritative playlistIndex echo: {error}"
                )
            })?;
            stream.flush().map_err(|error| {
                format!(
                    "playlist-echo mock TCP server failed to flush authoritative playlistIndex echo: {error}"
                )
            })?;

            let initial_playstate = serde_json::json!({
                "State": {
                    "playstate": {
                        "position": 0.0,
                        "paused": true,
                        "doSeek": false,
                        "setBy": username,
                    }
                }
            })
            .to_string();
            stream
                .write_all(initial_playstate.as_bytes())
                .map_err(|error| {
                    format!(
                        "playlist-echo mock TCP server failed to write initial authoritative playstate: {error}"
                    )
                })?;
            stream.write_all(b"\n").map_err(|error| {
                format!(
                    "playlist-echo mock TCP server failed to terminate initial authoritative playstate: {error}"
                )
            })?;
            stream.flush().map_err(|error| {
                format!(
                    "playlist-echo mock TCP server failed to flush initial authoritative playstate: {error}"
                )
            })?;
            playlist_exchange_tx
                .send((
                    request,
                    playlist_change_echo,
                    playlist_index_request,
                    playlist_index_echo,
                    initial_playstate,
                ))
                .map_err(|error| {
                    format!(
                        "playlist-echo mock TCP server failed to report exchange evidence: {error}"
                    )
                })?;

            let mut canonical_paused = true;
            let mut post_playlist_housekeeping_count = 0usize;
            loop {
                if release_rx.try_recv().is_ok() {
                    return Ok(());
                }
                let mut candidate = String::new();
                match reader.read_line(&mut candidate) {
                    Ok(0) => return Ok(()),
                    Ok(_) => {}
                    Err(error)
                        if matches!(
                            error.kind(),
                            ErrorKind::WouldBlock | ErrorKind::TimedOut
                        ) =>
                    {
                        continue;
                    }
                    Err(error) => {
                        return Err(format!(
                            "playlist-echo mock TCP server failed reading post-playlist client frame: {error}"
                        ));
                    }
                }
                let candidate = candidate.trim();
                if candidate.is_empty() {
                    return Err(
                        "playlist-echo mock TCP server received an empty post-playlist client frame"
                            .to_owned(),
                    );
                }
                let parsed: serde_json::Value =
                    serde_json::from_str(candidate).map_err(|error| {
                        format!(
                            "playlist-echo mock TCP server received malformed post-playlist client JSON: {error}"
                        )
                    })?;
                if let Some((paused, mut authoritative_playstate)) =
                    validated_client_playstate_transition(&parsed)?
                {
                    let do_seek = authoritative_playstate
                        .get("doSeek")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                    if paused == canonical_paused && !do_seek {
                        post_playlist_housekeeping_count =
                            post_playlist_housekeeping_count.saturating_add(1);
                        if post_playlist_housekeeping_count > 512 {
                            return Err(
                                "playlist-echo mock TCP server exceeded the post-playlist telemetry budget"
                                    .to_owned(),
                            );
                        }
                        continue;
                    }
                    authoritative_playstate
                        .as_object_mut()
                        .expect("validated authoritative playstate must remain an object")
                        .insert("setBy".to_owned(), serde_json::json!(username));
                    let authoritative_echo = serde_json::json!({
                        "State": {
                            "playstate": authoritative_playstate,
                        }
                    })
                    .to_string();
                    stream
                        .write_all(authoritative_echo.as_bytes())
                        .map_err(|error| {
                            format!(
                                "playlist-echo mock TCP server failed to write authoritative playstate echo: {error}"
                            )
                        })?;
                    stream.write_all(b"\n").map_err(|error| {
                        format!(
                            "playlist-echo mock TCP server failed to terminate authoritative playstate echo: {error}"
                        )
                    })?;
                    stream.flush().map_err(|error| {
                        format!(
                            "playlist-echo mock TCP server failed to flush authoritative playstate echo: {error}"
                        )
                    })?;
                    canonical_paused = paused;
                    playstate_exchange_tx
                        .send((candidate.to_owned(), authoritative_echo))
                        .map_err(|error| {
                            format!(
                                "playlist-echo mock TCP server failed to report authoritative playstate exchange: {error}"
                            )
                        })?;
                    continue;
                }
                if is_known_post_playlist_housekeeping_frame(&parsed, &expected_media_url) {
                    post_playlist_housekeeping_count =
                        post_playlist_housekeeping_count.saturating_add(1);
                    if post_playlist_housekeeping_count > 512 {
                        return Err(
                            "playlist-echo mock TCP server exceeded the post-playlist housekeeping budget"
                                .to_owned(),
                        );
                    }
                    continue;
                }
                return Err(format!(
                    "playlist-echo mock TCP server received an unexpected post-playlist client frame (redacted shape: {})",
                    redacted_playlist_echo_frame_shape(&parsed)
                ));
            }
        })
        .map_err(|error| format!("failed to spawn playlist-echo mock TCP server: {error}"))?;

    Ok(MockSessionServer {
        address: address.to_string(),
        port,
        peer_rx,
        hello_rx,
        playlist_exchange_rx: Some(playlist_exchange_rx),
        playstate_exchange_rx: Some(playstate_exchange_rx),
        release_tx,
        join_handle: Some(join_handle),
    })
}

pub(super) fn wait_for_named_edit_value<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    name: &str,
    expected_value: &str,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut last_value = None;
    let mut last_error = None;
    loop {
        match driver.get_named_edit_value(window, name) {
            Ok(value) => {
                if value == expected_value {
                    return Ok(());
                }
                last_value = Some(value);
            }
            Err(error) => {
                last_error = Some(error);
            }
        }
        if Instant::now() >= deadline {
            return if let Some(error) = last_error {
                Err(format!(
                    "timed out waiting for edit field {name:?} to equal {expected_value:?}; last read error: {error}"
                ))
            } else {
                Err(format!(
                    "timed out waiting for edit field {name:?} to equal {expected_value:?}; last value: {last_value:?}"
                ))
            };
        }
        thread::sleep(Duration::from_millis(50));
    }
}

pub(super) fn wait_for_accessible_name_with_page_down<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    name: &str,
    max_page_downs: usize,
    timeout: Duration,
) -> Result<usize, String> {
    let deadline = Instant::now() + timeout;
    let short_timeout = timeout.min(Duration::from_millis(800));
    for page_downs in 0..=max_page_downs {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        if wait_for_accessible_name(driver, window, name, short_timeout.min(remaining)).is_ok() {
            return Ok(page_downs);
        }
        if page_downs < max_page_downs {
            let _ = driver.scroll_active_view_page_down(window);
            thread::sleep(Duration::from_millis(120));
        }
    }
    Err(format!(
        "timed out waiting for accessible name {name:?} after {max_page_downs} page-down attempts"
    ))
}

pub(super) fn wait_for_accessible_name_with_page_up<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    name: &str,
    max_page_ups: usize,
    timeout: Duration,
) -> Result<usize, String> {
    let deadline = Instant::now() + timeout;
    let short_timeout = timeout.min(Duration::from_millis(800));
    for page_ups in 0..=max_page_ups {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        if wait_for_accessible_name(driver, window, name, short_timeout.min(remaining)).is_ok() {
            return Ok(page_ups);
        }
        if page_ups < max_page_ups {
            let _ = driver.scroll_active_view_page_up(window);
            thread::sleep(Duration::from_millis(120));
        }
    }
    Err(format!(
        "timed out waiting for accessible name {name:?} after {max_page_ups} page-up attempts"
    ))
}

pub(super) fn wait_for_accessible_name_with_named_control_scroll_down<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    name: &str,
    scroll_control_name: &str,
    scroll_control_kind: NativeControlKind,
    max_scrolls: usize,
    timeout: Duration,
) -> Result<usize, String> {
    let deadline = Instant::now() + timeout;
    let short_timeout = timeout.min(Duration::from_millis(500));
    for scrolls in 0..=max_scrolls {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        if wait_for_accessible_name(driver, window, name, short_timeout.min(remaining)).is_ok() {
            return Ok(scrolls);
        }
        if scrolls < max_scrolls {
            let _ =
                driver.scroll_named_control_down(window, scroll_control_name, scroll_control_kind);
            thread::sleep(Duration::from_millis(120));
        }
    }
    Err(format!(
        "timed out waiting for accessible name {name:?} after {max_scrolls} downward scrolls on {scroll_control_name:?}"
    ))
}

pub(super) fn wait_for_accessible_name_with_named_control_scroll_up<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    name: &str,
    scroll_control_name: &str,
    scroll_control_kind: NativeControlKind,
    max_scrolls: usize,
    timeout: Duration,
) -> Result<usize, String> {
    let deadline = Instant::now() + timeout;
    let short_timeout = timeout.min(Duration::from_millis(500));
    for scrolls in 0..=max_scrolls {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        if wait_for_accessible_name(driver, window, name, short_timeout.min(remaining)).is_ok() {
            return Ok(scrolls);
        }
        if scrolls < max_scrolls {
            let _ =
                driver.scroll_named_control_up(window, scroll_control_name, scroll_control_kind);
            thread::sleep(Duration::from_millis(120));
        }
    }
    Err(format!(
        "timed out waiting for accessible name {name:?} after {max_scrolls} upward scrolls on {scroll_control_name:?}"
    ))
}

#[cfg(test)]
mod failure_artifact_tests {
    use super::*;

    #[test]
    fn failure_artifact_identity_redacts_password_values_but_preserves_stable_ids() {
        let node = NativeAccessibilityNode {
            name: "native-failure-secret-canary".to_owned(),
            automation_id: "main-window:controller-auth:password".to_owned(),
            control_type: 0,
            enabled: true,
            focused: false,
            offscreen: false,
            bounds: None,
        };

        let (name, automation_id) = redacted_native_failure_node_identity(&node);

        assert_eq!(name, sorotte_secret::REDACTED_SECRET);
        assert_eq!(automation_id, node.automation_id);
    }

    #[test]
    fn failure_artifact_identity_preserves_non_secret_menu_evidence() {
        let node = NativeAccessibilityNode {
            name: "File".to_owned(),
            automation_id: FILE_MENU_AUTOMATION_ID.to_owned(),
            control_type: 0,
            enabled: true,
            focused: false,
            offscreen: false,
            bounds: Some([10, 10, 50, 30]),
        };

        let (name, automation_id) = redacted_native_failure_node_identity(&node);

        assert_eq!(name, "File");
        assert_eq!(automation_id, FILE_MENU_AUTOMATION_ID);
    }
}
