use syncplay_compat::LegacyPythonPeerChatMessage;

use super::super::SyncplayGuiShellAppState;
use super::LIVE_PYTHON_INTEROP_LOCAL_USERNAME;

pub(in crate::app::live_python_interop) fn local_user_ready(
    state: &SyncplayGuiShellAppState,
) -> Option<bool> {
    state
        .main_window
        .users
        .iter()
        .find(|user| user.username == LIVE_PYTHON_INTEROP_LOCAL_USERNAME && user.is_self)
        .map(|user| user.is_ready)
}

pub(in crate::app::live_python_interop) fn local_user_controller(
    state: &SyncplayGuiShellAppState,
) -> Option<bool> {
    state
        .main_window
        .users
        .iter()
        .find(|user| user.username == LIVE_PYTHON_INTEROP_LOCAL_USERNAME && user.is_self)
        .map(|user| user.is_controller)
}

pub(in crate::app::live_python_interop) fn peer_user_ready(
    state: &SyncplayGuiShellAppState,
    username: &str,
) -> Option<bool> {
    state
        .main_window
        .users
        .iter()
        .find(|user| {
            user.username == username
                && !user.is_self
                && user.room_name == state.main_window.room_name
        })
        .map(|user| user.is_ready)
}

pub(in crate::app::live_python_interop) fn peer_user_controller(
    state: &SyncplayGuiShellAppState,
    username: &str,
) -> Option<bool> {
    state
        .main_window
        .users
        .iter()
        .find(|user| {
            user.username == username
                && !user.is_self
                && user.room_name == state.main_window.room_name
        })
        .map(|user| user.is_controller)
}

pub(in crate::app::live_python_interop) fn gui_playlist(
    state: &SyncplayGuiShellAppState,
) -> Vec<String> {
    state
        .main_window
        .playlist
        .iter()
        .map(|row| row.label.clone())
        .collect()
}

pub(in crate::app::live_python_interop) fn merge_peer_chat_messages(
    destination: &mut Vec<LegacyPythonPeerChatMessage>,
    additional: Vec<LegacyPythonPeerChatMessage>,
) {
    for message in additional {
        if !destination.iter().any(|existing| existing == &message) {
            destination.push(message);
        }
    }
}
