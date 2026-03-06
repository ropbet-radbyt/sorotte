use std::{
    fmt, thread,
    time::{Duration, Instant},
};

use syncplay_compat::{
    InteropError, LegacyPythonPeerChatMessage, LegacyPythonPeerSnapshot,
    LegacyServerPythonPeerHarness,
};

use super::{
    GuiPendingCompletionRequest, GuiPersistedConfigRuntimeOwner, GuiQueuedRuntimeBridgeHandle,
    GuiQueuedRuntimeOwner, GuiRuntimeRequest, GuiShellAction, GuiShellView,
    StoredClientSettingsMvp, SyncplayGuiShellAppState,
};

pub(crate) const LIVE_PYTHON_INTEROP_LOCAL_USERNAME: &str = "interop-gui-user";
pub(crate) const LIVE_PYTHON_INTEROP_PEER_USERNAME: &str = "interop-py-peer";
pub(crate) const LIVE_PYTHON_INTEROP_ROOM: &str = "interop-room";
pub(crate) const LIVE_PYTHON_INTEROP_LOCAL_CHAT_MESSAGE: &str = "hello from gui";
pub(crate) const LIVE_PYTHON_INTEROP_PEER_CHAT_MESSAGE: &str = "hello from python";
const LIVE_PYTHON_INTEROP_TIMEOUT: Duration = Duration::from_secs(6);
const LIVE_PYTHON_INTEROP_POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LivePythonPeerInteropResult {
    pub room_name: String,
    pub local_user_present: bool,
    pub peer_user_present: bool,
    pub local_user_ready: bool,
    pub peer_user_ready: bool,
    pub gui_chat_messages: Vec<LegacyPythonPeerChatMessage>,
    pub peer_chat_messages: Vec<LegacyPythonPeerChatMessage>,
    pub widget_count: usize,
}

#[derive(Debug)]
pub(crate) enum LivePythonPeerInteropError {
    Interop(InteropError),
    Gui(String),
}

impl fmt::Display for LivePythonPeerInteropError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Interop(error) => write!(f, "{error}"),
            Self::Gui(error) => f.write_str(error),
        }
    }
}

impl From<InteropError> for LivePythonPeerInteropError {
    fn from(error: InteropError) -> Self {
        Self::Interop(error)
    }
}

#[allow(dead_code)]
pub(crate) fn live_python_interop_prerequisites_missing(
    error: &LivePythonPeerInteropError,
) -> bool {
    match error {
        LivePythonPeerInteropError::Interop(error) => {
            syncplay_compat::interop_prerequisites_missing(error)
        }
        LivePythonPeerInteropError::Gui(_) => false,
    }
}

pub(crate) fn run_live_python_peer_connect_flow()
-> Result<LivePythonPeerInteropResult, LivePythonPeerInteropError> {
    let mut harness = LegacyServerPythonPeerHarness::spawn(
        LIVE_PYTHON_INTEROP_PEER_USERNAME,
        LIVE_PYTHON_INTEROP_ROOM,
    )?;
    let outcome = run_live_python_peer_connect_flow_with_harness(&mut harness);
    let shutdown_result = harness.shutdown().map_err(LivePythonPeerInteropError::from);
    match (outcome, shutdown_result) {
        (Ok(result), Ok(())) => Ok(result),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(_shutdown_error)) => Err(error),
    }
}

