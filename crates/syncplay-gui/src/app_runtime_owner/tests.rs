use std::{
    io::{BufRead, Write},
    path::PathBuf,
};

use super::super::runtime_stack::GuiSessionRoomPlaystate;
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
    GuiLaunchMode, GuiMediaIndexJobId, GuiOwnedPlayer, GuiPendingAttachedMediaResolution,
    GuiPendingCompletionRequest, GuiPendingOperationKind, GuiPendingRoomChangeRequest,
    GuiPersistedUiState, GuiPlayerLaunchRuntimeState, GuiQueuedRuntimeBridgeHandle,
    GuiQueuedRuntimeOwner, GuiRuntimeRequest, GuiSessionRuntimeAdapter, GuiShellAction,
    GuiShellView, GuiTestPlayerAdapter, GuiTransientNotificationLevel, MainWindowPlaylistRow,
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

#[path = "tests/connection_runtime_tests.rs"]
mod connection_runtime_tests;
#[path = "tests/persistence_tests.rs"]
mod persistence_tests;
#[path = "tests/player_runtime_tests.rs"]
mod player_runtime_tests;
#[path = "tests/playlist_runtime_tests.rs"]
mod playlist_runtime_tests;
#[path = "tests/session_runtime_tests.rs"]
mod session_runtime_tests;
#[path = "tests/startup_tests.rs"]
mod startup_tests;
#[path = "tests/transport_tests.rs"]
mod transport_tests;
