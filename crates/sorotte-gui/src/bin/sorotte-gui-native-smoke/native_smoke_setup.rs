use super::*;

pub(super) fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

pub(super) fn default_binary_path() -> PathBuf {
    if let Ok(current_exe) = std::env::current_exe()
        && let Some(parent) = current_exe.parent()
    {
        let candidate = parent.join("sorotte-gui.exe");
        if candidate.exists() {
            return candidate;
        }
    }
    PathBuf::from("target")
        .join("debug")
        .join("sorotte-gui.exe")
}

pub(super) fn resolve_binary_path(path: &Path) -> Result<PathBuf, String> {
    fs::canonicalize(path)
        .map_err(|error| format!("failed to resolve sorotte-gui binary at {path:?}: {error}"))
}

pub(super) fn launch_sorotte_gui_with_test_overrides(
    binary_path: &Path,
    launch: GuiLaunchConfig<'_>,
    test_overrides: GuiLaunchTestOverrides<'_>,
) -> Result<Child, String> {
    let mut command = Command::new(binary_path);
    if let Some(parent) = binary_path.parent() {
        command.current_dir(parent);
    }
    for name in [
        "SOROTTE_GUI_ENABLE_CLIENT_CORE_CHAT_TCP",
        "SOROTTE_GUI_ENABLE_CLIENT_CORE_CHAT_LOOPBACK",
        "SOROTTE_GUI_ENABLE_TEST_PLAYER",
        "SOROTTE_CLIENT_HOST",
        "SOROTTE_CLIENT_PORT",
        "SOROTTE_CLIENT_USERNAME",
        "SOROTTE_CLIENT_ROOM",
        "SOROTTE_CLIENT_MPV_IPC_PATH",
        "SOROTTE_MPV_IPC_PATH",
        "SOROTTE_GUI_TEST_DROP_FILE_PATHS",
        "SOROTTE_GUI_TEST_DROP_TARGET",
        "SOROTTE_GUI_UPDATE_CHECK_RESPONSE",
        "SOROTTE_GUI_TEST_THEME",
        "SOROTTE_GUI_TEST_PLAYER_SETTINGS_DEGRADED",
        "SOROTTE_GUI_TEST_CONFIG_ROOT_BROWSE_PATH",
        "SOROTTE_GUI_TEST_DISABLE_STARTUP_SAVED_CONNECT",
        "SOROTTE_CLIENT_CONFIG_ROOT",
        "SOROTTE_CLIENT_INSTALL_ROOT",
    ] {
        command.env_remove(name);
    }
    if let Some(appdata_root) = test_overrides.appdata_root {
        command.env_remove("SOROTTE_CLIENT_CONFIG_PATH");
        command.env("APPDATA", appdata_root);
        command.env(
            "SOROTTE_CLIENT_INSTALL_ROOT",
            appdata_root.join("isolated-install-root"),
        );
    } else {
        command.env("SOROTTE_CLIENT_CONFIG_PATH", launch.config_path);
    }
    if let Some(theme) = test_overrides.theme {
        command.env("SOROTTE_GUI_TEST_THEME", theme);
    }
    if let Some(path) = test_overrides.config_storage_browse_path {
        command.env("SOROTTE_GUI_TEST_CONFIG_ROOT_BROWSE_PATH", path);
    }
    if test_overrides.disable_startup_saved_connect {
        command.env("SOROTTE_GUI_TEST_DISABLE_STARTUP_SAVED_CONNECT", "true");
    }
    if test_overrides.player_settings_degraded {
        command.env("SOROTTE_GUI_TEST_PLAYER_SETTINGS_DEGRADED", "true");
    }
    command.env(
        "SOROTTE_GUI_REFRESH_PUBLIC_SERVERS",
        launch.public_servers_spec,
    );
    command.env(
        "SOROTTE_GUI_TEST_OPEN_MEDIA_FILE_PATHS",
        launch.open_media_file_path.display().to_string(),
    );
    command.env(
        "SOROTTE_GUI_TEST_MEDIA_SEARCH_BROWSE_PATH",
        launch.media_search_browse_path.display().to_string(),
    );
    command.env(
        "SOROTTE_GUI_UPDATE_CHECK_RESPONSE",
        DEFAULT_UPDATE_CHECK_RESPONSE,
    );
    if let Some(drop_file_paths_spec) = launch.drop_file_paths_spec {
        command.env("SOROTTE_GUI_TEST_DROP_FILE_PATHS", drop_file_paths_spec);
    }
    if let Some(drop_target) = launch.drop_target {
        command.env("SOROTTE_GUI_TEST_DROP_TARGET", drop_target);
    }
    if let Some(tcp_session) = launch.tcp_session {
        command.env("SOROTTE_GUI_ENABLE_CLIENT_CORE_CHAT_TCP", "true");
        command.env("SOROTTE_CLIENT_HOST", tcp_session.host);
        command.env("SOROTTE_CLIENT_PORT", tcp_session.port.to_string());
        command.env("SOROTTE_CLIENT_USERNAME", tcp_session.username);
        command.env("SOROTTE_CLIENT_ROOM", tcp_session.room);
    } else if let Some((username, room)) = launch.loopback_session {
        command.env("SOROTTE_GUI_ENABLE_CLIENT_CORE_CHAT_LOOPBACK", "true");
        command.env("SOROTTE_CLIENT_USERNAME", username);
        command.env("SOROTTE_CLIENT_ROOM", room);
    }
    if launch.attach_test_player {
        command.env("SOROTTE_GUI_ENABLE_TEST_PLAYER", "true");
    }
    command
        .spawn()
        .map_err(|error| format!("failed to launch sorotte-gui at {binary_path:?}: {error}"))
}

