use std::{
    thread,
    time::{Duration, Instant},
};

use sorotte_compat::{LegacyPythonPeerSnapshot, LegacyServerPythonPeerHarness};

use super::super::{
    GuiPersistedConfigRuntimeOwner, GuiQueuedRuntimeBridgeHandle, SorotteGuiShellAppState,
};
use super::projection::{
    gui_playlist, local_user_controller, local_user_ready, peer_user_controller, peer_user_ready,
};
use super::runtime_actions::pump_and_apply;
use super::{
    LIVE_PYTHON_INTEROP_CONTROLLED_ROOM, LIVE_PYTHON_INTEROP_PEER_USERNAME,
    LIVE_PYTHON_INTEROP_POLL_INTERVAL, LIVE_PYTHON_INTEROP_ROOM, LIVE_PYTHON_INTEROP_TIMEOUT,
    LivePythonPeerInteropError,
};

pub(in crate::app::live_python_interop) fn wait_for_peer_observed_user_ready(
    harness: &mut LegacyServerPythonPeerHarness,
    username: &str,
    ready: bool,
    timeout: Duration,
) -> Result<LegacyPythonPeerSnapshot, LivePythonPeerInteropError> {
    harness
        .wait_for_peer_observed_user_ready(username, ready, timeout)
        .map_err(LivePythonPeerInteropError::from)
}

pub(in crate::app::live_python_interop) fn wait_for_peer_observed_user_controller(
    harness: &mut LegacyServerPythonPeerHarness,
    username: &str,
    controller: bool,
    timeout: Duration,
) -> Result<LegacyPythonPeerSnapshot, LivePythonPeerInteropError> {
    harness
        .wait_for_peer_observed_user_controller(username, controller, timeout)
        .map_err(LivePythonPeerInteropError::from)
}

pub(in crate::app::live_python_interop) fn wait_for_peer_observed_chat_message(
    harness: &mut LegacyServerPythonPeerHarness,
    username: &str,
    message: &str,
    timeout: Duration,
) -> Result<LegacyPythonPeerSnapshot, LivePythonPeerInteropError> {
    harness
        .wait_for_peer_observed_chat_message(username, message, timeout)
        .map_err(LivePythonPeerInteropError::from)
}

#[cfg(test)]
pub(in crate::app::live_python_interop) fn wait_for_peer_observed_user_file_name(
    harness: &mut LegacyServerPythonPeerHarness,
    username: &str,
    file_name: &str,
    timeout: Duration,
) -> Result<LegacyPythonPeerSnapshot, LivePythonPeerInteropError> {
    harness
        .wait_for_peer_observed_user_file_name(username, file_name, timeout)
        .map_err(LivePythonPeerInteropError::from)
}

pub(in crate::app::live_python_interop) fn wait_for_peer_observed_playlist(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SorotteGuiShellAppState,
    harness: &mut LegacyServerPythonPeerHarness,
    playlist: &[String],
    timeout: Duration,
) -> Result<LegacyPythonPeerSnapshot, LivePythonPeerInteropError> {
    let deadline = Instant::now() + timeout;
    loop {
        // Production transports advance one receipt-owned protocol frame at a
        // time. Keep the real owner pumping while polling the reference peer;
        // a blocking peer-side wait would otherwise starve compound playlist
        // operations after the optimistic shell projection already matches.
        pump_and_apply(owner, handle, state);
        let snapshot = harness.peer_snapshot()?;
        if snapshot.playlist == playlist {
            return Ok(snapshot);
        }
        if Instant::now() >= deadline {
            return Err(LivePythonPeerInteropError::Gui(format!(
                "timed out waiting for live Python peer playlist observation; expected={playlist:?}, observed={:?}, observed_index={:?}, room={:?}",
                snapshot.playlist, snapshot.playlist_index, snapshot.room
            )));
        }
        thread::sleep(LIVE_PYTHON_INTEROP_POLL_INTERVAL);
    }
}

