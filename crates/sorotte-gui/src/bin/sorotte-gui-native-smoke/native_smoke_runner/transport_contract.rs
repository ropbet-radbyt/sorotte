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
        MODAL_PLAYER_SETUP_RETRY_AUTOMATION_ID,
        NativeControlKind::Button,
        true,
        timeout,
    )?;
    invoke_named_control_with_wait(
        driver,
        window,
        MODAL_PLAYER_SETUP_OPEN_SETTINGS_AUTOMATION_ID,
        NativeControlKind::Button,
        timeout,
    )?;
    wait_for_accessible_name(driver, window, "view: setup", timeout)?;
    wait_for_accessible_name(driver, window, "modal: (none)", timeout)?;
    Ok(true)
}

pub(super) fn verify_transport_reconnect_contract<D: NativeGuiDriver>(
    driver: &D,
    binary_path: &Path,
    temp_root: &Path,
    media_search_browse_path: &Path,
    open_media_file_path: &Path,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let primary_server = start_phased_mock_session_server(&[
        r#"{"Hello":{"username":"smoke-user","room":{"name":"smoke-room"},"version":"1.7.5","features":{"chat":true,"readiness":true,"sharedPlaylists":true}}}"#,
    ])?;

    let transport_config_path = temp_root.join("sorotte-native-smoke-transport.ini");
    let _ = fs::remove_file(&transport_config_path);
    seed_native_smoke_config(&transport_config_path)?;
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(
        &transport_config_path,
        &StoredClientSettingsMvp {
            host: Some("127.0.0.1".to_owned()),
            port: Some(primary_server.port),
            username: Some(TRANSPORT_SESSION_USERNAME.to_owned()),
            room: Some(TRANSPORT_SESSION_ROOM.to_owned()),
            shared_playlist_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        },
    )
    .map_err(|error| {
        format!(
            "failed to prepare transport reconnect config {}: {error}",
            transport_config_path.display()
        )
    })?;
    let public_servers_spec = format!("[['Primary', '{}']]", primary_server.address);
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

    let launch_result = launch_sorotte_gui_with_retry(driver, binary_path, launch, timeout);
    let (mut child, window) = match launch_result {
        Ok(pair) => pair,
        Err(error) => {
            let primary_release = primary_server.release("primary");
            let mut combined_error =
                format!("failed to launch transport parity segment for native smoke: {error}");
            if let Err(release_error) = primary_release {
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
                MODAL_TLS_TRUST_AUTOMATION_ID,
                NativeControlKind::Button,
                step_timeout,
            )?;
            wait_for_accessible_name(driver, window, "modal: (none)", step_timeout)?;
        }
        if dismiss_existing_config_player_setup_modal(driver, window, step_timeout)? {
            steps.push("transport-player-setup-modal".to_owned());
        }
        navigate_to_view_with_wait(
            driver,
            window,
            ROOM_SURFACE_AUTOMATION_ID,
            "view: room",
            step_timeout,
        )?;

        let first_hello = primary_server.recv_hello(step_timeout, "primary")?;
        if !first_hello.contains("\"Hello\"") {
            return Err(format!(
                "primary mock TCP server did not receive an expected startup hello payload: {first_hello:?}"
            ));
        }
        wait_for_shared_playlist_visible(driver, window, step_timeout)?;
        steps.push("transport-saved-config-startup".to_owned());

        driver.close_window(window)?;
        wait_for_process_exit(&mut child, timeout)?;
        Ok(steps)
    })();

    if outcome.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }

    let primary_release = primary_server.release("primary");
    let mut release_errors = Vec::new();
    if let Err(error) = primary_release {
        release_errors.push(error);
    }

    match outcome {
        Ok(steps) if release_errors.is_empty() => Ok(steps),
        Ok(_) => Err(release_errors.join("; ")),
        Err(error) if release_errors.is_empty() => Err(error),
        Err(error) => Err(format!("{error}; {}", release_errors.join("; "))),
    }
}
