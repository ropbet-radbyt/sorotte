use super::*;

pub(super) fn verify_drag_and_drop_contract<D: NativeGuiDriver>(
    driver: &D,
    binary_path: &Path,
    temp_root: &Path,
    media_search_browse_path: &Path,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let step_timeout = timeout.min(Duration::from_millis(6_000));
    let mut steps = Vec::new();

    let window_drop_config_path = temp_root.join("drag-drop-window.ini");
    let window_drop_file_path = temp_root.join("drag-window-target.mkv");
    fs::write(&window_drop_file_path, b"drag-window-target")
        .map_err(|error| format!("failed to create native smoke drag-drop window file: {error}"))?;
    upsert_syncplay_ini_stored_client_settings_mvp_at_path(
        &window_drop_config_path,
        &StoredClientSettingsMvp::default(),
    )
    .map_err(|error| {
        format!(
            "failed to seed native smoke drag-drop window config {}: {error}",
            window_drop_config_path.display()
        )
    })?;
    let window_drop_spec = window_drop_file_path.display().to_string();
    let window_launch = GuiLaunchConfig {
        config_path: &window_drop_config_path,
        media_search_browse_path,
        open_media_file_path: &window_drop_file_path,
        public_servers_spec: DEFAULT_PUBLIC_SERVERS_SPEC,
        tcp_session: None,
        loopback_session: None,
        attach_test_player: true,
        drop_file_paths_spec: Some(&window_drop_spec),
        drop_target: Some("window"),
    };
    let (mut window_child, window_handle) =
        launch_syncplay_gui_with_retry(driver, binary_path, window_launch, timeout)?;
    let window_outcome = (|| -> Result<(), String> {
        wait_for_any_accessible_name(
            driver,
            window_handle,
            &["view: setup", "view: room"],
            step_timeout,
        )?;
        wait_for_accessible_name(driver, window_handle, "view: room", step_timeout)?;
        wait_for_accessible_name_with_page_down(
            driver,
            window_handle,
            "drag-window-target.mkv",
            4,
            step_timeout,
        )?;
        driver.close_window(window_handle)?;
        wait_for_process_exit(&mut window_child, timeout)?;
        Ok(())
    })();
    if window_outcome.is_err() {
        let _ = window_child.kill();
        let _ = window_child.wait();
    }
    window_outcome?;
    steps.push("drag-drop-window-media".to_owned());

    let playlist_drop_config_path = temp_root.join("drag-drop-playlist.ini");
    let playlist_drop_file_path = temp_root.join("drag-shared-playlist.m3u");
    fs::write(
        &playlist_drop_file_path,
        "drag-episode-1.mkv\ndrag-episode-2.mkv\n",
    )
    .map_err(|error| format!("failed to create native smoke drag-drop playlist file: {error}"))?;
    upsert_syncplay_ini_stored_client_settings_mvp_at_path(
        &playlist_drop_config_path,
        &StoredClientSettingsMvp {
            username: Some("drag-drop-user".to_owned()),
            room: Some("drag-drop-room".to_owned()),
            shared_playlist_enabled: Some(true),
            ..StoredClientSettingsMvp::default()
        },
    )
    .map_err(|error| {
        format!(
            "failed to seed native smoke drag-drop playlist config {}: {error}",
            playlist_drop_config_path.display()
        )
    })?;
    let playlist_drop_spec = playlist_drop_file_path.display().to_string();
    let playlist_launch = GuiLaunchConfig {
        config_path: &playlist_drop_config_path,
        media_search_browse_path,
        open_media_file_path: &playlist_drop_file_path,
        public_servers_spec: DEFAULT_PUBLIC_SERVERS_SPEC,
        tcp_session: None,
        loopback_session: Some(("drag-drop-user", "drag-drop-room")),
        attach_test_player: true,
        drop_file_paths_spec: Some(&playlist_drop_spec),
        drop_target: Some("playlist"),
    };
    let (mut playlist_child, playlist_handle) =
        launch_syncplay_gui_with_retry(driver, binary_path, playlist_launch, timeout)?;
    let playlist_outcome = (|| -> Result<(), String> {
        wait_for_any_accessible_name(
            driver,
            playlist_handle,
            &["view: setup", "view: room"],
            step_timeout,
        )?;
        wait_for_accessible_name(driver, playlist_handle, "view: room", step_timeout)?;
        wait_for_accessible_name(driver, playlist_handle, "drag-episode-1.mkv", step_timeout)?;
        wait_for_accessible_name(driver, playlist_handle, "drag-episode-2.mkv", step_timeout)?;
        wait_for_accessible_name_fragment(
            driver,
            playlist_handle,
            "Imported 2 entries into the shared playlist.",
            step_timeout,
        )?;
        driver.close_window(playlist_handle)?;
        wait_for_process_exit(&mut playlist_child, timeout)?;
        Ok(())
    })();
    if playlist_outcome.is_err() {
        let _ = playlist_child.kill();
        let _ = playlist_child.wait();
    }
    playlist_outcome?;
    steps.push("drag-drop-playlist-import".to_owned());

    Ok(steps)
}