pub(in crate::app::live_python_interop) fn wait_for_peer_observed_playlist_index(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SorotteGuiShellAppState,
    harness: &mut LegacyServerPythonPeerHarness,
    index: usize,
    timeout: Duration,
) -> Result<LegacyPythonPeerSnapshot, LivePythonPeerInteropError> {
    let deadline = Instant::now() + timeout;
    loop {
        pump_and_apply(owner, handle, state);
        let snapshot = harness.peer_snapshot()?;
        if snapshot.playlist_index == Some(index) {
            return Ok(snapshot);
        }
        if Instant::now() >= deadline {
            return Err(LivePythonPeerInteropError::Gui(format!(
                "timed out waiting for live Python peer playlist-index observation; expected={index}, observed={:?}, playlist={:?}, room={:?}",
                snapshot.playlist_index, snapshot.playlist, snapshot.room
            )));
        }
        thread::sleep(LIVE_PYTHON_INTEROP_POLL_INTERVAL);
    }
}

pub(in crate::app::live_python_interop) fn wait_for_peer_observed_user_presence(
    harness: &mut LegacyServerPythonPeerHarness,
    username: &str,
    timeout: Duration,
) -> Result<LegacyPythonPeerSnapshot, LivePythonPeerInteropError> {
    harness
        .wait_for_peer_observed_user_presence(username, timeout)
        .map_err(LivePythonPeerInteropError::from)
}

#[cfg(test)]
pub(in crate::app::live_python_interop) fn wait_for_sustained_connection_presence(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SorotteGuiShellAppState,
    harness: &mut LegacyServerPythonPeerHarness,
    username: &str,
    duration: Duration,
) -> Result<(), LivePythonPeerInteropError> {
    let deadline = Instant::now() + duration;
    loop {
        pump_and_apply(owner, handle, state);
        let peer_snapshot = harness.peer_snapshot()?;
        let local_present = local_user_ready(state).is_some();
        let peer_observes_local = peer_snapshot.observed_users.contains_key(username);
        if !local_present || !peer_observes_local {
            return Err(LivePythonPeerInteropError::Gui(format!(
                "live Python detached-connect keepalive dropped before {duration:?}; room={:?}, projected_local_present={}, peer_users={:?}",
                state.main_window.room_name, local_present, peer_snapshot.observed_users
            )));
        }
        if Instant::now() >= deadline {
            return Ok(());
        }
        thread::sleep(LIVE_PYTHON_INTEROP_POLL_INTERVAL);
    }
}

pub(in crate::app::live_python_interop) fn wait_for_projected_room_projection(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SorotteGuiShellAppState,
    expected_room: &str,
    expected_peer_visible: bool,
) -> Result<(), LivePythonPeerInteropError> {
    let deadline = Instant::now() + LIVE_PYTHON_INTEROP_TIMEOUT;
    loop {
        pump_and_apply(owner, handle, state);
        let peer_visible = peer_user_ready(state, LIVE_PYTHON_INTEROP_PEER_USERNAME).is_some();
        if state.main_window.room_name == expected_room
            && local_user_ready(state).is_some()
            && peer_visible == expected_peer_visible
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
                "timed out waiting for live Python room projection; expected_room={expected_room:?}, expected_peer_visible={expected_peer_visible}, actual_room={:?}, users=[{}]",
                state.main_window.room_name, projected_users
            )));
        }
        thread::sleep(LIVE_PYTHON_INTEROP_POLL_INTERVAL);
    }
}

pub(in crate::app::live_python_interop) fn wait_for_projected_chat_message(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SorotteGuiShellAppState,
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

pub(in crate::app::live_python_interop) fn wait_for_projected_user_absence(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SorotteGuiShellAppState,
    username: &str,
) -> Result<(), LivePythonPeerInteropError> {
    let deadline = Instant::now() + LIVE_PYTHON_INTEROP_TIMEOUT;
    loop {
        pump_and_apply(owner, handle, state);
        if local_user_ready(state).is_some() && peer_user_ready(state, username).is_none() {
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
                "timed out waiting for live Python peer disconnect projection; username={username:?}, room={:?}, users=[{}]",
                state.main_window.room_name, projected_users
            )));
        }
        thread::sleep(LIVE_PYTHON_INTEROP_POLL_INTERVAL);
    }
}

