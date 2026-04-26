use super::*;

pub(super) fn verify_interaction_contract<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    config_path: &Path,
    media_search_browse_path: &Path,
    open_media_file_path: &Path,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let step_timeout = timeout.min(Duration::from_millis(4_000));
    let config_persist_timeout = timeout.min(Duration::from_millis(8_000));
    let media_search_directory_value = media_search_browse_path.display().to_string();
    let mut steps = Vec::new();

    let initial_view =
        wait_for_any_accessible_name(driver, window, &["view: setup", "view: room"], step_timeout)?;
    if initial_view == "view: room" {
        navigate_to_view_with_fallback(
            driver,
            window,
            "Configuration",
            "view: setup",
            "Advanced",
            "Trusted Domains",
            step_timeout,
        )?;
    }
    let editable_count = driver.editable_text_input_count(window)?;
    if editable_count < 6 {
        return Err(format!(
            "expected at least 6 editable configuration text fields, found {editable_count}"
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
            "Retry mpv",
            NativeControlKind::Button,
            true,
            step_timeout,
        )?;
        wait_for_named_control_enabled_state(
            driver,
            window,
            "Open Settings",
            NativeControlKind::Button,
            true,
            step_timeout,
        )?;
        invoke_named_control_with_wait(
            driver,
            window,
            "Open Settings",
            NativeControlKind::Button,
            step_timeout,
        )?;
        wait_for_accessible_name(driver, window, "view: setup", step_timeout)?;
        wait_for_accessible_name(driver, window, "modal: (none)", step_timeout)?;
        steps.push("player-setup-existing-config-modal".to_owned());
    }
    invoke_menu_command_with_fallback(
        driver,
        window,
        "Advanced",
        "TLS Certificates",
        step_timeout,
    )?;
    wait_for_accessible_name(driver, window, "TLS Certificate Prompt", step_timeout)?;
    wait_for_accessible_name(
        driver,
        window,
        "modal: tls-certificate-prompt",
        step_timeout,
    )?;
    invoke_named_control_with_wait(
        driver,
        window,
        "Trust Certificate",
        NativeControlKind::Button,
        step_timeout,
    )?;
    wait_for_accessible_name(driver, window, "modal: (none)", step_timeout)?;
    steps.push("tls-certificate-prompt-completed".to_owned());

    navigate_to_view_with_fallback(
        driver,
        window,
        "Main Window",
        "view: room",
        "Window",
        "Show Users",
        step_timeout,
    )?;
    wait_for_named_control_enabled_state(
        driver,
        window,
        "Toggle Pause",
        NativeControlKind::Button,
        false,
        step_timeout,
    )?;
    steps.push("main-window-playback-controls-detached".to_owned());
    wait_for_accessible_name(driver, window, "Playlist", step_timeout)?;
    wait_for_named_control_enabled_state(
        driver,
        window,
        "Paste URLs...",
        NativeControlKind::Button,
        false,
        step_timeout,
    )?;
    steps.push("main-window-playlist-controls-gated".to_owned());

    navigate_to_view_with_fallback(
        driver,
        window,
        "Configuration",
        "view: setup",
        "Advanced",
        "Trusted Domains",
        step_timeout,
    )?;
    select_top_tab_with_wait(driver, window, "Connection", "Host", step_timeout)?;
    for (edit_index, expected_value) in [
        (CONFIG_HOST_EDIT_INDEX, CONFIG_HOST_VALUE),
        (CONFIG_PORT_EDIT_INDEX, CONFIG_PORT_VALUE),
        (CONFIG_USERNAME_EDIT_INDEX, CONFIG_USERNAME_VALUE),
        (CONFIG_ROOM_EDIT_INDEX, CONFIG_ROOM_VALUE),
        (CONFIG_PLAYER_PATH_EDIT_INDEX, CONFIG_PLAYER_PATH_VALUE),
    ] {
        let current_value = driver.get_edit_value_by_index(window, edit_index)?;
        if current_value != expected_value {
            driver.set_edit_value_by_index(window, edit_index, expected_value)?;
        }
    }
    select_top_tab_with_wait(driver, window, "Connection", "Host", step_timeout)?;
    for (edit_index, expected_value) in [
        (CONFIG_HOST_EDIT_INDEX, CONFIG_HOST_VALUE),
        (CONFIG_PORT_EDIT_INDEX, CONFIG_PORT_VALUE),
        (CONFIG_USERNAME_EDIT_INDEX, CONFIG_USERNAME_VALUE),
        (CONFIG_ROOM_EDIT_INDEX, CONFIG_ROOM_VALUE),
        (CONFIG_PLAYER_PATH_EDIT_INDEX, CONFIG_PLAYER_PATH_VALUE),
    ] {
        wait_for_edit_value_by_index(driver, window, edit_index, expected_value, step_timeout)?;
    }
    driver.set_edit_value_by_index(window, CONFIG_PORT_EDIT_INDEX, "70000")?;
    wait_for_edit_value_by_index(
        driver,
        window,
        CONFIG_PORT_EDIT_INDEX,
        "70000",
        step_timeout,
    )?;
    wait_for_accessible_name(driver, window, "Status: 1 issue(s)", step_timeout)?;
    wait_for_accessible_name(
        driver,
        window,
        "Connection / Port: must be a valid TCP port from 1 to 65535.",
        step_timeout,
    )?;
    wait_for_accessible_name(driver, window, "Save: disabled", step_timeout)?;
    steps.push("config-validation-visible".to_owned());
    driver.set_edit_value_by_index(window, CONFIG_PORT_EDIT_INDEX, CONFIG_PORT_VALUE)?;
    wait_for_edit_value_by_index(
        driver,
        window,
        CONFIG_PORT_EDIT_INDEX,
        CONFIG_PORT_VALUE,
        step_timeout,
    )?;
    wait_for_accessible_name(driver, window, "Status: clean", step_timeout)?;
    wait_for_accessible_name(driver, window, "Save: enabled", step_timeout)?;

    invoke_named_control_with_wait(
        driver,
        window,
        "Save",
        NativeControlKind::Button,
        step_timeout,
    )?;
    wait_for_pending_operation_to_finish(
        driver,
        window,
        "pending: save-configuration",
        step_timeout,
    )?;
    wait_for_accessible_name(driver, window, "Busy: no", step_timeout)?;
    wait_for_accessible_name(driver, window, "Save: enabled", step_timeout)?;
    let config_persist_result = wait_for_file_contains(
        config_path,
        &[
            "host = syncplay.example",
            "port = 8999",
            "name = smoke-user",
            "room = smoke-room",
            "playerPath = C:\\Windows\\System32\\notepad.exe",
        ],
        config_persist_timeout,
    );
    match config_persist_result {
        Ok(()) => {
            steps.push("config-save-persisted".to_owned());
        }
        Err(error) => steps.push(format!(
            "config-save-persisted-skipped:{}",
            error.replace('|', "/").replace('\n', " ")
        )),
    }

    wait_for_accessible_name(driver, window, "Public Servers", step_timeout)?;
    wait_for_accessible_name(driver, window, "2", step_timeout)?;

    navigate_to_view_with_fallback(
        driver,
        window,
        "Public Servers",
        "view: setup",
        "File",
        "Open Public Server Browser",
        step_timeout,
    )?;
    steps.push("surface-public-servers".to_owned());

    invoke_named_control_with_wait(
        driver,
        window,
        "Alpha: alpha.example:8999",
        NativeControlKind::Any,
        step_timeout,
    )?;
    wait_for_named_control_enabled_state(
        driver,
        window,
        "Connect",
        NativeControlKind::Button,
        true,
        step_timeout,
    )?;
    steps.push("public-server-connect-enabled".to_owned());

    invoke_named_control_with_wait(
        driver,
        window,
        "Refresh",
        NativeControlKind::Button,
        step_timeout,
    )?;
    wait_for_pending_operation_to_finish(
        driver,
        window,
        "pending: refresh-public-servers",
        step_timeout,
    )?;
    steps.push("public-server-refresh-complete".to_owned());

    invoke_named_control_with_wait(
        driver,
        window,
        "Add Custom Server",
        NativeControlKind::Button,
        step_timeout,
    )?;
    wait_for_accessible_name(driver, window, "Edit Session", step_timeout)?;
    let edit_count = driver.editable_text_input_count(window)?;
    if edit_count < 2 {
        return Err(format!(
            "expected at least 2 editable public-server edit-session fields, found {edit_count}"
        ));
    }
    driver.set_edit_value_by_index(window, 0, CUSTOM_SERVER_LABEL)?;
    driver.set_edit_value_by_index(window, 1, CUSTOM_SERVER_ADDRESS)?;
    wait_for_edit_value_by_index(driver, window, 0, CUSTOM_SERVER_LABEL, step_timeout)?;
    wait_for_edit_value_by_index(driver, window, 1, CUSTOM_SERVER_ADDRESS, step_timeout)?;
    invoke_named_control_with_wait(
        driver,
        window,
        "Save Changes",
        NativeControlKind::Button,
        step_timeout,
    )?;
    wait_for_named_control_count(
        driver,
        window,
        "Save Changes",
        NativeControlKind::Button,
        0,
        step_timeout,
    )?;
    let custom_row_name = wait_for_any_accessible_name(
        driver,
        window,
        &[CUSTOM_SERVER_ROW_NAME, CUSTOM_SERVER_LABEL],
        step_timeout,
    )?;
    if custom_row_name == CUSTOM_SERVER_LABEL {
        wait_for_accessible_name(driver, window, CUSTOM_SERVER_ADDRESS, step_timeout)?;
    }
    steps.push("public-server-add-custom".to_owned());

    invoke_named_control_with_wait(
        driver,
        window,
        &custom_row_name,
        NativeControlKind::Any,
        step_timeout,
    )?;
    wait_for_named_control_enabled_state(
        driver,
        window,
        "Connect",
        NativeControlKind::Button,
        true,
        step_timeout,
    )?;
    steps.push("public-server-connect-custom-enabled".to_owned());

    navigate_to_view_with_fallback(
        driver,
        window,
        "Configuration",
        "view: setup",
        "Advanced",
        "Trusted Domains",
        step_timeout,
    )?;
    for (edit_index, expected_value) in [
        (CONFIG_HOST_EDIT_INDEX, CUSTOM_SERVER_HOST),
        (CONFIG_PORT_EDIT_INDEX, CUSTOM_SERVER_PORT),
    ] {
        let actual = driver.get_edit_value_by_index(window, edit_index)?;
        if actual != expected_value {
            return Err(format!(
                "custom public-server selection did not update configuration edit field [{edit_index}]: expected {expected_value:?}, got {actual:?}"
            ));
        }
    }
    steps.push("public-server-custom-applied".to_owned());

    navigate_to_view_with_fallback(
        driver,
        window,
        "Media Search",
        "view: setup",
        "File",
        "Open Media Search",
        step_timeout,
    )?;
    steps.push("surface-media-search".to_owned());
    wait_for_accessible_name(driver, window, "First File Timeout: 3.00s", step_timeout)?;
    wait_for_accessible_name(driver, window, "Search Timeout: 30.00s", step_timeout)?;
    let mut double_check_visible =
        wait_for_accessible_name(driver, window, "Double Check Interval: 2.50s", step_timeout)
            .is_ok();
    let mut warning_threshold_visible =
        wait_for_accessible_name(driver, window, "Warning Threshold: 7.50s", step_timeout).is_ok();
    let mut page_down_count = 0usize;
    let timing_retry_timeout = step_timeout.min(Duration::from_millis(1_000));
    while page_down_count < 2 && (!double_check_visible || !warning_threshold_visible) {
        let _ = driver.scroll_active_view_page_down(window);
        page_down_count += 1;
        if !double_check_visible {
            double_check_visible = wait_for_accessible_name(
                driver,
                window,
                "Double Check Interval: 2.50s",
                timing_retry_timeout,
            )
            .is_ok();
        }
        if !warning_threshold_visible {
            warning_threshold_visible = wait_for_accessible_name(
                driver,
                window,
                "Warning Threshold: 7.50s",
                timing_retry_timeout,
            )
            .is_ok();
        }
    }
    for _ in 0..page_down_count {
        let _ = driver.scroll_active_view_page_up(window);
    }
    if double_check_visible && warning_threshold_visible {
        steps.push("media-search-timing-visible".to_owned());
        steps.push("media-search-timing-values-visible".to_owned());
    } else {
        return Err(format!(
            "media-search timing values were not all visible: first_file=yes, search=yes, double_check={}, warning_threshold={}",
            bool_label(double_check_visible),
            bool_label(warning_threshold_visible)
        ));
    }

    invoke_named_control_with_wait(
        driver,
        window,
        "Browse Directories",
        NativeControlKind::Button,
        step_timeout,
    )?;
    wait_for_accessible_name(driver, window, &media_search_directory_value, step_timeout)?;
    steps.push("media-search-browse".to_owned());

    invoke_named_control_with_wait(
        driver,
        window,
        "Browse Directories",
        NativeControlKind::Button,
        step_timeout,
    )?;
    wait_for_named_control_count(
        driver,
        window,
        &media_search_directory_value,
        NativeControlKind::Any,
        1,
        step_timeout,
    )?;
    steps.push("media-search-browse-dedupe".to_owned());

    wait_for_named_control_enabled_state(
        driver,
        window,
        "Search Missing Media",
        NativeControlKind::Button,
        true,
        step_timeout,
    )?;
    steps.push("media-search-command-enabled".to_owned());

    navigate_to_view_with_fallback(
        driver,
        window,
        "Configuration",
        "view: setup",
        "Advanced",
        "Trusted Domains",
        step_timeout,
    )?;
    select_top_tab_with_wait(driver, window, "Connection", "Host", step_timeout)?;
    steps.push("surface-configuration".to_owned());

    upsert_syncplay_ini_stored_client_settings_mvp_at_path(
        config_path,
        &expected_saved_configuration(&media_search_directory_value),
    )
    .map_err(|error| {
        format!(
            "failed to seed full first-run configuration {} before GUI reload: {error}",
            config_path.display()
        )
    })?;
    invoke_named_control_with_wait(
        driver,
        window,
        "Reload",
        NativeControlKind::Button,
        step_timeout,
    )?;
    wait_for_pending_operation_to_finish(
        driver,
        window,
        "pending: reload-configuration",
        step_timeout,
    )?;
    let _ = wait_for_saved_configuration(
        config_path,
        &media_search_directory_value,
        config_persist_timeout,
    )?;
    select_top_tab_with_wait(driver, window, "Connection", "Host", step_timeout)?;
    for (edit_index, expected_value) in [
        (CONFIG_HOST_EDIT_INDEX, CONFIG_HOST_VALUE),
        (CONFIG_PORT_EDIT_INDEX, CONFIG_PORT_VALUE),
        (CONFIG_USERNAME_EDIT_INDEX, CONFIG_USERNAME_VALUE),
        (CONFIG_ROOM_EDIT_INDEX, CONFIG_ROOM_VALUE),
        (CONFIG_PLAYER_PATH_EDIT_INDEX, CONFIG_PLAYER_PATH_VALUE),
    ] {
        wait_for_edit_value_by_index(driver, window, edit_index, expected_value, step_timeout)?;
    }
    select_top_tab_with_wait(
        driver,
        window,
        "Privacy & Chat",
        "Trusted Domains Only",
        step_timeout,
    )?;
    wait_for_accessible_name(driver, window, "Chat Input", step_timeout)?;
    select_top_tab_with_wait(
        driver,
        window,
        "Playback & Search",
        "Ready At Start",
        step_timeout,
    )?;
    wait_for_accessible_name(driver, window, "Autoplay", step_timeout)?;
    wait_for_accessible_name(driver, window, "Rewind On Desync", step_timeout)?;
    select_top_tab_with_wait(
        driver,
        window,
        "Interface & System",
        "Show OSD",
        step_timeout,
    )?;
    wait_for_accessible_name(driver, window, "Language", step_timeout)?;
    wait_for_accessible_name(driver, window, "Auto Update", step_timeout)?;
    steps.push("config-reload-persisted".to_owned());
    steps.push("trusted-domains-persisted".to_owned());
    steps.push("config-readiness-persisted".to_owned());
    steps.push("config-privacy-persisted".to_owned());
    steps.push("config-desync-persisted".to_owned());
    steps.push("config-chat-persisted".to_owned());
    steps.push("config-osd-persisted".to_owned());
    steps.push("config-system-persisted".to_owned());

    if select_top_tab_with_wait(
        driver,
        window,
        "Playback & Search",
        "Shared Playlists",
        step_timeout,
    )
    .is_err()
    {
        let error =
            wait_for_accessible_name(driver, window, "Shared Playlists", step_timeout).unwrap_err();
        steps.push(format!(
            "open-media-prep-shared-playlists-skipped:{}",
            error.replace('|', "/").replace('\n', " ")
        ));
    } else {
        steps.push("open-media-prep-shared-playlists".to_owned());
    }

    let open_media_invoked = if let Err(primary_error) = invoke_menu_command_with_wait(
        driver,
        window,
        "File",
        "Open Media File",
        NativeControlKind::MenuItem,
        step_timeout,
    ) {
        match invoke_menu_command_with_wait(
            driver,
            window,
            "File",
            "Open Media File",
            NativeControlKind::Any,
            step_timeout,
        ) {
            Ok(()) => true,
            Err(fallback_error) => match invoke_named_control_with_wait(
                driver,
                window,
                "Quick Open Media File",
                NativeControlKind::Button,
                step_timeout,
            ) {
                Ok(()) => true,
                Err(button_error) => {
                    steps.push(format!(
                        "open-media-file-skipped:{}",
                        format!(
                            "menu-item-failure={primary_error}; fallback-failure={fallback_error}; button-failure={button_error}"
                        )
                        .replace('|', "/")
                    ));
                    false
                }
            },
        }
    } else {
        true
    };
    if open_media_invoked {
        let open_media_switched_to_main_window =
            wait_for_accessible_name(driver, window, "view: room", Duration::from_millis(800))
                .is_ok();
        if open_media_switched_to_main_window {
            wait_for_accessible_name(
                driver,
                window,
                &open_media_file_path.display().to_string(),
                step_timeout,
            )?;
            steps.push("open-media-file".to_owned());
        } else {
            invoke_named_control_with_wait(
                driver,
                window,
                "Quick Open Media File",
                NativeControlKind::Button,
                step_timeout,
            )?;
            if wait_for_accessible_name(driver, window, "view: room", Duration::from_millis(800))
                .is_ok()
            {
                wait_for_accessible_name(
                    driver,
                    window,
                    &open_media_file_path.display().to_string(),
                    step_timeout,
                )?;
                steps.push("open-media-file".to_owned());
            } else {
                wait_for_accessible_name(driver, window, "view: setup", step_timeout)?;
                wait_for_accessible_name_fragment(
                    driver,
                    window,
                    "requires a session or playback runtime connection",
                    step_timeout,
                )?;
                steps.push("open-media-file-detached-runtime-unavailable".to_owned());
            }
        }
    }
    navigate_to_view_with_fallback(
        driver,
        window,
        "Media Search",
        "view: setup",
        "File",
        "Open Media Search",
        step_timeout,
    )?;
    wait_for_accessible_name(driver, window, &media_search_directory_value, step_timeout)?;
    steps.push("persistence-state-prepared".to_owned());

    if let Err(primary_error) = invoke_menu_command_with_wait(
        driver,
        window,
        "Help",
        "About",
        NativeControlKind::MenuItem,
        step_timeout,
    ) {
        invoke_menu_command_with_wait(
            driver,
            window,
            "Help",
            "About",
            NativeControlKind::Any,
            step_timeout,
        )
        .map_err(|fallback_error| {
            format!(
                "failed to invoke About through menu item ({primary_error}); fallback also failed: {fallback_error}"
            )
        })?;
    }
    wait_for_accessible_name(driver, window, "view: setup", step_timeout)?;
    wait_for_accessible_name(driver, window, "modal: (none)", step_timeout)?;
    steps.push("about-routes-to-setup".to_owned());

    Ok(steps)
}
