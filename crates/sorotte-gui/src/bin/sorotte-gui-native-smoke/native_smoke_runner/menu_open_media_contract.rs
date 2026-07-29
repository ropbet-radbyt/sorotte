use super::*;

pub(super) fn verify_menu_open_media_contract<D: NativeGuiDriver>(
    driver: &D,
    binary_path: &Path,
    temp_root: &Path,
    media_search_browse_path: &Path,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let step_timeout = timeout.min(Duration::from_millis(6_000));
    let config_path = temp_root.join("menu-open-media.ini");
    let media_path = temp_root.join("menu-open-media-target.mkv");
    let player_observation_path = temp_root.join("menu-open-media-player.jsonl");
    fs::write(&media_path, b"menu-open-media-target")
        .map_err(|error| format!("failed to create menu Open Media target: {error}"))?;
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(
        &config_path,
        &StoredClientSettingsMvp::default(),
    )
    .map_err(|error| {
        format!(
            "failed to seed menu Open Media config {}: {error}",
            config_path.display()
        )
    })?;

    let launch = GuiLaunchConfig {
        config_path: &config_path,
        media_search_browse_path,
        open_media_file_path: &media_path,
        public_servers_spec: DEFAULT_PUBLIC_SERVERS_SPEC,
        network_mode: NativeNetworkMode::Detached,
        attach_test_player: true,
        drop_file_paths_spec: None,
        drop_target: None,
    };
    let (mut child, window) = launch_sorotte_gui_with_retry_and_test_overrides(
        driver,
        binary_path,
        launch,
        timeout,
        GuiLaunchTestOverrides {
            disable_startup_saved_connect: true,
            test_player_observation_path: Some(&player_observation_path),
            ..GuiLaunchTestOverrides::default()
        },
    )?;
    let outcome = (|| -> Result<Vec<String>, String> {
        wait_for_any_accessible_name(driver, window, &["view: setup", "view: room"], step_timeout)?;

        verify_menu_action_enabled_state_by_id(
            driver,
            window,
            FILE_MENU_AUTOMATION_ID,
            OPEN_MEDIA_MENU_AUTOMATION_ID,
            true,
            step_timeout,
        )?;
        let mut steps = vec!["menu-open-media-enabled".to_owned()];

        invoke_menu_action_by_id_with_wait(
            driver,
            window,
            FILE_MENU_AUTOMATION_ID,
            OPEN_MEDIA_MENU_AUTOMATION_ID,
            step_timeout,
        )?;
        steps.push("menu-open-media-invoked-by-automation-id".to_owned());

        wait_for_accessible_name(driver, window, "view: room", step_timeout)?;
        wait_for_test_player_open_file_observation(
            &player_observation_path,
            &media_path.display().to_string(),
            step_timeout,
        )?;
        steps.push("menu-open-media-runtime-observed".to_owned());

        driver.close_window(window)?;
        wait_for_process_exit(&mut child, timeout)?;
        Ok(steps)
    })();

    if let Err(error) = &outcome {
        capture_native_failure_artifacts(driver, window, "menu-open-media", error);
        let _ = child.kill();
        let _ = child.wait();
    }
    outcome
}

fn wait_for_test_player_open_file_observation(
    observation_path: &Path,
    expected_media_path: &str,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        match fs::read_to_string(observation_path) {
            Ok(payload) => {
                let observed = payload.lines().any(|line| {
                    serde_json::from_str::<serde_json::Value>(line)
                        .ok()
                        .is_some_and(|value| {
                            value.get("event").and_then(serde_json::Value::as_str)
                                == Some("open_file")
                                && value.get("path").and_then(serde_json::Value::as_str)
                                    == Some(expected_media_path)
                        })
                });
                if observed {
                    return Ok(());
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "failed to read test-player observation {}: {error}",
                    observation_path.display()
                ));
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for the deterministic test player to receive Open Media path {expected_media_path:?}"
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}