pub(in crate::app::live_python_interop) fn wait_for_playlist_controls(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SorotteGuiShellAppState,
) -> Result<(), LivePythonPeerInteropError> {
    let deadline = Instant::now() + LIVE_PYTHON_INTEROP_TIMEOUT;
    loop {
        pump_and_apply(owner, handle, state);
        if state.main_window.playback.can_manage_playlist {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(LivePythonPeerInteropError::Gui(format!(
                "timed out waiting for live Python peer playlist controls; room={:?}, can_manage_playlist={}, playlist=[{}]",
                state.main_window.room_name,
                state.main_window.playback.can_manage_playlist,
                gui_playlist(state).join(" | ")
            )));
        }
        thread::sleep(LIVE_PYTHON_INTEROP_POLL_INTERVAL);
    }
}

pub(in crate::app::live_python_interop) fn wait_for_projected_playlist(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SorotteGuiShellAppState,
    expected_playlist: &[String],
    expected_index: Option<usize>,
) -> Result<(), LivePythonPeerInteropError> {
    let deadline = Instant::now() + LIVE_PYTHON_INTEROP_TIMEOUT;
    loop {
        pump_and_apply(owner, handle, state);
        if gui_playlist(state) == expected_playlist
            && state.selection.selected_main_window_playlist == expected_index
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(LivePythonPeerInteropError::Gui(format!(
                "timed out waiting for live Python peer playlist projection; expected_playlist={expected_playlist:?}, expected_index={expected_index:?}, actual_playlist={:?}, actual_index={:?}",
                gui_playlist(state),
                state.selection.selected_main_window_playlist
            )));
        }
        thread::sleep(LIVE_PYTHON_INTEROP_POLL_INTERVAL);
    }
}

pub(in crate::app::live_python_interop) fn wait_for_projection(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SorotteGuiShellAppState,
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

pub(in crate::app::live_python_interop) fn wait_for_controlled_room_projection(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SorotteGuiShellAppState,
) -> Result<(), LivePythonPeerInteropError> {
    let deadline = Instant::now() + LIVE_PYTHON_INTEROP_TIMEOUT;
    loop {
        pump_and_apply(owner, handle, state);

        let local_user_present = local_user_controller(state).is_some();
        let peer_user_present =
            peer_user_controller(state, LIVE_PYTHON_INTEROP_PEER_USERNAME).is_some();
        if state.main_window.room_name == LIVE_PYTHON_INTEROP_CONTROLLED_ROOM
            && local_user_present
            && peer_user_present
            && local_user_controller(state) == Some(true)
            && peer_user_controller(state, LIVE_PYTHON_INTEROP_PEER_USERNAME) == Some(false)
            && state.main_window.playback.can_manage_playlist
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
                "timed out waiting for live Python controlled-room projection; room={:?}, can_manage_playlist={}, users=[{}]",
                state.main_window.room_name,
                state.main_window.playback.can_manage_playlist,
                projected_users
            )));
        }

        thread::sleep(LIVE_PYTHON_INTEROP_POLL_INTERVAL);
    }
}

pub(in crate::app::live_python_interop) fn wait_for_controlled_room_peer_ready_projection(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SorotteGuiShellAppState,
    expected_peer_ready: bool,
) -> Result<(), LivePythonPeerInteropError> {
    let deadline = Instant::now() + LIVE_PYTHON_INTEROP_TIMEOUT;
    loop {
        pump_and_apply(owner, handle, state);

        let local_user_present = local_user_controller(state).is_some();
        let peer_user_present = peer_user_ready(state, LIVE_PYTHON_INTEROP_PEER_USERNAME).is_some();
        if state.main_window.room_name == LIVE_PYTHON_INTEROP_CONTROLLED_ROOM
            && local_user_present
            && peer_user_present
            && local_user_controller(state) == Some(true)
            && peer_user_controller(state, LIVE_PYTHON_INTEROP_PEER_USERNAME) == Some(false)
            && peer_user_ready(state, LIVE_PYTHON_INTEROP_PEER_USERNAME)
                == Some(expected_peer_ready)
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
                "timed out waiting for live Python controlled-room readiness projection; expected_peer_ready={expected_peer_ready}, room={:?}, users=[{}]",
                state.main_window.room_name, projected_users
            )));
        }

        thread::sleep(LIVE_PYTHON_INTEROP_POLL_INTERVAL);
    }
}
