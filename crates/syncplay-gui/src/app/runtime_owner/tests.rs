use std::{
    io::{BufRead, Write},
    path::PathBuf,
};

use super::super::runtime_stack::{GuiAttachedPlayerRuntimeAction, GuiSessionRoomPlaystate};
use super::GuiPersistedConfigRuntimeOwner;

use crate::app::testing::support::{
    browser_runtime_rooms, browser_runtime_user, pump_and_apply_runtime_owner_actions,
    pump_and_apply_runtime_owner_actions_until, test_temp_root,
};
use crate::app::{
    GuiAttachedMediaSearchBuildProgress, GuiAttachedMediaSearchBuildState,
    GuiAttachedMediaSearchBuildStatus, GuiAttachedMediaSearchIndex,
    GuiAttachedMediaSearchRootIndex, GuiAttachedMediaSearchRootRefreshResult,
    GuiCommandAvailabilityState, GuiCommandRuntimeSnapshot, GuiInteractionRuntimeSnapshot,
    GuiLaunchMode, GuiOwnedPlayer, GuiPendingAttachedMediaResolution, GuiPendingCompletionRequest,
    GuiPendingOperationKind, GuiPendingRoomChangeRequest, GuiPersistedUiState,
    GuiPlayerLaunchRuntimeState, GuiQueuedRuntimeBridgeHandle, GuiQueuedRuntimeOwner,
    GuiRuntimeRequest, GuiSessionRuntimeAdapter, GuiShellAction, GuiShellView,
    GuiTestPlayerAdapter, GuiTransientNotificationLevel, MainWindowPlaylistRow,
    MainWindowRuntimeChatSnapshot, MainWindowRuntimeSnapshot, MenuActionRuntimeOverride,
    MenuDialogRuntimeSnapshot, SyncplayGuiRuntimeSnapshot, SyncplayGuiShellAppState,
    legacy_gui_qsettings_store_path, persist_gui_ui_state_at_root,
};
use syncplay_client_app::app_boundary::persistence::{
    load_syncplay_ini_stored_client_settings_mvp_from_path,
    upsert_syncplay_ini_stored_client_settings_mvp_at_path,
};
use syncplay_client_app::app_boundary::state::StoredClientSettingsMvp;
use syncplay_player_api::PlayerAdapter;

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
    state: &mut SyncplayGuiShellAppState,
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
