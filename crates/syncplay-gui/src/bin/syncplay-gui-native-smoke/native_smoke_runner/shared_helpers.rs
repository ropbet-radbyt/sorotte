use super::*;

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
        return initial_result;
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
        return initial_result;
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
        return initial_result;
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
        return initial_result;
    }
    wait_for_accessible_name_with_page_down(
        driver,
        window,
        name,
        MAIN_WINDOW_LOCAL_READY_BUTTON_MAX_PAGE_DOWNS,
        remaining,
    )
    .map(|_| ())
}

pub(super) fn navigate_to_room_surface<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    timeout: Duration,
) -> Result<(), String> {
    navigate_to_view_with_fallback(
        driver,
        window,
        "Room",
        "view: room",
        "Window",
        "Show Users",
        timeout,
    )
}

pub(super) fn wait_for_room_browser_visible<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    timeout: Duration,
) -> Result<(), String> {
    navigate_to_room_surface(driver, window, timeout)?;
    wait_for_accessible_name(driver, window, MAIN_WINDOW_ROOM_BROWSER_NAME, timeout).map(|_| ())
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

    let _ = invoke_menu_command_with_fallback(
        driver,
        window,
        "Window",
        "Show Chat",
        Duration::from_millis(800),
    );

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
    pub(super) fn recv_hello(&self, timeout: Duration, label: &str) -> Result<String, String> {
        self.hello_rx.recv_timeout(timeout).map_err(|error| {
            format!("timed out waiting for {label} hello line from mock TCP server: {error}")
        })
    }

    pub(super) fn release(mut self, label: &str) -> Result<(), String> {
        let _ = self.release_tx.send(());
        let Some(join_handle) = self.join_handle.take() else {
            return Ok(());
        };
        match join_handle.join() {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(format!("{label} mock TCP server failed: {error}")),
            Err(_) => Err(format!("{label} mock TCP server thread panicked")),
        }
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
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("failed to bind mock TCP listener: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("failed to set mock TCP listener nonblocking mode: {error}"))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("failed to read mock TCP listener address: {error}"))?;
    let port = address.port();

    let (hello_tx, hello_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let join_handle = thread::spawn(move || -> Result<(), String> {
        let accept_deadline = Instant::now() + Duration::from_secs(25);
        let (mut stream, _) = loop {
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

        let _ = release_rx.recv_timeout(Duration::from_secs(10));
        Ok(())
    });

    Ok(MockSessionServer {
        address: address.to_string(),
        port,
        hello_rx,
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

    let (hello_tx, hello_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let join_handle = thread::spawn(move || -> Result<(), String> {
        let accept_deadline = Instant::now() + Duration::from_secs(25);
        let (mut stream, _) = loop {
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

        let _ = release_rx.recv_timeout(Duration::from_secs(10));
        Ok(())
    });

    Ok(MockSessionServer {
        address: address.to_string(),
        port,
        hello_rx,
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

pub(super) fn wait_for_edit_value_by_index<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    edit_index: usize,
    expected_value: &str,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut last_value = None;
    let mut last_error = None;
    loop {
        match driver.get_edit_value_by_index(window, edit_index) {
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
                    "timed out waiting for edit field [{edit_index}] to equal {expected_value:?}; last read error: {error}"
                ))
            } else {
                Err(format!(
                    "timed out waiting for edit field [{edit_index}] to equal {expected_value:?}; last value: {last_value:?}"
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