fn run_live_python_peer_connect_flow_with_harness(
    harness: &mut LegacyServerPythonPeerHarness,
) -> Result<LivePythonPeerInteropResult, LivePythonPeerInteropError> {
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None)
        .with_client_core_chat_tcp_session_runtime(
            LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
            LIVE_PYTHON_INTEROP_ROOM,
            harness.address(),
        )
        .map_err(LivePythonPeerInteropError::Gui)?;
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        username: Some(LIVE_PYTHON_INTEROP_LOCAL_USERNAME.to_owned()),
        room: Some(LIVE_PYTHON_INTEROP_ROOM.to_owned()),
        chat_input_enabled: Some(true),
        chat_output_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    let startup_deadline = Instant::now() + Duration::from_millis(600);
    while Instant::now() < startup_deadline {
        pump_and_apply(&mut owner, &handle, &mut state);
        thread::sleep(LIVE_PYTHON_INTEROP_POLL_INTERVAL);
    }
    harness.start_peer_connected()?;
    wait_for_projection(&mut owner, &handle, &mut state, false, false)?;
    wait_for_peer_observed_user_presence(
        harness,
        LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
        Duration::from_secs(3),
    )?;

    request_local_ready(&handle, &mut state, true)?;
    wait_for_projection(&mut owner, &handle, &mut state, true, false)?;
    wait_for_peer_observed_user_ready(
        harness,
        LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
        true,
        Duration::from_secs(3),
    )?;

    request_local_ready(&handle, &mut state, false)?;
    wait_for_projection(&mut owner, &handle, &mut state, false, false)?;
    wait_for_peer_observed_user_ready(
        harness,
        LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
        false,
        Duration::from_secs(3),
    )?;

    harness.set_peer_ready(true)?;
    let _ = harness.wait_for_peer_local_ready(true, Duration::from_secs(3))?;
    wait_for_projection(&mut owner, &handle, &mut state, false, true)?;

    harness.set_peer_ready(false)?;
    let _ = harness.wait_for_peer_local_ready(false, Duration::from_secs(3))?;
    wait_for_projection(&mut owner, &handle, &mut state, false, false)?;

    request_local_chat_send(&handle, &mut state, LIVE_PYTHON_INTEROP_LOCAL_CHAT_MESSAGE)?;
    wait_for_projected_chat_message(
        &mut owner,
        &handle,
        &mut state,
        LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
        LIVE_PYTHON_INTEROP_LOCAL_CHAT_MESSAGE,
    )?;
    wait_for_peer_observed_chat_message(
        harness,
        LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
        LIVE_PYTHON_INTEROP_LOCAL_CHAT_MESSAGE,
        Duration::from_secs(3),
    )?;

    harness.send_peer_chat_message(LIVE_PYTHON_INTEROP_PEER_CHAT_MESSAGE)?;
    wait_for_peer_observed_chat_message(
        harness,
        LIVE_PYTHON_INTEROP_PEER_USERNAME,
        LIVE_PYTHON_INTEROP_PEER_CHAT_MESSAGE,
        Duration::from_secs(3),
    )?;
    wait_for_projected_chat_message(
        &mut owner,
        &handle,
        &mut state,
        LIVE_PYTHON_INTEROP_PEER_USERNAME,
        LIVE_PYTHON_INTEROP_PEER_CHAT_MESSAGE,
    )?;

    let peer_snapshot = harness.peer_snapshot()?;
    state.apply(GuiShellAction::SwitchView(GuiShellView::MainWindow));
    Ok(LivePythonPeerInteropResult {
        room_name: state.main_window.room_name.clone(),
        local_user_present: local_user_ready(&state).is_some(),
        peer_user_present: peer_user_ready(&state, harness.peer_username()).is_some(),
        local_user_ready: local_user_ready(&state).unwrap_or(false),
        peer_user_ready: peer_user_ready(&state, harness.peer_username()).unwrap_or(false),
        gui_chat_messages: state
            .main_window
            .chat
            .iter()
            .map(|row| LegacyPythonPeerChatMessage {
                sender: row.sender.clone(),
                message: row.message.clone(),
            })
            .collect(),
        peer_chat_messages: peer_snapshot.chat_messages,
        widget_count: state.shell_widget_tree().node_count(),
    })
}

fn pump_and_apply(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
) {
    GuiQueuedRuntimeOwner::pump(owner, handle, state);
    for action in handle.drain_actions() {
        state.apply(action);
    }
}

fn request_local_ready(
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
    ready: bool,
) -> Result<(), LivePythonPeerInteropError> {
    let action = if ready {
        GuiShellAction::AnnounceLocalUserReady
    } else {
        GuiShellAction::AnnounceLocalUserNotReady
    };
    if !state.apply(action) {
        return Err(LivePythonPeerInteropError::Gui(format!(
            "failed to apply local readiness action; room={:?}",
            state.main_window.room_name
        )));
    }
    handle.push_request(GuiRuntimeRequest::SetLocalReady(ready));
    Ok(())
}

fn request_local_chat_send(
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
    message: &str,
) -> Result<(), LivePythonPeerInteropError> {
    if !state.apply(GuiShellAction::BeginLocalChatSend(message.to_owned())) {
        return Err(LivePythonPeerInteropError::Gui(format!(
            "failed to stage local chat send {message:?}; room={:?}",
            state.main_window.room_name
        )));
    }
    handle.push_request(GuiRuntimeRequest::CompletePendingOperation(
        GuiPendingCompletionRequest::SendChatMessage(message.to_owned()),
    ));
    Ok(())
}

