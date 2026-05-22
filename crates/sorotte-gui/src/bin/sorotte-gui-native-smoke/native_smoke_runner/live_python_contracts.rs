use super::*;

fn dismiss_existing_config_player_setup_modal<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    timeout: Duration,
) -> Result<bool, String> {
    if wait_for_accessible_name(
        driver,
        window,
        "modal: player-setup",
        timeout.min(Duration::from_millis(800)),
    )
    .is_err()
    {
        return Ok(false);
    }
    wait_for_accessible_name(driver, window, "Choose mpv.exe", timeout)?;
    wait_for_accessible_name(driver, window, "Open Settings", timeout)?;
    wait_for_named_control_enabled_state(
        driver,
        window,
        "Retry mpv",
        NativeControlKind::Button,
        true,
        timeout,
    )?;
    invoke_named_control_with_wait(
        driver,
        window,
        "Open Settings",
        NativeControlKind::Button,
        timeout,
    )?;
    wait_for_accessible_name(driver, window, "view: setup", timeout)?;
    wait_for_accessible_name(driver, window, "modal: (none)", timeout)?;
    Ok(true)
}

pub(super) fn verify_live_python_peer_connect_contract<D: NativeGuiDriver>(
    driver: &D,
    binary_path: &Path,
    temp_root: &Path,
    media_search_browse_path: &Path,
    open_media_file_path: &Path,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let mut python_harness = LegacyServerPythonPeerHarness::spawn(
        LIVE_PYTHON_INTEROP_PEER_USERNAME,
        LIVE_PYTHON_INTEROP_ROOM,
    )
    .map_err(|error| format!("failed to start live Python interop harness: {error}"))?;
    let interop_config_path = temp_root.join("sorotte-native-smoke-python-interop.ini");
    let _ = fs::remove_file(&interop_config_path);
    seed_native_smoke_config(&interop_config_path)?;
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(
        &interop_config_path,
        &StoredClientSettingsMvp {
            shared_playlist_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        },
    )
    .map_err(|error| {
        format!(
            "failed to enable shared playlists in native Python interop config {}: {error}",
            interop_config_path.display()
        )
    })?;
    let launch = GuiLaunchConfig {
        config_path: &interop_config_path,
        media_search_browse_path,
        open_media_file_path,
        public_servers_spec: DEFAULT_PUBLIC_SERVERS_SPEC,
        tcp_session: Some(TcpSessionBootstrap {
            host: python_harness.host(),
            port: python_harness.port(),
            username: LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
            room: LIVE_PYTHON_INTEROP_ROOM,
        }),
        loopback_session: None,
        attach_test_player: false,
        drop_file_paths_spec: None,
        drop_target: None,
    };

    let launch_result = launch_sorotte_gui_with_retry(driver, binary_path, launch, timeout);
    let (mut child, window) = match launch_result {
        Ok(pair) => pair,
        Err(error) => {
            let release = python_harness.shutdown();
            let mut combined_error =
                format!("failed to launch live Python interop segment for native smoke: {error}");
            if let Err(release_error) = release {
                combined_error.push_str("; ");
                combined_error.push_str(&release_error.to_string());
            }
            return Err(combined_error);
        }
    };

    let outcome = (|| -> Result<Vec<String>, String> {
        let step_timeout = timeout.min(Duration::from_millis(8_000));
        let mut steps = Vec::new();

        wait_for_any_accessible_name(driver, window, &["view: setup", "view: room"], step_timeout)?;
        if dismiss_existing_config_player_setup_modal(driver, window, step_timeout)? {
            steps.push("transport-python-peer-player-setup-modal".to_owned());
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
        wait_for_room_browser_visible(driver, window, step_timeout)?;
        navigate_to_view_with_fallback(
            driver,
            window,
            "Configuration",
            "view: setup",
            "Advanced",
            "Trusted Domains",
            step_timeout,
        )?;
        navigate_to_view_with_fallback(
            driver,
            window,
            "Main Window",
            "view: room",
            "Window",
            "Show Users",
            step_timeout,
        )?;
        wait_for_room_browser_visible(driver, window, step_timeout)?;
        wait_for_accessible_name(
            driver,
            window,
            LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
            step_timeout,
        )
        .map_err(|error| format!("live Python interop initial local row: {error}"))?;
        python_harness
            .start_peer_connected()
            .map_err(|error| format!("failed to connect live Python reference peer: {error}"))?;
        navigate_to_view_with_fallback(
            driver,
            window,
            "Configuration",
            "view: setup",
            "Advanced",
            "Trusted Domains",
            step_timeout,
        )?;
        navigate_to_view_with_fallback(
            driver,
            window,
            "Main Window",
            "view: room",
            "Window",
            "Show Users",
            step_timeout,
        )?;
        wait_for_room_browser_visible(driver, window, step_timeout)?;
        wait_for_accessible_name(
            driver,
            window,
            LIVE_PYTHON_INTEROP_PEER_USERNAME,
            step_timeout,
        )
        .map_err(|error| format!("live Python interop peer connect row: {error}"))?;
        steps.push("transport-python-peer-connect".to_owned());
        join_room_from_main_window(driver, window, LIVE_PYTHON_INTEROP_ALT_ROOM, step_timeout)?;
        thread::sleep(Duration::from_millis(500));
        wait_for_accessible_name(
            driver,
            window,
            LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
            step_timeout,
        )
        .map_err(|error| format!("live Python interop alternate-room local row: {error}"))?;
        steps.push("main-window-room-joined".to_owned());

        join_room_from_main_window(driver, window, LIVE_PYTHON_INTEROP_ROOM, step_timeout)?;
        wait_for_accessible_name(
            driver,
            window,
            LIVE_PYTHON_INTEROP_PEER_USERNAME,
            step_timeout,
        )
        .map_err(|error| format!("live Python interop room-switch peer row: {error}"))?;
        steps.push("main-window-room-switched".to_owned());

        python_harness
            .set_peer_ready(true)
            .map_err(|error| format!("failed to set Python reference peer ready=true: {error}"))?;
        python_harness
            .wait_for_peer_local_ready(true, step_timeout)
            .map_err(|error| {
                format!("python reference peer did not confirm ready=true: {error}")
            })?;
        python_harness
            .set_peer_ready(false)
            .map_err(|error| format!("failed to set Python reference peer ready=false: {error}"))?;
        python_harness
            .wait_for_peer_local_ready(false, step_timeout)
            .map_err(|error| {
                format!("python reference peer did not confirm ready=false: {error}")
            })?;
        steps.push("transport-python-peer-readiness".to_owned());

        python_harness
            .send_peer_chat_message(LIVE_PYTHON_INTEROP_PEER_CHAT_MESSAGE)
            .map_err(|error| format!("failed to send Python reference peer chat: {error}"))?;
        python_harness
            .wait_for_peer_observed_chat_message(
                LIVE_PYTHON_INTEROP_PEER_USERNAME,
                LIVE_PYTHON_INTEROP_PEER_CHAT_MESSAGE,
                step_timeout,
            )
            .map_err(|error| {
                format!("python reference peer did not confirm its own chat echo: {error}")
            })?;
        wait_for_visible_chat_message(
            driver,
            window,
            LIVE_PYTHON_INTEROP_PEER_USERNAME,
            LIVE_PYTHON_INTEROP_PEER_CHAT_MESSAGE,
            step_timeout,
        )?;
        steps.push("transport-python-peer-chat-peer-to-local".to_owned());

        if wait_for_shared_playlist_controls_enabled(driver, window, Duration::from_millis(500))
            .is_err()
        {
            navigate_to_view_with_fallback(
                driver,
                window,
                "Configuration",
                "view: setup",
                "Advanced",
                "Trusted Domains",
                step_timeout,
            )?;
            select_top_tab_with_wait(
                driver,
                window,
                "Playback & Search",
                "Shared Playlists",
                step_timeout,
            )?;
            invoke_named_control_with_wait(
                driver,
                window,
                "Shared Playlists",
                NativeControlKind::Any,
                step_timeout,
            )?;
            navigate_to_view_with_fallback(
                driver,
                window,
                "Room",
                "view: room",
                "Window",
                "Show Users",
                step_timeout,
            )?;
            wait_for_shared_playlist_controls_enabled(driver, window, step_timeout)?;
            steps.push("transport-python-peer-playlist-enable-setting".to_owned());
        }

        let initial_playlist = vec![
            LIVE_PYTHON_INTEROP_LOCAL_PLAYLIST_ENTRY_ONE.to_owned(),
            LIVE_PYTHON_INTEROP_LOCAL_PLAYLIST_ENTRY_TWO.to_owned(),
        ];
        wait_for_shared_playlist_visible(driver, window, step_timeout)?;
        python_harness
            .set_peer_playlist(&initial_playlist)
            .map_err(|error| format!("failed to seed Python reference peer playlist: {error}"))?;
        python_harness
            .wait_for_peer_playlist(&initial_playlist, step_timeout)
            .map_err(|error| {
                format!("python reference peer did not confirm the seeded playlist: {error}")
            })?;
        wait_for_shared_playlist_entry(
            driver,
            window,
            LIVE_PYTHON_INTEROP_LOCAL_PLAYLIST_ENTRY_ONE,
            step_timeout,
        )?;
        python_harness
            .set_peer_playlist_index(1)
            .map_err(|error| format!("failed to set seeded Python playlist index: {error}"))?;
        python_harness
            .wait_for_peer_playlist_index(1, step_timeout)
            .map_err(|error| {
                format!("python reference peer did not confirm the seeded playlist index: {error}")
            })?;
        wait_for_shared_playlist_entry(
            driver,
            window,
            LIVE_PYTHON_INTEROP_LOCAL_PLAYLIST_ENTRY_TWO,
            step_timeout,
        )?;

        let peer_playlist = vec![
            LIVE_PYTHON_INTEROP_PEER_PLAYLIST_ENTRY_ONE.to_owned(),
            LIVE_PYTHON_INTEROP_PEER_PLAYLIST_ENTRY_TWO.to_owned(),
        ];
        python_harness
            .set_peer_playlist(&peer_playlist)
            .map_err(|error| format!("failed to set Python reference peer playlist: {error}"))?;
        python_harness
            .wait_for_peer_playlist(&peer_playlist, step_timeout)
            .map_err(|error| {
                format!("python reference peer did not confirm its playlist update: {error}")
            })?;
        wait_for_shared_playlist_entry(
            driver,
            window,
            LIVE_PYTHON_INTEROP_PEER_PLAYLIST_ENTRY_ONE,
            step_timeout,
        )?;
        wait_for_shared_playlist_entry(
            driver,
            window,
            LIVE_PYTHON_INTEROP_PEER_PLAYLIST_ENTRY_TWO,
            step_timeout,
        )?;
        python_harness.set_peer_playlist_index(1).map_err(|error| {
            format!("failed to set Python reference peer playlist index: {error}")
        })?;
        python_harness
            .wait_for_peer_playlist_index(1, step_timeout)
            .map_err(|error| {
                format!("python reference peer did not confirm its playlist index update: {error}")
            })?;
        steps.push("transport-python-peer-playlist-peer-to-local".to_owned());

        python_harness
            .disconnect_peer()
            .map_err(|error| format!("failed to disconnect Python reference peer: {error}"))?;
        wait_for_named_control_count(
            driver,
            window,
            LIVE_PYTHON_INTEROP_PEER_USERNAME,
            NativeControlKind::Any,
            0,
            step_timeout,
        )?;
        wait_for_accessible_name(
            driver,
            window,
            LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
            step_timeout,
        )
        .map_err(|error| format!("live Python interop peer disconnect local row: {error}"))?;
        steps.push("transport-python-peer-disconnect".to_owned());

        python_harness
            .start_peer_connected()
            .map_err(|error| format!("failed to reconnect live Python reference peer: {error}"))?;
        wait_for_accessible_name(
            driver,
            window,
            LIVE_PYTHON_INTEROP_PEER_USERNAME,
            step_timeout,
        )
        .map_err(|error| format!("live Python interop peer reconnect row: {error}"))?;
        steps.push("transport-python-peer-reconnect".to_owned());

        python_harness
            .send_peer_chat_message(LIVE_PYTHON_INTEROP_PEER_RECONNECT_CHAT_MESSAGE)
            .map_err(|error| {
                format!("failed to send Python reference peer reconnect chat: {error}")
            })?;
        python_harness
            .wait_for_peer_observed_chat_message(
                LIVE_PYTHON_INTEROP_PEER_USERNAME,
                LIVE_PYTHON_INTEROP_PEER_RECONNECT_CHAT_MESSAGE,
                step_timeout,
            )
            .map_err(|error| {
                format!("python reference peer did not confirm its reconnect chat echo: {error}")
            })?;
        wait_for_visible_chat_message(
            driver,
            window,
            LIVE_PYTHON_INTEROP_PEER_USERNAME,
            LIVE_PYTHON_INTEROP_PEER_RECONNECT_CHAT_MESSAGE,
            step_timeout,
        )?;
        steps.push("transport-python-peer-reconnect-peer-to-local".to_owned());

        driver.close_window(window)?;
        wait_for_process_exit(&mut child, timeout)?;
        Ok(steps)
    })();

    if outcome.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }

    let release = python_harness.shutdown();
    match (outcome, release) {
        (Ok(steps), Ok(())) => Ok(steps),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error.to_string()),
        (Err(error), Err(release_error)) => Err(format!("{error}; {release_error}")),
    }
}

