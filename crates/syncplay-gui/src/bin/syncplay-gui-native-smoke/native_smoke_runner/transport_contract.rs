use super::*;

pub(super) fn verify_transport_reconnect_contract<D: NativeGuiDriver>(
    driver: &D,
    binary_path: &Path,
    temp_root: &Path,
    media_search_browse_path: &Path,
    open_media_file_path: &Path,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    // Leave the initial transport state visible long enough for the full suite to
    // re-enter the main window and observe the reconnect rows before churn begins.
    let initial_state_observation_delay = Duration::from_millis(1_500);
    let primary_server = start_timed_mock_session_server(
        &[
            r#"{"Hello":{"username":"smoke-user","room":{"name":"smoke-room"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#,
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"smoke-user"}}}"#,
            r#"{"Set":{"playlistIndex":{"index":1,"user":"smoke-user"}}}"#,
            r#"{"Set":{"ready":{"isReady":true,"username":"smoke-user"}}}"#,
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"smoke-user"}}}"#,
            r#"{"Set":{"user":{"bob":{"room":{"name":"smoke-room"},"file":{"name":"bob.mp4"},"isReady":true,"controller":true}}}}"#,
        ],
        initial_state_observation_delay,
        &[
            r#"{"Set":{"playlistChange":{"files":["postchat1.mkv","postchat2.mkv"],"user":"smoke-user"}}}"#,
            r#"{"Set":{"playlistIndex":{"index":1,"user":"smoke-user"}}}"#,
            r#"{"Set":{"ready":{"isReady":false,"username":"smoke-user"}}}"#,
            r#"{"State":{"playstate":{"position":20.0,"paused":false,"doSeek":false,"setBy":"smoke-user"}}}"#,
            r#"{"Set":{"user":{"bob":{"room":{"name":"smoke-room"},"file":{"name":"bob-post.mp4"},"isReady":false,"controller":false}}}}"#,
        ],
        Duration::from_millis(700),
        &[r#"{"Set":{"user":{"bob":{"event":{"left":true}}}}}"#],
    )?;
    let reconnect_server = start_timed_mock_session_server(
        &[
            r#"{"Hello":{"username":"smoke-user","room":{"name":"smoke-room"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#,
            r#"{"Set":{"playlistChange":{"files":["reconnect1.mkv","reconnect2.mkv"],"user":"smoke-user"}}}"#,
            r#"{"Set":{"playlistIndex":{"index":1,"user":"smoke-user"}}}"#,
            r#"{"Set":{"ready":{"isReady":false,"username":"smoke-user"}}}"#,
            r#"{"State":{"playstate":{"position":20.0,"paused":false,"doSeek":false,"setBy":"smoke-user"}}}"#,
            r#"{"Set":{"user":{"carol":{"room":{"name":"smoke-room"},"file":{"name":"carol.mp4"},"isReady":false,"controller":false}}}}"#,
        ],
        initial_state_observation_delay,
        &[
            r#"{"Set":{"playlistChange":{"files":["reconnect-post1.mkv","reconnect-post2.mkv"],"user":"smoke-user"}}}"#,
            r#"{"Set":{"playlistIndex":{"index":1,"user":"smoke-user"}}}"#,
            r#"{"Set":{"ready":{"isReady":true,"username":"smoke-user"}}}"#,
            r#"{"State":{"playstate":{"position":30.0,"paused":true,"doSeek":false,"setBy":"smoke-user"}}}"#,
            r#"{"Set":{"user":{"carol":{"room":{"name":"smoke-room"},"file":{"name":"carol-post.mp4"},"isReady":true,"controller":true}}}}"#,
        ],
        Duration::from_millis(700),
        &[r#"{"Set":{"user":{"carol":{"event":{"left":true}}}}}"#],
    )?;

    let transport_config_path = temp_root.join("syncplay-native-smoke-transport.ini");
    let _ = fs::remove_file(&transport_config_path);
    seed_native_smoke_config(&transport_config_path)?;
    upsert_syncplay_ini_stored_client_settings_mvp_at_path(
        &transport_config_path,
        &StoredClientSettingsMvp {
            host: Some("127.0.0.1".to_owned()),
            port: Some(primary_server.port),
            username: Some(TRANSPORT_SESSION_USERNAME.to_owned()),
            room: Some(TRANSPORT_SESSION_ROOM.to_owned()),
            ..StoredClientSettingsMvp::default()
        },
    )
    .map_err(|error| {
        format!(
            "failed to prepare transport reconnect config {}: {error}",
            transport_config_path.display()
        )
    })?;
    let public_servers_spec = format!(
        "[['Primary', '{}'], ['Reconnect', '{}']]",
        primary_server.address, reconnect_server.address
    );
    let launch = GuiLaunchConfig {
        config_path: &transport_config_path,
        media_search_browse_path,
        open_media_file_path,
        public_servers_spec: &public_servers_spec,
        tcp_session: None,
        loopback_session: None,
        attach_test_player: false,
        drop_file_paths_spec: None,
        drop_target: None,
    };

    let launch_result = launch_syncplay_gui_with_retry(driver, binary_path, launch, timeout);
    let (mut child, window) = match launch_result {
        Ok(pair) => pair,
        Err(error) => {
            let primary_release = primary_server.release("primary");
            let reconnect_release = reconnect_server.release("reconnect");
            let mut combined_error =
                format!("failed to launch transport parity segment for native smoke: {error}");
            if let Err(release_error) = primary_release {
                combined_error.push_str("; ");
                combined_error.push_str(&release_error);
            }
            if let Err(release_error) = reconnect_release {
                combined_error.push_str("; ");
                combined_error.push_str(&release_error);
            }
            return Err(combined_error);
        }
    };

    let outcome = (|| -> Result<Vec<String>, String> {
        let step_timeout = timeout.min(Duration::from_millis(8_000));
        let mut steps = Vec::new();

        wait_for_any_accessible_name(
            driver,
            window,
            &[
                "modal: tls-certificate-prompt",
                "view: menus-and-dialogs",
                "view: configuration",
                "view: main-window",
            ],
            step_timeout,
        )?;
        if wait_for_accessible_name(
            driver,
            window,
            "modal: tls-certificate-prompt",
            step_timeout.min(Duration::from_millis(800)),
        )
        .is_ok()
        {
            invoke_named_control_with_wait(
                driver,
                window,
                "Trust Certificate",
                NativeControlKind::Button,
                step_timeout,
            )?;
            wait_for_accessible_name(driver, window, "modal: (none)", step_timeout)?;
        }
        navigate_to_view_with_fallback(
            driver,
            window,
            "Main Window",
            "view: main-window",
            "Window",
            "Show Users",
            step_timeout,
        )?;

        let first_hello = primary_server.recv_hello(step_timeout, "primary")?;
        if !first_hello.contains("\"Hello\"") {
            return Err(format!(
                "primary mock TCP server did not receive an expected startup hello payload: {first_hello:?}"
            ));
        }
        wait_for_accessible_name(driver, window, "episode2.mkv", step_timeout)?;
        wait_for_main_window_user_row_name(
            driver,
            window,
            "self=no, ready=yes, controller=yes",
            step_timeout,
        )?;
        wait_for_main_window_user_row_name(
            driver,
            window,
            LIVE_PYTHON_INTEROP_LOCAL_READY_ROW_NAME,
            step_timeout,
        )
        .map_err(|error| format!("transport initial local ready row: {error}"))?;
        steps.push("transport-saved-config-startup".to_owned());

        wait_for_accessible_name(driver, window, "postchat2.mkv", step_timeout)?;
        wait_for_main_window_user_row_name(
            driver,
            window,
            "self=no, ready=no, controller=no",
            step_timeout,
        )
        .map_err(|error| format!("transport primary post-ready row: {error}"))?;
        steps.push("transport-primary-post-ready-churn".to_owned());

        wait_for_named_control_count(
            driver,
            window,
            "self=no, ready=no, controller=no",
            NativeControlKind::Any,
            0,
            step_timeout,
        )?;
        steps.push("transport-primary-user-left".to_owned());

        navigate_to_view_with_fallback(
            driver,
            window,
            "Public Servers",
            "view: public-servers",
            "File",
            "Open Public Server Browser",
            step_timeout,
        )?;
        invoke_named_control_with_wait(
            driver,
            window,
            &format!("Reconnect: {}", reconnect_server.address),
            NativeControlKind::Any,
            step_timeout,
        )?;
        invoke_named_control_with_wait(
            driver,
            window,
            "Connect",
            NativeControlKind::Button,
            step_timeout,
        )?;
        wait_for_pending_operation_to_finish(
            driver,
            window,
            "pending: connect-public-server",
            step_timeout,
        )?;

        let second_hello = reconnect_server.recv_hello(step_timeout, "reconnect")?;
        if !second_hello.contains("\"Hello\"") {
            return Err(format!(
                "reconnect mock TCP server did not receive an expected reconnect hello payload: {second_hello:?}"
            ));
        }
        navigate_to_view_with_fallback(
            driver,
            window,
            "Main Window",
            "view: main-window",
            "Window",
            "Show Users",
            step_timeout,
        )?;
        wait_for_accessible_name(driver, window, "reconnect2.mkv", step_timeout)?;
        wait_for_main_window_user_row_name(
            driver,
            window,
            "self=no, ready=no, controller=no",
            step_timeout,
        )
        .map_err(|error| format!("transport reconnect initial row: {error}"))?;
        wait_for_main_window_user_row_name(
            driver,
            window,
            LIVE_PYTHON_INTEROP_LOCAL_ROW_NAME,
            step_timeout,
        )
        .map_err(|error| format!("transport reconnect initial local row: {error}"))?;
        steps.push("transport-public-server-reconnect".to_owned());

        wait_for_accessible_name(driver, window, "reconnect-post2.mkv", step_timeout)?;
        wait_for_main_window_user_row_name(
            driver,
            window,
            "self=no, ready=yes, controller=yes",
            step_timeout,
        )
        .map_err(|error| format!("transport reconnect post-ready row: {error}"))?;
        steps.push("transport-reconnect-post-ready-churn".to_owned());

        wait_for_named_control_count(
            driver,
            window,
            "self=no, ready=yes, controller=yes",
            NativeControlKind::Any,
            0,
            step_timeout,
        )?;
        steps.push("transport-reconnect-user-left".to_owned());

        driver.close_window(window)?;
        wait_for_process_exit(&mut child, timeout)?;
        Ok(steps)
    })();

    if outcome.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }

    let primary_release = primary_server.release("primary");
    let reconnect_release = reconnect_server.release("reconnect");
    let mut release_errors = Vec::new();
    if let Err(error) = primary_release {
        release_errors.push(error);
    }
    if let Err(error) = reconnect_release {
        release_errors.push(error);
    }

    match outcome {
        Ok(steps) if release_errors.is_empty() => Ok(steps),
        Ok(_) => Err(release_errors.join("; ")),
        Err(error) if release_errors.is_empty() => Err(error),
        Err(error) => Err(format!("{error}; {}", release_errors.join("; "))),
    }
}
