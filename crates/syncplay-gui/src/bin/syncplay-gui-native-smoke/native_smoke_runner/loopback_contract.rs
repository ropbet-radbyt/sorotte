use super::*;

pub(super) fn verify_loopback_chat_contract<D: NativeGuiDriver>(
    driver: &D,
    binary_path: &Path,
    config_path: &Path,
    media_search_browse_path: &Path,
    open_media_file_path: &Path,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let launch = GuiLaunchConfig {
        config_path,
        media_search_browse_path,
        open_media_file_path,
        public_servers_spec: DEFAULT_PUBLIC_SERVERS_SPEC,
        tcp_session: None,
        loopback_session: Some((TRANSPORT_SESSION_USERNAME, TRANSPORT_SESSION_ROOM)),
        attach_test_player: false,
        drop_file_paths_spec: None,
        drop_target: None,
    };
    let (mut child, window) = launch_syncplay_gui_with_retry(driver, binary_path, launch, timeout)?;

    let outcome = (|| -> Result<Vec<String>, String> {
        let step_timeout = timeout.min(Duration::from_millis(6_000));
        let mut steps = Vec::new();

        wait_for_any_accessible_name(
            driver,
            window,
            &["view: configuration", "view: main-window"],
            step_timeout,
        )?;
        navigate_to_view_with_fallback(
            driver,
            window,
            "Main Window",
            "view: main-window",
            "Window",
            "Show Users",
            step_timeout,
        )?;

        send_chat_message_and_complete(driver, window, "helloloopback", step_timeout)?;
        steps.push("loopback-chat-send".to_owned());

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
