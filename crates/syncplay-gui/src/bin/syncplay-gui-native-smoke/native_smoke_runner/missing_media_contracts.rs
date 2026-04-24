use super::*;

pub(super) fn verify_detached_missing_media_contract<D: NativeGuiDriver>(
    driver: &D,
    binary_path: &Path,
    temp_root: &Path,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let detached_search_path = temp_root.join("detached-missing-media-search");
    let _ = fs::remove_dir_all(&detached_search_path);
    fs::create_dir_all(&detached_search_path).map_err(|error| {
        format!(
            "failed to create detached missing-media search directory {}: {error}",
            detached_search_path.display()
        )
    })?;
    let search_target_path = detached_search_path.join("search-target.mkv");
    fs::write(&search_target_path, b"detached-search-target").map_err(|error| {
        format!(
            "failed to create detached missing-media target {}: {error}",
            search_target_path.display()
        )
    })?;

    let config_path = temp_root.join("syncplay-native-smoke-detached-missing-media.ini");
    let _ = fs::remove_file(&config_path);
    seed_native_smoke_config_with_saved_server(&config_path, None, None)?;
    upsert_syncplay_ini_stored_client_settings_mvp_at_path(
        &config_path,
        &StoredClientSettingsMvp {
            shared_playlist_enabled: Some(true),
            media_search_directories: Some(vec![detached_search_path.display().to_string()]),
            ..StoredClientSettingsMvp::default()
        },
    )
    .map_err(|error| {
        format!(
            "failed to prepare detached missing-media config {}: {error}",
            config_path.display()
        )
    })?;

    let launch = GuiLaunchConfig {
        config_path: &config_path,
        media_search_browse_path: &detached_search_path,
        open_media_file_path: &search_target_path,
        public_servers_spec: DEFAULT_PUBLIC_SERVERS_SPEC,
        tcp_session: None,
        loopback_session: None,
        attach_test_player: true,
        drop_file_paths_spec: None,
        drop_target: None,
    };
    let (mut child, window) = launch_syncplay_gui_with_retry(driver, binary_path, launch, timeout)?;

    let outcome = (|| -> Result<Vec<String>, String> {
        let step_timeout = timeout.min(Duration::from_millis(15_000));
        let mut steps = Vec::new();

        let _initial_state = wait_for_any_accessible_name(
            driver,
            window,
            &[
                "modal: tls-certificate-prompt",
                "view: setup",
                "view: room",
                "view: setup",
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

        if wait_for_accessible_name(driver, window, "view: room", Duration::from_millis(800))
            .is_err()
        {
            navigate_to_view_with_fallback(
                driver,
                window,
                "Main Window",
                "view: room",
                "Window",
                "Show Users",
                step_timeout,
            )?;
        }

        let search_target_value = search_target_path.display().to_string();
        add_shared_playlist_url_entry(driver, window, &search_target_value, step_timeout)?;
        steps.push("detached-missing-media-target-staged".to_owned());

        navigate_to_view_with_fallback(
            driver,
            window,
            "Media Search",
            "view: setup",
            "File",
            "Open Media Search",
            step_timeout,
        )?;
        wait_for_accessible_name(
            driver,
            window,
            &detached_search_path.display().to_string(),
            step_timeout,
        )?;
        invoke_named_control_with_wait(
            driver,
            window,
            "Search Missing Media",
            NativeControlKind::Button,
            step_timeout,
        )?;
        wait_for_pending_operation_to_finish(
            driver,
            window,
            "pending: search-missing-media",
            step_timeout,
        )?;
        if wait_for_accessible_name(driver, window, "view: room", Duration::from_millis(1_200))
            .is_err()
        {
            wait_for_accessible_name(driver, window, "view: setup", step_timeout)?;
            navigate_to_view_with_fallback(
                driver,
                window,
                "Main Window",
                "view: room",
                "Window",
                "Show Users",
                step_timeout,
            )?;
        }
        steps.push("detached-missing-media-search-success".to_owned());

        driver.close_window(window)?;
        wait_for_process_exit(&mut child, timeout)?;
        Ok(steps)
    })();

    if outcome.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }

    outcome
}

pub(super) fn verify_missing_media_continue_session_contract<D: NativeGuiDriver>(
    driver: &D,
    binary_path: &Path,
    temp_root: &Path,
    _media_search_browse_path: &Path,
    open_media_file_path: &Path,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let session_server = start_mock_session_server(
        &[
            r#"{"Hello":{"username":"smoke-user","room":{"name":"smoke-room"},"version":"1.7.5","features":{"chat":true,"readiness":true}}}"#,
            r#"{"Set":{"playlistChange":{"files":["missing-source-a.mkv","missing-target.mkv"],"user":"smoke-user"}}}"#,
            r#"{"Set":{"playlistIndex":{"index":1,"user":"smoke-user"}}}"#,
            r#"{"Set":{"ready":{"isReady":true,"username":"smoke-user"}}}"#,
            r#"{"Set":{"user":{"bob":{"room":{"name":"smoke-room"},"file":{"name":"bob.mp4"},"isReady":true,"controller":true}}}}"#,
        ],
        &[],
        &[],
    )?;
    let continue_media_search_path = temp_root.join("missing-media-continue-session-search");
    let _ = fs::remove_dir_all(&continue_media_search_path);
    fs::create_dir_all(&continue_media_search_path).map_err(|error| {
        format!(
            "failed to create missing-media continuation search directory {}: {error}",
            continue_media_search_path.display()
        )
    })?;
    let located_media_path = continue_media_search_path.join("missing-target.mkv");
    fs::write(&located_media_path, b"missing-media-continuation-target").map_err(|error| {
        format!(
            "failed to create missing-media continuation target {}: {error}",
            located_media_path.display()
        )
    })?;

    let continue_config_path = temp_root.join("syncplay-native-smoke-missing-media.ini");
    let _ = fs::remove_file(&continue_config_path);
    seed_native_smoke_config(&continue_config_path)?;
    upsert_syncplay_ini_stored_client_settings_mvp_at_path(
        &continue_config_path,
        &StoredClientSettingsMvp {
            shared_playlist_enabled: Some(false),
            media_search_directories: Some(vec![continue_media_search_path.display().to_string()]),
            ..StoredClientSettingsMvp::default()
        },
    )
    .map_err(|error| {
        format!(
            "failed to prepare missing-media continuation config {}: {error}",
            continue_config_path.display()
        )
    })?;

    let launch = GuiLaunchConfig {
        config_path: &continue_config_path,
        media_search_browse_path: &continue_media_search_path,
        open_media_file_path,
        public_servers_spec: DEFAULT_PUBLIC_SERVERS_SPEC,
        tcp_session: Some(TcpSessionBootstrap {
            host: "127.0.0.1",
            port: session_server.port,
            username: TRANSPORT_SESSION_USERNAME,
            room: TRANSPORT_SESSION_ROOM,
        }),
        loopback_session: None,
        attach_test_player: true,
        drop_file_paths_spec: None,
        drop_target: None,
    };
    let launch_result = launch_syncplay_gui_with_retry(driver, binary_path, launch, timeout);
    let (mut child, window) = match launch_result {
        Ok(pair) => pair,
        Err(error) => {
            let release_error = session_server.release("missing-media continuation");
            return match release_error {
                Ok(()) => Err(format!(
                    "failed to launch missing-media continuation segment for native smoke: {error}"
                )),
                Err(release_error) => Err(format!(
                    "failed to launch missing-media continuation segment for native smoke: {error}; {release_error}"
                )),
            };
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
                "view: setup",
                "view: setup",
                "view: room",
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
            "view: room",
            "Window",
            "Show Users",
            step_timeout,
        )?;

        let hello = session_server.recv_hello(step_timeout, "missing-media continuation")?;
        if !hello.contains("\"Hello\"") {
            return Err(format!(
                "missing-media continuation mock TCP server did not receive an expected startup hello payload: {hello:?}"
            ));
        }
        wait_for_main_window_user_row_name(
            driver,
            window,
            "self=no, ready=yes, controller=yes",
            step_timeout,
        )
        .map_err(|error| format!("transport initial remote row: {error}"))?;

        navigate_to_view_with_fallback(
            driver,
            window,
            "Media Search",
            "view: setup",
            "File",
            "Open Media Search",
            step_timeout,
        )?;
        wait_for_accessible_name(
            driver,
            window,
            &continue_media_search_path.display().to_string(),
            step_timeout,
        )?;
        invoke_named_control_with_wait(
            driver,
            window,
            "Search Missing Media",
            NativeControlKind::Button,
            step_timeout,
        )?;
        wait_for_pending_operation_to_finish(
            driver,
            window,
            "pending: search-missing-media",
            step_timeout,
        )?;
        if wait_for_accessible_name(driver, window, "view: room", Duration::from_millis(1_200))
            .is_err()
        {
            wait_for_accessible_name(driver, window, "view: setup", step_timeout)?;
            navigate_to_view_with_fallback(
                driver,
                window,
                "Main Window",
                "view: room",
                "Window",
                "Show Users",
                step_timeout,
            )?;
        }
        wait_for_main_window_user_row_name(
            driver,
            window,
            "self=no, ready=yes, controller=yes",
            step_timeout,
        )?;
        steps.push("main-window-missing-media-continue-session".to_owned());

        driver.close_window(window)?;
        wait_for_process_exit(&mut child, timeout)?;
        Ok(steps)
    })();

    if outcome.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }

    let release_error = session_server.release("missing-media continuation");
    match outcome {
        Ok(steps) => match release_error {
            Ok(()) => Ok(steps),
            Err(release_error) => Err(release_error),
        },
        Err(error) => match release_error {
            Ok(()) => Err(error),
            Err(release_error) => Err(format!("{error}; {release_error}")),
        },
    }
}
