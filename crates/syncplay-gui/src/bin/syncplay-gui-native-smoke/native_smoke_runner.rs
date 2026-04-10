use super::*;

#[path = "native_smoke_runner/baseline_contract.rs"]
mod baseline_contract;
#[path = "native_smoke_runner/drag_drop_contract.rs"]
mod drag_drop_contract;
#[path = "native_smoke_runner/live_python_contracts.rs"]
mod live_python_contracts;
#[path = "native_smoke_runner/loopback_contract.rs"]
mod loopback_contract;
#[path = "native_smoke_runner/missing_media_contracts.rs"]
mod missing_media_contracts;
#[path = "native_smoke_runner/relaunch_contract.rs"]
mod relaunch_contract;
#[path = "native_smoke_runner/transport_contract.rs"]
mod transport_contract;

use baseline_contract::verify_interaction_contract;
use drag_drop_contract::verify_drag_and_drop_contract;
use live_python_contracts::{
    verify_live_python_peer_connect_contract, verify_live_python_peer_controlled_room_contract,
};
use loopback_contract::verify_loopback_chat_contract;
use missing_media_contracts::{
    verify_detached_missing_media_contract, verify_missing_media_continue_session_contract,
};
use relaunch_contract::verify_relaunch_config_reload_contract;
use transport_contract::verify_transport_reconnect_contract;

fn wait_for_main_window_user_row_name<D: NativeGuiDriver>(
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

fn assert_chat_input_cleared<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    timeout: Duration,
) -> Result<(), String> {
    wait_for_named_edit_value(driver, window, "Chat Input", "", timeout)
}

fn wait_for_visible_chat_message<D: NativeGuiDriver>(
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

    let _ = select_top_tab_with_wait(
        driver,
        window,
        "Chat",
        "Chat Input",
        Duration::from_millis(800),
    );

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

fn send_chat_message_and_complete<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    message: &str,
    timeout: Duration,
) -> Result<(), String> {
    let _ = select_top_tab_with_wait(driver, window, "Chat", "Chat Input", timeout);
    driver.set_named_edit_value(window, "Chat Input", message, true)?;
    wait_for_pending_operation_to_finish(driver, window, "pending: send-chat-message", timeout)?;
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
}