pub(super) fn wait_for_main_window<D: NativeGuiDriver>(
    driver: &D,
    child: &mut Child,
    timeout: Duration,
) -> Result<D::WindowHandle, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("failed to poll sorotte-gui process state: {error}"))?
        {
            return Err(format!(
                "sorotte-gui exited before exposing a main window (status: {status})"
            ));
        }

        if let Some(window) = driver.find_main_window(child.id())? {
            return Ok(window);
        }

        if Instant::now() >= deadline {
            return Err("timed out waiting for the sorotte-gui main window".to_owned());
        }
        thread::sleep(Duration::from_millis(50));
    }
}

pub(super) fn wait_for_process_exit(child: &mut Child, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if child
            .try_wait()
            .map_err(|error| format!("failed to poll sorotte-gui exit state: {error}"))?
            .is_some()
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("timed out waiting for sorotte-gui to exit after close request".to_owned());
        }
        thread::sleep(Duration::from_millis(50));
    }
}

pub(super) fn seed_native_smoke_config_with_saved_server(
    config_path: &Path,
    host: Option<&str>,
    port: Option<u16>,
) -> Result<(), String> {
    let settings = StoredClientSettingsMvp {
        host: host.map(str::to_owned),
        port,
        username: Some(CONFIG_USERNAME_VALUE.to_owned()),
        room: Some(CONFIG_ROOM_VALUE.to_owned()),
        player_path: Some(CONFIG_PLAYER_PATH_VALUE.to_owned()),
        folder_search_first_file_timeout_seconds: Some(MEDIA_SEARCH_FIRST_FILE_TIMEOUT_SECONDS),
        folder_search_timeout_seconds: Some(MEDIA_SEARCH_TIMEOUT_SECONDS),
        folder_search_double_check_interval_seconds: Some(
            MEDIA_SEARCH_DOUBLE_CHECK_INTERVAL_SECONDS,
        ),
        folder_search_warning_threshold_seconds: Some(MEDIA_SEARCH_WARNING_THRESHOLD_SECONDS),
        ..StoredClientSettingsMvp::default()
    };
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(config_path, &settings).map_err(|error| {
        format!(
            "failed to seed native smoke config {}: {error}",
            config_path.display()
        )
    })
}

pub(super) fn seed_native_smoke_config(config_path: &Path) -> Result<(), String> {
    seed_native_smoke_config_with_saved_server(
        config_path,
        Some(CONFIG_HOST_VALUE),
        Some(CONFIG_PORT_VALUE.parse().unwrap()),
    )
}

pub(super) fn legacy_gui_qsettings_store_path(root: &Path, store_name: &str) -> PathBuf {
    root.join(format!("{store_name}.ini"))
}

pub(super) fn write_legacy_gui_qsettings_ini(
    path: &Path,
    sections: &[(&str, Vec<(&str, String)>)],
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create native smoke legacy GUI store directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let mut contents = String::new();
    for (section, entries) in sections {
        if entries.is_empty() {
            continue;
        }
        contents.push('[');
        contents.push_str(section);
        contents.push_str("]\n");
        for (key, value) in entries {
            contents.push_str(key);
            contents.push_str(" = ");
            contents.push_str(&value.replace('%', "%%"));
            contents.push('\n');
        }
        contents.push('\n');
    }
    fs::write(path, contents).map_err(|error| {
        format!(
            "failed to write native smoke legacy GUI store {}: {error}",
            path.display()
        )
    })
}

