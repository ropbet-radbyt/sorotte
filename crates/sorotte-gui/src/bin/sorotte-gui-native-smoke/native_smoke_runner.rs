use super::*;

#[path = "native_smoke_runner/baseline_contract.rs"]
mod baseline_contract;
#[path = "native_smoke_runner/drag_drop_contract.rs"]
mod drag_drop_contract;
#[path = "native_smoke_runner/live_python_contracts.rs"]
mod live_python_contracts;
#[path = "native_smoke_runner/loopback_contract.rs"]
mod loopback_contract;
#[path = "native_smoke_runner/menu_open_media_contract.rs"]
mod menu_open_media_contract;
#[path = "native_smoke_runner/missing_media_contracts.rs"]
mod missing_media_contracts;
#[path = "native_smoke_runner/participant_status_system.rs"]
mod participant_status_system;
#[path = "native_smoke_runner/real_mpv_vertical.rs"]
mod real_mpv_vertical;
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
use menu_open_media_contract::verify_menu_open_media_contract;
use missing_media_contracts::{
    verify_detached_missing_media_contract, verify_missing_media_continue_session_contract,
};
pub(super) use participant_status_system::run_participant_status_system_from_args;
pub(super) use real_mpv_vertical::run_real_mpv_vertical_from_args;
use relaunch_contract::verify_relaunch_config_reload_contract;
use transport_contract::verify_transport_reconnect_contract;

#[path = "native_smoke_runner/shared_helpers.rs"]
mod shared_helpers;
use shared_helpers::*;

