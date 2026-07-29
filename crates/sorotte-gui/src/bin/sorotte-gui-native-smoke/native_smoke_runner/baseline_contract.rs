use super::*;

pub(super) fn verify_interaction_contract<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    config_path: &Path,
    media_search_browse_path: &Path,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let step_timeout = timeout.min(Duration::from_millis(4_000));
    let config_persist_timeout = timeout.min(Duration::from_millis(8_000));
    let media_search_directory_value = media_search_browse_path.display().to_string();
    let saved_config_host_value = "saved.syncplay.example";
    let mut steps = Vec::new();

    let initial_view =
        wait_for_any_accessible_name(driver, window, &["view: setup", "view: room"], step_timeout)?;
    if initial_view == "view: room" {
        navigate_to_view_with_fallback(
            driver,
            window,
            SETUP_SURFACE_AUTOMATION_ID,
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
            MODAL_PLAYER_SETUP_RETRY_AUTOMATION_ID,
            NativeControlKind::Button,
            true,
            step_timeout,
        )?;
        wait_for_named_control_enabled_state(
            driver,
            window,
            MODAL_PLAYER_SETUP_OPEN_SETTINGS_AUTOMATION_ID,
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
        steps.push("player-setup-existing-config-modal".to_owned());
    }
    invoke_menu_action_by_id_with_wait(
        driver,
        window,
        ADVANCED_MENU_AUTOMATION_ID,
        TLS_CERTIFICATES_MENU_AUTOMATION_ID,
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
        MODAL_TLS_TRUST_AUTOMATION_ID,
        NativeControlKind::Button,
        step_timeout,
    )?;
    wait_for_accessible_name(driver, window, "modal: (none)", step_timeout)?;
    steps.push("tls-certificate-prompt-completed".to_owned());

    navigate_to_view_with_wait(
        driver,
        window,
        ROOM_SURFACE_AUTOMATION_ID,
        "view: room",
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
    for (automation_id, expected_value) in [
        (CONFIG_HOST_AUTOMATION_ID, CONFIG_HOST_VALUE),
        (CONFIG_PORT_AUTOMATION_ID, CONFIG_PORT_VALUE),
        (CONFIG_USERNAME_AUTOMATION_ID, CONFIG_USERNAME_VALUE),
        (CONFIG_ROOM_AUTOMATION_ID, CONFIG_ROOM_VALUE),
        (CONFIG_PLAYER_PATH_AUTOMATION_ID, CONFIG_PLAYER_PATH_VALUE),
    ] {
        let current_value = driver.get_named_edit_value(window, automation_id)?;
        if current_value != expected_value {
            driver.set_named_edit_value(window, automation_id, expected_value, false)?;
        }
    }
    select_top_tab_with_wait(
        driver,
        window,
        CONFIG_CONNECTION_TAB_AUTOMATION_ID,
        "Host",
        step_timeout,
    )?;
    for (automation_id, expected_value) in [
        (CONFIG_HOST_AUTOMATION_ID, CONFIG_HOST_VALUE),
        (CONFIG_PORT_AUTOMATION_ID, CONFIG_PORT_VALUE),
        (CONFIG_USERNAME_AUTOMATION_ID, CONFIG_USERNAME_VALUE),
        (CONFIG_ROOM_AUTOMATION_ID, CONFIG_ROOM_VALUE),
        (CONFIG_PLAYER_PATH_AUTOMATION_ID, CONFIG_PLAYER_PATH_VALUE),
    ] {
        wait_for_named_edit_value(driver, window, automation_id, expected_value, step_timeout)?;
    }
    driver.set_named_edit_value(window, CONFIG_PORT_AUTOMATION_ID, "70000", false)?;
    wait_for_named_edit_value(
        driver,
        window,
        CONFIG_PORT_AUTOMATION_ID,
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
    driver.set_named_edit_value(window, CONFIG_PORT_AUTOMATION_ID, CONFIG_PORT_VALUE, false)?;
    wait_for_named_edit_value(
        driver,
        window,
        CONFIG_PORT_AUTOMATION_ID,
        CONFIG_PORT_VALUE,
        step_timeout,
    )?;
    driver.set_named_edit_value(
        window,
        CONFIG_HOST_AUTOMATION_ID,
        saved_config_host_value,
        false,
    )?;
    wait_for_named_edit_value(
        driver,
        window,
        CONFIG_HOST_AUTOMATION_ID,
        saved_config_host_value,
        step_timeout,
    )?;
    wait_for_accessible_name(driver, window, "Status: clean", step_timeout)?;
    wait_for_accessible_name(driver, window, "Save: enabled", step_timeout)?;

    invoke_named_control_with_wait(
        driver,
        window,
        CONFIG_SAVE_AUTOMATION_ID,
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
    wait_for_accessible_name(driver, window, "Save: disabled", step_timeout)?;
    let config_persist_result = wait_for_file_contains(
        config_path,
        &[
            "host = saved.syncplay.example",
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
        PUBLIC_SERVER_CONNECT_AUTOMATION_ID,
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
    driver.set_named_edit_value(
        window,
        PUBLIC_SERVER_EDIT_LABEL_AUTOMATION_ID,
        CUSTOM_SERVER_LABEL,
        false,
    )?;
    driver.set_named_edit_value(
        window,
        PUBLIC_SERVER_EDIT_ADDRESS_AUTOMATION_ID,
        CUSTOM_SERVER_ADDRESS,
        false,
    )?;
    wait_for_named_edit_value(
        driver,
        window,
        PUBLIC_SERVER_EDIT_LABEL_AUTOMATION_ID,
        CUSTOM_SERVER_LABEL,
        step_timeout,
    )?;
    wait_for_named_edit_value(
        driver,
        window,
        PUBLIC_SERVER_EDIT_ADDRESS_AUTOMATION_ID,
        CUSTOM_SERVER_ADDRESS,
        step_timeout,
    )?;
    invoke_named_control_with_wait(
        driver,
        window,
        PUBLIC_SERVER_EDIT_COMMIT_AUTOMATION_ID,
        NativeControlKind::Button,
        step_timeout,
    )?;
    wait_for_named_control_count(
        driver,
        window,
        PUBLIC_SERVER_EDIT_COMMIT_AUTOMATION_ID,
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
        PUBLIC_SERVER_CONNECT_AUTOMATION_ID,
        NativeControlKind::Button,
        true,
        step_timeout,
    )?;
    steps.push("public-server-connect-custom-enabled".to_owned());

    navigate_to_view_with_fallback(
        driver,
        window,
        SETUP_SURFACE_AUTOMATION_ID,
        "view: setup",
        "Advanced",
        "Trusted Domains",
        step_timeout,
    )?;
    for (automation_id, expected_value) in [
        (CONFIG_HOST_AUTOMATION_ID, CUSTOM_SERVER_HOST),
        (CONFIG_PORT_AUTOMATION_ID, CUSTOM_SERVER_PORT),
    ] {
        let actual = driver.get_named_edit_value(window, automation_id)?;
        if actual != expected_value {
            return Err(format!(
                "custom public-server selection did not update configuration field {automation_id:?}: expected {expected_value:?}, got {actual:?}"
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
    steps.push("surface-configuration".to_owned());

    upsert_sorotte_ini_stored_client_settings_mvp_at_path(
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
        CONFIG_RELOAD_AUTOMATION_ID,
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
    select_top_tab_with_wait(
        driver,
        window,
        CONFIG_CONNECTION_TAB_AUTOMATION_ID,
        "Host",
        step_timeout,
    )?;
    for (automation_id, expected_value) in [
        (CONFIG_HOST_AUTOMATION_ID, CONFIG_HOST_VALUE),
        (CONFIG_PORT_AUTOMATION_ID, CONFIG_PORT_VALUE),
        (CONFIG_USERNAME_AUTOMATION_ID, CONFIG_USERNAME_VALUE),
        (CONFIG_ROOM_AUTOMATION_ID, CONFIG_ROOM_VALUE),
        (CONFIG_PLAYER_PATH_AUTOMATION_ID, CONFIG_PLAYER_PATH_VALUE),
    ] {
        wait_for_named_edit_value(driver, window, automation_id, expected_value, step_timeout)?;
    }
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
        "Ready At Start",
        step_timeout,
    )?;
    wait_for_accessible_name(driver, window, "Autoplay", step_timeout)?;
    wait_for_accessible_name(driver, window, "Rewind On Desync", step_timeout)?;
    driver
        .activate_named_control_by_keyboard(
            window,
            CONFIG_INTERFACE_SYSTEM_TAB_AUTOMATION_ID,
            NativeControlKind::Button,
        )
        .map_err(|error| {
            format!(
                "failed the required focused-keyboard activation of the Interface & System tab: {error}"
            )
        })?;
    wait_for_accessible_name(driver, window, "Show OSD", step_timeout)?;
    wait_for_accessible_name(driver, window, "Language", step_timeout)?;
    steps.push("config-tab-focused-keyboard-activation".to_owned());
    steps.push("config-reload-persisted".to_owned());
    steps.push("trusted-domains-persisted".to_owned());
    steps.push("config-readiness-persisted".to_owned());
    steps.push("config-privacy-persisted".to_owned());
    steps.push("config-desync-persisted".to_owned());
    steps.push("config-chat-persisted".to_owned());
    steps.push("config-osd-persisted".to_owned());
    steps.push("config-system-persisted".to_owned());

    select_top_tab_with_wait(
        driver,
        window,
        CONFIG_PLAYBACK_SEARCH_TAB_AUTOMATION_ID,
        "Shared Playlists",
        step_timeout,
    )?;
    steps.push("open-media-prep-shared-playlists".to_owned());

    verify_menu_action_enabled_state_by_id(
        driver,
        window,
        FILE_MENU_AUTOMATION_ID,
        OPEN_MEDIA_MENU_AUTOMATION_ID,
        false,
        step_timeout,
    )?;
    steps.push("open-media-file-detached-disabled".to_owned());
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

    invoke_menu_action_by_id_with_wait(
        driver,
        window,
        HELP_MENU_AUTOMATION_ID,
        ABOUT_MENU_AUTOMATION_ID,
        step_timeout,
    )?;
    wait_for_accessible_name(driver, window, "view: setup", step_timeout)?;
    wait_for_accessible_name(driver, window, "modal: about", step_timeout)?;
    wait_for_accessible_name(driver, window, "About Sorotte", step_timeout)?;
    invoke_named_control_with_wait(
        driver,
        window,
        MODAL_CLOSE_AUTOMATION_ID,
        NativeControlKind::Button,
        step_timeout,
    )?;
    wait_for_accessible_name(driver, window, "modal: (none)", step_timeout)?;
    steps.push("about-opens-and-closes-modal".to_owned());

    for _ in 0..25 {
        verify_menu_action_enabled_state_by_id(
            driver,
            window,
            FILE_MENU_AUTOMATION_ID,
            EXIT_MENU_AUTOMATION_ID,
            true,
            step_timeout,
        )?;
    }
    steps.push("menu-input-stress-25".to_owned());

    Ok(steps)
}