pub(super) fn verify_live_python_peer_controlled_room_contract<D: NativeGuiDriver>(
    driver: &D,
    binary_path: &Path,
    temp_root: &Path,
    media_search_browse_path: &Path,
    open_media_file_path: &Path,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let mut python_harness = LegacyServerPythonPeerHarness::spawn(
        LIVE_PYTHON_INTEROP_PEER_USERNAME,
        LIVE_PYTHON_INTEROP_CONTROLLED_ROOM,
    )
    .map_err(|error| format!("failed to start live Python controlled-room harness: {error}"))?;
    let interop_config_path = temp_root.join("sorotte-native-smoke-python-controlled-room.ini");
    let _ = fs::remove_file(&interop_config_path);
    seed_native_smoke_config(&interop_config_path)?;
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(
        &interop_config_path,
        &StoredClientSettingsMvp {
            shared_playlist_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        },
    )
    .map_err(|error| {
        format!(
            "failed to enable shared playlists in native Python controlled-room config {}: {error}",
            interop_config_path.display()
        )
    })?;
    let launch = GuiLaunchConfig {
        config_path: &interop_config_path,
        media_search_browse_path,
        open_media_file_path,
        public_servers_spec: DEFAULT_PUBLIC_SERVERS_SPEC,
        tcp_session: Some(TcpSessionBootstrap {
            host: python_harness.host(),
            port: python_harness.port(),
            username: LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
            room: LIVE_PYTHON_INTEROP_CONTROLLED_ROOM_INPUT,
        }),
        loopback_session: None,
        attach_test_player: false,
        drop_file_paths_spec: None,
        drop_target: None,
    };

    let launch_result = launch_sorotte_gui_with_retry(driver, binary_path, launch, timeout);
    let (mut child, window) = match launch_result {
        Ok(pair) => pair,
        Err(error) => {
            let release = python_harness.shutdown();
            let mut combined_error = format!(
                "failed to launch live Python controlled-room segment for native smoke: {error}"
            );
            if let Err(release_error) = release {
                combined_error.push_str("; ");
                combined_error.push_str(&release_error.to_string());
            }
            return Err(combined_error);
        }
    };

    let outcome = (|| -> Result<Vec<String>, String> {
        let step_timeout = timeout.min(Duration::from_millis(8_000));
        let mut steps = Vec::new();

        wait_for_any_accessible_name(driver, window, &["view: setup", "view: room"], step_timeout)?;
        if dismiss_existing_config_player_setup_modal(driver, window, step_timeout)? {
            steps.push("transport-python-peer-controlled-room-player-setup-modal".to_owned());
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
        wait_for_accessible_name(
            driver,
            window,
            LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
            step_timeout,
        )?;

        python_harness.start_peer_connected().map_err(|error| {
            format!("failed to connect live Python reference peer in controlled room: {error}")
        })?;
        wait_for_accessible_name(
            driver,
            window,
            LIVE_PYTHON_INTEROP_PEER_USERNAME,
            step_timeout,
        )?;
        steps.push("transport-python-peer-controlled-room-connect".to_owned());

        python_harness
            .wait_for_peer_observed_user_controller(
                LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
                true,
                step_timeout,
            )
            .map_err(|error| {
                format!(
                    "python reference peer did not observe GUI controller status in controlled room: {error}"
                )
            })?;
        python_harness
            .wait_for_peer_local_controller(false, step_timeout)
            .map_err(|error| {
                format!(
                    "python reference peer did not remain non-controller in controlled room: {error}"
                )
            })?;
        steps.push("transport-python-peer-controlled-room-auth".to_owned());

        wait_for_shared_playlist_controls_enabled(driver, window, step_timeout)?;
        steps.push("transport-python-peer-controlled-room-playlist-enabled".to_owned());

        driver.close_window(window)?;
        wait_for_process_exit(&mut child, timeout)?;
        Ok(steps)
    })();

    if outcome.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }

    let release = python_harness.shutdown();
    match (outcome, release) {
        (Ok(steps), Ok(())) => Ok(steps),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error.to_string()),
        (Err(error), Err(release_error)) => Err(format!("{error}; {release_error}")),
    }
}