pub(super) fn run_native_smoke(options: &NativeSmokeOptions) -> Result<NativeSmokeReport, String> {
    let configured_binary_path = options
        .binary_path
        .clone()
        .unwrap_or_else(default_binary_path);
    let binary_path = resolve_binary_path(&configured_binary_path)?;
    if !binary_path.exists() {
        return Err(format!(
            "syncplay-gui binary does not exist: {binary_path:?}"
        ));
    }

    let temp_root = std::env::temp_dir().join(format!(
        "syncplay-gui-native-smoke-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    fs::create_dir_all(&temp_root)
        .map_err(|error| format!("failed to create native smoke temp directory: {error}"))?;
    let config_path = temp_root.join("syncplay-native-smoke.ini");
    let media_search_browse_path = temp_root.join("media-search");
    let open_media_file_path = temp_root.join("open-target.mkv");
    let _ = fs::remove_file(&config_path);
    fs::create_dir_all(&media_search_browse_path)
        .map_err(|error| format!("failed to create native smoke media directory: {error}"))?;
    fs::write(&open_media_file_path, b"open-target")
        .map_err(|error| format!("failed to create native smoke media file: {error}"))?;
    seed_native_smoke_config(&config_path)?;

    let started_at = Instant::now();
    let driver = PlatformNativeGuiDriver;
    let launch = GuiLaunchConfig {
        config_path: &config_path,
        media_search_browse_path: &media_search_browse_path,
        open_media_file_path: &open_media_file_path,
        public_servers_spec: DEFAULT_PUBLIC_SERVERS_SPEC,
        tcp_session: None,
        loopback_session: None,
        attach_test_player: false,
        drop_file_paths_spec: None,
        drop_target: None,
    };
    let (mut child, window) =
        launch_syncplay_gui_with_retry(&driver, &binary_path, launch, options.timeout)?;
    let pid = child.id();

    let result = (|| {
        let window_title = driver.window_title(window)?;
        if !window_title.contains("Syncplay") {
            return Err(format!(
                "main window title did not match expected prefix; got {window_title:?}"
            ));
        }

        let accessible_names = driver.accessible_names(window)?;
        verify_accessibility_contract(&accessible_names)?;
        let mut interaction_steps = if scenario_selected(options, "baseline") {
            verify_interaction_contract(
                &driver,
                window,
                &config_path,
                &media_search_browse_path,
                &open_media_file_path,
                options.timeout,
            )?
        } else {
            Vec::new()
        };
        if scenario_selected(options, "drag-drop") {
            interaction_steps.extend(verify_drag_and_drop_contract(
                &driver,
                &binary_path,
                &temp_root,
                &media_search_browse_path,
                options.timeout,
            )?);
        }
        let interaction_contract = "verified".to_owned();

        let menu_labels = driver.top_level_menu_labels(window)?;
        let menu_contract = if menu_labels.is_empty() {
            "skipped-no-native-menu".to_owned()
        } else {
            verify_menu_contract(&menu_labels)?;
            "verified".to_owned()
        };
        let accessibility_contract = "verified".to_owned();

        if options.keep_open {
            return Ok(NativeSmokeReport {
                binary_path: binary_path.display().to_string(),
                pid,
                window_title,
                menu_labels,
                menu_contract,
                accessible_name_count: accessible_names.len(),
                accessibility_contract,
                interaction_steps,
                interaction_contract,
                duration_ms: started_at.elapsed().as_millis(),
                closed: false,
            });
        }

        let close_step_timeout = options.timeout.min(Duration::from_millis(4_000));
        let closed_via_file_exit = if let Err(primary_error) = invoke_menu_command_with_wait(
            &driver,
            window,
            "File",
            "Exit",
            NativeControlKind::MenuItem,
            close_step_timeout,
        ) {
            match invoke_menu_command_with_wait(
                &driver,
                window,
                "File",
                "Exit",
                NativeControlKind::Any,
                close_step_timeout,
            ) {
                Ok(()) => {
                    wait_for_process_exit(&mut child, options.timeout)?;
                    interaction_steps.push("file-exit".to_owned());
                    true
                }
                Err(fallback_error) => {
                    interaction_steps.push(format!(
                        "file-exit-skipped:{}",
                        format!(
                            "menu-item-failure={primary_error}; fallback-failure={fallback_error}"
                        )
                        .replace('|', "/")
                        .replace('\n', " ")
                    ));
                    false
                }
            }
        } else {
            wait_for_process_exit(&mut child, options.timeout)?;
            interaction_steps.push("file-exit".to_owned());
            true
        };
        if !closed_via_file_exit {
            driver.close_window(window)?;
            wait_for_process_exit(&mut child, options.timeout)?;
            interaction_steps.push("window-close-fallback".to_owned());
        }

        if scenario_selected(options, "relaunch") {
            let relaunch_steps = verify_relaunch_config_reload_contract(
                &driver,
                &binary_path,
                &config_path,
                &media_search_browse_path,
                &open_media_file_path,
                options.timeout,
            )?;
            interaction_steps.extend(relaunch_steps);
        }

        if scenario_selected(options, "loopback") {
            let loopback_steps = verify_loopback_chat_contract(
                &driver,
                &binary_path,
                &config_path,
                &media_search_browse_path,
                &open_media_file_path,
                options.timeout,
            )?;
            interaction_steps.extend(loopback_steps);
        }

        if scenario_selected(options, "live-python") {
            let live_python_interop_steps = verify_live_python_peer_connect_contract(
                &driver,
                &binary_path,
                &temp_root,
                &media_search_browse_path,
                &open_media_file_path,
                options.timeout,
            )?;
            interaction_steps.extend(live_python_interop_steps);
        }

        if scenario_selected(options, "controlled-room") {
            let live_python_controlled_room_steps =
                verify_live_python_peer_controlled_room_contract(
                    &driver,
                    &binary_path,
                    &temp_root,
                    &media_search_browse_path,
                    &open_media_file_path,
                    options.timeout,
                )?;
            interaction_steps.extend(live_python_controlled_room_steps);
        }

        if scenario_selected(options, "detached-missing-media") {
            let detached_missing_media_steps = verify_detached_missing_media_contract(
                &driver,
                &binary_path,
                &temp_root,
                options.timeout,
            )?;
            interaction_steps.extend(detached_missing_media_steps);
        }

        if scenario_selected(options, "missing-media-continue") {
            let missing_media_continue_steps = verify_missing_media_continue_session_contract(
                &driver,
                &binary_path,
                &temp_root,
                &media_search_browse_path,
                &open_media_file_path,
                options.timeout,
            )?;
            interaction_steps.extend(missing_media_continue_steps);
        }

        if scenario_selected(options, "transport") {
            let transport_steps = verify_transport_reconnect_contract(
                &driver,
                &binary_path,
                &temp_root,
                &media_search_browse_path,
                &open_media_file_path,
                options.timeout,
            )?;
            interaction_steps.extend(transport_steps);
        }

        Ok(NativeSmokeReport {
            binary_path: binary_path.display().to_string(),
            pid,
            window_title,
            menu_labels,
            menu_contract,
            accessible_name_count: accessible_names.len(),
            accessibility_contract,
            interaction_steps,
            interaction_contract,
            duration_ms: started_at.elapsed().as_millis(),
            closed: true,
        })
    })();

    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }

    let _ = fs::remove_file(&config_path);
    let _ = fs::remove_dir_all(&temp_root);

    result
}

fn wait_for_named_control_enabled_state<D: NativeGuiDriver>(
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
    fn recv_hello(&self, timeout: Duration, label: &str) -> Result<String, String> {
        self.hello_rx.recv_timeout(timeout).map_err(|error| {
            format!("timed out waiting for {label} hello line from mock TCP server: {error}")
        })
    }

    fn release(mut self, label: &str) -> Result<(), String> {
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

fn read_mock_session_startup_hello_line(
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

fn start_mock_session_server(
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

fn start_timed_mock_session_server(
    initial_lines: &'static [&'static str],
    first_followup_delay: Duration,
    first_followup_lines: &'static [&'static str],
    second_followup_delay: Duration,
    second_followup_lines: &'static [&'static str],
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

        if !first_followup_lines.is_empty()
            && release_rx.recv_timeout(first_followup_delay).is_err()
        {
            write_lines("first follow-up", first_followup_lines)?;
        }
        if !second_followup_lines.is_empty()
            && release_rx.recv_timeout(second_followup_delay).is_err()
        {
            write_lines("second follow-up", second_followup_lines)?;
        }

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

fn wait_for_named_edit_value<D: NativeGuiDriver>(
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

fn trusted_domains_edit_index(player_arguments_enabled: bool) -> usize {
    if player_arguments_enabled { 8 } else { 7 }
}

fn wait_for_edit_value_by_index<D: NativeGuiDriver>(
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

fn wait_for_accessible_name_with_page_down<D: NativeGuiDriver>(
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

fn wait_for_accessible_name_with_page_up<D: NativeGuiDriver>(
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

fn wait_for_accessible_name_with_named_control_scroll_down<D: NativeGuiDriver>(
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

fn wait_for_accessible_name_with_named_control_scroll_up<D: NativeGuiDriver>(
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
