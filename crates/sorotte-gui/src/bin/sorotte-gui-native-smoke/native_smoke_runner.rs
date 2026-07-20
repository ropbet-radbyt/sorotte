use super::*;

#[path = "native_smoke_runner/baseline_contract.rs"]
mod baseline_contract;
#[path = "native_smoke_runner/drag_drop_contract.rs"]
mod drag_drop_contract;
#[path = "native_smoke_runner/live_python_contracts.rs"]
mod live_python_contracts;
#[path = "native_smoke_runner/loopback_contract.rs"]
mod loopback_contract;
#[path = "native_smoke_runner/missing_media_contracts.rs"]
mod missing_media_contracts;
#[path = "native_smoke_runner/relaunch_contract.rs"]
mod relaunch_contract;
#[path = "native_smoke_runner/transport_contract.rs"]
mod transport_contract;

use baseline_contract::verify_interaction_contract;
use drag_drop_contract::verify_drag_and_drop_contract;
use live_python_contracts::{
    verify_live_python_peer_connect_contract, verify_live_python_peer_controlled_room_contract,
};
use loopback_contract::verify_loopback_chat_contract;
use missing_media_contracts::{
    verify_detached_missing_media_contract, verify_missing_media_continue_session_contract,
};
use relaunch_contract::verify_relaunch_config_reload_contract;
use transport_contract::verify_transport_reconnect_contract;

#[path = "native_smoke_runner/shared_helpers.rs"]
mod shared_helpers;
use shared_helpers::*;

pub(super) fn start_visual_mock_session_server(
    initial_lines: &'static [&'static str],
) -> Result<MockSessionServer, String> {
    start_mock_session_server_with_hold_timeout(initial_lines, &[], &[], Duration::from_secs(60))
}

pub(super) fn visual_mock_session_server_port(server: &MockSessionServer) -> u16 {
    server.port
}

pub(super) fn recv_visual_mock_session_hello(
    server: &MockSessionServer,
    timeout: Duration,
    label: &str,
) -> Result<String, String> {
    server.recv_hello(timeout, label)
}

pub(super) fn release_visual_mock_session_server(
    server: MockSessionServer,
    label: &str,
) -> Result<(), String> {
    server.release(label)
}

