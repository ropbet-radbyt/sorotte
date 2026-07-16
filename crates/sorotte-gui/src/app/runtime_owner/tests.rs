use std::{
    io::{BufRead, Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

use super::super::runtime_stack::{GuiAttachedPlayerRuntimeAction, GuiSessionRoomPlaystate};
use super::{GuiMediaMatchBackgroundCancelDisposition, GuiPersistedConfigRuntimeOwner};

use crate::app::testing::support::{
    browser_runtime_rooms, browser_runtime_user, pump_and_apply_runtime_owner_actions,
    pump_and_apply_runtime_owner_actions_until, test_temp_root,
};
use crate::app::{
    GuiAttachedMediaSearchBuildProgress, GuiAttachedMediaSearchBuildState,
    GuiAttachedMediaSearchBuildStatus, GuiAttachedMediaSearchIndex,
    GuiAttachedMediaSearchRootIndex, GuiAttachedMediaSearchRootRefreshResult,
    GuiCommandAvailabilityState, GuiCommandRuntimeSnapshot, GuiConfigStorageChangeTarget,
    GuiConfigStorageRuntimeSnapshot, GuiInteractionRuntimeSnapshot, GuiLaunchMode, GuiOwnedPlayer,
    GuiPendingAttachedMediaResolution, GuiPendingCompletionRequest, GuiPendingOperationKind,
    GuiPendingRoomChangeRequest, GuiPersistedUiState, GuiPlayerLaunchRuntimeState,
    GuiPluginSelection, GuiQueuedRuntimeBridgeHandle, GuiQueuedRuntimeOwner, GuiRuntimeRequest,
    GuiSavedServerConnectIntent, GuiSessionRuntimeAdapter, GuiShellAction, GuiShellView,
    GuiTestPlayerAdapter, GuiTransientNotificationLevel, MainWindowPlaylistRow,
    MainWindowRuntimeChatSnapshot, MainWindowRuntimeSnapshot, MenuActionId,
    MenuActionRuntimeOverride, MenuDialogRuntimeSnapshot, SettingId, SorotteGuiRuntimeSnapshot,
    SorotteGuiShellAppState, legacy_gui_qsettings_store_path, persist_gui_ui_state_at_root,
};
use sorotte_client_app::app_boundary::persistence::{
    load_sorotte_ini_stored_client_settings_mvp_from_path,
    upsert_sorotte_ini_stored_client_settings_mvp_at_path,
};
use sorotte_client_app::app_boundary::state::StoredClientSettingsMvp;
use sorotte_client_app::app_boundary::storage::{
    SOROTTE_CLIENT_INSTALL_ROOT_ENV, parse_sorotte_client_install_locator_config_root,
    sorotte_client_install_locator_path,
};
use sorotte_player_api::PlayerAdapter;

static CONFIG_ROOT_ENV_LOCK: Mutex<()> = Mutex::new(());

struct TestEnvGuard<'a> {
    _guard: MutexGuard<'a, ()>,
}

impl<'a> TestEnvGuard<'a> {
    fn lock(lock: &'a Mutex<()>) -> Self {
        Self {
            _guard: lock.lock().expect("lock poisoned"),
        }
    }

    fn set_var<K, V>(&self, key: K, value: V)
    where
        K: AsRef<std::ffi::OsStr>,
        V: AsRef<std::ffi::OsStr>,
    {
        // SAFETY: This guard serializes runtime-owner tests that mutate config-root env vars.
        unsafe {
            std::env::set_var(key, value);
        }
    }

    fn remove_var<K>(&self, key: K)
    where
        K: AsRef<std::ffi::OsStr>,
    {
        // SAFETY: See set_var; the same guard serializes test-owned removals.
        unsafe {
            std::env::remove_var(key);
        }
    }
}

fn serve_runtime_plex_json_responses(responses: Vec<String>) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("Plex test listener should bind");
    let address = listener
        .local_addr()
        .expect("Plex test listener should expose its address");
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for body in responses {
            let (mut stream, _) = listener.accept().expect("Plex test server should accept");
            let mut buffer = [0_u8; 8192];
            let read = stream
                .read(&mut buffer)
                .expect("Plex test server should read request");
            let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
            tx.send(request)
                .expect("Plex test server should send captured request");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("Plex test server should write response");
        }
    });
    (format!("http://{address}"), rx)
}

