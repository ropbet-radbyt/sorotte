use std::{
    fmt, thread,
    time::{Duration, Instant},
};

use syncplay_compat::{InteropError, LegacyServerPythonPeerHarness};

use super::{
    GuiPersistedConfigRuntimeOwner, GuiQueuedRuntimeBridgeHandle, GuiQueuedRuntimeOwner,
    GuiShellAction, GuiShellView, StoredClientSettingsMvp, SyncplayGuiShellAppState,
};

pub(crate) const LIVE_PYTHON_INTEROP_LOCAL_USERNAME: &str = "interop-gui-user";
pub(crate) const LIVE_PYTHON_INTEROP_PEER_USERNAME: &str = "interop-py-peer";
pub(crate) const LIVE_PYTHON_INTEROP_ROOM: &str = "interop-room";
const LIVE_PYTHON_INTEROP_TIMEOUT: Duration = Duration::from_secs(6);
const LIVE_PYTHON_INTEROP_POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LivePythonPeerInteropResult {
    pub room_name: String,
    pub local_user_present: bool,
    pub peer_user_present: bool,
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
        GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
        for action in handle.drain_actions() {
            state.apply(action);
        }
        thread::sleep(LIVE_PYTHON_INTEROP_POLL_INTERVAL);
    }
    harness.start_peer_connected()?;

    let deadline = Instant::now() + LIVE_PYTHON_INTEROP_TIMEOUT;
    loop {
        GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &state);
        for action in handle.drain_actions() {
            state.apply(action);
        }

        let local_user_present = state
            .main_window
            .users
            .iter()
            .any(|user| user.username == LIVE_PYTHON_INTEROP_LOCAL_USERNAME && user.is_self);
        let peer_user_present = state
            .main_window
            .users
            .iter()
            .any(|user| user.username == harness.peer_username() && !user.is_self);
        if state.main_window.room_name == harness.room() && local_user_present && peer_user_present
        {
            state.apply(GuiShellAction::SwitchView(GuiShellView::MainWindow));
            return Ok(LivePythonPeerInteropResult {
                room_name: state.main_window.room_name.clone(),
                local_user_present,
                peer_user_present,
                widget_count: state.shell_widget_tree().node_count(),
            });
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
                "timed out waiting for live Python peer interop projection; room={:?}, users=[{}]",
                state.main_window.room_name, projected_users
            )));
        }

        thread::sleep(LIVE_PYTHON_INTEROP_POLL_INTERVAL);
    }
}
