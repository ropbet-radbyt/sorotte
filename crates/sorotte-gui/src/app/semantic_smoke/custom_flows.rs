use std::path::PathBuf;

use super::super::{
    GuiAppHost, GuiLaunchMode, GuiPendingCompletionRequest, GuiPersistedConfigRuntimeOwner,
    GuiPersistedUiState, GuiQueuedRuntimeBridgeHandle, GuiRuntimeRequest, GuiShellAction,
    GuiShellView, SorotteGuiShellAppState, StoredClientSettingsMvp, load_gui_ui_state_from_root,
    persist_gui_ui_state_at_root, run_gui_host_with_startup_actions_and_gui_state,
    upsert_sorotte_ini_stored_client_settings_mvp_at_path,
};
use super::GuiSemanticScenarioReport;

fn semantic_temp_root(prefix: &str) -> PathBuf {
    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "sorotte-gui-semantic-{prefix}-{}-{unique_suffix}",
        std::process::id()
    ))
}

fn report_from_state(name: &str, state: &SorotteGuiShellAppState) -> GuiSemanticScenarioReport {
    GuiSemanticScenarioReport {
        scenario: name.to_owned(),
        view: state.active_view.label().to_owned(),
        modal: state
            .open_modal
            .map(|modal| modal.label().to_owned())
            .unwrap_or_else(|| "none".to_owned()),
        pending: state
            .pending_operation
            .as_ref()
            .map(|pending| pending.kind.label().to_owned())
            .unwrap_or_else(|| "none".to_owned()),
        widgets: state.shell_widget_tree().node_count(),
    }
}