pub(super) fn seed_native_smoke_gui_state(
    root: &Path,
    active_view: Option<&str>,
    selected_public_server_address: Option<&str>,
    public_servers: &[(String, String)],
    last_media_dialog_directory: Option<&Path>,
) -> Result<(), String> {
    let main_window_entries = [
        active_view.map(|value| ("activeView", value.to_owned())),
        selected_public_server_address
            .map(|value| ("selectedPublicServerAddress", value.to_owned())),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    if !main_window_entries.is_empty() {
        write_legacy_gui_qsettings_ini(
            &legacy_gui_qsettings_store_path(root, "MainWindow"),
            &[("MainWindow", main_window_entries)],
        )?;
    }
    if !public_servers.is_empty() {
        write_legacy_gui_qsettings_ini(
            &legacy_gui_qsettings_store_path(root, "Interface"),
            &[(
                "PublicServerList",
                vec![(
                    "publicServers",
                    format_serialized_public_servers_list_legacy_compatible(public_servers),
                )],
            )],
        )?;
    }
    if let Some(directory) = last_media_dialog_directory {
        write_legacy_gui_qsettings_ini(
            &legacy_gui_qsettings_store_path(root, "MediaBrowseDialog"),
            &[(
                "MediaBrowseDialog",
                vec![("mediadir", directory.display().to_string())],
            )],
        )?;
    }
    Ok(())
}

pub(super) fn launch_sorotte_gui_with_retry<D: NativeGuiDriver>(
    driver: &D,
    binary_path: &Path,
    launch: GuiLaunchConfig<'_>,
    timeout: Duration,
) -> Result<(Child, D::WindowHandle), String> {
    launch_sorotte_gui_with_retry_and_test_overrides(
        driver,
        binary_path,
        launch,
        timeout,
        GuiLaunchTestOverrides::default(),
    )
}

pub(super) fn launch_sorotte_gui_with_retry_and_test_overrides<D: NativeGuiDriver>(
    driver: &D,
    binary_path: &Path,
    launch: GuiLaunchConfig<'_>,
    timeout: Duration,
    test_overrides: GuiLaunchTestOverrides<'_>,
) -> Result<(Child, D::WindowHandle), String> {
    let mut last_error = String::new();
    for attempt in 1..=LAUNCH_ATTEMPTS {
        let mut child =
            launch_sorotte_gui_with_test_overrides(binary_path, launch, test_overrides)?;
        match wait_for_main_window(driver, &mut child, timeout) {
            Ok(window) => {
                if let Err(error) = driver.prepare_window_for_smoke(window) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "failed to prepare deterministic native smoke window bounds: {error}"
                    ));
                }
                return Ok((child, window));
            }
            Err(error) => {
                let retryable = child
                    .try_wait()
                    .ok()
                    .flatten()
                    .and_then(|status| status.code())
                    .is_some_and(|status| status as u32 == DLL_INIT_FAILED_STATUS);
                last_error = error;
                let _ = child.kill();
                let _ = child.wait();
                if retryable && attempt < LAUNCH_ATTEMPTS {
                    thread::sleep(Duration::from_millis(500));
                    continue;
                }
                break;
            }
        }
    }
    Err(last_error)
}

