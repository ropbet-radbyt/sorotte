use super::super::{
    GuiPendingCompletionRequest, GuiPersistedConfigRuntimeOwner, GuiQueuedRuntimeBridgeHandle,
    GuiQueuedRuntimeOwner, GuiRuntimeRequest, GuiShellAction, SyncplayGuiShellAppState,
};
use super::LivePythonPeerInteropError;

pub(in crate::app::live_python_interop) fn pump_and_apply(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
) {
    GuiQueuedRuntimeOwner::pump(owner, handle, state);
    for action in handle.drain_actions() {
        state.apply(action);
    }
}

pub(in crate::app::live_python_interop) fn request_local_ready(
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

pub(in crate::app::live_python_interop) fn request_remote_user_ready(
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
    username: &str,
    ready: bool,
) -> Result<(), LivePythonPeerInteropError> {
    if !state.apply(GuiShellAction::RequestMainWindowUserReady {
        username: username.to_owned(),
        ready,
    }) {
        return Err(LivePythonPeerInteropError::Gui(format!(
            "failed to stage remote readiness change for {username:?}; room={:?}",
            state.main_window.room_name
        )));
    }
    handle.push_request(GuiRuntimeRequest::SetReadyForUser {
        username: username.to_owned(),
        ready,
    });
    Ok(())
}

pub(in crate::app::live_python_interop) fn request_local_room_join(
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
    room: &str,
) -> Result<(), LivePythonPeerInteropError> {
    if !state.apply(GuiShellAction::JoinMainWindowRoom(room.to_owned())) {
        return Err(LivePythonPeerInteropError::Gui(format!(
            "failed to apply local room join {room:?}; room={:?}",
            state.main_window.room_name
        )));
    }
    handle.push_request(GuiRuntimeRequest::SetRoom(room.to_owned()));
    Ok(())
}

pub(in crate::app::live_python_interop) fn request_local_playlist_queue(
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
    entry: &str,
    select_after_queue: bool,
) -> Result<(), LivePythonPeerInteropError> {
    if !state.apply(GuiShellAction::AppendSharedPlaylistEntries(vec![
        entry.to_owned(),
    ])) {
        return Err(LivePythonPeerInteropError::Gui(format!(
            "failed to queue a local shared playlist entry {entry:?}; room={:?}",
            state.main_window.room_name
        )));
    }
    handle.push_request(GuiRuntimeRequest::QueuePlaylistEntry {
        entry: entry.to_owned(),
        select_after_queue,
    });
    Ok(())
}

pub(in crate::app::live_python_interop) fn request_local_playlist_selection(
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
    index: usize,
) -> Result<(), LivePythonPeerInteropError> {
    if !state.apply(GuiShellAction::SelectMainWindowPlaylist(index)) {
        return Err(LivePythonPeerInteropError::Gui(format!(
            "failed to select local shared playlist index {index}; room={:?}",
            state.main_window.room_name
        )));
    }
    handle.push_request(GuiRuntimeRequest::SetPlaylistIndex(index));
    Ok(())
}

pub(in crate::app::live_python_interop) fn request_local_playlist_remove_selected(
    handle: &GuiQueuedRuntimeBridgeHandle,
    state: &mut SyncplayGuiShellAppState,
) -> Result<(), LivePythonPeerInteropError> {
    let Some(index) = state.selection.selected_main_window_playlist else {
        return Err(LivePythonPeerInteropError::Gui(
            "failed to remove a local shared playlist entry because no row is selected.".to_owned(),
        ));
    };
    if !state.apply(GuiShellAction::RemoveSelectedMainWindowPlaylist) {
        return Err(LivePythonPeerInteropError::Gui(format!(
            "failed to remove local shared playlist index {index}; room={:?}",
            state.main_window.room_name
        )));
    }
    handle.push_request(GuiRuntimeRequest::DeletePlaylistIndex(index));
    Ok(())
}

#[cfg(test)]
pub(in crate::app::live_python_interop) fn request_local_shared_playlist_open(
    handle: &GuiQueuedRuntimeBridgeHandle,
    paths: &[&str],
) {
    handle.push_request(GuiRuntimeRequest::OpenMediaFiles {
        paths: paths.iter().map(|path| (*path).to_owned()).collect(),
        load_into_shared_playlist: true,
        playlist_insert_slot: None,
    });
}

pub(in crate::app::live_python_interop) fn request_local_chat_send(
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