pub(super) fn run_gui_semantic_persistence_reset_flow() -> Result<GuiSemanticScenarioReport, String>
{
    #[derive(Default)]
    struct RecordingHost;

    impl GuiAppHost for RecordingHost {
        type Output = SorotteGuiShellAppState;

        fn render(&mut self, state: SorotteGuiShellAppState) -> Self::Output {
            state
        }
    }

    let root = semantic_temp_root("persistence-reset");
    let result = (|| -> Result<GuiSemanticScenarioReport, String> {
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).map_err(|error| {
            format!(
                "failed to create semantic persistence-reset temp root {}: {error}",
                root.display()
            )
        })?;
        let config_path = root.join("sorotte.ini");
        let settings = StoredClientSettingsMvp {
            host: Some("persisted.example".to_owned()),
            room: Some("PersistenceRoom".to_owned()),
            player_path: Some("C:/Windows/System32/notepad.exe".to_owned()),
            media_search_directories: Some(vec!["C:/Media".to_owned()]),
            ..StoredClientSettingsMvp::default()
        };
        upsert_sorotte_ini_stored_client_settings_mvp_at_path(&config_path, &settings).map_err(
            |error| {
                format!(
                    "failed to seed semantic persistence-reset config {}: {error}",
                    config_path.display()
                )
            },
        )?;

        let persisted_ui_state = GuiPersistedUiState {
            active_view: Some(GuiShellView::Setup),
            selected_public_server_address: Some("custom.example:9001".to_owned()),
            selected_media_search_directory: Some("C:/Media".to_owned()),
            hide_empty_rooms: false,
            last_media_dialog_directory: Some("D:/Dialogs".to_owned()),
            last_checked_for_updates: None,
            public_servers: vec![("Custom".to_owned(), "custom.example:9001".to_owned())],
            ..Default::default()
        };
        persist_gui_ui_state_at_root(&root, &persisted_ui_state).map_err(|error| {
            format!(
                "failed to seed semantic persistence-reset GUI state at {}: {error}",
                root.display()
            )
        })?;
        let loaded_ui_state = load_gui_ui_state_from_root(&root)?
            .ok_or_else(|| "seeded semantic GUI state was not reloadable".to_owned())?;

        let mut host = RecordingHost;
        let restored_state = run_gui_host_with_startup_actions_and_gui_state(
            &settings,
            Some(&loaded_ui_state),
            Vec::new(),
            &mut host,
        );
        if restored_state.active_view != GuiShellView::Setup {
            return Err(format!(
                "expected persisted semantic startup view setup, got {}",
                restored_state.active_view.label()
            ));
        }
        if restored_state.selected_public_server_index() != Some(0) {
            return Err(
                "expected persisted semantic startup to restore the custom server selection"
                    .to_owned(),
            );
        }
        if restored_state.selection.selected_media_search_directory != Some(0) {
            return Err(
                "expected persisted semantic startup to restore the media-search selection"
                    .to_owned(),
            );
        }
        if restored_state.last_media_dialog_directory.as_deref() != Some("D:/Dialogs") {
            return Err(
                "expected persisted semantic startup to restore the last media dialog directory"
                    .to_owned(),
            );
        }
        if restored_state.saved_configuration.public_servers.as_ref()
            != Some(&persisted_ui_state.public_servers)
        {
            return Err(
                "expected persisted semantic startup to promote GUI public servers when sorotte.ini had none"
                    .to_owned(),
            );
        }

        let mut host = RecordingHost;
        let migrated_state = run_gui_host_with_startup_actions_and_gui_state(
            &StoredClientSettingsMvp {
                host: Some("saved.example".to_owned()),
                port: Some(8999),
                public_servers: Some(vec![("Saved".to_owned(), "saved.example:8999".to_owned())]),
                ..settings.clone()
            },
            Some(&loaded_ui_state),
            Vec::new(),
            &mut host,
        );
        let migrated_servers = migrated_state
            .public_servers
            .servers
            .iter()
            .map(|row| (row.label.clone(), row.address.clone()))
            .collect::<Vec<_>>();
        if migrated_servers != vec![("Custom".to_owned(), "custom.example:9001".to_owned())] {
            return Err(format!(
                "expected GUI-owned public servers to win over sorotte.ini migration rows, got {:?}",
                migrated_servers
            ));
        }

        let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path));
        let handle = GuiQueuedRuntimeBridgeHandle::default();
        let mut clear_state = restored_state;
        if !clear_state.apply(GuiShellAction::BeginClearGuiData) {
            return Err("failed to begin semantic clear-GUI-data flow".to_owned());
        }
        if !clear_state.apply(GuiShellAction::ConfirmClearGuiData) {
            return Err("failed to confirm semantic clear-GUI-data flow".to_owned());
        }
        handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
            GuiPendingCompletionRequest::ClearGuiData,
        ));
        owner.pump_compatibility_state(&handle, &clear_state);
        let actions = handle.drain_actions();
        if !actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteClearGuiData))
        {
            return Err(
                "semantic clear-GUI-data flow did not produce a completion action".to_owned(),
            );
        }
        for action in actions {
            if !clear_state.apply(action) {
                return Err("semantic clear-GUI-data completion action was rejected".to_owned());
            }
        }

        if root.join("sorotte.ini").exists() {
            return Err("semantic clear-GUI-data flow did not remove sorotte.ini".to_owned());
        }
        if load_gui_ui_state_from_root(&root)?.is_some() {
            return Err(
                "semantic clear-GUI-data flow did not remove the persisted GUI state stores"
                    .to_owned(),
            );
        }
        if clear_state.configuration.launch_mode != GuiLaunchMode::FirstRun {
            return Err(
                "semantic clear-GUI-data flow did not restore first-run launch mode".to_owned(),
            );
        }
        if clear_state.active_view != GuiShellView::Setup {
            return Err("semantic clear-GUI-data flow did not restore the setup view".to_owned());
        }
        if clear_state.saved_configuration != StoredClientSettingsMvp::default() {
            return Err(
                "semantic clear-GUI-data flow did not restore the default saved configuration"
                    .to_owned(),
            );
        }

        Ok(report_from_state("persistence-reset-flow", &clear_state))
    })();
    let _ = std::fs::remove_dir_all(&root);
    result
}