fn wait_for_peer_observed_user_ready(
    harness: &mut LegacyServerPythonPeerHarness,
    username: &str,
    ready: bool,
    timeout: Duration,
) -> Result<LegacyPythonPeerSnapshot, LivePythonPeerInteropError> {
    harness
        .wait_for_peer_observed_user_ready(username, ready, timeout)
        .map_err(LivePythonPeerInteropError::from)
}

fn wait_for_peer_observed_chat_message(
    harness: &mut LegacyServerPythonPeerHarness,
    username: &str,
    message: &str,
    timeout: Duration,
) -> Result<LegacyPythonPeerSnapshot, LivePythonPeerInteropError> {
    harness
        .wait_for_peer_observed_chat_message(username, message, timeout)
        .map_err(LivePythonPeerInteropError::from)
}

fn wait_for_peer_observed_user_presence(
    harness: &mut LegacyServerPythonPeerHarness,
    username: &str,
    timeout: Duration,
) -> Result<LegacyPythonPeerSnapshot, LivePythonPeerInteropError> {
    let deadline = Instant::now() + timeout;
    loop {
        let snapshot = harness.peer_snapshot()?;
        if snapshot.observed_users.contains_key(username) {
            return Ok(snapshot);
        }
        if Instant::now() >= deadline {
            return Err(LivePythonPeerInteropError::Gui(format!(
                "timed out waiting for Python reference peer to observe GUI user {username:?}; users={:?}",
                snapshot.observed_users
            )));
        }
        thread::sleep(LIVE_PYTHON_INTEROP_POLL_INTERVAL);
    }
}

fn wait_for_projected_chat_message(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
    sender: &str,
    message: &str,
) -> Result<(), LivePythonPeerInteropError> {
    let deadline = Instant::now() + LIVE_PYTHON_INTEROP_TIMEOUT;
    loop {
        pump_and_apply(owner, handle, state);
        if state
            .main_window
            .chat
            .iter()
            .any(|row| row.sender == sender && row.message == message)
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let projected_chat = state
                .main_window
                .chat
                .iter()
                .map(|row| format!("{}>{}", row.sender, row.message))
                .collect::<Vec<_>>()
                .join(" | ");
            return Err(LivePythonPeerInteropError::Gui(format!(
                "timed out waiting for live Python peer chat projection {sender:?}>{message:?}; room={:?}, chat=[{}]",
                state.main_window.room_name, projected_chat
            )));
        }
        thread::sleep(LIVE_PYTHON_INTEROP_POLL_INTERVAL);
    }
}

fn wait_for_projection(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
    expected_local_ready: bool,
    expected_peer_ready: bool,
) -> Result<(), LivePythonPeerInteropError> {
    let deadline = Instant::now() + LIVE_PYTHON_INTEROP_TIMEOUT;
    loop {
        pump_and_apply(owner, handle, state);

        let local_ready_matches = local_user_ready(state) == Some(expected_local_ready);
        let peer_ready_matches =
            peer_user_ready(state, LIVE_PYTHON_INTEROP_PEER_USERNAME) == Some(expected_peer_ready);
        let local_user_present = local_user_ready(state).is_some();
        let peer_user_present = peer_user_ready(state, LIVE_PYTHON_INTEROP_PEER_USERNAME).is_some();
        if state.main_window.room_name == LIVE_PYTHON_INTEROP_ROOM
            && local_user_present
            && peer_user_present
            && local_ready_matches
            && peer_ready_matches
        {
            return Ok(());
        }

        if Instant::now() >= deadline {
            let projected_users = state
                .main_window
                .users
                .iter()
                .map(|user| {
                    format!(
                        "{}(self={}, ready={}, controller={})",
                        user.username, user.is_self, user.is_ready, user.is_controller
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            return Err(LivePythonPeerInteropError::Gui(format!(
                "timed out waiting for live Python peer readiness projection; expected_local_ready={expected_local_ready}, expected_peer_ready={expected_peer_ready}, room={:?}, users=[{}]",
                state.main_window.room_name, projected_users
            )));
        }

        thread::sleep(LIVE_PYTHON_INTEROP_POLL_INTERVAL);
    }
}

fn local_user_ready(state: &SyncplayGuiShellAppState) -> Option<bool> {
    state
        .main_window
        .users
        .iter()
        .find(|user| user.username == LIVE_PYTHON_INTEROP_LOCAL_USERNAME && user.is_self)
        .map(|user| user.is_ready)
}

fn peer_user_ready(state: &SyncplayGuiShellAppState, username: &str) -> Option<bool> {
    state
        .main_window
        .users
        .iter()
        .find(|user| user.username == username && !user.is_self)
        .map(|user| user.is_ready)
}