pub(super) fn run_native_smoke(options: &NativeSmokeOptions) -> Result<NativeSmokeReport, String> {
    let configured_binary_path = options
        .binary_path
        .clone()
        .unwrap_or_else(default_binary_path);
    let binary_path = resolve_binary_path(&configured_binary_path)?;
    if !binary_path.exists() {
        return Err(format!(
            "sorotte-gui binary does not exist: {binary_path:?}"
        ));
    }

    let temp_root = std::env::temp_dir().join(format!(
        "sorotte-gui-native-smoke-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    fs::create_dir_all(&temp_root)
        .map_err(|error| format!("failed to create native smoke temp directory: {error}"))?;
    let config_path = temp_root.join("sorotte-native-smoke.ini");
    let media_search_browse_path = temp_root.join("media-search");
    let open_media_file_path = temp_root.join("open-target.mkv");
    let _ = fs::remove_file(&config_path);
    fs::create_dir_all(&media_search_browse_path)
        .map_err(|error| format!("failed to create native smoke media directory: {error}"))?;
    fs::write(&open_media_file_path, b"open-target")
        .map_err(|error| format!("failed to create native smoke media file: {error}"))?;
    seed_native_smoke_config(&config_path)?;

    let started_at = Instant::now();
    let driver = PlatformNativeGuiDriver;
    let launch = GuiLaunchConfig {
        config_path: &config_path,
        media_search_browse_path: &media_search_browse_path,
        open_media_file_path: &open_media_file_path,
        public_servers_spec: DEFAULT_PUBLIC_SERVERS_SPEC,
        tcp_session: None,
        loopback_session: None,
        attach_test_player: false,
        drop_file_paths_spec: None,
        drop_target: None,
    };
    let (mut child, window) =
        launch_sorotte_gui_with_retry(&driver, &binary_path, launch, options.timeout)?;
    let pid = child.id();

    let result = (|| {
        let window_title = driver.window_title(window)?;
        if !window_title.contains("Sorotte") {
            return Err(format!(
                "main window title did not match expected prefix; got {window_title:?}"
            ));
        }

        let accessible_names = driver.accessible_names(window)?;
        verify_accessibility_contract(&accessible_names)?;
        let mut interaction_steps = if scenario_selected(options, "baseline") {
            verify_interaction_contract(
                &driver,
                window,
                &config_path,
                &media_search_browse_path,
                &open_media_file_path,
                options.timeout,
            )?
        } else {
            Vec::new()
        };
        if scenario_selected(options, "drag-drop") {
            interaction_steps.extend(verify_drag_and_drop_contract(
                &driver,
                &binary_path,
                &temp_root,
                &media_search_browse_path,
                options.timeout,
            )?);
        }
        let interaction_contract = "verified".to_owned();

        let menu_labels = driver.top_level_menu_labels(window)?;
        let menu_contract = if menu_labels.is_empty() {
            "skipped-no-native-menu".to_owned()
        } else {
            verify_menu_contract(&menu_labels)?;
            "verified".to_owned()
        };
        let accessibility_contract = "verified".to_owned();

        if options.keep_open {
            return Ok(NativeSmokeReport {
                binary_path: binary_path.display().to_string(),
                pid,
                window_title,
                menu_labels,
                menu_contract,
                accessible_name_count: accessible_names.len(),
                accessibility_contract,
                interaction_steps,
                interaction_contract,
                duration_ms: started_at.elapsed().as_millis(),
                closed: false,
            });
        }

        let close_step_timeout = options.timeout.min(Duration::from_millis(4_000));
        let closed_via_file_exit = if let Err(primary_error) = invoke_menu_command_with_wait(
            &driver,
            window,
            "File",
            "Exit",
            NativeControlKind::MenuItem,
            close_step_timeout,
        ) {
            match invoke_menu_command_with_wait(
                &driver,
                window,
                "File",
                "Exit",
                NativeControlKind::Any,
                close_step_timeout,
            ) {
                Ok(()) => {
                    wait_for_process_exit(&mut child, options.timeout)?;
                    interaction_steps.push("file-exit".to_owned());
                    true
                }
                Err(fallback_error) => {
                    interaction_steps.push(format!(
                        "file-exit-skipped:{}",
                        format!(
                            "menu-item-failure={primary_error}; fallback-failure={fallback_error}"
                        )
                        .replace('|', "/")
                        .replace('\n', " ")
                    ));
                    false
                }
            }
        } else {
            wait_for_process_exit(&mut child, options.timeout)?;
            interaction_steps.push("file-exit".to_owned());
            true
        };
        if !closed_via_file_exit {
            driver.close_window(window)?;
            wait_for_process_exit(&mut child, options.timeout)?;
            interaction_steps.push("window-close-fallback".to_owned());
        }

        if scenario_selected(options, "relaunch") {
            let relaunch_steps = verify_relaunch_config_reload_contract(
                &driver,
                &binary_path,
                &config_path,
                &media_search_browse_path,
                &open_media_file_path,
                options.timeout,
            )?;
            interaction_steps.extend(relaunch_steps);
        }

        if scenario_selected(options, "loopback") {
            let loopback_steps = verify_loopback_chat_contract(
                &driver,
                &binary_path,
                &config_path,
                &media_search_browse_path,
                &open_media_file_path,
                options.timeout,
            )?;
            interaction_steps.extend(loopback_steps);
        }

        if scenario_selected(options, "live-python") {
            let live_python_interop_steps = verify_live_python_peer_connect_contract(
                &driver,
                &binary_path,
                &temp_root,
                &media_search_browse_path,
                &open_media_file_path,
                options.timeout,
            )?;
            interaction_steps.extend(live_python_interop_steps);
        }

        if scenario_selected(options, "controlled-room") {
            let live_python_controlled_room_steps =
                verify_live_python_peer_controlled_room_contract(
                    &driver,
                    &binary_path,
                    &temp_root,
                    &media_search_browse_path,
                    &open_media_file_path,
                    options.timeout,
                )?;
            interaction_steps.extend(live_python_controlled_room_steps);
        }

        if scenario_selected(options, "detached-missing-media") {
            let detached_missing_media_steps = verify_detached_missing_media_contract(
                &driver,
                &binary_path,
                &temp_root,
                options.timeout,
            )?;
            interaction_steps.extend(detached_missing_media_steps);
        }

        if scenario_selected(options, "missing-media-continue") {
            let missing_media_continue_steps = verify_missing_media_continue_session_contract(
                &driver,
                &binary_path,
                &temp_root,
                &media_search_browse_path,
                &open_media_file_path,
                options.timeout,
            )?;
            interaction_steps.extend(missing_media_continue_steps);
        }

        if scenario_selected(options, "transport") {
            let transport_steps = verify_transport_reconnect_contract(
                &driver,
                &binary_path,
                &temp_root,
                &media_search_browse_path,
                &open_media_file_path,
                options.timeout,
            )?;
            interaction_steps.extend(transport_steps);
        }

        Ok(NativeSmokeReport {
            binary_path: binary_path.display().to_string(),
            pid,
            window_title,
            menu_labels,
            menu_contract,
            accessible_name_count: accessible_names.len(),
            accessibility_contract,
            interaction_steps,
            interaction_contract,
            duration_ms: started_at.elapsed().as_millis(),
            closed: true,
        })
    })();

    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }

    let _ = fs::remove_file(&config_path);
    let _ = fs::remove_dir_all(&temp_root);

    result
}