pub(super) fn wait_for_file_contains(
    path: &Path,
    expected_snippets: &[&str],
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut last_contents = String::new();
    loop {
        match fs::read_to_string(path) {
            Ok(contents) => {
                if expected_snippets
                    .iter()
                    .all(|snippet| contents.contains(snippet))
                {
                    return Ok(());
                }
                last_contents = contents;
            }
            Err(error) => {
                if Instant::now() >= deadline {
                    return Err(format!(
                        "timed out waiting for config file {path:?} to contain required lines; last read error: {error}"
                    ));
                }
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for config file {:?} to contain [{}]. Last file contents:\n{}",
                path,
                expected_snippets
                    .iter()
                    .map(|snippet| format!("{snippet:?}"))
                    .collect::<Vec<_>>()
                    .join(", "),
                last_contents
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

pub(super) fn saved_configuration_mismatch_message(
    settings: &StoredClientSettingsMvp,
    media_search_directory: &str,
) -> Option<String> {
    let expected = expected_saved_configuration(media_search_directory);

    let mut normalized = settings.clone();
    if normalized
        .last_checked_for_updates
        .as_deref()
        .is_some_and(looks_like_legacy_update_timestamp)
    {
        normalized.last_checked_for_updates = None;
    }

    if normalized == expected {
        None
    } else {
        Some(format!("expected {:?}, got {:?}", expected, settings,))
    }
}

pub(super) fn looks_like_legacy_update_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 23
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b' '
        && bytes[13] == b':'
        && bytes[16] == b':'
        && bytes[19] == b'.'
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            10 => *byte == b' ',
            13 | 16 => *byte == b':',
            19 => *byte == b'.',
            _ => byte.is_ascii_digit(),
        })
}

pub(super) fn expected_saved_configuration(
    media_search_directory: &str,
) -> StoredClientSettingsMvp {
    StoredClientSettingsMvp {
        host: Some(CONFIG_HOST_VALUE.to_owned()),
        port: Some(CONFIG_PORT_VALUE.parse().unwrap()),
        username: Some(CONFIG_USERNAME_VALUE.to_owned()),
        room: Some(CONFIG_ROOM_VALUE.to_owned()),
        player_path: Some(CONFIG_PLAYER_PATH_VALUE.to_owned()),
        public_servers: Some(vec![
            ("Alpha".to_owned(), "alpha.example:8999".to_owned()),
            ("Beta".to_owned(), "beta.example:9000".to_owned()),
        ]),
        ready_at_start: Some(true),
        autoplay_initial_state: Some(true),
        autoplay_require_same_filenames: Some(true),
        shared_playlist_enabled: Some(true),
        pause_on_leave: Some(true),
        unpause_action: Some(UnpauseActionMode::Always),
        autoplay_min_users: Some(AutoplayThresholdOverride::Set(3)),
        filename_privacy_mode: Some(PrivacyMode::SendHashed),
        filesize_privacy_mode: Some(PrivacyMode::DoNotSend),
        only_switch_to_trusted_domains: Some(true),
        trusted_domains: Some(vec![
            "youtube.com".to_owned(),
            "*.example.com/videos".to_owned(),
        ]),
        rewind_on_desync: Some(true),
        fastforward_on_desync: Some(true),
        slow_on_desync: Some(true),
        dont_slow_down_with_me: Some(true),
        rewind_threshold_seconds: Some(CONFIG_REWIND_THRESHOLD_VALUE.parse().unwrap()),
        fastforward_threshold_seconds: Some(CONFIG_FASTFORWARD_THRESHOLD_VALUE.parse().unwrap()),
        slowdown_threshold_seconds: Some(CONFIG_SLOWDOWN_THRESHOLD_VALUE.parse().unwrap()),
        media_search_directories: Some(vec![media_search_directory.to_owned()]),
        folder_search_first_file_timeout_seconds: Some(MEDIA_SEARCH_FIRST_FILE_TIMEOUT_SECONDS),
        folder_search_timeout_seconds: Some(MEDIA_SEARCH_TIMEOUT_SECONDS),
        folder_search_double_check_interval_seconds: Some(
            MEDIA_SEARCH_DOUBLE_CHECK_INTERVAL_SECONDS,
        ),
        folder_search_warning_threshold_seconds: Some(MEDIA_SEARCH_WARNING_THRESHOLD_SECONDS),
        chat_input_enabled: Some(true),
        chat_output_enabled: Some(true),
        chat_direct_input: Some(true),
        chat_move_osd: Some(true),
        chat_max_lines: Some(CONFIG_CHAT_MAX_LINES_VALUE.parse().unwrap()),
        chat_input_font_family: Some(CONFIG_CHAT_INPUT_FONT_VALUE.to_owned()),
        chat_output_font_family: Some(CONFIG_CHAT_OUTPUT_FONT_VALUE.to_owned()),
        show_osd: Some(true),
        show_duration_notification: Some(true),
        show_same_room_osd: Some(true),
        show_osd_warnings: Some(true),
        show_noncontroller_osd: Some(true),
        show_different_room_osd: Some(true),
        show_contact_info: Some(true),
        language: Some(CONFIG_LANGUAGE_VALUE.to_owned()),
        check_for_updates_automatically: Some(true),
        ..StoredClientSettingsMvp::default()
    }
}

pub(super) fn wait_for_saved_configuration(
    config_path: &Path,
    media_search_directory: &str,
    timeout: Duration,
) -> Result<StoredClientSettingsMvp, String> {
    let deadline = Instant::now() + timeout;
    let mut last_contents = String::new();

    let last_mismatch = loop {
        let mismatch = match fs::read_to_string(config_path) {
            Ok(contents) => {
                last_contents = contents;
                match load_sorotte_ini_stored_client_settings_mvp_from_path(config_path) {
                    Ok(Some(settings)) => {
                        if let Some(mismatch) =
                            saved_configuration_mismatch_message(&settings, media_search_directory)
                        {
                            mismatch
                        } else {
                            return Ok(settings);
                        }
                    }
                    Ok(None) => "config file parsed successfully but did not contain any settings"
                        .to_owned(),
                    Err(error) => format!("config file parse failed: {error}"),
                }
            }
            Err(error) => format!("config file read failed: {error}"),
        };

        if Instant::now() >= deadline {
            break mismatch;
        }
        thread::sleep(Duration::from_millis(50));
    };

    Err(format!(
        "timed out waiting for configuration {} to match the expected first-run save contract; last mismatch: {}; last file contents:\n{}",
        config_path.display(),
        last_mismatch,
        last_contents,
    ))
}
