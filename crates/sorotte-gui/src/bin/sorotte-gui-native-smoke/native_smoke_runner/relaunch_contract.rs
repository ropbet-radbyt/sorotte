use super::*;

pub(super) fn verify_relaunch_config_reload_contract<D: NativeGuiDriver>(
    driver: &D,
    binary_path: &Path,
    config_path: &Path,
    media_search_browse_path: &Path,
    open_media_file_path: &Path,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let media_search_directory_value = media_search_browse_path.display().to_string();
    let _ = wait_for_saved_configuration(config_path, &media_search_directory_value, timeout)?;
    let gui_state_root = config_path.parent().ok_or_else(|| {
        format!(
            "native smoke config path {} had no parent directory for GUI-state checks",
            config_path.display()
        )
    })?;
    let launch = GuiLaunchConfig {
        config_path,
        media_search_browse_path,
        open_media_file_path,
        public_servers_spec: DEFAULT_PUBLIC_SERVERS_SPEC,
        network_mode: NativeNetworkMode::Detached,
        attach_test_player: false,
        drop_file_paths_spec: None,
        drop_target: None,
    };
    let (mut child, window) = launch_sorotte_gui_with_retry(driver, binary_path, launch, timeout)?;

    let outcome = (|| -> Result<Vec<String>, String> {
        let step_timeout = timeout.min(Duration::from_millis(4_000));
        let mut steps = Vec::new();

        let _initial_state = wait_for_any_accessible_name(
            driver,
            window,
            &["modal: tls-certificate-prompt", "view: setup", "view: room"],
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

        let initial_view = wait_for_any_accessible_name(
            driver,
            window,
            &["view: setup", "view: room"],
            step_timeout,
        )?;
        if initial_view != "view: setup" {
            return Err(format!(
                "expected relaunch to restore the setup view, got {initial_view:?}"
            ));
        }
        if wait_for_accessible_name(
            driver,
            window,
            "modal: player-setup",
            step_timeout.min(Duration::from_millis(800)),
        )
        .is_ok()
        {
            wait_for_accessible_name(driver, window, "Choose mpv.exe", step_timeout)?;
            wait_for_accessible_name(driver, window, "Open Settings", step_timeout)?;
            wait_for_named_control_enabled_state(
                driver,
                window,
                MODAL_PLAYER_SETUP_RETRY_AUTOMATION_ID,
                NativeControlKind::Button,
                true,
                step_timeout,
            )?;
            invoke_named_control_with_wait(
                driver,
                window,
                MODAL_PLAYER_SETUP_OPEN_SETTINGS_AUTOMATION_ID,
                NativeControlKind::Button,
                step_timeout,
            )?;
            wait_for_accessible_name(driver, window, "view: setup", step_timeout)?;
            wait_for_accessible_name(driver, window, "modal: (none)", step_timeout)?;
            steps.push("gui-state-player-setup-modal".to_owned());
        }
        wait_for_accessible_name(driver, window, "view: setup", step_timeout)?;
        wait_for_accessible_name(driver, window, "modal: (none)", step_timeout)?;
        navigate_to_view_with_fallback(
            driver,
            window,
            "Media Search",
            "view: setup",
            "File",
            "Open Media Search",
            step_timeout,
        )?;
        wait_for_accessible_name(driver, window, "First File Timeout: 3.00s", step_timeout)?;
        wait_for_accessible_name(driver, window, "Search Timeout: 30.00s", step_timeout)?;
        wait_for_accessible_name(driver, window, &media_search_directory_value, step_timeout)?;
        steps.push("gui-state-restored".to_owned());
        navigate_to_view_with_fallback(
            driver,
            window,
            SETUP_SURFACE_AUTOMATION_ID,
            "view: setup",
            "Advanced",
            "Trusted Domains",
            step_timeout,
        )?;
        select_top_tab_with_wait(
            driver,
            window,
            CONFIG_CONNECTION_TAB_AUTOMATION_ID,
            "Host",
            step_timeout,
        )?;
        let editable_count = driver.editable_text_input_count(window)?;
        if editable_count < 6 {
            return Err(format!(
                "expected at least 6 editable configuration text fields after relaunch, found {editable_count}"
            ));
        }
        for (automation_id, expected_value) in [
            (CONFIG_USERNAME_AUTOMATION_ID, CONFIG_USERNAME_VALUE),
            (CONFIG_ROOM_AUTOMATION_ID, CONFIG_ROOM_VALUE),
            (CONFIG_PLAYER_PATH_AUTOMATION_ID, CONFIG_PLAYER_PATH_VALUE),
        ] {
            wait_for_named_edit_value(driver, window, automation_id, expected_value, step_timeout)?;
        }
        steps.push("config-reload-persisted".to_owned());
        select_top_tab_with_wait(
            driver,
            window,
            CONFIG_PRIVACY_CHAT_TAB_AUTOMATION_ID,
            "Trusted Domains Only",
            step_timeout,
        )?;
        wait_for_accessible_name(driver, window, "Chat Input", step_timeout)?;
        select_top_tab_with_wait(
            driver,
            window,
            CONFIG_PLAYBACK_SEARCH_TAB_AUTOMATION_ID,
            "Rewind On Desync",
            step_timeout,
        )?;
        select_top_tab_with_wait(
            driver,
            window,
            CONFIG_INTERFACE_SYSTEM_TAB_AUTOMATION_ID,
            "Show OSD",
            step_timeout,
        )?;
        wait_for_accessible_name(driver, window, "Language", step_timeout)?;
        steps.push("trusted-domains-persisted".to_owned());

        navigate_to_view_with_fallback(
            driver,
            window,
            SETUP_SURFACE_AUTOMATION_ID,
            "view: setup",
            "Advanced",
            "Trusted Domains",
            step_timeout,
        )?;
        invoke_named_control_with_wait(
            driver,
            window,
            CONFIG_CLEAR_GUI_DATA_AUTOMATION_ID,
            NativeControlKind::Button,
            step_timeout,
        )?;
        invoke_named_control_with_wait(
            driver,
            window,
            CONFIG_CONFIRM_CLEAR_GUI_DATA_AUTOMATION_ID,
            NativeControlKind::Button,
            step_timeout,
        )?;
        steps.push("clear-gui-data-confirmed".to_owned());
        wait_for_pending_operation_to_finish(
            driver,
            window,
            "pending: clear-gui-data",
            step_timeout,
        )?;
        let clear_deadline = Instant::now() + step_timeout;
        while config_path.exists() && Instant::now() < clear_deadline {
            thread::sleep(Duration::from_millis(50));
        }
        steps.push("clear-gui-data-completed".to_owned());
        if config_path.exists() {
            return Err(format!(
                "clear-GUI-data did not remove config file {}",
                config_path.display()
            ));
        }
        for store_name in ["MainWindow", "Interface", "MediaBrowseDialog"] {
            let store_path = legacy_gui_qsettings_store_path(gui_state_root, store_name);
            if store_path.exists() {
                return Err(format!(
                    "clear-GUI-data did not remove legacy GUI state file {}",
                    store_path.display()
                ));
            }
        }
        select_top_tab_with_wait(
            driver,
            window,
            CONFIG_CONNECTION_TAB_AUTOMATION_ID,
            "Host",
            step_timeout,
        )?;
        for automation_id in [
            CONFIG_HOST_AUTOMATION_ID,
            CONFIG_PORT_AUTOMATION_ID,
            CONFIG_USERNAME_AUTOMATION_ID,
            CONFIG_ROOM_AUTOMATION_ID,
            CONFIG_PLAYER_PATH_AUTOMATION_ID,
        ] {
            let value = driver.get_named_edit_value(window, automation_id)?;
            if !value.is_empty() && value != "(unset)" {
                return Err(format!(
                    "expected first-run configuration field {automation_id:?} to be blank after clear-GUI-data, got {value:?}"
                ));
            }
        }
        steps.push("clear-gui-data-first-run".to_owned());

        driver.close_window(window)?;
        wait_for_process_exit(&mut child, timeout)?;
        steps.push("clear-gui-data-session-close".to_owned());

        let first_run_launch = GuiLaunchConfig {
            config_path,
            media_search_browse_path,
            open_media_file_path,
            public_servers_spec: DEFAULT_PUBLIC_SERVERS_SPEC,
            network_mode: NativeNetworkMode::Detached,
            attach_test_player: false,
            drop_file_paths_spec: None,
            drop_target: None,
        };
        let (mut first_run_child, first_run_window) =
            launch_sorotte_gui_with_retry(driver, binary_path, first_run_launch, timeout)?;
        let first_run_outcome = (|| -> Result<(), String> {
            let _initial_state = wait_for_any_accessible_name(
                driver,
                first_run_window,
                &["modal: tls-certificate-prompt", "view: setup", "view: room"],
                step_timeout,
            )?;
            if wait_for_accessible_name(
                driver,
                first_run_window,
                "modal: tls-certificate-prompt",
                step_timeout.min(Duration::from_millis(800)),
            )
            .is_ok()
            {
                invoke_named_control_with_wait(
                    driver,
                    first_run_window,
                    MODAL_TLS_TRUST_AUTOMATION_ID,
                    NativeControlKind::Button,
                    step_timeout,
                )?;
                let _ = wait_for_any_accessible_name(
                    driver,
                    first_run_window,
                    &["modal: (none)", "modal: player-setup"],
                    step_timeout,
                )?;
            }

            let first_run_view = wait_for_any_accessible_name(
                driver,
                first_run_window,
                &["view: setup", "view: room"],
                step_timeout,
            )?;
            if first_run_view != "view: setup" {
                return Err(format!(
                    "expected first launch after clear-GUI-data to return to setup, got {first_run_view:?}"
                ));
            }
            wait_for_accessible_name(
                driver,
                first_run_window,
                "modal: player-setup",
                step_timeout,
            )?;
            wait_for_accessible_name(driver, first_run_window, "Auto-detect mpv", step_timeout)?;
            wait_for_accessible_name(driver, first_run_window, "Choose mpv.exe", step_timeout)?;
            wait_for_accessible_name(driver, first_run_window, "Open Settings", step_timeout)?;
            wait_for_named_control_enabled_state(
                driver,
                first_run_window,
                CONFIG_CONNECT_ONCE_AUTOMATION_ID,
                NativeControlKind::Button,
                false,
                step_timeout,
            )?;
            wait_for_named_control_enabled_state(
                driver,
                first_run_window,
                MODAL_PLAYER_SETUP_RETRY_AUTOMATION_ID,
                NativeControlKind::Button,
                false,
                step_timeout,
            )?;
            Ok(())
        })();
        if let Err(error) = &first_run_outcome {
            capture_native_failure_artifacts(driver, first_run_window, "relaunch-first-run", error);
            let _ = first_run_child.kill();
            let _ = first_run_child.wait();
        }
        first_run_outcome?;
        driver.close_window(first_run_window)?;
        wait_for_process_exit(&mut first_run_child, timeout)?;
        steps.push("clear-gui-data-relaunch-first-run".to_owned());
        steps.push("clear-gui-data-player-setup-blocked".to_owned());

        let migration_settings = StoredClientSettingsMvp {
            host: Some(MIGRATION_INI_SERVER_HOST.to_owned()),
            port: Some(MIGRATION_INI_SERVER_PORT.parse().unwrap()),
            username: Some(CONFIG_USERNAME_VALUE.to_owned()),
            room: Some(CONFIG_ROOM_VALUE.to_owned()),
            player_path: Some(CONFIG_PLAYER_PATH_VALUE.to_owned()),
            public_servers: Some(vec![(
                MIGRATION_INI_SERVER_LABEL.to_owned(),
                MIGRATION_INI_SERVER_ADDRESS.to_owned(),
            )]),
            ..StoredClientSettingsMvp::default()
        };
        upsert_sorotte_ini_stored_client_settings_mvp_at_path(config_path, &migration_settings)
            .map_err(|error| {
                format!(
                    "failed to seed config-migration config {}: {error}",
                    config_path.display()
                )
            })?;
        seed_native_smoke_gui_state(
            gui_state_root,
            Some("public-servers"),
            Some(MIGRATION_GUI_SERVER_ADDRESS),
            &[(
                MIGRATION_GUI_SERVER_LABEL.to_owned(),
                MIGRATION_GUI_SERVER_ADDRESS.to_owned(),
            )],
            None,
        )?;

        let migration_launch = GuiLaunchConfig {
            config_path,
            media_search_browse_path,
            open_media_file_path,
            public_servers_spec: DEFAULT_PUBLIC_SERVERS_SPEC,
            network_mode: NativeNetworkMode::Detached,
            attach_test_player: false,
            drop_file_paths_spec: None,
            drop_target: None,
        };
        let (mut migration_child, migration_window) =
            launch_sorotte_gui_with_retry(driver, binary_path, migration_launch, timeout)?;
        let migration_outcome = (|| -> Result<(), String> {
            let _migration_initial_state = wait_for_any_accessible_name(
                driver,
                migration_window,
                &["modal: tls-certificate-prompt", "view: setup", "view: room"],
                step_timeout,
            )?;
            if wait_for_accessible_name(
                driver,
                migration_window,
                "modal: tls-certificate-prompt",
                step_timeout.min(Duration::from_millis(800)),
            )
            .is_ok()
            {
                invoke_named_control_with_wait(
                    driver,
                    migration_window,
                    MODAL_TLS_TRUST_AUTOMATION_ID,
                    NativeControlKind::Button,
                    step_timeout,
                )?;
                wait_for_accessible_name(driver, migration_window, "modal: (none)", step_timeout)?;
            }

            let migration_view = wait_for_any_accessible_name(
                driver,
                migration_window,
                &["view: setup", "view: room"],
                step_timeout,
            )?;
            if migration_view != "view: setup" {
                return Err(format!(
                    "expected config-migration launch to restore setup, got {migration_view:?}"
                ));
            }
            wait_for_named_control_count(
                driver,
                migration_window,
                MIGRATION_GUI_SERVER_ROW_NAME,
                NativeControlKind::Any,
                1,
                step_timeout,
            )?;
            wait_for_named_control_count(
                driver,
                migration_window,
                MIGRATION_INI_SERVER_ROW_NAME,
                NativeControlKind::Any,
                0,
                step_timeout,
            )?;
            navigate_to_view_with_fallback(
                driver,
                migration_window,
                SETUP_SURFACE_AUTOMATION_ID,
                "view: setup",
                "Advanced",
                "Trusted Domains",
                step_timeout,
            )?;
            wait_for_named_edit_value(
                driver,
                migration_window,
                CONFIG_HOST_AUTOMATION_ID,
                "gui-only.example",
                step_timeout,
            )?;
            wait_for_named_edit_value(
                driver,
                migration_window,
                CONFIG_PORT_AUTOMATION_ID,
                "9002",
                step_timeout,
            )?;
            Ok(())
        })();
        if let Err(error) = &migration_outcome {
            capture_native_failure_artifacts(driver, migration_window, "relaunch-migration", error);
            let _ = migration_child.kill();
            let _ = migration_child.wait();
        }
        migration_outcome?;
        driver.close_window(migration_window)?;
        wait_for_process_exit(&mut migration_child, timeout)?;
        steps.push("config-migration-predictable".to_owned());

        Ok(steps)
    })();

    if let Err(error) = &outcome {
        capture_native_failure_artifacts(driver, window, "relaunch", error);
        let _ = child.kill();
        let _ = child.wait();
    }
    outcome
}