pub(super) fn start_visual_mock_session_server(
    initial_lines: &[&str],
) -> Result<MockSessionServer, String> {
    start_mock_session_server_with_keepalive(initial_lines)
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
    let lifecycle_observation_path = temp_root.join("primary-lifecycle.jsonl");
    let media_search_browse_path = temp_root.join("media-search");
    let open_media_file_path = temp_root.join("open-target.mkv");
    let _ = fs::remove_file(&config_path);
    let _ = fs::remove_file(&lifecycle_observation_path);
    fs::create_dir_all(&media_search_browse_path)
        .map_err(|error| format!("failed to create native smoke media directory: {error}"))?;
    fs::write(&open_media_file_path, b"open-target")
        .map_err(|error| format!("failed to create native smoke media file: {error}"))?;
    seed_native_smoke_config(&config_path)?;

    let started_at = Instant::now();
    let driver = PlatformNativeGuiDriver::new(options.input_mode);
    let launch = GuiLaunchConfig {
        config_path: &config_path,
        media_search_browse_path: &media_search_browse_path,
        open_media_file_path: &open_media_file_path,
        public_servers_spec: DEFAULT_PUBLIC_SERVERS_SPEC,
        network_mode: NativeNetworkMode::Detached,
        attach_test_player: false,
        drop_file_paths_spec: None,
        drop_target: None,
    };
    let (mut child, window) = launch_sorotte_gui_with_retry_and_test_overrides(
        &driver,
        &binary_path,
        launch,
        options.timeout,
        GuiLaunchTestOverrides {
            // The detached baseline verifies persisted configuration editing, not
            // connectivity. Keep its representative host names without performing
            // external DNS or socket I/O; connectivity scenarios own loopback fixtures.
            disable_startup_saved_connect: true,
            lifecycle_observation_path: Some(&lifecycle_observation_path),
            ..GuiLaunchTestOverrides::default()
        },
    )?;
    let pid = child.id();

    let result = (|| {
        let window_title = driver.window_title(window)?;
        if !window_title.contains("Sorotte") {
            return Err(format!(
                "main window title did not match expected prefix; got {window_title:?}"
            ));
        }

        let startup_modal_timeout = options.timeout.min(Duration::from_millis(4_000));
        if wait_for_accessible_name(
            &driver,
            window,
            "modal: player-setup",
            Duration::from_millis(800),
        )
        .is_ok()
        {
            invoke_named_control_with_wait(
                &driver,
                window,
                MODAL_CLOSE_AUTOMATION_ID,
                NativeControlKind::Button,
                startup_modal_timeout,
            )?;
            wait_for_accessible_name(&driver, window, "modal: (none)", startup_modal_timeout)?;
        }

        let accessible_names = driver.accessible_names(window)?;
        verify_accessibility_contract(&accessible_names)?;
        let accessibility_nodes = driver.accessibility_nodes(window)?;
        let menu_evidence = verify_menu_contract(&accessibility_nodes)?;
        let menu_source = MENU_SOURCE_UIA_ACCESSKIT.to_owned();
        let menu_labels = menu_evidence.labels;
        let menu_automation_ids = menu_evidence.automation_ids;
        let menu_contract = "verified".to_owned();
        let accessibility_contract = "verified".to_owned();
        if options.input_mode == NativeInputMode::UiaOnly {
            let close_step_timeout = options.timeout.min(Duration::from_millis(4_000));
            invoke_menu_action_by_id_uia_only_with_wait(
                &driver,
                window,
                FILE_MENU_AUTOMATION_ID,
                EXIT_MENU_AUTOMATION_ID,
                close_step_timeout,
            )?;
            wait_for_process_exit(&mut child, close_step_timeout)?;
            wait_for_lifecycle_events(
                &lifecycle_observation_path,
                &[
                    "exit-action-applied",
                    "viewport-close-requested",
                    "runtime-stop-requested",
                    "runtime-worker-stopped",
                    "app-drop-complete",
                ],
                close_step_timeout,
            )?;
            let desktop_input_attempts = driver.desktop_input_attempt_count();
            if desktop_input_attempts != 0 {
                return Err(format!(
                    "uia-only native smoke reached {desktop_input_attempts} blocked desktop-input attempt(s)"
                ));
            }
            let interaction_steps = vec![
                "uia-only-menu-inventory".to_owned(),
                "uia-only-file-exit".to_owned(),
                "uia-only-file-exit-lifecycle-observed".to_owned(),
            ];
            let capability_outcomes =
                uia_only_capability_outcomes(&menu_automation_ids, desktop_input_attempts);
            return Ok(NativeSmokeReport {
                input_mode: options.input_mode,
                binary_path: binary_path.display().to_string(),
                pid,
                window_title,
                menu_source,
                menu_labels,
                menu_automation_ids,
                menu_contract,
                accessible_name_count: accessible_names.len(),
                accessibility_contract,
                interaction_steps,
                interaction_contract: "local-uia-only-non-authoritative".to_owned(),
                capability_outcomes,
                duration_ms: started_at.elapsed().as_millis(),
                closed: true,
            });
        }
        let mut interaction_steps = if scenario_selected(options, "baseline") {
            verify_interaction_contract(
                &driver,
                window,
                &config_path,
                &media_search_browse_path,
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

        if options.keep_open {
            let capability_outcomes =
                native_capability_outcomes(&menu_automation_ids, &interaction_steps);
            return Ok(NativeSmokeReport {
                input_mode: options.input_mode,
                binary_path: binary_path.display().to_string(),
                pid,
                window_title,
                menu_source,
                menu_labels,
                menu_automation_ids,
                menu_contract,
                accessible_name_count: accessible_names.len(),
                accessibility_contract,
                interaction_steps,
                interaction_contract,
                capability_outcomes,
                duration_ms: started_at.elapsed().as_millis(),
                closed: false,
            });
        }

        let close_step_timeout = options.timeout.min(Duration::from_millis(4_000));
        invoke_menu_action_by_id_with_wait(
            &driver,
            window,
            FILE_MENU_AUTOMATION_ID,
            EXIT_MENU_AUTOMATION_ID,
            close_step_timeout,
        )?;
        wait_for_process_exit(&mut child, close_step_timeout)?;
        wait_for_lifecycle_events(
            &lifecycle_observation_path,
            &[
                "exit-action-applied",
                "viewport-close-requested",
                "runtime-stop-requested",
                "runtime-worker-stopped",
                "app-drop-complete",
            ],
            close_step_timeout,
        )?;
        interaction_steps.push("file-exit".to_owned());
        interaction_steps.push("file-exit-lifecycle-observed".to_owned());

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

        if scenario_selected(options, "menu-open-media") {
            let menu_open_media_steps = verify_menu_open_media_contract(
                &driver,
                &binary_path,
                &temp_root,
                &media_search_browse_path,
                options.timeout,
            )?;
            interaction_steps.extend(menu_open_media_steps);
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

        let capability_outcomes =
            native_capability_outcomes(&menu_automation_ids, &interaction_steps);
        Ok(NativeSmokeReport {
            input_mode: options.input_mode,
            binary_path: binary_path.display().to_string(),
            pid,
            window_title,
            menu_source,
            menu_labels,
            menu_automation_ids,
            menu_contract,
            accessible_name_count: accessible_names.len(),
            accessibility_contract,
            interaction_steps,
            interaction_contract,
            capability_outcomes,
            duration_ms: started_at.elapsed().as_millis(),
            closed: true,
        })
    })();

    if let Err(error) = &result {
        if child.try_wait().ok().flatten().is_none() {
            capture_native_failure_artifacts(&driver, window, "primary", error);
        }
        let _ = child.kill();
        let _ = child.wait();
    }

    let _ = fs::remove_file(&config_path);
    let _ = fs::remove_dir_all(&temp_root);

    result
}

fn native_capability_outcomes(
    menu_automation_ids: &[String],
    interaction_steps: &[String],
) -> Vec<NativeCapabilityOutcome> {
    let has_step = |expected: &str| interaction_steps.iter().any(|step| step == expected);
    let mut outcomes = vec![NativeCapabilityOutcome {
        capability_id: "native.menu.inventory".to_owned(),
        outcome: "required-pass".to_owned(),
        source: MENU_SOURCE_UIA_ACCESSKIT.to_owned(),
        evidence: menu_automation_ids.to_vec(),
    }];
    if has_step("file-exit-lifecycle-observed") {
        outcomes.push(NativeCapabilityOutcome {
            capability_id: "native.shutdown.file-exit".to_owned(),
            outcome: "required-pass".to_owned(),
            source: "accesskit+eframe+lifecycle-jsonl".to_owned(),
            evidence: vec![
                "exit-action-applied".to_owned(),
                "viewport-close-requested".to_owned(),
                "runtime-stop-requested".to_owned(),
                "runtime-worker-stopped".to_owned(),
                "app-drop-complete".to_owned(),
            ],
        });
    }
    if has_step("menu-input-stress-25") {
        outcomes.push(NativeCapabilityOutcome {
            capability_id: "native.menu.physical-input".to_owned(),
            outcome: "required-pass".to_owned(),
            source: "uia-hit-test+win32-sendinput".to_owned(),
            evidence: vec![
                "menu-input-stress-25".to_owned(),
                "menu-input-single-delivery".to_owned(),
            ],
        });
    }
    if has_step("open-media-file-detached-disabled") {
        outcomes.push(NativeCapabilityOutcome {
            capability_id: "native.menu.open-media.detached".to_owned(),
            outcome: "required-pass".to_owned(),
            source: MENU_SOURCE_UIA_ACCESSKIT.to_owned(),
            evidence: vec![
                format!("{OPEN_MEDIA_MENU_AUTOMATION_ID}=disabled"),
                "open-media-file-detached-disabled".to_owned(),
            ],
        });
    }
    if has_step("menu-open-media-runtime-observed") {
        outcomes.push(NativeCapabilityOutcome {
            capability_id: "native.menu.open-media.attached".to_owned(),
            outcome: "required-pass".to_owned(),
            source: "uia-accesskit+deterministic-test-player".to_owned(),
            evidence: vec![
                format!("{OPEN_MEDIA_MENU_AUTOMATION_ID}=enabled"),
                "menu-open-media-invoked-by-automation-id".to_owned(),
                "player.open_file=observed".to_owned(),
            ],
        });
    }
    outcomes
}

fn uia_only_capability_outcomes(
    menu_automation_ids: &[String],
    desktop_input_attempts: usize,
) -> Vec<NativeCapabilityOutcome> {
    let no_desktop_input = format!("desktop-input-attempt-count={desktop_input_attempts}");
    vec![
        NativeCapabilityOutcome {
            capability_id: "native.menu.inventory".to_owned(),
            outcome: "required-pass".to_owned(),
            source: MENU_SOURCE_UIA_ACCESSKIT.to_owned(),
            evidence: menu_automation_ids.to_vec(),
        },
        NativeCapabilityOutcome {
            capability_id: "native.shutdown.file-exit".to_owned(),
            outcome: "required-pass".to_owned(),
            source: "uia-accesskit+eframe+lifecycle-jsonl".to_owned(),
            evidence: vec![
                "exit-action-applied".to_owned(),
                "viewport-close-requested".to_owned(),
                "runtime-stop-requested".to_owned(),
                "runtime-worker-stopped".to_owned(),
                "app-drop-complete".to_owned(),
            ],
        },
        NativeCapabilityOutcome {
            capability_id: "native.menu.physical-input".to_owned(),
            outcome: "optional-skip".to_owned(),
            source: "local-uia-mode".to_owned(),
            evidence: vec![
                "reason=local-uia-mode".to_owned(),
                "win32-sendinput=disabled".to_owned(),
                no_desktop_input.clone(),
            ],
        },
        NativeCapabilityOutcome {
            capability_id: "native.input.focused-keyboard".to_owned(),
            outcome: "optional-skip".to_owned(),
            source: "local-uia-mode".to_owned(),
            evidence: vec![
                "reason=local-uia-mode".to_owned(),
                "focused-keyboard-fallback=disabled".to_owned(),
                "win32-sendinput=disabled".to_owned(),
                no_desktop_input,
            ],
        },
    ]
}