fn read_client_hello_after_optional_start_tls<R, W>(
    reader: &mut R,
    writer: &mut W,
    context: &str,
) -> String
where
    R: BufRead,
    W: Write,
{
    let mut first_line = String::new();
    reader.read_line(&mut first_line).unwrap_or_else(|error| {
        panic!("{context} should read the first client protocol line: {error}")
    });
    if first_line.contains("\"TLS\"") {
        writer
            .write_all(br#"{"TLS":{"startTLS":"false"}}"#)
            .unwrap_or_else(|error| {
                panic!("{context} should decline the client startTLS request: {error}")
            });
        writer
            .write_all(b"\n")
            .unwrap_or_else(|error| panic!("{context} should terminate the TLS response: {error}"));
        writer
            .flush()
            .unwrap_or_else(|error| panic!("{context} should flush the TLS response: {error}"));

        let mut hello_line = String::new();
        reader.read_line(&mut hello_line).unwrap_or_else(|error| {
            panic!("{context} should read the client hello after declining TLS: {error}")
        });
        hello_line
    } else {
        first_line
    }
}

fn is_default_ready_publish_line(line: &str) -> bool {
    let Ok(message) = serde_json::from_str::<serde_json::Value>(line) else {
        return false;
    };
    let Some(ready) = message.get("Set").and_then(|set| set.get("ready")) else {
        return false;
    };
    ready.get("isReady").and_then(serde_json::Value::as_bool) == Some(false)
        && ready
            .get("manuallyInitiated")
            .and_then(serde_json::Value::as_bool)
            == Some(false)
}

fn without_default_ready_publish_lines(lines: Vec<String>) -> Vec<String> {
    lines
        .into_iter()
        .filter(|line| !is_default_ready_publish_line(line))
        .collect()
}

#[test]
fn runtime_owner_searches_and_resolves_plex_playlist_picker_items() {
    let (server_url, rx) = serve_runtime_plex_json_responses(vec![
        serde_json::json!({
            "MediaContainer": {
                "Directory": [
                    { "key": "1", "type": "show", "title": "Anime" }
                ]
            }
        })
        .to_string(),
        serde_json::json!({
            "MediaContainer": {
                "Metadata": [{
                    "ratingKey": "14452",
                    "type": "episode",
                    "title": "Episode 11",
                    "parentTitle": "Season 4",
                    "grandparentTitle": "Re:Zero",
                    "duration": 1470058,
                    "Media": [{
                        "Part": [{
                            "file": "E:/Anime/Re Zero/Episode 11.mkv"
                        }]
                    }]
                }]
            }
        })
        .to_string(),
        serde_json::json!({
            "MediaContainer": {
                "Metadata": []
            }
        })
        .to_string(),
        serde_json::json!({
            "MediaContainer": {
                "Metadata": []
            }
        })
        .to_string(),
        serde_json::json!({
            "MediaContainer": {
                "machineIdentifier": "machine-from-root"
            }
        })
        .to_string(),
        serde_json::json!({
            "MediaContainer": {
                "Metadata": [{
                    "ratingKey": "14452",
                    "type": "episode",
                    "title": "Episode 11",
                    "duration": 1470058,
                    "Media": [{
                        "Part": [{
                            "id": "part-1",
                            "key": "/library/parts/1/file.mkv",
                            "file": "E:/Anime/Re Zero/Episode 11.mkv",
                            "duration": 1470058,
                            "size": 458900243
                        }]
                    }]
                }]
            }
        })
        .to_string(),
    ]);
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path_and_startup_player(None);
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        plex_user_token: Some("user-token".into()),
        plex_selected_server_url: Some(server_url),
        plex_selected_server_token: Some("server-token".into()),
        ..StoredClientSettingsMvp::default()
    });
    state.main_window.playback.can_manage_playlist = true;
    assert!(state.apply(GuiShellAction::BeginPlexPlaylistSearch));
    assert!(state.apply(GuiShellAction::SubmitPlexPlaylistSearch {
        query: "zero".to_owned(),
    }));

    handle.push_request(GuiRuntimeRequest::SearchSelectedPlexServerMedia {
        query: "zero".to_owned(),
    });
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(3),
        |state| {
            state
                .plex_playlist_search
                .as_ref()
                .is_some_and(|search| !search.searching && !search.results.is_empty())
        },
        "Plex picker search result",
    );

    let search = state
        .plex_playlist_search
        .as_ref()
        .expect("Plex picker should remain open");
    assert_eq!(search.results[0].rating_key, "14452");
    assert_eq!(
        search.results[0].grandparent_title.as_deref(),
        Some("Re:Zero")
    );
    assert_eq!(state.plex.status, "ready");
    assert_eq!(state.plex.last_error, None);

    assert!(state.apply(GuiShellAction::AddSelectedPlexPlaylistSearchResult));
    handle.push_request(GuiRuntimeRequest::ResolvePlexPlaylistItem {
        rating_key: "14452".to_owned(),
    });
    pump_and_apply_runtime_owner_actions_until(
        &mut owner,
        &handle,
        &mut state,
        std::time::Duration::from_secs(3),
        |state| {
            state
                .current_shared_playlist_entries()
                .iter()
                .any(|entry| entry.starts_with("plex://machine-from-root/metadata/14452?"))
        },
        "Plex picker playlist append",
    );

    let entries = state.current_shared_playlist_entries();
    let plex_entry = entries
        .iter()
        .find(|entry| entry.starts_with("plex://machine-from-root/metadata/14452?"))
        .expect("Plex entry should be appended");
    assert!(plex_entry.contains("file=Episode%2011.mkv"));
    assert!(!plex_entry.to_ascii_lowercase().contains("token"));
    let queued_requests = handle.drain_requests();
    assert_eq!(
        queued_requests,
        vec![GuiRuntimeRequest::QueuePlaylistEntry {
            entry: plex_entry.clone(),
            select_after_queue: false,
        }]
    );
    assert!(
        state
            .plex_playlist_search
            .as_ref()
            .is_some_and(|search| search.adding_rating_key.is_none())
    );
    let requests = (0..6)
        .map(|_| {
            rx.recv_timeout(std::time::Duration::from_secs(2))
                .expect("Plex request should be captured")
        })
        .collect::<Vec<_>>();
    assert!(requests[0].starts_with("GET /library/sections HTTP/1.1"));
    assert!(requests[1].starts_with("GET /library/sections/1/all?"));
    assert!(requests[1].contains("title=zero"));
    assert!(requests[2].starts_with("GET /library/sections/1/all?"));
    assert!(requests[2].contains("show.title=zero"));
    assert!(requests[3].starts_with("GET /library/sections/1/all?"));
    assert!(requests[3].contains("file=zero"));
    assert!(requests[4].starts_with("GET / HTTP/1.1"));
    assert!(requests[5].starts_with("GET /library/metadata/14452 HTTP/1.1"));
}

