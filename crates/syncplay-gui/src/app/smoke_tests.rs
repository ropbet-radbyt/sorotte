use std::io::{BufRead, Write};

use super::testing::support::{
    pump_and_apply_runtime_owner_actions, pump_and_apply_runtime_owner_actions_until,
};
use syncplay_client_app::app_boundary::{
    persistence::load_syncplay_ini_stored_client_settings_mvp_from_path,
    state::{AutoplayThresholdOverride, StoredClientSettingsMvp},
};
use syncplay_client_core::{PrivacyMode, UnpauseActionMode};

#[cfg(feature = "live-python-interop")]
use super::live_python_interop;
use super::runtime_bridge::{
    GuiPendingCompletionRequest, GuiQueuedRuntimeOwner, GuiRuntimeRequest,
};
use super::runtime_owner::GuiPersistedConfigRuntimeOwner;
use super::runtime_queue::GuiQueuedRuntimeBridgeHandle;
use super::shell_state::{
    GuiPendingOperationKind, GuiShellAction, GuiShellView, SyncplayGuiShellAppState,
};
use super::{GuiPreviewRuntimeBridge, upsert_syncplay_ini_stored_client_settings_mvp_at_path};

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

fn read_protocol_line_matching<R, F>(reader: &mut R, mut predicate: F, context: &str) -> String
where
    R: BufRead,
    F: FnMut(&str) -> bool,
{
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).unwrap_or_else(|error| {
            panic!("{context} should read the next protocol line: {error}")
        });
        assert!(
            read > 0,
            "{context} should not hit EOF before the expected line"
        );
        if predicate(&line) {
            return line;
        }
    }
}

#[cfg(feature = "live-python-interop")]
mod live_python_smoke;
#[cfg(windows)]
mod managed_mpv_smoke;
mod player_setup_smoke;
mod portable_persistence_transport_smoke;
mod portable_script_parity_smoke;
mod portable_tcp_reconnect_smoke;