pub(super) fn run_gui_semantic_detached_runtime_ownership_flow()
-> Result<GuiSemanticScenarioReport, String> {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    fn read_client_hello_after_optional_start_tls<R, W>(
        reader: &mut R,
        writer: &mut W,
    ) -> Result<String, String>
    where
        R: BufRead,
        W: Write,
    {
        let mut first_line = String::new();
        reader
            .read_line(&mut first_line)
            .map_err(|error| format!("detached semantic TCP hello read failed: {error}"))?;
        if first_line.contains("\"TLS\"") {
            writer
                .write_all(br#"{"TLS":{"startTLS":"false"}}"#)
                .map_err(|error| {
                    format!("detached semantic TCP TLS response write failed: {error}")
                })?;
            writer.write_all(b"\n").map_err(|error| {
                format!("detached semantic TCP TLS response termination failed: {error}")
            })?;
            writer.flush().map_err(|error| {
                format!("detached semantic TCP TLS response flush failed: {error}")
            })?;

            let mut hello_line = String::new();
            reader.read_line(&mut hello_line).map_err(|error| {
                format!("detached semantic TCP hello-after-TLS read failed: {error}")
            })?;
            Ok(hello_line)
        } else {
            Ok(first_line)
        }
    }

    fn recv_from_channel_while_pumping_runtime<T>(
        owner: &mut GuiPersistedConfigRuntimeOwner,
        handle: &GuiQueuedRuntimeBridgeHandle,
        state: &mut SorotteGuiShellAppState,
        receiver: &mpsc::Receiver<T>,
        timeout: Duration,
        context: &str,
    ) -> Result<T, String> {
        let deadline = Instant::now() + timeout;
        loop {
            owner.pump_compatibility_state(handle, state);
            let actions = handle.drain_actions();
            for action in actions {
                if !state.apply(action) {
                    return Err(format!(
                        "{context} rejected a projected runtime action while waiting"
                    ));
                }
            }
            if let Ok(value) = receiver.try_recv() {
                return Ok(value);
            }
            if Instant::now() >= deadline {
                return Err(format!("timed out waiting for {context}"));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("failed to bind detached semantic TCP listener: {error}"))?;
    let address = listener.local_addr().map_err(|error| {
        format!("failed to read detached semantic TCP listener address: {error}")
    })?;
    let (hello_tx, hello_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server_thread = thread::spawn(move || -> Result<(), String> {
        let (mut stream, _) = listener
            .accept()
            .map_err(|error| format!("detached semantic TCP listener accept failed: {error}"))?;
        let mut reader = BufReader::new(
            stream
                .try_clone()
                .map_err(|error| format!("detached semantic TCP stream clone failed: {error}"))?,
        );
        let hello_line = read_client_hello_after_optional_start_tls(&mut reader, &mut stream)?;
        hello_tx
            .send(hello_line)
            .map_err(|error| format!("detached semantic TCP hello report failed: {error}"))?;
        stream
            .write_all(
                br#"{"Hello":{"username":"semantic-user","room":{"name":"semantic-room"},"version":"1.7.5","features":{"chat":true}}}"#,
            )
            .map_err(|error| format!("detached semantic TCP write failed: {error}"))?;
        stream
            .write_all(b"\r\n")
            .map_err(|error| format!("detached semantic TCP line termination failed: {error}"))?;
        stream
            .flush()
            .map_err(|error| format!("detached semantic TCP flush failed: {error}"))?;
        release_rx
            .recv_timeout(Duration::from_secs(1))
            .map_err(|error| format!("detached semantic TCP release wait failed: {error}"))?;
        Ok(())
    });

    let root = semantic_temp_root("detached-runtime");
    let result = (|| -> Result<GuiSemanticScenarioReport, String> {
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).map_err(|error| {
            format!(
                "failed to create detached semantic temp root {}: {error}",
                root.display()
            )
        })?;
        let found_path = root.join("missing-target.mkv");
        std::fs::write(&found_path, b"semantic-detached-runtime-target").map_err(|error| {
            format!(
                "failed to create detached semantic target {}: {error}",
                found_path.display()
            )
        })?;
        let found_path_text = found_path.display().to_string();

        let mut connect_owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        let connect_handle = GuiQueuedRuntimeBridgeHandle::default();
        let mut connect_state =
            SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
                username: Some("semantic-user".to_owned()),
                room: Some("semantic-room".to_owned()),
                public_servers: Some(vec![("Primary".to_owned(), address.to_string())]),
                shared_playlist_enabled: Some(true),
                ..StoredClientSettingsMvp::default()
            });

        if !connect_state.apply(GuiShellAction::SelectPublicServer(0))
            || !connect_state.apply(GuiShellAction::BeginSelectedPublicServerConnect)
        {
            return Err(
                "detached semantic connect flow could not stage the selected public-server connect"
                    .to_owned(),
            );
        }
        connect_handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
            GuiPendingCompletionRequest::from_state(&connect_state)
                .expect("staged semantic connect should capture its submitted public server"),
        ));
        connect_owner.pump_compatibility_state(&connect_handle, &connect_state);
        let connect_actions = connect_handle.drain_actions();
        if !connect_actions
            .iter()
            .any(|action| matches!(action, GuiShellAction::CompleteSelectedPublicServerConnect))
        {
            return Err(
                "detached semantic connect flow did not complete through the runtime owner"
                    .to_owned(),
            );
        }
        for action in connect_actions {
            if !connect_state.apply(action) {
                return Err(
                    "detached semantic connect flow rejected a runtime completion action"
                        .to_owned(),
                );
            }
        }
        let hello_line = recv_from_channel_while_pumping_runtime(
            &mut connect_owner,
            &connect_handle,
            &mut connect_state,
            &hello_rx,
            Duration::from_secs(1),
            "detached semantic connect flow GUI hello",
        )
        .map_err(|error| {
            format!("detached semantic connect flow did not observe a GUI hello: {error}")
        })?;
        if !hello_line.contains("\"semantic-user\"") || !hello_line.contains("\"semantic-room\"") {
            return Err(format!(
                "detached semantic connect flow emitted an unexpected hello payload: {hello_line:?}"
            ));
        }

        connect_owner.pump_compatibility_state(&connect_handle, &connect_state);
        for action in connect_handle.drain_actions() {
            if !connect_state.apply(action) {
                return Err(
                    "detached semantic connect flow rejected a projected session action".to_owned(),
                );
            }
        }

        let mut refresh_owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        let refresh_handle = GuiQueuedRuntimeBridgeHandle::default();
        let mut refresh_state =
            SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
                public_servers: Some(vec![
                    (" Primary ".to_owned(), " syncplay.pl:8999 ".to_owned()),
                    ("Duplicate".to_owned(), "SYNCPLAY.PL:8999".to_owned()),
                    ("Backup".to_owned(), "backup.example:9000".to_owned()),
                ]),
                ..StoredClientSettingsMvp::default()
            });
        if !refresh_state.apply(GuiShellAction::BeginPublicServerRefresh) {
            return Err("detached semantic refresh flow could not begin refresh".to_owned());
        }
        let requested_servers = refresh_state
            .public_servers
            .servers
            .iter()
            .map(|row| (row.label.clone(), row.address.clone()))
            .collect();
        let fetch_calls = std::cell::Cell::new(0_u8);
        let mut projected_refresh_state = refresh_state.clone();
        if !refresh_owner.handle_complete_public_server_refresh_request_with_fetcher(
            &refresh_handle,
            &mut projected_refresh_state,
            requested_servers,
            |_language| {
                fetch_calls.set(fetch_calls.get() + 1);
                Ok(vec![
                    (
                        "Remote Primary".to_owned(),
                        "remote.example:8999".to_owned(),
                    ),
                    (
                        "Remote Backup".to_owned(),
                        "remote-backup.example:9000".to_owned(),
                    ),
                ])
            },
        ) {
            return Err("detached semantic refresh flow rejected the fetch".to_owned());
        }
        if fetch_calls.get() != 1 {
            return Err(
                "detached semantic refresh flow did not invoke its fetcher once".to_owned(),
            );
        }
        let refresh_actions = refresh_handle.drain_actions();
        if !refresh_actions.iter().any(|action| {
            matches!(
                action,
                GuiShellAction::CompletePublicServerRefresh(servers)
                    if servers
                        == &vec![
                            ("Remote Primary".to_owned(), "remote.example:8999".to_owned()),
                            (
                                "Remote Backup".to_owned(),
                                "remote-backup.example:9000".to_owned(),
                            ),
                        ]
            )
        }) {
            return Err(
                "detached semantic refresh flow did not apply the fetched public-server replacement"
                    .to_owned(),
            );
        }
        for action in refresh_actions {
            if !refresh_state.apply(action) {
                return Err(
                    "detached semantic refresh flow rejected a refresh completion action"
                        .to_owned(),
                );
            }
        }

        let mut search_owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
        let search_handle = GuiQueuedRuntimeBridgeHandle::default();
        let mut search_state =
            SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
                media_search_directories: Some(vec![root.display().to_string()]),
                shared_playlist_enabled: Some(true),
                ..StoredClientSettingsMvp::default()
            });
        if !search_state.apply(GuiShellAction::SwitchView(GuiShellView::Setup))
            || !search_state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
                "missing-target.mkv".to_owned(),
            ]))
            || !search_state.apply(GuiShellAction::BeginMissingMediaSearch)
        {
            return Err("detached semantic search flow could not stage the search".to_owned());
        }
        search_handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
            GuiPendingCompletionRequest::SearchMissingMedia,
        ));
        search_owner.pump_compatibility_state(&search_handle, &search_state);
        let search_actions = search_handle.drain_actions();
        if !search_actions.iter().any(|action| {
            matches!(
                action,
                GuiShellAction::CompleteMissingMediaSearch(Some(path))
                    if path == &found_path_text
            )
        }) {
            return Err(format!(
                "detached semantic search flow did not emit the completed missing-media action: {search_actions:?}"
            ));
        }
        if search_actions.iter().any(|action| {
            !matches!(
                action,
                GuiShellAction::CompleteMissingMediaSearch(Some(path))
                    if path == &found_path_text
            ) && !matches!(action, GuiShellAction::ApplyGuiMediaIndexRuntimeSnapshot(_))
                && !matches!(action, GuiShellAction::ApplyGuiMediaMatchRuntimeSnapshot(_))
                && !matches!(action, GuiShellAction::ApplyMainWindowRuntimeSnapshot(_))
                && !matches!(action, GuiShellAction::ApplyMenuDialogRuntimeSnapshot(_))
                && !matches!(action, GuiShellAction::ApplyGuiCommandRuntimeSnapshot(_))
        }) {
            return Err(format!(
                "detached semantic search flow returned unexpected actions: {search_actions:?}"
            ));
        }
        for action in search_actions {
            if !search_state.apply(action) {
                return Err(
                    "detached semantic search flow rejected a search completion action".to_owned(),
                );
            }
        }
        let expected_search_message = format!("Missing media found: {found_path_text}.");
        if search_state
            .notifications
            .iter()
            .any(|notification| notification.message == expected_search_message)
        {
            return Err(
                "detached semantic search flow unexpectedly surfaced a success notification"
                    .to_owned(),
            );
        }

        Ok(report_from_state(
            "detached-runtime-ownership-flow",
            &search_state,
        ))
    })();

    let _ = release_tx.send(());
    let server_result = server_thread
        .join()
        .map_err(|_| "detached semantic TCP server thread panicked".to_owned())?;
    let _ = std::fs::remove_dir_all(&root);
    match (result, server_result) {
        (Ok(report), Ok(())) => Ok(report),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(server_error)) => Err(format!("{error}; {server_error}")),
    }
}

pub(super) fn run_gui_semantic_live_python_peer_connect_flow()
-> Result<GuiSemanticScenarioReport, String> {
    let result = super::super::live_python_interop::run_live_python_peer_connect_flow()
        .map_err(|error| error.to_string())?;
    Ok(GuiSemanticScenarioReport {
        scenario: "live-python-peer-connect-flow".to_owned(),
        view: "room".to_owned(),
        modal: "none".to_owned(),
        pending: "none".to_owned(),
        widgets: result.widget_count,
    })
}

pub(super) fn run_gui_semantic_live_python_peer_controlled_room_flow()
-> Result<GuiSemanticScenarioReport, String> {
    let result = super::super::live_python_interop::run_live_python_peer_controlled_room_flow()
        .map_err(|error| error.to_string())?;
    Ok(GuiSemanticScenarioReport {
        scenario: "live-python-peer-controlled-room-flow".to_owned(),
        view: "room".to_owned(),
        modal: "none".to_owned(),
        pending: "none".to_owned(),
        widgets: result.widget_count,
    })
}