fn read_next_non_default_ready_line<R>(reader: &mut R, context: &str) -> String
where
    R: BufRead,
{
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .unwrap_or_else(|error| panic!("{context} should read a protocol line: {error}"));
        if !is_default_ready_publish_line(&line) {
            return line;
        }
    }
}

fn recv_from_channel_while_pumping_runtime<T>(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SorotteGuiShellAppState,
    receiver: &std::sync::mpsc::Receiver<T>,
    timeout: std::time::Duration,
    context: &str,
) -> T {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        pump_and_apply_runtime_owner_actions(owner, handle, state);
        if let Ok(value) = receiver.try_recv() {
            return value;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {context}"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn runtime_chat_pane_ready(chat: &[MainWindowRuntimeChatSnapshot]) -> bool {
    chat == runtime_chat_pane_ready_rows()
}

fn runtime_chat_pane_ready_rows() -> Vec<MainWindowRuntimeChatSnapshot> {
    vec![MainWindowRuntimeChatSnapshot {
        sender: "system".to_owned(),
        message: "Chat pane ready".to_owned(),
    }]
}

mod connection_runtime_tests;
mod persistence_tests;
mod player_runtime_tests;
mod playlist_runtime_tests;
mod session_runtime_tests;
mod startup_tests;
mod transport_tests;
